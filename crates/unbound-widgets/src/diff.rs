//! 两帧之间的差异，表现为上传计划（Diff）。
//!
//! UI 帧从应用状态重建，因此每帧都拥有*完整*场景。全部重新上传会造成浪费：
//! 悬停仅重绘单个控件，而一个控件在数千个图元的场景中仅占少数几个图元。
//!
//! [`diff_primitives`] 回答渲染器唯一关心的问题——“图元池中哪些字节范围已过期？”——
//! 当切片补丁无法修复图元池时则拒绝使用补丁，从而确保渲染器始终拥有正确的回退方案。

use unbound_gpu::data::GpuPrimitiveInstance;

/// 图元池中发生变化的单个切片，以 `[start, end)` 图元索引表示。
pub type Range = (u32, u32);

/// 两帧之间的变化类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneDiff {
    /// 无任何变化：无需上传。
    Identical,
    /// 仅图元*内容*发生变化，渲染器可精确修补这些切片并保留图元池其余部分。
    Ranges(Vec<Range>),
    /// 帧结构发生变化——图元数量改变，或变换/裁剪/载荷表改变——切片补丁无法修复。
    Whole,
}

impl SceneDiff {
    /// 对于包含 `primitives` 个图元的帧，此差异将通过总线传输的字节数。
    #[must_use]
    pub fn bytes(&self, primitives: usize) -> u32 {
        let stride = core::mem::size_of::<GpuPrimitiveInstance>();
        match self {
            Self::Identical => 0,
            Self::Ranges(ranges) => ranges
                .iter()
                .map(|(start, end)| (end - start) as usize * stride)
                .sum::<usize>() as u32,
            Self::Whole => (primitives * stride) as u32,
        }
    }

    /// 渲染器是否需要推进上传代数。
    ///
    /// 切片补丁无需推进：图元池版本号不变，仅更新部分内容。其余情况均需推进。
    #[must_use]
    pub fn bumps_generation(&self) -> bool {
        matches!(self, Self::Whole)
    }

    /// 需要修补的范围列表，或空切片。
    #[must_use]
    pub fn ranges(&self) -> &[Range] {
        match self {
            Self::Ranges(ranges) => ranges,
            _ => &[],
        }
    }
}

/// 计算两个图元列表之间的差异。
///
/// `structure_changed` 表示图元*索引指向*的表（变换、裁剪、载荷）是否发生变化。
/// 补丁仅写入图元本身，因此即使图元数量未变，表结构变化也会使整个图元池失效。
///
/// 仅当补丁成本低于重新上传整个场景时才值得使用：超过半数图元变化时直接整帧重传，
/// 因为驱动遍历补丁列表的耗时将高于直接拷贝字节。
#[must_use]
pub fn diff_primitives(
    previous: &[GpuPrimitiveInstance],
    current: &[GpuPrimitiveInstance],
    structure_changed: bool,
) -> SceneDiff {
    if structure_changed || previous.len() != current.len() {
        return SceneDiff::Whole;
    }

    let stride = core::mem::size_of::<GpuPrimitiveInstance>() as u64;
    let total = current.len() as u64 * stride;
    let mut ranges: Vec<Range> = Vec::new();
    let mut start: Option<u32> = None;
    for (index, (old, new)) in previous.iter().zip(current.iter()).enumerate() {
        if old == new {
            if let Some(begin) = start.take() {
                ranges.push((begin, index as u32));
            }
        } else if start.is_none() {
            start = Some(index as u32);
        }
    }
    if let Some(begin) = start {
        ranges.push((begin, current.len() as u32));
    }

    if ranges.is_empty() {
        return SceneDiff::Identical;
    }

    let dirty: u64 = ranges
        .iter()
        .map(|(start, end)| u64::from(end - start) * stride)
        .sum();
    if dirty * 2 >= total {
        return SceneDiff::Whole;
    }
    SceneDiff::Ranges(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成 `count` 个互不相同的图元列表，以便区分单个图元的变更。
    fn frame(count: usize) -> Vec<GpuPrimitiveInstance> {
        (0..count)
            .map(|index| GpuPrimitiveInstance {
                local_pos: [index as f32, 0.0],
                ..GpuPrimitiveInstance::default()
            })
            .collect()
    }

    #[test]
    fn an_unchanged_frame_uploads_nothing() {
        let previous = frame(100);
        let current = previous.clone();
        assert_eq!(
            diff_primitives(&previous, &current, false),
            SceneDiff::Identical
        );
        assert_eq!(
            diff_primitives(&previous, &current, false).bytes(100),
            0,
            "an identical frame must not move a single byte"
        );
    }

    #[test]
    fn a_hover_patches_only_its_own_primitives() {
        // 同屏 100 个控件，每个包含 8 个图元。
        const CONTROLS: usize = 100;
        const PER_CONTROL: usize = 8;
        let previous = frame(CONTROLS * PER_CONTROL);
        let mut current = previous.clone();
        // 第 5 个控件发生变化：连续 8 个图元。
        let first = 4 * PER_CONTROL;
        for prim in &mut current[first..first + PER_CONTROL] {
            prim.local_pos[1] += 1.0;
        }
        let diff = diff_primitives(&previous, &current, false);
        assert_eq!(
            diff,
            SceneDiff::Ranges(vec![(first as u32, (first + PER_CONTROL) as u32)])
        );
        let stride = core::mem::size_of::<GpuPrimitiveInstance>() as u32;
        assert_eq!(diff.bytes(current.len()), PER_CONTROL as u32 * stride);
        assert!(
            diff.bytes(current.len()) < 1024,
            "hovering one of 100 controls must stay under 1 KiB, got {} B",
            diff.bytes(current.len())
        );
        assert!(!diff.bumps_generation(), "a patch keeps the generation");
    }

    #[test]
    fn scattered_changes_become_separate_ranges() {
        let previous = frame(100);
        let mut current = previous.clone();
        current[10].local_pos[1] += 1.0;
        current[70].local_pos[1] += 1.0;
        assert_eq!(
            diff_primitives(&previous, &current, false),
            SceneDiff::Ranges(vec![(10, 11), (70, 71)])
        );
    }

    #[test]
    fn a_run_of_changes_is_merged_into_one_range() {
        let previous = frame(100);
        let mut current = previous.clone();
        for prim in &mut current[20..30] {
            prim.local_pos[1] += 1.0;
        }
        assert_eq!(
            diff_primitives(&previous, &current, false),
            SceneDiff::Ranges(vec![(20, 30)]),
            "one contiguous run is one copy, not ten"
        );
    }

    #[test]
    fn a_changed_shape_uploads_the_whole_frame() {
        let previous = frame(100);
        let current = frame(101);
        assert_eq!(
            diff_primitives(&previous, &current, false),
            SceneDiff::Whole
        );
        assert!(diff_primitives(&previous, &current, false).bumps_generation());
    }

    #[test]
    fn a_changed_transform_table_uploads_the_whole_frame() {
        // 图元列表完全相同，但变换表中的某个索引含义已变，若打补丁则会绘制旧几何。
        let previous = frame(100);
        let current = previous.clone();
        assert_eq!(diff_primitives(&previous, &current, true), SceneDiff::Whole);
    }

    #[test]
    fn a_patch_that_costs_more_than_half_the_frame_gives_up() {
        let previous = frame(100);
        let mut current = previous.clone();
        // 100 个图元中有 60 个发生变化：补丁列表开销高于其节省的拷贝量。
        for prim in &mut current[..60] {
            prim.local_pos[1] += 1.0;
        }
        assert_eq!(
            diff_primitives(&previous, &current, false),
            SceneDiff::Whole
        );
    }
}

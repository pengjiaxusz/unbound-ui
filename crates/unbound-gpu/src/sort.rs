//! 64 位深度排序键及 GPU OneSweep 基数排序 + 间接命令压缩的 CPU 参考实现
//! （参见 `docs/architecture/gpu-renderer.md`）。
//!
//! 实际计算在 GPU 上完成；保留本模块基于三个原因：
//!
//! 1. 将位布局集中定义在唯一位置，供计算着色器与测试共享；
//! 2. 提供 CPU 参考实现，使单元测试可逐比特校验 GPU 输出；
//! 3. [`build_indirect_runs`] 精确记录了排序后的键如何折叠为单帧提交的少量
//!    `VkDrawIndirectCommand`。

use crate::data::{DrawIndirectCommand, PassLayer, SubElement};

/// 64 位排序键的位布局定义。
pub mod bits {
    /// 8 位 Pass / Portal 图层。
    pub const PASS_SHIFT: u32 = 56;
    pub const PASS_MASK: u64 = 0xff << PASS_SHIFT;
    /// 16 位渲染图屏障 ID。
    pub const BARRIER_SHIFT: u32 = 40;
    pub const BARRIER_MASK: u64 = 0xffff << BARRIER_SHIFT;
    /// 4 位管线变体。
    pub const VARIANT_SHIFT: u32 = 36;
    pub const VARIANT_MASK: u64 = 0xf << VARIANT_SHIFT;
    /// 28 位拓扑 Z 序（约 2.68 亿层）。
    pub const Z_SHIFT: u32 = 8;
    pub const Z_MASK: u64 = 0x0fff_ffff << Z_SHIFT;
    /// 8 位子元素顺序。
    pub const SUB_MASK: u64 = 0xff;
    pub const MAX_Z: u32 = 0x0fff_ffff;
    pub const MAX_BARRIER: u16 = 0xffff;
    pub const MAX_VARIANT: u8 = 0x0f;
}

/// 打包后的 64 位深度排序键。
///
/// 升序排列可在同一 Pass 图层内得到严格由后向前的画家顺序，
/// 且 Pass/Portal 图层始终排在画布图层之上。
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SortKey(pub u64);

impl SortKey {
    /// 构建排序键，将各字段钳制到对应位宽内。
    #[must_use]
    pub const fn new(layer: u8, barrier: u16, variant: u8, z: u32, sub: u8) -> Self {
        let z = if z > bits::MAX_Z { bits::MAX_Z } else { z };
        let variant = if variant > bits::MAX_VARIANT {
            bits::MAX_VARIANT
        } else {
            variant
        };
        Self(
            ((layer as u64) << bits::PASS_SHIFT)
                | (((barrier as u64) & bits::MAX_BARRIER as u64) << bits::BARRIER_SHIFT)
                | ((variant as u64) << bits::VARIANT_SHIFT)
                | ((z as u64) << bits::Z_SHIFT)
                | (sub as u64),
        )
    }

    /// 根据语义图层与子元素类型构建排序键。
    #[must_use]
    pub const fn simple(layer: PassLayer, z: u32, sub: SubElement) -> Self {
        Self::new(layer as u8, 0, 0, z, sub as u8)
    }

    #[inline]
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn layer(self) -> u8 {
        ((self.0 & bits::PASS_MASK) >> bits::PASS_SHIFT) as u8
    }

    #[inline]
    #[must_use]
    pub const fn barrier(self) -> u16 {
        ((self.0 & bits::BARRIER_MASK) >> bits::BARRIER_SHIFT) as u16
    }

    #[inline]
    #[must_use]
    pub const fn variant(self) -> u8 {
        ((self.0 & bits::VARIANT_MASK) >> bits::VARIANT_SHIFT) as u8
    }

    #[inline]
    #[must_use]
    pub const fn z(self) -> u32 {
        ((self.0 & bits::Z_MASK) >> bits::Z_SHIFT) as u32
    }

    #[inline]
    #[must_use]
    pub const fn sub(self) -> u8 {
        (self.0 & bits::SUB_MASK) as u8
    }

    /// 替换 Z 序同时保留其余字段。
    #[must_use]
    pub const fn with_z(self, z: u32) -> Self {
        let z = if z > bits::MAX_Z { bits::MAX_Z } else { z };
        Self((self.0 & !bits::Z_MASK) | ((z as u64) << bits::Z_SHIFT))
    }

    /// 替换屏障 ID。
    #[must_use]
    pub const fn with_barrier(self, barrier: u16) -> Self {
        Self((self.0 & !bits::BARRIER_MASK) | ((barrier as u64) << bits::BARRIER_SHIFT))
    }

    /// 定义单个间接绘制批次的 `(layer, barrier, variant)` 三元组。
    #[inline]
    #[must_use]
    pub const fn run_id(self) -> u32 {
        ((self.layer() as u32) << 20) | ((self.barrier() as u32) << 4) | (self.variant() as u32)
    }
}

/// CPU 参考实现：基于 8 位数字的 LSD 基数排序。
///
/// 稳定、复杂度 O(8n) 且分配有界（仅一个暂存向量）。供测试校验 GPU 实现，
/// 并在设备不支持 `shaderInt64` 时作为回退路径。
pub fn radix_sort_u64(keys: &mut [u64]) {
    radix_sort_by(keys, |k| *k);
}

/// 带键提取器的稳定基数排序（允许直接排序 `(key, payload)` 对而无需二次间接寻址）。
pub fn radix_sort_by<T, F>(items: &mut [T], key_of: F)
where
    T: Copy + Default,
    F: Fn(&T) -> u64,
{
    let n = items.len();
    if n < 2 {
        return;
    }
    let mut src = items.to_vec();
    let mut dst = vec![T::default(); n];
    for shift in (0..64).step_by(8) {
        let mut counts = [0u32; 256];
        for item in &src {
            let digit = ((key_of(item) >> shift) & 0xff) as usize;
            counts[digit] += 1;
        }
        // 提前退出：仅有一个非空桶说明本趟排序无需搬移数据。
        if counts.iter().filter(|c| **c > 0).count() == 1 {
            continue;
        }
        let mut sum = 0u32;
        for c in &mut counts {
            let v = *c;
            *c = sum;
            sum += v;
        }
        for item in &src {
            let digit = ((key_of(item) >> shift) & 0xff) as usize;
            dst[counts[digit] as usize] = *item;
            counts[digit] += 1;
        }
        core::mem::swap(&mut src, &mut dst);
    }
    items.copy_from_slice(&src);
}

/// 将已排序键数组折叠为连续的间接绘制批次。
///
/// 每当 `(pass, barrier, variant)` 三元组变化时即开启新批次——这正是渲染图
/// 需要插入屏障或切换管线的位置，因此每个批次对应一条 `vkCmdDrawIndirect`。
#[must_use]
pub fn build_indirect_runs(keys: &[SortKey]) -> Vec<DrawIndirectCommand> {
    let mut runs: Vec<DrawIndirectCommand> = Vec::new();
    if keys.is_empty() {
        return runs;
    }
    let mut current_run = keys[0].run_id();
    let mut start: u32 = 0;
    for (i, key) in keys.iter().enumerate().skip(1) {
        let run = key.run_id();
        if run != current_run {
            runs.push(DrawIndirectCommand::new(i as u32 - start, start));
            current_run = run;
            start = i as u32;
        }
    }
    runs.push(DrawIndirectCommand::new(keys.len() as u32 - start, start));
    runs
}

/// 批次列表覆盖的实例总数（用于压缩结果合理性校验）。
#[must_use]
pub fn total_instances(runs: &[DrawIndirectCommand]) -> u32 {
    runs.iter().map(|r| r.instance_count).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_round_trip_at_their_extremes() {
        let k = SortKey::new(255, 0xffff, 0x0f, bits::MAX_Z, 255);
        assert_eq!(k.layer(), 255);
        assert_eq!(k.barrier(), 0xffff);
        assert_eq!(k.variant(), 0x0f);
        assert_eq!(k.z(), bits::MAX_Z);
        assert_eq!(k.sub(), 255);
        assert_eq!(k.raw(), u64::MAX);
    }

    #[test]
    fn fields_do_not_bleed_into_each_other() {
        let k = SortKey::new(2, 0x1234, 3, 0x0abc_def0, 5);
        assert_eq!(k.layer(), 2);
        assert_eq!(k.barrier(), 0x1234);
        assert_eq!(k.variant(), 3);
        assert_eq!(k.z(), 0x0abc_def0);
        assert_eq!(k.sub(), 5);
    }

    #[test]
    fn out_of_range_values_are_clamped_not_wrapped() {
        let k = SortKey::new(1, 0, 0xff, u32::MAX, 1);
        assert_eq!(k.variant(), bits::MAX_VARIANT);
        assert_eq!(k.z(), bits::MAX_Z);
        assert_eq!(k.layer(), 1, "z must not overflow into the layer bits");
    }

    #[test]
    fn with_z_preserves_the_other_fields() {
        let k = SortKey::new(3, 77, 2, 10, 4).with_z(999);
        assert_eq!(
            (k.layer(), k.barrier(), k.variant(), k.z(), k.sub()),
            (3, 77, 2, 999, 4)
        );
    }

    #[test]
    fn with_barrier_preserves_the_other_fields() {
        let k = SortKey::new(3, 1, 2, 10, 4).with_barrier(0xbeef);
        assert_eq!(
            (k.layer(), k.barrier(), k.variant(), k.z(), k.sub()),
            (3, 0xbeef, 2, 10, 4)
        );
    }

    #[test]
    fn ordering_is_layer_then_z_then_subelement() {
        let behind = SortKey::simple(PassLayer::Ui, 10, SubElement::Background);
        let front = SortKey::simple(PassLayer::Ui, 20, SubElement::Background);
        let overlay = SortKey::simple(PassLayer::Overlay, 1, SubElement::Background);
        assert!(behind < front);
        assert!(front < overlay, "portals always paint above the UI layer");
        // 同一元素内部顺序：阴影 < 背景 < 内容 < 边框。
        let shadow = SortKey::simple(PassLayer::Ui, 10, SubElement::Shadow);
        let border = SortKey::simple(PassLayer::Ui, 10, SubElement::Border);
        assert!(shadow < behind);
        assert!(behind < border);
    }

    #[test]
    fn run_id_groups_by_layer_barrier_variant() {
        let a = SortKey::new(1, 0, 0, 5, 0);
        let b = SortKey::new(1, 0, 0, 9, 3);
        let c = SortKey::new(1, 1, 0, 9, 3);
        assert_eq!(a.run_id(), b.run_id());
        assert_ne!(a.run_id(), c.run_id());
    }

    #[test]
    fn radix_sort_matches_std_sort_on_random_input() {
        let mut seed = 0xdead_beef_1234_5678u64;
        let mut keys: Vec<u64> = (0..100_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed
            })
            .collect();
        let mut expected = keys.clone();
        expected.sort_unstable();
        radix_sort_u64(&mut keys);
        assert_eq!(keys, expected);
    }

    #[test]
    fn radix_sort_is_stable() {
        #[derive(Copy, Clone, Default, Debug, PartialEq)]
        struct Item {
            key: u64,
            tag: u32,
        }
        let mut items: Vec<Item> = (0..1000)
            .map(|i| Item {
                key: (i % 7) as u64,
                tag: i,
            })
            .collect();
        radix_sort_by(&mut items, |i| i.key);
        for w in items.windows(2) {
            assert!(w[0].key <= w[1].key);
            if w[0].key == w[1].key {
                assert!(w[0].tag < w[1].tag, "equal keys must keep input order");
            }
        }
    }

    #[test]
    fn radix_sort_handles_degenerate_inputs() {
        let mut empty: Vec<u64> = vec![];
        radix_sort_u64(&mut empty);
        assert!(empty.is_empty());
        let mut one = vec![42u64];
        radix_sort_u64(&mut one);
        assert_eq!(one, vec![42]);
        let mut same = vec![7u64; 100];
        radix_sort_u64(&mut same);
        assert!(same.iter().all(|k| *k == 7));
        let mut reversed: Vec<u64> = (0..64).rev().collect();
        radix_sort_u64(&mut reversed);
        assert!(reversed.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn indirect_runs_cover_every_instance_exactly_once() {
        let mut keys: Vec<SortKey> = Vec::new();
        // 3 个 UI 面板（各含 100 个图元）外加 50 个浮层项。
        for panel in 0..3u32 {
            for i in 0..100u32 {
                keys.push(SortKey::simple(
                    PassLayer::Ui,
                    panel * 1000 + i,
                    SubElement::Background,
                ));
            }
        }
        for i in 0..50u32 {
            keys.push(SortKey::simple(PassLayer::Overlay, i, SubElement::Content));
        }
        radix_sort_by(&mut keys, |k| k.0);
        let runs = build_indirect_runs(&keys);
        assert_eq!(runs.len(), 2, "one run per layer, got {runs:?}");
        assert_eq!(total_instances(&runs), 350);
        assert_eq!(runs[0].instance_count, 300);
        assert_eq!(runs[1].first_instance, 300);
    }

    #[test]
    fn barrier_change_splits_a_run() {
        let keys = vec![
            SortKey::new(1, 0, 0, 1, 0),
            SortKey::new(1, 0, 0, 2, 0),
            SortKey::new(1, 1, 0, 3, 0),
        ];
        let runs = build_indirect_runs(&keys);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].instance_count, 2);
        assert_eq!(runs[1].first_instance, 2);
    }

    #[test]
    fn empty_input_yields_no_runs() {
        assert!(build_indirect_runs(&[]).is_empty());
        assert_eq!(total_instances(&[]), 0);
    }
}

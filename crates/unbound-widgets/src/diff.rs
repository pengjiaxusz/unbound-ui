//! The diff between two frames, expressed as an upload plan.
//!
//! A UI frame is rebuilt from the application state, so the *whole* scene is
//! available on every frame. Re-uploading all of it would be wasteful: a
//! hover repaints one control, and one control is a handful of primitives in
//! a scene that may hold thousands.
//!
//! [`diff_primitives`] answers the only question the renderer has to ask —
//! "which byte ranges of the primitive pool are stale?" — and refuses to
//! answer it when a slice patch could not repair the pool, so the renderer
//! always has a correct fallback.

use unbound_gpu::data::GpuPrimitiveInstance;

/// One changed slice of the primitive pool, as `[start, end)` primitive
/// indices.
pub type Range = (u32, u32);

/// What changed between two frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneDiff {
    /// Nothing changed: there is nothing to upload.
    Identical,
    /// Only primitive *contents* changed, so the renderer can patch exactly
    /// these slices and leave the rest of the pool alone.
    Ranges(Vec<Range>),
    /// The frame's shape changed — a different primitive count, or a
    /// different transform / clip / payload table — and a slice patch could
    /// not repair it.
    Whole,
}

impl SceneDiff {
    /// The bytes this diff would put on the bus, for a frame of
    /// `primitives` primitives.
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

    /// Whether the renderer has to move the upload generation forward.
    ///
    /// A slice patch does not: the version the pool is at does not change,
    /// only some of its contents. Anything else does.
    #[must_use]
    pub fn bumps_generation(&self) -> bool {
        matches!(self, Self::Whole)
    }

    /// The ranges to patch, or an empty slice.
    #[must_use]
    pub fn ranges(&self) -> &[Range] {
        match self {
            Self::Ranges(ranges) => ranges,
            _ => &[],
        }
    }
}

/// The diff between two primitive lists.
///
/// `structure_changed` says whether the tables a primitive *indexes into*
/// (transforms, clips, payloads) changed. A patch only writes primitives, so
/// a changed table invalidates the whole pool even when the primitive count
/// is unchanged.
///
/// A patch is only worth it while it stays cheaper than re-uploading the
/// scene: past half the pool the whole scene goes up again, because the
/// driver would spend more time walking the patch list than copying the
/// bytes.
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

    /// A primitive list of `count` instances that all differ from each other,
    /// so a change to one of them is distinguishable.
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
        // A hundred controls on one screen, eight primitives each.
        const CONTROLS: usize = 100;
        const PER_CONTROL: usize = 8;
        let previous = frame(CONTROLS * PER_CONTROL);
        let mut current = previous.clone();
        // The fifth control changed: eight consecutive primitives.
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
        // The primitive list is identical, but an index into the transform
        // table now means something else, so a patch would draw the old
        // geometry.
        let previous = frame(100);
        let current = previous.clone();
        assert_eq!(diff_primitives(&previous, &current, true), SceneDiff::Whole);
    }

    #[test]
    fn a_patch_that_costs_more_than_half_the_frame_gives_up() {
        let previous = frame(100);
        let mut current = previous.clone();
        // Sixty of a hundred primitives changed: the patch list would be
        // longer than the copy it saves.
        for prim in &mut current[..60] {
            prim.local_pos[1] += 1.0;
        }
        assert_eq!(
            diff_primitives(&previous, &current, false),
            SceneDiff::Whole
        );
    }
}

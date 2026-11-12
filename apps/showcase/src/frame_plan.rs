//! The frame plan: what size a frame is presented at, and what the renderer
//! has to upload.
//!
//! Two rules live here, and both of them exist because the window and the
//! swapchain answer different questions:
//!
//! * **The presented size is the committed swapchain extent.** The driver
//!   decides how large the swapchain images are — `resolve_extent` takes the
//!   driver's `currentExtent` over the requested size — and the image is what
//!   the compositor shows. The layout therefore follows the extent, not the
//!   window: a window size the driver answered differently would lay the UI
//!   out for one size and project it into another, and the UI would either
//!   leave a blank band on the right/bottom or be cropped away.
//! * **A rebuilt frame is a fully uploaded frame.** The runtime moves its
//!   diff base on every rebuild, whether the frame reaches the GPU or not. A
//!   frame that was dropped therefore leaves the pool behind the base, and
//!   the next upload has to be a full one — otherwise the dropped slice is
//!   never repaired and stays stale on screen forever.

use unbound_widgets::diff::SceneDiff;

/// The size a frame is presented at.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameSize {
    /// The committed swapchain extent, in physical pixels.
    pub physical: (u32, u32),
    /// The layout viewport, in logical units.
    pub logical: (f32, f32),
}

/// Derive the layout viewport from the committed swapchain extent.
///
/// The scale is the effective one (`OS scale × app zoom`): the layout works
/// in logical units and the camera scales them back up.
#[must_use]
pub fn frame_size(chain_extent: (u32, u32), scale: f32) -> FrameSize {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let physical = (chain_extent.0.max(1), chain_extent.1.max(1));
    FrameSize {
        physical,
        logical: (physical.0 as f32 / scale, physical.1 as f32 / scale),
    }
}

/// Whether the swapchain has to be rebuilt.
///
/// Both sizes are **requested** sizes: the one the window asks for now and
/// the one the chain was last created for. The chain's *actual* extent is the
/// driver's answer and may legitimately differ from the request — the driver
/// clamps it to its own limits and reports its own view of the window.
/// Comparing the actual extent against the request would rebuild the chain on
/// every frame and never converge, and it would present a layout that belongs
/// to one size in an image of another.
#[must_use]
pub fn needs_rebuild(requested: (u32, u32), committed_request: (u32, u32), dirty: bool) -> bool {
    dirty || requested != committed_request
}

/// What the renderer has to upload for one frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct UploadPlan<'a> {
    /// The generation the renderer compares against its own.
    pub generation: u64,
    /// The slices to patch; an empty list means "upload everything".
    pub ranges: &'a [(u32, u32)],
}

/// The upload plan for the frame that is about to be drawn.
///
/// `dropped` says that the **previous** frame was rebuilt but never reached
/// the GPU (the swapchain was out of date, the render failed, the ring was
/// exhausted). The runtime's diff base moved on regardless, so the pool is
/// behind it: patching the diff's slices would repair the wrong bytes, and
/// the next identical frame would report "nothing changed" and upload
/// nothing at all. Moving the generation turns the next upload into a full
/// one, which is the price of a dropped frame.
#[must_use]
pub fn upload_plan<'a>(generation: u64, diff: &'a SceneDiff, dropped: bool) -> UploadPlan<'a> {
    let mut generation = generation;
    if diff.bumps_generation() {
        generation = generation.wrapping_add(1);
    }
    if dropped {
        return UploadPlan {
            generation: generation.wrapping_add(1),
            ranges: &[],
        };
    }
    UploadPlan {
        generation,
        ranges: diff.ranges(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_widgets::diff::SceneDiff;

    /// The layout viewport is derived from the extent, never from the window.
    #[test]
    fn the_layout_follows_the_committed_extent() {
        let size = frame_size((1919, 1039), 1.5);
        assert_eq!(size.physical, (1919, 1039));
        assert!((size.logical.0 - 1919.0 / 1.5).abs() < 1e-4);
        assert!((size.logical.1 - 1039.0 / 1.5).abs() < 1e-4);
    }

    /// A degenerate scale must not produce a degenerate viewport.
    #[test]
    fn a_broken_scale_falls_back_to_one() {
        let size = frame_size((800, 600), 0.0);
        assert_eq!(size.logical, (800.0, 600.0));
        let size = frame_size((800, 600), f32::NAN);
        assert_eq!(size.logical, (800.0, 600.0));
    }

    /// A chain that is one pixel off the request must not rebuild forever.
    #[test]
    fn a_one_pixel_difference_does_not_rebuild_forever() {
        assert!(
            !needs_rebuild((1920, 1040), (1920, 1040), false),
            "the request did not change"
        );
        assert!(
            needs_rebuild((1921, 1040), (1920, 1040), false),
            "the window asked for a different size"
        );
        assert!(
            needs_rebuild((1920, 1040), (1920, 1040), true),
            "the driver asked for a rebuild"
        );
    }

    /// A dropped frame must make the next upload a full one.
    #[test]
    fn a_dropped_frame_forces_a_full_upload() {
        let diff = SceneDiff::Ranges(vec![(3, 4)]);
        let plan = upload_plan(7, &diff, true);
        assert!(
            plan.ranges.is_empty(),
            "a patch would repair the wrong slices"
        );
        assert_ne!(
            plan.generation, 7,
            "the generation has to move so the renderer re-uploads everything"
        );
    }

    /// A frame that reached the GPU keeps the cheap path.
    #[test]
    fn an_uploaded_frame_keeps_its_patch() {
        let diff = SceneDiff::Ranges(vec![(3, 4)]);
        let plan = upload_plan(7, &diff, false);
        assert_eq!(plan.ranges, &[(3, 4)]);
        assert_eq!(plan.generation, 7);
    }

    /// A shape change always moves the generation: the pool has to be
    /// rewritten from scratch.
    #[test]
    fn a_shape_change_moves_the_generation() {
        let plan = upload_plan(7, &SceneDiff::Whole, false);
        assert_eq!(plan.generation, 8);
        assert!(plan.ranges.is_empty());
    }
}

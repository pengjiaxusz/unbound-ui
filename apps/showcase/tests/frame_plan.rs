//! The frame plan: the size a frame is presented at, and what it uploads.
//!
//! Two invariants live here, and both of them were violated in the field:
//!
//! * The layout follows the **committed swapchain extent**, not the window
//!   size. The driver decides how large the images are, and the image is what
//!   the compositor shows; a layout built for the window size would leave a
//!   blank band on the right/bottom or be cropped away.
//! * A frame that was **dropped** (rebuilt, but never drawn) is repaired by a
//!   **full** upload. The runtime moves its diff base on every rebuild, so
//!   after a drop the pool is behind the base: patching the diff's slices
//!   would repair the wrong bytes, and the next identical frame would report
//!   "nothing changed" and upload nothing at all.

use unbound_app::AppRuntime;
use unbound_math::Vec2;
use unbound_widgets::diff::SceneDiff;

use showcase::frame_plan::{frame_size, needs_rebuild, upload_plan};
use showcase::pages::PageId;
use showcase::runtime_app::GalleryRuntime;

fn runtime() -> AppRuntime<GalleryRuntime> {
    AppRuntime::new(GalleryRuntime::new(), Vec2::new(1280.0, 800.0))
}

/// The layout viewport is derived from the extent the frame is drawn at.
///
/// The chain answers with the driver's own size, which may differ from the
/// requested one by a pixel. The page has to be laid out for the size the
/// image actually has.
#[test]
fn the_layout_follows_the_committed_extent() {
    let size = frame_size((1920, 1040), 1.0);
    assert_eq!(size.physical, (1920, 1040));
    assert_eq!(size.logical, (1920.0, 1040.0));

    // A zoom divides the physical size: the layout shrinks, the frame does
    // not.
    let zoomed = frame_size((1920, 1040), 2.0);
    assert_eq!(zoomed.physical, (1920, 1040));
    assert_eq!(zoomed.logical, (960.0, 520.0));
}

/// The window's request and the chain's actual extent are never compared.
///
/// The driver clamps the chain to its own limits and reports its own view of
/// the window, so the two can differ legitimately. Comparing them would
/// rebuild the chain on every frame — and never converge.
#[test]
fn a_one_pixel_difference_does_not_rebuild_forever() {
    assert!(
        !needs_rebuild((1920, 1040), (1920, 1040), false),
        "the request did not change, so the chain is fine"
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

/// A dropped frame is repaired by a full upload.
///
/// The runtime's diff base moved on when the dropped frame was rebuilt, so
/// the pool is behind it. The next upload has to carry the whole scene —
/// a patch would repair the wrong slices, and the frame after that would
/// report "identical" and upload nothing at all.
#[test]
fn a_dropped_frame_is_repaired_by_a_full_upload() {
    let mut runtime = runtime();
    let first = runtime.frame(0.0);
    assert_eq!(
        first.diff,
        SceneDiff::Whole,
        "the first frame has no predecessor"
    );

    // The frame was rebuilt but never drawn: the runtime's base moved on,
    // the GPU's pool did not.
    runtime
        .application_mut()
        .shell_mut()
        .navigate(PageId::Telemetry);
    runtime.invalidate();
    let dropped = runtime.frame(0.0);
    assert_ne!(dropped.diff, SceneDiff::Identical);

    // The next frame would be "identical" — the base already moved.
    let next = runtime.frame(0.0);
    assert_eq!(
        next.diff,
        SceneDiff::Identical,
        "the base moved on when the frame was rebuilt"
    );

    // A patch here would leave the dropped frame's content on screen
    // forever. The plan has to be a full upload.
    let plan = upload_plan(7, &next.diff, true);
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

/// A shape change always moves the generation: the pool has to be rewritten
/// from scratch.
#[test]
fn a_shape_change_moves_the_generation() {
    let plan = upload_plan(7, &SceneDiff::Whole, false);
    assert_eq!(plan.generation, 8);
    assert!(plan.ranges.is_empty());
}

/// The page reflows for the size the frame is drawn at.
///
/// The host hands the runtime the logical viewport derived from the
/// committed extent; the page has to be laid out for *that* viewport, not
/// for the window's.
#[test]
fn the_page_reflows_for_the_presented_size() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    let wide = runtime.application().shell().layout.content.size.x;

    // The window was maximised: the chain now answers with a wider extent,
    // and the logical viewport follows it.
    let presented = frame_size((2560, 1400), 1.0);
    runtime.resize(Vec2::new(presented.logical.0, presented.logical.1));
    let _ = runtime.frame(0.0);
    let wider = runtime.application().shell().layout.content.size.x;

    assert!(
        wider > wide,
        "the page did not reflow for the new size: {wider} <= {wide}"
    );
    assert!(
        (runtime.application().shell().layout.viewport.size.x - 2560.0).abs() < 1.0,
        "the shell laid out for a stale viewport"
    );
}

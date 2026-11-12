//! The showcase, driven through the generic application runtime.
//!
//! The window host no longer owns the scene diff: `AppRuntime` rebuilds the
//! frame, diffs it against the previous one and reports the upload plan. These
//! tests exercise exactly that path without a GPU.

use unbound_app::AppRuntime;
use unbound_math::Vec2;
use unbound_widgets::diff::SceneDiff;

use showcase::pages::PageId;
use showcase::runtime_app::GalleryRuntime;

fn runtime() -> AppRuntime<GalleryRuntime> {
    AppRuntime::new(GalleryRuntime::new(), Vec2::new(1280.0, 800.0))
}

#[test]
fn the_first_frame_builds_the_whole_scene() {
    let mut runtime = runtime();
    let report = runtime.frame(0.0);
    assert!(report.primitives > 0, "the shell painted nothing");
    assert_eq!(
        report.diff,
        SceneDiff::Whole,
        "the first frame has no predecessor to patch"
    );
    assert!(
        !runtime.scene().focus_order.is_empty(),
        "the tab order must reach the runtime"
    );
    assert!(
        runtime.scene().semantics.pending() > 0,
        "the semantic tree must reach the runtime"
    );
}

#[test]
fn a_frame_without_changes_uploads_nothing() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    let idle = runtime.frame(0.0);
    assert_eq!(idle.diff, SceneDiff::Identical);
    assert_eq!(idle.bytes, 0);
}

#[test]
fn navigating_rebuilds_and_reports_a_new_scene() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    runtime
        .application_mut()
        .shell_mut()
        .navigate(PageId::Telemetry);
    // A mutation the host made outside the reducer has to be announced, the
    // same way the window shell does it for zoom and window chrome.
    runtime.invalidate();
    let moved = runtime.frame(0.0);
    assert_ne!(moved.diff, SceneDiff::Identical);
    assert!(moved.primitives > 0);
}

#[test]
fn an_accessibility_click_reaches_the_shell() {
    let mut runtime = runtime();
    let _ = runtime.frame(0.0);
    // The nav rows register stable identities; clicking the Telemetry row
    // must navigate even though no pointer event ever happened.
    let node = showcase::target::id(showcase::target::Target::NavPage(
        PageId::Telemetry.index() as u8
    ));
    runtime.a11y(node, unbound_a11y::A11yAction::Click, None);
    let _ = runtime.frame(0.0);
    assert_eq!(
        runtime.application().shell().state.active_page,
        PageId::Telemetry
    );
}

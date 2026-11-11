//! The driver's own contract: pointer sequences, camera mapping, focus and
//! sanitizer plumbing. Widget behaviour is tested by the widget's harness.

use unbound_headless::driver::TestApp;
use unbound_headless::{FrameBuilder, NodeKind};
use unbound_input::{HitRegion, PointerEvent};
use unbound_math::{Rect, Vec2};

/// A session with one 200 × 40 target at `(100, 100)`.
fn app_with_target() -> TestApp {
    let mut app = TestApp::new((1280.0, 720.0));
    let region = HitRegion::new(7, 1, 0, Rect::new(100.0, 100.0, 200.0, 40.0));
    let semantics = core::mem::take(&mut app.semantics);
    app.install(vec![region], semantics);
    app
}

#[test]
fn a_click_reaches_the_node_under_the_pointer() {
    let mut app = app_with_target();
    let report = app.click(Vec2::new(200.0, 120.0));

    assert_eq!(report.pressed, Some(7));
    assert_eq!(report.clicked(), Some(7));
    assert_eq!(report.click_count(), Some(1));
    assert_eq!(app.router.hover(), Some(7));
    assert_eq!(app.generation(), 1);
}

#[test]
fn zoom_and_pan_keep_the_target_under_its_pixels() {
    let mut app = app_with_target();
    app.pan_by(Vec2::new(30.0, -10.0));
    app.zoom_at(Vec2::new(640.0, 360.0), 2.0);

    let canvas = Vec2::new(200.0, 120.0);
    let screen = app.canvas_to_screen(canvas);
    let round_trip = app.screen_to_canvas(screen);
    assert!((round_trip.x - canvas.x).abs() < 1e-3);
    assert!((round_trip.y - canvas.y).abs() < 1e-3);

    assert_eq!(app.pick(screen), Some(7));
    assert_eq!(app.click(screen).clicked(), Some(7));
}

#[test]
fn dragging_off_the_target_cancels_the_click() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(200.0, 120.0));
    app.pointer_down();
    app.move_to(Vec2::new(200.0, 500.0));
    let events = app.pointer_up();

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { .. })),
        "a release outside must not synthesize a click: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, PointerEvent::Up { target: Some(7), .. })),
        "the captured node must still receive the release: {events:?}"
    );
}

#[test]
fn cancelling_the_interaction_drops_the_capture() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(200.0, 120.0));
    app.pointer_down();
    assert_eq!(app.router.pointer().pressed(), Some(7));

    assert!(matches!(
        app.cancel(),
        Some(PointerEvent::CaptureLost { target: 7 })
    ));
    assert_eq!(app.router.pointer().pressed(), None);
    assert!(app.click(Vec2::new(200.0, 120.0)).clicked().is_some());
}

#[test]
fn installing_a_scene_preserves_focus_and_bumps_the_generation() {
    let mut app = app_with_target();
    app.set_focus_key(7);
    assert_eq!(app.semantics.focus(), 7);

    let semantics = core::mem::take(&mut app.semantics);
    app.install(Vec::new(), semantics);

    assert_eq!(app.semantics.focus(), 7);
    assert_eq!(app.generation(), 2);
}

#[test]
fn the_sanitizers_run_through_the_driver() {
    let app = app_with_target();
    let mut builder = FrameBuilder::new(Rect::new(0.0, 0.0, 1280.0, 720.0));
    builder.open(
        NodeKind::Interactive {
            min_size: NodeKind::DEFAULT_MIN_SIZE,
            occluded: false,
        },
        Rect::new(10.0, 10.0, 10.0, 10.0),
        "tiny",
    );
    builder.close();
    let report = app.check(&builder.build());
    assert!(
        !report.diagnostics.is_empty(),
        "a 10 × 10 interactive target must be reported"
    );
}

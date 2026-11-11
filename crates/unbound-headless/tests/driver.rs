//! The driver's own contract: pointer sequences, camera mapping, focus and
//! sanitizer plumbing. Widget behaviour is tested by the widget's harness.

use accesskit::Role;
use unbound_a11y::{A11yNode, Semantics};
use unbound_headless::driver::{TestApp, DEFAULT_ROOT};
use unbound_headless::{FrameBuilder, NodeKind};
use unbound_input::{HitRegion, PointerEvent};
use unbound_math::{Rect, Vec2};

fn target_rect() -> Rect {
    Rect::new(100.0, 100.0, 200.0, 40.0)
}

fn target_region() -> HitRegion {
    HitRegion::new(7, 1, 0, target_rect())
}

/// The semantic tree that matches [`target_region`].
fn target_semantics() -> Semantics {
    let mut semantics = Semantics::new(DEFAULT_ROOT);
    semantics.upsert(7, A11yNode::new(Role::Button, "target", target_rect()));
    semantics
}

/// A session with one 200 × 40 target at `(100, 100)`.
fn app_with_target() -> TestApp {
    let mut app = TestApp::new((1280.0, 720.0));
    app.install(vec![target_region()], target_semantics());
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
        events.iter().any(|event| matches!(
            event,
            PointerEvent::Up {
                target: Some(7),
                ..
            }
        )),
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

    app.install(vec![target_region()], target_semantics());

    assert_eq!(app.semantics.focus(), 7);
    assert_eq!(app.generation(), 2);

    // A node that disappears from the scene must not keep focus: the semantic
    // tree would otherwise point at an id that no longer exists.
    app.install(Vec::new(), Semantics::new(DEFAULT_ROOT));
    assert_eq!(app.focused_key(), None);
    assert_eq!(app.semantics.focus(), DEFAULT_ROOT);
}

#[test]
fn installing_new_regions_refreshes_the_hover_without_moving_the_pointer() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(200.0, 120.0));
    assert_eq!(app.router.hover(), Some(7));

    app.install(Vec::new(), target_semantics());
    assert_eq!(
        app.router.hover(),
        None,
        "the pointer stayed, but its target is no longer in the scene"
    );

    app.install(vec![target_region()], target_semantics());
    assert_eq!(
        app.router.hover(),
        Some(7),
        "the target returned under the stationary pointer"
    );
}

#[test]
fn installing_a_scene_without_the_captured_node_cancels_the_capture() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(200.0, 120.0));
    assert_eq!(app.pointer_down(), Some(7));

    let lost = app.install(Vec::new(), Semantics::new(DEFAULT_ROOT));
    assert!(matches!(
        lost,
        Some(PointerEvent::CaptureLost { target: 7 })
    ));
    assert_eq!(app.router.pointer().pressed(), None);

    let events = app.pointer_up();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { .. })),
        "a release after the target vanished must not click: {events:?}"
    );
}

#[test]
fn panning_and_zooming_repick_the_hover_under_a_stationary_pointer() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(200.0, 120.0));
    assert_eq!(app.router.hover(), Some(7));

    app.pan_by(Vec2::new(0.0, 300.0));
    assert_eq!(
        app.router.hover(),
        None,
        "the canvas moved under the pointer"
    );

    app.pan_by(Vec2::new(0.0, -300.0));
    assert_eq!(app.router.hover(), Some(7));

    app.zoom_at(Vec2::new(200.0, 120.0), 2.0);
    assert_eq!(
        app.router.hover(),
        Some(7),
        "zooming around the pointer keeps the target under it"
    );

    app.zoom_at(Vec2::new(640.0, 360.0), 2.0);
    assert_eq!(
        app.router.hover(),
        None,
        "zooming around the viewport centre moves the target away"
    );
}

#[test]
fn cancelling_after_dragging_away_clears_the_hover() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(200.0, 120.0));
    app.pointer_down();
    app.move_to(Vec2::new(200.0, 500.0));
    assert_eq!(
        app.router.hover(),
        Some(7),
        "hover is frozen while the capture is active"
    );

    assert!(matches!(
        app.cancel(),
        Some(PointerEvent::CaptureLost { target: 7 })
    ));
    assert_eq!(
        app.router.hover(),
        None,
        "the pointer is far away from the button"
    );
}

#[test]
fn a_release_just_outside_the_target_is_not_a_click() {
    let mut app = app_with_target();
    app.move_to(Vec2::new(101.0, 120.0));
    assert_eq!(app.pointer_down(), Some(7));

    // Two pixels out: below the three-pixel drag slop, so only the release
    // position can tell that this is not a click.
    app.move_to(Vec2::new(99.0, 120.0));
    let events = app.pointer_up();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { .. })),
        "releasing outside the target must not activate it: {events:?}"
    );
    assert_eq!(app.router.hover(), None);

    let report = app.click(Vec2::new(200.0, 120.0));
    assert_eq!(
        report.click_count(),
        Some(1),
        "the withheld click must not pair into a phantom double click"
    );
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

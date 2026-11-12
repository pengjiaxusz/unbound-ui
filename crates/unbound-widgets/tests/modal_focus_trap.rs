//! A modal traps the keyboard and swallows clicks aimed at the background.
//!
//! Both halves are exercised together because they are the same promise: while
//! a dialog is open, the UI behind it is inert. The focus test drives the real
//! [`FocusManager`]; the pointer test installs the real scrim hit region in a
//! [`TestApp`] and clicks through it.

use unbound_a11y::Semantics;
use unbound_headless::driver::TestApp;
use unbound_input::{FocusKey, FocusManager, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_widgets::{emit_modal, ControlState, ModalDesc};

fn viewport() -> Rect {
    Rect::new(0.0, 0.0, 1280.0, 720.0)
}

#[test]
fn tab_cannot_escape_a_modal_focus_scope() {
    let modal = emit_modal(
        100,
        viewport(),
        &ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)),
        &[10, 11, 12],
        ControlState::default(),
    );

    let mut focus = FocusManager::default();
    focus.set_order([1, 2, 3, 10, 11, 12]);
    focus.set_trap(modal.entry.focus_scope.iter().copied());

    for _ in 0..10 {
        let focused = focus
            .handle_key(FocusKey::Tab)
            .expect("modal has focusables");
        assert!([10, 11, 12].contains(&focused), "escaped to {focused}");
    }
    for _ in 0..10 {
        let focused = focus
            .handle_key(FocusKey::ShiftTab)
            .expect("modal has focusables");
        assert!([10, 11, 12].contains(&focused), "escaped to {focused}");
    }
}

#[test]
fn escape_drops_focus_but_the_trap_still_holds_the_next_tab() {
    let modal = emit_modal(
        100,
        viewport(),
        &ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)),
        &[10, 11],
        ControlState::default(),
    );
    let mut focus = FocusManager::default();
    focus.set_order([1, 2, 10, 11]);
    focus.set_trap(modal.entry.focus_scope.iter().copied());

    assert_eq!(focus.handle_key(FocusKey::Escape), None);
    let focused = focus.handle_key(FocusKey::Tab).expect("trap is populated");
    assert!([10, 11].contains(&focused), "escaped to {focused}");
}

#[test]
fn clicking_the_scrim_never_reaches_the_background_button() {
    const BUTTON: u64 = 7;
    let modal = emit_modal(
        100,
        viewport(),
        &ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)).with_label("Settings"),
        &[10, 11],
        ControlState::default(),
    );

    let mut app = TestApp::new((1280.0, 720.0));
    let button = HitRegion::new(BUTTON, 1, 0, Rect::new(20.0, 20.0, 200.0, 40.0));
    let card = modal.output.hit.clone().expect("modal card is pickable");
    let scrim = modal.scrim.clone();
    app.install(vec![button, scrim, card], Semantics::new(1));

    // A point over the background button but outside the dialog card.
    let report = app.click(Vec2::new(100.0, 40.0));
    assert_ne!(
        report.clicked(),
        Some(BUTTON),
        "the scrim must intercept the click"
    );
    assert_eq!(report.clicked(), Some(modal.scrim.node));
}

#[test]
fn clicking_inside_the_card_targets_the_card_not_the_background() {
    const BUTTON: u64 = 7;
    let modal = emit_modal(
        100,
        viewport(),
        &ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)),
        &[],
        ControlState::default(),
    );
    let mut app = TestApp::new((1280.0, 720.0));
    let button = HitRegion::new(BUTTON, 1, 0, Rect::new(20.0, 20.0, 200.0, 40.0));
    let card = modal.output.hit.clone().expect("modal card is pickable");
    let scrim = modal.scrim.clone();
    app.install(vec![button, scrim, card], Semantics::new(1));

    let report = app.click(Vec2::new(640.0, 360.0));
    assert_eq!(report.clicked(), Some(100));
}

//! 模态框捕获键盘焦点并吞掉指向背景的点击测试。
//!
//! 两部分一起测试，因为它们是同一个契约：对话框打开时，其后的界面必须处于非活动状态。
//! 焦点测试驱动真实的 [`FocusManager`]；指针测试在 [`TestApp`] 中安装真实遮罩命中区域并穿透点击。
//!
//!

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

    // 位于背景按钮上方但在对话框卡片之外的点。
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

#[test]
fn opening_a_modal_saves_the_focused_key_and_closing_restores_it() {
    use unbound_input::FocusCause;
    use unbound_widgets::modal::ModalFocus;

    let mut focus = FocusManager::default();
    focus.set_order([1, 2, 3]);
    focus.focus(2, FocusCause::Keyboard);
    assert_eq!(focus.focused(), Some(2));

    let mut stack = ModalFocus::new();
    stack.open(&mut focus, &[10, 11]);
    assert_eq!(stack.saved(), &[Some(2)], "the previous key is remembered");
    assert!(focus.is_trapped(10), "the dialog trap is installed");
    assert!(focus.is_trapped(11));

    assert_eq!(stack.close(&mut focus), Some(2));
    assert!(focus.trap().is_none(), "the trap is gone");
    assert_eq!(focus.focused(), Some(2), "focus returns to the trigger");
    assert_eq!(stack.depth(), 0);
}

#[test]
fn nested_modals_restore_the_previous_dialog_focus() {
    use unbound_input::FocusCause;
    use unbound_widgets::modal::ModalFocus;

    let mut focus = FocusManager::default();
    focus.set_order([1, 2, 10, 11]);
    focus.focus(2, FocusCause::Keyboard);

    let mut stack = ModalFocus::new();
    stack.open(&mut focus, &[10, 11]);
    assert!(focus.focused().is_some_and(|key| [10, 11].contains(&key)));
    stack.open(&mut focus, &[11]);
    assert_eq!(stack.depth(), 2);

    assert_eq!(stack.close(&mut focus), focus.focused());
    assert_eq!(stack.depth(), 1);
    assert_eq!(stack.close(&mut focus), Some(2));
    assert_eq!(focus.focused(), Some(2));
}

#[test]
fn the_close_button_sits_in_the_title_bar_and_paints_two_strokes() {
    let plain = ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0));
    assert!(plain.close_button_rect().is_none(), "opt-in only");
    let plain_output = emit_modal(1, viewport(), &plain, &[], ControlState::default());
    assert_eq!(plain_output.output.primitives.len(), 4);

    let dialog = ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)).with_close_button();
    let button = dialog.close_button_rect().expect("enabled");
    assert_eq!(button.size, Vec2::new(24.0, 24.0));
    assert_eq!(button.max().x, 940.0 - 12.0, "inset from the trailing edge");
    assert!(
        dialog.title_bar_rect().contains_rect(&button),
        "the button centres in the title bar"
    );

    let output = emit_modal(1, viewport(), &dialog, &[], ControlState::default());
    assert_eq!(
        output.output.primitives.len(),
        6,
        "card + two cross strokes"
    );
    assert_eq!(output.output.payloads.len(), 2);
}

#[test]
fn footer_actions_are_right_aligned_and_fit_the_footer() {
    let dialog = ModalDesc::new(Rect::new(100.0, 100.0, 400.0, 300.0)).with_footer(56.0);
    let footer = dialog.footer_rect().expect("footer configured");
    assert_eq!(footer.max().y, dialog.rect.max().y);
    assert_eq!(footer.height(), 56.0);

    let actions = dialog.footer_actions(3);
    assert_eq!(actions.len(), 3);
    assert!(
        actions
            .windows(2)
            .all(|pair| pair[0].max().x < pair[1].origin.x),
        "actions read left to right"
    );
    assert!(actions.iter().all(|action| action.size.y == 32.0));
    assert!(
        actions.iter().all(|action| footer.contains_rect(action)),
        "every action stays inside the footer"
    );
    assert!(dialog.footer_actions(0).is_empty());
    assert!(ModalDesc::new(dialog.rect).footer_rect().is_none());
}

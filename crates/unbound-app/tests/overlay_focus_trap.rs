//! OverlayManager（S5.2）与 FocusGroup 漫游 tabindex（S5.1）集成测试。

use unbound_app::{
    AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome, FocusGroup,
    FocusGroupMode, Modifiers, RovingOrientation,
};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_widgets::button::emit_button;
use unbound_widgets::overlay::OverlayEntry;
use unbound_widgets::ButtonDesc;

const TRIGGER: u64 = 1;
const TAB_A: u64 = 2;
const TAB_B: u64 = 3;
const TAB_C: u64 = 4;
const MODAL_NODE: u64 = 100;
const MODAL_OK: u64 = 101;
const MODAL_CANCEL: u64 = 102;
const POPOVER_NODE: u64 = 200;

#[derive(Default)]
struct OverlayApp {
    show_modal: bool,
}

impl Application for OverlayApp {
    type State = bool;

    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {
        self.show_modal
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut unbound_app::ViewBuilder) {
        for (idx, id) in [TRIGGER, TAB_A, TAB_B, TAB_C].into_iter().enumerate() {
            let desc = ButtonDesc::new(
                Rect::new(20.0 + idx as f32 * 100.0, 20.0, 90.0, 32.0),
                "Item",
            );
            let ctrl = view.ctx.control_state(id, false, false);
            view.push(id, emit_button(id, &desc, ctrl, &mut []));
        }
        view.register_focus_group(FocusGroup::new(
            500,
            FocusGroupMode::Linear(RovingOrientation::Horizontal),
            vec![TAB_A, TAB_B, TAB_C],
        ));
        if *state {
            for (idx, id) in [MODAL_OK, MODAL_CANCEL].into_iter().enumerate() {
                let desc = ButtonDesc::new(
                    Rect::new(200.0 + idx as f32 * 100.0, 200.0, 80.0, 32.0),
                    "ModalBtn",
                );
                let ctrl = view.ctx.control_state(id, false, false);
                view.push(id, emit_button(id, &desc, ctrl, &mut []));
            }
        }
    }
}

#[test]
fn modal_traps_focus_and_escape_restores_trigger_focus() {
    let mut rt = AppRuntime::new(OverlayApp::default(), Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    // Tab 切换至 TRIGGER
    rt.key(AppKey::Tab, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(TRIGGER));

    // 打开模态框，将焦点限制在 [MODAL_OK, MODAL_CANCEL]
    *rt.state_mut() = true;
    rt.frame(1.0 / 60.0);
    rt.open_overlay(OverlayEntry::modal(
        MODAL_NODE,
        Rect::new(180.0, 160.0, 300.0, 200.0),
        vec![MODAL_OK, MODAL_CANCEL],
    ));
    assert!(rt.overlays().is_modal_active());
    assert_eq!(rt.context().focused, Some(MODAL_OK));

    // Tab 保持在模态框焦点陷阱内部
    rt.key(AppKey::Tab, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(MODAL_CANCEL));
    rt.key(AppKey::Tab, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(MODAL_OK));

    // 按 Escape 关闭模态框并将焦点恢复至 TRIGGER
    rt.key(AppKey::Escape, true, Modifiers::default());
    assert!(!rt.overlays().is_modal_active());
    assert_eq!(rt.context().focused, Some(TRIGGER));
}

#[test]
fn outside_click_dismisses_anchored_overlay_and_roving_arrows_navigate_group() {
    let mut rt = AppRuntime::new(OverlayApp::default(), Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    // 按两次 Tab -> TAB_A，再按右箭头 -> TAB_B -> TAB_C -> End/Home
    rt.key(AppKey::Tab, true, Modifiers::default());
    rt.key(AppKey::Tab, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(TAB_A));
    rt.key(AppKey::Right, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(TAB_B));
    rt.key(AppKey::End, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(TAB_C));
    rt.key(AppKey::Home, true, Modifiers::default());
    assert_eq!(rt.context().focused, Some(TAB_A));

    // 打开锚定弹出框并点击外部以轻关闭
    rt.open_overlay(OverlayEntry::popover(
        POPOVER_NODE,
        TAB_A,
        Rect::new(120.0, 60.0, 180.0, 120.0),
    ));
    assert!(rt.overlays().stack().contains(POPOVER_NODE));

    rt.pointer_down(Vec2::new(10.0, 10.0), PointerButton::Left);
    assert!(!rt.overlays().stack().contains(POPOVER_NODE));
}

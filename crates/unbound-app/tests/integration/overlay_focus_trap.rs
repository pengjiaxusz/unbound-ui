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

#[test]
fn escape_and_outside_click_dispatch_overlay_dismissed_receipt() {
    #[derive(Default)]
    struct DismissReceiptApp {
        dismissed: Vec<u64>,
    }

    impl Application for DismissReceiptApp {
        type State = ();

        fn init(&mut self, _ctx: &mut unbound_app::AppContext) {}

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            event: AppEvent,
            _ctx: &mut unbound_app::AppContext,
        ) -> EventOutcome {
            if let AppEvent::OverlayDismissed { node } = event {
                self.dismissed.push(node);
                return EventOutcome::new(DirtyFlags::PAINT);
            }
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, _state: &Self::State, view: &mut unbound_app::ViewBuilder) {
            view.button(
                TRIGGER,
                ButtonDesc::new(Rect::new(20.0, 20.0, 90.0, 32.0), "Trigger"),
            );
        }
    }

    let mut rt = AppRuntime::new(DismissReceiptApp::default(), Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    // 1. Escape 关闭浮层后应向应用派发 OverlayDismissed 回执。
    rt.open_overlay(OverlayEntry::modal(
        MODAL_NODE,
        Rect::new(180.0, 160.0, 300.0, 200.0),
        vec![],
    ));
    rt.key(AppKey::Escape, true, Modifiers::default());
    assert!(!rt.overlays().stack().contains(MODAL_NODE));
    assert_eq!(rt.app().dismissed, vec![MODAL_NODE]);

    // 2. 点击浮层外部轻关闭后同样应派发 OverlayDismissed 回执。
    rt.open_overlay(OverlayEntry::popover(
        POPOVER_NODE,
        TRIGGER,
        Rect::new(200.0, 200.0, 180.0, 120.0),
    ));
    rt.pointer_down(Vec2::new(10.0, 10.0), PointerButton::Left);
    assert!(!rt.overlays().stack().contains(POPOVER_NODE));
    assert_eq!(rt.app().dismissed, vec![MODAL_NODE, POPOVER_NODE]);
}

#[test]
fn context_menu_anchored_to_tab_sub_hit_survives_same_frame_prune_and_closes_on_escape() {
    use unbound_widgets::context_menu::ContextClose;
    use unbound_widgets::{
        ContextMenuState, MenuDesc, MenuItem, TabItem, TabViewStyle, TabWidthMode, TabsDesc,
    };

    const TABS_KEY: u64 = 7100;
    const MENU_KEY: u64 = 7200;

    struct TabContextMenuApp {
        tabs: TabsDesc,
        menu: ContextMenuState,
        dismissed: Vec<u64>,
    }

    impl Application for TabContextMenuApp {
        type State = ();

        fn init(&mut self, _ctx: &mut unbound_app::AppContext) {}

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            event: AppEvent,
            _ctx: &mut unbound_app::AppContext,
        ) -> EventOutcome {
            match event {
                AppEvent::ContextMenu { node, pos } if !self.menu.is_open() => {
                    self.menu.open_at(pos, node, None);
                    EventOutcome::stopping(DirtyFlags::ALL)
                }
                AppEvent::OverlayDismissed { node } if node == MENU_KEY => {
                    self.menu.close(ContextClose::Escape);
                    self.dismissed.push(node);
                    EventOutcome::new(DirtyFlags::ALL)
                }
                _ => EventOutcome::new(DirtyFlags::NONE),
            }
        }

        fn view(&mut self, _state: &Self::State, view: &mut unbound_app::ViewBuilder) {
            view.tabs(TABS_KEY, self.tabs.clone());
            let menu_desc = MenuDesc::context_menu(
                Rect::new(0.0, 0.0, 180.0, 0.0),
                vec![MenuItem::action("Close Tab"), MenuItem::action("Duplicate")],
            );
            view.context_menu(MENU_KEY, menu_desc, &self.menu);
        }
    }

    let tabs = TabsDesc::new(Rect::new(0.0, 0.0, 480.0, 36.0), &[])
        .with_items(vec![TabItem::new(1, "Alpha"), TabItem::new(2, "Beta")])
        .with_style(TabViewStyle::ShellChrome)
        .with_width_mode(TabWidthMode::adaptive_default());

    let app = TabContextMenuApp {
        tabs: tabs.clone(),
        menu: ContextMenuState::new(),
        dismissed: Vec::new(),
    };
    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    let tabs_rect = rt.widget_rect(TABS_KEY).expect("tabs mounted rect");
    let mut laid = tabs;
    laid.rect = tabs_rect;
    let tab_point = laid.label_rect(1, 0.0).center();

    // 右键点击第二个标签页子热区并打开上下文菜单。
    rt.pointer_move(tab_point);
    rt.pointer_down(tab_point, PointerButton::Right);
    rt.pointer_up(tab_point, PointerButton::Right);
    assert!(rt.app().menu.is_open());
    assert_ne!(rt.app().menu.target, Some(TABS_KEY));

    // 下一帧执行声明式浮层同步：锚点为子热区键，应经 hit_owners 折算宿主键后存活。
    rt.frame(1.0 / 60.0);
    assert!(rt.overlays().stack().contains(MENU_KEY));

    // 按 Escape 关闭菜单：浮层移除且业务侧收到 OverlayDismissed 回执并清理打开态。
    rt.key(AppKey::Escape, true, Modifiers::default());
    assert!(!rt.overlays().stack().contains(MENU_KEY));
    assert!(!rt.app().menu.is_open());
    assert_eq!(rt.app().dismissed, vec![MENU_KEY]);
}

#[test]
fn side_buttons_back_and_forward_dispatch_to_application() {
    #[derive(Default)]
    struct SideButtonApp {
        seen: Vec<(&'static str, PointerButton)>,
    }

    impl Application for SideButtonApp {
        type State = ();

        fn init(&mut self, _ctx: &mut unbound_app::AppContext) {}

        fn reduce(
            &mut self,
            _state: &mut Self::State,
            event: AppEvent,
            _ctx: &mut unbound_app::AppContext,
        ) -> EventOutcome {
            match event {
                AppEvent::PointerDown { button, .. }
                    if matches!(button, PointerButton::Back | PointerButton::Forward) =>
                {
                    self.seen.push(("down", button));
                    EventOutcome::stopping(DirtyFlags::PAINT)
                }
                AppEvent::PointerUp { button, .. }
                    if matches!(button, PointerButton::Back | PointerButton::Forward) =>
                {
                    self.seen.push(("up", button));
                    EventOutcome::stopping(DirtyFlags::PAINT)
                }
                _ => EventOutcome::new(DirtyFlags::NONE),
            }
        }

        fn view(&mut self, _state: &Self::State, view: &mut unbound_app::ViewBuilder) {
            view.button(
                TRIGGER,
                ButtonDesc::new(Rect::new(20.0, 20.0, 120.0, 36.0), "Target"),
            );
        }
    }

    let mut rt = AppRuntime::new(SideButtonApp::default(), Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    let pos = Vec2::new(50.0, 36.0);
    rt.pointer_move(pos);
    rt.pointer_down(pos, PointerButton::Back);
    rt.pointer_up(pos, PointerButton::Back);
    rt.pointer_down(pos, PointerButton::Forward);
    rt.pointer_up(pos, PointerButton::Forward);

    assert_eq!(
        rt.app().seen,
        vec![
            ("down", PointerButton::Back),
            ("up", PointerButton::Back),
            ("down", PointerButton::Forward),
            ("up", PointerButton::Forward),
        ]
    );
}

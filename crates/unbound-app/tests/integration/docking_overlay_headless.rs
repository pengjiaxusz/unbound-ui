//! 跨窗停靠预览叠加层与集中式 `MotionPolicy` 无头集成测试（#282）。

use std::cell::RefCell;
use std::rc::Rc;

use unbound_app::prelude::*;
use unbound_platform::WindowConfig;
use unbound_widgets::button::emit_button;

fn win_cfg(title: &str) -> WindowConfig {
    let mut cfg = WindowConfig::default().with_title(title);
    cfg.width = 800;
    cfg.height = 600;
    cfg
}

const BTN_KEY: u64 = 10;
const POPOVER_KEY: u64 = 200;
const DOCK_OVERLAY_KEY: u64 = 9001;

#[derive(Clone, Default)]
struct ObservedFrame {
    last_policy: Option<MotionPolicy>,
    overlay_alpha: f32,
}

struct DockingTestApp {
    overlay_state: Rc<RefCell<DockOverlayState>>,
    observed: Rc<RefCell<ObservedFrame>>,
    inject_dirty_hit: bool,
}

impl Default for DockingTestApp {
    fn default() -> Self {
        Self {
            overlay_state: Rc::new(RefCell::new(DockOverlayState::new())),
            observed: Rc::new(RefCell::new(ObservedFrame::default())),
            inject_dirty_hit: false,
        }
    }
}

impl Application for DockingTestApp {
    type State = bool;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        false
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, show_popover: &Self::State, view: &mut ViewBuilder) {
        self.observed.borrow_mut().last_policy = Some(view.ctx.motion_policy);

        let desc = ButtonDesc::new(Rect::new(24.0, 24.0, 120.0, 36.0), "Action");
        let ctrl = view.ctx.control_state(BTN_KEY, false, false);
        view.push(BTN_KEY, emit_button(BTN_KEY, &desc, ctrl, &mut []));

        if *show_popover {
            let entry =
                OverlayEntry::popover(POPOVER_KEY, BTN_KEY, Rect::new(160.0, 24.0, 180.0, 90.0));
            view.overlay_layer(POPOVER_KEY, entry, |_| {});
        }

        let bounds = Rect::new(0.0, 0.0, view.ctx.viewport.x, view.ctx.viewport.y);
        let pointer = Vec2::new(200.0, 200.0);
        if self.inject_dirty_hit {
            let mut out = emit_docking_overlay(
                &self.overlay_state.borrow(),
                bounds,
                pointer,
                &[],
                DockOverlayTokens::from_theme(),
                0,
            );
            // 故意注入命中、焦点与语义节点，验证 `ViewBuilder::docking_overlay` 强制剥离。
            out.hit = Some(unbound_input::HitRegion::new(
                DOCK_OVERLAY_KEY,
                unbound_input::PORTAL_LAYER,
                999,
                bounds,
            ));
            out.focusable = true;
            out.a11y = Some(unbound_a11y::A11yNode::new(
                Role::GenericContainer,
                "DirtyOverlay",
                bounds,
            ));
            view.docking_overlay(DOCK_OVERLAY_KEY, out);
        } else {
            view.docking_overlay_state(
                DOCK_OVERLAY_KEY,
                &self.overlay_state.borrow(),
                bounds,
                pointer,
                &[],
                DockOverlayTokens::from_theme(),
            );
        }

        self.observed.borrow_mut().overlay_alpha = self.overlay_state.borrow().preview_alpha();
    }
}

fn single_pane_window(window_id: u64, rect: Rect) -> DockCandidateWindow {
    DockCandidateWindow {
        window_id,
        window_rect: rect,
        z_order: 1,
        root_edge_band: DEFAULT_ROOT_EDGE_BAND_PX,
        panes: vec![DockCandidatePane {
            pane_id: window_id * 100 + 1,
            rect,
            tab_bar_height: 32.0,
            tab_rects: vec![Rect::new(
                rect.origin.x + 8.0,
                rect.origin.y + 4.0,
                80.0,
                24.0,
            )],
            allow_center: true,
            allow_nest: true,
            is_source_group: false,
        }],
    }
}

#[test]
fn motion_policy_passthrough_and_reduced_motion_snaps_overlay_and_theme() {
    let app = DockingTestApp::default();
    let overlay_state = Rc::clone(&app.overlay_state);
    let observed = Rc::clone(&app.observed);

    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));
    rt.set_reduced_motion(false);
    rt.set_motion_speed_scale(1.5);
    rt.frame(0.0);

    let policy = observed.borrow().last_policy.expect("view should run");
    assert!(!policy.reduced_motion);
    assert!((policy.speed_scale() - 1.5).abs() < 1e-4);

    // 正常动效下，`MotionToken::OverlayPreview` (120ms) 推进 16ms 不会立即满值。
    let win = single_pane_window(1, Rect::new(0.0, 0.0, 800.0, 600.0));
    {
        let mut st = overlay_state.borrow_mut();
        st.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(400.0, 300.0));
        st.update(
            Vec2::new(400.0, 300.0),
            std::slice::from_ref(&win),
            0.016,
            rt.motion_policy(),
        );
        assert!(st.preview_alpha() > 0.0 && st.preview_alpha() < 1.0);
    }

    // 切换为减弱动画 `reduced_motion = true` 后，实时透传至 ViewContext 并单步瞬切至 1.0。
    rt.set_reduced_motion(true);
    rt.frame(0.016);
    assert!(observed.borrow().last_policy.unwrap().reduced_motion);
    {
        let mut st = overlay_state.borrow_mut();
        st.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(400.0, 300.0));
        st.update(Vec2::new(400.0, 300.0), &[win], 0.016, rt.motion_policy());
        assert!((st.preview_alpha() - 1.0).abs() < 1e-6);
    }

    // MultiWindowCoordinator 广播 `MotionPolicy` 并驱动无头主题过渡瞬切。
    let primary_cfg = win_cfg("Primary");
    let mut coord =
        MultiWindowCoordinator::new(primary_cfg.clone(), |_id: WindowId, _cfg: &WindowConfig| {
            DockingTestApp::default()
        });
    coord.register_headless_window(WindowId(1), primary_cfg.clone());
    coord.register_headless_window(WindowId(2), primary_cfg);
    coord.set_reduced_motion_all(true);

    for id in [WindowId(1), WindowId(2)] {
        let ctx = coord.registry().get(id).expect("window exists");
        assert!(ctx.runtime().motion_policy().reduced_motion);
    }
}

#[test]
fn multi_window_screen_origin_coordinate_routing_and_candidate_arbitration() {
    let primary_cfg = win_cfg("Win1");
    let secondary_cfg = win_cfg("Win2");
    let mut coord =
        MultiWindowCoordinator::new(primary_cfg.clone(), |_id: WindowId, _cfg: &WindowConfig| {
            DockingTestApp::default()
        });
    coord.register_headless_window(WindowId(1), primary_cfg);
    coord.register_headless_window(WindowId(2), secondary_cfg);

    assert!(coord.set_window_screen_origin(WindowId(1), (0, 0)));
    assert!(coord.set_window_screen_origin(WindowId(2), (900, 100)));
    assert_eq!(coord.window_screen_origin(WindowId(2)), Some((900, 100)));

    // 从 Win1 局部坐标 `(1000, 250)` 出发（即桌面物理 `(1000, 250)`），应命中 Win2 局部 `(100, 150)`。
    let hit = coord
        .route_cross_window_drag(WindowId(1), Vec2::new(1000.0, 250.0))
        .expect("source window exists");
    assert_eq!(
        hit,
        CrossWindowDragHit::Window {
            window_id: WindowId(2),
            local_logical: Vec2::new(100.0, 150.0),
            screen_physical: Vec2::new(1000.0, 250.0),
            screen_logical: Vec2::new(1000.0, 250.0),
        }
    );

    // 驱动跨窗候选仲裁：在 Win2 内创建局部面板，协调器自动偏移至桌面坐标并命中 Win2。
    let mut overlay = DockOverlayState::new();
    let mut proxy = DockDragProxy::default();
    overlay.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(100.0, 100.0));
    let _ = coord.update_docking_overlay(
        &mut overlay,
        &mut proxy,
        WindowId(1),
        Vec2::new(1000.0, 250.0),
        0.016,
        |id, ctx| {
            let vp = ctx.runtime().context().viewport;
            vec![DockCandidatePane {
                pane_id: id.0 * 10 + 1,
                rect: Rect::new(0.0, 0.0, vp.x, vp.y),
                tab_bar_height: 32.0,
                tab_rects: Vec::new(),
                allow_center: true,
                allow_nest: true,
                is_source_group: false,
            }]
        },
    );

    let target = overlay.active_target().expect("should hit Win2 target");
    assert_eq!(target.window_id, 2);
    assert!(overlay.is_preview_visible());
    assert!(!proxy.visible);
}

#[test]
fn nudge_level_wheel_drilldown_switches_nested_candidates() {
    let primary_cfg = win_cfg("Win1");
    let mut coord =
        MultiWindowCoordinator::new(primary_cfg.clone(), |_id: WindowId, _cfg: &WindowConfig| {
            DockingTestApp::default()
        });
    coord.register_headless_window(WindowId(1), primary_cfg);
    assert!(coord.set_window_screen_origin(WindowId(1), (0, 0)));

    let mut overlay = DockOverlayState::new();
    let mut proxy = DockDragProxy::default();
    overlay.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(12.0, 300.0));

    // 指针位于左侧 12px 处：同时处于面板左缘吸附区与整窗外缘吸附带，存在多层候选。
    let _ = coord.update_docking_overlay(
        &mut overlay,
        &mut proxy,
        WindowId(1),
        Vec2::new(12.0, 300.0),
        0.016,
        |id, ctx| {
            let vp = ctx.runtime().context().viewport;
            vec![DockCandidatePane {
                pane_id: id.0 * 10 + 1,
                rect: Rect::new(0.0, 0.0, vp.x, vp.y),
                tab_bar_height: 32.0,
                tab_rects: Vec::new(),
                allow_center: true,
                allow_nest: true,
                is_source_group: false,
            }]
        },
    );

    assert!(overlay.candidates().len() >= 2);
    assert_eq!(overlay.level_index(), 0);
    let inner_kind = overlay.active_target().unwrap().kind;

    overlay.nudge_level(1);
    assert_eq!(overlay.level_index(), 1);
    let outer_kind = overlay.active_target().unwrap().kind;
    assert_ne!(inner_kind, outer_kind);

    overlay.nudge_level(-1);
    assert_eq!(overlay.level_index(), 0);
}

#[test]
fn tear_out_outside_all_windows_activates_drag_proxy_and_hides_host_preview() {
    let primary_cfg = win_cfg("Win1");
    let secondary_cfg = win_cfg("Win2");
    let mut coord =
        MultiWindowCoordinator::new(primary_cfg.clone(), |_id: WindowId, _cfg: &WindowConfig| {
            DockingTestApp::default()
        });
    coord.register_headless_window(WindowId(1), primary_cfg);
    coord.register_headless_window(WindowId(2), secondary_cfg);
    assert!(coord.set_window_screen_origin(WindowId(1), (0, 0)));
    assert!(coord.set_window_screen_origin(WindowId(2), (900, 0)));

    let mut overlay = DockOverlayState::new();
    let mut proxy = DockDragProxy::default();
    overlay.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(850.0, 200.0));

    // `(850, 200)` 落在两窗间隙（窗外自由空间），触发 `TearOut` 并点亮外部拖拽代理。
    let hit = coord
        .update_docking_overlay(
            &mut overlay,
            &mut proxy,
            WindowId(1),
            Vec2::new(850.0, 200.0),
            0.016,
            |_, _| Vec::new(),
        )
        .expect("hit result");

    assert_eq!(
        hit,
        CrossWindowDragHit::TearOut {
            screen_physical: Vec2::new(850.0, 200.0),
            screen_logical: Vec2::new(850.0, 200.0),
        }
    );
    assert_eq!(
        overlay.active_target().map(|t| t.kind),
        Some(DockDropKind::TearOut)
    );
    assert!(!overlay.is_preview_visible());
    assert!(proxy.visible);
    assert_eq!(proxy.position, Vec2::new(850.0, 200.0));
}

#[test]
fn view_builder_docking_overlay_bypasses_overlay_manager_with_zero_hit_zero_focus_and_no_light_dismiss(
) {
    let app = DockingTestApp {
        inject_dirty_hit: true,
        ..Default::default()
    };
    let win = single_pane_window(1, Rect::new(0.0, 0.0, 800.0, 600.0));
    {
        let mut st = app.overlay_state.borrow_mut();
        st.begin_drag(DockDragPayloadKind::Tab, Some(1), Vec2::new(200.0, 200.0));
        st.update(
            Vec2::new(200.0, 200.0),
            &[win],
            0.016,
            MotionPolicy::default().with_reduced_motion(true),
        );
    }

    let mut rt = AppRuntime::new(app, Vec2::new(800.0, 600.0));
    rt.frame(0.0);
    rt.open_overlay(OverlayEntry::popover(
        POPOVER_KEY,
        BTN_KEY,
        Rect::new(160.0, 24.0, 180.0, 90.0),
    ));

    // 1. 停靠叠加层发射了高 Z 序视觉图元，但绝不进入 `OverlayManager` 栈。
    assert!(rt
        .scene()
        .primitives
        .iter()
        .any(|p| p.depth_u16() >= unbound_abi::PORTAL_Z_BASE as u16));
    assert!(!rt
        .overlays()
        .stack()
        .entries()
        .iter()
        .any(|e| e.node == DOCK_OVERLAY_KEY));

    // 2. 即使上游输出携带了 `hit` / `focusable` / `a11y`，挂载路径也已强制剥离。
    assert!(!rt
        .scene()
        .regions
        .iter()
        .any(|r| r.node == DOCK_OVERLAY_KEY));
    assert!(!rt.scene().focus_order.contains(&DOCK_OVERLAY_KEY));
    assert!(rt.semantics().get(DOCK_OVERLAY_KEY).is_none());

    // 3. `Tab` 焦点只落在正常按钮上，不会抢占至停靠叠加层。
    rt.key(AppKey::Tab, true, Modifiers::default());
    assert_ne!(rt.context().focused, Some(DOCK_OVERLAY_KEY));

    // 4. 点击空白处触发 `Popover` 的 `light_dismiss`，停靠叠加层仍保持可见图元且不受轻关闭影响。
    assert!(rt
        .overlays()
        .stack()
        .entries()
        .iter()
        .any(|e| e.node == POPOVER_KEY));
    rt.pointer_down(Vec2::new(700.0, 500.0), PointerButton::Left);
    rt.frame(0.016);
    assert!(!rt
        .overlays()
        .stack()
        .entries()
        .iter()
        .any(|e| e.node == POPOVER_KEY));
    assert!(rt
        .scene()
        .primitives
        .iter()
        .any(|p| p.depth_u16() >= unbound_abi::PORTAL_Z_BASE as u16));
}

//! 统一事件传播与冒泡回归测试：
//! - 离散指针事件沿 `UiWorld` 逻辑父链冒泡与 `Propagation::Stop` 截断
//! - 内置有状态控件消费 `PointerDown` 后自动阻止继续冒泡
//! - Portal / 模态边界阻断事件渗透至背景页面
//! - 高 `z_index` 的普通容器不再被误判为浮层边界
//! - 多部件控件的合成子命中键经归属表接回逻辑父链
//! - 键盘焦点链优先于 `OverlayManager::handle_escape` 冒泡，并通过 `ctx.event_target()` 暴露当前祖先
//! - `Inspector` 对指针 / 键盘 / 滚轮事件统一记录轨迹
//! - 标题栏冒泡兜底产出 `WindowCommand`，且可被应用层拦截、被浮层隔离

use unbound_abi::{Dim, RawNodeDesc};
use unbound_app::{
    ui_key, AppContext, AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome,
    Modifiers, TitleBarSlots, ViewBuilder, WindowCommand,
};
use unbound_input::{HitRegion, PointerButton};
use unbound_math::{Rect, Vec2};
use unbound_widgets::ergonomics::{TextCtx, UiWidget};
use unbound_widgets::{
    BadgeDesc, ButtonDesc, ButtonVariant, CheckboxDesc, ControlState, OverlayEntry, OverlayKind,
    TextInputDesc, TitleBarDesc, WidgetOutput, TITLE_BAR_CLOSE_ID, TITLE_BAR_MAXIMIZE_ID,
    TITLE_BAR_MINIMIZE_ID,
};

const BG_PAGE: u64 = 100;
const CARD: u64 = 101;
const INNER_BTN: u64 = 102;
const PORTAL_ROOT: u64 = 200;
const PORTAL_PANEL: u64 = 201;
const PORTAL_INPUT: u64 = 202;
const SUB_WIDGET: u64 = 300;
const SUB_HIT: u64 = SUB_WIDGET ^ (1 << 63);

/// 发射一条与自身位置重叠、但使用合成键的子命中区域，模拟分页项 / 表格行 /
/// 滚动条滑块等多部件控件的 `sub_hits`。这类键不在世界树中，必须靠归属表接回父链。
#[derive(Clone, Debug)]
struct SubHitWidget;

impl UiWidget for SubHitWidget {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(200.0, 60.0)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut out = WidgetOutput {
            hit: Some(HitRegion::ui(node, 1, rect)),
            ..Default::default()
        };
        let child = Rect::new(rect.origin.x, rect.origin.y, 80.0, rect.size.y);
        out.sub_hits.push(HitRegion::ui(node ^ (1 << 63), 2, child));
        out
    }
}

#[derive(Default)]
struct BubbleProbe {
    stop_at_inner_click: bool,
    stop_escape_in_input: bool,
    show_portal: bool,
    /// 是否把浮层根声明为事件边界；为 false 时仅抬高 `z_index`。
    portal_is_boundary: bool,
    show_sub_hit: bool,
    clicks: Vec<u64>,
    pointer_downs: Vec<Option<u64>>,
    wheels: Vec<Option<u64>>,
    keys: Vec<(Option<u64>, AppKey)>,
}

struct BubbleApp;

impl Application for BubbleApp {
    type State = BubbleProbe;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        BubbleProbe::default()
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::PointerDown { target, .. } => {
                state.pointer_downs.push(target);
                EventOutcome::new(DirtyFlags::NONE)
            }
            AppEvent::Click { node, .. } => {
                state.clicks.push(node);
                if node == INNER_BTN && state.stop_at_inner_click {
                    return EventOutcome::stopping(DirtyFlags::PAINT);
                }
                EventOutcome::new(DirtyFlags::PAINT)
            }
            AppEvent::Wheel { target, .. } => {
                state.wheels.push(target);
                EventOutcome::new(DirtyFlags::NONE)
            }
            AppEvent::Key { key, pressed, .. } if pressed => {
                state.keys.push((ctx.event_target(), key));
                if ctx.event_target() == Some(PORTAL_INPUT)
                    && key == AppKey::Escape
                    && state.stop_escape_in_input
                {
                    return EventOutcome::stopping(DirtyFlags::PAINT);
                }
                EventOutcome::new(DirtyFlags::NONE)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.container(
            BG_PAGE,
            RawNodeDesc::container()
                .column()
                .width(Dim::px(600.0))
                .height(Dim::px(400.0)),
            |page| {
                page.container(
                    CARD,
                    RawNodeDesc::container()
                        .column()
                        .width(Dim::px(300.0))
                        .height(Dim::px(200.0))
                        .padding(unbound_abi::Edges::all(16.0)),
                    |card| {
                        // 复选框是有状态控件，其 `reduce` 消费 PointerDown 并返回 `true`，
                        // 运行时据此判定 `Propagation::Stop`，截断向祖先的继续冒泡。
                        card.checkbox(INNER_BTN, CheckboxDesc::labeled("Inner", false));
                    },
                );

                if state.show_sub_hit {
                    page.add_with_layout(
                        SUB_WIDGET,
                        RawNodeDesc::container()
                            .width(Dim::px(200.0))
                            .height(Dim::px(60.0)),
                        SubHitWidget,
                    );
                }

                if state.show_portal {
                    let portal_desc = RawNodeDesc::container()
                        .absolute()
                        .size(600.0, 400.0)
                        .z_index(1200);
                    let portal_desc = if state.portal_is_boundary {
                        portal_desc.portal_root()
                    } else {
                        portal_desc
                    };
                    page.container(PORTAL_ROOT, portal_desc, |portal| {
                        portal.container(
                            PORTAL_PANEL,
                            RawNodeDesc::container()
                                .column()
                                .width(Dim::px(240.0))
                                .height(Dim::px(140.0)),
                            |panel| {
                                panel.button(
                                    PORTAL_INPUT,
                                    ButtonDesc::label("ModalAction")
                                        .variant(ButtonVariant::Secondary),
                                );
                            },
                        );
                    });
                }
            },
        );
    }
}

#[test]
fn click_bubbles_along_logical_parents_and_respects_stop() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.frame(0.0);

    // 1. 允许冒泡：点击内部按钮后，Click 依次经过 INNER_BTN -> CARD -> BG_PAGE。
    let btn_pos = Vec2::new(32.0, 32.0);
    rt.pointer_move(btn_pos);
    rt.pointer_down(btn_pos, PointerButton::Left);
    rt.pointer_up(btn_pos, PointerButton::Left);
    assert_eq!(rt.state().clicks, vec![INNER_BTN, CARD, BG_PAGE]);

    // 内置 ButtonDesc 消费了 PointerDown，因此 PointerDown 不会冒泡到外层卡片。
    assert_eq!(rt.state().pointer_downs, vec![Some(INNER_BTN)]);

    // 2. 开启 Stop：点击内部按钮后仅触发 INNER_BTN，外层 CARD 与 BG_PAGE 不会收到。
    rt.state_mut().clicks.clear();
    rt.state_mut().pointer_downs.clear();
    rt.state_mut().stop_at_inner_click = true;
    rt.pointer_down(btn_pos, PointerButton::Left);
    rt.pointer_up(btn_pos, PointerButton::Left);
    assert_eq!(rt.state().clicks, vec![INNER_BTN]);
}

#[test]
fn portal_boundary_blocks_bubbling_to_background_page() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.state_mut().show_portal = true;
    rt.state_mut().portal_is_boundary = true;
    rt.invalidate();
    rt.frame(0.0);

    let pos = Vec2::new(32.0, 32.0);
    rt.pointer_move(pos);
    rt.pointer_down(pos, PointerButton::Left);
    rt.pointer_up(pos, PointerButton::Left);

    // 浮层内按钮未显式 Stop 时，最多冒泡至 PORTAL_PANEL -> PORTAL_ROOT，绝不渗透到 BG_PAGE。
    assert_eq!(
        rt.state().clicks,
        vec![PORTAL_INPUT, PORTAL_PANEL, PORTAL_ROOT]
    );

    rt.wheel(pos, Vec2::new(0.0, 1.0));
    assert_eq!(
        rt.state().wheels,
        vec![Some(PORTAL_INPUT), Some(PORTAL_PANEL), Some(PORTAL_ROOT)]
    );
}

#[test]
fn high_z_container_without_portal_flag_does_not_block_bubbling() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.state_mut().show_portal = true;
    // 只把绘制层级抬到 1200，不声明浮层根：事件边界与 z_index 解耦后，
    // 冒泡必须继续抵达背景页面，而不是被魔数阈值提前截断。
    rt.invalidate();
    rt.frame(0.0);

    let pos = Vec2::new(32.0, 32.0);
    rt.pointer_move(pos);
    rt.pointer_down(pos, PointerButton::Left);
    rt.pointer_up(pos, PointerButton::Left);

    assert_eq!(
        rt.state().clicks,
        vec![PORTAL_INPUT, PORTAL_PANEL, PORTAL_ROOT, BG_PAGE]
    );
}

#[test]
fn sub_hit_key_bubbles_through_owner_and_ancestors() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.state_mut().show_sub_hit = true;
    rt.invalidate();
    rt.frame(0.0);

    let pos = rt
        .router()
        .regions()
        .iter()
        .find(|r| r.node == SUB_HIT)
        .map(|r| r.bounds.center())
        .expect("sub-hit region must be registered in the router");
    rt.pointer_move(pos);
    rt.pointer_down(pos, PointerButton::Left);
    rt.pointer_up(pos, PointerButton::Left);

    assert!(
        rt.state().clicks.starts_with(&[SUB_HIT, SUB_WIDGET]),
        "synthetic sub-key must hit itself first, then bubble to its owner; actual path: {:?}",
        rt.state().clicks
    );
    assert!(
        rt.state().clicks.contains(&BG_PAGE),
        "sub-hit region must keep bubbling to logical ancestors; actual path: {:?}",
        rt.state().clicks
    );
}

#[test]
fn key_bubbles_before_overlay_escape_and_exposes_event_target() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.state_mut().show_portal = true;
    rt.state_mut().portal_is_boundary = true;
    rt.state_mut().stop_escape_in_input = true;
    rt.invalidate();
    rt.frame(0.0);

    rt.open_overlay(OverlayEntry {
        node: PORTAL_ROOT,
        anchor: None,
        kind: OverlayKind::Modal,
        bounds: Rect::new(0.0, 0.0, 240.0, 140.0),
        focus_scope: vec![PORTAL_INPUT],
        anchor_clipped: false,
        prefer_native_popup: false,
    });
    // 当前浮层条目只安装焦点陷阱、不自动移动焦点，显式聚焦模态内的输入节点。
    rt.focus_node(Some(PORTAL_INPUT));
    assert_eq!(rt.context().focused, Some(PORTAL_INPUT));

    // 1. 聚焦子控件消费 Escape 并返回 Stop：覆盖层保持打开，且不向父级继续冒泡。
    rt.key(AppKey::Escape, true, Modifiers::default());
    assert_eq!(rt.state().keys, vec![(Some(PORTAL_INPUT), AppKey::Escape)]);
    assert!(rt.overlays_mut().stack().contains(PORTAL_ROOT));

    // 2. 子控件不再拦截 Escape：沿逻辑父链冒泡并最终由 OverlayManager 关闭浮层。
    rt.state_mut().keys.clear();
    rt.state_mut().stop_escape_in_input = false;
    rt.key(AppKey::Escape, true, Modifiers::default());
    assert_eq!(
        rt.state().keys,
        vec![
            (Some(PORTAL_INPUT), AppKey::Escape),
            (Some(PORTAL_PANEL), AppKey::Escape),
            (Some(PORTAL_ROOT), AppKey::Escape),
        ]
    );
    assert!(!rt.overlays_mut().stack().contains(PORTAL_ROOT));
}

#[test]
fn inspector_records_event_trace_when_enabled() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.state_mut().stop_at_inner_click = true;
    rt.frame(0.0);

    // 按 F12 开启 Inspector。
    rt.key(AppKey::F(12), true, Modifiers::default());
    let btn_pos = Vec2::new(32.0, 32.0);
    rt.pointer_move(btn_pos);
    rt.pointer_down(btn_pos, PointerButton::Left);
    rt.pointer_up(btn_pos, PointerButton::Left);

    let trace = rt.last_event_trace().expect("event trace must be recorded");
    assert_eq!(trace.kind, "Click");
    assert_eq!(trace.path, vec![INNER_BTN]);
    assert_eq!(trace.stopped_at, Some(INNER_BTN));
}

#[test]
fn inspector_traces_key_and_wheel_after_unified_dispatch() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.frame(0.0);
    rt.key(AppKey::F(12), true, Modifiers::default());
    rt.focus_node(Some(INNER_BTN));

    rt.key(AppKey::Character('a'), true, Modifiers::default());
    assert_eq!(rt.last_event_trace().map(|t| t.kind), Some("Key"));
    assert!(rt.last_event_trace().is_some_and(|t| !t.path.is_empty()));

    rt.wheel(Vec2::new(32.0, 32.0), Vec2::new(0.0, 1.0));
    assert_eq!(rt.last_event_trace().map(|t| t.kind), Some("Wheel"));
}

// ============================================================================
// 标题栏冒泡兜底、WindowCommand 与关闭拦截回归测试
// ============================================================================

const TB_BADGE: u64 = 900_001;
const TB_INPUT: u64 = 900_002;
const TB_ACTION_BTN: u64 = 900_003;
const TB_SECOND_ROOT: u64 = 900_100;
const TB_SECOND_MIN: u64 = 900_101;
const TB_SECOND_MAX: u64 = 900_102;
const TB_SECOND_CLOSE: u64 = 900_103;
const TB_MODAL_ROOT: u64 = 900_200;
const TB_MODAL_SHIELD: u64 = 900_201;

/// 铺满视口的模态遮挡层：只发射一整块命中区，用于验证浮层边界阻断标题栏命令。
#[derive(Clone, Debug)]
struct ModalShield;

impl UiWidget for ModalShield {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(1000.0, 600.0)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        WidgetOutput {
            hit: Some(HitRegion::ui(node, 1, rect)),
            ..Default::default()
        }
    }
}

/// 标题栏交互探针：同时承载视图开关与「应用层是否拦到事件」的观测结果。
#[derive(Default)]
struct TitleBarProbe {
    block_close: bool,
    show_modal: bool,
    two_title_bars: bool,
    close_blocked: bool,
    second_close_clicked: bool,
}

struct TitleBarBubbleApp {
    block_close: bool,
}

impl TitleBarBubbleApp {
    fn new() -> Self {
        Self { block_close: false }
    }
}

impl Application for TitleBarBubbleApp {
    type State = TitleBarProbe;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        TitleBarProbe {
            block_close: self.block_close,
            ..Default::default()
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        // 只有应用层返回 `Propagation::Stop` 才能阻止运行时把默认窗口命令压入队列。
        if let AppEvent::Click { node, .. } = event {
            if node == TITLE_BAR_CLOSE_ID && state.block_close {
                state.close_blocked = true;
                return EventOutcome::stopping(DirtyFlags::NONE);
            }
            if node == TB_SECOND_CLOSE {
                state.second_close_clicked = true;
                return EventOutcome::stopping(DirtyFlags::NONE);
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        let two_title_bars = state.two_title_bars;
        let show_modal = state.show_modal;
        view.column(ui_key!("tb_bubble_page"), |col| {
            col.title_bar_custom(
                TitleBarDesc::new("Bubble Title"),
                TitleBarSlots::default()
                    .left(|left| {
                        // 纯展示徽章不发射命中区，也不消费按压：点击它必须冒泡到根容器。
                        left.badge(TB_BADGE, BadgeDesc::label("Online"));
                    })
                    .center(|center| {
                        center.text_input(TB_INPUT, TextInputDesc::text("").placeholder("Search"));
                    })
                    .actions(|actions| {
                        actions.button(TB_ACTION_BTN, ButtonDesc::label("Action"));
                    }),
            );
            if two_title_bars {
                // 第二个标题栏使用独立的根键与三个独立控制按钮键，验证多实例键空间隔离。
                col.title_bar_with(
                    TitleBarDesc::new("Second Bar")
                        .id(TB_SECOND_ROOT)
                        .minimize_id(TB_SECOND_MIN)
                        .maximize_id(TB_SECOND_MAX)
                        .close_id(TB_SECOND_CLOSE),
                );
            }
            if show_modal {
                let modal_desc = RawNodeDesc::container()
                    .absolute()
                    .inset(unbound_abi::Edges::all(0.0))
                    .size(1000.0, 600.0)
                    .z_index(1200)
                    .portal_root();
                col.container(TB_MODAL_ROOT, modal_desc, |portal| {
                    portal.add_with_layout(
                        TB_MODAL_SHIELD,
                        RawNodeDesc::container()
                            .absolute()
                            .inset(unbound_abi::Edges::all(0.0))
                            .size(1000.0, 600.0),
                        ModalShield,
                    );
                });
            }
        });
    }
}

/// 标题栏内不被任何交互控件覆盖的空白点：位于左侧插槽尾端与中间插槽之间。
fn title_bar_blank_point(rt: &AppRuntime<TitleBarBubbleApp>) -> Vec2 {
    let badge_right = rt
        .widget_rect(TB_BADGE)
        .expect("badge must be mounted")
        .max()
        .x;
    let search_left = rt
        .widget_rect(TB_INPUT)
        .expect("search input must be mounted")
        .origin
        .x;
    assert!(
        search_left > badge_right,
        "left slot and center slot must leave a gap: badge right {badge_right}, search left {search_left}"
    );
    Vec2::new((badge_right + search_left) * 0.5, 18.0)
}

fn new_title_bar_runtime() -> AppRuntime<TitleBarBubbleApp> {
    let mut rt = AppRuntime::new(TitleBarBubbleApp::new(), Vec2::new(1000.0, 600.0));
    rt.frame(0.0);
    rt
}

#[test]
fn title_bar_blank_and_badge_bubble_into_drag_and_double_click_toggles_maximize() {
    let mut rt = new_title_bar_runtime();
    let blank = title_bar_blank_point(&rt);

    // 1. 空白处单击：按下瞬间即产出 StartDrag，供窗口立即进入系统拖拽。
    rt.pointer_down(blank, PointerButton::Left);
    assert_eq!(rt.drain_window_commands(), vec![WindowCommand::StartDrag]);
    rt.pointer_up(blank, PointerButton::Left);
    let _ = rt.drain_window_commands();

    // 2. 纯展示徽章不发射命中区也不消费按压，事件冒泡至标题栏根容器，同样是 StartDrag。
    let badge_point = rt
        .widget_rect(TB_BADGE)
        .expect("badge must be mounted")
        .center();
    rt.frame(0.05);
    rt.pointer_down(badge_point, PointerButton::Left);
    assert_eq!(rt.drain_window_commands(), vec![WindowCommand::StartDrag]);
    rt.pointer_up(badge_point, PointerButton::Left);
    let _ = rt.drain_window_commands();

    // 3. 双击空白处：第二次按下的 click_count 为 2，产出 ToggleMaximize 而非拖拽。
    rt.frame(0.05);
    rt.pointer_down(blank, PointerButton::Left);
    let _ = rt.drain_window_commands();
    rt.pointer_up(blank, PointerButton::Left);
    rt.frame(0.05);
    rt.pointer_down(blank, PointerButton::Left);
    assert_eq!(
        rt.drain_window_commands(),
        vec![WindowCommand::ToggleMaximize],
        "double-clicking the blank title bar area must toggle maximize instead of dragging again"
    );
    rt.pointer_up(blank, PointerButton::Left);
}

#[test]
fn interactive_widgets_and_caption_buttons_do_not_emit_drag_on_press() {
    let mut rt = new_title_bar_runtime();
    let interactive_widgets = [
        ("global search input", TB_INPUT),
        ("action button", TB_ACTION_BTN),
        ("minimize button", TITLE_BAR_MINIMIZE_ID),
        ("maximize button", TITLE_BAR_MAXIMIZE_ID),
        ("close button", TITLE_BAR_CLOSE_ID),
    ];
    for (label, key) in interactive_widgets {
        let point = rt
            .widget_rect(key)
            .unwrap_or_else(|| panic!("{label} must be mounted"))
            .center();
        rt.pointer_down(point, PointerButton::Left);
        assert!(
            rt.drain_window_commands().is_empty(),
            "pressing the {label} must not emit any window command (especially not a drag)"
        );
        rt.pointer_up(point, PointerButton::Left);
        let _ = rt.drain_window_commands();
    }
}

#[test]
fn title_bar_close_requires_press_and_release_on_the_same_button() {
    let mut rt = new_title_bar_runtime();
    let blank = title_bar_blank_point(&rt);
    let close_point = rt
        .widget_rect(TITLE_BAR_CLOSE_ID)
        .expect("close button must be mounted")
        .center();

    rt.pointer_down(blank, PointerButton::Left);
    let _ = rt.drain_window_commands();
    rt.pointer_up(close_point, PointerButton::Left);
    assert!(
        !rt.drain_window_commands().contains(&WindowCommand::Close),
        "pressing on blank title bar space and releasing on the close button must never close the window"
    );

    // 反向同样成立：在关闭按钮上按下、滑出后松手属于“反悔”，不得关闭。
    rt.pointer_down(close_point, PointerButton::Left);
    let _ = rt.drain_window_commands();
    rt.pointer_up(blank, PointerButton::Left);
    assert!(
        !rt.drain_window_commands().contains(&WindowCommand::Close),
        "pressing the close button and releasing outside it must cancel the close"
    );
}

#[test]
fn app_can_block_close_while_minimize_still_passes_through() {
    let mut app = TitleBarBubbleApp::new();
    app.block_close = true;
    let mut rt = AppRuntime::new(app, Vec2::new(1000.0, 600.0));
    rt.frame(0.0);

    let close_point = rt
        .widget_rect(TITLE_BAR_CLOSE_ID)
        .expect("close button must be mounted")
        .center();
    rt.pointer_down(close_point, PointerButton::Left);
    rt.pointer_up(close_point, PointerButton::Left);
    assert!(
        rt.state().close_blocked,
        "the app must receive the close button Click inside reduce"
    );
    assert!(
        !rt.drain_window_commands().contains(&WindowCommand::Close),
        "after the app returns Propagation::Stop no WindowCommand::Close may be queued"
    );

    let minimize_point = rt
        .widget_rect(TITLE_BAR_MINIMIZE_ID)
        .expect("minimize button must be mounted")
        .center();
    rt.pointer_down(minimize_point, PointerButton::Left);
    rt.pointer_up(minimize_point, PointerButton::Left);
    assert_eq!(
        rt.drain_window_commands(),
        vec![WindowCommand::Minimize],
        "an unblocked minimize button must fall back to the bubbling default and emit Minimize"
    );
}

#[test]
fn modal_overlay_blocks_caption_buttons_from_responding() {
    let mut rt = new_title_bar_runtime();
    let minimize_point = rt
        .widget_rect(TITLE_BAR_MINIMIZE_ID)
        .expect("minimize button must be mounted")
        .center();

    // 基线：无遮挡时点击最小化按钮产出 Minimize。
    rt.pointer_down(minimize_point, PointerButton::Left);
    rt.pointer_up(minimize_point, PointerButton::Left);
    assert_eq!(rt.drain_window_commands(), vec![WindowCommand::Minimize]);

    // 打开铺满视口的浮层后，事件在浮层边界截断，绝不穿透到标题栏。
    rt.state_mut().show_modal = true;
    rt.invalidate();
    rt.frame(0.016);
    rt.pointer_down(minimize_point, PointerButton::Left);
    rt.pointer_up(minimize_point, PointerButton::Left);
    assert!(
        rt.drain_window_commands().is_empty(),
        "with a modal overlay in place, clicking the top-right corner must not leak into window commands"
    );
}

#[test]
fn multiple_title_bars_hit_independently_and_route_custom_close_keys() {
    let mut rt = new_title_bar_runtime();
    rt.state_mut().two_title_bars = true;
    rt.invalidate();
    rt.frame(0.016);

    // 第二个标题栏的关闭按钮使用自定义键，Click 精确路由到应用拦截逻辑。
    let second_close_point = rt
        .widget_rect(TB_SECOND_CLOSE)
        .expect("second title bar close button must be mounted")
        .center();
    rt.pointer_down(second_close_point, PointerButton::Left);
    rt.pointer_up(second_close_point, PointerButton::Left);
    assert!(rt.state().second_close_clicked);
    assert!(
        rt.drain_window_commands().is_empty(),
        "the app already blocked the second title bar close, so no Close may be emitted"
    );

    // 第一个标题栏的关闭按钮仍走默认绑定，说明两个实例的绑定表互不覆盖。
    let first_close_point = rt
        .widget_rect(TITLE_BAR_CLOSE_ID)
        .expect("first title bar close button must be mounted")
        .center();
    rt.pointer_down(first_close_point, PointerButton::Left);
    rt.pointer_up(first_close_point, PointerButton::Left);
    assert_eq!(rt.drain_window_commands(), vec![WindowCommand::Close]);

    // 第二个标题栏的空白区同样冒泡出 StartDrag，不受第一个标题栏绑定影响。
    let second_bar_y = rt
        .widget_rect(TB_SECOND_MIN)
        .expect("second title bar minimize button must be mounted")
        .center()
        .y;
    let second_bar_blank = Vec2::new(300.0, second_bar_y);
    rt.pointer_down(second_bar_blank, PointerButton::Left);
    assert_eq!(rt.drain_window_commands(), vec![WindowCommand::StartDrag]);
    rt.pointer_up(second_bar_blank, PointerButton::Left);
}

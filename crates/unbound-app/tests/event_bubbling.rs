//! 统一事件传播与冒泡回归测试：
//! - 离散指针事件沿 `UiWorld` 逻辑父链冒泡与 `Propagation::Stop` 截断
//! - 内置有状态控件消费 `PointerDown` 后自动阻止继续冒泡
//! - Portal / 模态边界阻断事件渗透至背景页面
//! - 高 `z_index` 的普通容器不再被误判为浮层边界
//! - 多部件控件的合成子命中键经归属表接回逻辑父链
//! - 键盘焦点链优先于 `OverlayManager::handle_escape` 冒泡，并通过 `ctx.event_target()` 暴露当前祖先
//! - `Inspector` 对指针 / 键盘 / 滚轮事件统一记录轨迹

use unbound_abi::{Dim, RawNodeDesc};
use unbound_app::{
    AppContext, AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome, Modifiers,
    ViewBuilder,
};
use unbound_input::{HitRegion, PointerButton};
use unbound_math::{Rect, Vec2};
use unbound_widgets::ergonomics::{TextCtx, UiWidget};
use unbound_widgets::{
    ButtonDesc, ButtonVariant, CheckboxDesc, ControlState, OverlayEntry, OverlayKind, WidgetOutput,
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
        .expect("子命中区域必须已登记到路由器");
    rt.pointer_move(pos);
    rt.pointer_down(pos, PointerButton::Left);
    rt.pointer_up(pos, PointerButton::Left);

    assert!(
        rt.state().clicks.starts_with(&[SUB_HIT, SUB_WIDGET]),
        "合成子键须先命中自身、再冒泡到所属控件，实际路径: {:?}",
        rt.state().clicks
    );
    assert!(
        rt.state().clicks.contains(&BG_PAGE),
        "子命中区域须继续冒泡到逻辑祖先，实际路径: {:?}",
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

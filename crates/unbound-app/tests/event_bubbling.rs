//! 统一事件传播与冒泡回归测试：
//! - 离散指针事件沿 `UiWorld` 逻辑父链冒泡与 `Propagation::Stop` 截断
//! - 内置有状态控件消费 `PointerDown` 后自动阻止继续冒泡
//! - Portal / 模态边界阻断事件渗透至背景页面
//! - 键盘焦点链优先于 `OverlayManager::handle_escape` 冒泡，并通过 `ctx.event_target()` 暴露当前祖先
//! - `Inspector` 事件轨迹追踪

use unbound_abi::{Dim, RawNodeDesc};
use unbound_app::{
    AppContext, AppEvent, AppKey, AppRuntime, Application, DirtyFlags, EventOutcome, Modifiers,
    ViewBuilder,
};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_widgets::{ButtonDesc, ButtonVariant, CheckboxDesc, OverlayEntry, OverlayKind};

const BG_PAGE: u64 = 100;
const CARD: u64 = 101;
const INNER_BTN: u64 = 102;
const PORTAL_ROOT: u64 = 200;
const PORTAL_PANEL: u64 = 201;
const PORTAL_INPUT: u64 = 202;

#[derive(Default)]
struct BubbleProbe {
    stop_at_inner_click: bool,
    stop_escape_in_input: bool,
    show_portal: bool,
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

                if state.show_portal {
                    page.container(
                        PORTAL_ROOT,
                        RawNodeDesc::container()
                            .absolute()
                            .size(600.0, 400.0)
                            .z_index(1200),
                        |portal| {
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
                        },
                    );
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
    rt.invalidate();
    rt.frame(0.0);

    let pos = Vec2::new(32.0, 32.0);
    rt.pointer_move(pos);
    rt.pointer_down(pos, PointerButton::Left);
    rt.pointer_up(pos, PointerButton::Left);

    // Portal 内按钮未显式 Stop 时，最多冒泡至 PORTAL_PANEL -> PORTAL_ROOT，绝不渗透到 BG_PAGE。
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
fn key_bubbles_before_overlay_escape_and_exposes_event_target() {
    let mut rt = AppRuntime::new(BubbleApp, Vec2::new(600.0, 400.0));
    rt.state_mut().show_portal = true;
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

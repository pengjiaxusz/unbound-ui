//! 声明式富内容工具提示（`ViewBuilder::tooltip_with`）集成测试（ADR-0030）。
//!
//! 验证内容：
//! 1. `ViewBuilder::tooltip_with` 将气泡挂载于 `OverlayKind::Tooltip` 浮层并支持任意展示型子树；
//! 2. 气泡外壳及其所有富内容子组件强制保持鼠标穿透，绝不拦截下方控件的悬停与点击；
//! 3. 气泡及其子组件（即便闭包内包含可聚焦控件）绝不进入键盘 Tab 焦点顺序；
//! 4. 锚点消失后，浮层在同帧自动裁剪移除。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_widgets::button::emit_button;
use unbound_widgets::kbd::{emit_kbd, KbdDesc, KbdSize};

const ANCHOR_BTN: u64 = 3001;
const UNDERLYING_BTN: u64 = 3002;
const RICH_TOOLTIP_KEY: u64 = 3010;
const INNER_ROGUE_BTN: u64 = 3011;
const INNER_KBD_KEY: u64 = 3012;
const PLAIN_TOOLTIP_KEY: u64 = 3020;

struct RichTooltipAppState {
    show_anchor: bool,
    show_tooltip: bool,
    underlying_clicks: u32,
}

#[derive(Default)]
struct RichTooltipApp;

impl Application for RichTooltipApp {
    type State = RichTooltipAppState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        RichTooltipAppState {
            show_anchor: true,
            show_tooltip: true,
            underlying_clicks: 0,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == UNDERLYING_BTN => {
                state.underlying_clicks += 1;
                EventOutcome::new(DirtyFlags::PAINT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        // 锚点按钮与被气泡覆盖的底层按钮均位于固定位置，便于精确验证鼠标穿透。
        let anchor_rect = Rect::new(200.0, 260.0, 120.0, 32.0);
        let underlying_rect = Rect::new(160.0, 180.0, 200.0, 76.0);

        if state.show_anchor {
            view.container(
                ui_key!("anchor_slot"),
                RawNodeDesc::container().absolute().bounds(
                    anchor_rect.origin.x,
                    anchor_rect.origin.y,
                    anchor_rect.width(),
                    anchor_rect.height(),
                ),
                |c| {
                    c.button(ANCHOR_BTN, ButtonDesc::label("Trigger"));
                },
            );
        }

        // 底层按钮精确覆盖气泡上方落点区域，用于验证鼠标穿透。
        let underlying_desc = ButtonDesc::new(underlying_rect, "Underlying");
        let underlying_ctrl = view.ctx.control_state(UNDERLYING_BTN, false, false);
        view.push(
            UNDERLYING_BTN,
            emit_button(UNDERLYING_BTN, &underlying_desc, underlying_ctrl, &mut []),
        );

        if state.show_tooltip {
            let desc = TooltipDesc::rich(anchor_rect, viewport, Vec2::new(180.0, 56.0))
                .with_label("Command Palette Ctrl+K")
                .with_placement(Placement::Top);
            let kbd_origin = desc.content_rect().origin;
            view.tooltip_with(RICH_TOOLTIP_KEY, ANCHOR_BTN, desc, |tip| {
                tip.text_primary(ui_key!("tip_title"), "Command Palette", 12.0, 600);
                let kbd = KbdDesc::sized_at(kbd_origin, "Ctrl", KbdSize::Sm);
                tip.push(
                    INNER_KBD_KEY,
                    emit_kbd(INNER_KBD_KEY, &kbd, ControlState::default(), &[]),
                );
                // 故意在闭包内放入一个交互按钮，验证运行时强制剥离其命中区与焦点能力。
                tip.button(INNER_ROGUE_BTN, ButtonDesc::label("Must Not Focus"));
            });
        }

        // 同时验证纯文本 `ViewBuilder::tooltip` 仍向后兼容。
        view.tooltip(
            PLAIN_TOOLTIP_KEY,
            TooltipDesc::labeled(
                Rect::new(500.0, 260.0, 80.0, 32.0),
                viewport,
                "Plain text tooltip",
            ),
        );
    }
}

#[test]
fn tooltip_with_mounts_tooltip_overlay_and_keeps_pointer_passthrough_and_non_focusable() {
    let mut rt = AppRuntime::new(RichTooltipApp, Vec2::new(800.0, 600.0));
    rt.frame(0.0);

    // 1. 浮层栈中应存在 `OverlayKind::Tooltip` 且锚定到 ANCHOR_BTN。
    let entry = rt
        .overlays()
        .stack()
        .entries()
        .iter()
        .find(|e| e.node == RICH_TOOLTIP_KEY)
        .expect("rich tooltip should be mounted on OverlayStack");
    assert_eq!(entry.kind, OverlayKind::Tooltip);
    assert_eq!(entry.anchor, Some(ANCHOR_BTN));
    assert!(!rt.overlays().is_modal_active());

    // 2. 鼠标穿透：点击落在富内容气泡矩形内部且重叠在 UNDERLYING_BTN 之上的点，
    //    必须直接命中 UNDERLYING_BTN，绝不能被气泡或内部子控件截走。
    let probe = entry.bounds.center();
    rt.pointer_move(probe);
    assert_eq!(
        rt.context().hovered,
        Some(UNDERLYING_BTN),
        "pointer inside tooltip bounds must pass through to underlying widget"
    );
    rt.pointer_down(probe, PointerButton::Left);
    rt.pointer_up(probe, PointerButton::Left);
    assert_eq!(
        rt.state().underlying_clicks,
        1,
        "clicking through tooltip must still activate the underlying button"
    );

    // 3. 不抢焦点：连续按 Tab 只会在 ANCHOR_BTN 与 UNDERLYING_BTN 之间切换，
    //    绝不会聚焦到 RICH_TOOLTIP_KEY 或闭包内的 INNER_ROGUE_BTN。
    for _ in 0..6 {
        rt.key(AppKey::Tab, true, Modifiers::default());
        let focused = rt.context().focused;
        assert_ne!(focused, Some(RICH_TOOLTIP_KEY));
        assert_ne!(focused, Some(INNER_ROGUE_BTN));
        assert_ne!(focused, Some(INNER_KBD_KEY));
        assert!(
            matches!(focused, Some(ANCHOR_BTN) | Some(UNDERLYING_BTN)),
            "unexpected focused node: {focused:?}"
        );
    }

    // 4. 锚点消失后，富内容工具提示浮层在同帧自动裁剪。
    rt.state_mut().show_anchor = false;
    rt.frame(1.0 / 60.0);
    assert!(
        !rt.overlays().stack().contains(RICH_TOOLTIP_KEY),
        "tooltip overlay should be pruned when its anchor vanishes"
    );
}

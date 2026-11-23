//! 工具提示延迟计时、Win11 默认样式、自动换行与富内容容器集成测试。
//!
//! 本 crate 尚无定时器队列，因此 [`TooltipDelay`] 是本地截止时间值：
//! 通过 `schedule` 注册，每帧通过 `poll` 读取。此处使用模拟时钟测试这两部分，
//! 并覆盖 ADR-0030 要求的 Win11 Fluent 默认外观与纯展示富内容容器契约。

use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::Token;
use unbound_widgets::ergonomics::{TextCtx, UiWidget};
use unbound_widgets::kbd::{emit_kbd, KbdDesc, KbdSize};
use unbound_widgets::overlay::OverlayKind;
use unbound_widgets::tooltip::{
    emit_tooltip, wrap_text, TooltipDelay, TooltipDesc, TooltipPadding, TooltipState,
    TOOLTIP_BORDER_WIDTH, TOOLTIP_MAX_WIDTH, TOOLTIP_RADIUS, TOOLTIP_SHADOW,
};
use unbound_widgets::{ControlState, WidgetOutput};

fn desc() -> TooltipDesc {
    TooltipDesc::new(
        Rect::new(100.0, 100.0, 80.0, 32.0),
        Rect::new(0.0, 0.0, 800.0, 600.0),
        Vec2::new(120.0, 28.0),
    )
}

#[test]
fn a_deadline_is_due_at_start_plus_delay() {
    let delay = TooltipDelay::start(1_000.0, 400.0);
    assert_eq!(delay.deadline_ms(), 1_400.0);
    assert!(!delay.is_due(1_399.0));
    assert!(delay.is_due(1_400.0));
    assert_eq!(delay.remaining(1_300.0), 100.0);
    assert_eq!(delay.remaining(2_000.0), 0.0);
}

#[test]
fn scheduling_registers_the_delay_and_polling_opens_the_bubble() {
    let desc = desc();
    let mut state = TooltipState::new();

    state.schedule(&desc, Some(7), 1_000.0);
    assert_eq!(state.delay(), Some(TooltipDelay::start(1_000.0, 400.0)));
    assert!(!state.poll(1_200.0));
    assert!(!state.visible());

    assert!(state.poll(1_400.0), "the deadline elapsed");
    assert!(state.visible());
    assert_eq!(state.hovered_ms(), 400.0);

    // 每帧重复注册同一节点会保留原始截止时间。
    state.schedule(&desc, Some(7), 1_500.0);
    assert_eq!(state.delay(), Some(TooltipDelay::start(1_000.0, 400.0)));
    assert!(!state.poll(1_500.0), "the visibility did not change again");
    assert!(state.visible());
}

#[test]
fn moving_to_another_trigger_restarts_the_deadline() {
    let desc = desc();
    let mut state = TooltipState::new();
    state.schedule(&desc, Some(7), 1_000.0);
    state.poll(2_000.0);
    assert!(state.visible());

    state.schedule(&desc, Some(8), 3_000.0);
    assert!(!state.visible(), "a new anchor starts hidden");
    assert_eq!(state.delay(), Some(TooltipDelay::start(3_000.0, 400.0)));
    assert!(!state.poll(3_200.0));
    assert!(state.poll(3_400.0));
}

#[test]
fn leaving_the_trigger_clears_the_deadline() {
    let desc = desc();
    let mut state = TooltipState::new();
    state.schedule(&desc, Some(7), 1_000.0);
    state.poll(2_000.0);
    assert!(state.visible());

    state.schedule(&desc, None, 2_100.0);
    assert!(state.delay().is_none());
    assert!(!state.visible());
    assert!(!state.poll(2_200.0));
}

#[test]
fn a_long_text_wraps_inside_the_280px_measure() {
    let text = "A tooltip that keeps explaining the control long after one line \
                would have run out of room, so it has to wrap onto several lines.";
    let lines = wrap_text(text, TOOLTIP_MAX_WIDTH, 7.0);
    assert!(lines.len() >= 2, "the sentence needs multiple lines");
    for line in &lines {
        let measured = line.chars().count() as f32 * 7.0;
        assert!(
            measured <= TOOLTIP_MAX_WIDTH + 1e-3,
            "line {line:?} measures {measured}"
        );
    }
    assert_eq!(
        lines.join(" "),
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    );
}

#[test]
fn a_wrapped_bubble_sizes_itself_and_never_exceeds_the_max_width() {
    let long = "word ".repeat(80);
    let desc = desc().with_wrapped_text(&long, 7.0, 18.0);
    assert!(desc.size.x <= TOOLTIP_MAX_WIDTH);
    assert!(desc.size.y >= 18.0);
    assert_eq!(
        desc.size.y % 18.0,
        0.0,
        "the height is a whole number of lines"
    );
    assert_eq!(desc.max_width, TOOLTIP_MAX_WIDTH);
    assert!(!desc.wrapped_lines(&long, 7.0).is_empty());
}

#[test]
fn the_wrapped_bubble_still_emits_one_non_pickable_card() {
    let desc = desc().with_wrapped_text("two\nlines of explanation", 6.0, 16.0);
    let (output, _) = emit_tooltip(4, 2, &desc, ControlState::default());
    assert_eq!(output.primitives.len(), 3, "shadow + bubble + border");
    assert!(output.payloads.is_empty());
    assert!(output.hit.is_none(), "tooltips never eat the pointer");
}

#[test]
fn win11_fluent_default_shell_uses_card_border_and_asymmetric_padding() {
    let d = TooltipDesc::labeled(
        Rect::new(200.0, 200.0, 80.0, 32.0),
        Rect::new(0.0, 0.0, 800.0, 600.0),
        "Open command palette",
    );
    assert_eq!(d.radius, TOOLTIP_RADIUS);
    assert_eq!(d.radius, 4.0);
    assert_eq!(d.padding, TooltipPadding::WINUI_DEFAULT);
    assert_eq!(d.padding, TooltipPadding::new(9.0, 6.0, 9.0, 8.0));

    let (output, entry) = emit_tooltip(11, 2, &d, ControlState::default());
    assert_eq!(entry.kind, OverlayKind::Tooltip);
    assert!(output.hit.is_none());
    assert!(!output.focusable);
    assert_eq!(output.primitives.len(), 3);
    assert_eq!(output.primitives[0].shadow_blur(), TOOLTIP_SHADOW);
    assert_eq!(
        output.primitives[1].fill_token_or_uv,
        u32::from(Token::OVERLAY_BG.raw())
    );
    assert_eq!(
        output.primitives[2].border_token_or_tex,
        u32::from(Token::CARD_BORDER.raw())
    );
    assert_eq!(output.primitives[2].border_width(), TOOLTIP_BORDER_WIDTH);

    // 兼容既有 Vec2 对称内边距调用方。
    let symmetric = d.with_padding(Vec2::new(10.0, 6.0));
    assert_eq!(symmetric.padding, TooltipPadding::new(10.0, 6.0, 10.0, 6.0));
}

#[test]
fn rich_tooltip_container_composes_kbd_and_enforces_pointer_passthrough() {
    let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
    let anchor = Rect::new(240.0, 200.0, 88.0, 32.0);
    let content_size = Vec2::new(190.0, 46.0);
    let rich = TooltipDesc::rich(anchor, viewport, content_size).with_label("Quick Search Ctrl+K");

    let layout = rich.layout();
    assert_eq!(layout.outer_rect.width(), content_size.x + 18.0);
    assert_eq!(layout.outer_rect.height(), content_size.y + 14.0);
    assert_eq!(
        layout.content_rect.origin,
        Vec2::new(
            layout.outer_rect.origin.x + 9.0,
            layout.outer_rect.origin.y + 6.0
        )
    );
    assert_eq!(layout.content_rect.size, content_size);

    let (output, entry) = rich.render_with_content(50, 7, ControlState::default(), |content, z| {
        let mut kbd = KbdDesc::sized_at(content.origin, "Ctrl", KbdSize::Sm);
        kbd.z = z;
        let mut child: WidgetOutput = emit_kbd(51, &kbd, ControlState::default(), &[]);
        // 即使内部误带命中区域与焦点能力，纯展示契约也必须剥离。
        child.hit = Some(HitRegion::ui(51, u32::from(z), content));
        child.focusable = true;
        child
    });

    assert_eq!(entry.kind, OverlayKind::Tooltip);
    assert!(
        output.primitives.len() > 3,
        "shell primitives plus Kbd badge primitives should be emitted"
    );
    assert!(output.hit.is_none());
    assert!(output.sub_hits.is_empty());
    assert!(!output.focusable);
}

#[test]
fn ui_widget_tooltip_wraps_multiline_text_and_aligns_left() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut text_ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };

    let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
    let anchor = Rect::new(200.0, 200.0, 80.0, 32.0);
    let short_desc = TooltipDesc::labeled(anchor, viewport, "Save");
    let long_desc = TooltipDesc::labeled(
        anchor,
        viewport,
        "Save the current workspace layout and synchronize every panel state to disk automatically.",
    );

    let short_size = short_desc.intrinsic_size(&mut text_ctx);
    let long_size = long_desc.intrinsic_size(&mut text_ctx);
    assert!(long_size.x <= TOOLTIP_MAX_WIDTH + 1e-3);
    assert!(
        long_size.y > short_size.y,
        "long tooltip ({long_size:?}) should wrap onto multiple lines taller than short ({short_size:?})"
    );

    let output = long_desc.emit_laid_out(
        77,
        anchor,
        ControlState::default(),
        &TooltipState::new(),
        &mut text_ctx,
    );
    assert!(output.hit.is_none());
    assert!(!output.focusable);
    assert!(output.primitives.len() >= 3);
}

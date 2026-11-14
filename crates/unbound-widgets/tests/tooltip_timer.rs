//! 工具提示使用绝对悬停截止时间并在 280px 处自动换行。
//!
//! 本 crate 尚无定时器队列，因此 [`TooltipDelay`] 是本地截止时间值：
//! 通过 `schedule` 注册，每帧通过 `poll` 读取。此处使用模拟时钟测试这两部分。
//!

use unbound_math::{Rect, Vec2};
use unbound_widgets::tooltip::{
    emit_tooltip, wrap_text, TooltipDelay, TooltipDesc, TooltipState, TOOLTIP_MAX_WIDTH,
};
use unbound_widgets::ControlState;

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

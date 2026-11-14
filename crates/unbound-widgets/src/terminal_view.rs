//! TerminalView：高性能虚拟化 ANSI 日志与终端视图。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

pub const TERMINAL_LINE_HEIGHT: f32 = 18.0;

/// 终端日志视图描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalViewDesc {
    pub rect: Rect,
    pub lines: Vec<String>,
    pub follow_tail: bool,
    pub radius: f32,
}

impl TerminalViewDesc {
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            lines: Vec::new(),
            follow_tail: true,
            radius: 6.0,
        }
    }

    #[must_use]
    pub fn follow_tail(follow: bool) -> Self {
        let mut desc = Self::new(Rect::ZERO);
        desc.follow_tail = follow;
        desc
    }

    #[must_use]
    pub fn with_lines(mut self, lines: Vec<String>) -> Self {
        self.lines = lines;
        self
    }
}

impl Default for TerminalViewDesc {
    fn default() -> Self {
        Self::new(Rect::ZERO)
    }
}

/// 终端视图状态。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerminalViewState {
    pub scroll_y: f32,
    pub cursor_line: usize,
}

impl TerminalViewState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// 输出终端视图的图元与命中区域。
#[must_use]
pub fn emit_terminal_view(
    node: u64,
    desc: &TerminalViewDesc,
    state: &TerminalViewState,
    control: ControlState,
) -> WidgetOutput {
    emit_terminal_view_with_glyphs(node, desc, state, control, &[])
}

/// 输出带文本字形的终端视图图元与命中区域。
#[must_use]
pub fn emit_terminal_view_with_glyphs(
    node: u64,
    desc: &TerminalViewDesc,
    state: &TerminalViewState,
    control: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let z = 700u16;

    // 1. 深色终端背景
    out.primitives
        .push(surface(rect, desc.radius, Token::CANVAS_BACKDROP, z));

    // 2. 左侧行号边栏
    let gutter_w = 40.0_f32.min(rect.width() * 0.25);
    let gutter_rect = Rect::new(rect.origin.x, rect.origin.y, gutter_w, rect.height());
    out.primitives
        .push(surface(gutter_rect, 0.0, Token::SURFACE, z + 1));

    // 3. 当前光标行高亮（跟随 cursor_line 与 scroll_y，限制在视口内）
    let line_h = TERMINAL_LINE_HEIGHT.min((rect.height() - 8.0).max(4.0));
    let raw_y = rect.origin.y + 8.0 + state.cursor_line as f32 * line_h - state.scroll_y;
    let max_y = (rect.max().y - line_h - 4.0).max(rect.origin.y + 4.0);
    let cursor_y = raw_y.clamp(rect.origin.y + 4.0, max_y);
    let cursor_rect = Rect::new(
        rect.origin.x + gutter_w,
        cursor_y,
        (rect.width() - gutter_w).max(0.0),
        line_h,
    );
    out.primitives
        .push(surface(cursor_rect, 2.0, Token::PRIMARY_SOFT, z + 2));

    if !label_glyphs.is_empty() {
        emit_glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 4. 外边框
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if control.focused {
            Token::PRIMARY
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        z + 3,
    ));

    // 5. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    out.hit = (!control.disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Text)
    });

    out.a11y = Some(A11yNode::new(Role::Terminal, "TerminalView", rect).disabled(control.disabled));
    out.focusable = !control.disabled;
    out
}

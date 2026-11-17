//! TerminalView：高性能虚拟化 ANSI 日志与终端视图。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::{ControlState, WidgetOutput};

/// 终端行高：与只读代码块 `CODE_LINE_HEIGHT` 同档（20px，匹配 14px 正文）。
pub const TERMINAL_LINE_HEIGHT: f32 = 20.0;

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
            // 圆角对齐 WinUI `ControlCornerRadius`（4），与代码块容器一致。
            radius: 4.0,
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

    // 1. 终端背景：与只读代码块共用 CODE_BG / CODE_TEXT 这一组代码面板令牌。
    out.primitives
        .push(surface(rect, desc.radius, Token::CODE_BG, z));

    // 2. 左侧行号边栏：左侧两角跟随容器圆角，右侧直角与正文区拼合；
    // 底色与代码块/编辑器的行号槽一致（SURFACE_RAISED）。
    let gutter_w = 40.0_f32.min(rect.width() * 0.25);
    let gutter_rect = Rect::new(rect.origin.x, rect.origin.y, gutter_w, rect.height());
    out.primitives.push(instance(
        gutter_rect,
        [desc.radius, 0.0, 0.0, desc.radius],
        Token::SURFACE_RAISED,
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Background,
        z + 1,
    ));

    // 3. 当前光标行高亮（跟随 cursor_line 与 scroll_y，限制在视口内）。
    // 行高与代码块共用 20px 节奏；高亮用 `SubtleFillColorSecondary`（HOVER 令牌），比选区更轻。
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
    out.primitives.push(instance(
        cursor_rect,
        [0.0, desc.radius, desc.radius, 0.0],
        Token::HOVER,
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Background,
        z + 2,
    ));

    if !label_glyphs.is_empty() {
        emit_glyphs(
            label_glyphs,
            Token::CODE_TEXT,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 4. 外边框：1px `ControlStrokeColorDefault`，聚焦由外扩焦点环表达。
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        Token::CONTROL_STROKE,
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

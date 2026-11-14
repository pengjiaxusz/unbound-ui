//! 可编辑代码编辑器（CodeEditor），支持行号槽、当前行高亮、选区与括号匹配。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::code_block::{highlight_line, CodeSpan};
use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// [`CodeEditorDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeEditorState {
    /// 源码缓冲区。
    pub source: String,
    /// 0-based 当前光标所在行。
    pub cursor_line: usize,
    /// 0-based 当前光标所在列。
    pub cursor_col: usize,
    /// 可选选区 `(start_line, start_col)..(end_line, end_col)`。
    pub selection: Option<((usize, usize), (usize, usize))>,
}

impl CodeEditorState {
    /// 使用源码文本创建状态。
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            cursor_line: 0,
            cursor_col: 0,
            selection: None,
        }
    }

    /// 使用 [`highlight_line`] 高亮每一行。
    #[must_use]
    pub fn highlight_spans(&self) -> Vec<Vec<CodeSpan>> {
        self.source.lines().map(highlight_line).collect()
    }

    /// 若光标处为括号，则在 `(cursor_line, cursor_col)` 周围查找匹配的 `()`、`[]` 或 `{}`。
    #[must_use]
    pub fn find_matching_bracket(&self) -> Option<(usize, usize)> {
        let lines: Vec<&str> = self.source.lines().collect();
        let line = lines.get(self.cursor_line)?;
        let ch = line.as_bytes().get(self.cursor_col).copied()?;
        let (open, close, forward) = match ch {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b'{' => (b'{', b'}', true),
            b')' => (b'(', b')', false),
            b']' => (b'[', b']', false),
            b'}' => (b'{', b'}', false),
            _ => return None,
        };
        let mut depth = 0i32;
        if forward {
            for (li, l) in lines.iter().enumerate().skip(self.cursor_line) {
                let start_col = if li == self.cursor_line {
                    self.cursor_col
                } else {
                    0
                };
                for (ci, &b) in l.as_bytes().iter().enumerate().skip(start_col) {
                    if b == open {
                        depth += 1;
                    } else if b == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((li, ci));
                        }
                    }
                }
            }
        } else {
            for li in (0..=self.cursor_line).rev() {
                let l = lines[li];
                let end_col = if li == self.cursor_line {
                    self.cursor_col + 1
                } else {
                    l.len()
                };
                for ci in (0..end_col.min(l.len())).rev() {
                    let b = l.as_bytes()[ci];
                    if b == close {
                        depth += 1;
                    } else if b == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((li, ci));
                        }
                    }
                }
            }
        }
        None
    }
}

/// [`CodeEditor`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CodeEditorDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 总行数。
    pub line_count: usize,
    /// 0-based 当前光标行。
    pub cursor_line: usize,
    /// 0-based 当前光标列。
    pub cursor_col: usize,
    /// 用于语法显示的可选行切片。
    pub lines: Vec<String>,
    /// `cursor_line` 上可选的单行选区字节范围 `(start_col, end_col)`。
    pub selection: Option<(usize, usize)>,
    /// 可选的待高亮匹配括号坐标 `(line, col)`。
    pub matching_bracket: Option<(usize, usize)>,
    /// 垂直滚动偏移量。
    pub scroll: Vec2,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl CodeEditorDesc {
    /// 创建代码编辑器描述符。
    #[must_use]
    pub fn new(rect: Rect, line_count: usize, cursor_line: usize) -> Self {
        Self {
            rect,
            line_count: line_count.max(1),
            cursor_line,
            cursor_col: 0,
            lines: Vec::new(),
            selection: None,
            matching_bracket: None,
            scroll: Vec2::ZERO,
            label: "Code Editor".to_owned(),
            z: 600,
        }
    }

    /// 附加源码行到描述符。
    #[must_use]
    pub fn with_lines(mut self, lines: Vec<String>) -> Self {
        self.line_count = lines.len().max(1);
        self.lines = lines;
        self
    }

    /// 附加光标列位置。
    #[must_use]
    pub fn with_cursor_col(mut self, col: usize) -> Self {
        self.cursor_col = col;
        self
    }

    /// 附加活动选区范围。
    #[must_use]
    pub fn with_selection(mut self, selection: Option<(usize, usize)>) -> Self {
        self.selection = selection;
        self
    }

    /// 附加匹配括号坐标。
    #[must_use]
    pub fn with_matching_bracket(mut self, bracket: Option<(usize, usize)>) -> Self {
        self.matching_bracket = bracket;
        self
    }
}

/// 发射固定形状的代码编辑器（背景 + 行号槽 + 当前行 + 选区 + 括号 + 光标 + 焦点环 = 7 个固定图元）。
#[must_use]
pub fn emit_code_editor(
    node: u64,
    desc: &CodeEditorDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(7 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let gutter_w = 34.0_f32.min(desc.rect.width() * 0.25);
    primitives.push(surface(
        Rect::new(
            desc.rect.origin.x + 2.0,
            desc.rect.origin.y + 2.0,
            gutter_w,
            (desc.rect.height() - 4.0).max(8.0),
        ),
        4.0,
        Token::SURFACE_RAISED,
        desc.z + 1,
    ));

    let line_h = 18.0_f32.min(desc.rect.height() * 0.35);
    let visible_line = desc
        .cursor_line
        .saturating_sub((desc.scroll.y / line_h) as usize);
    let line_y = (desc.rect.origin.y + 6.0 + (visible_line % 16) as f32 * line_h)
        .min(desc.rect.max().y - line_h - 4.0);
    primitives.push(surface(
        Rect::new(
            desc.rect.origin.x + gutter_w + 4.0,
            line_y,
            (desc.rect.width() - gutter_w - 8.0).max(12.0),
            line_h,
        ),
        3.0,
        Token::PRIMARY_SOFT,
        desc.z + 1,
    ));

    // 选区条带（无选区时保持透明以维持固定形状）。
    let (sel_x, sel_w, sel_alpha) = if let Some((s, e)) = desc.selection {
        let lo = s.min(e) as f32 * 8.0;
        let hi = s.max(e) as f32 * 8.0;
        (
            desc.rect.origin.x + gutter_w + 10.0 + lo,
            (hi - lo).max(2.0),
            if e != s { 1.0 } else { 0.0 },
        )
    } else {
        (desc.rect.origin.x + gutter_w + 10.0, 0.0, 0.0)
    };
    let mut sel_prim = surface(
        Rect::new(sel_x, line_y + 1.0, sel_w, (line_h - 2.0).max(6.0)),
        2.0,
        Token::SELECTION,
        desc.z + 1,
    );
    sel_prim.z_and_opacity =
        unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(desc.z + 1, sel_alpha);
    primitives.push(sel_prim);

    // 匹配括号高亮（无匹配时保持透明以维持固定形状）。
    let (brk_rect, brk_alpha) = if let Some((bl, bc)) = desc.matching_bracket {
        let by = (desc.rect.origin.y + 6.0 + (bl % 16) as f32 * line_h)
            .min(desc.rect.max().y - line_h - 4.0);
        let bx = desc.rect.origin.x + gutter_w + 10.0 + bc as f32 * 8.0;
        (Rect::new(bx, by + 1.0, 8.0, (line_h - 2.0).max(6.0)), 1.0)
    } else {
        (
            Rect::new(desc.rect.origin.x, desc.rect.origin.y, 0.0, 0.0),
            0.0,
        )
    };
    let mut brk_prim = outlined(
        brk_rect,
        2.0,
        Token::NONE,
        Token::ACCENT,
        1.0,
        0.0,
        desc.z + 2,
    );
    brk_prim.z_and_opacity =
        unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(desc.z + 2, brk_alpha);
    primitives.push(brk_prim);

    let caret_x = (desc.rect.origin.x + gutter_w + 10.0 + desc.cursor_col as f32 * 8.0
        - desc.scroll.x)
        .clamp(desc.rect.origin.x + gutter_w + 6.0, desc.rect.max().x - 6.0);
    primitives.push(surface(
        Rect::new(caret_x, line_y + 2.0, 2.0, (line_h - 4.0).max(8.0)),
        1.0,
        Token::PRIMARY,
        desc.z + 2,
    ));

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Text)
    });
    let mut a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);
    if !desc.lines.is_empty() {
        a11y.value = desc.lines.join("\n");
    }

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

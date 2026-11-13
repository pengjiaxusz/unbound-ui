//! Editable CodeEditor with gutter line numbers, active line highlight, selection & bracket match.

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

/// Interactive state for [`CodeEditorDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeEditorState {
    /// Source buffer.
    pub source: String,
    /// 0-based active cursor line.
    pub cursor_line: usize,
    /// 0-based cursor column.
    pub cursor_col: usize,
    /// Optional `(start_line, start_col)..(end_line, end_col)` selection.
    pub selection: Option<((usize, usize), (usize, usize))>,
}

impl CodeEditorState {
    /// Create a state with source text.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            cursor_line: 0,
            cursor_col: 0,
            selection: None,
        }
    }

    /// Highlight every line using [`highlight_line`].
    #[must_use]
    pub fn highlight_spans(&self) -> Vec<Vec<CodeSpan>> {
        self.source.lines().map(highlight_line).collect()
    }

    /// Find matching `()`, `[]`, or `{}` around `(cursor_line, cursor_col)` if the character at cursor is a bracket.
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
                let start_col = if li == self.cursor_line { self.cursor_col } else { 0 };
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

/// Descriptor for a [`CodeEditor`].
#[derive(Clone, Debug, PartialEq)]
pub struct CodeEditorDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Total line count.
    pub line_count: usize,
    /// Active 0-based cursor line.
    pub cursor_line: usize,
    /// Active 0-based cursor column.
    pub cursor_col: usize,
    /// Optional line slices for syntax display.
    pub lines: Vec<String>,
    /// Optional single-line selection byte range `(start_col, end_col)` on `cursor_line`.
    pub selection: Option<(usize, usize)>,
    /// Optional matching bracket `(line, col)` to highlight.
    pub matching_bracket: Option<(usize, usize)>,
    /// Vertical scroll offset.
    pub scroll: Vec2,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl CodeEditorDesc {
    /// Create a code editor descriptor.
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

    /// Attach source lines to the descriptor.
    #[must_use]
    pub fn with_lines(mut self, lines: Vec<String>) -> Self {
        self.line_count = lines.len().max(1);
        self.lines = lines;
        self
    }

    /// Attach a cursor column.
    #[must_use]
    pub fn with_cursor_col(mut self, col: usize) -> Self {
        self.cursor_col = col;
        self
    }

    /// Attach an active selection range.
    #[must_use]
    pub fn with_selection(mut self, selection: Option<(usize, usize)>) -> Self {
        self.selection = selection;
        self
    }

    /// Attach a matching bracket coordinate.
    #[must_use]
    pub fn with_matching_bracket(mut self, bracket: Option<(usize, usize)>) -> Self {
        self.matching_bracket = bracket;
        self
    }
}

/// Emit a stable-shape code editor (bg + gutter + active line + selection + bracket + caret + focus ring = 7 fixed primitives).
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
    let visible_line = desc.cursor_line.saturating_sub((desc.scroll.y / line_h) as usize);
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

    // Selection band (transparent when absent for Stable Shape).
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

    // Matching bracket highlight (transparent when absent for Stable Shape).
    let (brk_rect, brk_alpha) = if let Some((bl, bc)) = desc.matching_bracket {
        let by = (desc.rect.origin.y + 6.0 + (bl % 16) as f32 * line_h)
            .min(desc.rect.max().y - line_h - 4.0);
        let bx = desc.rect.origin.x + gutter_w + 10.0 + bc as f32 * 8.0;
        (Rect::new(bx, by + 1.0, 8.0, (line_h - 2.0).max(6.0)), 1.0)
    } else {
        (Rect::new(desc.rect.origin.x, desc.rect.origin.y, 0.0, 0.0), 0.0)
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

    let caret_x = (desc.rect.origin.x + gutter_w + 10.0 + desc.cursor_col as f32 * 8.0 - desc.scroll.x)
        .clamp(desc.rect.origin.x + gutter_w + 6.0, desc.rect.max().x - 6.0);
    primitives.push(surface(
        Rect::new(
            caret_x,
            line_y + 2.0,
            2.0,
            (line_h - 4.0).max(8.0),
        ),
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

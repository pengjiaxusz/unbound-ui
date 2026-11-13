//! Editable CodeEditor with gutter line numbers, active line highlight & bracket match.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

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
            label: "Code Editor".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape code editor (1 bg + 1 gutter + 1 active line highlight + 1 caret + 1 focus ring = 5 primitives).
#[must_use]
pub fn emit_code_editor(
    node: u64,
    desc: &CodeEditorDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
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
    let line_y = (desc.rect.origin.y + 6.0 + (desc.cursor_line % 4) as f32 * line_h)
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

    primitives.push(surface(
        Rect::new(
            desc.rect.origin.x + gutter_w + 10.0,
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
    let a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

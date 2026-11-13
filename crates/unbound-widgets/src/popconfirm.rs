//! Popconfirm anchored lightweight confirmation bubble.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// Descriptor for a [`Popconfirm`] anchored bubble.
#[derive(Clone, Debug, PartialEq)]
pub struct PopconfirmDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Confirmation prompt title.
    pub title: String,
    /// Confirm button label.
    pub confirm_label: String,
    /// Cancel button label.
    pub cancel_label: String,
    /// Base z-index.
    pub z: u16,
}

impl PopconfirmDesc {
    /// Create a popconfirm descriptor.
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            confirm_label: "Confirm".to_owned(),
            cancel_label: "Cancel".to_owned(),
            z: 800,
        }
    }
}

/// Emit a stable-shape popconfirm bubble (1 card + cancel button + confirm button + 1 focus ring = 4 primitives).
#[must_use]
pub fn emit_popconfirm(
    node: u64,
    desc: &PopconfirmDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        Token::SURFACE_RAISED,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        6.0,
        desc.z,
    ));

    let btn_w = 56.0_f32.min((desc.rect.width() - 24.0) * 0.45);
    let btn_h = 22.0_f32.min(desc.rect.height() * 0.4);
    let btn_y = desc.rect.max().y - btn_h - 8.0;
    let confirm_r = Rect::new(desc.rect.max().x - btn_w - 8.0, btn_y, btn_w, btn_h);
    let cancel_r = Rect::new(confirm_r.origin.x - btn_w - 6.0, btn_y, btn_w, btn_h);

    primitives.push(outlined(
        cancel_r,
        4.0,
        Token::SURFACE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        desc.z + 1,
    ));
    primitives.push(outlined(
        confirm_r,
        4.0,
        Token::ERROR,
        Token::ERROR,
        1.0,
        0.0,
        desc.z + 1,
    ));

    primitives.push(focus_ring(
        desc.rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Dialog, desc.title.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

//! PinInput / OtpInput fixed-length segmented code input with paste-splitting.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_PIN_SLOTS: usize = 6;

/// Interactive state for [`PinInputDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinInputState {
    /// Entered characters up to `length`.
    pub digits: Vec<char>,
    /// Active cell index.
    pub cursor: usize,
}

impl PinInputState {
    /// Insert a character and advance cursor.
    pub fn push_char(&mut self, ch: char, length: usize) {
        let cap = length.clamp(1, MAX_PIN_SLOTS);
        if self.cursor < cap {
            if self.cursor < self.digits.len() {
                self.digits[self.cursor] = ch;
            } else {
                self.digits.push(ch);
            }
            self.cursor = (self.cursor + 1).min(cap);
        }
    }

    /// Paste a string, splitting characters across slots.
    pub fn paste(&mut self, text: &str, length: usize) {
        for ch in text.chars().filter(|c| !c.is_whitespace()) {
            if self.cursor >= length.clamp(1, MAX_PIN_SLOTS) {
                break;
            }
            self.push_char(ch, length);
        }
    }
}

/// Descriptor for a [`PinInput`].
#[derive(Clone, Debug, PartialEq)]
pub struct PinInputDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Number of PIN cells (`1..=6`).
    pub length: usize,
    /// Number of currently filled cells.
    pub filled: usize,
    /// Whether digits are masked.
    pub masked: bool,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl PinInputDesc {
    /// Create a PIN input descriptor.
    #[must_use]
    pub fn new(rect: Rect, length: usize, filled: usize) -> Self {
        Self {
            rect,
            length: length.clamp(1, MAX_PIN_SLOTS),
            filled,
            masked: false,
            label: "PIN Input".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape PIN input (6 cell boxes + 6 fill dots + 1 focus ring = 13 primitives).
#[must_use]
pub fn emit_pin_input(
    node: u64,
    desc: &PinInputDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(13 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let len = desc.length.clamp(1, MAX_PIN_SLOTS);
    let gap = 6.0;
    let cell_w = ((desc.rect.width() - gap * (len - 1) as f32) / len as f32).max(14.0);
    let cell_h = desc.rect.height().max(20.0);

    for idx in 0..MAX_PIN_SLOTS {
        let visible = idx < len;
        let r = Rect::new(
            desc.rect.origin.x + idx.min(len - 1) as f32 * (cell_w + gap),
            desc.rect.origin.y,
            cell_w,
            cell_h,
        );
        let mut cell = outlined(
            r,
            6.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            if state.focused && idx == desc.filled.min(len - 1) {
                Token::FOCUS_RING
            } else {
                Token::CARD_BORDER
            },
            1.0,
            0.0,
            desc.z,
        );
        if !visible {
            cell.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(cell.depth_u16(), 0.0);
        }
        primitives.push(cell);

        let dot_r = Rect::new(
            r.origin.x + (cell_w - 8.0) * 0.5,
            r.origin.y + (cell_h - 8.0) * 0.5,
            8.0,
            8.0,
        );
        let mut dot = surface(dot_r, 4.0, Token::PRIMARY, desc.z + 1);
        if !visible || idx >= desc.filled {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
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

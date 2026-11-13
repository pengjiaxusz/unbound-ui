//! Star Rating control with full/half-star precision and read-only mode.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_STARS: usize = 5;

/// Interactive state for [`RatingDesc`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RatingState {
    /// Current rating (`0.0..=5.0` in `0.5` or `1.0` increments).
    pub value: f32,
    /// Preview value while hovering.
    pub hover_value: Option<f32>,
}

/// Descriptor for a [`Rating`].
#[derive(Clone, Debug, PartialEq)]
pub struct RatingDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Current rating (`0.0..=5.0`).
    pub value: f32,
    /// Whether half-star steps are allowed.
    pub allow_half: bool,
    /// Read-only display mode.
    pub read_only: bool,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl RatingDesc {
    /// Create a rating descriptor.
    #[must_use]
    pub fn new(rect: Rect, value: f32) -> Self {
        Self {
            rect,
            value: value.clamp(0.0, MAX_STARS as f32),
            allow_half: true,
            read_only: false,
            label: "Rating".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape 5-star rating (1 bg + 5 star slots + 1 focus ring = 7 primitives).
#[must_use]
pub fn emit_rating(node: u64, desc: &RatingDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(7);
    let payloads = Vec::new();

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        if state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let gap = 6.0;
    let sz = ((desc.rect.width() - gap * (MAX_STARS + 1) as f32) / MAX_STARS as f32)
        .min(desc.rect.height() - 8.0)
        .max(12.0);
    let y = desc.rect.origin.y + (desc.rect.height() - sz) * 0.5;
    for i in 0..MAX_STARS {
        let r = Rect::new(desc.rect.origin.x + gap + i as f32 * (sz + gap), y, sz, sz);
        let filled = desc.value >= i as f32 + 0.5;
        primitives.push(surface(
            r,
            sz * 0.25,
            if filled {
                Token::WARNING
            } else {
                Token::CARD_BORDER
            },
            desc.z + 1,
        ));
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    let interactive = !state.disabled && !desc.read_only;
    let hit = interactive.then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y =
        A11yNode::new(Role::Slider, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = format!("{:.1}", desc.value);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: interactive,
    }
}

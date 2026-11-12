//! The switch: a track with a knob that slides to the other end.
//!
//! The switch is the only control whose *business* state (`on`) is not a
//! pointer phase: it lives in the descriptor, while the pointer phase and the
//! focus ring still come from the router.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

/// The UI layer a control's hit region lives in.
pub const CONTROL_LAYER: u8 = 1;

/// The default track size.
pub const SWITCH_TRACK: (f32, f32) = (44.0, 24.0);

/// The default knob size.
pub const SWITCH_KNOB: f32 = 16.0;

/// The distance the knob keeps from the track's edge.
const KNOB_INSET: f32 = 4.0;

/// The colour slots a switch paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchTokens {
    /// The track while the switch is off.
    pub track: Token,
    /// The track while the switch is on.
    pub track_on: Token,
    /// The knob.
    pub thumb: Token,
    /// The label while idle.
    pub label: Token,
    /// The label while hovered.
    pub label_hover: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl SwitchTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::CONTROL_TRACK,
            track_on: Token::PRIMARY,
            thumb: Token::CONTROL_THUMB,
            label: Token::TEXT_SECONDARY,
            label_hover: Token::TEXT_PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// The pure data descriptor of a switch.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchDesc {
    /// The row the switch occupies. The track sits at its right end.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// Whether the switch is on.
    pub on: bool,
    /// The track size.
    pub track_size: (f32, f32),
    /// The knob size.
    pub knob_size: f32,
    /// The draw order key.
    pub z: u16,
}

impl SwitchDesc {
    /// A switch with the default metrics.
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            on: false,
            track_size: SWITCH_TRACK,
            knob_size: SWITCH_KNOB,
            z: 700,
        }
    }

    /// The same switch in the other position.
    #[must_use]
    pub fn with_on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// The track rectangle: right-aligned, vertically centred in the row.
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        let (width, height) = self.track_size;
        Rect::new(
            self.rect.max().x - width,
            self.rect.origin.y + (self.rect.size.y - height) * 0.5,
            width,
            height,
        )
    }

    /// The knob rectangle.
    #[must_use]
    pub fn knob_rect(&self) -> Rect {
        let track = self.track_rect();
        let x = if self.on {
            track.max().x - self.knob_size - KNOB_INSET
        } else {
            track.origin.x + KNOB_INSET
        };
        Rect::new(
            x,
            track.origin.y + (track.size.y - self.knob_size) * 0.5,
            self.knob_size,
            self.knob_size,
        )
    }
}

/// Emit the switch: track, knob, label and the focus ring.
///
/// `label` carries the glyphs the caller has already positioned; the emitter
/// never touches them.
#[must_use]
pub fn emit_switch(
    node: u64,
    desc: &SwitchDesc,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: SwitchTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;

    // The track.
    primitives.push(surface(
        desc.track_rect(),
        desc.track_size.1 * 0.5,
        if desc.on {
            tokens.track_on
        } else {
            tokens.track
        },
        z,
    ));

    // The knob.
    let knob = desc.knob_rect();
    primitives.push(outlined(
        knob,
        desc.knob_size * 0.5,
        tokens.thumb,
        Token::NONE,
        0.0,
        2.0,
        z,
    ));

    // The label.
    let label_color = if state.phase == crate::PointerPhase::Hovered {
        tokens.label_hover
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // The focus ring.
    if state.focused {
        primitives.push(focus_ring(
            knob.inflate(3.0),
            desc.knob_size * 0.5 + 3.0,
            tokens.focus_ring,
            2.0,
            z,
        ));
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded {
                radii: [desc.track_size.1 * 0.5; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Switch, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.selected = desc.on;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PointerPhase;
    use unbound_math::Vec2;

    fn desc() -> SwitchDesc {
        SwitchDesc::new(Rect::new(0.0, 0.0, 200.0, 40.0), "Notifications")
    }

    #[test]
    fn the_track_sits_at_the_right_end_of_the_row() {
        let desc = desc();
        let track = desc.track_rect();
        assert_eq!(track.max().x, 200.0);
        assert_eq!(track.size, Vec2::new(44.0, 24.0));
        assert_eq!(track.origin.y, 8.0);
    }

    #[test]
    fn the_knob_slides_between_the_two_ends() {
        let mut desc = desc();
        desc.on = false;
        let off = desc.knob_rect();
        desc.on = true;
        let on = desc.knob_rect();
        assert!(on.origin.x > off.origin.x);
        assert_eq!(on.max().x, desc.track_rect().max().x - 4.0);
        assert_eq!(off.origin.x, desc.track_rect().origin.x + 4.0);
    }

    #[test]
    fn the_track_paints_the_state_colour() {
        let tokens = SwitchTokens::standard();
        let desc = desc();
        let output = emit_switch(1, &desc, ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.track.raw() as u32
        );

        let desc = desc.with_on(true);
        let output = emit_switch(1, &desc, ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.track_on.raw() as u32
        );
    }

    #[test]
    fn a_disabled_switch_leaves_the_hit_set() {
        let tokens = SwitchTokens::standard();
        let output = emit_switch(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output.hit.is_none());
        assert!(output.a11y.is_some());
        assert!(!output.focusable);
    }

    #[test]
    fn a_focused_switch_emits_a_focus_ring() {
        let tokens = SwitchTokens::standard();
        let output = emit_switch(
            1,
            &desc(),
            ControlState {
                focused: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == unbound_gpu::data::PrimType::Stroke));
    }

    #[test]
    fn the_semantic_node_reports_the_position() {
        let tokens = SwitchTokens::standard();
        let output = emit_switch(
            1,
            &desc().with_on(true),
            ControlState::default(),
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a switch is always reachable");
        assert_eq!(a11y.role, Role::Switch);
        assert!(a11y.selected);
    }

    #[test]
    fn a_hovered_switch_brightens_its_label() {
        let tokens = SwitchTokens::standard();
        let idle = emit_switch(1, &desc(), ControlState::default(), &[], tokens);
        let hovered = emit_switch(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert_eq!(idle.primitives.len(), hovered.primitives.len());
    }
}

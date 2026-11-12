//! The checkbox: a square that carries a check mark.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default box size.
pub const CHECKBOX_BOX: f32 = 18.0;

/// The gap between the box and the label.
pub const CHECKBOX_GAP: f32 = 28.0;

/// The colour slots a checkbox paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckboxTokens {
    /// The box while unchecked.
    pub box_empty: Token,
    /// The box while hovered.
    pub box_hover: Token,
    /// The box while checked.
    pub box_checked: Token,
    /// The border of an unchecked box.
    pub border: Token,
    /// The check mark.
    pub mark: Token,
    /// The label.
    pub label: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl CheckboxTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            box_empty: Token::CARD_BG,
            box_hover: Token::CONTROL_HOVER,
            box_checked: Token::PRIMARY,
            border: Token::OUTLINE,
            mark: Token::ON_PRIMARY,
            label: Token::TEXT_PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// The pure data descriptor of a checkbox.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckboxDesc {
    /// The row the checkbox occupies.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// Whether the box is checked.
    pub checked: bool,
    /// The box size.
    pub box_size: f32,
    /// The distance between the box and the label.
    pub gap: f32,
    /// The draw order key.
    pub z: u16,
}

impl CheckboxDesc {
    /// A checkbox with the default metrics.
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            checked: false,
            box_size: CHECKBOX_BOX,
            gap: CHECKBOX_GAP,
            z: 700,
        }
    }

    /// The same checkbox in the other position.
    #[must_use]
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// The box rectangle: left-aligned, vertically centred in the row.
    #[must_use]
    pub fn box_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.y - self.box_size) * 0.5,
            self.box_size,
            self.box_size,
        )
    }

    /// The label rectangle.
    #[must_use]
    pub fn label_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + self.gap,
            self.rect.origin.y,
            (self.rect.size.x - self.gap).max(0.0),
            self.rect.size.y,
        )
    }
}

/// Emit the checkbox: box, check mark, label and the focus ring.
///
/// `mark` carries the check mark the caller has already positioned (an empty
/// run paints no mark), `label` the text next to the box.
#[must_use]
pub fn emit_checkbox(
    node: u64,
    desc: &CheckboxDesc,
    state: ControlState,
    mark: &[GlyphQuad],
    label: &[GlyphQuad],
    tokens: CheckboxTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let box_rect = desc.box_rect();

    let fill = if desc.checked {
        tokens.box_checked
    } else if state.phase == PointerPhase::Hovered {
        tokens.box_hover
    } else {
        tokens.box_empty
    };
    let border = if desc.checked {
        Token::NONE
    } else {
        tokens.border
    };
    primitives.push(outlined(box_rect, 4.0, fill, border, 1.0, 0.0, z));

    // The check mark, then the label.
    glyphs(mark, tokens.mark, z, &mut primitives, &mut payloads);
    glyphs(label, tokens.label, z, &mut primitives, &mut payloads);

    if state.focused {
        primitives.push(focus_ring(
            box_rect.inflate(3.0),
            7.0,
            tokens.focus_ring,
            2.0,
            z,
        ));
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::CheckBox, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.selected = desc.checked;

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

    fn desc() -> CheckboxDesc {
        CheckboxDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Enable telemetry")
    }

    #[test]
    fn the_box_is_left_aligned_and_centred() {
        let desc = desc();
        let rect = desc.box_rect();
        assert_eq!(rect.origin.x, 0.0);
        assert_eq!(rect.origin.y, 7.0);
        assert_eq!(rect.size.x, 18.0);
    }

    #[test]
    fn an_unchecked_box_keeps_its_border() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(1, &desc(), ControlState::default(), &[], &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_empty.raw() as u32
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border.raw() as u32
        );
    }

    #[test]
    fn a_checked_box_fills_and_drops_the_border() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc().with_checked(true),
            ControlState::default(),
            &[],
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_checked.raw() as u32
        );
        assert_eq!(output.primitives[0].border_token_or_tex, 0);
    }

    #[test]
    fn a_hovered_box_changes_its_fill() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_hover.raw() as u32
        );
    }

    #[test]
    fn the_semantic_node_reports_the_checked_state() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc().with_checked(true),
            ControlState::default(),
            &[],
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a checkbox is always reachable");
        assert_eq!(a11y.role, Role::CheckBox);
        assert!(a11y.selected);
    }

    #[test]
    fn a_disabled_checkbox_leaves_the_hit_set() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            &[],
            tokens,
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

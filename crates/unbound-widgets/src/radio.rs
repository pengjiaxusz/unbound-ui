//! The radio button: a circle with a dot in the middle.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default circle size.
pub const RADIO_SIZE: f32 = 18.0;

/// The gap between the circle and the label.
pub const RADIO_GAP: f32 = 28.0;

/// The colour slots a radio button paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadioTokens {
    /// The circle.
    pub circle: Token,
    /// The border of an unselected circle.
    pub border: Token,
    /// The border of a selected circle.
    pub border_selected: Token,
    /// The dot.
    pub dot: Token,
    /// The circle of a disabled button.
    pub circle_disabled: Token,
    /// The border of a disabled circle.
    pub border_disabled: Token,
    /// The dot of a disabled, selected button.
    pub dot_disabled: Token,
    /// The label while idle.
    pub label: Token,
    /// The label while hovered.
    pub label_hover: Token,
    /// The label of a disabled button.
    pub label_disabled: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl RadioTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            circle: Token::CARD_BG,
            border: Token::OUTLINE,
            border_selected: Token::PRIMARY,
            dot: Token::PRIMARY,
            circle_disabled: Token::CONTROL_DISABLED_BG,
            border_disabled: Token::CONTROL_DISABLED_BORDER,
            dot_disabled: Token::CONTROL_DISABLED_THUMB,
            label: Token::TEXT_SECONDARY,
            label_hover: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_DISABLED,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// The pure data descriptor of a radio button.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioDesc {
    /// The row the radio button occupies.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// Whether this option is the selected one.
    pub selected: bool,
    /// The circle size.
    pub size: f32,
    /// The distance between the circle and the label.
    pub gap: f32,
    /// The draw order key.
    pub z: u16,
}

impl RadioDesc {
    /// A radio button with the default metrics.
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            selected: false,
            size: RADIO_SIZE,
            gap: RADIO_GAP,
            z: 700,
        }
    }

    /// The same radio button in the other position.
    #[must_use]
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The circle rectangle: left-aligned, vertically centred in the row.
    #[must_use]
    pub fn circle_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.y - self.size) * 0.5,
            self.size,
            self.size,
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

/// A key a radio group understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioGroupKey {
    /// Move to the previous option.
    Up,
    /// Move to the next option.
    Down,
    /// Move to the previous option.
    Left,
    /// Move to the next option.
    Right,
}

/// A radio group: the options plus the roving selection index.
///
/// The group is the *only* owner of the selection. A radio button never holds
/// focus by itself: exactly one option — the selected one — enters the tab
/// order, and the arrow keys move both the selection and the roving tab stop.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioGroupDesc {
    /// The options, in visual order.
    pub items: Vec<RadioDesc>,
    /// The selected option, if any.
    pub selected: Option<usize>,
}

impl RadioGroupDesc {
    /// A group over `items`, taking their current selection.
    #[must_use]
    pub fn new(items: Vec<RadioDesc>) -> Self {
        let selected = items.iter().position(|item| item.selected);
        Self { items, selected }
    }

    /// The number of options.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the group has no options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The roving tab stop: the selected option, or the first one when the
    /// group has never been selected.
    ///
    /// Radio buttons do not all take part in the tab order — Tab enters the
    /// group once and the arrow keys move inside it.
    #[must_use]
    pub fn focus_order(&self) -> Vec<usize> {
        if self.items.is_empty() {
            return Vec::new();
        }
        vec![self.selected.unwrap_or(0).min(self.items.len() - 1)]
    }
}

/// What one reduction produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RadioGroupOutcome {
    /// The selection after the key.
    pub selected: Option<usize>,
    /// Whether the selection moved.
    pub changed: bool,
}

/// Evolve a radio group for one arrow key, wrapping at both ends.
///
/// Up and Left select the previous option, Down and Right the next one. When
/// nothing is selected yet, the first key selects the first option.
pub fn reduce_radio_group(group: &mut RadioGroupDesc, key: RadioGroupKey) -> RadioGroupOutcome {
    let len = group.items.len();
    if len == 0 {
        return RadioGroupOutcome::default();
    }
    let next = match group.selected {
        None => 0,
        Some(current) => {
            let current = current.min(len - 1);
            match key {
                RadioGroupKey::Up | RadioGroupKey::Left => (current + len - 1) % len,
                RadioGroupKey::Down | RadioGroupKey::Right => (current + 1) % len,
            }
        }
    };
    let changed = group.selected != Some(next);
    group.selected = Some(next);
    for (index, item) in group.items.iter_mut().enumerate() {
        item.selected = index == next;
    }
    RadioGroupOutcome {
        selected: Some(next),
        changed,
    }
}

/// Emit the radio button: circle, dot, label and the focus ring.
#[must_use]
pub fn emit_radio(
    node: u64,
    desc: &RadioDesc,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: RadioTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let circle = desc.circle_rect();

    primitives.push(outlined(
        circle,
        desc.size * 0.5,
        if state.disabled {
            tokens.circle_disabled
        } else {
            tokens.circle
        },
        if state.disabled {
            tokens.border_disabled
        } else if desc.selected {
            tokens.border_selected
        } else {
            tokens.border
        },
        2.0,
        0.0,
        z,
    ));

    // The dot: a smaller circle in the middle.
    if desc.selected {
        primitives.push(surface(
            circle.inflate(-5.0),
            4.0,
            if state.disabled {
                tokens.dot_disabled
            } else {
                tokens.dot
            },
            z,
        ));
    }

    let label_color = if state.disabled {
        tokens.label_disabled
    } else if state.phase == PointerPhase::Hovered {
        tokens.label_hover
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // The ring is always emitted: focus changes its opacity, not the
    // pool's shape, so focus is a paint change and never a re-upload.
    primitives.push(focus_ring(
        circle.inflate(3.0),
        desc.size * 0.5 + 3.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded {
                radii: [desc.size * 0.5; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::RadioButton, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.selected = desc.selected;

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

    fn desc() -> RadioDesc {
        RadioDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Solid")
    }

    #[test]
    fn an_unselected_circle_keeps_the_plain_border() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(1, &desc(), ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border.raw() as u32
        );
        // The circle and its focus ring — the ring is always emitted, and
        // opaque only when the control has focus.
        assert_eq!(output.primitives.len(), 2);
        assert!(
            output
                .primitives
                .iter()
                .all(|primitive| primitive.fill_token_or_uv != tokens.dot.raw() as u32),
            "没有选中就没有圆点"
        );
    }

    #[test]
    fn a_selected_circle_paints_a_dot() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(
            1,
            &desc().with_selected(true),
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border_selected.raw() as u32
        );
        // The circle, the dot and the focus ring.
        assert_eq!(output.primitives.len(), 3);
        assert_eq!(
            output.primitives[1].fill_token_or_uv,
            tokens.dot.raw() as u32
        );
    }

    #[test]
    fn the_circle_is_left_aligned_and_centred() {
        let desc = desc();
        let circle = desc.circle_rect();
        assert_eq!(circle.origin.x, 0.0);
        assert_eq!(circle.origin.y, 7.0);
    }

    #[test]
    fn the_semantic_node_reports_the_selection() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(
            1,
            &desc().with_selected(true),
            ControlState::default(),
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a radio button is always reachable");
        assert_eq!(a11y.role, Role::RadioButton);
        assert!(a11y.selected);
    }

    #[test]
    fn a_disabled_radio_button_leaves_the_hit_set() {
        let tokens = RadioTokens::standard();
        let output = emit_radio(
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
        assert!(!output.focusable);
    }
}

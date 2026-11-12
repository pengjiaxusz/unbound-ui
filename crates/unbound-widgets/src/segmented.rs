//! The segmented control: a row of slots with one of them selected.
//!
//! Every slot is its own control — its own identity, its own hit region and
//! its own semantic node — but they share one descriptor, so the geometry of
//! the row cannot drift away from the geometry of its slots.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The inset of a slot inside its cell.
pub const SEGMENT_INSET: f32 = 2.0;

/// The colour slots a segmented control paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentedTokens {
    /// The frame behind the slots.
    pub frame: Token,
    /// The selected slot.
    pub selected: Token,
    /// The border of the selected slot.
    pub selected_border: Token,
    /// A hovered slot.
    pub hover: Token,
    /// The label of the selected slot.
    pub label_selected: Token,
    /// The label of an unselected slot.
    pub label: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl SegmentedTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            frame: Token::CONTROL_TRACK,
            selected: Token::PRIMARY_SOFT,
            selected_border: Token::PRIMARY,
            hover: Token::CONTROL_HOVER,
            label_selected: Token::PRIMARY,
            label: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// The pure data descriptor of a segmented control.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentedDesc {
    /// The frame rectangle.
    pub rect: Rect,
    /// The slot labels.
    pub labels: Vec<String>,
    /// The index of the selected slot.
    pub selected: usize,
    /// The draw order key.
    pub z: u16,
}

impl SegmentedDesc {
    /// A segmented control with the given labels.
    #[must_use]
    pub fn new(rect: Rect, labels: &[&str]) -> Self {
        Self {
            rect,
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            selected: 0,
            z: 700,
        }
    }

    /// The same control with another slot selected.
    #[must_use]
    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    /// How many slots the control has.
    #[must_use]
    pub fn count(&self) -> usize {
        self.labels.len().max(1)
    }

    /// The cell of one slot: the frame is split into equal columns.
    #[must_use]
    pub fn cell_rect(&self, index: usize) -> Rect {
        let count = self.count();
        let width = self.rect.size.x / count as f32;
        Rect::new(
            self.rect.origin.x + index as f32 * width,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }

    /// The painted rectangle of one slot, inset from its cell.
    #[must_use]
    pub fn segment_rect(&self, index: usize) -> Rect {
        let cell = self.cell_rect(index);
        Rect::new(
            cell.origin.x + SEGMENT_INSET,
            cell.origin.y + SEGMENT_INSET,
            (cell.size.x - SEGMENT_INSET * 2.0).max(8.0),
            (cell.size.y - SEGMENT_INSET * 2.0).max(8.0),
        )
    }
}

/// Emit the frame that holds the slots.
///
/// The frame carries no hit region: only the slots themselves are pickable.
#[must_use]
pub fn emit_segmented_frame(desc: &SegmentedDesc, tokens: SegmentedTokens) -> WidgetOutput {
    let primitives = vec![surface(desc.rect, 7.0, tokens.frame, desc.z)];
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
    }
}

/// Emit one slot of a segmented control.
///
/// `label` carries the glyphs the caller has already positioned inside the
/// slot.
#[must_use]
pub fn emit_segmented_segment(
    node: u64,
    desc: &SegmentedDesc,
    index: usize,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: SegmentedTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let segment = desc.segment_rect(index);
    let active = index == desc.selected;

    // One pill, always. The active slot paints it as a raised, bordered
    // surface; a hovered slot paints the hover fill and no border; an idle
    // slot paints nothing at all. Because the primitive exists in every
    // state, hovering a slot patches one instance instead of renumbering the
    // pool.
    let (fill, border, border_width) = if active {
        (tokens.selected, tokens.selected_border, 1.0)
    } else if state.phase == PointerPhase::Hovered {
        (tokens.hover, Token::NONE, 0.0)
    } else {
        (Token::NONE, Token::NONE, 0.0)
    };
    primitives.push(outlined(segment, 5.0, fill, border, border_width, 0.0, z));

    let label_color = if active {
        tokens.label_selected
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // The ring is always emitted: focus changes its opacity, not the
    // pool's shape, so focus is a paint change and never a re-upload.
    primitives.push(focus_ring(
        segment.inflate(2.0),
        7.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), segment)
            .with_shape(Shape::Rounded { radii: [5.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let label_text = desc.labels.get(index).cloned().unwrap_or_default();
    let mut a11y = A11yNode::new(Role::Tab, label_text, segment)
        .clickable()
        .disabled(state.disabled);
    a11y.selected = active;

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

    fn desc() -> SegmentedDesc {
        SegmentedDesc::new(
            Rect::new(0.0, 0.0, 300.0, 32.0),
            &["Light", "Dark", "System"],
        )
    }

    #[test]
    fn the_slots_split_the_frame_into_equal_columns() {
        let desc = desc();
        assert_eq!(desc.count(), 3);
        assert_eq!(desc.cell_rect(0).size.x, 100.0);
        assert_eq!(desc.cell_rect(1).origin.x, 100.0);
        assert_eq!(desc.cell_rect(2).origin.x, 200.0);
    }

    #[test]
    fn the_slot_is_inset_from_its_cell() {
        let desc = desc();
        let segment = desc.segment_rect(0);
        assert_eq!(segment.origin.x, 2.0);
        assert_eq!(segment.size.x, 96.0);
    }

    #[test]
    fn the_selected_slot_is_filled_and_bordered() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_segment(
            1,
            &desc().with_selected(1),
            1,
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.selected.raw() as u32
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.selected_border.raw() as u32
        );
    }

    #[test]
    fn an_unselected_slot_paints_nothing_on_its_own() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_segment(
            1,
            &desc().with_selected(1),
            0,
            ControlState::default(),
            &[],
            tokens,
        );
        // An idle slot paints nothing, but it still emits its two
        // primitives — the invisible pill and the invisible ring — so that
        // hovering it patches fields instead of inserting primitives.
        assert_eq!(output.primitives.len(), 2);
        assert!(
            output
                .primitives
                .iter()
                .all(|primitive| primitive.fill_token_or_uv == Token::NONE.raw() as u32),
            "an idle slot must not paint a fill"
        );
    }

    #[test]
    fn a_hovered_unselected_slot_paints_the_hover_fill() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_segment(
            1,
            &desc().with_selected(1),
            0,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.hover.raw() as u32
        );
    }

    #[test]
    fn the_frame_carries_no_hit_region() {
        let tokens = SegmentedTokens::standard();
        let output = emit_segmented_frame(&desc(), tokens);
        assert!(output.hit.is_none());
        assert!(output.a11y.is_none());
        assert_eq!(output.primitives.len(), 1);
    }

    #[test]
    fn every_slot_is_its_own_control() {
        let tokens = SegmentedTokens::standard();
        let desc = desc();
        for index in 0..desc.count() {
            let output = emit_segmented_segment(
                10 + index as u64,
                &desc,
                index,
                ControlState::default(),
                &[],
                tokens,
            );
            let hit = output.hit.expect("每个槽位都可点击");
            assert_eq!(hit.node, 10 + index as u64);
            let a11y = output.a11y.expect("每个槽位都有语义节点");
            assert_eq!(a11y.role, Role::Tab);
        }
    }
}

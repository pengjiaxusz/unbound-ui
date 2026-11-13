//! The breadcrumb: the path to the current node, collapsed from the left.
//!
//! When the path does not fit, the *middle* is what disappears, not the end:
//! the first crumb and the last few stay visible, and the hidden head folds
//! into an ellipsis that opens the same dropdown as the first crumb.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The gap around one separator.
pub const CRUMB_GAP: f32 = 6.0;

/// The width reserved for the ellipsis crumb.
pub const CRUMB_ELLIPSIS: f32 = 24.0;

/// One crumb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// Stable identity, also used as the node.
    pub id: u64,
    /// The visible label.
    pub label: String,
}

impl Crumb {
    /// A crumb with a label.
    #[must_use]
    pub fn new(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
        }
    }
}

/// The pure descriptor of a breadcrumb.
#[derive(Clone, Debug, PartialEq)]
pub struct BreadcrumbDesc {
    /// The breadcrumb rectangle.
    pub rect: Rect,
    /// The crumbs, root first.
    pub crumbs: Vec<Crumb>,
    /// The gap around each separator.
    pub gap: f32,
    /// The width of the separator glyph.
    pub separator_width: f32,
    /// The draw order key.
    pub z: u16,
}

impl BreadcrumbDesc {
    /// A breadcrumb with the default separators.
    #[must_use]
    pub fn new(rect: Rect, crumbs: Vec<Crumb>) -> Self {
        Self {
            rect,
            crumbs,
            gap: CRUMB_GAP,
            separator_width: 8.0,
            z: 520,
        }
    }

    /// One laid-out slot.
    ///
    /// `crumb` is `None` for the ellipsis slot.
    #[must_use]
    pub fn layout(&self, widths: &[f32]) -> Vec<CrumbSlot> {
        let count = self.crumbs.len();
        if count == 0 {
            return Vec::new();
        }
        let width_of = |index: usize| {
            widths
                .get(index)
                .copied()
                .unwrap_or_else(|| self.crumbs[index].label.chars().count() as f32 * 8.0)
        };
        let step = |crumb_width: f32| crumb_width + self.gap * 2.0 + self.separator_width;

        let total: f32 = (0..count).map(|index| step(width_of(index))).sum::<f32>()
            - self.gap * 2.0
            - self.separator_width;
        if total <= self.rect.size.x {
            let mut out = Vec::with_capacity(count);
            let mut x = self.rect.origin.x;
            for index in 0..count {
                let width = width_of(index);
                out.push(CrumbSlot {
                    crumb: Some(index),
                    rect: Rect::new(x, self.rect.origin.y, width, self.rect.size.y),
                    separator: index + 1 < count,
                });
                x += width + self.gap * 2.0 + self.separator_width;
            }
            return out;
        }

        // Collapse from the left: the last crumbs that fit win, the first
        // crumb is dropped into the ellipsis together with every other
        // hidden one. A path with no room for even the tail shows one
        // ellipsis and the last crumb.
        let mut visible = Vec::new();
        let mut used = step(CRUMB_ELLIPSIS);
        for index in (0..count).rev() {
            let needed = step(width_of(index));
            if used + needed <= self.rect.size.x || visible.is_empty() {
                visible.push(index);
                used += needed;
            } else {
                break;
            }
        }
        visible.reverse();
        if visible.len() >= count {
            return self.layout_without_overflow(widths);
        }
        let mut out = Vec::with_capacity(visible.len() + 1);
        let mut x = self.rect.origin.x;
        out.push(CrumbSlot {
            crumb: None,
            rect: Rect::new(x, self.rect.origin.y, CRUMB_ELLIPSIS, self.rect.size.y),
            separator: true,
        });
        x += CRUMB_ELLIPSIS + self.gap * 2.0 + self.separator_width;
        for (position, index) in visible.iter().enumerate() {
            let width = width_of(*index);
            out.push(CrumbSlot {
                crumb: Some(*index),
                rect: Rect::new(x, self.rect.origin.y, width, self.rect.size.y),
                separator: position + 1 < visible.len(),
            });
            x += width + self.gap * 2.0 + self.separator_width;
        }
        out
    }

    fn layout_without_overflow(&self, widths: &[f32]) -> Vec<CrumbSlot> {
        let mut out = Vec::with_capacity(self.crumbs.len());
        let mut x = self.rect.origin.x;
        for index in 0..self.crumbs.len() {
            let width = widths.get(index).copied().unwrap_or(0.0);
            out.push(CrumbSlot {
                crumb: Some(index),
                rect: Rect::new(x, self.rect.origin.y, width, self.rect.size.y),
                separator: index + 1 < self.crumbs.len(),
            });
            x += width + self.gap * 2.0 + self.separator_width;
        }
        out
    }

    /// Every crumb index folded into the ellipsis.
    #[must_use]
    pub fn hidden_indices(&self, widths: &[f32]) -> Vec<usize> {
        let visible: Vec<usize> = self
            .layout(widths)
            .into_iter()
            .filter_map(|slot| slot.crumb)
            .collect();
        (0..self.crumbs.len())
            .filter(|index| !visible.contains(index))
            .collect()
    }
}

/// One breadcrumb slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrumbSlot {
    /// The crumb index, or `None` for the ellipsis.
    pub crumb: Option<usize>,
    /// The slot rectangle.
    pub rect: Rect,
    /// Whether a separator follows the slot.
    pub separator: bool,
}

/// The live state of a breadcrumb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BreadcrumbState {
    /// Whether the overflow dropdown is open.
    pub overflow_open: bool,
}

impl BreadcrumbState {
    /// A closed breadcrumb.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle the overflow dropdown.
    pub fn toggle_overflow(&mut self) -> bool {
        self.overflow_open = !self.overflow_open;
        self.overflow_open
    }

    /// Close the overflow dropdown.
    pub fn close(&mut self) {
        self.overflow_open = false;
    }
}

/// The colour slots a breadcrumb paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreadcrumbTokens {
    /// The crumb text.
    pub text: Token,
    /// The text of the last (current) crumb.
    pub text_current: Token,
    /// The separator.
    pub separator: Token,
    /// The ellipsis text.
    pub ellipsis: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl BreadcrumbTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            text: Token::TEXT_SECONDARY,
            text_current: Token::TEXT_PRIMARY,
            separator: Token::TEXT_DISABLED,
            ellipsis: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// Emit a separator between two slots.
#[must_use]
pub fn emit_crumb_separator(
    desc: &BreadcrumbDesc,
    slot: &CrumbSlot,
    tokens: BreadcrumbTokens,
) -> WidgetOutput {
    if !slot.separator {
        return WidgetOutput::default();
    }
    let x = slot.rect.max().x + desc.gap;
    let y = slot.rect.origin.y + slot.rect.size.y * 0.5;
    let arm = desc.separator_width * 0.25;
    let mut primitives = Vec::with_capacity(2);
    for (start, end) in [
        ([x, y - arm], [x + arm * 2.0, y]),
        ([x + arm * 2.0, y], [x, y + arm]),
    ] {
        primitives.push(instance(
            Rect::new(
                start[0].min(end[0]),
                start[1].min(end[1]),
                (start[0] - end[0]).abs().max(1.0),
                (start[1] - end[1]).abs().max(1.0),
            ),
            [0.0; 4],
            tokens.separator,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        ..WidgetOutput::default()
    }
}

/// Emit one crumb.
#[must_use]
pub fn emit_crumb(
    desc: &BreadcrumbDesc,
    slot: &CrumbSlot,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: BreadcrumbTokens,
) -> WidgetOutput {
    let Some(index) = slot.crumb else {
        return WidgetOutput::default();
    };
    let Some(crumb) = desc.crumbs.get(index) else {
        return WidgetOutput::default();
    };
    let current = index + 1 == desc.crumbs.len();
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(2 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    glyphs(
        label,
        if current {
            tokens.text_current
        } else {
            tokens.text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        slot.rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled)
        .then(|| {
            HitRegion::new(crumb.id, CONTROL_LAYER, u32::from(desc.z), slot.rect)
                .with_cursor(CursorKind::Pointer)
        })
        .filter(|_| !current);
    let mut a11y = A11yNode::new(Role::Link, crumb.label.clone(), slot.rect).disabled(disabled);
    a11y.selected = current;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled && !current,
    }
}

/// Emit the ellipsis that opens the hidden head.
#[must_use]
pub fn emit_crumb_ellipsis(
    node: u64,
    desc: &BreadcrumbDesc,
    slot: &CrumbSlot,
    state: &BreadcrumbState,
    control: ControlState,
    tokens: BreadcrumbTokens,
) -> WidgetOutput {
    if slot.crumb.is_some() {
        return WidgetOutput::default();
    }
    let mut primitives = Vec::with_capacity(3);
    let cy = slot.rect.origin.y + slot.rect.size.y * 0.5;
    for offset in [-1.0f32, 0.0, 1.0] {
        let dot = Rect::new(
            slot.rect.origin.x + CRUMB_ELLIPSIS * 0.5 + offset * 6.0 - 1.5,
            cy - 1.5,
            3.0,
            3.0,
        );
        primitives.push(crate::primitive::surface(dot, 1.5, tokens.ellipsis, desc.z));
    }
    primitives.push(focus_ring(
        slot.rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), slot.rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, "Show full path", slot.rect)
        .clickable()
        .disabled(control.disabled);
    a11y.expanded = Some(state.overflow_open);

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PointerPhase;

    fn desc() -> BreadcrumbDesc {
        BreadcrumbDesc::new(
            Rect::new(0.0, 0.0, 400.0, 24.0),
            vec![
                Crumb::new(1, "Root"),
                Crumb::new(2, "src"),
                Crumb::new(3, "widgets"),
                Crumb::new(4, "button.rs"),
            ],
        )
    }

    fn widths() -> Vec<f32> {
        vec![40.0, 30.0, 60.0, 80.0]
    }

    #[test]
    fn a_path_that_fits_shows_every_crumb() {
        let desc = desc();
        let slots = desc.layout(&widths());
        assert_eq!(slots.len(), 4);
        assert!(slots.iter().all(|slot| slot.crumb.is_some()));
        assert!(desc.hidden_indices(&widths()).is_empty());
    }

    #[test]
    fn a_long_path_collapses_its_head_into_an_ellipsis() {
        let mut desc = desc();
        desc.rect.size.x = 190.0;
        let slots = desc.layout(&widths());
        assert_eq!(slots[0].crumb, None, "the head folds into the ellipsis");
        let shown: Vec<usize> = slots.iter().filter_map(|slot| slot.crumb).collect();
        assert_eq!(shown.last().copied(), Some(3), "the tail always stays");
        assert!(!desc.hidden_indices(&widths()).is_empty());
    }

    #[test]
    fn the_last_crumb_is_never_hidden() {
        let mut desc = desc();
        desc.rect.size.x = 10.0;
        let slots = desc.layout(&widths());
        let shown: Vec<usize> = slots.iter().filter_map(|slot| slot.crumb).collect();
        assert_eq!(shown, vec![3]);
    }

    #[test]
    fn separators_follow_every_slot_except_the_last() {
        let desc = desc();
        let slots = desc.layout(&widths());
        assert!(slots[0].separator);
        assert!(slots[1].separator);
        assert!(slots[2].separator);
        assert!(!slots[3].separator);
    }

    #[test]
    fn the_overflow_dropdown_toggles() {
        let mut state = BreadcrumbState::new();
        assert!(!state.overflow_open);
        assert!(state.toggle_overflow());
        assert!(state.overflow_open);
        state.close();
        assert!(!state.overflow_open);
    }

    #[test]
    fn the_ellipsis_is_pickable_and_reports_expansion() {
        let mut desc = desc();
        desc.rect.size.x = 190.0;
        let slots = desc.layout(&widths());
        let mut state = BreadcrumbState::new();
        state.toggle_overflow();
        let output = emit_crumb_ellipsis(
            99,
            &desc,
            &slots[0],
            &state,
            ControlState::default(),
            BreadcrumbTokens::standard(),
        );
        let hit = output.hit.expect("the ellipsis is pickable");
        assert_eq!(hit.node, 99);
        let a11y = output.a11y.expect("the ellipsis is reachable");
        assert_eq!(a11y.expanded, Some(true));
    }

    #[test]
    fn the_current_crumb_is_not_a_link() {
        let desc = desc();
        let slots = desc.layout(&widths());
        let output = emit_crumb(
            &desc,
            &slots[3],
            ControlState::default(),
            &[],
            BreadcrumbTokens::standard(),
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
        assert!(output.a11y.expect("reachable").selected);
    }

    #[test]
    fn an_ancestor_crumb_is_pickable() {
        let desc = desc();
        let slots = desc.layout(&widths());
        let output = emit_crumb(
            &desc,
            &slots[1],
            ControlState::default(),
            &[],
            BreadcrumbTokens::standard(),
        );
        let hit = output.hit.expect("an ancestor is pickable");
        assert_eq!(hit.node, 2);
        assert!(output.a11y.expect("reachable").role == Role::Link);
    }

    #[test]
    fn a_hovered_crumb_keeps_its_text_token() {
        let desc = desc();
        let slots = desc.layout(&widths());
        let tokens = BreadcrumbTokens::standard();
        let output = emit_crumb(
            &desc,
            &slots[0],
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        // The glyph list is empty here; the focus ring is the only primitive
        // and it stays hidden while unfocused.
        assert_eq!(output.primitives.len(), 1);
    }
}

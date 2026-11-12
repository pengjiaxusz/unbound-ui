//! Overlay placement and the popup stack.
//!
//! A popup has two problems that the rest of the widget layer does not:
//!
//! 1. **It must never leave the window.** [`place_anchored_popup`] is the pure
//!    answer: it prefers the requested side, flips when that side has no room,
//!    and finally shifts the popup back inside the viewport.
//! 2. **It must stack, dismiss and clean up in the right order.** The
//!    [`OverlayStack`] hands out monotonically increasing sort keys, closes
//!    the top layer on `Escape` or an outside click, and prunes entries whose
//!    anchor vanished in the same frame — a popup must never outlive its
//!    trigger and leave a phantom hit region behind.

use unbound_gpu::data::PassLayer;
use unbound_math::{Rect, Vec2};

/// The gap between an anchor and its popup, in logical pixels.
pub const POPUP_GAP: f32 = 4.0;

/// Where a popup would like to appear relative to its anchor.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// Below the anchor, aligned to its start edge.
    #[default]
    BottomStart,
    /// Below the anchor, aligned to its end edge.
    BottomEnd,
    /// Above the anchor, aligned to its start edge.
    TopStart,
    /// Above the anchor, aligned to its end edge.
    TopEnd,
    /// To the right of the anchor, aligned to its top edge.
    RightStart,
    /// To the left of the anchor, aligned to its top edge.
    LeftStart,
}

impl Placement {
    /// The opposite side, used after a flip.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Self::BottomStart => Self::TopStart,
            Self::BottomEnd => Self::TopEnd,
            Self::TopStart => Self::BottomStart,
            Self::TopEnd => Self::BottomEnd,
            Self::RightStart => Self::LeftStart,
            Self::LeftStart => Self::RightStart,
        }
    }

    const fn vertical(self) -> bool {
        matches!(
            self,
            Self::BottomStart | Self::BottomEnd | Self::TopStart | Self::TopEnd
        )
    }

    const fn below(self) -> bool {
        matches!(self, Self::BottomStart | Self::BottomEnd)
    }

    const fn right(self) -> bool {
        matches!(self, Self::RightStart)
    }

    const fn aligned_to_end(self) -> bool {
        matches!(self, Self::BottomEnd | Self::TopEnd)
    }
}

/// Place a popup next to an anchor, always inside the viewport.
///
/// The returned rectangle is never larger than the viewport: a popup that does
/// not fit is shrunk first, then positioned. Flipping and shifting make the
/// result satisfy `Rule::ViewportContainment` for every anchor on or near an
/// edge.
#[must_use]
pub fn place_anchored_popup(
    anchor: Rect,
    popup_size: Vec2,
    viewport: Rect,
    placement: Placement,
) -> Rect {
    let width = popup_size.x.max(0.0).min(viewport.width().max(0.0));
    let height = popup_size.y.max(0.0).min(viewport.height().max(0.0));

    let mut origin = if placement.vertical() {
        let x = if placement.aligned_to_end() {
            anchor.max().x - width
        } else {
            anchor.origin.x
        };
        let y = if placement.below() {
            anchor.max().y + POPUP_GAP
        } else {
            anchor.origin.y - POPUP_GAP - height
        };
        Vec2::new(x, y)
    } else {
        let x = if placement.right() {
            anchor.max().x + POPUP_GAP
        } else {
            anchor.origin.x - POPUP_GAP - width
        };
        Vec2::new(x, anchor.origin.y)
    };

    // Flip when the preferred side overflows and the opposite side fits.
    if placement.vertical() {
        let overflows = if placement.below() {
            origin.y + height > viewport.max().y
        } else {
            origin.y < viewport.origin.y
        };
        if overflows {
            let flipped_y = if placement.below() {
                anchor.origin.y - POPUP_GAP - height
            } else {
                anchor.max().y + POPUP_GAP
            };
            if flipped_y >= viewport.origin.y && flipped_y + height <= viewport.max().y {
                origin.y = flipped_y;
            }
        }
    } else {
        let overflows = if placement.right() {
            origin.x + width > viewport.max().x
        } else {
            origin.x < viewport.origin.x
        };
        if overflows {
            let flipped_x = if placement.right() {
                anchor.origin.x - POPUP_GAP - width
            } else {
                anchor.max().x + POPUP_GAP
            };
            if flipped_x >= viewport.origin.x && flipped_x + width <= viewport.max().x {
                origin.x = flipped_x;
            }
        }
    }

    // Shift the remaining overflow back inside.
    let max_x = (viewport.max().x - width).max(viewport.origin.x);
    let max_y = (viewport.max().y - height).max(viewport.origin.y);
    origin.x = origin.x.clamp(viewport.origin.x, max_x);
    origin.y = origin.y.clamp(viewport.origin.y, max_y);

    Rect::new(origin.x, origin.y, width, height)
}

/// A request for the render graph's backdrop-blur pass.
///
/// Components that want frosted glass behind them return one of these. The
/// caller collects the requests into a frame; an empty list means the graph
/// skips the blur passes entirely.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BackdropRequest {
    /// The node that owns the effect.
    pub node: u64,
    /// The rectangle to blur behind.
    pub rect: Rect,
    /// The blur radius, in logical pixels.
    pub blur: f32,
}

impl BackdropRequest {
    /// A request, or `None` when the radius is zero.
    #[must_use]
    pub fn new(node: u64, rect: Rect, blur: f32) -> Option<Self> {
        (blur > 0.0 && !rect.is_empty()).then_some(Self { node, rect, blur })
    }
}

/// What kind of overlay an entry is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// A dropdown, select or popover.
    Popover,
    /// A context / dropdown menu.
    Menu,
    /// A non-interactive tooltip.
    Tooltip,
    /// A modal dialog with a scrim.
    Modal,
}

impl OverlayKind {
    /// The pass layer this kind paints into.
    #[must_use]
    pub const fn layer(self) -> PassLayer {
        match self {
            Self::Popover | Self::Menu | Self::Tooltip => PassLayer::Overlay,
            Self::Modal => PassLayer::Modal,
        }
    }

    /// Whether an outside click dismisses it.
    #[must_use]
    pub const fn light_dismiss(self) -> bool {
        !matches!(self, Self::Modal)
    }

    /// Whether `Escape` closes it.
    #[must_use]
    pub const fn escape_closes(self) -> bool {
        !matches!(self, Self::Tooltip)
    }
}

/// One entry in the overlay stack.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayEntry {
    /// The popup's own node id.
    pub node: u64,
    /// The trigger it is anchored to, if any.
    pub anchor: Option<u64>,
    /// The kind, which decides the layer and dismissal policy.
    pub kind: OverlayKind,
    /// The popup bounds in window space.
    pub bounds: Rect,
    /// The focusable nodes a modal traps `Tab` inside.
    pub focus_scope: Vec<u64>,
}

impl OverlayEntry {
    /// A popover anchored to a trigger.
    #[must_use]
    pub fn popover(node: u64, anchor: u64, bounds: Rect) -> Self {
        Self {
            node,
            anchor: Some(anchor),
            kind: OverlayKind::Popover,
            bounds,
            focus_scope: Vec::new(),
        }
    }

    /// A menu anchored to a trigger.
    #[must_use]
    pub fn menu(node: u64, anchor: u64, bounds: Rect) -> Self {
        Self {
            node,
            anchor: Some(anchor),
            kind: OverlayKind::Menu,
            bounds,
            focus_scope: Vec::new(),
        }
    }

    /// A tooltip anchored to a trigger.
    #[must_use]
    pub fn tooltip(node: u64, anchor: u64, bounds: Rect) -> Self {
        Self {
            node,
            anchor: Some(anchor),
            kind: OverlayKind::Tooltip,
            bounds,
            focus_scope: Vec::new(),
        }
    }

    /// A modal dialog with a focus scope.
    #[must_use]
    pub fn modal(node: u64, bounds: Rect, focus_scope: Vec<u64>) -> Self {
        Self {
            node,
            anchor: None,
            kind: OverlayKind::Modal,
            bounds,
            focus_scope,
        }
    }
}

/// The popup stack.
///
/// Entry order is insertion order. A later entry paints above an earlier one
/// when their layers are equal, and `PassLayer::Modal` always paints above
/// `PassLayer::Overlay` so a select inside a dialog stays inside the dialog's
/// visual layer.
#[derive(Clone, Debug, Default)]
pub struct OverlayStack {
    entries: Vec<OverlayEntry>,
    order: u32,
}

impl OverlayStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an overlay and return its ordering key.
    pub fn push(&mut self, entry: OverlayEntry) -> u64 {
        self.order = self.order.wrapping_add(1);
        let order = self.order;
        self.entries.push(entry);
        self.sort_key(self.entries.len() - 1, order)
    }

    /// The topmost entry.
    #[must_use]
    pub fn top(&self) -> Option<&OverlayEntry> {
        self.entries.iter().max_by_key(|entry| self.key_of(entry))
    }

    /// How many overlays are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry, bottom first.
    #[must_use]
    pub fn entries(&self) -> &[OverlayEntry] {
        &self.entries
    }

    /// Whether a node is open.
    #[must_use]
    pub fn contains(&self, node: u64) -> bool {
        self.entries.iter().any(|entry| entry.node == node)
    }

    /// Remove one overlay by node. Returns whether it was open.
    pub fn close(&mut self, node: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.node != node);
        before != self.entries.len()
    }

    /// Close the topmost overlay. Returns the closed node.
    pub fn close_top(&mut self) -> Option<u64> {
        let node = self.top()?.node;
        self.close(node);
        Some(node)
    }

    /// Handle `Escape`: close the topmost dismissible overlay.
    pub fn close_on_escape(&mut self) -> Option<u64> {
        let node = self
            .entries
            .iter()
            .max_by_key(|entry| self.key_of(entry))
            .filter(|entry| entry.kind.escape_closes())?
            .node;
        self.close(node);
        Some(node)
    }

    /// Handle an outside click: close the topmost light-dismissable overlay
    /// when the point is outside its bounds.
    pub fn close_outside(&mut self, point: Vec2) -> Option<u64> {
        let entry = self.entries.iter().max_by_key(|entry| self.key_of(entry))?;
        if !entry.kind.light_dismiss() || entry.bounds.contains_point(point) {
            return None;
        }
        let node = entry.node;
        self.close(node);
        Some(node)
    }

    /// Drop overlays whose anchor no longer exists.
    ///
    /// Run once per frame after the scene is rebuilt: a trigger that was
    /// removed must take its popup (and any focus trap) with it.
    pub fn prune_anchors(&mut self, mut valid: impl FnMut(u64) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|entry| match entry.anchor {
            Some(anchor) => valid(anchor),
            None => true,
        });
        before - self.entries.len()
    }

    /// The focus scope of the topmost modal, if one is open.
    #[must_use]
    pub fn focus_scope(&self) -> Option<&[u64]> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == OverlayKind::Modal)
            .max_by_key(|entry| self.key_of(entry))
            .map(|entry| entry.focus_scope.as_slice())
    }

    /// The sort prefix for an entry's layer and order.
    #[must_use]
    pub fn sort_key(&self, _index: usize, order: u32) -> u64 {
        (order as u64) << 8
    }

    /// The full ordering key of an entry, layer first.
    #[must_use]
    pub fn key_of(&self, entry: &OverlayEntry) -> u64 {
        let index = self
            .entries
            .iter()
            .position(|candidate| core::ptr::eq(candidate, entry))
            .unwrap_or(0);
        let layer = match entry.kind.layer() {
            PassLayer::Canvas => 0u64,
            PassLayer::Ui => 1,
            PassLayer::Overlay => 2,
            PassLayer::Modal => 3,
            PassLayer::Debug => 255,
        };
        (layer << 56) | self.sort_key(index, self.order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }

    fn assert_contained(popup: Rect, viewport: Rect) {
        assert!(
            viewport.contains_rect(&popup),
            "popup {popup:?} escaped viewport {viewport:?}"
        );
    }

    #[test]
    fn a_popup_prefers_the_requested_side() {
        let anchor = Rect::new(100.0, 100.0, 120.0, 36.0);
        let popup = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 160.0),
            viewport(),
            Placement::BottomStart,
        );
        assert_eq!(popup.origin.x, 100.0);
        assert_eq!(popup.origin.y, 140.0);
    }

    #[test]
    fn a_popup_flips_up_when_the_bottom_edge_has_no_room() {
        let anchor = Rect::new(100.0, 650.0, 120.0, 36.0);
        let popup = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 160.0),
            viewport(),
            Placement::BottomStart,
        );
        assert!(popup.max().y <= anchor.origin.y, "{popup:?}");
        assert_contained(popup, viewport());
    }

    #[test]
    fn a_popup_flips_left_when_the_right_edge_has_no_room() {
        let anchor = Rect::new(1200.0, 100.0, 60.0, 30.0);
        let popup = place_anchored_popup(
            anchor,
            Vec2::new(200.0, 100.0),
            viewport(),
            Placement::RightStart,
        );
        assert!(popup.max().x <= anchor.origin.x);
        assert_contained(popup, viewport());
    }

    #[test]
    fn every_corner_and_edge_stays_contained() {
        let size = Vec2::new(240.0, 180.0);
        let anchors = [
            Rect::new(0.0, 0.0, 80.0, 30.0),
            Rect::new(1200.0, 0.0, 80.0, 30.0),
            Rect::new(0.0, 690.0, 80.0, 30.0),
            Rect::new(1200.0, 690.0, 80.0, 30.0),
            Rect::new(600.0, 0.0, 80.0, 30.0),
            Rect::new(600.0, 690.0, 80.0, 30.0),
            Rect::new(0.0, 350.0, 80.0, 30.0),
            Rect::new(1200.0, 350.0, 80.0, 30.0),
            Rect::new(-20.0, -10.0, 80.0, 30.0),
            Rect::new(1240.0, 710.0, 80.0, 30.0),
        ];
        let placements = [
            Placement::BottomStart,
            Placement::BottomEnd,
            Placement::TopStart,
            Placement::TopEnd,
            Placement::RightStart,
            Placement::LeftStart,
        ];
        for anchor in anchors {
            for placement in placements {
                let popup = place_anchored_popup(anchor, size, viewport(), placement);
                assert_contained(popup, viewport());
            }
        }
    }

    #[test]
    fn an_oversized_popup_is_shrunk_to_the_viewport() {
        let anchor = Rect::new(0.0, 0.0, 10.0, 10.0);
        let popup = place_anchored_popup(
            anchor,
            Vec2::new(4000.0, 4000.0),
            viewport(),
            Placement::BottomStart,
        );
        assert_eq!(popup, viewport());
    }

    #[test]
    fn the_stack_orders_later_overlays_on_top() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        stack.push(OverlayEntry::menu(2, 10, Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(stack.top().map(|entry| entry.node), Some(2));
    }

    #[test]
    fn escape_closes_only_the_top_layer() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        stack.push(OverlayEntry::menu(2, 10, Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(stack.close_on_escape(), Some(2));
        assert!(stack.contains(1));
        assert_eq!(stack.close_on_escape(), Some(1));
        assert!(stack.is_empty());
    }

    #[test]
    fn a_click_outside_dismisses_a_popover_but_not_a_modal() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        ));
        assert_eq!(stack.close_outside(Vec2::new(500.0, 500.0)), Some(1));
        assert!(stack.is_empty());

        stack.push(OverlayEntry::modal(
            2,
            Rect::new(10.0, 10.0, 100.0, 100.0),
            vec![20, 21],
        ));
        assert_eq!(stack.close_outside(Vec2::new(500.0, 500.0)), None);
        assert!(stack.contains(2));
    }

    #[test]
    fn a_click_inside_the_popup_does_not_dismiss_it() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        ));
        assert_eq!(stack.close_outside(Vec2::new(50.0, 50.0)), None);
        assert!(stack.contains(1));
    }

    #[test]
    fn a_vanished_anchor_takes_its_popup_with_it() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::popover(
            1,
            10,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ));
        stack.push(OverlayEntry::modal(
            2,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            vec![],
        ));
        assert_eq!(stack.prune_anchors(|anchor| anchor != 10), 1);
        assert!(!stack.contains(1));
        assert!(stack.contains(2), "a modal has no anchor and survives");
    }

    #[test]
    fn a_modal_exposes_its_focus_scope_and_wins_over_a_dialog_popover() {
        let mut stack = OverlayStack::new();
        stack.push(OverlayEntry::modal(
            1,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            vec![7, 8],
        ));
        stack.push(OverlayEntry::popover(2, 7, Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(stack.focus_scope(), Some([7, 8].as_slice()));
    }
}

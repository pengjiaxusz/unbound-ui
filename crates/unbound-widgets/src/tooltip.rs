//! Hover tooltips with a delay and mouse pass-through.
//!
//! A tooltip is the one overlay that must **not** receive input: it appears
//! under the pointer, and a hit region there would steal the very hover that
//! keeps it alive. [`emit_tooltip`] therefore returns a `WidgetOutput` with
//! `hit: None`, and [`TooltipState`] owns the delay so a pointer sweeping
//! across a toolbar does not flash a bubble per button.

use accesskit::Role;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::overlay::{place_anchored_popup, OverlayEntry, Placement};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// How long the pointer must rest before the tooltip appears.
pub const TOOLTIP_DELAY_MS: f32 = 400.0;
const TOOLTIP_SHADOW: f32 = 14.0;

/// A tooltip descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipDesc {
    /// The trigger rectangle.
    pub anchor: Rect,
    /// The viewport the bubble must stay inside.
    pub viewport: Rect,
    /// The measured bubble size.
    pub size: Vec2,
    /// Where the bubble prefers to sit.
    pub placement: Placement,
    /// The delay before it appears.
    pub delay_ms: f32,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
}

impl TooltipDesc {
    #[must_use]
    pub fn new(anchor: Rect, viewport: Rect, size: Vec2) -> Self {
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::TopStart,
            delay_ms: TOOLTIP_DELAY_MS,
            radius: 6.0,
            z: 940,
        }
    }

    /// The placed bubble rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        place_anchored_popup(self.anchor, self.size, self.viewport, self.placement)
    }
}

/// The delay state of one tooltip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TooltipState {
    hovered_ms: f32,
    visible: bool,
    hovered_anchor: Option<u64>,
}

impl TooltipState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the bubble is currently shown.
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// How long the pointer has rested, in milliseconds.
    #[must_use]
    pub const fn hovered_ms(&self) -> f32 {
        self.hovered_ms
    }

    /// Advance the delay.
    ///
    /// `hovered` is the node under the pointer (`None` when it is over
    /// nothing). Moving to a different node restarts the timer, which is what
    /// keeps a sweep across a row of buttons from showing a bubble for each
    /// one. Returns whether the visibility changed.
    pub fn update(&mut self, desc: &TooltipDesc, hovered: Option<u64>, dt_ms: f32) -> bool {
        let was = self.visible;
        match hovered {
            Some(node) if self.hovered_anchor == Some(node) => {
                self.hovered_ms += dt_ms.max(0.0);
                self.visible = self.hovered_ms >= desc.delay_ms;
            }
            Some(node) => {
                self.hovered_anchor = Some(node);
                self.hovered_ms = dt_ms.max(0.0);
                self.visible = self.hovered_ms >= desc.delay_ms;
            }
            None => {
                self.hovered_anchor = None;
                self.hovered_ms = 0.0;
                self.visible = false;
            }
        }
        self.visible != was
    }

    /// Hide the bubble immediately (e.g. a press).
    pub fn hide(&mut self) {
        self.visible = false;
        self.hovered_ms = 0.0;
    }
}

/// A tooltip descriptor bound to an identity.
#[derive(Clone, Debug)]
pub struct TooltipWidget {
    /// The bubble node.
    pub node: u64,
    /// The trigger node.
    pub anchor_node: u64,
    /// The descriptor.
    pub desc: TooltipDesc,
}

impl WidgetDesc for TooltipWidget {
    fn rect(&self) -> Rect {
        self.desc.rect()
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_tooltip(self.node, self.anchor_node, &self.desc, state).0
    }
}

/// Emit a tooltip bubble. The output never contains a hit region.
#[must_use]
pub fn emit_tooltip(
    node: u64,
    anchor_node: u64,
    desc: &TooltipDesc,
    state: ControlState,
) -> (WidgetOutput, OverlayEntry) {
    let rect = desc.rect();
    let radii = [desc.radius; 4];
    let primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            radii,
            Token::NONE,
            Token::NONE,
            0.0,
            TOOLTIP_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::OVERLAY_BG, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];

    let a11y = unbound_a11y::A11yNode::new(Role::Tooltip, "", rect).disabled(state.disabled);

    (
        WidgetOutput {
            primitives,
            payloads: Vec::new(),
            // Mouse pass-through: a tooltip must never steal the hover that
            // keeps it open.
            hit: None,
            a11y: Some(a11y),
            focusable: false,
        },
        OverlayEntry::tooltip(node, anchor_node, rect),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TooltipDesc {
        TooltipDesc::new(
            Rect::new(100.0, 100.0, 80.0, 32.0),
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            Vec2::new(120.0, 28.0),
        )
    }

    #[test]
    fn the_bubble_waits_for_the_delay() {
        let desc = desc();
        let mut state = TooltipState::new();
        assert!(!state.visible());
        assert!(!state.update(&desc, Some(7), 399.0));
        assert!(!state.visible());
        assert!(state.update(&desc, Some(7), 2.0));
        assert!(state.visible());
    }

    #[test]
    fn moving_to_another_trigger_restarts_the_timer() {
        let desc = desc();
        let mut state = TooltipState::new();
        state.update(&desc, Some(7), 399.0);
        assert!(!state.update(&desc, Some(8), 16.0));
        assert!(!state.visible());
        assert_eq!(state.hovered_ms(), 16.0);
    }

    #[test]
    fn leaving_the_trigger_hides_it_immediately() {
        let desc = desc();
        let mut state = TooltipState::new();
        state.update(&desc, Some(7), 500.0);
        assert!(state.visible());
        assert!(state.update(&desc, None, 16.0));
        assert!(!state.visible());
        assert_eq!(state.hovered_ms(), 0.0);
    }

    #[test]
    fn a_tooltip_never_receives_pointer_events() {
        let desc = desc();
        let (output, entry) = emit_tooltip(3, 2, &desc, ControlState::default());
        assert!(output.hit.is_none());
        assert!(desc.viewport.contains_rect(&entry.bounds));
        assert_eq!(output.primitives.len(), 3);
    }

    #[test]
    fn the_bubble_flips_above_a_trigger_near_the_top() {
        let mut desc = desc();
        desc.anchor = Rect::new(100.0, 0.0, 80.0, 30.0);
        desc.placement = Placement::TopStart;
        let rect = desc.rect();
        assert!(rect.origin.y >= desc.viewport.origin.y);
        assert!(desc.viewport.contains_rect(&rect));
    }
}

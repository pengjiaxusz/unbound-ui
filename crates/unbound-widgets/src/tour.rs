//! Tour / Spotlight guided walkthrough overlay with target cutout & step bubble.

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

/// Single step in a [`TourDesc`].
#[derive(Clone, Debug, PartialEq)]
pub struct TourStep {
    /// Target rectangle to spotlight.
    pub target: Rect,
    /// Step title.
    pub title: String,
    /// Step description.
    pub body: String,
}

/// Interactive state for [`TourDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TourState {
    /// Whether the tour is active.
    pub active: bool,
    /// Current step index.
    pub step: usize,
}

impl TourState {
    /// Advance to the next step or close if on the last step.
    pub fn next(&mut self, total: usize) {
        if self.step + 1 < total {
            self.step += 1;
        } else {
            self.active = false;
        }
    }
}

/// Descriptor for a [`Tour`] spotlight overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct TourDesc {
    /// Viewport bounds.
    pub rect: Rect,
    /// Spotlight target rectangle.
    pub target: Rect,
    /// Step title.
    pub title: String,
    /// Current 1-based step index.
    pub step: usize,
    /// Total steps.
    pub total_steps: usize,
    /// Base z-index.
    pub z: u16,
}

impl TourDesc {
    /// Create a tour descriptor.
    #[must_use]
    pub fn new(rect: Rect, target: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            target,
            title: title.into(),
            step: 1,
            total_steps: 3,
            z: 900,
        }
    }
}

/// Emit a stable-shape tour spotlight (1 scrim + 1 spotlight ring + 1 popover card + 1 next pill + 1 focus ring = 5 primitives).
#[must_use]
pub fn emit_tour(
    node: u64,
    desc: &TourDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let mut scrim = surface(desc.rect, 6.0, Token::SURFACE_SUNKEN, desc.z);
    scrim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(scrim.depth_u16(), 0.45);
    primitives.push(scrim);

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.target,
        6.0,
        Token::SURFACE,
        if hovered {
            Token::PRIMARY
        } else {
            Token::FOCUS_RING
        },
        2.0,
        0.0,
        desc.z + 1,
    ));

    let bubble_w = 140.0_f32.min(desc.rect.width() - 16.0).max(40.0);
    let bubble_h = 44.0_f32.min(desc.rect.height() * 0.45).max(24.0);
    let bubble_r = Rect::new(
        (desc.target.origin.x)
            .min(desc.rect.max().x - bubble_w - 8.0)
            .max(desc.rect.origin.x + 8.0),
        (desc.target.max().y + 6.0).min(desc.rect.max().y - bubble_h - 6.0),
        bubble_w,
        bubble_h,
    );
    primitives.push(outlined(
        bubble_r,
        8.0,
        Token::SURFACE_RAISED,
        Token::CARD_BORDER,
        1.0,
        6.0,
        desc.z + 2,
    ));

    let next_r = Rect::new(bubble_r.max().x - 42.0, bubble_r.max().y - 20.0, 34.0, 14.0);
    primitives.push(surface(next_r, 4.0, Token::PRIMARY, desc.z + 3));

    primitives.push(focus_ring(
        bubble_r,
        8.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 4,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 5,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z + 2), bubble_r)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Dialog, desc.title.clone(), bubble_r).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

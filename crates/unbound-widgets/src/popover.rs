//! Anchored popovers and dropdown containers.
//!
//! A popover is a surface plus an [`OverlayEntry`]: the geometry comes from
//! [`place_anchored_popup`] so it can never escape the window, and the entry
//! lets [`OverlayStack`] dismiss it, order it and prune it with its trigger.
//!
//! The content is emitted by the caller; this module paints the container, the
//! border, the shadow and the (optional) backdrop-blur request.

use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{HitRegion, PORTAL_LAYER};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::overlay::{place_anchored_popup, OverlayEntry, Placement};
use crate::primitive::{instance, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The hit-test layer of a popover: the portal band.
pub const POPOVER_LAYER: u8 = PORTAL_LAYER;
/// The shadow blur of a popover.
const POPOVER_SHADOW: f32 = 24.0;

/// A popover anchored to a trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct PopoverDesc {
    /// The trigger rectangle.
    pub anchor: Rect,
    /// The viewport the popover must stay inside.
    pub viewport: Rect,
    /// The popover's preferred size.
    pub size: Vec2,
    /// The preferred side.
    pub placement: Placement,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
    /// Backdrop blur radius behind the popover; `0.0` disables the pass.
    pub backdrop_blur: f32,
}

impl PopoverDesc {
    #[must_use]
    pub fn new(anchor: Rect, viewport: Rect, size: Vec2) -> Self {
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::BottomStart,
            radius: 10.0,
            z: 900,
            backdrop_blur: 0.0,
        }
    }

    /// The placed popover rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        place_anchored_popup(self.anchor, self.size, self.viewport, self.placement)
    }
}

/// A popover descriptor bound to an identity.
#[derive(Clone, Debug)]
pub struct PopoverWidget {
    /// The popover's node id.
    pub node: u64,
    /// The trigger's node id.
    pub anchor_node: u64,
    /// The descriptor.
    pub desc: PopoverDesc,
}

impl WidgetDesc for PopoverWidget {
    fn rect(&self) -> Rect {
        self.desc.rect()
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_popover(self.node, self.anchor_node, &self.desc, state).output
    }
}

/// What one popover emits.
#[derive(Clone, Debug)]
pub struct PopoverOutput {
    /// The painted container and its hit region.
    pub output: WidgetOutput,
    /// The stack entry the caller pushes.
    pub entry: OverlayEntry,
    /// The blur request, when `backdrop_blur > 0`.
    pub backdrop: Option<crate::overlay::BackdropRequest>,
}

/// Emit a popover container.
#[must_use]
pub fn emit_popover(
    node: u64,
    anchor_node: u64,
    desc: &PopoverDesc,
    state: ControlState,
) -> PopoverOutput {
    let rect = desc.rect();
    let radii = [desc.radius; 4];
    let primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            radii,
            Token::NONE,
            Token::NONE,
            0.0,
            POPOVER_SHADOW,
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

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, POPOVER_LAYER, u32::from(desc.z), rect)
            .with_shape(unbound_input::Shape::Rounded { radii })
    });
    let a11y = unbound_a11y::A11yNode::new(accesskit::Role::GenericContainer, "", rect)
        .disabled(state.disabled);

    PopoverOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit,
            a11y: Some(a11y),
            focusable: false,
        },
        entry: OverlayEntry::popover(node, anchor_node, rect),
        backdrop: crate::overlay::BackdropRequest::new(node, rect, desc.backdrop_blur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_popover_stays_on_screen_and_layers_above_the_ui() {
        let desc = PopoverDesc::new(
            Rect::new(1200.0, 680.0, 40.0, 30.0),
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            Vec2::new(240.0, 200.0),
        );
        let output = emit_popover(1, 2, &desc, ControlState::default());
        assert!(desc.viewport.contains_rect(&output.entry.bounds));
        assert_eq!(output.output.hit.as_ref().unwrap().layer, POPOVER_LAYER);
        assert_eq!(output.output.primitives.len(), 3);
    }
}

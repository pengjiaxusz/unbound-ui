//! Anchored popovers and dropdown containers.
//!
//! A popover is a surface plus an [`OverlayEntry`]: the geometry comes from
//! [`place_anchored_popup`] so it can never escape the window, and the entry
//! lets [`OverlayStack`] dismiss it, order it and prune it with its trigger.
//!
//! The content is emitted by the caller; this module paints the container, the
//! border, the shadow and the (optional) backdrop-blur request.

use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{HitRegion, PORTAL_LAYER};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::overlay::{OverlayEntry, PlacedPopup, Placement};
use crate::primitive::{instance, surface};
use crate::{ControlState, SceneCollector, WidgetDesc, WidgetOutput};

/// The hit-test layer of a popover: the portal band.
pub const POPOVER_LAYER: u8 = PORTAL_LAYER;
/// The shadow blur of a popover.
const POPOVER_SHADOW: f32 = 24.0;
/// The depth of an arrow tip, and half its base width.
const ARROW_SIZE: f32 = 8.0;
/// The stroke width of an arrow edge.
const ARROW_STROKE: f32 = 2.0;

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
    /// Whether to paint the arrow that points at the anchor.
    pub show_arrow: bool,
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
            show_arrow: false,
        }
    }

    /// The placed popup, including the chosen side and the arrow tip.
    #[must_use]
    pub fn placed(&self) -> PlacedPopup {
        crate::overlay::place_anchored_popup_detailed(
            self.anchor,
            self.size,
            self.viewport,
            self.placement,
        )
    }

    /// The placed popover rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.placed().rect
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
    let placed = desc.placed();
    let rect = placed.rect;
    let radii = [desc.radius; 4];
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
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
    let mut payloads = Vec::new();
    if desc.show_arrow {
        arrow_edges(
            placed.chosen_placement,
            placed.arrow_tip,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, POPOVER_LAYER, u32::from(desc.z), rect)
            .with_shape(unbound_input::Shape::Rounded { radii })
    });
    let a11y = unbound_a11y::A11yNode::new(accesskit::Role::GenericContainer, "", rect)
        .disabled(state.disabled);

    PopoverOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        entry: OverlayEntry::popover(node, anchor_node, rect),
        backdrop: crate::overlay::BackdropRequest::new(node, rect, desc.backdrop_blur),
    }
}

/// Emit a popover and its children in one scope.
///
/// The container is painted first. The children are then collected inside the
/// popover's clip and lifted to [`POPOVER_LAYER`] so they win hit-testing
/// against the UI beneath the portal.
pub fn emit_popover_scope<I>(
    collector: &mut SceneCollector,
    node: u64,
    anchor_node: u64,
    desc: &PopoverDesc,
    state: ControlState,
    children: I,
) -> PopoverOutput
where
    I: IntoIterator<Item = (u64, WidgetOutput)>,
{
    let output = emit_popover(node, anchor_node, desc, state);
    if let Some(backdrop) = output.backdrop {
        collector.backdrops.push(backdrop);
    }
    let rect = output.entry.bounds;
    collector.push(node, output.output.clone());

    let clip = GpuClipNode::from_aabb([rect.min().x, rect.min().y, rect.max().x, rect.max().y]);
    collector.push_clip(clip, rect);
    for (key, mut child) in children {
        if let Some(hit) = child.hit.as_mut() {
            hit.layer = POPOVER_LAYER;
        }
        collector.push(key, child);
    }
    collector.pop_clip();
    output
}

/// Two quadratic strokes forming the arrow that points at the anchor.
fn arrow_edges(
    placement: Placement,
    tip: Vec2,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    // The direction from the popup edge back into the popup body.
    let inward = match placement {
        Placement::BottomStart | Placement::BottomEnd | Placement::Bottom => Vec2::new(0.0, 1.0),
        Placement::TopStart | Placement::TopEnd | Placement::Top => Vec2::new(0.0, -1.0),
        Placement::RightStart | Placement::RightEnd | Placement::Right => Vec2::new(1.0, 0.0),
        Placement::LeftStart | Placement::LeftEnd | Placement::Left => Vec2::new(-1.0, 0.0),
    };
    let base = tip + inward * ARROW_SIZE;
    let half = Vec2::new(-inward.y, inward.x) * ARROW_SIZE;
    for corner in [base + half, base - half] {
        curve_stroke(
            [tip, (tip + corner) * 0.5, corner],
            ARROW_STROKE,
            Token::OUTLINE,
            z,
            primitives,
            payloads,
        );
    }
}

/// One quadratic stroke packed into its tight local box.
fn curve_stroke(
    points: [Vec2; 3],
    width: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let local = points.map(|point| [point.x - min_x, point.y - min_y]);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(local[0], local[1], local[2], width));
    let mut primitive = instance(
        Rect::new(
            min_x,
            min_y,
            (max_x - min_x).max(0.5),
            (max_y - min_y).max(0.5),
        ),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
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

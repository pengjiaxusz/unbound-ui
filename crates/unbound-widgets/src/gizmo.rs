//! The transform gizmo: eight resize grips, a rotation grip and the live size
//! bubble.
//!
//! The gizmo is *screen-space by contract*: its border and grips keep a fixed
//! pixel size no matter the canvas zoom, which is why the bounds are converted
//! with the caller's zoom before painting. Every grip reports its own hit
//! region with the matching resize cursor, so a drag can start on any of them.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_canvas::{Handle, SnapGuide};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The grip square.
pub const GRIP_SIZE: f32 = 8.0;
/// The rotation grip diameter.
pub const ROTATE_GRIP: f32 = 10.0;
/// The gap between the top edge and the rotation grip.
pub const ROTATE_OFFSET: f32 = 22.0;
/// The size bubble height.
pub const SIZE_BUBBLE: f32 = 20.0;

/// The gizmo descriptor. `bounds` is in **screen** space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoDesc {
    /// The selection bounds, already projected to screen space.
    pub bounds: Rect,
    /// The canvas zoom, used to keep strokes one pixel wide.
    pub zoom: f32,
    /// The rotation grip offset.
    pub rotate_offset: f32,
    /// Whether to paint the size bubble.
    pub show_size: bool,
    /// The draw order key.
    pub z: u16,
}

impl GizmoDesc {
    #[must_use]
    pub fn new(bounds: Rect, zoom: f32) -> Self {
        Self {
            bounds,
            zoom: if zoom > 0.0 { zoom } else { 1.0 },
            rotate_offset: ROTATE_OFFSET,
            show_size: true,
            z: 900,
        }
    }

    /// The same gizmo without the size bubble.
    #[must_use]
    pub fn without_size(mut self) -> Self {
        self.show_size = false;
        self
    }

    /// The hairline width, constant in screen pixels.
    #[must_use]
    pub fn line_width(&self) -> f32 {
        (1.0 / self.zoom).clamp(0.5, 2.0)
    }

    /// The grip rectangle for one handle, at rest.
    #[must_use]
    pub fn grip_rect(&self, handle: Handle) -> Rect {
        grip_rect_for(self.bounds, self.rotate_offset, handle)
    }

    /// The grip under `point`, within `radius`.
    #[must_use]
    pub fn grip_at(&self, point: Vec2, radius: f32) -> Option<Handle> {
        Handle::hit(self.bounds, point, radius, self.rotate_offset)
    }

    /// The size bubble rectangle.
    #[must_use]
    pub fn size_bubble_rect(&self, width: f32) -> Rect {
        Rect::new(
            self.bounds.center().x - width * 0.5,
            self.bounds.max().y + 12.0,
            width,
            SIZE_BUBBLE,
        )
    }
}

/// The grip rectangle for `handle` around `bounds`.
#[must_use]
pub fn grip_rect_for(bounds: Rect, rotate_offset: f32, handle: Handle) -> Rect {
    let centre = handle.position(bounds, rotate_offset);
    let size = if handle == Handle::Rotate {
        ROTATE_GRIP
    } else {
        GRIP_SIZE
    };
    Rect::new(centre.x - size * 0.5, centre.y - size * 0.5, size, size)
}

/// What one gizmo emits.
#[derive(Clone, Debug)]
pub struct GizmoOutput {
    /// The outline, grips, guides and bubble.
    pub output: WidgetOutput,
    /// One hit region per grip, in [`Handle::ALL`] order.
    pub hits: Vec<(Handle, HitRegion)>,
    /// The size bubble, when painted.
    pub bubble: Option<Rect>,
}

/// Emit the gizmo, the active drag preview and the size bubble.
///
/// `preview` is `(handle, bounds)` from the caller's [`unbound_canvas::HandleDrag`];
/// when present the outline follows it and the dragged grip is highlighted.
#[must_use]
pub fn emit_gizmo(
    node: u64,
    desc: &GizmoDesc,
    preview: Option<(Handle, Rect)>,
    guides: &[SnapGuide],
    size_glyphs: &[GlyphQuad],
    control: ControlState,
) -> GizmoOutput {
    let width = desc.line_width();
    let bounds = preview.map_or(desc.bounds, |(_, bounds)| bounds);
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![instance(
        bounds,
        [0.0; 4],
        Token::NONE,
        Token::PRIMARY,
        width,
        0.0,
        PrimType::Stroke,
        SubElement::FocusRing,
        desc.z,
    )];

    for guide in guides {
        let rect = match guide.axis {
            unbound_canvas::Axis::X => Rect::new(
                guide.coordinate - 0.5,
                guide.start,
                1.0,
                (guide.end - guide.start).max(1.0),
            ),
            unbound_canvas::Axis::Y => Rect::new(
                guide.start,
                guide.coordinate - 0.5,
                (guide.end - guide.start).max(1.0),
                1.0,
            ),
        };
        primitives.push(surface(rect, 0.0, Token::SNAP_GUIDE, desc.z + 1));
    }

    let mut hits = Vec::with_capacity(Handle::ALL.len());
    for handle in Handle::ALL {
        let grip = grip_rect_for(bounds, desc.rotate_offset, handle);
        let radius = grip.size.x * 0.5;
        primitives.push(surface(grip, radius, Token::SURFACE_RAISED, desc.z + 1));
        let ink = if preview.is_some_and(|(active, _)| active == handle) {
            Token::PRIMARY
        } else {
            Token::OUTLINE_STRONG
        };
        primitives.push(surface(grip.inflate(width), radius + width, ink, desc.z));
        if control.focused && handle == Handle::SouthEast {
            primitives.push(focus_ring(
                grip.inflate(3.0),
                6.0,
                Token::FOCUS_RING,
                2.0,
                desc.z + 2,
                true,
            ));
        }
        hits.push((
            handle,
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), grip.inflate(2.0))
                .with_cursor(CursorKind::for_handle(handle)),
        ));
    }

    let mut payloads = Vec::new();
    let bubble = desc.show_size.then(|| {
        let rect = desc.size_bubble_rect(96.0);
        primitives.push(surface(rect, 6.0, Token::HUD_BG, desc.z + 2));
        glyphs(
            size_glyphs,
            Token::HUD_TEXT,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
        rect
    });

    let mut a11y = A11yNode::new(Role::Group, "变换手柄", bounds).clickable();
    a11y.value = format!("{:.0} × {:.0}", bounds.size.x, bounds.size.y);

    GizmoOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: None,
            a11y: Some(a11y),
            focusable: false,
        },
        hits,
        bubble,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_canvas::{Axis, HandleDrag, Modifiers, SnapKind};

    fn desc() -> GizmoDesc {
        GizmoDesc::new(Rect::new(100.0, 100.0, 200.0, 100.0), 2.0)
    }

    #[test]
    fn grips_sit_on_the_eight_edges_and_the_rotation_grip_above() {
        let desc = desc();
        assert_eq!(
            desc.grip_rect(Handle::NorthWest).origin,
            Vec2::new(96.0, 96.0)
        );
        assert_eq!(
            desc.grip_rect(Handle::SouthEast).origin,
            Vec2::new(296.0, 196.0)
        );
        let rotate = desc.grip_rect(Handle::Rotate);
        assert!(rotate.center().y < desc.bounds.origin.y);
        assert_eq!(rotate.center().x, desc.bounds.center().x);
    }

    #[test]
    fn the_hairline_is_screen_constant() {
        assert_eq!(desc().line_width(), 0.5);
        assert_eq!(
            GizmoDesc::new(Rect::new(0.0, 0.0, 1.0, 1.0), 1.0).line_width(),
            1.0
        );
    }

    #[test]
    fn a_grip_is_hit_within_its_radius() {
        let desc = desc();
        let centre = Handle::East.position(desc.bounds, desc.rotate_offset);
        assert_eq!(
            desc.grip_at(centre + Vec2::new(2.0, 0.0), 8.0),
            Some(Handle::East)
        );
        assert!(desc.grip_at(centre + Vec2::new(40.0, 40.0), 8.0).is_none());
    }

    #[test]
    fn a_drag_preview_moves_the_outline_and_the_bubble() {
        let desc = desc();
        let drag = HandleDrag::new(Handle::SouthEast, desc.bounds, desc.bounds.max(), 0.0);
        let preview = drag.preview(desc.bounds.max() + Vec2::new(20.0, 10.0), Modifiers::NONE);
        let guides = [SnapGuide {
            axis: Axis::X,
            coordinate: 320.0,
            start: 0.0,
            end: 500.0,
            kind: SnapKind::Right,
        }];
        let output = emit_gizmo(
            4,
            &desc,
            Some((Handle::SouthEast, preview)),
            &guides,
            &[],
            ControlState::default(),
        );
        assert_eq!(output.hits.len(), Handle::ALL.len());
        assert!(output.bubble.is_some());
        let outline = &output.output.primitives[0];
        assert_eq!(outline.size, [preview.size.x, preview.size.y]);
    }

    #[test]
    fn every_grip_carries_the_right_cursor() {
        let output = emit_gizmo(4, &desc(), None, &[], &[], ControlState::default());
        for (handle, hit) in &output.hits {
            assert_eq!(hit.cursor, Some(CursorKind::for_handle(*handle)));
        }
    }

    #[test]
    fn the_gizmo_announces_the_selection_size() {
        let output = emit_gizmo(4, &desc(), None, &[], &[], ControlState::default());
        let a11y = output.output.a11y.expect("gizmo");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.value, "200 × 100");
    }
}

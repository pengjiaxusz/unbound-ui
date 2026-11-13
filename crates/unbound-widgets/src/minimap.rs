//! Canvas minimaps.
//!
//! The minimap is a second camera into the same world: it maps `world_bounds`
//! onto its own rectangle with a uniform scale, paints a block per item and
//! outlines the region the main camera currently shows. Dragging inside it
//! only moves the camera — no scene graph is rebuilt, so a pan from the
//! minimap costs one push-constant write.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The empty margin kept around the world inside the minimap.
pub const MINIMAP_INSET: f32 = 6.0;
/// One minimap block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapItem {
    /// The item's world bounds.
    pub bounds: Rect,
    /// The block colour.
    pub token: Token,
}

/// The minimap descriptor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapDesc {
    /// The minimap rectangle, in screen space.
    pub rect: Rect,
    /// The world rectangle the minimap covers.
    pub world_bounds: Rect,
    /// The camera's current world rectangle.
    pub camera_viewport: Rect,
    /// The draw order key.
    pub z: u16,
}

impl MinimapDesc {
    #[must_use]
    pub fn new(rect: Rect, world_bounds: Rect, camera_viewport: Rect) -> Self {
        Self {
            rect,
            world_bounds,
            camera_viewport,
            z: 880,
        }
    }

    /// The uniform world-to-minimap scale.
    #[must_use]
    pub fn scale(&self) -> f32 {
        let inner = self.inner_rect();
        if self.world_bounds.size.x <= 0.0 || self.world_bounds.size.y <= 0.0 {
            return 1.0;
        }
        (inner.size.x / self.world_bounds.size.x)
            .min(inner.size.y / self.world_bounds.size.y)
            .max(0.0)
    }

    /// The area world content is mapped into.
    #[must_use]
    pub fn inner_rect(&self) -> Rect {
        self.rect.inflate(-MINIMAP_INSET)
    }

    /// Project a world point into the minimap.
    #[must_use]
    pub fn project(&self, point: Vec2) -> Vec2 {
        let scale = self.scale();
        let inner = self.inner_rect();
        let content = Vec2::new(
            self.world_bounds.size.x * scale,
            self.world_bounds.size.y * scale,
        );
        let offset = Vec2::new(
            inner.origin.x + (inner.size.x - content.x) * 0.5,
            inner.origin.y + (inner.size.y - content.y) * 0.5,
        );
        offset + (point - self.world_bounds.min()) * scale
    }

    /// Project a world rectangle into the minimap.
    #[must_use]
    pub fn project_rect(&self, rect: Rect) -> Rect {
        let min = self.project(rect.min());
        let max = self.project(rect.max());
        Rect::new(
            min.x,
            min.y,
            (max.x - min.x).max(1.0),
            (max.y - min.y).max(1.0),
        )
    }

    /// Un-project a minimap point back to world space.
    #[must_use]
    pub fn unproject(&self, point: Vec2) -> Vec2 {
        let scale = self.scale();
        if scale <= f32::EPSILON {
            return self.world_bounds.min();
        }
        let min = self.project(self.world_bounds.min());
        self.world_bounds.min() + (point - min) / scale
    }

    /// The world point at the minimap's centre (the camera target for a click).
    #[must_use]
    pub fn camera_target(&self, point: Vec2) -> Vec2 {
        let centre = self.unproject(point);
        Vec2::new(
            centre
                .x
                .clamp(self.world_bounds.min().x, self.world_bounds.max().x),
            centre
                .y
                .clamp(self.world_bounds.min().y, self.world_bounds.max().y),
        )
    }
}

/// The dragging state of one minimap.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MinimapState {
    /// Whether the pointer is dragging the camera.
    pub dragging: bool,
}

impl MinimapState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a camera drag at `point` (minimap space).
    pub fn begin(&mut self) {
        self.dragging = true;
    }

    /// End the drag.
    pub fn end(&mut self) {
        self.dragging = false;
    }
}

/// Emit a minimap and its hit region.
#[must_use]
pub fn emit_minimap(
    node: u64,
    desc: &MinimapDesc,
    items: &[MinimapItem],
    state: &MinimapState,
    control: ControlState,
) -> (WidgetOutput, HitRegion) {
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            desc.rect,
            [10.0; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            14.0,
            unbound_gpu::data::PrimType::Shadow,
            unbound_gpu::data::SubElement::Shadow,
            desc.z,
        ),
        surface(desc.rect, 10.0, Token::HUD_BG, desc.z),
        instance(
            desc.rect,
            [10.0; 4],
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            unbound_gpu::data::PrimType::RoundedRect,
            unbound_gpu::data::SubElement::Border,
            desc.z,
        ),
    ];
    for item in items {
        let block = desc.project_rect(item.bounds);
        primitives.push(surface(block, 1.5, item.token, desc.z + 1));
    }
    let viewport = desc.project_rect(desc.camera_viewport);
    primitives.push(outlined(
        viewport,
        2.0,
        Token::NONE,
        Token::PRIMARY,
        1.5,
        0.0,
        desc.z + 2,
    ));
    // The drag/focus outline always occupies a slot; its opacity alone says
    // whether it is visible, so the primitive count never changes.
    let mut ring = outlined(
        desc.rect,
        10.0,
        Token::NONE,
        Token::FOCUS_RING,
        1.5,
        0.0,
        desc.z + 3,
    );
    ring.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        ring.depth_u16(),
        if state.dragging || control.focused {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(ring);

    let mut a11y = A11yNode::new(Role::Group, "小地图", desc.rect).clickable();
    a11y.description = "点击或拖拽以平移画布".to_owned();

    (
        WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit: Some(
                HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect).with_cursor(
                    if state.dragging {
                        CursorKind::Grabbing
                    } else {
                        CursorKind::Pointer
                    },
                ),
            ),
            a11y: Some(a11y),
            focusable: false,
        },
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> MinimapDesc {
        MinimapDesc::new(
            Rect::new(1000.0, 500.0, 200.0, 150.0),
            Rect::new(0.0, 0.0, 1000.0, 1000.0),
            Rect::new(200.0, 100.0, 400.0, 300.0),
        )
    }

    #[test]
    fn the_world_is_centred_with_a_uniform_scale() {
        let desc = desc();
        let centre = desc.project(Vec2::new(500.0, 500.0));
        assert!((centre.x - desc.rect.center().x).abs() < 0.001);
        assert!((centre.y - desc.rect.center().y).abs() < 0.001);
        assert_eq!(desc.scale(), desc.scale());
    }

    #[test]
    fn projecting_and_unprojecting_round_trips() {
        let desc = desc();
        for point in [
            Vec2::new(0.0, 0.0),
            Vec2::new(250.0, 750.0),
            Vec2::new(1000.0, 1000.0),
        ] {
            let round = desc.unproject(desc.project(point));
            assert!((round.x - point.x).abs() < 0.001);
            assert!((round.y - point.y).abs() < 0.001);
        }
    }

    #[test]
    fn the_camera_viewport_is_outlined_inside_the_map() {
        let desc = desc();
        let viewport = desc.project_rect(desc.camera_viewport);
        assert!(desc.rect.contains_rect(&viewport));
        assert!(viewport.size.x > 0.0 && viewport.size.y > 0.0);
    }

    #[test]
    fn a_click_maps_back_to_a_world_camera_target() {
        let desc = desc();
        let target = desc.camera_target(desc.rect.center());
        assert!((target.x - 500.0).abs() < 0.001);
        assert!((target.y - 500.0).abs() < 0.001);
        let clamped = desc.camera_target(Vec2::new(0.0, 0.0));
        assert!(clamped.x >= 0.0 && clamped.y >= 0.0);
    }

    #[test]
    fn the_emitter_paints_blocks_and_a_viewport_outline() {
        let desc = desc();
        let items = [MinimapItem {
            bounds: Rect::new(100.0, 100.0, 200.0, 80.0),
            token: Token::PRIMARY,
        }];
        let (output, hit) = emit_minimap(
            2,
            &desc,
            &items,
            &MinimapState::new(),
            ControlState::default(),
        );
        assert_eq!(hit.bounds, desc.rect);
        assert_eq!(
            output.hit.as_ref().unwrap().cursor,
            Some(CursorKind::Pointer)
        );
        // Shadow, background, border, one block, one viewport outline and the
        // always-present (invisible) drag/focus ring slot.
        assert_eq!(output.primitives.len(), 6);
        assert_eq!(
            output.a11y.as_ref().map(|node| node.role),
            Some(Role::Group)
        );
    }

    #[test]
    fn dragging_switches_the_cursor() {
        let mut state = MinimapState::new();
        state.begin();
        let (output, _) = emit_minimap(2, &desc(), &[], &state, ControlState::default());
        assert_eq!(
            output.hit.as_ref().unwrap().cursor,
            Some(CursorKind::Grabbing)
        );
        state.end();
        assert!(!state.dragging);
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let mut counts = Vec::new();
        for dragging in [false, true] {
            let mut state = MinimapState::new();
            if dragging {
                state.begin();
            }
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let (output, _) = emit_minimap(2, &desc, &[], &state, control);
                            counts.push(output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

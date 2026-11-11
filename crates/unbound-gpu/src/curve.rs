//! Curves on the GPU (`docs` §3.4).
//!
//! A curve reaches the GPU as three things: a primitive whose box is the
//! **tight oriented** bounding rectangle of the segment, a transform that
//! rotates that box into place, and a payload that carries the three control
//! points in the box's own frame. The fragment shader then evaluates the
//! analytic distance to the quadratic Bézier, so the stroke is exact even
//! though the rasteriser only sees a rectangle.

use unbound_math::Vec2;
use unbound_vector::CurveInstance;

use crate::data::{
    GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType, SubElement, CLIP_NONE,
};

/// A curve, ready to be uploaded.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CurveDraw {
    /// The primitive: the bounding box of the segment.
    pub primitive: GpuPrimitiveInstance,
    /// The payload: the control points and the stroke width.
    pub payload: GpuPayload,
    /// The transform: the rotation of the oriented box.
    ///
    /// The translation is zero: the box is already anchored in the chunk.
    pub transform: GpuTransform,
}

impl CurveDraw {
    /// Build a curve draw call.
    ///
    /// * `chunk_id` — the chunk the curve lives in. Pass `[i32::MAX; 2]` to
    ///   derive the chunk from the geometry.
    /// * `instance` — the segment, in world coordinates.
    /// * `fill_token` — the theme token of the stroke.
    /// * `line_width` — the stroke width in logical pixels.
    /// * `z` — the depth of the curve.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chunk_id: [i32; 2],
        instance: &CurveInstance,
        fill_token: u32,
        line_width: f32,
        z: u16,
    ) -> Self {
        // The box lives in the chunk; its corner is the anchor.
        let world_origin = Vec2::new(instance.origin[0], instance.origin[1]);
        let (derived, local) = unbound_vector::to_chunk_local(world_origin);
        let primitive = GpuPrimitiveInstance {
            chunk_id: if chunk_id == [i32::MAX; 2] {
                derived
            } else {
                chunk_id
            },
            local_pos: local,
            size: instance.size,
            corner_radii: [0; 4],
            border_width: 0,
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: CLIP_NONE,
            fill_token_or_uv: fill_token,
            border_token_or_tex: fill_token,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                PrimType::Curve,
                crate::data::BlendMode::Normal,
                PrimFlags::with_sub(SubElement::Background, PrimFlags::EMPTY).bits(),
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: 0,
        };
        let payload = GpuPayload::curve(instance.p0, instance.p1, instance.p2, line_width);
        let transform = GpuTransform {
            linear: instance.matrix,
            translation: [0.0, 0.0],
            scale_hint: 1.0,
            flags: 0,
        };
        Self {
            primitive,
            payload,
            transform,
        }
    }
}

/// Split a cubic Bézier into GPU-ready curve draw calls.
///
/// The control points must already be in world coordinates; the chunk is
/// derived from them.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn cubic_to_draws(
    p0: Vec2,
    p1: Vec2,
    p2: Vec2,
    p3: Vec2,
    tolerance: f32,
    fill_token: u32,
    line_width: f32,
    z: u16,
) -> Vec<CurveDraw> {
    unbound_vector::cubic_to_instances(p0, p1, p2, p3, tolerance, line_width)
        .into_iter()
        .map(|instance| CurveDraw::new([i32::MAX; 2], &instance, fill_token, line_width, z))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_carries_its_control_points_in_the_payload() {
        let instance = CurveInstance {
            origin: [10.0, 20.0],
            size: [40.0, 30.0],
            matrix: [1.0, 0.0, 0.0, 1.0],
            p0: [0.0, 10.0],
            p1: [20.0, 0.0],
            p2: [40.0, 25.0],
        };
        let draw = CurveDraw::new([0, 0], &instance, 2, 3.0, 100);
        assert_eq!(draw.primitive.local_pos, [10.0, 20.0]);
        assert_eq!(draw.primitive.size, [40.0, 30.0]);
        assert_eq!(draw.payload.p0, [0.0, 10.0]);
        assert_eq!(draw.payload.p1, [20.0, 0.0]);
        assert_eq!(draw.payload.p2, [40.0, 25.0]);
        assert_eq!(draw.payload.line_width, 3.0);
        assert_eq!(draw.transform.linear, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_cubic_becomes_several_draw_calls() {
        let draws = cubic_to_draws(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 200.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(300.0, 0.0),
            0.05,
            2,
            2.0,
            100,
        );
        assert!(draws.len() >= 2);
        for draw in &draws {
            assert!(draw.primitive.size[0] > 0.0 && draw.primitive.size[1] > 0.0);
        }
    }
}

//! Curves on the GPU (`docs` §3.4).
//!
//! A curve reaches the GPU as two things: a primitive whose box is the tight
//! bounding rectangle of the segment, and a payload that carries the three
//! control points *relative to that box*. The fragment shader then evaluates
//! the analytic distance to the quadratic Bézier, so the stroke is exact even
//! though the rasteriser only sees a rectangle.

use unbound_math::Vec2;
use unbound_vector::CurveInstance;

use crate::data::{GpuPayload, GpuPrimitiveInstance, PrimFlags, PrimType, SubElement, CLIP_NONE};

/// A curve, ready to be uploaded.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CurveDraw {
    /// The primitive: the bounding box of the segment.
    pub primitive: GpuPrimitiveInstance,
    /// The payload: the control points and the stroke width.
    pub payload: GpuPayload,
}

impl CurveDraw {
    /// Build a curve draw call.
    ///
    /// * `chunk_id` — the chunk the control points live in.
    /// * `instance` — the segment, already in chunk-local coordinates.
    /// * `fill_token` — the theme token of the stroke.
    /// * `line_width` — the stroke width in logical pixels.
    /// * `z` — the depth of the curve.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chunk_id: [i32; 2],
        instance: &CurveInstance,
        fill_token: u32,
        line_width: f32,
        z: u16,
    ) -> Self {
        let primitive = GpuPrimitiveInstance {
            chunk_id,
            local_pos: instance.origin,
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
        Self { primitive, payload }
    }
}

/// Split a cubic Bézier into GPU-ready curve draw calls.
///
/// The control points must already be in world coordinates; the chunk is
/// derived from them.
#[must_use]
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
    unbound_vector::cubic_to_instances(p0, p1, p2, p3, tolerance)
        .into_iter()
        .map(|instance| {
            let chunk = unbound_vector::to_chunk_local(Vec2::new(instance.origin[0], instance.origin[1]));
            CurveDraw::new(chunk.0, &instance, fill_token, line_width, z)
        })
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

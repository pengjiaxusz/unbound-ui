//! GPU 曲线渲染（参见 `docs/architecture/gpu-renderer.md`）。
//!
//! 曲线以三部分数据提交至 GPU：以线段**紧凑有向包围盒（OBB）**为边界的图元、
//! 将该包围盒旋转到位的变换矩阵，以及在包围盒自身局部坐标系下存储三个控制点的载荷。
//! 片元着色器随后计算到二次 Bézier 曲线的解析距离，因此即便光栅化器只看到矩形，
//! 描边依然精确无误。

use unbound_math::Vec2;
use unbound_vector::CurveInstance;

use crate::data::{
    GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType, SubElement, CLIP_NONE,
};

/// 已就绪、可直接上传的曲线数据。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CurveDraw {
    /// 图元：曲线段的包围盒。
    pub primitive: GpuPrimitiveInstance,
    /// 载荷：控制点与描边宽度。
    pub payload: GpuPayload,
    /// 变换：有向包围盒的旋转矩阵。
    ///
    /// 平移分量为零：包围盒已直接锚定在 chunk 内。
    pub transform: GpuTransform,
}

impl CurveDraw {
    /// 构建单条曲线的绘制数据。
    ///
    /// * `chunk_id` — 曲线所属的 chunk；传入 `[i32::MAX; 2]` 时自动从几何坐标推导。
    /// * `instance` — 世界坐标下的曲线段。
    /// * `fill_token` — 描边的主题颜色令牌。
    /// * `line_width` — 逻辑像素描边宽度。
    /// * `z` — 曲线深度值。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chunk_id: [i32; 2],
        instance: &CurveInstance,
        fill_token: u32,
        line_width: f32,
        z: u16,
    ) -> Self {
        // 包围盒位于 chunk 内，以其角点作为锚点。
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

/// 将三次 Bézier 曲线拆分为可提交至 GPU 的二次曲线绘制列表。
///
/// 控制点须已处于世界坐标系；所属 chunk 将自动由控制点推导。
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

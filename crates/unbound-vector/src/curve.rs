//! 将贝塞尔曲线转换为 GPU 可绘制的实例（`docs` §3.4）。
//!
//! GPU 将曲线绘制为一个**包围盒**加三个控制点：包围盒是线段控制多边形的
//! **紧凑有向包围矩形（OBB）**，片元着色器在其内部计算到二次贝塞尔曲线的解析距离。
//!
//! 方向至关重要：对角连线的轴对齐包围盒覆盖面积是连线本身的数倍，
//! 每个多余片元都要付出一次三次方程求根的代价。紧凑包围盒表示为实例的
//! **变换**（旋转矩阵），使顶点着色器无需第二条代码路径即可定位，
//! 且控制点直接存储在该包围盒的局部坐标系中——这正是片元着色器重建的坐标系。
//!
//! 三次贝塞尔曲线会先拆分为二次贝塞尔曲线。三次曲线存在**精确的**双二次分解
//! （在 `t = 0.5` 处），因此拆分本身不引入近似；仅展平容差影响最终结果。
//!
//!
//!
//!

use unbound_math::Vec2;

use crate::{cubic_to_quads, Obb, QuadSegment};

/// 为 GPU 准备的二次贝塞尔曲线实例。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CurveInstance {
    /// 包围盒左下角的世界坐标。
    pub origin: [f32; 2],
    /// 包围盒在世界单位下的尺寸。
    pub size: [f32; 2],
    /// 将包围盒映射到世界空间的旋转矩阵：`m00, m10, m01, m11`。
    pub matrix: [f32; 4],
    /// 第一个控制点，位于包围盒局部坐标系中。
    pub p0: [f32; 2],
    /// 中间控制点，位于包围盒局部坐标系中。
    pub p1: [f32; 2],
    /// 最后一个控制点，位于包围盒局部坐标系中。
    pub p2: [f32; 2],
}

impl CurveInstance {
    /// 从单条二次贝塞尔线段构建 GPU 实例。
    ///
    /// 包围盒为控制多边形的紧凑有向包围盒（OBB），向外扩展半个描边宽度
    /// 加上 1 像素抗锯齿边距。
    #[must_use]
    pub fn from_quad(segment: &QuadSegment, line_width: f32) -> Self {
        let obb = Obb::from_quad(segment, line_width);
        let corner = obb.to_world(Vec2::ZERO);
        let size = obb.size();
        let local = |point: Vec2| obb.to_local(point).to_array();
        Self {
            origin: corner.to_array(),
            size: size.to_array(),
            matrix: obb.matrix(),
            p0: local(segment.p0),
            p1: local(segment.p1),
            p2: local(segment.p2),
        }
    }
}

/// 将三次贝塞尔曲线拆分为可直接送入 GPU 的二次贝塞尔线段。
///
/// 返回的线段按沿曲线的顺序排列，每条线段携带其自身包围盒坐标系下的控制点。
///
#[must_use]
pub fn cubic_to_instances(
    p0: Vec2,
    p1: Vec2,
    p2: Vec2,
    p3: Vec2,
    tolerance: f32,
    line_width: f32,
) -> Vec<CurveInstance> {
    cubic_to_quads(p0, p1, p2, p3, tolerance)
        .iter()
        .map(|segment| CurveInstance::from_quad(segment, line_width))
        .collect()
}

/// 整条三次贝塞尔曲线在世界坐标中的轴对齐包围盒。
#[must_use]
pub fn cubic_bounds(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> (Vec2, Vec2) {
    let lo = p0.min(p1).min(p2).min(p3);
    let hi = p0.max(p1).max(p2).max(p3);
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_points_are_in_the_box_frame() {
        let segment = QuadSegment::new(
            Vec2::new(10.0, 20.0),
            Vec2::new(30.0, 5.0),
            Vec2::new(50.0, 40.0),
        );
        let instance = CurveInstance::from_quad(&segment, 2.0);
        // 包围盒是有向的：其尺寸沿弦向而非世界坐标轴，
        // 因此不等于控制多边形的宽高。
        assert!(instance.size[0] > 0.0 && instance.size[1] > 0.0);
        // 所有控制点都必须位于包围盒内部。
        for point in [instance.p0, instance.p1, instance.p2] {
            assert!(point[0] >= -0.001 && point[0] <= instance.size[0] + 0.001);
            assert!(point[1] >= -0.001 && point[1] <= instance.size[1] + 0.001);
        }
    }

    #[test]
    fn a_diagonal_curve_gets_a_tighter_box_than_an_axis_aligned_one() {
        // 对角线：有向包围盒是细长条带，而轴对齐包围盒是正方形。
        //
        let segment = QuadSegment::line(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
        let instance = CurveInstance::from_quad(&segment, 2.0);
        let area = instance.size[0] * instance.size[1];
        assert!(
            area < 100.0 * 100.0 * 0.5,
            "the oriented box is {area} px², the axis-aligned one would be 10 000 px²"
        );
    }

    #[test]
    fn the_box_follows_the_curve() {
        let instance = CurveInstance::from_quad(
            &QuadSegment::line(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)),
            2.0,
        );
        // 水平线：包围盒宽而扁平。
        assert!(instance.size[0] > 90.0);
        assert!(instance.size[1] < 10.0);
    }

    #[test]
    fn every_point_of_the_curve_lies_inside_the_box() {
        let instances = cubic_to_instances(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 200.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(300.0, 0.0),
            0.05,
            2.0,
        );
        assert!(!instances.is_empty());
        for instance in &instances {
            for point in [instance.p0, instance.p1, instance.p2] {
                assert!(point[0] >= -0.001 && point[0] <= instance.size[0] + 0.001);
                assert!(point[1] >= -0.001 && point[1] <= instance.size[1] + 0.001);
            }
        }
    }

    #[test]
    fn the_cubic_is_split_into_more_than_one_segment() {
        let instances = cubic_to_instances(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 200.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(300.0, 0.0),
            0.05,
            2.0,
        );
        assert!(
            instances.len() >= 2,
            "a cubic needs two quadratics at least"
        );
    }

    #[test]
    fn the_bounds_cover_the_control_polygon() {
        let (lo, hi) = cubic_bounds(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 200.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(300.0, 0.0),
        );
        assert_eq!(lo, Vec2::new(0.0, 0.0));
        assert_eq!(hi, Vec2::new(300.0, 200.0));
    }
}

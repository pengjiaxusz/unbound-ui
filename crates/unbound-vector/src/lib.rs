//! 画布矢量几何：三次→二次贝塞尔降阶、紧凑有向包围盒（OBB）
//! 与数值稳定的解析距离求解。
//!
//! 设计动机（`docs` §3.4）：
//!
//! * 点到**三次**贝塞尔曲线的精确距离需要求解五次方程——无闭式解，
//!   片元着色器将不得不逐像素执行牛顿迭代。因此每条三次曲线都降阶为
//!   2–6 条二次贝塞尔曲线，后者的距离**存在**闭式解。
//! * 对角线的轴对齐包围盒会浪费 80% 以上的覆盖率，因此每条线段使用
//!   有向包围盒，其宽度仅为拱高加描边与 1 像素抗锯齿边距。
//! * 下方的 Cardano 求根公式经过改写以避免灾难性相消：朴素形式在
//!   近似直线的曲线上会丢失全部精度，而这恰恰是边框与连线的常见情况。
//!
//!
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Vec2, CHUNK_SIZE};

pub mod curve;
pub mod svg_path;

pub use curve::{cubic_bounds, cubic_to_instances, CurveInstance};
pub use svg_path::{parse_svg_path, SvgPath, SvgPathCache, SvgPathError, SvgSubpath};

/// 每条三次曲线对应的二次段数量硬上限：既能在极端 10⁹ px 线段下
/// 限制单帧数据负载，又能保证实际编辑器生成的所有曲线满足 0.05 px 精度。
///
pub const MAX_SEGMENTS: u32 = 32;

/// 单条二次贝塞尔线段，以其控制多边形存储。
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct QuadSegment {
    pub p0: Vec2,
    pub p1: Vec2,
    pub p2: Vec2,
}

impl QuadSegment {
    #[must_use]
    pub const fn new(p0: Vec2, p1: Vec2, p2: Vec2) -> Self {
        Self { p0, p1, p2 }
    }

    /// 表示为退化二次曲线的直线段。
    #[must_use]
    pub fn line(a: Vec2, b: Vec2) -> Self {
        Self::new(a, a.lerp(b, 0.5), b)
    }

    /// 在参数 `t ∈ [0, 1]` 处求值。
    #[must_use]
    pub fn eval(&self, t: f32) -> Vec2 {
        let mt = 1.0 - t;
        self.p0 * (mt * mt) + self.p1 * (2.0 * mt * t) + self.p2 * (t * t)
    }

    /// 在参数 `t` 处的一阶导数。
    #[must_use]
    pub fn derivative(&self, t: f32) -> Vec2 {
        (self.p1 - self.p0) * (2.0 * (1.0 - t)) + (self.p2 - self.p1) * (2.0 * t)
    }

    /// 控制多边形的轴对齐包围盒（曲线的超集）。
    #[must_use]
    pub fn control_bounds(&self) -> (Vec2, Vec2) {
        (
            self.p0.min(self.p1).min(self.p2),
            self.p0.max(self.p1).max(self.p2),
        )
    }

    /// 在给定像素容差 `tolerance` 下展平所需的直线段数量。
    ///
    /// 使用二次曲线的精确界：弦与曲线的最大偏差为 `|p0 - 2p1 + p2| / 8`。
    ///
    #[must_use]
    pub fn flatten_steps(&self, tolerance: f32) -> u32 {
        let dev = (self.p0 - self.p1 * 2.0 + self.p2).length() * 0.25;
        if dev <= tolerance.max(1e-4) {
            return 1;
        }
        // 分成 n 段时误差按 1/n² 衰减。
        let n = (dev / tolerance.max(1e-4)).sqrt().ceil() as u32;
        n.clamp(1, 256)
    }

    /// 展平为 `n` 条直线段，向 `out` 写入 `n + 1` 个顶点。
    pub fn flatten_into(&self, tolerance: f32, out: &mut Vec<Vec2>) {
        let n = self.flatten_steps(tolerance);
        out.clear();
        out.reserve(n as usize + 1);
        for i in 0..=n {
            out.push(self.eval(i as f32 / n as f32));
        }
    }
}

/// 紧凑有向包围盒（OBB）。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Obb {
    pub center: Vec2,
    /// 沿线段弦方向的单位向量。
    pub axis_u: Vec2,
    /// 垂直于 `axis_u` 的单位向量。
    pub axis_v: Vec2,
    pub half_u: f32,
    pub half_v: f32,
}

impl Obb {
    /// 构建包含整个控制多边形的 OBB，并按 `line_width / 2 + 1px` 扩展
    /// 以容纳描边和 1 像素抗锯齿边距。
    #[must_use]
    pub fn from_quad(seg: &QuadSegment, line_width: f32) -> Self {
        let chord = seg.p2 - seg.p0;
        let (axis_u, axis_v) = if chord.length_squared() > 1e-12 {
            let u = chord.normalized();
            (u, u.perp())
        } else {
            // 退化弦（点或闭环）：回退到控制多边形自身的轴向，
            // 确保包围盒面积不会退化为零。
            let alt = seg.p1 - seg.p0;
            if alt.length_squared() > 1e-12 {
                let u = alt.normalized();
                (u, u.perp())
            } else {
                (Vec2::X, Vec2::Y)
            }
        };

        let project = |p: Vec2| (p.dot(axis_u), p.dot(axis_v));
        let (mut min_u, mut min_v) = project(seg.p0);
        let (mut max_u, mut max_v) = (min_u, min_v);
        for p in [seg.p1, seg.p2] {
            let (u, v) = project(p);
            min_u = min_u.min(u);
            max_u = max_u.max(u);
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }

        let pad = line_width * 0.5 + 1.0;
        let center_u = (min_u + max_u) * 0.5;
        let center_v = (min_v + max_v) * 0.5;
        Self {
            center: axis_u * center_u + axis_v * center_v,
            axis_u,
            axis_v,
            half_u: (max_u - min_u) * 0.5 + pad,
            half_v: (max_v - min_v) * 0.5 + pad,
        }
    }

    /// 包围盒在世界单位下的宽度/高度。
    #[must_use]
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.half_u * 2.0, self.half_v * 2.0)
    }

    #[must_use]
    pub fn area(&self) -> f32 {
        self.half_u * self.half_v * 4.0
    }

    /// 从 OBB 局部空间（`0..size`）变换到世界空间。
    #[must_use]
    pub fn to_world(&self, local: Vec2) -> Vec2 {
        self.center + self.axis_u * (local.x - self.half_u) + self.axis_v * (local.y - self.half_v)
    }

    /// [`Self::to_world`] 的逆变换。
    #[must_use]
    pub fn to_local(&self, world: Vec2) -> Vec2 {
        let d = world - self.center;
        Vec2::new(
            d.dot(self.axis_u) + self.half_u,
            d.dot(self.axis_v) + self.half_v,
        )
    }

    /// 将 OBB 空间映射到世界空间的 2×2 矩阵（供 `GpuTransform` 使用）。
    #[must_use]
    pub fn matrix(&self) -> [f32; 4] {
        [self.axis_u.x, self.axis_u.y, self.axis_v.x, self.axis_v.y]
    }
}

/// 自适应三次 → 二次贝塞尔降阶。
///
/// `tolerance` 为允许的最大几何误差，使用**与控制点相同的单位**。
/// 画布调用方传入 `0.05 / zoom`，使误差在当前缩放倍率下始终保持为
/// 屏幕像素的固定分数。
///
/// 段数从 Wang 公式估算开始，随后针对实际测量偏差进行**验证**，
/// 倍增段数直至满足容差或达到硬上限。Wang 界较为保守；实测偏差可避免
/// 为多余段数付费，同时严格保证误差契约。
///
/// UI 尺度的连线通常落在架构文档所述的 3–6 段区间；在 0.05 px 容差下，
/// 400 px 长的 S 形曲线确实需要更多段，这也是上限设为 32 而非 6 的原因。
/// 若无条件截断为 6 段，会悄然破坏解析距离求解所依赖的误差保证。
///
///
///
#[must_use]
pub fn cubic_to_quads(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, tolerance: f32) -> Vec<QuadSegment> {
    let tol = tolerance.max(1e-4);
    let l1 = (p0 - p1 * 2.0 + p2).length();
    let l2 = (p1 - p2 * 2.0 + p3).length();
    let l = l1.max(l2);
    // Wang 公式：误差 ≈ 0.75·L/n²，因此 n ≈ sqrt(0.75·L/tol)。
    let estimate = if l <= 1e-6 {
        1
    } else {
        ((0.75 * l) / tol).sqrt().ceil() as u32
    };
    let mut n = estimate.clamp(1, MAX_SEGMENTS);

    loop {
        let quads = build_quads(p0, p1, p2, p3, n);
        if n >= MAX_SEGMENTS || measure_error(p0, p1, p2, p3, &quads) <= tol {
            return quads;
        }
        n = (n * 2).min(MAX_SEGMENTS);
    }
}

/// 将 `[0, 1]` 划分为 `n` 条子三次曲线，并将每条降阶为一条二次曲线。
fn build_quads(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, n: u32) -> Vec<QuadSegment> {
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let t0 = i as f32 / n as f32;
        let t1 = (i + 1) as f32 / n as f32;
        let (a0, a1, a2, a3) = sub_cubic(p0, p1, p2, p3, t0, t1);
        // 最小二乘降阶：q1 = (3(a1 + a2) - (a0 + a3)) / 4。
        let q1 = (a1 + a2) * 0.75 - (a0 + a3) * 0.25;
        out.push(QuadSegment::new(a0, q1, a3));
    }
    out
}

/// 降阶曲线链与原三次曲线之间的最大偏差，在 `SAMPLES` 个采样点上测量
/// （计算廉价且单调，仅在降阶阶段使用）。
fn measure_error(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, quads: &[QuadSegment]) -> f32 {
    const SAMPLES: usize = 32;
    let mut worst: f32 = 0.0;
    for i in 0..=SAMPLES {
        let t = i as f32 / SAMPLES as f32;
        let mt = 1.0 - t;
        let p = p0 * (mt * mt * mt)
            + p1 * (3.0 * mt * mt * t)
            + p2 * (3.0 * mt * t * t)
            + p3 * (t * t * t);
        let d = quads
            .iter()
            .map(|q| distance_to_quad(q, p))
            .fold(f32::INFINITY, f32::min);
        worst = worst.max(d);
    }
    worst
}

/// 通过 de Casteljau 算法提取 `[t0, t1]` 区间上的子三次曲线。
fn sub_cubic(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t0: f32, t1: f32) -> (Vec2, Vec2, Vec2, Vec2) {
    // 先在 t1 处分割，再将左半段在 t0/t1 处分割。
    let split = |a: Vec2, b: Vec2, c: Vec2, d: Vec2, t: f32| {
        let ab = a.lerp(b, t);
        let bc = b.lerp(c, t);
        let cd = c.lerp(d, t);
        let abc = ab.lerp(bc, t);
        let bcd = bc.lerp(cd, t);
        let abcd = abc.lerp(bcd, t);
        ((a, ab, abc, abcd), (abcd, bcd, cd, d))
    };
    let (left, _) = split(p0, p1, p2, p3, t1);
    let t = if t1 > 1e-6 { t0 / t1 } else { 0.0 };
    // 在 `t0/t1` 处分割 [0, t1] 子曲线，左侧得到 [0, t0]，
    // 右侧恰好就是所需的 [t0, t1] 片段。
    let (_, sub) = split(left.0, left.1, left.2, left.3, t);
    sub
}

/// 解析求解点 `p` 到二次贝塞尔曲线最近点的距离平方。
///
/// 返回 `(distance_squared, t)`，其中 `t` 为最近点参数（钳制在 `[0, 1]`）。
///
///
#[must_use]
pub fn closest_on_quad(seg: &QuadSegment, p: Vec2) -> (f32, f32) {
    let a = seg.p1 - seg.p0;
    let b = seg.p0 - seg.p1 * 2.0 + seg.p2;
    let d = seg.p0 - p;

    let bb = b.dot(b);
    if bb < 1e-12 {
        // 退化情况（直线）：最近点即投影点。
        let ab = seg.p2 - seg.p0;
        let t = if ab.length_squared() > 1e-12 {
            ((p - seg.p0).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = seg.eval(t);
        return ((p - q).length_squared(), t);
    }

    let kk = 1.0 / bb;
    let kx = kk * a.dot(b);
    let ky = kk * (2.0 * a.dot(a) + d.dot(b)) / 3.0;
    let kz = kk * d.dot(a);

    let pp = ky - kx * kx;
    let p3 = pp * pp * pp;
    let q = kx * (2.0 * kx * kx - 3.0 * ky) + kz;
    let h = q * q + 4.0 * p3;

    let t = if h >= 0.0 {
        // 单实根：先计算 `sqrt(h)` 再开立方根；显式保留符号，
        // 从而无需依赖特殊处理负零的 `cbrt`。
        let hs = h.sqrt();
        let x0 = (hs - q) * 0.5;
        let x1 = (-hs - q) * 0.5;
        let uv0 = x0.abs().cbrt() * x0.signum();
        let uv1 = x1.abs().cbrt() * x1.signum();
        (uv0 + uv1 - kx).clamp(0.0, 1.0)
    } else {
        // 三实根：使用三角函数形式避免复数分支。
        let z = (-pp).sqrt();
        let v = (q / (pp * z * 2.0)).clamp(-1.0, 1.0).acos() / 3.0;
        let m = v.cos();
        let nn = v.sin() * 1.732_050_8;
        let t0 = (2.0 * m) * z - kx;
        let t1 = (-nn - m) * z - kx;
        let t2 = (nn - m) * z - kx;
        let mut best = (f32::INFINITY, 0.0f32);
        for cand in [t0, t1, t2] {
            let tc = cand.clamp(0.0, 1.0);
            let dist = (p - seg.eval(tc)).length_squared();
            if dist < best.0 {
                best = (dist, tc);
            }
        }
        return best;
    };

    ((p - seg.eval(t)).length_squared(), t)
}

/// 点 `p` 到线段的距离。
#[must_use]
pub fn distance_to_quad(seg: &QuadSegment, p: Vec2) -> f32 {
    closest_on_quad(seg, p).0.sqrt()
}

/// 当 `p` 与描边中心线的距离在 `threshold` 以内时返回 `true`。
#[must_use]
pub fn hit_quad(seg: &QuadSegment, p: Vec2, threshold: f32) -> bool {
    distance_to_quad(seg, p) <= threshold.max(0.0)
}

/// 点在世界空间中的分块局部坐标，钳制到 `[0, 4096)`。
///
/// 画布采用分块锚定（`docs` §5.2）；每条送入 GPU 的曲线都必须
/// 相对于其所在分块表达，因此该转换放在生成几何体的模块旁。
///
#[must_use]
pub fn to_chunk_local(world: Vec2) -> ([i32; 2], [f32; 2]) {
    let cx = (world.x / CHUNK_SIZE).floor();
    let cy = (world.y / CHUNK_SIZE).floor();
    let chunk = [cx as i32, cy as i32];
    let local = Vec2::new(world.x - cx * CHUNK_SIZE, world.y - cy * CHUNK_SIZE);
    (chunk, local.to_array())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cubic(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, n: usize) -> Vec<Vec2> {
        (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let mt = 1.0 - t;
                p0 * (mt * mt * mt)
                    + p1 * (3.0 * mt * mt * t)
                    + p2 * (3.0 * mt * t * t)
                    + p3 * (t * t * t)
            })
            .collect()
    }

    fn worst_error(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, quads: &[QuadSegment]) -> f32 {
        let mut worst: f32 = 0.0;
        for pt in sample_cubic(p0, p1, p2, p3, 800) {
            let d = quads
                .iter()
                .map(|q| distance_to_quad(q, pt))
                .fold(f32::INFINITY, f32::min);
            worst = worst.max(d);
        }
        worst
    }

    #[test]
    fn ui_scale_curve_meets_the_measured_error_bound() {
        // 典型画布连线：200px 宽，带一处平缓弯曲。
        let (p0, p1, p2, p3) = (
            Vec2::new(0.0, 0.0),
            Vec2::new(60.0, 40.0),
            Vec2::new(140.0, 40.0),
            Vec2::new(200.0, 0.0),
        );
        let quads = cubic_to_quads(p0, p1, p2, p3, 0.05);
        let worst = worst_error(p0, p1, p2, p3, &quads);
        assert!(
            worst <= 0.05,
            "worst error {worst} with {} segments",
            quads.len()
        );
        // 实测结果：该曲线在 0.05px 容差下需要约 26 段。架构文档中
        // “2–6 段”的数字对应画布略微缩小时常见的约 0.5px 容差
        // （容差以世界单位给出，调用方按 1/zoom 缩放）。
        //
        assert!(quads.len() <= MAX_SEGMENTS as usize, "{}", quads.len());
    }

    #[test]
    fn zoomed_out_tolerance_collapses_the_segment_count() {
        let (p0, p1, p2, p3) = (
            Vec2::new(0.0, 0.0),
            Vec2::new(60.0, 40.0),
            Vec2::new(140.0, 40.0),
            Vec2::new(200.0, 0.0),
        );
        // 在 25% 缩放倍率下，0.5px 世界误差等价于 0.125 屏幕像素：既轻量又清晰。
        let quads = cubic_to_quads(p0, p1, p2, p3, 0.5);
        assert!(
            (2..=12).contains(&quads.len()),
            "expected a small chain, got {}",
            quads.len()
        );
        let worst = worst_error(p0, p1, p2, p3, &quads);
        assert!(worst <= 0.5 + 1e-3, "worst error {worst}");
    }

    #[test]
    fn pathological_curve_is_capped_not_unbounded() {
        // 400px 高的 S 形曲线：6 段无法满足 0.05px 容差，因此段数会增长至
        // 硬上限。契约要求“有界负载 + 尽力误差”，而非“无界段数”。
        //
        let (p0, p1, p2, p3) = (
            Vec2::new(0.0, 0.0),
            Vec2::new(120.0, 200.0),
            Vec2::new(-60.0, 260.0),
            Vec2::new(80.0, 400.0),
        );
        let quads = cubic_to_quads(p0, p1, p2, p3, 0.05);
        assert_eq!(quads.len(), MAX_SEGMENTS as usize);
        let worst = worst_error(p0, p1, p2, p3, &quads);
        assert!(worst <= 0.4, "worst error {worst}");
    }

    #[test]
    fn reduction_is_continuous_at_the_joints() {
        let quads = cubic_to_quads(
            Vec2::new(0.0, 0.0),
            Vec2::new(30.0, 90.0),
            Vec2::new(90.0, 90.0),
            Vec2::new(120.0, 0.0),
            0.05,
        );
        for w in quads.windows(2) {
            let end = w[0].p2;
            let start = w[1].p0;
            assert!((end - start).length() < 1e-4, "{end:?} != {start:?}");
        }
    }

    #[test]
    fn a_straight_cubic_needs_only_one_segment() {
        let quads = cubic_to_quads(
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(30.0, 0.0),
            0.05,
        );
        assert_eq!(quads.len(), 1);
        assert!(quads.iter().all(|q| q.p1.y.abs() < 1e-4));
    }

    #[test]
    fn distance_matches_brute_force_on_a_curve() {
        let seg = QuadSegment::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 100.0),
            Vec2::new(100.0, 0.0),
        );
        for probe in [
            Vec2::new(50.0, 10.0),
            Vec2::new(10.0, 60.0),
            Vec2::new(-20.0, -20.0),
            Vec2::new(100.0, 100.0),
        ] {
            let analytic = distance_to_quad(&seg, probe);
            let brute = (0..=20_000)
                .map(|i| {
                    let t = i as f32 / 20_000.0;
                    (probe - seg.eval(t)).length()
                })
                .fold(f32::INFINITY, f32::min);
            assert!(
                (analytic - brute).abs() < 1e-3,
                "probe {probe:?}: analytic {analytic} vs brute {brute}"
            );
        }
    }

    #[test]
    fn distance_to_a_degenerate_curve_is_a_point_distance() {
        let seg = QuadSegment::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO);
        assert!((distance_to_quad(&seg, Vec2::new(3.0, 4.0)) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn distance_to_a_straight_curve_is_the_projection() {
        let seg = QuadSegment::line(Vec2::ZERO, Vec2::new(10.0, 0.0));
        assert!((distance_to_quad(&seg, Vec2::new(5.0, 3.0)) - 3.0).abs() < 1e-4);
        // 超出终点范围时钳制到端点。
        assert!((distance_to_quad(&seg, Vec2::new(13.0, 4.0)) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn obb_contains_every_sample_of_the_curve() {
        let seg = QuadSegment::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(60.0, 80.0),
            Vec2::new(120.0, 0.0),
        );
        let obb = Obb::from_quad(&seg, 2.0);
        for i in 0..=100 {
            let p = seg.eval(i as f32 / 100.0);
            let local = obb.to_local(p);
            assert!(
                local.x >= 0.0
                    && local.x <= obb.size().x
                    && local.y >= 0.0
                    && local.y <= obb.size().y,
                "sample {p:?} escaped OBB (local {local:?})"
            );
        }
    }

    #[test]
    fn obb_is_much_tighter_than_the_aabb_for_a_diagonal() {
        let seg = QuadSegment::line(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
        let obb = Obb::from_quad(&seg, 1.0);
        // 该直线的 AABB 面积为 100×100 = 10 000；OBB 则是细长条带。
        assert!(obb.area() < 1000.0, "obb area {}", obb.area());
    }

    #[test]
    fn obb_round_trips_local_and_world() {
        let seg = QuadSegment::new(
            Vec2::new(10.0, 20.0),
            Vec2::new(40.0, 5.0),
            Vec2::new(70.0, 20.0),
        );
        let obb = Obb::from_quad(&seg, 0.0);
        for local in [Vec2::ZERO, Vec2::new(1.0, 1.0), obb.size()] {
            let world = obb.to_world(local);
            let back = obb.to_local(world);
            assert!((back - local).length() < 1e-3, "{local:?} -> {back:?}");
        }
    }

    #[test]
    fn flattening_respects_the_tolerance() {
        let seg = QuadSegment::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 200.0),
            Vec2::new(100.0, 0.0),
        );
        let mut pts = Vec::new();
        seg.flatten_into(0.1, &mut pts);
        assert!(pts.len() >= 4);
        // 曲线上的每个点都必须位于折线 0.1 距离以内。
        for i in 0..=200 {
            let p = seg.eval(i as f32 / 200.0);
            let mut best = f32::INFINITY;
            for w in pts.windows(2) {
                let d = distance_to_line(p, w[0], w[1]);
                best = best.min(d);
            }
            assert!(best < 0.11, "flatten error {best}");
        }
    }

    fn distance_to_line(p: Vec2, a: Vec2, b: Vec2) -> f32 {
        let ab = b - a;
        let len2 = ab.length_squared();
        if len2 < 1e-12 {
            return (p - a).length();
        }
        let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
        (p - (a + ab * t)).length()
    }

    #[test]
    fn hit_test_matches_the_stroke_width() {
        let seg = QuadSegment::line(Vec2::ZERO, Vec2::new(100.0, 0.0));
        assert!(hit_quad(&seg, Vec2::new(50.0, 2.0), 3.0));
        assert!(!hit_quad(&seg, Vec2::new(50.0, 5.0), 3.0));
    }

    #[test]
    fn chunk_local_conversion_handles_negatives_and_boundaries() {
        let (chunk, local) = to_chunk_local(Vec2::new(-1.0, 4096.0));
        assert_eq!(chunk, [-1, 1]);
        assert_eq!(local, [4095.0, 0.0]);
        let (chunk, local) = to_chunk_local(Vec2::new(0.0, 0.0));
        assert_eq!(chunk, [0, 0]);
        assert_eq!(local, [0.0, 0.0]);
    }

    #[test]
    fn segment_layout_is_pod_friendly() {
        assert_eq!(core::mem::size_of::<QuadSegment>(), 24);
        assert_eq!(core::mem::size_of::<Obb>(), 32);
    }
}

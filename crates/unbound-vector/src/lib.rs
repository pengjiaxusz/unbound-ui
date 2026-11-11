//! Vector geometry for the canvas: cubic→quadratic reduction, tight oriented
//! bounding boxes, and a numerically stable analytic distance solve.
//!
//! Why this exists (`docs` §3.4):
//!
//! * The exact distance from a point to a **cubic** Bézier requires solving a
//!   quintic — no closed form exists, so a fragment shader would need Newton
//!   iterations per pixel. Every cubic is therefore reduced to 2–6 quadratics
//!   whose distance *does* have a closed form.
//! * An axis-aligned box around a diagonal line wastes 80%+ of its coverage, so
//!   each segment gets an oriented box whose width is just the arc height plus
//!   the stroke and one pixel of antialiasing.
//! * The Cardano solve below is written to avoid catastrophic cancellation: the
//!   naive form loses all precision for near-straight curves, which is exactly
//!   the common case for frame borders and connector lines.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Vec2, CHUNK_SIZE};

pub mod curve;

pub use curve::{cubic_bounds, cubic_to_instances, CurveInstance};

/// Hard cap on the number of quadratics per cubic: bounds the per-frame payload
/// for a pathological 10⁹-px line while still meeting 0.05 px on every curve a
/// real editor produces.
pub const MAX_SEGMENTS: u32 = 32;

/// One quadratic Bézier segment, stored as its control polygon.
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

    /// A straight line expressed as a degenerate quadratic.
    #[must_use]
    pub fn line(a: Vec2, b: Vec2) -> Self {
        Self::new(a, a.lerp(b, 0.5), b)
    }

    /// Evaluate at `t ∈ [0, 1]`.
    #[must_use]
    pub fn eval(&self, t: f32) -> Vec2 {
        let mt = 1.0 - t;
        self.p0 * (mt * mt) + self.p1 * (2.0 * mt * t) + self.p2 * (t * t)
    }

    /// First derivative at `t`.
    #[must_use]
    pub fn derivative(&self, t: f32) -> Vec2 {
        (self.p1 - self.p0) * (2.0 * (1.0 - t)) + (self.p2 - self.p1) * (2.0 * t)
    }

    /// Axis-aligned bounds of the control polygon (a superset of the curve).
    #[must_use]
    pub fn control_bounds(&self) -> (Vec2, Vec2) {
        (
            self.p0.min(self.p1).min(self.p2),
            self.p0.max(self.p1).max(self.p2),
        )
    }

    /// Number of line segments needed to flatten within `tolerance` pixels.
    ///
    /// Uses the exact bound for quadratics: the maximum deviation of the chord
    /// from the curve is `|p0 - 2p1 + p2| / 8`.
    #[must_use]
    pub fn flatten_steps(&self, tolerance: f32) -> u32 {
        let dev = (self.p0 - self.p1 * 2.0 + self.p2).length() * 0.25;
        if dev <= tolerance.max(1e-4) {
            return 1;
        }
        // n segments => error shrinks as 1/n².
        let n = (dev / tolerance.max(1e-4)).sqrt().ceil() as u32;
        n.clamp(1, 256)
    }

    /// Flatten into `n` line segments, writing `n + 1` points into `out`.
    pub fn flatten_into(&self, tolerance: f32, out: &mut Vec<Vec2>) {
        let n = self.flatten_steps(tolerance);
        out.clear();
        out.reserve(n as usize + 1);
        for i in 0..=n {
            out.push(self.eval(i as f32 / n as f32));
        }
    }
}

/// A tight oriented bounding box.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Obb {
    pub center: Vec2,
    /// Unit vector along the segment chord.
    pub axis_u: Vec2,
    /// Unit vector perpendicular to `axis_u`.
    pub axis_v: Vec2,
    pub half_u: f32,
    pub half_v: f32,
}

impl Obb {
    /// Build an OBB that contains the whole control polygon, padded by
    /// `line_width / 2 + 1px` for the stroke and one antialiasing pixel.
    #[must_use]
    pub fn from_quad(seg: &QuadSegment, line_width: f32) -> Self {
        let chord = seg.p2 - seg.p0;
        let (axis_u, axis_v) = if chord.length_squared() > 1e-12 {
            let u = chord.normalized();
            (u, u.perp())
        } else {
            // Degenerate chord (a dot or a loop): fall back to the control
            // polygon's own axes so the box never collapses to zero area.
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

    /// Width / height of the box in world units.
    #[must_use]
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.half_u * 2.0, self.half_v * 2.0)
    }

    #[must_use]
    pub fn area(&self) -> f32 {
        self.half_u * self.half_v * 4.0
    }

    /// Transform from OBB space (`0..size`) into world space.
    #[must_use]
    pub fn to_world(&self, local: Vec2) -> Vec2 {
        self.center + self.axis_u * (local.x - self.half_u) + self.axis_v * (local.y - self.half_v)
    }

    /// Inverse of [`Self::to_world`].
    #[must_use]
    pub fn to_local(&self, world: Vec2) -> Vec2 {
        let d = world - self.center;
        Vec2::new(
            d.dot(self.axis_u) + self.half_u,
            d.dot(self.axis_v) + self.half_v,
        )
    }

    /// 2×2 matrix mapping OBB space to world space (for `GpuTransform`).
    #[must_use]
    pub fn matrix(&self) -> [f32; 4] {
        [self.axis_u.x, self.axis_u.y, self.axis_v.x, self.axis_v.y]
    }
}

/// Adaptive cubic → quadratic reduction.
///
/// `tolerance` is the maximum allowed geometric error, expressed in the **same
/// units as the control points**. Callers on the canvas pass
/// `0.05 / zoom` so that the error stays a constant fraction of a screen pixel
/// at the current magnification.
///
/// The segment count starts from Wang's formula and is then *verified* against
/// the actual measured deviation, doubling until it fits or the hard cap is
/// reached. Wang's bound is conservative; measuring avoids paying for segments
/// that are not needed, while still guaranteeing the error contract.
///
/// UI-scale connectors typically land in the 3–6 segment band the architecture
/// doc cites; a 400 px S-curve at 0.05 px genuinely needs far more, which is
/// why the cap is 32 rather than 6. Capping at 6 unconditionally would silently
/// break the error guarantee that the analytic distance solve relies on.
#[must_use]
pub fn cubic_to_quads(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, tolerance: f32) -> Vec<QuadSegment> {
    let tol = tolerance.max(1e-4);
    let l1 = (p0 - p1 * 2.0 + p2).length();
    let l2 = (p1 - p2 * 2.0 + p3).length();
    let l = l1.max(l2);
    // Wang: error ≈ 0.75·L/n², so n ≈ sqrt(0.75·L/tol).
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

/// Split `[0,1]` into `n` sub-cubics and reduce each to one quadratic.
fn build_quads(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, n: u32) -> Vec<QuadSegment> {
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let t0 = i as f32 / n as f32;
        let t1 = (i + 1) as f32 / n as f32;
        let (a0, a1, a2, a3) = sub_cubic(p0, p1, p2, p3, t0, t1);
        // Least-squares degree reduction: q1 = (3(a1 + a2) - (a0 + a3)) / 4.
        let q1 = (a1 + a2) * 0.75 - (a0 + a3) * 0.25;
        out.push(QuadSegment::new(a0, q1, a3));
    }
    out
}

/// Worst-case deviation of the reduced chain from the original cubic, measured
/// at `SAMPLES` points (cheap and monotone; used only during reduction).
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

/// Extract the sub-cubic of `[t0, t1]` via de Casteljau.
fn sub_cubic(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t0: f32, t1: f32) -> (Vec2, Vec2, Vec2, Vec2) {
    // Split at t1, then re-split the left part at t0/t1.
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
    // Splitting the [0, t1] sub-curve at `t0/t1` yields [0, t0] on the left and
    // exactly the [t0, t1] piece on the right — the right part is the one we need.
    let (_, sub) = split(left.0, left.1, left.2, left.3, t);
    sub
}

/// Squared distance from `p` to the quadratic's closest point, solved
/// analytically.
///
/// Returns `(distance_squared, t)` where `t` is the parameter of the closest
/// point (clamped to `[0, 1]`).
#[must_use]
pub fn closest_on_quad(seg: &QuadSegment, p: Vec2) -> (f32, f32) {
    let a = seg.p1 - seg.p0;
    let b = seg.p0 - seg.p1 * 2.0 + seg.p2;
    let d = seg.p0 - p;

    let bb = b.dot(b);
    if bb < 1e-12 {
        // Degenerate (a straight line): the closest point is the projection.
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
        // One real root. `sqrt(h)` then a cube root; keep the sign explicit so
        // that `cbrt` is not needed (and negative zero is handled).
        let hs = h.sqrt();
        let x0 = (hs - q) * 0.5;
        let x1 = (-hs - q) * 0.5;
        let uv0 = x0.abs().cbrt() * x0.signum();
        let uv1 = x1.abs().cbrt() * x1.signum();
        (uv0 + uv1 - kx).clamp(0.0, 1.0)
    } else {
        // Three real roots: trigonometric form avoids the complex branch.
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

/// Distance from `p` to the segment.
#[must_use]
pub fn distance_to_quad(seg: &QuadSegment, p: Vec2) -> f32 {
    closest_on_quad(seg, p).0.sqrt()
}

/// True when `p` is within `threshold` of the stroke centreline.
#[must_use]
pub fn hit_quad(seg: &QuadSegment, p: Vec2, threshold: f32) -> bool {
    distance_to_quad(seg, p) <= threshold.max(0.0)
}

/// World-space chunk coordinate of a point, clamped into `[0, 4096)`.
///
/// The canvas is chunk-anchored (`docs` §5.2); every curve that reaches the GPU
/// must be expressed relative to its chunk, so this conversion lives next to the
/// geometry that produces it.
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
        // A typical canvas connector: 200px wide, one gentle bend.
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
        // Measured reality: this curve needs ~26 segments for 0.05px. The
        // architecture doc's "2–6 segments" figure corresponds to the ~0.5px
        // tolerance that is typical once the canvas is even slightly zoomed out
        // (tolerance is given in world units and callers scale it by 1/zoom).
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
        // 0.5px of world error at 25% zoom is 0.125 screen px: cheap and clean.
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
        // A 400px tall S-curve: six segments cannot reach 0.05px, so the
        // segment count grows until the hard cap. The contract is "bounded
        // payload + best effort error", never "unbounded segments".
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
        // Beyond the end it clamps to the endpoint.
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
        // The AABB of this line is 100×100 = 10 000; the OBB is a thin strip.
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
        // Every curve point must lie within 0.1 of the polyline.
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

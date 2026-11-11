//! Turning a Bézier into something the GPU can draw (`docs` §3.4).
//!
//! The GPU draws a curve as a *box* plus three control points: the box is the
//! tight bounding rectangle, and the fragment shader evaluates the analytic
//! distance to the quadratic Bézier inside it. Everything the driver needs is
//! therefore relative to the box origin — the shader works in the primitive's
//! local frame, and a curve that leaves its chunk must be re-anchored before
//! it gets here.
//!
//! The cubic is split into quadratics first. A cubic Bézier has an *exact*
//! two-quadratic decomposition (at `t = 0.5`), so nothing is approximated by
//! the split itself; only the flattening tolerance affects the result.

use unbound_math::Vec2;

use crate::{cubic_to_quads, QuadSegment};

/// A quadratic Bézier prepared for the GPU.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CurveInstance {
    /// The origin of the bounding box, in chunk-local coordinates.
    pub origin: [f32; 2],
    /// The size of the bounding box.
    pub size: [f32; 2],
    /// The first control point, relative to `origin`.
    pub p0: [f32; 2],
    /// The middle control point, relative to `origin`.
    pub p1: [f32; 2],
    /// The last control point, relative to `origin`.
    pub p2: [f32; 2],
}

impl CurveInstance {
    /// Build an instance from a quadratic segment.
    ///
    /// `origin` is the chunk-local origin of the bounding box; the control
    /// points are stored relative to it.
    #[must_use]
    pub fn from_quad(segment: &QuadSegment, origin: Vec2) -> Self {
        let (lo, hi) = segment.control_bounds();
        let size = hi - lo;
        let relative = |point: Vec2| [point.x - origin.x, point.y - origin.y];
        Self {
            origin: origin.to_array(),
            size: [size.x, size.y],
            p0: relative(segment.p0),
            p1: relative(segment.p1),
            p2: relative(segment.p2),
        }
    }
}

/// Split a cubic Bézier into GPU-ready quadratic segments.
///
/// The segments are returned in the order they appear along the curve, and
/// each one carries the control points relative to its own bounding box.
///
/// `origin` is the chunk-local origin of the *whole* curve; the per-segment
/// box is placed inside it.
#[must_use]
pub fn cubic_to_instances(
    p0: Vec2,
    p1: Vec2,
    p2: Vec2,
    p3: Vec2,
    tolerance: f32,
) -> Vec<CurveInstance> {
    cubic_to_quads(p0, p1, p2, p3, tolerance)
        .iter()
        .map(|segment| {
            let (lo, _hi) = segment.control_bounds();
            CurveInstance::from_quad(segment, lo)
        })
        .collect()
}

/// The bounding box of a whole cubic, in the same frame as the instances.
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
    fn the_control_points_are_relative_to_the_box() {
        let segment = QuadSegment::new(
            Vec2::new(10.0, 20.0),
            Vec2::new(30.0, 5.0),
            Vec2::new(50.0, 40.0),
        );
        let instance = CurveInstance::from_quad(&segment, Vec2::new(10.0, 5.0));
        assert_eq!(instance.origin, [10.0, 5.0]);
        assert_eq!(instance.p0, [0.0, 15.0]);
        assert_eq!(instance.p1, [20.0, 0.0]);
        assert_eq!(instance.p2, [40.0, 35.0]);
        assert_eq!(instance.size, [40.0, 35.0]);
    }

    #[test]
    fn every_point_of_the_curve_lies_inside_the_box() {
        let instances = cubic_to_instances(
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 200.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(300.0, 0.0),
            0.05,
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
        );
        assert!(instances.len() >= 2, "a cubic needs two quadratics at least");
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

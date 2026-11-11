//! Turning a Bézier into something the GPU can draw (`docs` §3.4).
//!
//! The GPU draws a curve as a *box* plus three control points: the box is the
//! **tight oriented** bounding rectangle of the segment, and the fragment
//! shader evaluates the analytic distance to the quadratic Bézier inside it.
//!
//! The orientation matters: an axis-aligned box around a diagonal connector
//! covers several times the area of the connector itself, and every extra
//! fragment pays for a cubic root solve. The tight box is expressed as the
//! instance's *transform* (a rotation), so the vertex shader can place it
//! without a second code path, and the control points are stored in the box's
//! own frame — which is exactly the frame the fragment shader reconstructs.
//!
//! The cubic is split into quadratics first. A cubic Bézier has an *exact*
//! two-quadratic decomposition (at `t = 0.5`), so nothing is approximated by
//! the split itself; only the flattening tolerance affects the result.

use unbound_math::Vec2;

use crate::{cubic_to_quads, Obb, QuadSegment};

/// A quadratic Bézier prepared for the GPU.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct CurveInstance {
    /// The lower-left corner of the bounding box, in world coordinates.
    pub origin: [f32; 2],
    /// The size of the bounding box, in world units.
    pub size: [f32; 2],
    /// The rotation that maps the box onto the world: `m00, m10, m01, m11`.
    pub matrix: [f32; 4],
    /// The first control point, in the box's local frame.
    pub p0: [f32; 2],
    /// The middle control point, in the box's local frame.
    pub p1: [f32; 2],
    /// The last control point, in the box's local frame.
    pub p2: [f32; 2],
}

impl CurveInstance {
    /// Build an instance from a quadratic segment.
    ///
    /// The box is the tight oriented bounding box of the control polygon,
    /// padded by half the stroke width plus one pixel of antialiasing.
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

/// Split a cubic Bézier into GPU-ready quadratic segments.
///
/// The segments are returned in the order they appear along the curve, and
/// each one carries the control points in its own box frame.
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

/// The bounding box of a whole cubic, in world coordinates.
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
        // The box is oriented: its extents follow the chord, not the world
        // axes, so they are *not* the control polygon's width and height.
        assert!(instance.size[0] > 0.0 && instance.size[1] > 0.0);
        // Every control point must be inside the box.
        for point in [instance.p0, instance.p1, instance.p2] {
            assert!(point[0] >= -0.001 && point[0] <= instance.size[0] + 0.001);
            assert!(point[1] >= -0.001 && point[1] <= instance.size[1] + 0.001);
        }
    }

    #[test]
    fn a_diagonal_curve_gets_a_tighter_box_than_an_axis_aligned_one() {
        // A diagonal line: the oriented box is a thin strip, the axis-aligned
        // one a square.
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
        // A horizontal line: the box is wide and thin.
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

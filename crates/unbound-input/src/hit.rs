//! Double-layer hit testing (`docs` §6.1).
//!
//! The pipeline is deliberately two-staged, exactly as the doc draws it:
//!
//! 1. **broad phase** — an AABB test against the region's bounds, and against
//!    every clip that applies to it,
//! 2. **narrow phase** — the real geometry: a rounded-rect SDF (four per-corner
//!    radii, so the GPU's two-slot corner mask is a strict subset), an even-odd
//!    polygon test, or an analytic quadratic-Bézier distance.
//!
//! Regions are plain values, not trait objects: the caller can build them from
//! the Taffy tree, from the canvas quadtree, or from a Portal overlay without
//! this crate knowing about any of them. Ordering is `(layer, z, node)`, which
//! makes a pick independent of insertion order — the property that keeps input
//! deterministic across a hot reload.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Rect, Vec2};
use unbound_vector::{hit_quad, QuadSegment};

use crate::cursor::CursorKind;

/// First overlay layer. Anything at or above it wins cursor priority over the
/// UI layer underneath (`docs` §6.2).
pub const PORTAL_LAYER: u8 = 2;

/// A rounded-rectangle clip, the CPU mirror of `GpuClipNode`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipRect {
    pub rect: Rect,
    /// Corner radii in `TL, TR, BR, BL` order.
    pub radii: [f32; 4],
}

impl ClipRect {
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            radii: [0.0; 4],
        }
    }

    #[must_use]
    pub fn rounded(rect: Rect, radius: f32) -> Self {
        Self {
            rect,
            radii: [radius; 4],
        }
    }

    /// True when `point` survives this clip.
    #[must_use]
    pub fn contains(&self, point: Vec2) -> bool {
        if !self.rect.contains_point(point) {
            return false;
        }
        if self.radii.iter().all(|r| *r <= 0.0) {
            return true;
        }
        rounded_rect_sdf(self.rect, self.radii, point) <= 0.0
    }
}

/// Narrow-phase geometry of a hit region.
///
/// Every variant is evaluated in the **same space as the region's bounds**
/// (the parent's coordinate space), so the vertices of a polygon, the control
/// points of a Bézier and the point being tested are all directly
/// comparable.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Plain rectangle.
    Rect,
    /// Rectangle with four per-corner radii (`TL, TR, BR, BL`).
    Rounded { radii: [f32; 4] },
    /// Closed polygon, even-odd fill.
    ///
    /// The vertices are in the same space as the region's `bounds`.
    Polygon(Vec<Vec2>),
    /// A quadratic Bézier stroke: hit when the distance is within the width.
    ///
    /// The control points are in the same space as the region's `bounds`.
    Bezier { quad: QuadSegment, half_width: f32 },
}

impl Shape {
    #[must_use]
    pub fn contains(&self, bounds: Rect, point: Vec2) -> bool {
        match self {
            Self::Rect => bounds.contains_point(point),
            Self::Rounded { radii } => rounded_rect_sdf(bounds, *radii, point) <= 0.0,
            Self::Polygon(points) => point_in_polygon(points, point),
            Self::Bezier { quad, half_width } => {
                bounds.inflate(*half_width).contains_point(point)
                    && hit_quad(quad, point, *half_width)
            }
        }
    }
}

/// One pickable surface.
#[derive(Clone, Debug, PartialEq)]
pub struct HitRegion {
    /// Node identity. Must be unique: it is the final ordering tie-break.
    pub node: u64,
    /// Overlay layer: `0` canvas, `1` UI, `2..=254` portals, `255` debug.
    pub layer: u8,
    /// Topological order inside the layer; larger is on top.
    pub z: u32,
    pub bounds: Rect,
    pub shape: Shape,
    /// Clip inherited from ancestors, already intersected by the caller.
    pub clip: Option<ClipRect>,
    /// When false the region is painted but transparent to input (`pointer-events:
    /// none`), which is what lets a label inside a button be click-through.
    pub receive_events: bool,
    /// Cursor this region asks for while hovered, if any.
    pub cursor: Option<CursorKind>,
}

impl HitRegion {
    /// Rectangular, event-receiving region.
    #[must_use]
    pub fn new(node: u64, layer: u8, z: u32, bounds: Rect) -> Self {
        Self {
            node,
            layer,
            z,
            bounds,
            shape: Shape::Rect,
            clip: None,
            receive_events: true,
            cursor: None,
        }
    }

    /// Ask for a specific cursor while this region is hovered.
    #[must_use]
    pub fn with_cursor(mut self, cursor: CursorKind) -> Self {
        self.cursor = Some(cursor);
        self
    }

    #[must_use]
    pub fn with_shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }

    #[must_use]
    pub fn with_clip(mut self, clip: ClipRect) -> Self {
        self.clip = Some(clip);
        self
    }

    #[must_use]
    pub fn pass_through(mut self) -> Self {
        self.receive_events = false;
        self
    }

    /// Broad + narrow phase for one point.
    #[must_use]
    pub fn hit(&self, point: Vec2) -> bool {
        if !self.receive_events {
            return false;
        }
        if !self.bounds.contains_point(point) {
            // The Bézier case inflates by half the stroke width inside `Shape`;
            // everything else can reject here.
            if !matches!(self.shape, Shape::Bezier { .. }) {
                return false;
            }
        }
        if let Some(clip) = &self.clip {
            if !clip.contains(point) {
                return false;
            }
        }
        self.shape.contains(self.bounds, point)
    }

    /// Sort key: higher wins. Node id makes the order total.
    fn rank(&self) -> (u8, u32, u64) {
        (self.layer, self.z, self.node)
    }
}

/// Signed distance to a rounded rectangle; negative inside.
///
/// The per-corner radius is picked from the quadrant of the sample, which is the
/// same approximation the uber-shader uses for its two-slot corner mask: exact
/// for uniform radii, and conservative (never larger than the true shape) for
/// radii that differ between corners.
#[must_use]
pub fn rounded_rect_sdf(bounds: Rect, radii: [f32; 4], point: Vec2) -> f32 {
    let half = Vec2::new(bounds.size.x * 0.5, bounds.size.y * 0.5);
    if half.x < 0.0 || half.y < 0.0 || !bounds.is_finite() {
        return f32::INFINITY;
    }
    let local = point - bounds.center();
    let radius = if local.x >= 0.0 {
        if local.y >= 0.0 {
            radii[2]
        } else {
            radii[1]
        }
    } else if local.y >= 0.0 {
        radii[3]
    } else {
        radii[0]
    };
    let radius = radius.max(0.0).min(half.x.min(half.y));
    let q = Vec2::new(local.x.abs(), local.y.abs()) - (half - Vec2::splat(radius));
    let outside = Vec2::new(q.x.max(0.0), q.y.max(0.0)).length();
    let inside = q.x.max(q.y).min(0.0);
    outside + inside - radius
}

/// Even-odd point-in-polygon.
#[must_use]
pub fn point_in_polygon(points: &[Vec2], point: Vec2) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[j];
        // Cast a ray to +X; count crossings. The half-open comparison keeps
        // vertices from being counted twice.
        if (a.y > point.y) != (b.y > point.y) {
            let t = (point.y - a.y) / (b.y - a.y);
            if point.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Topmost region under `point`, or `None`.
#[must_use]
pub fn pick(regions: &[HitRegion], point: Vec2) -> Option<&HitRegion> {
    regions
        .iter()
        .filter(|region| region.hit(point))
        .max_by_key(|region| region.rank())
}

/// Index of the topmost region under `point`.
#[must_use]
pub fn pick_index(regions: &[HitRegion], point: Vec2) -> Option<usize> {
    regions
        .iter()
        .enumerate()
        .filter(|(_, region)| region.hit(point))
        .max_by_key(|(_, region)| region.rank())
        .map(|(index, _)| index)
}

/// Every region under `point`, topmost first.
///
/// This is the "pick path" the cursor stack consumes: the top entry decides the
/// click target, the rest decides what the pointer *looks like* when the top
/// entry is a pass-through or a resize handle.
#[must_use]
pub fn pick_stack(regions: &[HitRegion], point: Vec2) -> Vec<&HitRegion> {
    let mut hits: Vec<&HitRegion> = regions.iter().filter(|r| r.hit(point)).collect();
    hits.sort_by_key(|r| core::cmp::Reverse(r.rank()));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_plain_rect_hits_inside_only() {
        let region = HitRegion::new(1, 1, 0, rect(0.0, 0.0, 100.0, 50.0));
        assert!(region.hit(Vec2::new(50.0, 25.0)));
        assert!(region.hit(Vec2::new(0.0, 0.0)));
        assert!(!region.hit(Vec2::new(-1.0, 25.0)));
        assert!(!region.hit(Vec2::new(50.0, 51.0)));
    }

    #[test]
    fn rounded_corners_reject_the_corner_pixel() {
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let region =
            HitRegion::new(1, 1, 0, bounds).with_shape(Shape::Rounded { radii: [20.0; 4] });
        assert!(region.hit(Vec2::new(50.0, 50.0)));
        assert!(!region.hit(Vec2::new(1.0, 1.0)), "corner must be cut away");
        assert!(region.hit(Vec2::new(20.0, 20.0)), "inside the arc");
        assert!(rounded_rect_sdf(bounds, [20.0; 4], Vec2::new(50.0, 50.0)) < 0.0);
        assert!(rounded_rect_sdf(bounds, [20.0; 4], Vec2::new(0.0, 0.0)) > 0.0);
    }

    #[test]
    fn an_asymmetric_radius_cuts_only_its_own_corner() {
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        // Only the top-left corner is rounded.
        let radii = [30.0, 0.0, 0.0, 0.0];
        assert!(rounded_rect_sdf(bounds, radii, Vec2::new(2.0, 2.0)) > 0.0);
        assert!(rounded_rect_sdf(bounds, radii, Vec2::new(98.0, 2.0)) < 0.0);
    }

    #[test]
    fn a_clip_rejects_points_outside_it() {
        let region = HitRegion::new(1, 1, 0, rect(0.0, 0.0, 100.0, 100.0))
            .with_clip(ClipRect::new(rect(0.0, 0.0, 50.0, 100.0)));
        assert!(region.hit(Vec2::new(25.0, 50.0)));
        assert!(!region.hit(Vec2::new(75.0, 50.0)));
    }

    #[test]
    fn a_rounded_clip_respects_its_arc() {
        let clip = ClipRect::rounded(rect(0.0, 0.0, 100.0, 100.0), 40.0);
        assert!(clip.contains(Vec2::new(50.0, 50.0)));
        assert!(!clip.contains(Vec2::new(3.0, 3.0)));
        assert!(!clip.contains(Vec2::new(-5.0, 50.0)));
    }

    #[test]
    fn higher_layers_win_regardless_of_z() {
        let regions = vec![
            HitRegion::new(1, 1, 999, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 2, 0, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(pick(&regions, Vec2::new(50.0, 50.0)).unwrap().node, 2);
    }

    #[test]
    fn z_orders_within_a_layer() {
        let regions = vec![
            HitRegion::new(1, 1, 1, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 1, 2, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(pick(&regions, Vec2::new(50.0, 50.0)).unwrap().node, 2);
    }

    #[test]
    fn pick_is_independent_of_the_input_order() {
        let a = HitRegion::new(7, 1, 5, rect(0.0, 0.0, 10.0, 10.0));
        let b = HitRegion::new(9, 1, 5, rect(0.0, 0.0, 10.0, 10.0));
        let forward = vec![a.clone(), b.clone()];
        let backward = vec![b, a];
        assert_eq!(
            pick(&forward, Vec2::new(1.0, 1.0)).unwrap().node,
            pick(&backward, Vec2::new(1.0, 1.0)).unwrap().node
        );
    }

    #[test]
    fn pass_through_regions_are_skipped() {
        let regions = vec![
            HitRegion::new(1, 1, 9, rect(0.0, 0.0, 100.0, 100.0)).pass_through(),
            HitRegion::new(2, 1, 1, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(pick(&regions, Vec2::new(50.0, 50.0)).unwrap().node, 2);
    }

    #[test]
    fn polygons_use_even_odd_fill() {
        let tri = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(0.0, 100.0),
        ];
        assert!(point_in_polygon(&tri, Vec2::new(10.0, 10.0)));
        assert!(!point_in_polygon(&tri, Vec2::new(90.0, 90.0)));
        assert!(!point_in_polygon(&tri, Vec2::new(-1.0, 10.0)));
    }

    #[test]
    fn a_bezier_stroke_hits_near_the_curve_only() {
        let quad = QuadSegment {
            p0: Vec2::new(0.0, 0.0),
            p1: Vec2::new(50.0, 100.0),
            p2: Vec2::new(100.0, 0.0),
        };
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let region = HitRegion::new(1, 0, 0, bounds).with_shape(Shape::Bezier {
            quad,
            half_width: 4.0,
        });
        // The curve's midpoint is (50, 50).
        assert!(region.hit(Vec2::new(50.0, 50.0)));
        assert!(!region.hit(Vec2::new(10.0, 90.0)));
    }

    #[test]
    fn the_pick_stack_is_ordered_topmost_first() {
        let regions = vec![
            HitRegion::new(1, 1, 1, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 1, 2, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(3, 3, 0, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        let stack = pick_stack(&regions, Vec2::new(50.0, 50.0));
        assert_eq!(
            stack.iter().map(|r| r.node).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }

    #[test]
    fn a_degenerate_rect_is_never_hit() {
        let region = HitRegion::new(1, 1, 0, rect(0.0, 0.0, f32::NAN, 10.0));
        assert!(!region.hit(Vec2::new(0.0, 0.0)));
        assert_eq!(
            rounded_rect_sdf(rect(0.0, 0.0, f32::NAN, 1.0), [0.0; 4], Vec2::ZERO),
            f32::INFINITY
        );
    }
}

//! Axis-aligned rectangle: layout boxes, clip boxes, quadtree bounds, dirty rects.

use bytemuck::{Pod, Zeroable};

use crate::vec2::Vec2;

/// An axis-aligned rectangle stored as origin + size.
///
/// Invariant used throughout the engine: `size >= 0` for layout rects; empty
/// intersection is signalled by `size <= 0` on one axis, which the vertex
/// shader turns into a fully collapsed quad (see `docs` §3.2).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Rect {
    pub origin: Vec2,
    pub size: Vec2,
}

impl Rect {
    pub const ZERO: Self = Self {
        origin: Vec2::ZERO,
        size: Vec2::ZERO,
    };

    #[inline]
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            origin: Vec2::new(x, y),
            size: Vec2::new(w, h),
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self {
            origin: min,
            size: Vec2::new(max.x - min.x, max.y - min.y),
        }
    }

    #[inline]
    #[must_use]
    pub fn from_points(a: Vec2, b: Vec2) -> Self {
        Self::from_min_max(a.min(b), a.max(b))
    }

    #[inline]
    #[must_use]
    pub fn min(&self) -> Vec2 {
        self.origin
    }

    #[inline]
    #[must_use]
    pub fn max(&self) -> Vec2 {
        self.origin + self.size
    }

    #[inline]
    #[must_use]
    pub fn center(&self) -> Vec2 {
        self.origin + self.size * 0.5
    }

    #[inline]
    #[must_use]
    pub fn width(&self) -> f32 {
        self.size.x
    }

    #[inline]
    #[must_use]
    pub fn height(&self) -> f32 {
        self.size.y
    }

    #[inline]
    #[must_use]
    pub fn area(&self) -> f32 {
        self.size.x.max(0.0) * self.size.y.max(0.0)
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.x <= 0.0 || self.size.y <= 0.0
    }

    #[inline]
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.origin.is_finite() && self.size.is_finite()
    }

    #[inline]
    #[must_use]
    pub fn contains_point(&self, p: Vec2) -> bool {
        p.x >= self.origin.x
            && p.y >= self.origin.y
            && p.x <= self.origin.x + self.size.x
            && p.y <= self.origin.y + self.size.y
    }

    /// True when `other` lies entirely inside `self`.
    ///
    /// The inclusive edge test matches [`Self::contains_point`], which counts
    /// a point on the edge as contained.
    #[inline]
    #[must_use]
    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.min().x >= self.min().x
            && other.min().y >= self.min().y
            && other.max().x <= self.max().x
            && other.max().y <= self.max().y
    }

    #[inline]
    #[must_use]
    pub fn intersects(&self, other: &Rect) -> bool {
        let a_max = self.max();
        let b_max = other.max();
        self.origin.x < b_max.x
            && other.origin.x < a_max.x
            && self.origin.y < b_max.y
            && other.origin.y < a_max.y
    }

    /// True when the two rectangles overlap *or touch*.
    ///
    /// [`Self::intersects`] is strict: two boxes that share only an edge do
    /// not intersect. That is the right convention for a conservative
    /// broad-phase filter, but it disagrees with
    /// [`Self::contains_point`], which counts a point on the edge as
    /// contained. An exact selection test wants the inclusive rule: a marquee
    /// that ends exactly on an entity's edge should select it.
    #[inline]
    #[must_use]
    pub fn touches(&self, other: &Rect) -> bool {
        let a_max = self.max();
        let b_max = other.max();
        self.origin.x <= b_max.x
            && other.origin.x <= a_max.x
            && self.origin.y <= b_max.y
            && other.origin.y <= a_max.y
    }

    /// Intersection; returns an empty rect (zero or negative size) when the
    /// boxes are disjoint. Never returns `NaN` for finite inputs.
    #[inline]
    #[must_use]
    pub fn intersect(&self, other: &Rect) -> Rect {
        let min = self.min().max(other.min());
        let max = self.max().min(other.max());
        if max.x <= min.x || max.y <= min.y {
            Rect {
                origin: min,
                size: Vec2::ZERO,
            }
        } else {
            Rect::from_min_max(min, max)
        }
    }

    #[inline]
    #[must_use]
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::from_min_max(self.min().min(other.min()), self.max().max(other.max()))
    }

    #[inline]
    #[must_use]
    pub fn inflate(&self, by: f32) -> Rect {
        Rect::new(
            self.origin.x - by,
            self.origin.y - by,
            self.size.x + by * 2.0,
            self.size.y + by * 2.0,
        )
    }

    #[inline]
    #[must_use]
    pub fn inset(&self, by: f32) -> Rect {
        self.inflate(-by)
    }

    #[inline]
    #[must_use]
    pub fn translate(&self, by: Vec2) -> Rect {
        Rect {
            origin: self.origin + by,
            size: self.size,
        }
    }

    /// Scale about the rect origin — used when converting logical -> physical px.
    #[inline]
    #[must_use]
    pub fn scale(&self, s: f32) -> Rect {
        Rect::new(
            self.origin.x * s,
            self.origin.y * s,
            self.size.x * s,
            self.size.y * s,
        )
    }

    /// Split into `(min_x, min_y, max_x, max_y)` as consumed by `GpuClipNode`.
    #[inline]
    #[must_use]
    pub fn to_min_max_array(&self) -> [f32; 4] {
        let max = self.max();
        [self.origin.x, self.origin.y, max.x, max.y]
    }

    /// Rebuild from `(min_x, min_y, max_x, max_y)`.
    #[inline]
    #[must_use]
    pub fn from_min_max_array(a: [f32; 4]) -> Rect {
        Rect {
            origin: Vec2::new(a[0], a[1]),
            size: Vec2::new(a[2] - a[0], a[3] - a[1]),
        }
    }

    /// Centre + half-extent form used by the rounded-clip SDF slots.
    #[inline]
    #[must_use]
    pub fn to_center_half(&self) -> [f32; 4] {
        let c = self.center();
        [c.x, c.y, self.size.x * 0.5, self.size.y * 0.5]
    }
}

/// Project an **outer** rounded clip's corner arcs onto an **inner** rectangle.
///
/// The shader only carries two rounded clip slots, so a third nested container
/// has to be folded on the CPU. Intersecting the outer rounded rect with the
/// inner one turns each outer corner arc into a smaller effective radius at the
/// corresponding inner corner — the same "inner radius = outer radius − inset"
/// rule CSS uses for nested rounded containers. The result is only valid when
/// the two rects are axis-aligned, which is the case the fold path handles.
///
/// `radii` are ordered `[top-left, top-right, bottom-right, bottom-left]`,
/// matching the primitive corner-radius order and `sd_rounded_box`.
#[must_use]
pub fn project_rounded_radii(
    inner: Rect,
    inner_radii: [f32; 4],
    outer: Rect,
    outer_radii: [f32; 4],
) -> [f32; 4] {
    let mut out = inner_radii;
    let inner_min = inner.min();
    let outer_min = outer.min();
    let outer_max = outer.max();
    for (corner, radius) in outer_radii.iter().enumerate() {
        let r = *radius;
        if r <= 0.0 {
            continue;
        }
        // Distance from the inner corner to the outer corner, per axis.
        let (dx, dy) = match corner {
            0 => (inner_min.x - outer_min.x, inner_min.y - outer_min.y),
            1 => (outer_max.x - inner.max().x, inner_min.y - outer_min.y),
            2 => (outer_max.x - inner.max().x, outer_max.y - inner.max().y),
            _ => (inner_min.x - outer_min.x, outer_max.y - inner.max().y),
        };
        // The inner corner needs at least the outer radius minus the inset it
        // sits at; the larger of the two axes is the conservative choice for
        // non-uniform insets.
        let projected = (r - dx.max(dy)).max(0.0);
        out[corner] = out[corner].max(projected);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_of_overlapping_rects() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
    }

    #[test]
    fn disjoint_intersection_is_empty_not_nan() {
        let a = Rect::new(0.0, 0.0, 4.0, 4.0);
        let b = Rect::new(10.0, 10.0, 1.0, 1.0);
        let i = a.intersect(&b);
        assert!(i.is_empty());
        assert!(i.is_finite());
        assert_eq!(i.size, Vec2::ZERO);
    }

    #[test]
    fn union_expands_to_cover_both() {
        let a = Rect::new(0.0, 0.0, 4.0, 4.0);
        let b = Rect::new(-2.0, 6.0, 2.0, 2.0);
        let u = a.union(&b);
        assert!(u.contains_rect(&a));
        assert!(u.contains_rect(&b));
        assert_eq!(u, Rect::new(-2.0, 0.0, 6.0, 8.0));
    }

    #[test]
    fn center_half_round_trip() {
        let r = Rect::new(2.0, 4.0, 8.0, 6.0);
        let ch = r.to_center_half();
        assert_eq!(ch, [6.0, 7.0, 4.0, 3.0]);
    }

    #[test]
    fn min_max_array_round_trip() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(Rect::from_min_max_array(r.to_min_max_array()), r);
    }

    /// `intersects` is strict, `touches` is inclusive.
    ///
    /// The two boxes share only their edge: `intersects` says no (the
    /// convention for a conservative broad-phase filter), `touches` says yes
    /// (the convention for an exact selection test, matching
    /// `contains_point`).
    #[test]
    fn touching_rects_are_distinguished_from_intersecting_ones() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let touching = Rect::new(10.0, 0.0, 10.0, 10.0);
        let overlapping = Rect::new(5.0, 0.0, 10.0, 10.0);
        let apart = Rect::new(11.0, 0.0, 10.0, 10.0);

        assert!(!a.intersects(&touching));
        assert!(a.touches(&touching));
        assert!(a.intersects(&overlapping));
        assert!(a.touches(&overlapping));
        assert!(!a.touches(&apart));
    }

    #[test]
    fn a_nested_corner_radius_is_projected_onto_the_inner_rect() {
        let outer = Rect::new(0.0, 0.0, 100.0, 100.0);
        let inner = Rect::new(10.0, 10.0, 80.0, 80.0);
        let projected = project_rounded_radii(inner, [0.0; 4], outer, [20.0; 4]);
        // Each inner corner sits 10px inside the outer one, so the outer 20px
        // arc projects to 20 - 10 = 10px at the inner corner.
        for r in projected {
            assert!((r - 10.0).abs() < 1e-3, "{projected:?}");
        }

        // Containers that share a corner keep the full radius.
        let same = project_rounded_radii(outer, [0.0; 4], outer, [20.0; 4]);
        assert!(same.iter().all(|r| (r - 20.0).abs() < 1e-4));

        // An inner rect far from an arc needs no radius at all.
        let far = project_rounded_radii(inner, [4.0; 4], outer, [8.0; 4]);
        assert_eq!(far, [4.0; 4]);
    }
}

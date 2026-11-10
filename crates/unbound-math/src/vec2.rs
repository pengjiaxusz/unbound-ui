//! 2D vector / point type shared by layout, canvas and GPU packing.

use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use bytemuck::{Pod, Zeroable};

/// A 2D point or vector in `f32`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };
    pub const X: Self = Self { x: 1.0, y: 0.0 };
    pub const Y: Self = Self { x: 0.0, y: 1.0 };

    #[inline]
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    #[must_use]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    #[inline]
    #[must_use]
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y
    }

    /// 2D cross product (z component of the 3D cross product).
    #[inline]
    #[must_use]
    pub fn cross(self, rhs: Self) -> f32 {
        self.x * rhs.y - self.y * rhs.x
    }

    #[inline]
    #[must_use]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    #[must_use]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    #[inline]
    #[must_use]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > f32::EPSILON {
            self / len
        } else {
            Self::ZERO
        }
    }

    #[inline]
    #[must_use]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Rotate by `cos`/`sin` without allocating a matrix.
    #[inline]
    #[must_use]
    pub fn rotate(self, cos: f32, sin: f32) -> Self {
        Self::new(self.x * cos - self.y * sin, self.x * sin + self.y * cos)
    }

    #[inline]
    #[must_use]
    pub fn min(self, rhs: Self) -> Self {
        Self::new(self.x.min(rhs.x), self.y.min(rhs.y))
    }

    #[inline]
    #[must_use]
    pub fn max(self, rhs: Self) -> Self {
        Self::new(self.x.max(rhs.x), self.y.max(rhs.y))
    }

    #[inline]
    #[must_use]
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs())
    }

    #[inline]
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    #[inline]
    #[must_use]
    pub fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }

    #[inline]
    #[must_use]
    pub const fn from_array(a: [f32; 2]) -> Self {
        Self { x: a[0], y: a[1] }
    }

    #[inline]
    pub fn lerp(self, rhs: Self, t: f32) -> Self {
        self + (rhs - self) * t
    }
}

impl Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Mul<Vec2> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Vec2) -> Self {
        Self::new(self.x * rhs.x, self.y * rhs.y)
    }
}

impl Div<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f32) -> Self {
        Self::new(self.x / rhs, self.y / rhs)
    }
}

impl Neg for Vec2 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl From<[f32; 2]> for Vec2 {
    #[inline]
    fn from(v: [f32; 2]) -> Self {
        Self::from_array(v)
    }
}

impl From<Vec2> for [f32; 2] {
    #[inline]
    fn from(v: Vec2) -> Self {
        v.to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_normalise() {
        let v = Vec2::new(3.0, 4.0);
        assert_eq!(v.length(), 5.0);
        assert!((v.normalized().length() - 1.0).abs() < 1e-6);
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
    }

    #[test]
    fn perp_and_cross_are_orthogonal() {
        let a = Vec2::new(2.0, 1.0);
        assert_eq!(a.dot(a.perp()), 0.0);
        assert_eq!(a.cross(a.perp()), a.length_squared());
    }

    #[test]
    fn rotation_preserves_length() {
        let v = Vec2::new(7.0, -3.0);
        let r = v.rotate(0.3f32.cos(), 0.3f32.sin());
        assert!((r.length() - v.length()).abs() < 1e-4);
    }

    #[test]
    fn layout_of_vec2_is_two_floats() {
        assert_eq!(core::mem::size_of::<Vec2>(), 8);
    }
}

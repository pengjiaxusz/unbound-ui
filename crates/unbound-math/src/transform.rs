//! 独立动画通道使用的 2D 仿射变换。
//!
//! 根据 `docs` §4.4，该变换从不进入 Taffy：它存储在 `transforms_bda` 中，
//! 并在布局**之后**应用。矩阵按顶点着色器期望的相同 2×3 布局存储。
//!

use bytemuck::{Pod, Zeroable};

use crate::rect::Rect;
use crate::vec2::Vec2;

/// 列主序 2×3 仿射矩阵：
///
/// ```text
/// | m00 m01 tx |     x' = m00*x + m01*y + tx
/// | m10 m11 ty |     y' = m10*x + m11*y + ty
/// ```
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct Affine2 {
    pub m00: f32,
    pub m10: f32,
    pub m01: f32,
    pub m11: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Affine2 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine2 {
    pub const IDENTITY: Self = Self {
        m00: 1.0,
        m10: 0.0,
        m01: 0.0,
        m11: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    #[inline]
    #[must_use]
    pub const fn translation(t: Vec2) -> Self {
        Self {
            m00: 1.0,
            m10: 0.0,
            m01: 0.0,
            m11: 1.0,
            tx: t.x,
            ty: t.y,
        }
    }

    #[inline]
    #[must_use]
    pub fn scale(s: Vec2) -> Self {
        Self {
            m00: s.x,
            m10: 0.0,
            m01: 0.0,
            m11: s.y,
            tx: 0.0,
            ty: 0.0,
        }
    }

    #[inline]
    #[must_use]
    pub fn rotation(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self {
            m00: cos,
            m10: sin,
            m01: -sin,
            m11: cos,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// 绕枢轴缩放，再绕同一枢轴旋转，最后平移。
    /// 这正是动画通道为原地缩放/旋转卡片生成的复合变换。
    ///
    #[must_use]
    pub fn around(pivot: Vec2, scale: Vec2, radians: f32, translate: Vec2) -> Self {
        let r = Self::rotation(radians);
        let s = Self::scale(scale);
        // T(pivot) * R * S * T(-pivot) * T(translate)
        let rs = r.then(&s);
        let mut m = rs;
        m.tx = pivot.x - (rs.m00 * pivot.x + rs.m01 * pivot.y);
        m.ty = pivot.y - (rs.m10 * pivot.x + rs.m11 * pivot.y);
        m.tx += translate.x;
        m.ty += translate.y;
        m
    }

    /// `self * rhs`——先应用 `rhs`，再应用 `self`。
    #[must_use]
    pub fn then(&self, rhs: &Self) -> Self {
        Self {
            m00: self.m00 * rhs.m00 + self.m01 * rhs.m10,
            m10: self.m10 * rhs.m00 + self.m11 * rhs.m10,
            m01: self.m00 * rhs.m01 + self.m01 * rhs.m11,
            m11: self.m10 * rhs.m01 + self.m11 * rhs.m11,
            tx: self.m00 * rhs.tx + self.m01 * rhs.ty + self.tx,
            ty: self.m10 * rhs.tx + self.m11 * rhs.ty + self.ty,
        }
    }

    #[inline]
    #[must_use]
    pub fn determinant(&self) -> f32 {
        self.m00 * self.m11 - self.m01 * self.m10
    }

    /// 逆变换。奇异矩阵返回 `None`，以便调用方回退到未变换路径，
    /// 而非产生 `NaN`。
    #[must_use]
    pub fn inverse(&self) -> Option<Self> {
        let det = self.determinant();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        let m00 = self.m11 * inv_det;
        let m11 = self.m00 * inv_det;
        let m01 = -self.m01 * inv_det;
        let m10 = -self.m10 * inv_det;
        Some(Self {
            m00,
            m10,
            m01,
            m11,
            tx: -(m00 * self.tx + m01 * self.ty),
            ty: -(m10 * self.tx + m11 * self.ty),
        })
    }

    #[inline]
    #[must_use]
    pub fn apply(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.m00 * p.x + self.m01 * p.y + self.tx,
            self.m10 * p.x + self.m11 * p.y + self.ty,
        )
    }

    #[inline]
    #[must_use]
    pub fn apply_vector(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.m00 * p.x + self.m01 * p.y,
            self.m10 * p.x + self.m11 * p.y,
        )
    }

    /// 近似均匀缩放（两轴缩放的几何平均值）——用于判断亚像素动画
    /// 何时必须绕过像素对齐。
    #[must_use]
    pub fn max_scale(&self) -> f32 {
        let sx = Vec2::new(self.m00, self.m10).length();
        let sy = Vec2::new(self.m01, self.m11).length();
        sx.max(sy)
    }

    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.m00.is_finite()
            && self.m01.is_finite()
            && self.m10.is_finite()
            && self.m11.is_finite()
            && self.tx.is_finite()
            && self.ty.is_finite()
    }

    /// `rect` 变换后的轴对齐包围盒。
    #[must_use]
    pub fn transform_rect_bounds(&self, rect: &Rect) -> Rect {
        let corners = [
            self.apply(rect.min()),
            self.apply(Vec2::new(rect.max().x, rect.min().y)),
            self.apply(rect.max()),
            self.apply(Vec2::new(rect.min().x, rect.max().y)),
        ];
        let mut min = corners[0];
        let mut max = corners[0];
        for c in &corners[1..] {
            min = min.min(*c);
            max = max.max(*c);
        }
        Rect::from_min_max(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Vec2, b: Vec2) {
        assert!((a.x - b.x).abs() < 1e-4, "{a:?} != {b:?}");
        assert!((a.y - b.y).abs() < 1e-4, "{a:?} != {b:?}");
    }

    #[test]
    fn identity_is_neutral() {
        let p = Vec2::new(3.0, -7.0);
        approx(Affine2::IDENTITY.apply(p), p);
        let m = Affine2::rotation(0.4);
        let composed = m.then(&Affine2::IDENTITY);
        approx(composed.apply(p), m.apply(p));
    }

    #[test]
    fn inverse_round_trips() {
        let m = Affine2::around(
            Vec2::new(10.0, 5.0),
            Vec2::new(2.0, 0.5),
            0.7,
            Vec2::new(3.0, -1.0),
        );
        let inv = m.inverse().expect("invertible");
        for p in [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 5.0),
            Vec2::new(-4.0, 22.0),
        ] {
            approx(inv.apply(m.apply(p)), p);
        }
    }

    #[test]
    fn singular_has_no_inverse() {
        let m = Affine2::scale(Vec2::new(0.0, 1.0));
        assert!(m.inverse().is_none());
    }

    #[test]
    fn rotation_bounds_are_tight_enough() {
        let m = Affine2::rotation(std::f32::consts::FRAC_PI_4);
        let bounds = m.transform_rect_bounds(&Rect::new(0.0, 0.0, 10.0, 10.0));
        let diag = 10.0 * std::f32::consts::SQRT_2;
        assert!((bounds.width() - diag).abs() < 1e-3);
        assert!((bounds.height() - diag).abs() < 1e-3);
    }

    #[test]
    fn max_scale_tracks_uniform_zoom() {
        assert!((Affine2::scale(Vec2::new(3.0, 3.0)).max_scale() - 3.0).abs() < 1e-6);
        assert!(Affine2::IDENTITY.is_identity());
        assert!(Affine2::IDENTITY.is_finite());
    }
}

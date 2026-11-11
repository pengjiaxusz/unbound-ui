//! Linear-light colour, sRGB transfer functions, Oklab and RGBA16F LUT packing.
//!
//! Per `docs` §2.2 every colour token lives in **linear space** so that
//! translucent stacking, gradient interpolation and antialiased edges blend
//! physically correctly and never produce dark fringes. Oklab is provided for
//! *authoring* (perceptually uniform lightness ramps) — it is converted back to
//! linear before it ever reaches the GPU.

use bytemuck::{Pod, Zeroable};
use core::ops::{Add, Mul, Sub};

use crate::f16::f32_to_f16;

/// Linear-light RGBA, components nominally in `[0, 1]` but not clamped so that
/// HDR wide-gamut swapchains can carry values > 1.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

impl Color {
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);

    #[inline]
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    #[must_use]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    /// Build from 8-bit **sRGB** components (what designers type).
    #[inline]
    #[must_use]
    pub fn from_srgb8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            srgb_to_linear(r as f32 / 255.0),
            srgb_to_linear(g as f32 / 255.0),
            srgb_to_linear(b as f32 / 255.0),
            a as f32 / 255.0,
        )
    }

    /// Build from a `#RRGGBB` / `#RRGGBBAA` hex literal (sRGB).
    #[must_use]
    pub fn from_hex(hex: u32) -> Self {
        Self::from_srgb8(
            ((hex >> 16) & 0xff) as u8,
            ((hex >> 8) & 0xff) as u8,
            (hex & 0xff) as u8,
            255,
        )
    }

    #[inline]
    #[must_use]
    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    #[inline]
    #[must_use]
    pub fn scale_alpha(self, k: f32) -> Self {
        Self {
            a: self.a * k,
            ..self
        }
    }

    #[inline]
    #[must_use]
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Premultiplied form for straight-alpha blending pipelines.
    #[inline]
    #[must_use]
    pub fn premultiplied(self) -> Self {
        Self::new(self.r * self.a, self.g * self.a, self.b * self.a, self.a)
    }

    /// Oklab representation (perceptually uniform).
    #[must_use]
    pub fn to_oklab(self) -> Oklab {
        Oklab::from_linear(self)
    }

    /// Perceptual lightness in `[0, 1]` (Oklab `L`).
    #[must_use]
    pub fn luminance_perceptual(self) -> f32 {
        self.to_oklab().l
    }

    /// Relative luminance (WCAG), linear space.
    #[must_use]
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// Pack into four IEEE binary16 words — the `RGBA16F` theme LUT element
    /// layout (`theme_lut_bda`).
    #[inline]
    #[must_use]
    pub fn to_rgba16f(self) -> [u16; 4] {
        [
            f32_to_f16(self.r),
            f32_to_f16(self.g),
            f32_to_f16(self.b),
            f32_to_f16(self.a),
        ]
    }

    /// Linear interpolation in linear space (never in sRGB — that is what
    /// causes muddy gradients).
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            crate::lerp(self.r, other.r, t),
            crate::lerp(self.g, other.g, t),
            crate::lerp(self.b, other.b, t),
            crate::lerp(self.a, other.a, t),
        )
    }

    /// Oklab interpolation — perceptually even ramps for gradients and theme
    /// transitions.
    #[must_use]
    pub fn lerp_oklab(self, other: Self, t: f32) -> Self {
        self.to_oklab().lerp(other.to_oklab(), t).to_linear()
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }
}

impl Add for Color {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.r + rhs.r, self.g + rhs.g, self.b + rhs.b, self.a + rhs.a)
    }
}

impl Mul<f32> for Color {
    type Output = Self;
    #[inline]
    fn mul(self, k: f32) -> Self {
        Self::new(self.r * k, self.g * k, self.b * k, self.a * k)
    }
}

impl Sub for Color {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.r - rhs.r, self.g - rhs.g, self.b - rhs.b, self.a - rhs.a)
    }
}

/// 8-bit sRGB colour, the authoring/storage format for theme files.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Srgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Srgba {
    #[inline]
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    #[must_use]
    pub fn to_linear(self) -> Color {
        Color::from_srgb8(self.r, self.g, self.b, self.a)
    }
}

/// sRGB electro-optical transfer function (EOTF).
#[inline]
#[must_use]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse sRGB transfer function (OETF).
#[inline]
#[must_use]
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Perceptually uniform Oklab colour.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
    pub alpha: f32,
}

impl Oklab {
    /// Convert linear sRGB to Oklab (Björn Ottosson's matrices).
    #[must_use]
    pub fn from_linear(c: Color) -> Self {
        let l = 0.412_221_47 * c.r + 0.536_332_55 * c.g + 0.051_445_99 * c.b;
        let m = 0.211_903_5 * c.r + 0.680_699_5 * c.g + 0.107_396_96 * c.b;
        let s = 0.088_302_46 * c.r + 0.281_718_85 * c.g + 0.629_978_7 * c.b;

        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        Self {
            l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
            alpha: c.a,
        }
    }

    /// Convert back to linear sRGB.
    #[must_use]
    pub fn to_linear(self) -> Color {
        let l_ = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m_ = self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b;
        let s_ = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        Color::new(
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
            self.alpha,
        )
    }

    /// Interpolate in Oklab (used by gradient ramps and theme fades).
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            l: crate::lerp(self.l, other.l, t),
            a: crate::lerp(self.a, other.a, t),
            b: crate::lerp(self.b, other.b, t),
            alpha: crate::lerp(self.alpha, other.alpha, t),
        }
    }

    /// Perceptual lightness delta — theme logic uses this to decide whether a
    /// surface is "dark" and therefore needs stem darkening compensation.
    #[must_use]
    pub fn is_dark(self) -> bool {
        self.l < 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_transfer_round_trips() {
        for i in 0..=255u32 {
            let s = i as f32 / 255.0;
            let l = srgb_to_linear(s);
            assert!((linear_to_srgb(l) - s).abs() < 1e-5);
        }
    }

    #[test]
    fn mid_gray_is_linear_0_214() {
        assert!((srgb_to_linear(0.5) - 0.214_041_14).abs() < 1e-4);
    }

    #[test]
    fn hex_and_srgb8_agree() {
        assert_eq!(Color::from_hex(0xFF8800), Color::from_srgb8(255, 136, 0, 255));
        assert!((Color::from_srgb8(255, 255, 255, 255) - Color::WHITE).luminance() < 1e-6);
    }

    #[test]
    fn oklab_round_trips_within_1e_4() {
        for c in [
            Color::from_srgb8(12, 34, 56, 255),
            Color::from_srgb8(250, 200, 100, 128),
            Color::BLACK,
            Color::WHITE,
            Color::new(0.5, 0.1, 0.9, 1.0),
        ] {
            let back = c.to_oklab().to_linear();
            assert!((back.r - c.r).abs() < 1e-4, "{c:?} -> {back:?}");
            assert!((back.g - c.g).abs() < 1e-4);
            assert!((back.b - c.b).abs() < 1e-4);
            assert!((back.a - c.a).abs() < 1e-6);
        }
    }

    #[test]
    fn oklab_lightness_is_monotonic() {
        let mut prev = -1.0;
        for i in 0..=15u32 {
            let v = (i * 17) as u8;
            let c = Color::from_srgb8(v, v, v, 255);
            let l = c.to_oklab().l;
            assert!(l > prev);
            prev = l;
        }
    }

    #[test]
    fn oklab_lightness_of_srgb_mid_grey_is_about_0_6() {
        // Oklab's L is perceptual, so sRGB 0.5 lands near 0.5987 rather than
        // 0.5 — this is the whole point of using it for ramps.
        let l = Color::from_srgb8(128, 128, 128, 255).to_oklab().l;
        assert!((l - 0.598_7).abs() < 0.01, "L = {l}");
    }

    #[test]
    fn interpolation_never_produces_dark_fringe() {
        // Linear-space lerp of black -> white must be the arithmetic mean.
        let mid = Color::BLACK.lerp(Color::WHITE, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-6);
        // sRGB-space lerp would have produced 0.5 sRGB == 0.214 linear; assert
        // we are clearly brighter than that.
        assert!(mid.r > srgb_to_linear(0.5));
    }

    #[test]
    fn rgba16f_packing_preserves_unit_range() {
        let c = Color::from_srgb8(200, 100, 50, 255);
        let packed = c.to_rgba16f();
        let unpacked = crate::f16::unpack_radii(packed);
        for (u, expected) in unpacked.iter().zip(c.to_array()) {
            assert!((u - expected).abs() < 1e-3);
        }
    }

    #[test]
    fn premultiply_is_componentwise() {
        let p = Color::new(0.8, 0.4, 0.2, 0.5).premultiplied();
        assert_eq!(p.to_array(), [0.4, 0.2, 0.1, 0.5]);
    }
}

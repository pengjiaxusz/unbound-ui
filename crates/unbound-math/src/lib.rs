//! Foundational scalar/vector/colour/transform math for the Unbound engine.
//!
//! Everything in this crate is `Copy`, allocation free and `#[repr(C)]` where it
//! is intended to cross the CPU/GPU boundary. It deliberately avoids generic
//! scalar traits: the renderer is `f32`-only on the hot path (see
//! `docs/gpu_ui_v2_0.md` §4.5 — three orthogonal zoom layers all collapse into
//! one `f32` device-space transform).

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod color;
pub mod f16;
pub mod rect;
pub mod transform;
pub mod vec2;

pub use color::{Color, Oklab, Srgba};
pub use f16::{f16_to_f32, f32_to_f16};
pub use rect::Rect;
pub use transform::Affine2;
pub use vec2::Vec2;

/// Logical size of one canvas chunk in world pixels (`docs` §5.2, 4096² grid).
pub const CHUNK_SIZE: f32 = 4096.0;

/// Reciprocal of [`CHUNK_SIZE`], precomputed to keep shader-side maths identical.
pub const INV_CHUNK_SIZE: f32 = 1.0 / CHUNK_SIZE;

/// Clamp helper shared by the layout and animation layers.
#[inline]
#[must_use]
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Linear interpolation that does not overshoot for `t` outside `0..=1` is *not*
/// wanted here — springs rely on overshoot, so this is a plain lerp.
#[inline]
#[must_use]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Snap a device-space coordinate onto the physical pixel grid.
///
/// Used for static UI nodes (`docs` §4.4): `round(pos * dpi) / dpi` keeps 1px
/// borders razor sharp instead of blurring across two pixel centres.
#[inline]
#[must_use]
pub fn snap_to_pixel(v: f32, dpi_scale: f32) -> f32 {
    if dpi_scale <= 0.0 || !dpi_scale.is_finite() {
        return v;
    }
    (v * dpi_scale).round() / dpi_scale
}

/// Snap a length (width/height) onto whole physical pixels, with a floor of one
/// physical pixel so that snapped geometry never collapses to zero.
#[inline]
#[must_use]
pub fn snap_size_to_pixel(v: f32, dpi_scale: f32) -> f32 {
    if dpi_scale <= 0.0 || !dpi_scale.is_finite() {
        return v;
    }
    let snapped = (v * dpi_scale).round();
    (if snapped < 1.0 { 1.0 } else { snapped }) / dpi_scale
}

/// True when `v` is a usable finite number (used by the layout sanitizers).
#[inline]
#[must_use]
pub fn is_finite(v: f32) -> bool {
    v.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_snap_rounds_to_device_grid() {
        // 1.5x DPI: 10.2 logical -> 15.3 physical -> 15 -> 10.0 logical
        assert_eq!(snap_to_pixel(10.2, 1.5), 10.0);
        assert!((snap_to_pixel(10.34, 1.5) - 10.666_667).abs() < 1e-5);
        assert!((snap_to_pixel(10.4, 1.5) - 10.666_667).abs() < 1e-5);
        // Already aligned values stay untouched.
        assert_eq!(snap_to_pixel(12.0, 1.0), 12.0);
        assert_eq!(snap_to_pixel(12.0, 2.0), 12.0);
    }

    #[test]
    fn pixel_snap_survives_degenerate_dpi() {
        assert_eq!(snap_to_pixel(3.3, 0.0), 3.3);
        assert_eq!(snap_to_pixel(3.3, f32::NAN), 3.3);
        assert_eq!(snap_size_to_pixel(0.1, 0.0), 0.1);
    }

    #[test]
    fn sizes_never_collapse_to_zero() {
        assert_eq!(snap_size_to_pixel(0.1, 1.0), 1.0);
        assert_eq!(snap_size_to_pixel(0.4, 2.0), 0.5);
    }

    #[test]
    fn chunk_constants_are_consistent() {
        assert_eq!(CHUNK_SIZE * INV_CHUNK_SIZE, 1.0);
        assert_eq!(CHUNK_SIZE, 4096.0);
    }
}

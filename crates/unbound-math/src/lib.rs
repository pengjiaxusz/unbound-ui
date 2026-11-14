//! Unbound 引擎的基础标量、向量、色彩与变换数学库。
//!
//! 本 crate 中的所有类型均为 `Copy`、零堆分配，并在跨 CPU/GPU 边界时
//! 标记 `#[repr(C)]`。刻意避免使用泛型标量 trait：渲染器在热路径上
//! 仅使用 `f32`（参见 `docs/architecture/animation-and-theme.md`——
//! 三个正交缩放层最终合并为单一的 `f32` 设备空间变换）。
//!

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod color;
pub mod f16;
pub mod rect;
pub mod transform;
pub mod vec2;

pub use color::{Color, Oklab, Srgba};
pub use f16::{f16_to_f32, f32_to_f16};
pub use rect::{project_rounded_radii, Rect};
pub use transform::Affine2;
pub use vec2::Vec2;

/// 单个画布分块在世界像素中的逻辑尺寸（`docs` §5.2，4096² 网格）。
pub const CHUNK_SIZE: f32 = 4096.0;

/// [`CHUNK_SIZE`] 的倒数，预计算以确保与着色器端数学运算完全一致。
pub const INV_CHUNK_SIZE: f32 = 1.0 / CHUNK_SIZE;

/// 布局层与动画层共用的钳制辅助函数。
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

/// 纯线性插值：当 `t` 超出 `0..=1` 时不限制过冲——弹簧动画依赖过冲，
/// 因此此处保留朴素线性插值。
#[inline]
#[must_use]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 将设备空间坐标对齐到物理像素网格。
///
/// 用于静态 UI 节点（`docs` §4.4）：`round(pos * dpi) / dpi` 可使 1px
/// 边框保持锐利，避免跨越两个像素中心产生模糊。
#[inline]
#[must_use]
pub fn snap_to_pixel(v: f32, dpi_scale: f32) -> f32 {
    if dpi_scale <= 0.0 || !dpi_scale.is_finite() {
        return v;
    }
    (v * dpi_scale).round() / dpi_scale
}

/// 将长度（宽度/高度）对齐到整物理像素，下限为一物理像素，
/// 以防对齐后的几何体退化为零。
#[inline]
#[must_use]
pub fn snap_size_to_pixel(v: f32, dpi_scale: f32) -> f32 {
    if dpi_scale <= 0.0 || !dpi_scale.is_finite() {
        return v;
    }
    let snapped = (v * dpi_scale).round();
    (if snapped < 1.0 { 1.0 } else { snapped }) / dpi_scale
}

/// 当 `v` 为可用的有限数值时返回 `true`（供布局合法性检查使用）。
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
        // 1.5x DPI：10.2 逻辑像素 -> 15.3 物理像素 -> 15 -> 10.0 逻辑像素
        assert_eq!(snap_to_pixel(10.2, 1.5), 10.0);
        assert!((snap_to_pixel(10.34, 1.5) - 10.666_667).abs() < 1e-5);
        assert!((snap_to_pixel(10.4, 1.5) - 10.666_667).abs() < 1e-5);
        // 已对齐的值保持不变。
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

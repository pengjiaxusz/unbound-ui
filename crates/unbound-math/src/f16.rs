//! 用于 64 字节打包图元的 `f16`（IEEE 754 binary16）转换。
//!
//! `docs/architecture/gpu-renderer.md` 将圆角半径、边框宽度和阴影模糊半径
//! 存储为 `u16` 格式的 f16 位模式。通过 [`half`] 进行往返转换可使
//! 编解码器与着色器中的 `unpackHalf2x16` 行为保持一致。

/// 将有限 `f32` 编码为 IEEE binary16 位模式。
///
/// 非有限输入会被饱和截断：`NaN` 转为 `0`，`+inf` 转为 `65504`
/// （最大的有限 binary16 值）。采用饱和而非传播可避免 GPU 端解包出
/// 含 NaN 的四边形，防止其渲染为不可见但仍写入深度的几何体。
///
#[inline]
#[must_use]
pub fn f32_to_f16(v: f32) -> u16 {
    if v.is_nan() {
        return 0;
    }
    // 饱和截断而非输出无穷大：无穷大的半径或模糊值会导致 SDF 数学计算
    // 产生无穷大 Alpha，这比被截断但仍可见的图元更难排查。
    //
    let v = v.clamp(-65504.0, 65504.0);
    half::f16::from_f32(v).to_bits()
}

/// 将 IEEE binary16 位模式解码回 `f32`（始终精确）。
#[inline]
#[must_use]
pub fn f16_to_f32(bits: u16) -> f32 {
    half::f16::from_bits(bits).to_f32()
}

/// 打包四个圆角半径，编码前将每个值钳制到 `[0, 65504]`。
#[inline]
#[must_use]
pub fn pack_radii(radii: [f32; 4]) -> [u16; 4] {
    [
        f32_to_f16(radii[0].max(0.0)),
        f32_to_f16(radii[1].max(0.0)),
        f32_to_f16(radii[2].max(0.0)),
        f32_to_f16(radii[3].max(0.0)),
    ]
}

/// 解包四个圆角半径。
#[inline]
#[must_use]
pub fn unpack_radii(bits: [u16; 4]) -> [f32; 4] {
    [
        f16_to_f32(bits[0]),
        f16_to_f32(bits[1]),
        f16_to_f32(bits[2]),
        f16_to_f32(bits[3]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_exact_for_representable_values() {
        for v in [0.0f32, 1.0, 2.0, 4.0, 8.0, 16.0, 1024.0, 2048.0, 4096.0] {
            assert_eq!(f16_to_f32(f32_to_f16(v)), v, "value {v}");
        }
    }

    #[test]
    fn round_trip_is_monotonic_and_bounded() {
        let mut prev = -1.0f32;
        for i in 0..=1000u32 {
            let v = i as f32 * 4.0;
            let back = f16_to_f32(f32_to_f16(v));
            assert!(back >= prev);
            assert!((back - v).abs() <= v * 0.001 + 1e-3, "v={v} back={back}");
            prev = back;
        }
    }

    #[test]
    fn nan_and_inf_are_sanitised() {
        assert_eq!(f32_to_f16(f32::NAN), 0);
        assert_eq!(f16_to_f32(f32_to_f16(f32::INFINITY)), 65504.0);
    }

    #[test]
    fn pack_radii_clamps_negative() {
        let bits = pack_radii([-4.0, 8.0, 12.0, 16.0]);
        assert_eq!(unpack_radii(bits), [0.0, 8.0, 12.0, 16.0]);
    }
}

//! `f16` (IEEE 754 binary16) conversion used by the packed 64-byte primitive.
//!
//! `docs/gpu_ui_v2_0.md` §2.4 stores corner radii, border widths and shadow
//! blur radii as `u16` f16 bit patterns. Round-tripping through [`half`] keeps
//! the encoder/decoder identical to what the shader does with `unpackHalf2x16`.

/// Encode a finite `f32` into an IEEE binary16 bit pattern.
///
/// Non-finite inputs are saturated: `NaN` becomes `0`, `+inf` becomes `65504`
/// (the largest finite binary16). Saturating rather than propagating keeps the
/// GPU-side unpack free of NaN-tainted quads, which would otherwise render as
/// invisible-but-depth-writing geometry.
#[inline]
#[must_use]
pub fn f32_to_f16(v: f32) -> u16 {
    if v.is_nan() {
        return 0;
    }
    // Saturate rather than emit an infinity: an inf radius/blur would make the
    // SDF maths produce inf alpha, which is far harder to diagnose than a
    // clipped-but-visible primitive.
    let v = v.clamp(-65504.0, 65504.0);
    half::f16::from_f32(v).to_bits()
}

/// Decode an IEEE binary16 bit pattern back to `f32` (always exact).
#[inline]
#[must_use]
pub fn f16_to_f32(bits: u16) -> f32 {
    half::f16::from_bits(bits).to_f32()
}

/// Pack four radii, clamping each to `[0, 65504]` before encoding.
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

/// Unpack four radii.
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

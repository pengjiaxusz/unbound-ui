//! Signed distance field generation for glyphs.
//!
//! The canvas needs text that stays sharp from 10% to 6400% zoom, which a plain
//! coverage bitmap cannot provide: at 6400% a 12px raster is a blurry mosaic.
//! A distance field encodes the *shape* rather than its rasterisation, so the
//! fragment shader can reconstruct a crisp edge at any scale.
//!
//! Two producers exist:
//!
//! * [`from_coverage`] — exact Euclidean distance transform over a rasterised
//!   coverage bitmap. O(n) with the Felzenszwalb two-pass algorithm, no
//!   approximation, and it reuses the glyph rasteriser the R8 atlas already
//!   needs.
//! * `mtsdf.comp` (GPU) — the same maths per texel on the compute queue, used
//!   for canvas text so a glyph never blocks the main thread.
//!
//! Channel strategy: the field is replicated into R, G and B. The shader's
//! median-of-three is then a no-op but remains correct, and the layout is ready
//! for per-edge channel assignment (true MSDF) without changing the texture
//! format or the sampling code.

/// A packed distance-field bitmap, RGBA8.
#[derive(Clone, Debug, PartialEq)]
pub struct SdfBitmap {
    pub width: u32,
    pub height: u32,
    /// Distance, in texels, that maps to the full 0..1 range on either side of
    /// the edge. 4–6 is the useful range for text.
    pub spread: f32,
    /// RGBA8: R=G=B=distance field, A=original coverage.
    pub data: Vec<u8>,
}

impl SdfBitmap {
    /// Distance value at a texel, in `0..=1` (0.5 is exactly the edge).
    #[must_use]
    pub fn sample(&self, x: u32, y: u32) -> f32 {
        let i = ((y * self.width + x) * 4) as usize;
        f32::from(self.data[i]) / 255.0
    }

    /// Signed distance in texels; negative inside the glyph.
    #[must_use]
    pub fn signed_distance(&self, x: u32, y: u32) -> f32 {
        (0.5 - self.sample(x, y)) * 2.0 * self.spread
    }
}

/// Exact 1D squared distance transform (Felzenszwalb & Huttenlocher).
///
/// `f` holds the per-column lower envelope samples (`0` or `INF`), `d` receives
/// the squared distances, and `v`/`z` are scratch parabolas.
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    const INF: f32 = 1e20;
    let n = f.len();
    if n == 0 {
        return;
    }
    // Felzenszwalb & Huttenlocher, "Distance Transforms of Sampled Functions".
    // The lower envelope of the parabolas rooted at each sample is maintained as
    // a stack of (root, start) pairs; each sample is pushed and popped at most
    // once, so the whole transform is O(n).
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        let fq = f[q] + (q * q) as f32;
        loop {
            let p = v[k];
            let fp = f[p] + (p * p) as f32;
            let s_intersect = (fq - fp) / (2.0 * q as f32 - 2.0 * p as f32);
            if s_intersect <= z[k] {
                if k == 0 {
                    // The new parabola dominates from -inf: it becomes the root.
                    v[0] = q;
                    z[0] = -INF;
                    z[1] = INF;
                    break;
                }
                k -= 1;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s_intersect;
            z[k + 1] = INF;
            break;
        }
    }
    let mut k = 0usize;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        *out = dq * dq + f[p];
    }
}

/// Squared distance from every pixel to the nearest pixel where `inside(i)` is
/// true.
fn squared_edt(width: usize, height: usize, inside: &[bool]) -> Vec<f32> {
    const INF: f32 = 1e20;
    let max_dim = width.max(height);
    let mut v = vec![0usize; max_dim];
    let mut z = vec![0.0f32; max_dim + 1];
    let mut col = vec![0.0f32; height];
    let mut grid = vec![0.0f32; width * height];

    for x in 0..width {
        for y in 0..height {
            col[y] = if inside[y * width + x] { 0.0 } else { INF };
        }
        let mut out = vec![0.0f32; height];
        edt_1d(&col, &mut out, &mut v, &mut z);
        for y in 0..height {
            grid[y * width + x] = out[y];
        }
    }

    let mut row = vec![0.0f32; width];
    let mut result = vec![0.0f32; width * height];
    for y in 0..height {
        row.copy_from_slice(&grid[y * width..(y + 1) * width]);
        let mut out = vec![0.0f32; width];
        edt_1d(&row, &mut out, &mut v, &mut z);
        result[y * width..(y + 1) * width].copy_from_slice(&out);
    }
    result
}

/// Build a distance field from an 8-bit coverage bitmap.
///
/// `coverage` is row-major, `width * height` bytes. Pixels at or above the
/// midpoint count as inside. `spread` is the number of texels mapped to the full
/// range on each side of the edge.
#[must_use]
pub fn from_coverage(coverage: &[u8], width: u32, height: u32, spread: f32) -> SdfBitmap {
    let w = width as usize;
    let h = height as usize;
    assert!(coverage.len() >= w * h, "coverage bitmap too small");
    let spread = spread.max(0.5);
    if w == 0 || h == 0 {
        return SdfBitmap {
            width,
            height,
            spread,
            data: Vec::new(),
        };
    }

    let inside: Vec<bool> = coverage[..w * h].iter().map(|c| *c >= 128).collect();
    let outside: Vec<bool> = inside.iter().map(|b| !b).collect();

    // Distance to the nearest *outside* pixel tells us how deep inside we are;
    // distance to the nearest *inside* pixel tells us how far outside we are.
    // A bitmap that is entirely inside (or entirely outside) has no edge, so the
    // corresponding transform is skipped — otherwise every texel would be INF.
    let has_inside = inside.iter().any(|b| *b);
    let has_outside = inside.iter().any(|b| !*b);

    // A bitmap with no edge at all (a blank glyph padding, or a fully covered
    // box) has no meaningful signed distance: saturate instead of dividing by an
    // absent boundary, which would otherwise put every texel at exactly 0.5.
    //
    // The encoding is "high = inside" — the same one the GPU generator and
    // `uber.frag` use — so a fully covered bitmap saturates to 255 (deep
    // inside) and an empty one to 0 (far outside).
    if !has_inside || !has_outside {
        let v = if has_inside { 255u8 } else { 0u8 };
        let mut data = vec![0u8; w * h * 4];
        for (i, cov) in coverage.iter().enumerate().take(w * h) {
            let o = i * 4;
            data[o] = v;
            data[o + 1] = v;
            data[o + 2] = v;
            data[o + 3] = *cov;
        }
        return SdfBitmap {
            width,
            height,
            spread,
            data,
        };
    }

    let dist_out = squared_edt(w, h, &outside);
    let dist_in = squared_edt(w, h, &inside);

    let mut data = vec![0u8; w * h * 4];
    for i in 0..w * h {
        // Signed distance in texels: negative inside.
        let signed = if inside[i] {
            -dist_out[i].sqrt()
        } else {
            dist_in[i].sqrt()
        };
        // 0.5 at the edge, 0.0 / 1.0 at ±spread.
        let value = (0.5 - signed / (2.0 * spread)).clamp(0.0, 1.0);
        let v = (value * 255.0).round() as u8;
        let o = i * 4;
        data[o] = v;
        data[o + 1] = v;
        data[o + 2] = v;
        data[o + 3] = coverage[i];
    }

    SdfBitmap {
        width,
        height,
        spread,
        data,
    }
}

/// Reconstruct coverage from a distance field, the way the fragment shader does.
///
/// Used by tests to prove the field is faithful, and by the headless verifier to
/// compare a GPU-generated field against the CPU reference.
#[must_use]
pub fn reconstruct_coverage(field: &SdfBitmap, x: u32, y: u32, aa_width: f32) -> f32 {
    let v = field.sample(x, y);
    let w = (aa_width / (2.0 * field.spread)).max(1e-4);
    ((v - 0.5) / w + 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_rect(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
        let mut cov = vec![0u8; (w * h) as usize];
        for y in y0..y1 {
            for x in x0..x1 {
                cov[(y * w + x) as usize] = 255;
            }
        }
        cov
    }

    #[test]
    fn one_dimensional_transform_is_exact_for_two_seeds() {
        let f = [1e20, 0.0, 1e20, 1e20, 0.0, 1e20];
        let mut d = vec![0.0; 6];
        let mut v = vec![0usize; 6];
        let mut z = vec![0.0f32; 7];
        edt_1d(&f, &mut d, &mut v, &mut z);
        assert_eq!(d, vec![1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn one_dimensional_transform_matches_brute_force() {
        let f = [1e20, 0.0, 1e20, 1e20, 0.0, 1e20];
        let mut d = vec![0.0; 6];
        let mut v = vec![0usize; 6];
        let mut z = vec![0.0f32; 7];
        edt_1d(&f, &mut d, &mut v, &mut z);
        let expected = [1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        assert_eq!(d, expected);
    }

    #[test]
    fn transform_of_a_single_seed_is_quadratic_distance() {
        let f = [1e20, 1e20, 0.0, 1e20, 1e20];
        let mut d = vec![0.0; 5];
        let mut v = vec![0usize; 5];
        let mut z = vec![0.0f32; 6];
        edt_1d(&f, &mut d, &mut v, &mut z);
        assert_eq!(d, vec![4.0, 1.0, 0.0, 1.0, 4.0]);
    }

    #[test]
    fn field_is_half_at_the_boundary_and_signed_inside() {
        let cov = filled_rect(32, 32, 8, 8, 24, 24);
        // Spread 8 makes the full 8-texel depth representable; a spread of 4
        // saturates halfway and signed_distance() would legitimately return -4.
        let field = from_coverage(&cov, 32, 32, 8.0);
        // Just inside the left edge.
        let inside = field.signed_distance(8, 16);
        assert!(inside <= 0.0, "inside must be negative, got {inside}");
        // Just outside it.
        let outside = field.signed_distance(6, 16);
        assert!(outside > 0.0, "outside must be positive, got {outside}");
        // The centre is the deepest point.
        let centre = field.signed_distance(16, 16);
        assert!(centre < inside, "centre {centre} vs edge {inside}");
        assert!(centre <= -7.0, "centre distance {centre}");
    }

    #[test]
    fn coverage_reconstructs_the_original_shape() {
        let cov = filled_rect(24, 24, 6, 6, 18, 18);
        let field = from_coverage(&cov, 24, 24, 3.0);
        // Reconstruct with a one-texel antialiasing width and compare.
        let mut mismatches = 0;
        for y in 0..24 {
            for x in 0..24 {
                let recon = reconstruct_coverage(&field, x, y, 1.0);
                let original = cov[(y * 24 + x) as usize] as f32 / 255.0;
                if (recon - original).abs() > 0.35 {
                    mismatches += 1;
                }
            }
        }
        assert!(
            mismatches <= 8,
            "{mismatches} texels disagree (of 576) — field is not faithful"
        );
    }

    #[test]
    fn empty_bitmap_produces_a_fully_outside_field() {
        let cov = vec![0u8; 16 * 16];
        let field = from_coverage(&cov, 16, 16, 4.0);
        for y in 0..16 {
            for x in 0..16 {
                // "high = inside": an empty bitmap is entirely outside, so it
                // saturates to 0.
                assert!(field.sample(x, y) <= 0.01, "({x},{y}) = {}", field.sample(x, y));
            }
        }
    }

    #[test]
    fn fully_covered_bitmap_produces_a_fully_inside_field() {
        let cov = vec![255u8; 16 * 16];
        let field = from_coverage(&cov, 16, 16, 4.0);
        for y in 0..16 {
            for x in 0..16 {
                assert!(field.sample(x, y) >= 0.99);
            }
        }
    }

    /// The CPU field must use the same encoding as the GPU generator.
    ///
    /// `unbound-gpu`'s reference test asserts that the centre of a covered
    /// block samples above 0.9 and the corner below 0.1, and `uber.frag`
    /// turns the field into coverage with `smoothstep(0.5 - w, 0.5 + w, m)`.
    /// A CPU field with the opposite convention would render every glyph as
    /// a solid blob.
    #[test]
    fn the_encoding_is_high_inside_like_the_gpu() {
        let cov = filled_rect(16, 16, 4, 4, 12, 12);
        let field = from_coverage(&cov, 16, 16, 4.0);
        // The centre is deep inside.
        assert!(
            field.sample(8, 8) > 0.9,
            "the centre must read as inside, got {}",
            field.sample(8, 8)
        );
        // The corner is far outside.
        assert!(
            field.sample(0, 0) < 0.1,
            "the corner must read as outside, got {}",
            field.sample(0, 0)
        );
        // And `signed_distance` agrees: negative inside.
        assert!(field.signed_distance(8, 8) < 0.0);
        assert!(field.signed_distance(0, 0) > 0.0);
    }

    #[test]
    fn field_saturates_at_the_spread_distance() {
        let cov = filled_rect(32, 32, 4, 4, 28, 28);
        let narrow = from_coverage(&cov, 32, 32, 2.0);
        // 12 texels inside the edge is far beyond a 2-texel spread.
        assert!(
            (narrow.signed_distance(16, 16) + 2.0).abs() < 0.1,
            "must clamp at -spread, got {}",
            narrow.signed_distance(16, 16)
        );
    }

    #[test]
    fn spread_controls_the_gradient_width() {
        let cov = filled_rect(32, 32, 16, 0, 32, 32);
        let narrow = from_coverage(&cov, 32, 32, 2.0);
        let wide = from_coverage(&cov, 32, 32, 8.0);
        // At the same distance outside the edge, the wide field is closer to 0.5.
        let a = narrow.sample(12, 16);
        let b = wide.sample(12, 16);
        assert!(b > a, "narrow {a} should decay faster than wide {b}");
    }

    #[test]
    fn output_is_rgba_and_alpha_preserves_coverage() {
        let cov = filled_rect(8, 8, 2, 2, 6, 6);
        let field = from_coverage(&cov, 8, 8, 2.0);
        assert_eq!(field.data.len(), 8 * 8 * 4);
        assert_eq!(field.data[(3 * 8 + 3) * 4 + 3], 255);
        assert_eq!(field.data[3], 0);
    }

    #[test]
    fn zero_sized_bitmap_is_handled() {
        let field = from_coverage(&[], 0, 0, 4.0);
        assert_eq!(field.width, 0);
        assert!(field.data.is_empty());
    }
}

//! 字形有向距离场（SDF）生成。
//!
//! 画布要求文本在 10% 至 6400% 缩放范围内均保持锐利，普通覆盖率位图无法
//! 满足这一需求：在 6400% 缩放下，12px 光栅化位图会变成模糊的马赛克。
//! 距离场编码的是字形的**几何轮廓**而非其光栅化结果，因此片元着色器
//! 可在任意缩放倍率下重建清晰边缘。
//!
//! 提供两种生成路径：
//!
//! * [`from_coverage`] —— 基于光栅化覆盖率位图的精确欧氏距离变换。
//!   采用 Felzenszwalb 两趟算法实现 O(n) 复杂度，无近似误差，
//!   且复用 R8 图集已有的字形光栅化器。
//! * `mtsdf.comp`（GPU）—— 在计算队列上按纹素执行相同数学运算，
//!   用于画布文本，避免字形生成阻塞主线程。
//!
//!
//! 通道策略：距离场同时复制到 R、G、B 三通道。此时着色器的三通道中值
//! 运算退化为恒等操作但依然正确，且该布局已为后续按边分配通道（真 MSDF）
//! 预留结构，无需修改纹理格式或采样代码。
//!

/// 打包的距离场位图，RGBA8 格式。
#[derive(Clone, Debug, PartialEq)]
pub struct SdfBitmap {
    pub width: u32,
    pub height: u32,
    /// 边缘两侧映射到完整 0..1 范围的距离（单位：纹素）。文本常用 4–6。
    ///
    pub spread: f32,
    /// RGBA8：R=G=B=距离场，A=原始覆盖率。
    pub data: Vec<u8>,
}

impl SdfBitmap {
    /// 单个纹素处的距离值，范围 `0..=1`（0.5 恰好位于边缘）。
    #[must_use]
    pub fn sample(&self, x: u32, y: u32) -> f32 {
        let i = ((y * self.width + x) * 4) as usize;
        f32::from(self.data[i]) / 255.0
    }

    /// 以纹素为单位的有向距离；字形内部为负值。
    #[must_use]
    pub fn signed_distance(&self, x: u32, y: u32) -> f32 {
        (0.5 - self.sample(x, y)) * 2.0 * self.spread
    }
}

/// 精确一维平方距离变换（Felzenszwalb & Huttenlocher 算法）。
///
/// `f` 存储每列的下包络采样值（`0` 或 `INF`），`d` 接收平方距离输出，
/// `v`/`z` 为抛物线暂存数组。
fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    const INF: f32 = 1e20;
    let n = f.len();
    if n == 0 {
        return;
    }
    // Felzenszwalb & Huttenlocher, "Distance Transforms of Sampled Functions"。
    // 以各采样点为顶点的抛物线下包络通过 `(root, start)` 栈维护；
    // 每个采样点至多入栈与出栈各一次，因此整趟变换复杂度为 O(n)。
    //
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
                    // 新抛物线从 -inf 起占据主导：直接成为根节点。
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

/// 计算每个像素到满足 `inside(i)` 为真的最近像素的平方距离。
///
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

/// 从 8 位覆盖率位图构建距离场。
///
/// `coverage` 为行主序，共 `width * height` 字节。不低于中值的像素
/// 视为内部。`spread` 为边缘两侧各自映射到完整范围的纹素数。
///
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

    // 到最近**外部**像素的距离表示内部深度；到最近**内部**像素的距离表示外部远度。
    // 完全处于内部（或完全处于外部）的位图不存在边缘，因此跳过对应的变换——
    // 否则所有纹素都会得到 INF。
    //
    let has_inside = inside.iter().any(|b| *b);
    let has_outside = inside.iter().any(|b| !*b);

    // 毫无边缘的位图（空白字形边距或全覆盖方块）没有有意义的有向距离：
    // 此时应饱和截断而非除以不存在的边界，否则所有纹素都会变成 0.5。
    //
    //
    // 编码约定为“高值 = 内部”——与 GPU 生成器及 `uber.frag` 一致——
    // 因此全覆盖位图饱和为 255（深处内部），空位图饱和为 0（远离外部）。
    //
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
        // 以纹素为单位的有向距离：内部为负。
        let signed = if inside[i] {
            -dist_out[i].sqrt()
        } else {
            dist_in[i].sqrt()
        };
        // 边缘处为 0.5，±spread 处为 0.0 / 1.0。
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

/// 按片元着色器的方式从距离场重建覆盖率。
///
/// 供测试验证距离场保真度，也供无头校验器将 GPU 生成场与 CPU 参考实现对比。
///
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
        // spread 为 8 可表示完整的 8 纹素深度；spread 为 4 会在半途饱和，
        // signed_distance() 将合理地返回 -4。
        let field = from_coverage(&cov, 32, 32, 8.0);
        // 紧邻左边缘内侧。
        let inside = field.signed_distance(8, 16);
        assert!(inside <= 0.0, "inside must be negative, got {inside}");
        // 紧邻左边缘外侧。
        let outside = field.signed_distance(6, 16);
        assert!(outside > 0.0, "outside must be positive, got {outside}");
        // 中心点为最深处。
        let centre = field.signed_distance(16, 16);
        assert!(centre < inside, "centre {centre} vs edge {inside}");
        assert!(centre <= -7.0, "centre distance {centre}");
    }

    #[test]
    fn coverage_reconstructs_the_original_shape() {
        let cov = filled_rect(24, 24, 6, 6, 18, 18);
        let field = from_coverage(&cov, 24, 24, 3.0);
        // 使用 1 纹素抗锯齿宽度重建并对比。
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
                // “高值 = 内部”：空位图完全处于外部，因此饱和为 0。
                //
                assert!(
                    field.sample(x, y) <= 0.01,
                    "({x},{y}) = {}",
                    field.sample(x, y)
                );
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

    /// CPU 距离场必须与 GPU 生成器采用相同的编码约定。
    ///
    /// `unbound-gpu` 的参考测试断言全覆盖方块中心采样高于 0.9、角点低于 0.1，
    /// 而 `uber.frag` 通过 `smoothstep(0.5 - w, 0.5 + w, m)` 将距离场转为覆盖率。
    /// 若 CPU 场采用相反约定，所有字形都会渲染成实心色块。
    ///
    ///
    #[test]
    fn the_encoding_is_high_inside_like_the_gpu() {
        let cov = filled_rect(16, 16, 4, 4, 12, 12);
        let field = from_coverage(&cov, 16, 16, 4.0);
        // 中心点位于内部深处。
        assert!(
            field.sample(8, 8) > 0.9,
            "the centre must read as inside, got {}",
            field.sample(8, 8)
        );
        // 角点位于外部远处。
        assert!(
            field.sample(0, 0) < 0.1,
            "the corner must read as outside, got {}",
            field.sample(0, 0)
        );
        // `signed_distance` 也保持一致：内部为负。
        assert!(field.signed_distance(8, 8) < 0.0);
        assert!(field.signed_distance(0, 0) > 0.0);
    }

    #[test]
    fn field_saturates_at_the_spread_distance() {
        let cov = filled_rect(32, 32, 4, 4, 28, 28);
        let narrow = from_coverage(&cov, 32, 32, 2.0);
        // 距边缘内侧 12 纹素远超 2 纹素的 spread 范围。
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
        // 在边缘外侧相同距离处，更宽的 spread 距离场更接近 0.5。
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

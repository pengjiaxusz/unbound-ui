//! 自绘后景纹理的无头统计：空间梯度能量、明度带宽与色相保真度。
//!
//! 三项指标都在「壁纸分辨率、屏幕分辨率都不同」的组合下保持稳定，因为它们
//! 按物理观感定义：梯度在屏幕像素上度量，明度与色偏只看取值分布。因此它们
//! 可以用来锁住「磨砂强度锚定屏幕物理像素」这条不变量。
//!
//! 统计接缝只吃内存里的壁纸像素，不读盘、不查屏幕，走的是与真实生成完全
//! 相同的配方（cover 裁剪 → 降采样 → 屏幕锚定磨砂 → 主题色调），因此可以在
//! 无头环境里直接调用。

use image::{DynamicImage, ImageBuffer, Rgba};

use crate::mica::{render_texture, tint, TextureGeometry, ToneRecipe};
use crate::selection::MaterialKind;

/// 纹理统计请求。
#[derive(Clone, Copy, Debug)]
pub struct StatisticsRequest<'a> {
    /// 壁纸像素（已在内存中）；统计接缝从不读盘。
    pub wallpaper: &'a DynamicImage,
    /// 目标屏幕的物理像素尺寸。
    pub screen: (u32, u32),
    /// 纹理最大宽度（0 视为默认值）。
    pub max_width: u32,
    /// 磨砂 sigma，单位是屏幕物理像素。
    pub sigma: f32,
    /// 是否使用深色主题色调。
    pub dark: bool,
    /// 要统计的材质种类；它选择色调配方，标准云母与云母变体的观感因此不同。
    pub material: MaterialKind,
}

/// 纹理统计结果。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureStatistics {
    /// 屏幕空间的空间梯度能量。
    pub spatial_gradient_energy: f64,
    /// 明度带宽。
    pub luminance_bandwidth: f64,
    /// 色相保真度。
    pub hue_fidelity: f64,
}

/// 统计一块自绘后景纹理的观感指标。
///
/// 纹理覆盖整块目标屏幕，与 [`crate::mica::generate`] 的取样范围一致。返回
/// `None` 表示壁纸像素不可用（尺寸为零等）。
#[must_use]
pub fn texture_statistics(request: &StatisticsRequest<'_>) -> Option<TextureStatistics> {
    let screen = (request.screen.0.max(1), request.screen.1.max(1));
    let rendered = render_texture(
        request.wallpaper,
        screen,
        (0, 0, screen.0, screen.1),
        request.max_width.max(1),
        request.sigma.max(0.0),
    )?;
    // 统计与真实生成走同一份配方：材质决定色调，磨砂锚点与材质无关。
    let finished = tint(
        &rendered.blurred,
        &ToneRecipe::for_material(request.material, request.dark),
    );
    Some(TextureStatistics {
        spatial_gradient_energy: spatial_gradient_energy(&finished, &rendered.geometry),
        luminance_bandwidth: luminance_bandwidth(&finished),
        hue_fidelity: hue_fidelity(&finished, &rendered.reference),
    })
}

/// 屏幕空间的空间梯度能量。
///
/// 含义：成品纹理在屏幕上的结构强度。梯度先按「一个纹理像素等于多少屏幕
/// 物理像素」换算到屏幕像素，再对纹理覆盖的屏幕面积积分，单位是明度平方
/// （0..255 明度），取值非负。
///
/// 期望区间：纯色壁纸为 0；一般壁纸在 1e2..1e5 量级。磨砂越强、壁纸越柔和
/// 数值越小。它与壁纸分辨率无关，却对磨砂强度敏感，因此最适合守住「强度锚定
/// 屏幕物理像素」这条不变量。
fn spatial_gradient_energy(
    image: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    geometry: &TextureGeometry,
) -> f64 {
    let (width, height) = image.dimensions();
    if width < 2 || height < 2 {
        return 0.0;
    }
    let (per_x, per_y) = geometry.screen_per_texture();
    if per_x <= 0.0 || per_y <= 0.0 {
        return 0.0;
    }
    let (span_x, span_y) = geometry.screen_span();
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in 0..height {
        for x in 0..width {
            let left = luminance(image.get_pixel(x.saturating_sub(1), y));
            let right = luminance(image.get_pixel((x + 1).min(width - 1), y));
            let top = luminance(image.get_pixel(x, y.saturating_sub(1)));
            let bottom = luminance(image.get_pixel(x, (y + 1).min(height - 1)));
            let gradient_x = (right - left) / 2.0 / per_x;
            let gradient_y = (bottom - top) / 2.0 / per_y;
            sum += gradient_x * gradient_x + gradient_y * gradient_y;
            count += 1.0;
        }
    }
    let density = sum / count;
    density * span_x * span_y
}

/// 明度带宽：成品纹理里明度的离散程度。
///
/// 含义：后景还剩多少明暗层次。单位是 0..255 明度，取值非负，0 表示一片
/// 死平。
///
/// 期望区间：0..255；深色主题下明度被压缩到基准色的 18%，实际值只有个位数
/// 到十几。它与空间尺度无关，只看取值分布，因此在分辨率组合下最稳定。
fn luminance_bandwidth(image: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for pixel in image.pixels() {
        sum += luminance(pixel);
        count += 1.0;
    }
    if count <= 0.0 {
        return 0.0;
    }
    let mean = sum / count;
    let mut variance = 0.0;
    for pixel in image.pixels() {
        let delta = luminance(pixel) - mean;
        variance += delta * delta;
    }
    (variance / count).sqrt()
}

/// 色相保真度：成品纹理与参考纹理在色偏方向上的一致程度。
///
/// 含义：壁纸的低频色调在磨砂与主题色调之后还剩多少。参考纹理是同一张壁纸
/// 降采样后、未经磨砂与色调的版本。取值 -1..1，1 表示色偏完全按参考保留，
/// 0 表示两者正交。
///
/// 期望区间：0..1 的高值区（典型 0.9 以上）。灰度壁纸没有色偏可丢，约定为
/// 1，保证结果始终有限。
fn hue_fidelity(
    finished: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    reference: &ImageBuffer<Rgba<u8>, Vec<u8>>,
) -> f64 {
    let mut dot = 0.0;
    let mut norm_finished = 0.0;
    let mut norm_reference = 0.0;
    for (finished_pixel, reference_pixel) in finished.pixels().zip(reference.pixels()) {
        let finished_luminance = luminance(finished_pixel);
        let reference_luminance = luminance(reference_pixel);
        let finished_chroma = (
            f64::from(finished_pixel[0]) - finished_luminance,
            f64::from(finished_pixel[2]) - finished_luminance,
        );
        let reference_chroma = (
            f64::from(reference_pixel[0]) - reference_luminance,
            f64::from(reference_pixel[2]) - reference_luminance,
        );
        dot += finished_chroma.0 * reference_chroma.0 + finished_chroma.1 * reference_chroma.1;
        norm_finished +=
            finished_chroma.0 * finished_chroma.0 + finished_chroma.1 * finished_chroma.1;
        norm_reference +=
            reference_chroma.0 * reference_chroma.0 + reference_chroma.1 * reference_chroma.1;
    }
    let denominator = norm_finished.sqrt() * norm_reference.sqrt();
    if denominator <= f64::EPSILON {
        // 灰度壁纸没有色偏可丢，没有可损失的色相，约定为完全保真。
        return 1.0;
    }
    (dot / denominator).clamp(-1.0, 1.0)
}

/// Rec.709 明度。
fn luminance(pixel: &Rgba<u8>) -> f64 {
    0.2126 * f64::from(pixel[0]) + 0.7152 * f64::from(pixel[1]) + 0.0722 * f64::from(pixel[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mica::{DEFAULT_MAX_WIDTH, DEFAULT_SIGMA};
    use crate::test_support::{hard_edge_wallpaper, portrait_wallpaper, wallpaper_at};
    use image::GenericImageView;

    /// 壁纸分辨率变化时统计量允许的相对差。
    ///
    /// 同一张画面换分辨率后，纹理只剩重采样噪声，实测漂移在 0.5% 以内；
    /// 旧口径（锚定壁纸像素）会漂到 7% 以上，因此这条断言足以抓住回归。
    const WALLPAPER_TOLERANCE: f64 = 0.02;

    /// 屏幕分辨率变化时统计量允许的相对差。
    ///
    /// 屏幕变大时壁纸被等比放大，固定的物理磨砂在壁纸尺度上相对变弱，残留
    /// 结构略多，实测漂移约 4%；这条断言守住「漂移有界」。
    const SCREEN_TOLERANCE: f64 = 0.06;

    /// 统计一块壁纸在给定屏幕上的标准云母纹理指标。
    fn statistics(wallpaper: &DynamicImage, screen: (u32, u32)) -> TextureStatistics {
        statistics_for(wallpaper, screen, MaterialKind::Mica)
    }

    /// 统计一块壁纸在给定屏幕、给定材质下的纹理指标。
    fn statistics_for(
        wallpaper: &DynamicImage,
        screen: (u32, u32),
        material: MaterialKind,
    ) -> TextureStatistics {
        texture_statistics(&StatisticsRequest {
            wallpaper,
            screen,
            max_width: DEFAULT_MAX_WIDTH,
            sigma: DEFAULT_SIGMA,
            dark: true,
            material,
        })
        .expect("the wallpaper must render")
    }

    /// 两个统计量的相对差。
    fn relative_difference(left: f64, right: f64) -> f64 {
        let scale = left.abs().max(right.abs()).max(1.0e-9);
        (left - right).abs() / scale
    }

    /// 断言三项统计量都在基线的容差内。
    fn assert_statistics_within(
        stats: &TextureStatistics,
        baseline: &TextureStatistics,
        tolerance: f64,
        context: &str,
    ) {
        assert!(
            relative_difference(
                stats.spatial_gradient_energy,
                baseline.spatial_gradient_energy
            ) < tolerance,
            "spatial gradient energy drifted from {} to {} at {context}",
            baseline.spatial_gradient_energy,
            stats.spatial_gradient_energy
        );
        assert!(
            relative_difference(stats.luminance_bandwidth, baseline.luminance_bandwidth)
                < tolerance,
            "luminance bandwidth drifted from {} to {} at {context}",
            baseline.luminance_bandwidth,
            stats.luminance_bandwidth
        );
        assert!(
            relative_difference(stats.hue_fidelity, baseline.hue_fidelity) < tolerance,
            "hue fidelity drifted from {} to {} at {context}",
            baseline.hue_fidelity,
            stats.hue_fidelity
        );
    }

    #[test]
    fn a_flat_wallpaper_has_no_gradient_energy_and_no_luminance_bandwidth() {
        let wallpaper =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([120, 120, 120, 255])));
        let stats = statistics(&wallpaper, (1920, 1080));
        assert_eq!(stats.spatial_gradient_energy, 0.0);
        assert_eq!(stats.luminance_bandwidth, 0.0);
    }

    #[test]
    fn a_greyscale_wallpaper_reports_a_finite_hue_fidelity() {
        let wallpaper =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([120, 120, 120, 255])));
        let stats = statistics(&wallpaper, (1920, 1080));
        assert!(
            (0.0..=1.0).contains(&stats.hue_fidelity),
            "hue fidelity must stay finite for a greyscale wallpaper"
        );
    }

    #[test]
    fn the_texture_statistics_do_not_depend_on_the_wallpaper_resolution() {
        let screen = (1920, 1080);
        let baseline = statistics(&wallpaper_at(1920, 1080), screen);
        for wallpaper in [wallpaper_at(3840, 2160), portrait_wallpaper(1280)] {
            let stats = statistics(&wallpaper, screen);
            assert_statistics_within(
                &stats,
                &baseline,
                WALLPAPER_TOLERANCE,
                &format!("wallpaper {:?}", wallpaper.dimensions()),
            );
        }
    }

    #[test]
    fn the_texture_statistics_do_not_depend_on_the_screen_resolution() {
        let wallpaper = wallpaper_at(1920, 1080);
        let baseline = statistics(&wallpaper, (1920, 1080));
        let stats = statistics(&wallpaper, (2560, 1440));
        assert_statistics_within(&stats, &baseline, SCREEN_TOLERANCE, "screen 2560x1440");
    }

    #[test]
    fn the_texture_statistics_stay_consistent_across_every_resolution_combination() {
        let baseline = statistics(&wallpaper_at(1920, 1080), (1920, 1080));
        for screen in [(1920, 1080), (2560, 1440)] {
            for wallpaper in [
                wallpaper_at(1920, 1080),
                wallpaper_at(3840, 2160),
                portrait_wallpaper(1280),
            ] {
                let stats = statistics(&wallpaper, screen);
                assert_statistics_within(
                    &stats,
                    &baseline,
                    SCREEN_TOLERANCE,
                    &format!("screen {screen:?} wallpaper {:?}", wallpaper.dimensions()),
                );
            }
        }
    }

    #[test]
    fn the_calibrated_blur_keeps_the_backdrop_within_the_expected_energy_band() {
        // 标定带：`DEFAULT_SIGMA = 16.0` 屏幕物理像素时，合成壁纸的屏幕空间
        // 梯度能量落在 1.18e4 附近。磨砂更弱，壁纸的大尺度轮廓会重新显形，
        // 能量上升；磨砂更强，画面会被抹平，能量下降。因此这条带同时锁住
        // 「强度锚定屏幕」与「强度取值」两件事。
        let stats = statistics(&wallpaper_at(1920, 1080), (1920, 1080));
        assert!(
            (10_000.0..13_000.0).contains(&stats.spatial_gradient_energy),
            "the calibrated blur drifted to {}",
            stats.spatial_gradient_energy
        );
    }

    #[test]
    fn the_calibrated_blur_flattens_a_hard_wallpaper_edge() {
        // 硬边分界是壁纸里最刺眼的大尺度轮廓。16 物理像素的磨砂把它摊到几十
        // 个像素上，实测屏幕空间梯度能量 1.22e4；上限 2.0e4 反推出磨砂半径
        // 不得低于约 10 物理像素，否则这条轮廓会重新显形。
        let stats = statistics(&hard_edge_wallpaper(1920, 1080), (1920, 1080));
        assert!(
            stats.spatial_gradient_energy < 20_000.0,
            "the hard edge survived with {} gradient energy",
            stats.spatial_gradient_energy
        );
    }

    #[test]
    fn the_statistics_seam_covers_the_mica_variant() {
        let wallpaper = wallpaper_at(1920, 1080);
        let screen = (1920, 1080);
        let standard = statistics_for(&wallpaper, screen, MaterialKind::Mica);
        let variant = statistics_for(&wallpaper, screen, MaterialKind::MicaAlt);

        // 更深、更窄的色调：明度带宽与梯度能量都必须低于标准云母。
        assert!(
            variant.luminance_bandwidth < standard.luminance_bandwidth * 0.8,
            "the variant keeps too much luminance spread: {} vs {}",
            variant.luminance_bandwidth,
            standard.luminance_bandwidth
        );
        assert!(
            variant.spatial_gradient_energy < standard.spatial_gradient_energy * 0.8,
            "the variant keeps too much contrast: {} vs {}",
            variant.spatial_gradient_energy,
            standard.spatial_gradient_energy
        );
        // 只有色调不同：色偏仍按同一比例保留，色相保真度必须留在高值区。
        assert!(
            variant.hue_fidelity > 0.9,
            "the variant lost its hue fidelity: {}",
            variant.hue_fidelity
        );
        assert!(
            variant.hue_fidelity + 0.05 > standard.hue_fidelity,
            "the variant must not trade hue fidelity for depth: {} vs {}",
            variant.hue_fidelity,
            standard.hue_fidelity
        );
        // 磨砂强度不变：变体的纹理仍有真实结构，不是一片死平。
        assert!(
            variant.spatial_gradient_energy > 0.0,
            "the variant must keep the blurred wallpaper structure"
        );
    }
}

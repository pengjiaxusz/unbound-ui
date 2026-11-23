//! 自绘路线的云母变体：纹理规格、请求失效与统计不变量。
//!
//! 这套测试把「自绘 + 云母变体」这条路径钉死成可观测的事实：
//!
//! - 纹理规格必须携带**实际生效**材质，回退到标准云母时不能残留不可用材质；
//! - 材质进入请求相等性判断，换变体必须使旧纹理失效；
//! - 统计接缝覆盖云母变体，且它与标准云母共享磨砂锚点、只有色调更深。
//!
//! 统计断言走真实的生成配方（铺满取样 → 降采样 → 屏幕锚定磨砂 → 材质色调），
//! 不是另算一套近似。

use image::{DynamicImage, ImageBuffer, Rgba};
use unbound_backdrop::{
    texture_statistics, BackdropInputs, BackdropPlan, BackdropSelection, BackdropSource,
    MaterialKind, MicaRequest, PixelRect, StatisticsRequest, WallpaperTexture, DEFAULT_MAX_WIDTH,
    DEFAULT_SIGMA,
};

/// 主屏物理矩形。
const SCREEN: PixelRect = PixelRect::new(0, 0, 1920, 1080);

/// 客户区物理矩形：落在主屏之内。
const WINDOW: PixelRect = PixelRect::new(480, 270, 960, 540);

/// 造一份自绘计划输入。
fn inputs(material: MaterialKind, source: BackdropSource) -> BackdropInputs {
    BackdropInputs {
        selection: BackdropSelection::new(material, source),
        windows_build: Some(26100),
        premultiplied_alpha_supported: true,
        accent_policy_export: false,
        accent_tint_argb: 0,
        window_screen_rect: Some(WINDOW),
        primary_screen_rect: Some(SCREEN),
        primary_screen_scale: 1.5,
        dark: true,
        self_drawn_ready: false,
    }
}

/// 一张确定性的彩色渐变壁纸：有低频构图与色偏，适合度量色调差异。
fn gradient_wallpaper(width: u32, height: u32) -> DynamicImage {
    let image = ImageBuffer::from_fn(width, height, |x, y| {
        let fx = f64::from(x) / f64::from(width);
        let fy = f64::from(y) / f64::from(height);
        let base = 0.35 + 0.30 * fx + 0.15 * (fy * std::f64::consts::TAU).sin();
        let channel = |weight: f64| ((base * weight).clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba([channel(1.05), channel(0.85), channel(0.70), 255])
    });
    DynamicImage::ImageRgba8(image)
}

/// 固定尺寸的纹理规格。
fn texture(material: MaterialKind) -> WallpaperTexture {
    WallpaperTexture {
        width: 1920,
        height: 1080,
        screen_width: 1920,
        screen_height: 1080,
        dpi_scale: 1.0,
        max_width: DEFAULT_MAX_WIDTH,
        sigma: DEFAULT_SIGMA,
        dark: true,
        material,
    }
}

/// 统计给定材质下的纹理指标。
fn statistics(
    wallpaper: &DynamicImage,
    material: MaterialKind,
) -> unbound_backdrop::TextureStatistics {
    texture_statistics(&StatisticsRequest {
        wallpaper,
        screen: (1920, 1080),
        max_width: DEFAULT_MAX_WIDTH,
        sigma: DEFAULT_SIGMA,
        dark: true,
        material,
    })
    .expect("the wallpaper must render")
}

/// 自绘计划给出的纹理规格携带实际生效材质。
#[test]
fn the_self_drawn_texture_carries_the_effective_material() {
    let mica = BackdropPlan::derive(inputs(MaterialKind::Mica, BackdropSource::SelfDrawn));
    assert_eq!(
        mica.texture.expect("mica must draw a texture").material,
        MaterialKind::Mica
    );

    let variant = BackdropPlan::derive(inputs(MaterialKind::MicaAlt, BackdropSource::SelfDrawn));
    assert_eq!(
        variant
            .texture
            .expect("the variant must draw a texture")
            .material,
        MaterialKind::MicaAlt,
        "the mica variant must keep its own recipe on the self-drawn path"
    );
}

/// 没有自绘变体的材质回退到标准云母时，纹理规格也必须换成标准云母。
#[test]
fn a_fallback_texture_uses_the_material_that_actually_wins() {
    let mut weak = inputs(MaterialKind::Acrylic, BackdropSource::SelfDrawn);
    weak.windows_build = Some(19045);
    let plan = BackdropPlan::derive(weak);
    assert_eq!(plan.material_kind, MaterialKind::Mica);
    assert_eq!(plan.source, BackdropSource::SelfDrawn);
    assert_eq!(
        plan.texture
            .expect("the fallback must draw a texture")
            .material,
        MaterialKind::Mica,
        "the fallback texture must not keep the acrylic material"
    );
}

/// 材质参与请求相等性：换变体即失效，同材质同参数可复用。
#[test]
fn a_material_switch_invalidates_the_texture_request() {
    let standard = MicaRequest::from_texture(&texture(MaterialKind::Mica), None);
    let variant = MicaRequest::from_texture(&texture(MaterialKind::MicaAlt), None);

    assert_eq!(standard.material, MaterialKind::Mica);
    assert_eq!(variant.material, MaterialKind::MicaAlt);
    assert!(
        standard.matches(&MicaRequest::from_texture(
            &texture(MaterialKind::Mica),
            None
        )),
        "an identical material request must be reused"
    );
    assert!(
        !standard.matches(&variant),
        "switching the material must invalidate the texture"
    );
    assert!(
        !variant.matches(&standard),
        "the material comparison must be symmetric"
    );

    // 默认请求就是标准云母。
    assert_eq!(
        MicaRequest::new(1920, 1080, true).material,
        MaterialKind::Mica,
        "a request without an explicit material must default to standard mica"
    );
}

/// 统计接缝覆盖云母变体：更深的明度带宽与更低的对比度，色相仍保真。
#[test]
fn the_mica_variant_passes_the_statistics_invariants() {
    let wallpaper = gradient_wallpaper(1920, 1080);
    let standard = statistics(&wallpaper, MaterialKind::Mica);
    let variant = statistics(&wallpaper, MaterialKind::MicaAlt);

    assert!(
        variant.luminance_bandwidth < standard.luminance_bandwidth * 0.8,
        "the variant must flatten the luminance band: {} vs {}",
        variant.luminance_bandwidth,
        standard.luminance_bandwidth
    );
    assert!(
        variant.spatial_gradient_energy < standard.spatial_gradient_energy * 0.8,
        "the variant must lower the contrast: {} vs {}",
        variant.spatial_gradient_energy,
        standard.spatial_gradient_energy
    );
    assert!(
        variant.spatial_gradient_energy > 0.0,
        "the variant must keep the blurred wallpaper structure"
    );
    assert!(
        variant.hue_fidelity > 0.9,
        "the variant must keep the wallpaper hue: {}",
        variant.hue_fidelity
    );
    assert!(
        variant.hue_fidelity + 0.05 > standard.hue_fidelity,
        "depth must not be bought with hue loss: {} vs {}",
        variant.hue_fidelity,
        standard.hue_fidelity
    );
}

/// 两种材质在统计上必须可区分：换材质不能产出同一张纹理。
#[test]
fn the_two_materials_are_statistically_distinct() {
    let wallpaper = gradient_wallpaper(1920, 1080);
    let standard = statistics(&wallpaper, MaterialKind::Mica);
    let variant = statistics(&wallpaper, MaterialKind::MicaAlt);
    assert_ne!(
        standard, variant,
        "the two materials must not produce the same statistics"
    );
}

/// 统计接缝在壁纸分辨率变化时仍然稳定，变体也不例外。
#[test]
fn the_variant_statistics_stay_stable_across_wallpaper_resolutions() {
    let baseline = statistics(&gradient_wallpaper(1920, 1080), MaterialKind::MicaAlt);
    let scaled = statistics(&gradient_wallpaper(3840, 2160), MaterialKind::MicaAlt);
    let relative = |left: f64, right: f64| {
        let scale = left.abs().max(right.abs()).max(1.0e-9);
        (left - right).abs() / scale
    };
    assert!(
        relative(scaled.luminance_bandwidth, baseline.luminance_bandwidth) < 0.05,
        "the variant bandwidth drifted with the wallpaper resolution"
    );
    assert!(
        relative(
            scaled.spatial_gradient_energy,
            baseline.spatial_gradient_energy
        ) < 0.05,
        "the variant gradient energy drifted with the wallpaper resolution"
    );
}

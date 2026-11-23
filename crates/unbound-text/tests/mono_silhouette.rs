//! 单色剪影（`EmojiPresentation::Mono`）的跨后端一致性断言。
//!
//! 验收目标是「`Mono` 是基字形剪影」，不是「把彩色位图转成灰度」：
//!
//! * 深色 Emoji（🖤 / 🎱）的基字形轮廓是实心的，剪影的平均覆盖率应当很高；
//!   若仍按亮度取灰，这两颗字会几乎全黑，平均覆盖率接近 0。
//! * DirectWrite 与 swash 两个后端对同一字形的剪影应当在尺寸与填充率上接近。
//!
//! 需要系统装有 Segoe UI Emoji 与 DirectWrite；其他环境整体跳过。

use unbound_math::Vec2;
use unbound_text::{
    AtlasTarget, EmojiBackend, EmojiPresentation, GlyphCache, RasterBackend, TextEngine,
    TextRenderReport, TextStyle,
};

fn cache_with(backend: RasterBackend, presentation: EmojiPresentation) -> (TextEngine, GlyphCache) {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::with_backend(backend);
    let mut settings = cache.settings().clone();
    settings.emoji_presentation = presentation;
    cache.apply_settings(&settings, Some(&mut engine));
    (engine, cache)
}

/// 渲染一个字形，返回 R8 覆盖率位图与其尺寸。
fn render_mono(backend: RasterBackend, text: &str) -> (Vec<u8>, u32, u32) {
    let (mut engine, mut cache) = cache_with(backend, EmojiPresentation::Mono);
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape(text, &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
    assert_eq!(quads.len(), 1, "{text} must shape into one cluster");

    let updates = cache.take_updates();
    assert!(
        updates
            .iter()
            .all(|u| matches!(u.target, AtlasTarget::Coverage)),
        "the mono path may only write the R8 atlas"
    );
    let update = updates.first().expect("mono path must upload coverage");
    (update.data.clone(), update.width, update.height)
}

/// 位图的平均覆盖率与非零像素占比。
fn coverage_stats(data: &[u8]) -> (f32, f32) {
    let sum: u64 = data.iter().map(|v| u64::from(*v)).sum();
    let non_zero = data.iter().filter(|v| **v > 0).count();
    let len = data.len().max(1) as f32;
    (sum as f32 / (255.0 * len), non_zero as f32 / len)
}

/// 验收：深色 Emoji 的 `Mono` 必须是实心剪影，而不是亮度灰度。
///
/// 判别力来自色彩本身：🖤 与 🎱 几乎全黑，取亮度得到的位图接近全 0；
/// 基字形剪影把整块区域填满，平均覆盖率接近「包围盒填充率」。
/// 依赖带基字形轮廓的 COLR/CPAL 字体（如 Windows 的 Segoe UI Emoji）；
/// Linux 常见 CBDT/CBLC 纯位图字体无基字形轮廓，按设计回退至占位框。
#[cfg(windows)]
#[test]
fn mono_presentation_draws_a_solid_silhouette_for_dark_emoji() {
    for text in ["\u{1F5A4}", "\u{1F3B1}"] {
        let (data, _, _) = render_mono(RasterBackend::default(), text);
        let (mean, non_zero) = coverage_stats(&data);
        println!("{text}: mean={mean:.3} non_zero={non_zero:.3}");
        assert!(
            mean > 0.40,
            "{text}: a base-outline silhouette must be mostly filled, \
             luminance conversion would stay near zero; mean coverage {mean}"
        );
        assert!(
            non_zero > 0.40,
            "{text}: silhouette must cover the glyph box; non-zero ratio {non_zero}"
        );
    }
}

/// 验收：两个后端对同一 `Mono` 设置的位图在容差内一致。
///
/// 只比较「包围盒填充率」与「宽高比」这类形状指标，不逐像素比对：
/// 两个光栅器的抗锯齿实现不同，逐像素比会长期飘红。
#[test]
fn mono_bitmap_shape_matches_across_backends() {
    if !cfg!(all(windows, feature = "backend-directwrite")) || !cfg!(feature = "backend-swash") {
        return;
    }
    let (dw_data, dw_w, dw_h) = render_mono(RasterBackend::DirectWrite, "\u{1F600}");
    let (sw_data, sw_w, sw_h) = render_mono(RasterBackend::Cosmic, "\u{1F600}");

    let (dw_mean, _) = coverage_stats(&dw_data);
    let (sw_mean, _) = coverage_stats(&sw_data);
    let dw_ratio = dw_w as f32 / dw_h as f32;
    let sw_ratio = sw_w as f32 / sw_h as f32;

    println!("directwrite {dw_w}x{dw_h} mean={dw_mean:.3}; swash {sw_w}x{sw_h} mean={sw_mean:.3}");
    assert!(
        (dw_ratio - sw_ratio).abs() < 0.25,
        "silhouette aspect ratios must agree: directwrite {dw_ratio:.3}, swash {sw_ratio:.3}"
    );
    assert!(
        (dw_mean - sw_mean).abs() < 0.25,
        "silhouette fill ratios must agree: directwrite {dw_mean:.3}, swash {sw_mean:.3}"
    );
}

/// 验收：`Mono` 报告不再带跨后端差异，且不产生彩色图集条目。
#[test]
fn mono_report_has_no_cross_backend_deviation() {
    let (mut engine, mut cache) = cache_with(RasterBackend::default(), EmojiPresentation::Mono);
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("\u{1F680}", &style);
    let _ = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    let report = cache.report();
    assert!(
        !report.rgba_atlas_allocated,
        "the mono path must not allocate the RGBA atlas"
    );
    #[cfg(windows)]
    assert_eq!(
        report.fallback_count, 0,
        "the emoji font must not record a fallback in mono mode"
    );
    // 非 Windows 平台常见 CBDT/CBLC 纯位图字体（如 Noto Color Emoji）缺少基字形轮廓，
    // Mono 模式下回退至占位框并记账属于预期行为。
    #[cfg(not(windows))]
    if report.fallback_count > 0 {
        assert!(
            report
                .last_fallback_reason
                .as_deref()
                .is_some_and(|r| r.contains("no base outline")),
            "unexpected fallback in mono mode: {:?}",
            report.last_fallback_reason
        );
    }
    // 只有「无 swash 构建 + 显式选 Swash 回退」才允许有已知差异。
    if cfg!(feature = "backend-swash") {
        assert_eq!(report.known_deviation, None);
    }
    assert!(
        report.emoji_path == TextRenderReport::EMOJI_PATH_SWASH_MONO
            || report.emoji_path == TextRenderReport::EMOJI_PATH_DIRECTWRITE_MONO,
        "unexpected mono path: {}",
        report.emoji_path
    );
    assert_eq!(cache.settings().emoji_backend, EmojiBackend::Follow);
}

/// 验收：`Color` → `Mono` 的回退与直接选 `Mono` 得到同一张图。
///
/// 用 `UNBOUND_TEXT_FORCE_DW_INIT_FAIL` 之外的确定性方式构造回退：直接比较
/// 「`Mono` 呈现 + `EmojiFallback::Mono`」与「`Mono` 呈现 + 默认回退」两条设置，
/// 只要 `Mono` 不依赖彩色层，两者必然一致——它们执行的是同一条光栅路径。
#[test]
fn mono_bitmap_is_independent_of_emoji_fallback_policy() {
    use unbound_text::EmojiFallback;

    let render = |fallback: EmojiFallback| {
        let mut engine = TextEngine::new();
        let mut cache = GlyphCache::new();
        let mut settings = cache.settings().clone();
        settings.emoji_presentation = EmojiPresentation::Mono;
        settings.emoji_fallback = fallback;
        cache.apply_settings(&settings, Some(&mut engine));
        let style = TextStyle {
            font_size: 32.0,
            ..Default::default()
        };
        let shaped = engine.shape("\u{1F5A4}", &style);
        let _ = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
        cache
            .take_updates()
            .into_iter()
            .find(|u| matches!(u.target, AtlasTarget::Coverage))
            .expect("mono path must upload coverage")
            .data
    };

    assert_eq!(
        render(EmojiFallback::Swash),
        render(EmojiFallback::Mono),
        "the mono bitmap must not depend on the color fallback policy"
    );
}

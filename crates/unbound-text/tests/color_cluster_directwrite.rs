//! 彩色簇层合成的端到端断言。
//!
//! 需要 Windows + `backend-directwrite` + 系统装有 Segoe UI Emoji；
//! 其他环境整体跳过（`cfg` 在文件级）。
//!
//! 只断言外部可观察结果：四边形数量、是否落 RGBA 图集、相位是否为 0、
//! 位图的色相占比、回退计数。不看内部字段。

#![cfg(all(windows, feature = "backend-directwrite"))]

use unbound_math::Vec2;
use unbound_text::{
    AtlasTarget, AtlasUpdate, EmojiBackend, EmojiPresentation, GlyphCache, TextEngine,
    TextRenderReport, TextStyle,
};

fn color_directwrite_cache() -> (TextEngine, GlyphCache) {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut settings = cache.settings().clone();
    settings.emoji_presentation = EmojiPresentation::Color;
    settings.emoji_backend = EmojiBackend::DirectWrite;
    cache.apply_settings(&settings, Some(&mut engine));
    (engine, cache)
}

/// 取出唯一的 RGBA 上传位图。
fn rgba_bitmap(updates: &[AtlasUpdate]) -> &[u8] {
    &updates
        .iter()
        .find(|u| matches!(u.target, AtlasTarget::Color))
        .expect("a color cluster must produce one RGBA upload")
        .data
}

/// 不透明像素里带真实色相的比例。
///
/// 彩色路径要证明的是「真的上了色」，而不是把同一簇画成单色剪影——
/// 剪影的 RGB 三通道相等，色相占比接近 0。
fn chroma_ratio(data: &[u8]) -> f32 {
    let mut opaque = 0usize;
    let mut colored = 0usize;
    for px in data.chunks_exact(4) {
        if px[3] < 32 {
            continue;
        }
        opaque += 1;
        let max = px[0].max(px[1]).max(px[2]);
        let min = px[0].min(px[1]).min(px[2]);
        if max - min > 8 {
            colored += 1;
        }
    }
    if opaque == 0 {
        0.0
    } else {
        colored as f32 / opaque as f32
    }
}

#[test]
fn emoji_cluster_emits_single_colored_quad() {
    let (mut engine, mut cache) = color_directwrite_cache();
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("😀", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    assert_eq!(
        quads.len(),
        1,
        "one color cluster must emit exactly one quad"
    );
    let quad = quads[0];
    assert!(
        quad.rect.size.x > 0.0 && quad.rect.size.y > 0.0,
        "a color cluster must have non-zero size"
    );
    assert!(quad.color, "a color cluster must land in the RGBA atlas");
    assert_eq!(quad.phase, 0, "color clusters lock phase 0");
    assert_eq!(
        cache.fallback_count(),
        0,
        "the default emoji font must not fall back"
    );

    let updates = cache.take_updates();
    let rgba = updates
        .iter()
        .find(|u| matches!(u.target, AtlasTarget::Color))
        .expect("a color cluster must produce one RGBA upload");
    let ratio = chroma_ratio(&rgba.data);
    assert!(
        ratio > 0.05,
        "color bitmap must carry real chroma; measured ratio {ratio}"
    );
}

/// 验收：报告要说真话——路径是 DirectWrite 层合成，且这一簇没记回退。
#[test]
fn color_path_reports_directwrite_layers() {
    let (mut engine, mut cache) = color_directwrite_cache();
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("😀", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
    assert_eq!(quads.len(), 1);

    let report = cache.report();
    assert_eq!(
        report.emoji_path,
        TextRenderReport::EMOJI_PATH_DIRECTWRITE_COLOR,
        "the report must name the layer path, not swash color"
    );
    assert_eq!(
        report.known_deviation, None,
        "the color path has no cross-backend deviation"
    );
    assert_eq!(
        report.fallback_count, 0,
        "compositing the emoji font must not fall back"
    );
    assert_eq!(report.last_fallback_reason, None);
    assert!(
        report.rgba_atlas_allocated,
        "a color cluster must allocate the RGBA atlas"
    );
}

/// 验收：色度统计。同一字形、同一后端，只切呈现模式。
///
/// 彩色路径要证明的是真的上色；单色路径写的是 R8 覆盖率，三个通道天然相等，
/// 所以那里的色相占比恒为 0——这正是「画成剪影」的定义。
#[test]
fn color_path_chroma_clearly_exceeds_mono_path() {
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };

    let (mut engine, mut cache) = color_directwrite_cache();
    let shaped = engine.shape("😀", &style);
    let _ = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
    let color_ratio = chroma_ratio(rgba_bitmap(&cache.take_updates()));

    let mut mono_engine = TextEngine::new();
    let mut mono_cache = GlyphCache::new();
    let mut settings = mono_cache.settings().clone();
    settings.emoji_presentation = EmojiPresentation::Mono;
    settings.emoji_backend = EmojiBackend::DirectWrite;
    mono_cache.apply_settings(&settings, Some(&mut mono_engine));
    let mono_shaped = mono_engine.shape("😀", &style);
    let _ = mono_cache.quads_for(&mut mono_engine, &mono_shaped, Vec2::ZERO, 400, false);
    let mono_updates = mono_cache.take_updates();
    assert!(
        mono_updates
            .iter()
            .all(|u| matches!(u.target, AtlasTarget::Coverage)),
        "the mono path may only write the R8 atlas"
    );

    const MONO_CHROMA_RATIO: f32 = 0.0;
    assert!(
        color_ratio > MONO_CHROMA_RATIO + 0.2,
        "color path must carry far more chroma than the mono silhouette; \
         color {color_ratio}, mono {MONO_CHROMA_RATIO}"
    );

    // 单色路径的已知差异必须写在报告里，面板不能把两个后端说成一致。
    let mono_report = mono_cache.report();
    assert_eq!(
        mono_report.emoji_path,
        TextRenderReport::EMOJI_PATH_DIRECTWRITE_MONO
    );
    assert_eq!(
        mono_report.known_deviation,
        Some(TextRenderReport::KNOWN_DEVIATION_SWASH_MONO.to_string())
    );
}

/// 验收：lean 构建冒烟。`--no-default-features --features backend-directwrite`
/// 里默认设置（不显式指定 Emoji 后端）也必须出彩色图元。
#[test]
fn default_settings_reach_color_path_in_lean_build() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    assert_eq!(
        cache.settings().emoji_presentation,
        EmojiPresentation::Color,
        "default presentation stays color"
    );

    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("🚀", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
    assert_eq!(quads.len(), 1, "one emoji cluster is one quad");
    assert!(quads[0].color, "the default path must land in RGBA");

    let report = cache.report();
    assert_eq!(
        report.emoji_path,
        TextRenderReport::EMOJI_PATH_DIRECTWRITE_COLOR
    );
    assert_eq!(
        report.fallback_count, 0,
        "the default path must not record a fallback"
    );
    assert!(
        chroma_ratio(rgba_bitmap(&cache.take_updates())) > 0.05,
        "the lean build must really paint, not draw a silhouette"
    );

    #[cfg(not(feature = "backend-swash"))]
    assert!(
        report.fallback_count == 0 && report.rgba_atlas_allocated,
        "this build has no swash, so the quad can only come from DirectWrite layers"
    );
}

#[test]
fn color_cluster_ignores_subpixel_phase() {
    let (mut engine, mut cache) = color_directwrite_cache();
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("😀", &style);

    let first = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
    assert_eq!(cache.take_updates().len(), 1);

    // 亚像素位移既不重建位图，也不改变图集条目。
    let shifted = cache.quads_for(&mut engine, &shaped, Vec2::new(0.3, 0.0), 400, false);
    assert!(
        cache.take_updates().is_empty(),
        "a subpixel shift must not rebuild the color bitmap"
    );
    assert_eq!(
        first[0].uv, shifted[0].uv,
        "the same cluster must hit the same atlas entry"
    );
    assert_eq!(
        first[0].rect.origin.x, shifted[0].rect.origin.x,
        "origin is rounded from pen.x, so a 0.3 px shift must not move it"
    );
}

#[test]
fn each_emoji_cluster_gets_its_own_atlas_entry() {
    let (mut engine, mut cache) = color_directwrite_cache();
    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("😀🚀", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    assert_eq!(quads.len(), 2, "two separate emoji are two clusters");
    assert_ne!(
        quads[0].uv, quads[1].uv,
        "two clusters must not share one atlas entry"
    );
    assert_eq!(cache.take_updates().len(), 2);
}

#[test]
fn mono_presentation_does_not_take_the_color_path() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut settings = cache.settings().clone();
    settings.emoji_presentation = EmojiPresentation::Mono;
    cache.apply_settings(&settings, Some(&mut engine));

    let style = TextStyle {
        font_size: 32.0,
        ..Default::default()
    };
    let shaped = engine.shape("😀", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    assert!(
        !quads[0].color,
        "mono presentation must not take the color path"
    );
    assert!(
        cache
            .take_updates()
            .iter()
            .all(|u| matches!(u.target, AtlasTarget::Coverage)),
        "mono presentation may only write the R8 atlas"
    );
}

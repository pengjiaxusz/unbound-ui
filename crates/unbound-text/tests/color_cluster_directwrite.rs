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
    AtlasTarget, EmojiBackend, EmojiPresentation, GlyphCache, TextEngine, TextStyle,
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

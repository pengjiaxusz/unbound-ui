//! 位图彩色层（CBDT / PNG / JPEG）的端到端断言，按系统字体可用性跳过。
//!
//! ## 夹具策略
//!
//! 不把字体塞进仓库，也不假设 CI 装了它。先用 DirectWrite 扫一遍系统字体
//! 集合，找声明支持 PNG / JPEG 层的家族：
//!
//! * 找到 →用它渲染一个 Emoji，断言落 RGBA8 图集且零回退；
//! * 找不到 → 打印跳过原因并返回。这是**按需跳过**，不是假通过：跳过的
//!   那一次没有任何断言被执行，输出里会明确写出为什么。
//!
//! 合成算法本身（over 顺序、直通 alpha、包围盒）由 `color_composite.rs` 的
//! `*_bitmap_*` 一组覆盖，那一组只喂像素、不依赖字体，因此在任何机器上都真跑。
//!
//! 本机（开发机）实测：`seguiemj.ttf` 全是 COLR 矢量层，`find_bitmap_color_family`
//! 返回 `None`，所以这一组在此处走的是跳过分支；位图分支的可测部分压在
//! `classify_layer_format` 的单元测试与 `bgra_premul_to_rgba` 的换算测试上。

use unbound_math::Vec2;
use unbound_text::{
    AtlasTarget, DirectWriteRasterizer, EmojiPresentation, GlyphCache, TextEngine, TextStyle,
};

/// 不透明像素里带真实色相的比例。位图层的颜色来自像素本身，不是实色染色。
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
fn bitmap_color_layer_composites_when_a_bitmap_font_is_installed() {
    let Some(rasterizer) = DirectWriteRasterizer::new() else {
        eprintln!("SKIP: DirectWrite is not available in this environment");
        return;
    };
    let Some(family) = rasterizer.find_bitmap_color_family() else {
        eprintln!(
            "SKIP: no bitmap color font (CBDT/PNG/JPEG) is installed; \
             the system fonts here only carry COLR vector layers"
        );
        return;
    };
    eprintln!("bitmap color font found: {family}");

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut settings = cache.settings().clone();
    settings.emoji_presentation = EmojiPresentation::Color;
    cache.apply_settings(&settings, Some(&mut engine));

    let style = TextStyle {
        font_size: 32.0,
        family: Some(family.clone()),
        ..Default::default()
    };
    let shaped = engine.shape("\u{1F600}", &style);
    if shaped.glyphs.is_empty() {
        eprintln!("SKIP: {family} does not cover U+1F600");
        return;
    }

    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
    assert!(!quads.is_empty(), "the bitmap cluster must emit a quad");
    assert!(
        quads.iter().any(|q| q.color),
        "a bitmap color layer must land in the RGBA atlas"
    );

    let updates = cache.take_updates();
    let rgba = updates
        .iter()
        .find(|u| matches!(u.target, AtlasTarget::Color))
        .expect("a bitmap color layer must upload RGBA");
    let ratio = chroma_ratio(&rgba.data);
    assert!(
        ratio > 0.05,
        "{family}: the composited bitmap must carry real chroma, measured {ratio}"
    );

    let report = cache.report();
    assert_eq!(
        report.fallback_count, 0,
        "{family}: a composable bitmap cluster must not record a fallback: {:?}",
        report.last_fallback_reason
    );
    assert!(report.rgba_atlas_allocated);
}

/// 探测本身要给出可读结论，而不是静默返回空。
///
/// 这条测试始终通过，但会打印本机的探测结果——`find_bitmap_color_family`
/// 失效时（例如永远返回 `None`）能从测试输出里直接看出来。
#[test]
fn bitmap_color_font_probe_reports_a_result() {
    let Some(rasterizer) = DirectWriteRasterizer::new() else {
        eprintln!("SKIP: DirectWrite is not available in this environment");
        return;
    };
    match rasterizer.find_bitmap_color_family() {
        Some(family) => eprintln!("bitmap color font: {family}"),
        None => eprintln!("bitmap color font: none (COLR-only system)"),
    }
}

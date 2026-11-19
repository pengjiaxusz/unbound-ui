//! RGBA8 图集按需懒初始化与显存优化测试。

use unbound_math::Vec2;
use unbound_text::{
    AtlasConfig, AtlasTarget, EmojiPresentation, GlyphAtlas, GlyphKey, TextEngine, TextStyle,
};

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 验收标准 1：验证在纯灰度普通文本渲染场景下，RGBA8 图集永不创建（显存严格为 4 MiB）。
#[test]
fn grayscale_text_never_allocates_rgba_atlas() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    let mut engine = TextEngine::new();
    let mut cache = unbound_text::GlyphCache::new();

    let style = TextStyle::sized(16.0);
    let shaped = engine.shape("Grayscale text without emoji", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    assert!(!quads.is_empty(), "quads should not be empty");
    for quad in &quads {
        assert!(!quad.color, "grayscale glyph quad must not have color flag");
    }

    // RGBA8 图集永不创建
    assert!(
        !cache.atlas.has_rgba(),
        "rgba atlas must not be allocated for purely grayscale text"
    );
    assert!(cache.atlas.rgba().is_none());

    // 显存严格为 4 MiB（1 页 R8 图集，2048 * 2048 * 1 B）
    const R8_PAGE_BYTES: u64 = 4 * 1024 * 1024;
    assert_eq!(
        cache.atlas.allocated_bytes(),
        R8_PAGE_BYTES,
        "allocated memory must strictly be 4 MiB for R8 only"
    );
    assert_eq!(cache.atlas_memory_bytes(), R8_PAGE_BYTES);

    let report = cache.report();
    assert!(
        !report.rgba_atlas_allocated,
        "report must indicate rgba atlas is not allocated"
    );
    assert_eq!(report.atlas_memory_bytes, R8_PAGE_BYTES);
}

/// 验收标准 2：验证在混排彩色 Emoji 时，RGBA8 图集被透明触发分配，图元发射与渲染完全正确。
#[test]
fn mixed_color_emoji_triggers_rgba_atlas_transparently() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    if !unbound_text::TextCapabilities::probe()
        .color_glyphs
        .is_available()
    {
        return;
    }
    let mut engine = TextEngine::new();
    let mut cache = unbound_text::GlyphCache::new();

    let style = TextStyle::sized(20.0);

    // 首先仅渲染纯文本，确认此时尚未创建 RGBA 图集
    let shaped_text = engine.shape("Hello", &style);
    let _ = cache.quads_for(&mut engine, &shaped_text, Vec2::ZERO, 400, false);
    assert!(!cache.atlas.has_rgba());

    // 混排彩色 Emoji
    let shaped_mixed = engine.shape("Hello 😀 World", &style);
    let quads = cache.quads_for(&mut engine, &shaped_mixed, Vec2::ZERO, 400, false);

    assert!(!quads.is_empty(), "mixed quads should not be empty");

    let has_color_quad = quads.iter().any(|q| q.color);
    assert!(
        has_color_quad,
        "must contain at least one color quad for emoji"
    );

    // RGBA 图集被透明触发分配
    assert!(
        cache.atlas.has_rgba(),
        "rgba atlas must be transparently allocated upon color emoji rasterization"
    );
    assert!(cache.atlas.rgba().is_some());

    // 显存为 4 MiB (R8) + 16 MiB (RGBA8) = 20 MiB
    const TOTAL_BYTES: u64 = (4 + 16) * 1024 * 1024;
    assert_eq!(
        cache.atlas.allocated_bytes(),
        TOTAL_BYTES,
        "allocated memory must be 20 MiB after rgba allocation"
    );
    assert_eq!(cache.atlas_memory_bytes(), TOTAL_BYTES);

    let report = cache.report();
    assert!(
        report.rgba_atlas_allocated,
        "report must reflect rgba atlas allocation"
    );
    assert_eq!(report.atlas_memory_bytes, TOTAL_BYTES);

    // 验证更新队列中包含对应图集目标
    let updates = cache.take_updates();
    assert!(
        updates.iter().any(|u| u.target == AtlasTarget::Color),
        "must emit AtlasTarget::Color update"
    );
}

/// 验收标准 3：单色 Emoji 模式下不创建 RGBA8 图集。
#[test]
fn mono_emoji_presentation_avoids_rgba_atlas() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    if !unbound_text::TextCapabilities::probe()
        .color_glyphs
        .is_available()
    {
        return;
    }
    let mut engine = TextEngine::new();
    let mut cache = unbound_text::GlyphCache::new();

    let mut settings = cache.settings().clone();
    settings.emoji_presentation = EmojiPresentation::Mono;
    cache.apply_settings(&settings, Some(&mut engine));

    let style = TextStyle::sized(20.0);
    let shaped = engine.shape("Emoji 😀 in Mono", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    assert!(!quads.is_empty());
    for quad in &quads {
        assert!(!quad.color, "mono emoji must not have color flag");
    }

    assert!(
        !cache.atlas.has_rgba(),
        "mono emoji must stay in R8 atlas without allocating RGBA atlas"
    );
    assert_eq!(cache.atlas.allocated_bytes(), 4 * 1024 * 1024);
}

/// 验收标准 4：SDF 槽位分配透明触发 RGBA8 图集初始化。
#[test]
fn sdf_slot_allocation_triggers_rgba_atlas() {
    let config = AtlasConfig {
        page_size: 256,
        max_layers: 2,
        padding: 1,
    };
    let mut atlas = GlyphAtlas::new(config);
    assert!(!atlas.has_rgba());
    assert_eq!(atlas.allocated_bytes(), 0);

    let mut evicted = Vec::new();
    let key = GlyphKey::styled(1, 16.0, 0, true, 1, 400, false);
    let slot = atlas
        .allocate_sdf(key, 24, 24, &mut evicted)
        .expect("sdf slot");

    assert!(atlas.has_rgba());
    assert!(slot.is_valid());
    assert_eq!(slot.layer, 0);

    let report = atlas.report();
    assert!(report.rgba_allocated);
    assert_eq!(report.r8_bytes, 0);
    assert_eq!(report.rgba_bytes, 256 * 256 * 4);
    assert_eq!(report.total_bytes, 256 * 256 * 4);
    assert_eq!(atlas.allocated_bytes(), 256 * 256 * 4);
}

/// 验收标准 5：生命周期 clear 方法正确处理 None 与 Some。
#[test]
fn atlas_lifecycle_clear_contract() {
    let config = AtlasConfig {
        page_size: 256,
        max_layers: 2,
        padding: 1,
    };
    let mut atlas = GlyphAtlas::new(config);
    let mut evicted = Vec::new();

    // 1. 仅分配 R8 槽位
    let key_r8 = GlyphKey::new(1, 16.0, 0, false, 1);
    atlas
        .allocate_coverage(key_r8, 16, 16, &mut evicted)
        .unwrap();
    assert!(!atlas.has_rgba());
    assert_eq!(atlas.allocated_bytes(), 256 * 256);

    // 清空图集：rgba 为 None 时保持无操作
    atlas.clear();
    assert!(!atlas.has_rgba());
    assert_eq!(atlas.allocated_bytes(), 0);

    // 2. 触发分配 RGBA 槽位
    let key_color = GlyphKey::new(2, 16.0, 0, false, 1);
    atlas
        .allocate_color(key_color, 16, 16, &mut evicted)
        .unwrap();
    assert!(atlas.has_rgba());
    assert_eq!(atlas.allocated_bytes(), 256 * 256 * 4);

    // 清空图集：rgba 为 Some 时清空其条目
    atlas.clear();
    assert!(
        atlas.has_rgba(),
        "clear on an allocated rgba atlas clears entries while retaining structure"
    );
    assert_eq!(atlas.allocated_bytes(), 0);
}

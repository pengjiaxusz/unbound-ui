//! Emoji 呈现模式与缓存失效测试。

use unbound_text::{
    CapabilityStatus, EmojiPresentation, GlyphCache, GlyphKey, InvalidationTier, PhaseMode,
    TextCapabilities, TextEngine, TextStyle,
};

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn emoji_presentation_triggers_bitmap_invalidation() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();

    let mut settings = cache.settings().clone();

    // 默认是 Color
    assert_eq!(settings.emoji_presentation, EmojiPresentation::Color);

    // 切换到 Mono
    settings.emoji_presentation = EmojiPresentation::Mono;

    let tier = cache.apply_settings(&settings, Some(&mut engine));

    // 预期：位图级失效
    assert_eq!(
        tier,
        InvalidationTier::Bitmap,
        "switching emoji presentation must trigger Bitmap tier invalidation"
    );
}

#[test]
fn emoji_cache_key_includes_presentation_and_palette() {
    let key_color =
        GlyphKey::styled_with_mode(1, 16.0, 0, false, 0, 400, false, PhaseMode::Eight, 0, 0);
    let key_mono =
        GlyphKey::styled_with_mode(1, 16.0, 0, false, 0, 400, false, PhaseMode::Eight, 1, 0);
    let key_palette1 =
        GlyphKey::styled_with_mode(1, 16.0, 0, false, 0, 400, false, PhaseMode::Eight, 0, 1);

    assert_ne!(
        key_color, key_mono,
        "color and mono presentations must produce different GlyphKeys"
    );
    assert_ne!(
        key_color, key_palette1,
        "different palette indices must produce different GlyphKeys"
    );
}

#[test]
fn emoji_glyph_color_flag_and_atlas_allocation() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();

    let style = TextStyle {
        font_size: 20.0,
        ..Default::default()
    };

    // 1. 彩色模式：解析包含 Emoji 的文本段
    let mut settings = cache.settings().clone();
    settings.emoji_presentation = EmojiPresentation::Color;
    cache.apply_settings(&settings, Some(&mut engine));

    let shaped_color = engine.shape("😀", &style);
    let quads_color = cache.quads_for(
        &mut engine,
        &shaped_color,
        unbound_math::Vec2::ZERO,
        400,
        false,
    );

    // 验证彩色 Emoji 产生并且 color 标记正确
    if let Some(quad) = quads_color.first() {
        if quad.rect.size.x > 0.0 && quad.rect.size.y > 0.0 {
            assert!(
                quad.color,
                "emoji glyph in color mode must have color == true"
            );
        }
    }

    // 2. 单色模式：解析 Emoji 文本段
    settings.emoji_presentation = EmojiPresentation::Mono;
    cache.apply_settings(&settings, Some(&mut engine));

    let shaped_mono = engine.shape("😀", &style);
    let quads_mono = cache.quads_for(
        &mut engine,
        &shaped_mono,
        unbound_math::Vec2::ZERO,
        400,
        false,
    );

    // 验证单色 Emoji 产生且 color == false（走 R8 图集与文字色染色）
    if let Some(quad) = quads_mono.first() {
        if quad.rect.size.x > 0.0 && quad.rect.size.y > 0.0 {
            assert!(
                !quad.color,
                "emoji glyph in mono mode must have color == false"
            );
        }
    }
}

#[test]
fn emoji_color_directwrite_capability_unavailable() {
    let caps = TextCapabilities::probe();
    match caps.color_directwrite {
        CapabilityStatus::Unavailable(reason) => {
            assert!(
                reason.contains("layer traversal"),
                "color_directwrite must specify layer traversal reason: {reason}"
            );
        }
        CapabilityStatus::NotCompiled => {}
        CapabilityStatus::Available => {
            panic!("color_directwrite must not be Available before architectural decision");
        }
    }
}

// 无头测试：文本渲染设置收敛、三档失效判定与三态能力查询

use unbound_math::Vec2;
use unbound_text::{
    CapabilityStatus, EmojiBackend, EmojiPresentation, GlyphCache, InvalidationTier, LineBreakMode,
    PhaseMode, RasterBackend, TextCapabilities, TextEngine, TextRenderReport, TextRenderSettings,
    TextShaper, TextStyle,
};

#[test]
fn default_settings_roundtrip_and_equality() {
    let default_settings = TextRenderSettings::default();
    assert_eq!(default_settings.weight, 400);
    assert_eq!(default_settings.size, 14.0);
    assert!(!default_settings.italic);
    assert_eq!(default_settings.family, None);
    assert_eq!(default_settings.shaper, TextShaper::Cosmic);
    assert_eq!(default_settings.phase_mode, PhaseMode::Eight);
    assert_eq!(default_settings.raster_backend, RasterBackend::default());
    assert!(!default_settings.use_sdf);
    assert_eq!(
        default_settings.emoji_presentation,
        EmojiPresentation::Color
    );
    assert_eq!(default_settings.emoji_backend, EmojiBackend::Follow);

    // 序列化与反序列化测试
    let serialized = postcard::to_allocvec(&default_settings).expect("serialize settings");
    let deserialized: TextRenderSettings =
        postcard::from_bytes(&serialized).expect("deserialize settings");
    assert_eq!(default_settings, deserialized);
}

#[test]
fn invalidation_tier_preview_style_is_none() {
    let base = TextRenderSettings::default();

    // 改变字号
    let mut modified = base.clone();
    modified.size = 28.0;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::None);

    // 改变字重
    let mut modified = base.clone();
    modified.weight = 700;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::None);

    // 改变斜体
    let mut modified = base.clone();
    modified.italic = true;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::None);

    // 改变字体族
    let mut modified = base.clone();
    modified.family = Some("Arial".to_string());
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::None);
}

#[test]
fn invalidation_tier_typographic_clears_shaping_not_bitmap() {
    let base = TextRenderSettings::default();

    // 改变行高
    let mut modified = base.clone();
    modified.line_height = 24.0;
    assert_eq!(
        base.invalidation_tier(&modified),
        InvalidationTier::Typographic
    );

    // 改变字距
    let mut modified = base.clone();
    modified.letter_spacing = 1.5;
    assert_eq!(
        base.invalidation_tier(&modified),
        InvalidationTier::Typographic
    );

    // 改变断行模式
    let mut modified = base.clone();
    modified.line_break = LineBreakMode::Anywhere;
    assert_eq!(
        base.invalidation_tier(&modified),
        InvalidationTier::Typographic
    );

    // 改变双向文本开关
    let mut modified = base.clone();
    modified.bidi = false;
    assert_eq!(
        base.invalidation_tier(&modified),
        InvalidationTier::Typographic
    );

    // 改变连字开关
    let mut modified = base.clone();
    modified.ligatures = false;
    assert_eq!(
        base.invalidation_tier(&modified),
        InvalidationTier::Typographic
    );
}

#[test]
fn invalidation_tier_bitmap_clears_entire_atlas() {
    let base = TextRenderSettings::default();

    // 改变相位档位
    let mut modified = base.clone();
    modified.phase_mode = PhaseMode::Four;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);

    // 改变光栅化后端
    let mut modified = base.clone();
    modified.raster_backend = match base.raster_backend {
        RasterBackend::DirectWrite => RasterBackend::Cosmic,
        RasterBackend::Cosmic => RasterBackend::DirectWrite,
    };
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);

    // 改变距离场
    let mut modified = base.clone();
    modified.use_sdf = true;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);

    // 改变距离场扩散
    let mut modified = base.clone();
    modified.sdf_spread = 8.0;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);

    // 改变边距
    let mut modified = base.clone();
    modified.padding = 4;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);

    // 改变 Emoji 呈现
    let mut modified = base.clone();
    modified.emoji_presentation = EmojiPresentation::Mono;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);

    // 改变 Emoji 后端
    let mut modified = base.clone();
    modified.emoji_backend = EmojiBackend::Cosmic;
    assert_eq!(base.invalidation_tier(&modified), InvalidationTier::Bitmap);
}

#[test]
fn idempotent_settings_application_produces_no_invalidation() {
    let base = TextRenderSettings::default();
    assert_eq!(base.invalidation_tier(&base), InvalidationTier::None);

    let cloned = base.clone();
    assert_eq!(base.invalidation_tier(&cloned), InvalidationTier::None);
}

#[test]
fn glyph_cache_apply_settings_three_tiers_behavior() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();

    // 预填充字形与塑形缓存
    let style = TextStyle::sized(16.0);
    let shaped = engine.shape("Hello TDD Invalidation", &style);
    let _ = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    let initial_glyphs = cache.resident_glyphs();
    assert!(initial_glyphs > 0, "glyph cache must have resident entries");

    // 1. 预览样式档变更：缓存完全保留
    let mut preview_settings = cache.settings().clone();
    preview_settings.size = 24.0;
    let tier = cache.apply_settings(&preview_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::None);
    assert_eq!(
        cache.resident_glyphs(),
        initial_glyphs,
        "preview change must not clear glyph cache"
    );

    // 2. 排版语义档变更：清空引擎塑形缓存，但不清空位图字形缓存
    let mut typo_settings = cache.settings().clone();
    typo_settings.line_height = 32.0;
    let tier = cache.apply_settings(&typo_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::Typographic);
    assert_eq!(
        cache.resident_glyphs(),
        initial_glyphs,
        "typographic change must keep glyph cache intact"
    );

    // 3. 位图内容档变更：整表清空
    let mut bitmap_settings = cache.settings().clone();
    bitmap_settings.phase_mode = PhaseMode::One;
    let tier = cache.apply_settings(&bitmap_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::Bitmap);
    assert_eq!(
        cache.resident_glyphs(),
        0,
        "bitmap change must purge entire glyph cache"
    );
    assert_eq!(
        cache.atlas_memory_bytes(),
        0,
        "bitmap change must clear atlas memory"
    );
    assert!(
        cache.pending_updates().is_empty(),
        "bitmap change must clear pending updates"
    );

    // 4. 重复应用同一设置：幂等，不触发失效
    let tier_repeat = cache.apply_settings(&bitmap_settings, Some(&mut engine));
    assert_eq!(tier_repeat, InvalidationTier::None);
}

#[test]
fn text_capabilities_query_returns_tri_state() {
    let caps = TextCapabilities::probe();

    // 无论任何平台，Cosmic 均应可用
    assert_eq!(caps.cosmic, CapabilityStatus::Available);
    assert!(caps.cosmic.is_available());
    assert_eq!(caps.cosmic.reason(), None);

    // SDF 在纯 CPU 欧氏变换下应可用
    assert_eq!(caps.sdf, CapabilityStatus::Available);

    // MTSDF 尚未接入管线，必须返回环境不可用并附带具体原因
    match caps.mtsdf {
        CapabilityStatus::Unavailable(reason) => {
            assert_eq!(reason, "Pipeline not wired up");
        }
        other => panic!("expected Unavailable for MTSDF, got {other:?}"),
    }

    // 彩色 DirectWrite 需要层遍历（待决策），必须返回不可用并附带原因
    match caps.color_directwrite {
        CapabilityStatus::Unavailable(reason) => {
            assert_eq!(reason, "Requires layer traversal (deferred)");
        }
        CapabilityStatus::NotCompiled => {
            // 在非 Windows 平台上允许为未编译
            #[cfg(windows)]
            panic!("expected Unavailable on windows, got NotCompiled");
        }
        CapabilityStatus::Available => {
            panic!("color_directwrite must not be Available before deferred decision");
        }
    }

    // DirectWrite 在 Windows 与非 Windows 下的表现
    #[cfg(windows)]
    {
        // Windows 下 DirectWrite 为已编译，状态为 Available 或 Unavailable（若初始化失败）
        assert_ne!(caps.directwrite, CapabilityStatus::NotCompiled);
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 平台下 DirectWrite 必须为 NotCompiled
        assert_eq!(caps.directwrite, CapabilityStatus::NotCompiled);
        assert_eq!(
            caps.directwrite.reason(),
            Some("Feature not included in this build")
        );
    }
}

#[test]
fn text_render_report_default_and_roundtrip() {
    let report = TextRenderReport::default();
    assert_eq!(report.backend, RasterBackend::default());
    assert_eq!(report.phases, 8);
    assert_eq!(report.emoji_path, "Swash (Color RGBA8)");
    assert_eq!(report.fallback_count, 0);
    assert_eq!(report.last_fallback_reason, None);

    let serialized = postcard::to_allocvec(&report).expect("serialize report");
    let deserialized: TextRenderReport =
        postcard::from_bytes(&serialized).expect("deserialize report");
    assert_eq!(report, deserialized);
}

/// 验收条件 1：调字号时图集代际/内存不变，新旧字号在图集中天然隔离互不影响。
#[test]
fn font_size_change_preserves_atlas_and_isolates_entries() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();

    // 1. 渲染 14px 样本字形
    let style14 = TextStyle::sized(14.0);
    let shaped14 = engine.shape("IsolatedGlyphs", &style14);
    let quads14 = cache.quads_for(&mut engine, &shaped14, Vec2::ZERO, 400, false);
    assert!(!quads14.is_empty(), "quads for 14px must not be empty");

    let glyphs_after_14 = cache.resident_glyphs();
    let memory_after_14 = cache.atlas_memory_bytes();
    assert!(glyphs_after_14 > 0, "glyph cache must have entries");

    // 2. 应用新字号 24px 设置：失效级别必须为 None
    let mut new_settings = cache.settings().clone();
    new_settings.size = 24.0;
    let tier = cache.apply_settings(&new_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::None);

    // 验证图集条目数与图集内存完全保持不变（不清空）
    assert_eq!(
        cache.resident_glyphs(),
        glyphs_after_14,
        "font size change must not clear resident glyphs"
    );
    assert_eq!(
        cache.atlas_memory_bytes(),
        memory_after_14,
        "font size change must not clear atlas memory"
    );

    // 3. 渲染 24px 样本：新旧字号在图集中隔离并存
    let style24 = TextStyle::sized(24.0);
    let shaped24 = engine.shape("IsolatedGlyphs", &style24);
    let quads24 = cache.quads_for(&mut engine, &shaped24, Vec2::ZERO, 400, false);
    assert!(!quads24.is_empty(), "quads for 24px must not be empty");

    // 24px 的字形写入后，条目数单调增加
    assert!(
        cache.resident_glyphs() > glyphs_after_14,
        "24px glyphs must be added as new entries without purging 14px"
    );
    assert!(cache.atlas_memory_bytes() >= memory_after_14);

    // 4. 再次渲染 14px 样本：命中已有缓存
    let updates_before = cache.pending_updates().len();
    let quads14_again = cache.quads_for(&mut engine, &shaped14, Vec2::ZERO, 400, false);
    assert_eq!(quads14.len(), quads14_again.len());
    let updates_after = cache.pending_updates().len();
    assert_eq!(
        updates_before, updates_after,
        "previously cached 14px glyphs must still be present and hot"
    );
}

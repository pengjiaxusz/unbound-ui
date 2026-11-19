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

    // Cosmic 能力状态受 backend-swash 特性门控
    #[cfg(feature = "backend-swash")]
    {
        assert_eq!(caps.cosmic, CapabilityStatus::Available);
        assert!(caps.cosmic.is_available());
        assert_eq!(caps.cosmic.reason(), None);
    }
    #[cfg(not(feature = "backend-swash"))]
    {
        assert_eq!(caps.cosmic, CapabilityStatus::NotCompiled);
    }

    // SDF 在纯 CPU 欧氏变换下应可用
    assert_eq!(caps.sdf, CapabilityStatus::Available);

    // MTSDF 尚未接入管线，必须返回环境不可用并附带具体原因
    match caps.mtsdf {
        CapabilityStatus::Unavailable(reason) => {
            assert_eq!(reason, "Pipeline not wired up");
        }
        other => panic!("expected Unavailable for MTSDF, got {other:?}"),
    }

    // 彩色 DirectWrite 由层遍历支撑（ADR 0017）：编译了且工厂可用即可用。
    match caps.color_directwrite {
        CapabilityStatus::Available => {
            #[cfg(all(windows, feature = "backend-directwrite"))]
            assert_eq!(
                caps.directwrite,
                CapabilityStatus::Available,
                "color traversal only exists when DirectWrite itself is available"
            );
            assert!(
                caps.color_glyphs.is_available(),
                "an available color path must make color glyphs available"
            );
        }
        CapabilityStatus::Unavailable(reason) => {
            // 只允许因 DirectWrite 初始化失败而不可用，不能再是「待决策」。
            assert_eq!(reason, "DirectWrite initialization failed");
            #[cfg(all(windows, feature = "backend-directwrite"))]
            assert!(
                !caps.directwrite.is_available(),
                "color traversal must not be unavailable while DirectWrite works"
            );
        }
        CapabilityStatus::NotCompiled => {
            // 未开启 backend-directwrite 或在非 Windows 平台上允许为未编译
            #[cfg(all(windows, feature = "backend-directwrite"))]
            panic!("color traversal must be compiled on windows");
        }
    }

    // 彩色字形是「任一彩色路径可用」：swash 彩色或 DirectWrite 层合成。
    #[cfg(feature = "backend-swash")]
    assert_eq!(
        caps.color_glyphs,
        CapabilityStatus::Available,
        "swash color alone must make color glyphs available"
    );

    // DirectWrite 在启用与未启用特性下的表现
    #[cfg(all(windows, feature = "backend-directwrite"))]
    {
        // Windows 下 DirectWrite 为已编译，状态为 Available 或 Unavailable（若初始化失败）
        assert_ne!(caps.directwrite, CapabilityStatus::NotCompiled);
    }
    #[cfg(not(all(windows, feature = "backend-directwrite")))]
    {
        // 未开启 backend-directwrite 平台下 DirectWrite 必须为 NotCompiled
        assert_eq!(caps.directwrite, CapabilityStatus::NotCompiled);
    }
}

#[test]
fn text_render_report_default_and_roundtrip() {
    let report = TextRenderReport::default();
    assert_eq!(report.backend, RasterBackend::default());
    assert_eq!(report.phases, 8);
    assert_eq!(
        report.emoji_path,
        TextRenderReport::EMOJI_PATH_SWASH_COLOR,
        "the default report must use the shared constant, not a literal"
    );
    assert_eq!(report.known_deviation, None);
    assert_eq!(report.fallback_count, 0);
    assert_eq!(report.last_fallback_reason, None);
    assert!(!report.rgba_atlas_allocated);
    assert_eq!(report.atlas_memory_bytes, 0);

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

#[test]
fn typographic_ligatures_toggle_reshapes_without_invalidating_atlas() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();

    let mut style = TextStyle::sized(16.0);
    // 优先选用支持丰富连字的字体（如 Cascadia Code 编程连字或 Calibri 标准连字）
    let has_cascadia = engine
        .font_system_mut()
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == "Cascadia Code"));
    let has_calibri = engine
        .font_system_mut()
        .db()
        .faces()
        .any(|f| f.families.iter().any(|(name, _)| name == "Calibri"));
    if has_cascadia {
        style.family = Some("Cascadia Code".to_string());
    } else if has_calibri {
        style.family = Some("Calibri".to_string());
    }

    let sample = "==> != -> <= >= Office conflict and difficult flow";

    // 1. 默认设置开启连字 (ligatures = true)
    let initial_settings = cache.settings().clone();
    assert!(initial_settings.ligatures);
    let shaped_with_ligatures = engine.shape(sample, &style);
    let _ = cache.quads_for(&mut engine, &shaped_with_ligatures, Vec2::ZERO, 400, false);
    let initial_resident_glyphs = cache.resident_glyphs();
    assert!(initial_resident_glyphs > 0);

    // 2. 切换连字为 false
    let mut no_liga_settings = initial_settings.clone();
    no_liga_settings.ligatures = false;
    let tier = cache.apply_settings(&no_liga_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::Typographic);

    // 验证图集代际与位图缓存完全保留
    assert_eq!(
        cache.resident_glyphs(),
        initial_resident_glyphs,
        "typographic change must not clear glyph cache"
    );

    // 3. 再次塑形，连字关闭后文本重排生效（独立字形产生，字形数量或坐标改变）
    let shaped_without_ligatures = engine.shape(sample, &style);
    assert!(
        shaped_without_ligatures.glyphs.len() >= shaped_with_ligatures.glyphs.len(),
        "disabling ligatures should produce at least as many or more individual glyphs"
    );
    // 且塑形结果必须与开连字时有明确差异（文本重排生效）
    assert_ne!(
        shaped_with_ligatures.glyphs, shaped_without_ligatures.glyphs,
        "glyphs must differ when ligatures are toggled"
    );
}

#[test]
fn typographic_line_break_and_bidi_toggle() {
    let mut engine = TextEngine::new();
    let style = TextStyle::sized(14.0);

    // 1. 测试断行模式 LineBreakMode::None / Anywhere / Word
    let wrap_text =
        "Supercalifragilisticexpialidocious text with long words wrapping around container";
    let max_w = 120.0;

    // Word 模式：长词换行
    let word_settings = TextRenderSettings {
        line_break: LineBreakMode::Word,
        ..Default::default()
    };
    engine.apply_settings(&TextRenderSettings::default(), &word_settings);
    let wrapped_word = engine.wrap(wrap_text, &style, max_w);
    assert!(wrapped_word.lines.len() > 1);

    // None 模式：强制不换行，仅有 1 行
    let none_settings = TextRenderSettings {
        line_break: LineBreakMode::None,
        ..Default::default()
    };
    engine.apply_settings(&word_settings, &none_settings);
    let wrapped_none = engine.wrap(wrap_text, &style, max_w);
    assert_eq!(
        wrapped_none.lines.len(),
        1,
        "LineBreakMode::None must not break into multiple lines"
    );

    // Anywhere 模式：允许在任意字符截断换行
    let anywhere_settings = TextRenderSettings {
        line_break: LineBreakMode::Anywhere,
        ..Default::default()
    };
    engine.apply_settings(&none_settings, &anywhere_settings);
    let wrapped_anywhere = engine.wrap(wrap_text, &style, max_w);
    assert!(
        wrapped_anywhere.lines.len() >= wrapped_word.lines.len(),
        "Anywhere mode should break at least as eagerly as Word mode"
    );

    // 2. 测试双向文本 BiDi 开关
    let bidi_text = "Hello שלום World"; // lang-ok: Hebrew sample text for BiDi test
    let bidi_on_settings = TextRenderSettings {
        bidi: true,
        ..Default::default()
    };
    engine.apply_settings(&anywhere_settings, &bidi_on_settings);
    let shaped_bidi_on = engine.shape(bidi_text, &style);
    let has_rtl = shaped_bidi_on.glyphs.iter().any(|g| g.is_rtl());
    assert!(has_rtl, "RTL text with bidi=true must contain RTL glyphs");

    let bidi_off_settings = TextRenderSettings {
        bidi: false,
        ..Default::default()
    };
    engine.apply_settings(&bidi_on_settings, &bidi_off_settings);
    let shaped_bidi_off = engine.shape(bidi_text, &style);
    // 禁用双向重排后，所有字形 level 均为 0（即非 RTL 倒序，纯从左往右）
    assert!(
        shaped_bidi_off.glyphs.iter().all(|g| !g.is_rtl()),
        "bidi=false must not produce RTL glyph levels"
    );
    assert_ne!(
        shaped_bidi_on.glyphs, shaped_bidi_off.glyphs,
        "glyphs arrangement must differ between bidi on and off"
    );
}

#[test]
fn typographic_letter_spacing_and_line_height() {
    let mut engine = TextEngine::new();
    let style = TextStyle::sized(16.0);
    let sample = "Letter spacing and line height adjustments";

    // 1. 测试字距 letter_spacing
    let default_settings = TextRenderSettings {
        letter_spacing: 0.0,
        ..Default::default()
    };
    engine.apply_settings(&TextRenderSettings::default(), &default_settings);
    let normal = engine.shape(sample, &style);

    let spaced_settings = TextRenderSettings {
        letter_spacing: 2.0,
        ..Default::default()
    };
    engine.apply_settings(&default_settings, &spaced_settings);
    let spaced = engine.shape(sample, &style);

    assert!(
        spaced.width > normal.width,
        "positive letter spacing must increase total text width"
    );

    // 2. 测试行高 line_height
    let line_height_settings = TextRenderSettings {
        line_height: 36.0,
        ..Default::default()
    };
    engine.apply_settings(&spaced_settings, &line_height_settings);
    let tall_line = engine.shape(sample, &style);
    assert_eq!(tall_line.line_height, 36.0);
    assert!(tall_line.height >= 36.0);
}

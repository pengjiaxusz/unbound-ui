//! 无头测试：亚像素相位档位运行期切换（8 / 4 / 1）、图集键联动与整表清空。

use rustc_hash::FxHashSet;
use unbound_math::Vec2;
use unbound_text::{GlyphCache, GlyphKey, InvalidationTier, PhaseMode, TextEngine, TextStyle};

/// 验收条件 1：测试 PhaseMode 的量化函数、掩码与相位步长。
#[test]
fn test_phase_mode_quantization_and_masks() {
    // 1. 8 相位档位（掩码 0b111）
    let mode_8 = PhaseMode::Eight;
    assert_eq!(mode_8.phases(), 8);
    assert_eq!(mode_8.mask(), 0b111);
    assert!(!mode_8.snap_phases());

    // 测试 0.125 步长采样
    for i in 0..8 {
        let x = 10.0 + (i as f32 * 0.125);
        let (base_x, phase) = mode_8.quantize_x(x);
        assert_eq!(base_x, 10.0);
        assert_eq!(phase, i as u8);
        assert!((mode_8.phase_offset(phase) - i as f32 * 0.125).abs() < 1e-5);
    }

    // 2. 4 相位档位（掩码 0b11）
    let mode_4 = PhaseMode::Four;
    assert_eq!(mode_4.phases(), 4);
    assert_eq!(mode_4.mask(), 0b11);
    assert!(!mode_4.snap_phases());

    // 测试 0.25 步长采样
    for i in 0..4 {
        let x = 10.0 + (i as f32 * 0.25);
        let (base_x, phase) = mode_4.quantize_x(x);
        assert_eq!(base_x, 10.0);
        assert_eq!(phase, i as u8);
        assert!((mode_4.phase_offset(phase) - i as f32 * 0.25).abs() < 1e-5);
    }

    // 3. 1 相位档位（固定相位 0，掩码 0）
    let mode_1 = PhaseMode::One;
    assert_eq!(mode_1.phases(), 1);
    assert_eq!(mode_1.mask(), 0);
    assert!(mode_1.snap_phases());

    for i in 0..10 {
        let x = 10.0 + (i as f32 * 0.1);
        let (base_x, phase) = mode_1.quantize_x(x);
        assert_eq!(base_x, x.round());
        assert_eq!(phase, 0);
        assert_eq!(mode_1.phase_offset(phase), 0.0);
    }
}

/// 验收条件 1：断言档位 4 时同一字形最多产生 4 个不同图集键；档位 1 时只有 1 个；档位 8 最多 8 个。
#[test]
fn test_glyph_keys_count_by_phase_mode() {
    let mut engine = TextEngine::new();
    let text = "A";
    let shaped = engine.shape(text, &TextStyle::sized(16.0));
    let glyph = &shaped.glyphs[0];
    let face = shaped.faces[glyph.face_index as usize];

    // 在 [0.0, 1.0) 范围内均匀采样 64 个细密画笔位置
    let sample_count = 64;
    let pen_xs: Vec<f32> = (0..sample_count)
        .map(|i| 100.0 + (i as f32 / sample_count as f32))
        .collect();

    // 1. 验证 8 档：产生至多 8 个不同图集键（覆盖 8 个相位）
    {
        let mut cache = GlyphCache::new();
        let mut settings = cache.settings().clone();
        settings.phase_mode = PhaseMode::Eight;
        cache.apply_settings(&settings, Some(&mut engine));

        let mut observed_keys = FxHashSet::default();
        for &pen_x in &pen_xs {
            let pen = Vec2::new(pen_x, 50.0);
            let resolved = cache
                .resolve(&mut engine, glyph, face, pen, 400, false)
                .expect("resolved glyph in PhaseMode::Eight");
            assert!(resolved.slot.is_valid());
            let phase = cache.phase_at(pen.x);
            assert!(phase < 8);

            let key = GlyphKey::styled_with_mode(
                glyph.glyph_id,
                glyph.font_size,
                phase,
                false,
                0,
                400,
                false,
                PhaseMode::Eight,
                0,
                0,
            );
            observed_keys.insert(key);
        }
        assert_eq!(
            observed_keys.len(),
            8,
            "PhaseMode::Eight must produce exactly 8 unique keys across [0, 1)"
        );
        assert_eq!(cache.resident_glyphs(), 8);
    }

    // 2. 验证 4 档：同一字形最多产生 4 个不同图集键
    {
        let mut cache = GlyphCache::new();
        let mut settings = cache.settings().clone();
        settings.phase_mode = PhaseMode::Four;
        cache.apply_settings(&settings, Some(&mut engine));

        let mut observed_keys = FxHashSet::default();
        for &pen_x in &pen_xs {
            let pen = Vec2::new(pen_x, 50.0);
            let resolved = cache
                .resolve(&mut engine, glyph, face, pen, 400, false)
                .expect("resolved glyph in PhaseMode::Four");
            assert!(resolved.slot.is_valid());
            let phase = cache.phase_at(pen.x);
            assert!(
                phase < 4,
                "phase under PhaseMode::Four must be in 0..4, got {phase}"
            );

            let key = GlyphKey::styled_with_mode(
                glyph.glyph_id,
                glyph.font_size,
                phase,
                false,
                0,
                400,
                false,
                PhaseMode::Four,
                0,
                0,
            );
            observed_keys.insert(key);
        }
        assert_eq!(
            observed_keys.len(),
            4,
            "PhaseMode::Four must produce at most 4 unique keys across [0, 1)"
        );
        assert_eq!(cache.resident_glyphs(), 4);
    }

    // 3. 验证 1 档：同一字形只有 1 个图集键（phase 固定为 0）
    {
        let mut cache = GlyphCache::new();
        let mut settings = cache.settings().clone();
        settings.phase_mode = PhaseMode::One;
        cache.apply_settings(&settings, Some(&mut engine));

        let mut observed_keys = FxHashSet::default();
        for &pen_x in &pen_xs {
            let pen = Vec2::new(pen_x, 50.0);
            let resolved = cache
                .resolve(&mut engine, glyph, face, pen, 400, false)
                .expect("resolved glyph in PhaseMode::One");
            assert!(resolved.slot.is_valid());
            let phase = cache.phase_at(pen.x);
            assert_eq!(phase, 0, "PhaseMode::One must always yield phase 0");

            let key = GlyphKey::styled_with_mode(
                glyph.glyph_id,
                glyph.font_size,
                phase,
                false,
                0,
                400,
                false,
                PhaseMode::One,
                0,
                0,
            );
            observed_keys.insert(key);
        }
        assert_eq!(
            observed_keys.len(),
            1,
            "PhaseMode::One must produce exactly 1 key regardless of pen position"
        );
        assert_eq!(cache.resident_glyphs(), 1);
    }
}

/// 验收条件 1：切换档位属于位图级失效，触发整表清空，无旧字形残留，整表重建。
#[test]
fn test_phase_mode_switch_clears_cache_and_rebuilds_cleanly() {
    let mut engine = TextEngine::new();
    let text = "Quantum Phase 8 -> 4 -> 1";
    let shaped = engine.shape(text, &TextStyle::sized(14.0));

    let mut cache = GlyphCache::new();
    assert_eq!(cache.phase_mode(), PhaseMode::Eight);

    // 1. 在 8 档下解析
    let quads_8 = cache.quads_for(&mut engine, &shaped, Vec2::new(10.2, 20.3), 400, false);
    assert!(!quads_8.is_empty());
    assert!(cache.resident_glyphs() > 0);
    assert!(!cache.pending_updates().is_empty());
    let _ = cache.take_updates(); // 模拟上传排空
    assert_eq!(cache.report().phases, 8);

    // 2. 切换到 4 档：触发 Bitmap 失效，整表清空
    let mut settings_4 = cache.settings().clone();
    settings_4.phase_mode = PhaseMode::Four;
    let tier = cache.apply_settings(&settings_4, Some(&mut engine));
    assert_eq!(
        tier,
        InvalidationTier::Bitmap,
        "switching phase mode must trigger Bitmap invalidation"
    );

    // 断言缓存条目归零、无残留
    assert_eq!(
        cache.resident_glyphs(),
        0,
        "resident glyphs must be 0 after phase switch"
    );
    assert_eq!(
        cache.atlas_memory_bytes(),
        0,
        "atlas memory must be reset after phase switch"
    );
    assert!(
        cache.pending_updates().is_empty(),
        "updates must be empty immediately after phase switch"
    );
    assert_eq!(cache.phase_mode(), PhaseMode::Four);
    assert_eq!(cache.report().phases, 4);

    // 3. 重新解析，验证整表重建与待上传队列非空
    let quads_4 = cache.quads_for(&mut engine, &shaped, Vec2::new(10.2, 20.3), 400, false);
    assert!(!quads_4.is_empty());
    assert!(
        cache.resident_glyphs() > 0,
        "glyphs must be rebuilt in atlas"
    );
    assert!(
        !cache.pending_updates().is_empty(),
        "pending updates must be non-empty for rebuilt glyphs"
    );

    // 验证所有 4 档四边形的相位在 0..4 范围内
    for q in &quads_4 {
        assert!(
            q.phase < 4,
            "all quads in PhaseMode::Four must have phase < 4, got {}",
            q.phase
        );
    }

    // 4. 再次切换到 1 档
    let mut settings_1 = cache.settings().clone();
    settings_1.phase_mode = PhaseMode::One;
    let tier = cache.apply_settings(&settings_1, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::Bitmap);
    assert_eq!(cache.resident_glyphs(), 0);
    assert_eq!(cache.report().phases, 1);

    // 重新解析，验证所有四边形相位严格为 0
    let quads_1 = cache.quads_for(&mut engine, &shaped, Vec2::new(10.2, 20.3), 400, false);
    for q in &quads_1 {
        assert_eq!(
            q.phase, 0,
            "all quads in PhaseMode::One must have phase == 0, got {}",
            q.phase
        );
    }
}

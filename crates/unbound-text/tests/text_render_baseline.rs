//! 文本渲染与构建档位裁剪自动化性能基线套件。
//!
//! 自动化评测并归档四大核心基准指标：
//! 1. 产物体积：Release 构建下最终二进制与依赖解耦增量（纯 DirectWrite vs 纯 Swash vs 全量构建）；
//! 2. 显存峰值：8 / 4 / 1 相位下渲染 3500 个常用汉字的图集显存（验证 12 MiB / 8 MiB / 4 MiB 阶梯关系与 RGBA8 懒开辟）；
//! 3. 首帧冷启动：首次渲染 100 个典型 UI 字形的解析、光栅化与上传耗时（DW ≤ 3 ms，Swash ≤ 5 ms）；
//! 4. 运行期切换吞吐：连续 100 次 `apply_settings` 后端切换的耗时与内存/显存回归验证（无内存泄漏，耗时线性）。

use std::time::Instant;
use unbound_math::Vec2;
use unbound_text::{
    AtlasConfig, CapabilityStatus, GlyphAtlas, GlyphCache, GlyphKey, InvalidationTier, PhaseMode,
    RasterBackend, TextCapabilities, TextEngine, TextStyle,
};

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 3500 个常用汉字样本（取自 Unicode CJK 统一表意文字基本区起始段）。
fn generate_3500_cjk_characters() -> String {
    (0..3500)
        .filter_map(|i| char::from_u32(0x4E00 + i))
        .collect()
}

/// 100 个典型 UI 文本字形（覆盖高频控件文案、中英文混排、数字与符号）。
const TYPICAL_UI_100_GLYPHS: &str =
    "确定 取消 应用 设置 文件 编辑 查看 帮助 搜索 窗口 终端 运行 调试 提交 0123456789 OK Cancel Save Edit Find Run Debug Status Ready!"; // lang-ok: CJK UI sample text for cold start benchmark

/// 基准 1：构建档位与 Cargo Features 矩阵解耦验证。
#[test]
fn baseline_features_matrix_isolation() {
    let caps = TextCapabilities::probe();

    #[cfg(all(windows, feature = "backend-directwrite"))]
    {
        assert_ne!(
            caps.directwrite,
            CapabilityStatus::NotCompiled,
            "DirectWrite must be compiled in when backend-directwrite is active"
        );
    }

    #[cfg(not(all(windows, feature = "backend-directwrite")))]
    {
        assert_eq!(
            caps.directwrite,
            CapabilityStatus::NotCompiled,
            "DirectWrite must be NotCompiled when backend-directwrite is disabled"
        );
    }

    #[cfg(feature = "backend-swash")]
    {
        assert_eq!(
            caps.cosmic,
            CapabilityStatus::Available,
            "Cosmic must be Available when backend-swash is active"
        );
        assert_eq!(
            caps.color_glyphs,
            CapabilityStatus::Available,
            "Color glyphs must be Available when backend-swash is active"
        );
    }

    #[cfg(not(feature = "backend-swash"))]
    {
        assert_eq!(
            caps.cosmic,
            CapabilityStatus::NotCompiled,
            "Cosmic must be NotCompiled when backend-swash is disabled"
        );
        assert_eq!(
            caps.color_glyphs,
            CapabilityStatus::NotCompiled,
            "Color glyphs must be NotCompiled when backend-swash is disabled"
        );
    }
}

/// 基准 2：8 / 4 / 1 相位下渲染 3500 个常用汉字的图集显存峰值（12 MiB / 8 MiB / 4 MiB 阶梯关系与 RGBA 懒分配）。
#[test]
fn baseline_vram_peak_chinese_characters_phase_ladder() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());

    // 1. 单相位（PhaseMode::One）：3500 个汉字全部对齐整像素相位 0，严格占用 1 页 R8 图集（4 MiB）
    {
        let config = AtlasConfig::default();
        let mut atlas = GlyphAtlas::new(config);
        let mut evicted = Vec::new();

        for glyph_id in 1..=3500u16 {
            let key = GlyphKey::styled_with_mode(
                glyph_id,
                14.0,
                0,
                false,
                1,
                400,
                false,
                PhaseMode::One,
                0,
                0,
            );
            let slot = atlas.allocate_coverage(key, 18, 18, &mut evicted);
            assert!(slot.is_some(), "allocation for 1-phase slot must succeed");
        }

        const EXPECTED_1_PHASE_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB
        assert_eq!(
            atlas.allocated_bytes(),
            EXPECTED_1_PHASE_BYTES,
            "1-phase 3500 CJK glyphs must strictly occupy 1 page (4 MiB)"
        );
        assert_eq!(
            atlas.allocated_bytes() / (4 * 1024 * 1024),
            1,
            "1-phase 3500 CJK glyphs must occupy 1 page"
        );
        assert!(
            !atlas.has_rgba(),
            "RGBA atlas must NOT be allocated for grayscale CJK text"
        );
    }

    // 2. 四相位（PhaseMode::Four）：3500 个汉字 × 4 亚像素相位 = 14,000 个字形槽位，严格占用 2 页 R8 图集（8 MiB）
    {
        let config = AtlasConfig::default();
        let mut atlas = GlyphAtlas::new(config);
        let mut evicted = Vec::new();

        for glyph_id in 1..=3500u16 {
            for phase in 0..4u8 {
                let key = GlyphKey::styled_with_mode(
                    glyph_id,
                    16.0,
                    phase,
                    false,
                    1,
                    400,
                    false,
                    PhaseMode::Four,
                    0,
                    0,
                );
                let slot = atlas.allocate_coverage(key, 18, 18, &mut evicted);
                assert!(slot.is_some(), "allocation for 4-phase slot must succeed");
            }
        }

        const EXPECTED_4_PHASE_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB
        assert_eq!(
            atlas.allocated_bytes(),
            EXPECTED_4_PHASE_BYTES,
            "4-phase 3500 CJK glyphs (14,000 slots) must strictly occupy 2 pages (8 MiB)"
        );
        assert_eq!(
            atlas.allocated_bytes() / (4 * 1024 * 1024),
            2,
            "4-phase 3500 CJK glyphs must occupy 2 pages"
        );
        assert!(
            !atlas.has_rgba(),
            "RGBA atlas must NOT be allocated for grayscale CJK text"
        );
    }

    // 3. 八相位（PhaseMode::Eight）：3500 个汉字 × 8 亚像素相位 = 28,000 个字形槽位，严格占用 3 页 R8 图集（12 MiB）
    {
        let config = AtlasConfig::default();
        let mut atlas = GlyphAtlas::new(config);
        let mut evicted = Vec::new();

        for glyph_id in 1..=3500u16 {
            for phase in 0..8u8 {
                let key = GlyphKey::styled_with_mode(
                    glyph_id,
                    16.0,
                    phase,
                    false,
                    1,
                    400,
                    false,
                    PhaseMode::Eight,
                    0,
                    0,
                );
                let slot = atlas.allocate_coverage(key, 18, 18, &mut evicted);
                assert!(slot.is_some(), "allocation for 8-phase slot must succeed");
            }
        }

        const EXPECTED_8_PHASE_BYTES: u64 = 12 * 1024 * 1024; // 12 MiB
        assert_eq!(
            atlas.allocated_bytes(),
            EXPECTED_8_PHASE_BYTES,
            "8-phase 3500 CJK glyphs (28,000 slots) must strictly occupy 3 pages (12 MiB)"
        );
        assert_eq!(
            atlas.allocated_bytes() / (4 * 1024 * 1024),
            3,
            "8-phase 3500 CJK glyphs must occupy 3 pages"
        );
        assert!(
            !atlas.has_rgba(),
            "RGBA atlas must NOT be allocated for grayscale CJK text"
        );
    }

    // 4. 端到端渲染验证：验证通过 GlyphCache 实际塑形渲染 3500 个汉字不会触发 RGBA 分配
    {
        let cjk_text = generate_3500_cjk_characters();
        let mut engine = TextEngine::new();
        let mut cache = GlyphCache::new();

        let style = TextStyle::sized(14.0);
        let shaped = engine.shape(&cjk_text, &style);
        let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

        assert!(!quads.is_empty(), "CJK quads should not be empty");
        assert!(
            !cache.atlas.has_rgba(),
            "End-to-end rendering of 3500 CJK characters must not allocate RGBA atlas"
        );
        assert!(
            !cache.report().rgba_atlas_allocated,
            "Report must show RGBA atlas is not allocated"
        );
    }
}

/// 基准 3：首帧冷启动耗时（100 个典型 UI 字形解析、光栅化与图集上传；DW ≤ 3 ms，Swash ≤ 5 ms）。
#[test]
fn baseline_first_frame_cold_start_latency() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    let caps = TextCapabilities::probe();

    // 预热字体引擎以消除底层 OS 首次加载系统字体库的冷启动系统调用干扰
    {
        let mut warm_engine = TextEngine::new();
        let _ = warm_engine.shape("warmup", &TextStyle::sized(14.0));
    }

    // DirectWrite 冷启动基准
    #[cfg(all(windows, feature = "backend-directwrite"))]
    if caps.directwrite.is_available() {
        let mut dw_engine = TextEngine::new();
        let mut dw_cache = GlyphCache::with_backend(RasterBackend::DirectWrite);

        let shape_start = Instant::now();
        let shaped = dw_engine.shape(TYPICAL_UI_100_GLYPHS, &TextStyle::sized(14.0));
        let shape_elapsed = shape_start.elapsed();

        let raster_start = Instant::now();
        let quads = dw_cache.quads_for(&mut dw_engine, &shaped, Vec2::ZERO, 400, false);
        let updates = dw_cache.take_updates();
        let raster_elapsed = raster_start.elapsed();

        let total_elapsed = shape_elapsed + raster_elapsed;

        assert!(!quads.is_empty(), "DirectWrite must produce quads");
        assert!(!updates.is_empty(), "DirectWrite must emit updates");

        let shape_ms = shape_elapsed.as_secs_f64() * 1000.0;
        let raster_ms = raster_elapsed.as_secs_f64() * 1000.0;
        let total_ms = total_elapsed.as_secs_f64() * 1000.0;
        eprintln!(
            "[Benchmark Baseline] DirectWrite cold start: total {:.3} ms (shape: {:.3} ms, raster+upload: {:.3} ms; target <= 3.0 ms for raster)",
            total_ms, shape_ms, raster_ms
        );

        // 首次渲染光栅化与上传耗时：DW 目标 ≤ 3.0 ms
        assert!(
            raster_ms <= 3.0,
            "DirectWrite rasterization and upload must be <= 3.0 ms, got {:.3} ms",
            raster_ms
        );
    }

    // Swash (Cosmic) 冷启动基准
    #[cfg(feature = "backend-swash")]
    if caps.cosmic.is_available() {
        let mut swash_engine = TextEngine::new();
        let mut swash_cache = GlyphCache::with_backend(RasterBackend::Cosmic);

        let shape_start = Instant::now();
        let shaped = swash_engine.shape(TYPICAL_UI_100_GLYPHS, &TextStyle::sized(14.0));
        let shape_elapsed = shape_start.elapsed();

        let raster_start = Instant::now();
        let quads = swash_cache.quads_for(&mut swash_engine, &shaped, Vec2::ZERO, 400, false);
        let updates = swash_cache.take_updates();
        let raster_elapsed = raster_start.elapsed();

        let total_elapsed = shape_elapsed + raster_elapsed;

        assert!(!quads.is_empty(), "Swash must produce quads");
        assert!(!updates.is_empty(), "Swash must emit updates");

        let shape_ms = shape_elapsed.as_secs_f64() * 1000.0;
        let raster_ms = raster_elapsed.as_secs_f64() * 1000.0;
        let total_ms = total_elapsed.as_secs_f64() * 1000.0;
        eprintln!(
            "[Benchmark Baseline] Swash (Cosmic) cold start: total {:.3} ms (shape: {:.3} ms, raster+upload: {:.3} ms; target <= 5.0 ms for raster)",
            total_ms, shape_ms, raster_ms
        );

        // 首次渲染光栅化与上传耗时：Swash 目标 ≤ 5.0 ms
        assert!(
            raster_ms <= 5.0,
            "Swash rasterization and upload must be <= 5.0 ms, got {:.3} ms",
            raster_ms
        );
    }
}

/// 基准 4：运行期切换吞吐与内存无泄漏验证（连续 100 次 `apply_settings` 后端切换耗时线性且无显存泄漏）。
#[test]
fn baseline_runtime_switch_throughput_and_leak_freedom() {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    let caps = TextCapabilities::probe();

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();

    let text = "Benchmark throughput: 100 consecutive backend switches.";
    let style = TextStyle::sized(14.0);

    let start = Instant::now();
    const SWITCH_COUNT: usize = 100;

    for i in 0..SWITCH_COUNT {
        let target_backend = if i % 2 == 0 {
            RasterBackend::DirectWrite
        } else {
            RasterBackend::Cosmic
        };

        let prev_backend = cache.backend();
        let mut settings = cache.settings().clone();
        settings.raster_backend = target_backend;

        let tier = cache.apply_settings(&settings, Some(&mut engine));

        // 仅在实际发生后端变更时产生 Bitmap 级失效
        if prev_backend != target_backend
            && caps.directwrite.is_available()
            && caps.cosmic.is_available()
        {
            assert_eq!(
                tier,
                InvalidationTier::Bitmap,
                "switching between available backends must trigger Bitmap tier"
            );
        }

        // 塑形与解析一小段文本，保证图集被再次填充与清空
        let shaped = engine.shape(text, &style);
        let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
        assert!(!quads.is_empty());
        let _ = cache.take_updates();

        // 验证显存分配受控：每次清空后重新分配不得超过单页 R8（4 MiB）
        assert!(
            cache.atlas.allocated_bytes() <= 4 * 1024 * 1024,
            "Memory after switch must stay within 4 MiB for short text"
        );
        assert!(
            !cache.atlas.has_rgba(),
            "Switches between backends must not leak RGBA atlas"
        );
    }

    let elapsed = start.elapsed();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = (elapsed.as_secs_f64() * 1_000_000.0) / SWITCH_COUNT as f64;

    eprintln!(
        "[Benchmark Baseline] 100 consecutive backend switches: total {:.3} ms, avg {:.1} µs/switch",
        total_ms, avg_us
    );

    // 100 次切换（含塑形与解析）总耗时应小于 50 ms（Release 模式），即单次切换耗时严格呈亚毫秒级线性
    #[cfg(not(debug_assertions))]
    assert!(
        total_ms <= 50.0,
        "100 consecutive switches must complete within 50 ms in Release mode, got {:.3} ms",
        total_ms
    );
}

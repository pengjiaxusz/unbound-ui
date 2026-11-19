//! 光栅器运行期切换、整表重建与回退诊断的无头测试。

use unbound_math::Vec2;
use unbound_text::{
    AtlasTarget, GlyphCache, InvalidationTier, RasterBackend, TextEngine, TextStyle,
};

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn create_engine() -> TextEngine {
    TextEngine::new()
}

/// 验收条件 1、3：来回切换 20 次，无字形串味、无图集泄漏、图集页数不增长。
#[test]
fn switching_rasterizer_twenty_times_no_leak_no_growth() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let mut engine = create_engine();
    let mut cache = GlyphCache::new();

    let text = "The quick brown fox jumps over the lazy dog";
    let style = TextStyle::sized(14.0);

    let mut dw_settings = cache.settings().clone();
    dw_settings.raster_backend = RasterBackend::DirectWrite;

    let mut cosmic_settings = cache.settings().clone();
    cosmic_settings.raster_backend = RasterBackend::Cosmic;

    let mut initial_page_count = 0;

    for i in 0..20 {
        let target_backend = if i % 2 == 0 {
            RasterBackend::Cosmic
        } else {
            RasterBackend::DirectWrite
        };
        let target_settings = if i % 2 == 0 {
            &cosmic_settings
        } else {
            &dw_settings
        };

        // 1. 应用设置并断言失效层级为位图级（Bitmap）
        let tier = cache.apply_settings(target_settings, Some(&mut engine));
        assert_eq!(
            tier,
            InvalidationTier::Bitmap,
            "switching backend must trigger Bitmap tier invalidation"
        );

        // 2. 确认旧字形缓存条目已被彻底清空，无旧字形残留
        assert_eq!(
            cache.resident_glyphs(),
            0,
            "glyph cache must be empty immediately after switching"
        );

        // 3. 重新塑形并光栅化字形
        let shaped = engine.shape(text, &style);
        let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
        assert!(
            !quads.is_empty(),
            "quads must be generated for test specimen"
        );

        let updates = cache.take_updates();
        assert!(
            !updates.is_empty(),
            "updates must be generated after clearing"
        );

        // 4. 图集页数必须保持稳定，绝不随着切换次数累积增长
        let r8_pages = cache.atlas.r8.page_count();
        if i == 0 {
            initial_page_count = r8_pages;
        } else {
            assert_eq!(
                r8_pages, initial_page_count,
                "atlas r8 page count must not grow across switching iterations"
            );
        }

        // 5. 验证当前生效后端
        #[cfg(windows)]
        {
            let report = cache.report();
            assert_eq!(report.backend, target_backend);
        }
    }
}

/// 验收条件 1、3：切换后新一批位图覆盖率分布与目标后端一致（像素统计范式）。
#[cfg(windows)]
#[test]
fn switching_coverage_distribution_matches_target_backend() {
    let _guard = ENV_MUTEX.lock().unwrap();
    if std::env::var("UNBOUND_TEXT_BACKEND").is_ok_and(|v| v == "cosmic") {
        return;
    }

    let mut engine = create_engine();
    let mut cache = GlyphCache::new();

    let text = "Hamburgefonstiv AXVW 0123456789";
    let style = TextStyle::sized(16.0);

    // 1. 切换至 DirectWrite
    let mut dw_settings = cache.settings().clone();
    dw_settings.raster_backend = RasterBackend::DirectWrite;
    cache.apply_settings(&dw_settings, Some(&mut engine));

    let shaped_dw = engine.shape(text, &style);
    cache.quads_for(&mut engine, &shaped_dw, Vec2::new(0.25, 0.0), 400, false);

    let mut hist_dw = [0u32; 256];
    for u in cache.take_updates() {
        if u.target == AtlasTarget::Coverage {
            for &b in &u.data {
                hist_dw[b as usize] += 1;
            }
        }
    }
    let total_dw: u32 = hist_dw.iter().sum();
    assert!(total_dw > 0);
    let mid_dw: u32 = hist_dw[1..255].iter().sum();
    let ratio_dw = f64::from(mid_dw) / f64::from(total_dw);

    // 2. 切换至 Cosmic (Swash)
    let mut cosmic_settings = cache.settings().clone();
    cosmic_settings.raster_backend = RasterBackend::Cosmic;
    cache.apply_settings(&cosmic_settings, Some(&mut engine));

    let shaped_cosmic = engine.shape(text, &style);
    cache.quads_for(
        &mut engine,
        &shaped_cosmic,
        Vec2::new(0.25, 0.0),
        400,
        false,
    );

    let mut hist_cosmic = [0u32; 256];
    for u in cache.take_updates() {
        if u.target == AtlasTarget::Coverage {
            for &b in &u.data {
                hist_cosmic[b as usize] += 1;
            }
        }
    }
    let total_cosmic: u32 = hist_cosmic.iter().sum();
    assert!(total_cosmic > 0);
    let mid_cosmic: u32 = hist_cosmic[1..255].iter().sum();
    let ratio_cosmic = f64::from(mid_cosmic) / f64::from(total_cosmic);

    // DirectWrite 与 Swash 光栅化器在同一字串上的中间灰占比与总像素具有统计差异
    assert!(ratio_dw > 0.10, "DirectWrite coverage must be antialiased");
    assert!(ratio_cosmic > 0.10, "Cosmic coverage must be antialiased");
    assert_ne!(
        hist_dw, hist_cosmic,
        "coverage distribution must differ between backends"
    );
}

/// 验收条件 1、3：初始化失败静默回退 swash，回退单调递增且清空时不复位。
#[test]
fn directwrite_init_failure_fallback_and_monotonic_counter() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let mut engine = create_engine();
    let mut cache = GlyphCache::with_backend(RasterBackend::Cosmic);

    // 记录初始回退数
    let initial_fallback_count = cache.fallback_count();

    // 设置强制初始化失败环境变量
    std::env::set_var("UNBOUND_TEXT_FORCE_DW_INIT_FAIL", "1");

    let mut dw_settings = cache.settings().clone();
    dw_settings.raster_backend = RasterBackend::DirectWrite;

    let tier = cache.apply_settings(&dw_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::Bitmap);

    // 清理环境变量
    std::env::remove_var("UNBOUND_TEXT_FORCE_DW_INIT_FAIL");

    // 验证回退到 Cosmic
    let report = cache.report();
    assert_eq!(
        report.backend,
        RasterBackend::Cosmic,
        "must fallback to Cosmic backend on init failure"
    );
    assert_eq!(
        report.fallback_count,
        initial_fallback_count + 1,
        "fallback count must increment"
    );
    assert!(
        report.last_fallback_reason.is_some(),
        "last fallback reason must be recorded"
    );
    assert!(
        report
            .last_fallback_reason
            .as_ref()
            .unwrap()
            .contains("initialization failed"),
        "reason should explain initialization failure"
    );

    // 验证执行 clear 后计数保持不复位
    cache.clear();
    let after_clear_report = cache.report();
    assert_eq!(
        after_clear_report.fallback_count,
        initial_fallback_count + 1,
        "fallback counter must not reset upon clear"
    );
}

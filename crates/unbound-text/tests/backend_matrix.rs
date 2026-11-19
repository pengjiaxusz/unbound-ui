//! 构建矩阵与单后端编译隔离验证测试。

use unbound_math::Vec2;
use unbound_text::{
    CapabilityStatus, GlyphCache, RasterBackend, TextCapabilities, TextEngine, TextStyle,
};

/// 验证三态能力探测与当前启用的 Feature 矩阵严格对齐。
#[test]
fn single_backend_feature_matrix_capabilities_consistency() {
    let caps = TextCapabilities::probe();

    #[cfg(all(windows, feature = "backend-directwrite"))]
    {
        // 开启 backend-directwrite 时，Windows 下 DirectWrite 状态不应为 NotCompiled
        assert_ne!(
            caps.directwrite,
            CapabilityStatus::NotCompiled,
            "DirectWrite must be compiled in when backend-directwrite is active"
        );
    }

    #[cfg(not(all(windows, feature = "backend-directwrite")))]
    {
        // 未开启 backend-directwrite 时，DirectWrite 状态必须为 NotCompiled
        assert_eq!(
            caps.directwrite,
            CapabilityStatus::NotCompiled,
            "DirectWrite must be NotCompiled when backend-directwrite is not active"
        );
    }

    #[cfg(feature = "backend-swash")]
    {
        // 开启 backend-swash 时，Cosmic 状态必须为 Available，彩色字形亦可用
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
        // 未开启 backend-swash 时，Cosmic 状态必须为 NotCompiled，彩色字形亦未编译
        assert_eq!(
            caps.cosmic,
            CapabilityStatus::NotCompiled,
            "Cosmic must be NotCompiled when backend-swash is not active"
        );
        assert_eq!(
            caps.color_glyphs,
            CapabilityStatus::NotCompiled,
            "Color glyphs must be NotCompiled when backend-swash is not active"
        );
    }
}

/// 验证单后端编译下的字形光栅化与图集驻留冒烟测试。
#[test]
fn single_backend_glyph_rasterization_smoke() {
    let mut engine = TextEngine::new();
    let style = TextStyle::sized(16.0);
    let text = "Hello Unbound Matrix!";
    let shaped = engine.shape(text, &style);

    #[cfg(all(windows, feature = "backend-directwrite"))]
    {
        let mut cache = GlyphCache::with_backend(RasterBackend::DirectWrite);
        if cache.backend() == RasterBackend::DirectWrite {
            let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
            assert!(
                !quads.is_empty(),
                "DirectWrite backend must produce quads for shaped text"
            );
            let updates = cache.take_updates();
            assert!(
                !updates.is_empty(),
                "DirectWrite backend must generate atlas updates"
            );
        }
    }

    #[cfg(feature = "backend-swash")]
    {
        let mut cache = GlyphCache::with_backend(RasterBackend::Cosmic);
        if cache.backend() == RasterBackend::Cosmic {
            let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);
            assert!(
                !quads.is_empty(),
                "Cosmic backend must produce quads for shaped text"
            );
            let updates = cache.take_updates();
            assert!(
                !updates.is_empty(),
                "Cosmic backend must generate atlas updates"
            );
        }
    }
}

/// 验证切换至当前构建未编译的后端时安全回退且不崩溃。
#[test]
fn switching_to_uncompiled_backend_is_safe_and_records_fallback() {
    #[allow(unused_mut)]
    let mut cache = GlyphCache::new();

    #[cfg(not(all(windows, feature = "backend-directwrite")))]
    {
        // 当 DirectWrite 未编译时，请求切换至 DirectWrite 应安全回退到 Cosmic
        cache.set_backend(RasterBackend::DirectWrite);
        assert_eq!(
            cache.backend(),
            RasterBackend::Cosmic,
            "Must fall back to Cosmic when DirectWrite is not compiled"
        );
        assert!(
            cache.fallback_count() > 0,
            "Fallback count must increment when requesting uncompiled DirectWrite"
        );
    }

    #[cfg(not(feature = "backend-swash"))]
    {
        // 当 Swash 未编译时，请求切换至 Cosmic 应安全回退到 DirectWrite 并记录回退
        cache.set_backend(RasterBackend::Cosmic);
        assert_eq!(
            cache.backend(),
            RasterBackend::DirectWrite,
            "Must fall back to DirectWrite when Swash is not compiled"
        );
        assert!(
            cache.fallback_count() > 0,
            "Fallback count must increment when requesting uncompiled Cosmic"
        );
        assert!(
            cache
                .last_fallback_reason()
                .unwrap_or("")
                .contains("Cosmic (swash) backend is not compiled"),
            "Fallback reason must note that Cosmic is not compiled"
        );
    }

    let _ = cache;
}

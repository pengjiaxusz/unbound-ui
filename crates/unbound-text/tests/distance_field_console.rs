// 距离场与合成组及解除 DirectWrite 距离场绑定的无头集成测试。

use unbound_math::Vec2;
use unbound_text::{
    BlendSpace, CapabilityStatus, DistanceFieldMode, GlyphCache, InvalidationTier, LayerMode,
    PhaseMode, RasterBackend, TextCapabilities, TextEngine, TextRenderSettings, TextStyle,
};

#[test]
fn distance_field_modes_and_composition_settings_roundtrip() {
    let mut settings = TextRenderSettings::default();
    assert_eq!(settings.distance_field, DistanceFieldMode::Off);
    assert_eq!(settings.blend_space, BlendSpace::Srgb);
    assert_eq!(settings.layer_mode, LayerMode::Direct);
    assert!(!settings.use_sdf);

    // 修改距离场与合成组设置
    settings.distance_field = DistanceFieldMode::Sdf;
    settings.use_sdf = true;
    settings.blend_space = BlendSpace::Linear;
    settings.layer_mode = LayerMode::Isolated;

    let serialized = postcard::to_allocvec(&settings).expect("serialize settings");
    let deserialized: TextRenderSettings =
        postcard::from_bytes(&serialized).expect("deserialize settings");
    assert_eq!(settings, deserialized);
    assert_eq!(deserialized.distance_field, DistanceFieldMode::Sdf);
    assert_eq!(deserialized.blend_space, BlendSpace::Linear);
    assert_eq!(deserialized.layer_mode, LayerMode::Isolated);
}

#[test]
fn distance_field_active_locks_phase_to_one() {
    let mut settings = TextRenderSettings {
        phase_mode: PhaseMode::Eight,
        distance_field: DistanceFieldMode::Sdf,
        use_sdf: true,
        ..Default::default()
    };
    // 互斥规则生效：强制锁定相位为 1（整像素对齐）
    settings.enforce_constraints();
    assert_eq!(settings.phase_mode, PhaseMode::One);
    assert!(settings.snap_phases());

    // 验证 apply_settings 同样强制锁定 snap_phases 为 true
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut sdf_settings = cache.settings().clone();
    sdf_settings.phase_mode = PhaseMode::Four;
    sdf_settings.distance_field = DistanceFieldMode::Sdf;
    sdf_settings.use_sdf = true;

    let tier = cache.apply_settings(&sdf_settings, Some(&mut engine));
    assert_eq!(tier, InvalidationTier::Bitmap);
    assert_eq!(cache.settings().phase_mode, PhaseMode::One);
    assert!(cache.settings().snap_phases());
    #[allow(deprecated)]
    {
        assert!(cache.snap_phases);
    }
}

#[test]
fn mtsdf_capability_is_unavailable_with_correct_reason() {
    let caps = TextCapabilities::probe();
    assert_eq!(
        caps.mtsdf,
        CapabilityStatus::Unavailable("Pipeline not wired up")
    );
    assert_eq!(caps.mtsdf.reason(), Some("Pipeline not wired up"));
}

#[test]
fn directwrite_coverage_generates_distance_field() {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::with_backend(RasterBackend::DirectWrite);

    // 启用距离场
    let mut settings = cache.settings().clone();
    settings.raster_backend = RasterBackend::DirectWrite;
    settings.distance_field = DistanceFieldMode::Sdf;
    settings.use_sdf = true;
    settings.sdf_spread = 4.0;
    cache.apply_settings(&settings, Some(&mut engine));

    let style = TextStyle::sized(20.0);
    let shaped = engine.shape("A", &style);
    let quads = cache.quads_for(&mut engine, &shaped, Vec2::ZERO, 400, false);

    assert_eq!(quads.len(), 1, "must generate quad for glyph");
    let initial_resident = cache.resident_glyphs();
    assert_eq!(initial_resident, 1);

    // 检查待上传更新队列，SDF 产物必须送入 RGBA 图集（AtlasTarget::Color）
    let updates = cache.take_updates();
    assert!(!updates.is_empty(), "must have atlas updates");
    let has_color_update = updates
        .iter()
        .any(|u| u.target == unbound_text::AtlasTarget::Color);
    assert!(
        has_color_update,
        "SDF glyph must be allocated into color atlas (AtlasTarget::Color)"
    );
}

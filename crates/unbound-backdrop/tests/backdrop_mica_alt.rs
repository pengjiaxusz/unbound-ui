//! 云母变体（Mica Alt）的组合解析穷举测试。
//!
//! 云母变体与标准云母共享同一档门槛（Windows 11 22H2）与同一个自绘变体，
//! 但它向系统请求的是另一档材质值。这套测试把「材质种类 × 来源 × 平台能力」
//! 的每个格子都钉死成字面期望：实际生效的组合、回退原因、机器能力不足的原因、
//! 请求的系统材质、清屏 alpha、根背景透明性与自绘纹理的有无。
//!
//! 期望值全部手写，不从被测实现反推：因此这些断言是独立的事实来源。

use unbound_backdrop::{
    has_self_drawn_variant, required_build, system_material_request, BackdropCapabilities,
    BackdropCompositeAlpha, BackdropInputs, BackdropPlan, BackdropSelection, BackdropSource,
    DwmMaterial, FallbackReason, MaterialKind, PixelRect, UnavailableReason, NATIVE_MICA_MIN_BUILD,
};

/// 主屏物理矩形。
const SCREEN: PixelRect = PixelRect::new(0, 0, 1920, 1080);

/// 客户区物理矩形：落在主屏之内，保证自绘纹理可以生成。
const WINDOW: PixelRect = PixelRect::new(480, 270, 960, 540);

/// 一条来源期望：某个能力档下、某个来源档的实际结论。
#[derive(Clone, Copy, Debug, PartialEq)]
struct SourceExpectation {
    /// 实际生效的来源。
    source: BackdropSource,
    /// 回退原因。
    fallback: Option<FallbackReason>,
    /// 机器能力不足的原因。
    unavailable: Option<UnavailableReason>,
    /// 计划请求的系统材质。
    material: DwmMaterial,
    /// 清屏 alpha。
    clear_alpha: f32,
    /// 是否生成自绘纹理。
    has_texture: bool,
    /// 自绘纹理就绪时的根背景透明性。
    root_transparent_ready: bool,
    /// 自绘纹理尚未就绪时的根背景透明性。
    root_transparent_not_ready: bool,
}

/// 原生路径成立时的期望：请求云母变体材质、清屏全透明、不生成纹理。
///
/// 「自动」与「系统原生」两个来源在原生路径可用时落到同一个结论。
const NATIVE_AVAILABLE: SourceExpectation = SourceExpectation {
    source: BackdropSource::Native,
    fallback: None,
    unavailable: None,
    material: DwmMaterial::MicaAlt,
    clear_alpha: 0.0,
    has_texture: false,
    root_transparent_ready: true,
    root_transparent_not_ready: true,
};

/// 「自动」来源在原生不可用时的期望：改走自绘，并说明为什么被降级。
const AUTO_FALLBACK: SourceExpectation = SourceExpectation {
    source: BackdropSource::SelfDrawn,
    fallback: Some(FallbackReason::NativeUnavailable {
        material: MaterialKind::MicaAlt,
    }),
    unavailable: None,
    material: DwmMaterial::None,
    clear_alpha: 1.0,
    has_texture: true,
    root_transparent_ready: true,
    root_transparent_not_ready: false,
};

/// 「引擎自绘」来源的期望：任何机器上都走自绘，不请求任何系统材质。
const SELF_DRAWN: SourceExpectation = SourceExpectation {
    source: BackdropSource::SelfDrawn,
    fallback: None,
    unavailable: None,
    material: DwmMaterial::None,
    clear_alpha: 1.0,
    has_texture: true,
    root_transparent_ready: true,
    root_transparent_not_ready: false,
};

/// 「系统原生」来源在机器承载不了时的期望：改走自绘，并给出能力不足的原因。
fn native_unavailable(reason: UnavailableReason) -> SourceExpectation {
    SourceExpectation {
        source: BackdropSource::SelfDrawn,
        fallback: None,
        unavailable: Some(reason),
        material: DwmMaterial::None,
        clear_alpha: 1.0,
        has_texture: true,
        root_transparent_ready: true,
        root_transparent_not_ready: false,
    }
}

/// 一档平台能力用例。
#[derive(Clone, Copy, Debug)]
struct CapabilityCase {
    /// 用例名称，仅用于断言消息。
    name: &'static str,
    /// 系统构建号。
    build: Option<u32>,
    /// 表面是否支持预乘合成。
    premultiplied: bool,
    /// 三个来源档的期望，顺序与 [`BackdropSource::ALL`] 一致。
    rows: [SourceExpectation; 3],
}

/// 能力矩阵：构建号未知、门槛下沿、门槛上沿与实验档门槛两侧，各带预乘合成开关。
fn capability_cases() -> Vec<CapabilityCase> {
    vec![
        CapabilityCase {
            name: "unknown build",
            build: None,
            premultiplied: true,
            rows: [
                AUTO_FALLBACK,
                native_unavailable(UnavailableReason::BuildUnknown),
                SELF_DRAWN,
            ],
        },
        CapabilityCase {
            name: "unknown build without premultiplied alpha",
            build: None,
            premultiplied: false,
            rows: [
                AUTO_FALLBACK,
                native_unavailable(UnavailableReason::BuildUnknown),
                SELF_DRAWN,
            ],
        },
        CapabilityCase {
            name: "build just below the mica gate",
            build: Some(22620),
            premultiplied: true,
            rows: [
                AUTO_FALLBACK,
                native_unavailable(UnavailableReason::BuildTooLow {
                    required: NATIVE_MICA_MIN_BUILD,
                }),
                SELF_DRAWN,
            ],
        },
        CapabilityCase {
            name: "build just below the mica gate without premultiplied alpha",
            build: Some(22620),
            premultiplied: false,
            rows: [
                AUTO_FALLBACK,
                native_unavailable(UnavailableReason::BuildTooLow {
                    required: NATIVE_MICA_MIN_BUILD,
                }),
                SELF_DRAWN,
            ],
        },
        CapabilityCase {
            name: "build on the mica gate",
            build: Some(22621),
            premultiplied: true,
            rows: [NATIVE_AVAILABLE, NATIVE_AVAILABLE, SELF_DRAWN],
        },
        CapabilityCase {
            name: "build on the mica gate without premultiplied alpha",
            build: Some(22621),
            premultiplied: false,
            rows: [
                AUTO_FALLBACK,
                native_unavailable(UnavailableReason::PremultipliedAlphaUnsupported),
                SELF_DRAWN,
            ],
        },
        CapabilityCase {
            name: "build on the immersive gate",
            build: Some(26100),
            premultiplied: true,
            rows: [NATIVE_AVAILABLE, NATIVE_AVAILABLE, SELF_DRAWN],
        },
        CapabilityCase {
            name: "build on the immersive gate without premultiplied alpha",
            build: Some(26100),
            premultiplied: false,
            rows: [
                AUTO_FALLBACK,
                native_unavailable(UnavailableReason::PremultipliedAlphaUnsupported),
                SELF_DRAWN,
            ],
        },
    ]
}

/// 造一份计划输入。
fn inputs(
    material: MaterialKind,
    source: BackdropSource,
    build: Option<u32>,
    premultiplied: bool,
    self_drawn_ready: bool,
) -> BackdropInputs {
    BackdropInputs {
        selection: BackdropSelection::new(material, source),
        windows_build: build,
        premultiplied_alpha_supported: premultiplied,
        window_screen_rect: Some(WINDOW),
        primary_screen_rect: Some(SCREEN),
        primary_screen_scale: 1.5,
        dark: false,
        self_drawn_ready,
    }
}

/// 云母变体在能力矩阵的每个格子上都给出预期结论。
#[test]
fn mica_alt_resolves_every_capability_and_source_case() {
    let mut checked = 0usize;
    for case in capability_cases() {
        for (index, source) in BackdropSource::ALL.into_iter().enumerate() {
            let expected = case.rows[index];
            for ready in [false, true] {
                let plan = BackdropPlan::derive(inputs(
                    MaterialKind::MicaAlt,
                    source,
                    case.build,
                    case.premultiplied,
                    ready,
                ));
                let label = format!(
                    "source {source:?} on the {} case (ready = {ready})",
                    case.name
                );
                assert_eq!(plan.material_kind, MaterialKind::MicaAlt, "{label}");
                assert_eq!(plan.source, expected.source, "{label}");
                assert_eq!(plan.fallback, expected.fallback, "{label}");
                assert_eq!(plan.unavailable, expected.unavailable, "{label}");
                assert_eq!(plan.material, expected.material, "{label}");
                assert_eq!(plan.clear_alpha, expected.clear_alpha, "{label}");
                assert_eq!(
                    plan.texture.is_some(),
                    expected.has_texture,
                    "{label} must decide about the self-drawn texture"
                );
                let root_transparent = if ready {
                    expected.root_transparent_ready
                } else {
                    expected.root_transparent_not_ready
                };
                assert_eq!(
                    plan.root_background_transparent, root_transparent,
                    "{label} must follow the texture readiness"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(
        checked, 24,
        "the mica variant must be pinned for every source and capability case"
    );
}

/// 云母变体与标准云母共享门槛与自绘变体，因此两者的原生可用性必须一致。
#[test]
fn mica_alt_shares_the_gate_with_standard_mica() {
    assert_eq!(
        required_build(MaterialKind::MicaAlt),
        NATIVE_MICA_MIN_BUILD,
        "the mica variant must share the standard mica build gate"
    );
    assert_eq!(
        required_build(MaterialKind::MicaAlt),
        required_build(MaterialKind::Mica),
        "the mica variant must not raise the build gate above standard mica"
    );
    assert!(
        has_self_drawn_variant(MaterialKind::MicaAlt),
        "the mica variant must have a self-drawn variant"
    );
    for case in capability_cases() {
        let caps = BackdropCapabilities {
            windows_build: case.build,
            premultiplied_alpha_supported: case.premultiplied,
        };
        let mica = BackdropPlan::derive(inputs(
            MaterialKind::Mica,
            BackdropSource::Auto,
            case.build,
            case.premultiplied,
            false,
        ));
        let mica_alt = BackdropPlan::derive(inputs(
            MaterialKind::MicaAlt,
            BackdropSource::Auto,
            case.build,
            case.premultiplied,
            false,
        ));
        assert_eq!(
            mica.source, mica_alt.source,
            "the mica variant must reach the same source on the {} case",
            case.name
        );
        assert_eq!(
            mica.unavailable, mica_alt.unavailable,
            "the mica variant must report the same capability shortfall on the {} case",
            case.name
        );
        assert_eq!(
            mica.fallback.is_some(),
            mica_alt.fallback.is_some(),
            "the mica variant must fall back under the same conditions on the {} case",
            case.name
        );
        assert_eq!(
            caps.baseline_native_available(),
            mica_alt.composition == BackdropCompositeAlpha::PreMultiplied,
            "the mica variant must follow the platform baseline on the {} case",
            case.name
        );
    }
}

/// 云母变体向系统请求自己的那一档材质值，与标准云母互不混淆。
#[test]
fn mica_alt_requests_its_own_system_material() {
    assert_eq!(
        system_material_request(MaterialKind::MicaAlt),
        Some(DwmMaterial::MicaAlt),
        "the mica variant must request its own system material"
    );
    assert_ne!(
        system_material_request(MaterialKind::MicaAlt),
        system_material_request(MaterialKind::Mica),
        "the mica variant must not be confused with standard mica"
    );
    let native = BackdropPlan::derive(inputs(
        MaterialKind::MicaAlt,
        BackdropSource::Native,
        Some(26100),
        true,
        false,
    ));
    assert_eq!(native.material, DwmMaterial::MicaAlt);
    assert_eq!(
        native.clear_alpha, 0.0,
        "a native material must clear the surface fully"
    );
    assert!(
        native.root_background_transparent,
        "a native material must let the root background show through"
    );
    assert!(
        native.texture.is_none(),
        "the native path must not generate a self-drawn texture"
    );
}

/// 云母变体与标准云母在自绘路径上必须完全一致：同一份纹理配方、同样的清屏 alpha、
/// 同样的根背景透明性、同样不请求任何系统材质。
#[test]
fn mica_alt_and_standard_mica_agree_on_the_self_drawn_path() {
    for case in capability_cases() {
        for ready in [false, true] {
            let mica = BackdropPlan::derive(inputs(
                MaterialKind::Mica,
                BackdropSource::SelfDrawn,
                case.build,
                case.premultiplied,
                ready,
            ));
            let mica_alt = BackdropPlan::derive(inputs(
                MaterialKind::MicaAlt,
                BackdropSource::SelfDrawn,
                case.build,
                case.premultiplied,
                ready,
            ));
            let label = format!("the {} case (ready = {ready})", case.name);
            assert_eq!(
                mica.texture, mica_alt.texture,
                "{label} must use the same texture recipe"
            );
            assert_eq!(mica.clear_alpha, mica_alt.clear_alpha, "{label}");
            assert_eq!(
                mica.root_background_transparent, mica_alt.root_background_transparent,
                "{label} must have the same root transparency"
            );
            assert_eq!(mica.material, DwmMaterial::None, "{label}");
            assert_eq!(
                mica_alt.material,
                DwmMaterial::None,
                "{label} must not request any system material"
            );
            assert_eq!(mica.uv, mica_alt.uv, "{label} must sample the same region");
            assert_eq!(mica.source, mica_alt.source, "{label}");
            assert_eq!(mica.fallback, mica_alt.fallback, "{label}");
            assert_eq!(mica.unavailable, mica_alt.unavailable, "{label}");
        }
    }
}

/// 自绘路径的纹理依赖窗口与主屏几何：几何未知时没有纹理，路径本身不变。
#[test]
fn the_self_drawn_texture_needs_known_geometry() {
    let mut input = inputs(
        MaterialKind::MicaAlt,
        BackdropSource::SelfDrawn,
        Some(26100),
        true,
        false,
    );
    input.window_screen_rect = None;
    let plan = BackdropPlan::derive(input);
    assert_eq!(plan.source, BackdropSource::SelfDrawn);
    assert!(
        plan.texture.is_none(),
        "an unknown window rect must leave the self-drawn path without a texture"
    );
    assert_eq!(plan.clear_alpha, 1.0);
    assert_eq!(plan.material, DwmMaterial::None);
}

/// 在材质之间切换不得改变表面合成方式：交换链在启动时一次性建好。
#[test]
fn switching_between_mica_and_mica_alt_keeps_the_surface_composition() {
    for case in capability_cases() {
        let mica = BackdropPlan::derive(inputs(
            MaterialKind::Mica,
            BackdropSource::Native,
            case.build,
            case.premultiplied,
            false,
        ));
        let mica_alt = BackdropPlan::derive(inputs(
            MaterialKind::MicaAlt,
            BackdropSource::Native,
            case.build,
            case.premultiplied,
            false,
        ));
        assert_eq!(
            mica.composition, mica_alt.composition,
            "the mica variant must not reconfigure the surface on the {} case",
            case.name
        );
    }
}

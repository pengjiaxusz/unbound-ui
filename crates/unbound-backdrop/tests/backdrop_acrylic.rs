//! 亚克力材质在两轴后景模型中的端到端行为。
//!
//! 亚克力是「有系统材质请求、但没有自绘变体」的那一档：它必须真的到达系统
//! （向 DWM 请求亚克力取值，清屏 alpha 归零让材质从透明表面后透出），也必须
//! 在自绘来源下诚实地回退——回退先保材质种类、再保来源，绝不静默地造出一份
//! 亚克力外观。
//!
//! 本文件在模型接缝上穷举「能力矩阵 × 三种来源」，期望值全部是手写字面量，
//! 独立于被测实现的推导。

use unbound_backdrop::{
    has_self_drawn_variant, native_available, required_build, resolve_combination,
    system_material_request, unavailable_reason, BackdropCapabilities, BackdropCompositeAlpha,
    BackdropInputs, BackdropPlan, BackdropSelection, BackdropSource, DwmMaterial, FallbackReason,
    MaterialKind, PixelRect, UnavailableReason,
};

/// 主屏物理矩形：自绘纹理的采样空间。
const SCREEN: PixelRect = PixelRect::new(0, 0, 1920, 1080);
/// 窗口物理矩形。
const WINDOW: PixelRect = PixelRect::new(480, 270, 960, 540);
/// 亚克力的最低系统构建号（Windows 11 22H2）。
const ACRYLIC_MIN_BUILD: u32 = 22621;

/// 一档平台能力下的亚克力期望。
///
/// 期望值都是手写字面量：原生是否可用、不可用原因、表面合成方式逐档列出。
struct AcrylicCase {
    /// 用例名称，仅用于断言消息。
    name: &'static str,
    /// 系统构建号。
    build: Option<u32>,
    /// 表面是否支持预乘 alpha 合成。
    premultiplied: bool,
    /// 该档能力下亚克力能否走原生路径。
    native_ok: bool,
    /// 该档能力下亚克力的不可用原因。
    unavailable: Option<UnavailableReason>,
    /// 该档能力下的表面合成方式。
    composition: BackdropCompositeAlpha,
}

/// 能力矩阵：构建号未知、门槛下沿、门槛上沿与实验档门槛，预乘 alpha 有无各一档。
const ACRYLIC_CASES: [AcrylicCase; 8] = [
    AcrylicCase {
        name: "unknown build",
        build: None,
        premultiplied: true,
        native_ok: false,
        unavailable: Some(UnavailableReason::BuildUnknown),
        composition: BackdropCompositeAlpha::Opaque,
    },
    AcrylicCase {
        name: "unknown build without premultiplied alpha",
        build: None,
        premultiplied: false,
        native_ok: false,
        unavailable: Some(UnavailableReason::BuildUnknown),
        composition: BackdropCompositeAlpha::Opaque,
    },
    AcrylicCase {
        name: "build just below the acrylic gate",
        build: Some(22620),
        premultiplied: true,
        native_ok: false,
        unavailable: Some(UnavailableReason::BuildTooLow {
            required: ACRYLIC_MIN_BUILD,
        }),
        composition: BackdropCompositeAlpha::Opaque,
    },
    AcrylicCase {
        name: "build just below the acrylic gate without premultiplied alpha",
        build: Some(22620),
        premultiplied: false,
        native_ok: false,
        unavailable: Some(UnavailableReason::BuildTooLow {
            required: ACRYLIC_MIN_BUILD,
        }),
        composition: BackdropCompositeAlpha::Opaque,
    },
    AcrylicCase {
        name: "build on the acrylic gate",
        build: Some(22621),
        premultiplied: true,
        native_ok: true,
        unavailable: None,
        composition: BackdropCompositeAlpha::PreMultiplied,
    },
    AcrylicCase {
        name: "build on the acrylic gate without premultiplied alpha",
        build: Some(22621),
        premultiplied: false,
        native_ok: false,
        unavailable: Some(UnavailableReason::PremultipliedAlphaUnsupported),
        composition: BackdropCompositeAlpha::Opaque,
    },
    AcrylicCase {
        name: "build on the immersive gate",
        build: Some(26100),
        premultiplied: true,
        native_ok: true,
        unavailable: None,
        composition: BackdropCompositeAlpha::PreMultiplied,
    },
    AcrylicCase {
        name: "build on the immersive gate without premultiplied alpha",
        build: Some(26100),
        premultiplied: false,
        native_ok: false,
        unavailable: Some(UnavailableReason::PremultipliedAlphaUnsupported),
        composition: BackdropCompositeAlpha::Opaque,
    },
];

/// 生效组合的期望。
struct CombinationExpectation {
    /// 实际生效的材质种类。
    material: MaterialKind,
    /// 实际生效的来源。
    source: BackdropSource,
    /// 回退原因。
    fallback: Option<FallbackReason>,
}

/// 原生路径可用时，三种来源各自的生效组合（按 Auto / Native / SelfDrawn 顺序）。
///
/// 自绘来源落在非法组合上：回退先保材质种类，来源改成原生。
const COMBINATION_WITH_NATIVE: [CombinationExpectation; 3] = [
    CombinationExpectation {
        material: MaterialKind::Acrylic,
        source: BackdropSource::Native,
        fallback: None,
    },
    CombinationExpectation {
        material: MaterialKind::Acrylic,
        source: BackdropSource::Native,
        fallback: None,
    },
    CombinationExpectation {
        material: MaterialKind::Acrylic,
        source: BackdropSource::Native,
        fallback: Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic,
        }),
    },
];

/// 原生路径不可用时，三种来源各自的生效组合（按 Auto / Native / SelfDrawn 顺序）。
///
/// 显式原生来源是机器能力不足：没有后景可言。自动与自绘来源回退到标准云母自绘。
const COMBINATION_WITHOUT_NATIVE: [CombinationExpectation; 3] = [
    CombinationExpectation {
        material: MaterialKind::Mica,
        source: BackdropSource::SelfDrawn,
        fallback: Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic,
        }),
    },
    CombinationExpectation {
        material: MaterialKind::None,
        source: BackdropSource::Native,
        fallback: None,
    },
    CombinationExpectation {
        material: MaterialKind::Mica,
        source: BackdropSource::SelfDrawn,
        fallback: Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic,
        }),
    },
];

/// 计划层面的期望。
struct PlanExpectation {
    /// 实际请求的系统材质。
    material: DwmMaterial,
    /// 清屏 alpha。
    clear_alpha: f32,
    /// 根背景是否透明。
    root_background_transparent: bool,
    /// 是否生成自绘纹理。
    has_texture: bool,
}

/// 原生路径可用时，三种来源各自的计划（按 Auto / Native / SelfDrawn 顺序）。
///
/// 三种来源最终都走原生：请求亚克力材质，清屏 alpha 归零，没有自绘纹理。
const PLAN_WITH_NATIVE: [PlanExpectation; 3] = [
    PlanExpectation {
        material: DwmMaterial::Acrylic,
        clear_alpha: 0.0,
        root_background_transparent: true,
        has_texture: false,
    },
    PlanExpectation {
        material: DwmMaterial::Acrylic,
        clear_alpha: 0.0,
        root_background_transparent: true,
        has_texture: false,
    },
    PlanExpectation {
        material: DwmMaterial::Acrylic,
        clear_alpha: 0.0,
        root_background_transparent: true,
        has_texture: false,
    },
];

/// 原生路径不可用时，三种来源各自的计划（按 Auto / Native / SelfDrawn 顺序）。
///
/// 显式原生来源没有后景也没有纹理；回退到自绘的两种来源带着壁纸纹理。
const PLAN_WITHOUT_NATIVE: [PlanExpectation; 3] = [
    PlanExpectation {
        material: DwmMaterial::None,
        clear_alpha: 1.0,
        root_background_transparent: true,
        has_texture: true,
    },
    PlanExpectation {
        material: DwmMaterial::None,
        clear_alpha: 1.0,
        root_background_transparent: false,
        has_texture: false,
    },
    PlanExpectation {
        material: DwmMaterial::None,
        clear_alpha: 1.0,
        root_background_transparent: true,
        has_texture: true,
    },
];

fn caps_of(case: &AcrylicCase) -> BackdropCapabilities {
    BackdropCapabilities {
        windows_build: case.build,
        premultiplied_alpha_supported: case.premultiplied,
    }
}

fn inputs_of(case: &AcrylicCase, source: BackdropSource, self_drawn_ready: bool) -> BackdropInputs {
    BackdropInputs {
        selection: BackdropSelection::new(MaterialKind::Acrylic, source),
        windows_build: case.build,
        premultiplied_alpha_supported: case.premultiplied,
        window_screen_rect: Some(WINDOW),
        primary_screen_rect: Some(SCREEN),
        primary_screen_scale: 1.0,
        dark: false,
        self_drawn_ready,
    }
}

/// 亚克力在材质种类里明确没有自绘变体。
#[test]
fn the_acrylic_material_declares_no_self_drawn_variant() {
    assert!(
        !has_self_drawn_variant(MaterialKind::Acrylic),
        "the acrylic material must declare that it has no self-drawn variant"
    );
    assert!(
        has_self_drawn_variant(MaterialKind::Mica),
        "standard Mica must keep its self-drawn variant"
    );
    assert!(
        has_self_drawn_variant(MaterialKind::MicaAlt),
        "the Mica variant must keep its self-drawn variant"
    );
    assert_eq!(
        required_build(MaterialKind::Acrylic),
        ACRYLIC_MIN_BUILD,
        "the acrylic material must require Windows build 22621"
    );
    assert_eq!(
        system_material_request(MaterialKind::Acrylic),
        Some(DwmMaterial::Acrylic),
        "the acrylic material must request the acrylic system material"
    );
}

/// 亚克力的原生可用性与不可用原因逐档符合门槛。
#[test]
fn the_acrylic_gate_follows_the_capability_matrix() {
    for case in &ACRYLIC_CASES {
        let caps = caps_of(case);
        assert_eq!(
            native_available(MaterialKind::Acrylic, &caps),
            case.native_ok,
            "the acrylic native availability must match the {} case",
            case.name
        );
        assert_eq!(
            unavailable_reason(MaterialKind::Acrylic, &caps),
            case.unavailable,
            "the acrylic unavailable reason must match the {} case",
            case.name
        );
    }
}

/// 能力矩阵 × 三种来源下，亚克力的生效组合逐项符合阶梯。
#[test]
fn the_acrylic_combination_ladder_is_pinned_for_every_capability_case() {
    let mut checked = 0usize;
    for case in &ACRYLIC_CASES {
        let caps = caps_of(case);
        for source in BackdropSource::ALL {
            let combination =
                resolve_combination(BackdropSelection::new(MaterialKind::Acrylic, source), &caps);
            let expected = if case.native_ok {
                &COMBINATION_WITH_NATIVE[source.index()]
            } else {
                &COMBINATION_WITHOUT_NATIVE[source.index()]
            };
            assert_eq!(
                combination.material, expected.material,
                "the effective acrylic material must match the {} case on {source:?}",
                case.name
            );
            assert_eq!(
                combination.source, expected.source,
                "the effective acrylic source must match the {} case on {source:?}",
                case.name
            );
            assert_eq!(
                combination.fallback, expected.fallback,
                "the acrylic fallback reason must match the {} case on {source:?}",
                case.name
            );
            // 显式原生来源在机器承载不了时报不可用；其余来源从不报不可用。
            let expected_unavailable = if source == BackdropSource::Native && !case.native_ok {
                case.unavailable
            } else {
                None
            };
            assert_eq!(
                combination.unavailable, expected_unavailable,
                "the acrylic unavailable reason must match the {} case on {source:?}",
                case.name
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 24,
        "the acrylic ladder must be pinned for every source and capability case"
    );
}

/// 亚克力的计划逐项符合期望：系统材质请求、清屏 alpha、根背景透明性与纹理。
#[test]
fn the_acrylic_plan_carries_the_expected_material_and_surface_state() {
    for case in &ACRYLIC_CASES {
        for source in BackdropSource::ALL {
            let plan = BackdropPlan::derive(inputs_of(case, source, true));
            let expected = if case.native_ok {
                &PLAN_WITH_NATIVE[source.index()]
            } else {
                &PLAN_WITHOUT_NATIVE[source.index()]
            };
            assert_eq!(
                plan.material, expected.material,
                "the requested system material must match the {} case on {source:?}",
                case.name
            );
            assert_eq!(
                plan.clear_alpha, expected.clear_alpha,
                "the clear alpha must match the {} case on {source:?}",
                case.name
            );
            assert_eq!(
                plan.root_background_transparent, expected.root_background_transparent,
                "the root transparency must match the {} case on {source:?}",
                case.name
            );
            assert_eq!(
                plan.texture.is_some(),
                expected.has_texture,
                "the self-drawn texture must match the {} case on {source:?}",
                case.name
            );
            assert_eq!(
                plan.composition, case.composition,
                "the surface composition must match the {} case on {source:?}",
                case.name
            );
        }
    }
}

/// 自绘纹理尚未就绪时，自绘路径的根背景仍是不透明的。
#[test]
fn the_self_drawn_fallback_keeps_an_opaque_root_until_the_texture_is_ready() {
    let case = ACRYLIC_CASES
        .iter()
        .find(|case| !case.native_ok)
        .expect("the matrix must contain a case without a native path");
    let plan = BackdropPlan::derive(inputs_of(case, BackdropSource::SelfDrawn, false));
    assert_eq!(
        plan.clear_alpha, 1.0,
        "the self-drawn fallback must keep the clear alpha opaque"
    );
    assert!(
        !plan.root_background_transparent,
        "the self-drawn fallback must keep the root opaque until the texture is ready"
    );
    assert!(
        plan.texture.is_some(),
        "the self-drawn fallback must still generate the wallpaper texture"
    );
}

/// 非法自绘组合必须带回退原因，且计划里没有亚克力系统材质请求。
#[test]
fn an_illegal_acrylic_self_drawn_choice_falls_back_without_faking_acrylic() {
    let capable = ACRYLIC_CASES
        .iter()
        .find(|case| case.native_ok)
        .expect("the matrix must contain a capable case");
    let weak = ACRYLIC_CASES
        .iter()
        .find(|case| {
            !case.native_ok
                && case.unavailable
                    == Some(UnavailableReason::BuildTooLow {
                        required: ACRYLIC_MIN_BUILD,
                    })
        })
        .expect("the matrix must contain a build-gated case");

    // 原生路径可用：回退保材质种类、来源改成原生，材质请求仍是亚克力。
    let capable_plan = BackdropPlan::derive(inputs_of(capable, BackdropSource::SelfDrawn, true));
    assert_eq!(
        capable_plan.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic,
        }),
        "the illegal self-drawn choice must report the missing self-drawn variant"
    );
    assert_eq!(
        capable_plan.material_kind,
        MaterialKind::Acrylic,
        "the fallback must keep the acrylic material kind"
    );
    assert_eq!(
        capable_plan.source,
        BackdropSource::Native,
        "the fallback must switch to the native source"
    );

    // 原生路径不可用：回退到标准云母自绘，计划里没有亚克力系统材质请求。
    let weak_plan = BackdropPlan::derive(inputs_of(weak, BackdropSource::SelfDrawn, true));
    assert_eq!(
        weak_plan.fallback,
        Some(FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic,
        }),
        "the illegal self-drawn choice must report the missing self-drawn variant"
    );
    assert_eq!(
        weak_plan.material_kind,
        MaterialKind::Mica,
        "the fallback must switch to the standard mica material kind"
    );
    assert_eq!(
        weak_plan.source,
        BackdropSource::SelfDrawn,
        "the fallback must keep the self-drawn source"
    );
    assert_eq!(
        weak_plan.material,
        DwmMaterial::None,
        "the fallback must not request any system material"
    );
    assert_eq!(
        weak_plan.clear_alpha, 1.0,
        "the fallback must keep the clear alpha opaque"
    );
}

/// 在云母与亚克力之间切换不改变表面合成方式，因此不重建交换链。
#[test]
fn switching_between_mica_and_acrylic_keeps_the_surface_composition() {
    let case = ACRYLIC_CASES
        .iter()
        .find(|case| case.native_ok)
        .expect("the matrix must contain a capable case");
    let mica = BackdropPlan::derive(BackdropInputs {
        selection: BackdropSelection::new(MaterialKind::Mica, BackdropSource::Native),
        windows_build: case.build,
        premultiplied_alpha_supported: case.premultiplied,
        window_screen_rect: Some(WINDOW),
        primary_screen_rect: Some(SCREEN),
        primary_screen_scale: 1.0,
        dark: false,
        self_drawn_ready: true,
    });
    let acrylic = BackdropPlan::derive(inputs_of(case, BackdropSource::Native, true));
    assert_eq!(
        acrylic.composition, mica.composition,
        "switching between Mica and Acrylic must not change the surface composition"
    );
    assert_eq!(
        acrylic.material,
        DwmMaterial::Acrylic,
        "the acrylic material must request the acrylic system material"
    );
    assert_eq!(
        mica.material,
        DwmMaterial::Mica,
        "the mica material must keep requesting the mica system material"
    );
    assert_eq!(
        acrylic.clear_alpha, mica.clear_alpha,
        "both native materials must clear to a fully transparent surface"
    );
}

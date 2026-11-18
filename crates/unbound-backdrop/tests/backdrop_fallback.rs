//! 后景两类原因的分离与自绘覆盖边界。
//!
//! 这个文件钉住两条彼此独立的事实：
//!
//! 1. **机器能力不足**只出现在不可用原因里，**组合非法**只出现在回退原因里，
//!    同一次解析绝不混用；前者对应的组合永远不可选，后者永远可选。
//! 2. 「该材质有没有自绘变体」是一份显式声明，`has_self_drawn_variant` 与
//!    `resolve_combination` 都必须从它派生，穷举「材质 × 来源 × 能力矩阵」
//!    时不得出现第二份真相。
//!
//! 期望值全部是手写字面量，独立于被测实现的推导。

use unbound_backdrop::{
    has_self_drawn_variant, is_illegal_combination, native_available, resolve_combination,
    selectable, self_drawn_coverage, unavailable_reason, BackdropCapabilities, BackdropSelection,
    BackdropSource, FallbackReason, MaterialKind, SelfDrawnCoverage, IMMERSIVE_BACKDROP_MIN_BUILD,
    NATIVE_MICA_MIN_BUILD,
};

/// 一档平台能力的期望，全部手写。
struct CapabilityCase {
    /// 用例名称，仅用于断言消息。
    name: &'static str,
    /// 系统构建号。
    build: Option<u32>,
    /// 表面是否支持预乘合成。
    premultiplied: bool,
}

/// 能力矩阵：构建号未知、两道门槛的下沿与上沿，预乘 alpha 有无各一档。
const CAPABILITY_CASES: [CapabilityCase; 8] = [
    CapabilityCase {
        name: "unknown build",
        build: None,
        premultiplied: true,
    },
    CapabilityCase {
        name: "unknown build without premultiplied alpha",
        build: None,
        premultiplied: false,
    },
    CapabilityCase {
        name: "build just below the mica gate",
        build: Some(22620),
        premultiplied: true,
    },
    CapabilityCase {
        name: "build on the mica gate",
        build: Some(22621),
        premultiplied: true,
    },
    CapabilityCase {
        name: "build on the mica gate without premultiplied alpha",
        build: Some(22621),
        premultiplied: false,
    },
    CapabilityCase {
        name: "build on the immersive gate",
        build: Some(26100),
        premultiplied: true,
    },
    CapabilityCase {
        name: "build on the immersive gate without premultiplied alpha",
        build: Some(26100),
        premultiplied: false,
    },
    CapabilityCase {
        name: "build above the immersive gate",
        build: Some(26101),
        premultiplied: true,
    },
];

fn caps_of(case: &CapabilityCase) -> BackdropCapabilities {
    BackdropCapabilities {
        windows_build: case.build,
        premultiplied_alpha_supported: case.premultiplied,
        accent_policy_export: false,
    }
}

fn resolve(
    material: MaterialKind,
    source: BackdropSource,
    caps: &BackdropCapabilities,
) -> unbound_backdrop::BackdropCombination {
    resolve_combination(BackdropSelection::new(material, source), caps)
}

/// 手写的覆盖边界期望，独立于被测实现。
fn expected_coverage(material: MaterialKind) -> SelfDrawnCoverage {
    match material {
        MaterialKind::Mica | MaterialKind::MicaAlt => SelfDrawnCoverage::Equivalent,
        MaterialKind::None
        | MaterialKind::FollowSystem
        | MaterialKind::Acrylic
        | MaterialKind::Immersive
        | MaterialKind::AcrylicTint => SelfDrawnCoverage::NoVariant,
    }
}

/// 能力不足与回退绝不同时出现。
#[test]
fn a_capability_shortfall_never_mixes_with_a_fallback() {
    let mut checked = 0usize;
    for material in MaterialKind::ALL {
        for source in BackdropSource::ALL {
            for case in &CAPABILITY_CASES {
                let combination = resolve(material, source, &caps_of(case));
                assert!(
                    !(combination.unavailable.is_some() && combination.fallback.is_some()),
                    "{material:?} on {source:?} in the {} case reported both reasons: {combination:?}",
                    case.name
                );
                checked += 1;
            }
        }
    }
    assert_eq!(
        checked, 168,
        "every material, source and capability case must be pinned"
    );
}

/// 显式原生来源在机器承载不了时只报不可用，绝不报回退。
#[test]
fn an_explicit_native_shortfall_is_never_reported_as_a_fallback() {
    for case in &CAPABILITY_CASES {
        let caps = caps_of(case);
        for material in MaterialKind::ALL {
            let combination = resolve(material, BackdropSource::Native, &caps);
            assert_eq!(
                combination.fallback, None,
                "{material:?} on the native source in the {} case must not report a fallback",
                case.name
            );
            assert_eq!(
                combination.unavailable,
                unavailable_reason(material, &caps),
                "{material:?} on the native source in the {} case must report the capability reason",
                case.name
            );
        }
    }
}

/// 自绘来源撞上没有自绘变体的材质时，组合非法但一律可选，且只报回退。
#[test]
fn an_illegal_self_drawn_combination_is_selectable_and_only_reports_a_fallback() {
    for case in &CAPABILITY_CASES {
        let caps = caps_of(case);
        for material in MaterialKind::ALL {
            if material == MaterialKind::None {
                continue;
            }
            assert_eq!(
                is_illegal_combination(BackdropSelection::new(material, BackdropSource::SelfDrawn)),
                expected_coverage(material) == SelfDrawnCoverage::NoVariant,
                "{material:?} must be illegal exactly when it has no self-drawn variant"
            );
            let combination = resolve(material, BackdropSource::SelfDrawn, &caps);
            if expected_coverage(material) == SelfDrawnCoverage::NoVariant {
                assert!(
                    selectable(material, BackdropSource::SelfDrawn, &caps),
                    "{material:?} on the self-drawn source in the {} case must stay selectable",
                    case.name
                );
                assert_eq!(
                    combination.fallback,
                    Some(FallbackReason::NoSelfDrawnVariant { material }),
                    "{material:?} in the {} case must explain the missing self-drawn variant",
                    case.name
                );
                assert_eq!(
                    combination.unavailable, None,
                    "{material:?} in the {} case must not claim a capability shortfall",
                    case.name
                );
            } else {
                assert_eq!(
                    combination.fallback, None,
                    "{material:?} in the {} case must satisfy the self-drawn request",
                    case.name
                );
                assert_eq!(
                    combination.unavailable, None,
                    "{material:?} in the {} case must not report a shortfall",
                    case.name
                );
            }
        }
    }
}

/// 覆盖声明是唯一真相：布尔接缝与解析阶梯都必须从它派生。
#[test]
fn the_self_drawn_coverage_declaration_is_the_only_source_of_truth() {
    for material in MaterialKind::ALL {
        let coverage = self_drawn_coverage(material);
        assert_eq!(
            coverage,
            expected_coverage(material),
            "{material:?} must declare the coverage boundary explicitly"
        );
        assert_eq!(
            has_self_drawn_variant(material),
            coverage == SelfDrawnCoverage::Equivalent,
            "{material:?} must derive the boolean view from the coverage declaration"
        );
    }
    for material in MaterialKind::ALL {
        for case in &CAPABILITY_CASES {
            let caps = caps_of(case);
            if material == MaterialKind::None {
                continue;
            }
            let self_drawn = resolve(material, BackdropSource::SelfDrawn, &caps);
            let expected_none = expected_coverage(material) == SelfDrawnCoverage::Equivalent;
            assert_eq!(
                self_drawn.fallback.is_none(),
                expected_none,
                "{material:?} on the self-drawn source in the {} case must follow the coverage declaration",
                case.name
            );

            if !native_available(material, &caps) {
                let auto = resolve(material, BackdropSource::Auto, &caps);
                let expected = if expected_none {
                    Some(FallbackReason::NativeUnavailable { material })
                } else {
                    Some(FallbackReason::NoSelfDrawnVariant { material })
                };
                assert_eq!(
                    auto.fallback, expected,
                    "{material:?} on the auto source in the {} case must follow the coverage declaration",
                    case.name
                );
            }
        }
    }
}

/// 回退的结果一定可以选中；能力不足的结果一定不可选。
#[test]
fn a_fallback_is_always_selectable_and_a_shortfall_never_is() {
    for material in MaterialKind::ALL {
        for source in BackdropSource::ALL {
            for case in &CAPABILITY_CASES {
                let caps = caps_of(case);
                let combination = resolve(material, source, &caps);
                let selectable_now = selectable(material, source, &caps);
                if combination.unavailable.is_some() {
                    assert!(
                        !selectable_now,
                        "{material:?} on {source:?} in the {} case reported a shortfall but stayed selectable",
                        case.name
                    );
                }
                // 「组合非法」只属于显式自绘来源。自动来源还有第三种状态：
                // 原生不可用且没有自绘变体，此时按钮按能力不足禁用，解析仍
                // 会说明它为什么落不到请求的材质上。
                if source == BackdropSource::SelfDrawn
                    && matches!(
                        combination.fallback,
                        Some(FallbackReason::NoSelfDrawnVariant { .. })
                    )
                {
                    assert!(
                        selectable_now,
                        "{material:?} on the {} case reported an illegal combination but was disabled",
                        case.name
                    );
                }
                // 显式来源的可用性与「是否报能力不足」完全等价；自动来源另有
                // 「两端都不可用」的第三种状态，因此不参与这条等价。
                if source != BackdropSource::Auto {
                    assert_eq!(
                        selectable_now,
                        combination.unavailable.is_none(),
                        "{material:?} on {source:?} in the {} case must keep selectability and the reason in step",
                        case.name
                    );
                }
            }
        }
    }
}

/// 亚克力与沉浸式的覆盖边界必须保持「无自绘变体」。
#[test]
fn the_acrylic_and_immersive_materials_declare_no_self_drawn_variant() {
    assert_eq!(
        self_drawn_coverage(MaterialKind::Acrylic),
        SelfDrawnCoverage::NoVariant,
        "acrylic is identified by its noise and layered feel, so a self-drawn copy would be misleading"
    );
    assert_eq!(
        self_drawn_coverage(MaterialKind::Immersive),
        SelfDrawnCoverage::NoVariant,
        "the immersive material has no system value at all, so it cannot have a self-drawn variant"
    );
    assert_eq!(
        self_drawn_coverage(MaterialKind::Mica),
        SelfDrawnCoverage::Equivalent,
        "standard Mica has a self-drawn equivalent"
    );
    assert_eq!(
        self_drawn_coverage(MaterialKind::MicaAlt),
        SelfDrawnCoverage::Equivalent,
        "the Mica variant has a self-drawn equivalent"
    );
    assert_eq!(
        NATIVE_MICA_MIN_BUILD, 22621,
        "the standard tier gate must stay at Windows 11 22H2"
    );
    assert_eq!(
        IMMERSIVE_BACKDROP_MIN_BUILD, 26100,
        "the immersive tier gate must stay at its experimental build"
    );
}

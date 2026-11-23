//! 亚克力自定义色调子档的独立门槛与独立通道。
//!
//! 这一档不使用 `DWMWA_SYSTEMBACKDROP_TYPE`：它的可用性由专属的
//! `accent_policy_export` 能力字段决定，系统请求通过 `BackdropPlan::accent_tint`
//! 表达。这里从公开 API 侧锁定这两点，避免实现细节回归。

use unbound_backdrop::{
    native_available, resolve_combination, selectable, unavailable_reason, BackdropCapabilities,
    BackdropInputs, BackdropPlan, BackdropSelection, BackdropSource, DwmMaterial, MaterialKind,
    UnavailableReason, ACCENT_POLICY_ACRYLIC_MIN_BUILD, DEFAULT_ACRYLIC_TINT_ARGB,
};

const SCREEN: unbound_backdrop::PixelRect = unbound_backdrop::PixelRect::new(0, 0, 2560, 1440);

fn caps(build: u32, accent_policy_export: bool) -> BackdropCapabilities {
    BackdropCapabilities {
        windows_build: Some(build),
        premultiplied_alpha_supported: true,
        accent_policy_export,
    }
}

fn inputs(selection: BackdropSelection, build: u32, accent_policy_export: bool) -> BackdropInputs {
    BackdropInputs {
        selection,
        windows_build: Some(build),
        premultiplied_alpha_supported: true,
        accent_policy_export,
        accent_tint_argb: DEFAULT_ACRYLIC_TINT_ARGB,
        window_screen_rect: Some(unbound_backdrop::PixelRect::new(120, 80, 960, 540)),
        primary_screen_rect: Some(SCREEN),
        primary_screen_scale: 1.5,
        dark: false,
        self_drawn_ready: false,
    }
}

#[test]
fn the_tier_gate_is_independent_from_the_system_material_gate() {
    // 构建号够、预乘合成够，但导出解析不到：只有这一档不可用。
    let no_export = caps(22621, false);
    assert!(
        !native_available(MaterialKind::AcrylicTint, &no_export),
        "the tinted acrylic tier must depend on its own export probe"
    );
    assert_eq!(
        unavailable_reason(MaterialKind::AcrylicTint, &no_export),
        Some(UnavailableReason::AccentPolicyUnavailable),
        "the missing export must surface as its own reason"
    );
    assert!(
        !selectable(
            MaterialKind::AcrylicTint,
            BackdropSource::Native,
            &no_export
        ),
        "an explicit native choice must be disabled without the channel"
    );
    // 同一台机器上，其它材质的原生通道完全不受影响。
    for material in [
        MaterialKind::FollowSystem,
        MaterialKind::Mica,
        MaterialKind::MicaAlt,
        MaterialKind::Acrylic,
    ] {
        assert!(
            native_available(material, &no_export),
            "{material:?} must not read the accent policy capability"
        );
    }

    // 导出可用：这一档在 17134 就能用，不需要等到云母的 22621。
    let on_old_build = caps(ACCENT_POLICY_ACRYLIC_MIN_BUILD, true);
    assert!(
        native_available(MaterialKind::AcrylicTint, &on_old_build),
        "the tinted acrylic tier must be native on Windows 10 1803"
    );
    assert_eq!(
        unavailable_reason(MaterialKind::AcrylicTint, &on_old_build),
        None
    );
    assert!(
        !native_available(MaterialKind::Mica, &on_old_build),
        "the Mica gate must not be lowered by the new tier"
    );

    let below = caps(ACCENT_POLICY_ACRYLIC_MIN_BUILD - 1, true);
    assert_eq!(
        unavailable_reason(MaterialKind::AcrylicTint, &below),
        Some(UnavailableReason::BuildTooLow {
            required: ACCENT_POLICY_ACRYLIC_MIN_BUILD,
        })
    );
}

#[test]
fn the_plan_uses_a_separate_accent_channel() {
    let plan = BackdropPlan::derive(inputs(
        BackdropSelection::new(MaterialKind::AcrylicTint, BackdropSource::Native),
        26100,
        true,
    ));
    assert_eq!(plan.material_kind, MaterialKind::AcrylicTint);
    assert_eq!(
        plan.material,
        DwmMaterial::None,
        "the accent tint must not be encoded as a DWM material value"
    );
    assert_eq!(
        plan.accent_tint,
        Some(DEFAULT_ACRYLIC_TINT_ARGB),
        "the plan must carry the explicit ABGR tint"
    );
    assert_eq!(
        plan.clear_alpha, 0.0,
        "a system compositor material needs a fully transparent clear"
    );
    assert!(
        plan.root_background_transparent,
        "the root background must be transparent under the accent material"
    );

    // 其它材质仍然走 DWM 通道，且绝不携带色调。
    let mica = BackdropPlan::derive(inputs(
        BackdropSelection::new(MaterialKind::Mica, BackdropSource::Native),
        26100,
        true,
    ));
    assert_eq!(mica.material, DwmMaterial::Mica);
    assert_eq!(mica.accent_tint, None);
    assert_eq!(mica.clear_alpha, 0.0);

    // 通道不可用时计划降级，不会悄悄改走 DWM 亚克力。
    let degraded = BackdropPlan::derive(inputs(
        BackdropSelection::new(MaterialKind::AcrylicTint, BackdropSource::Native),
        26100,
        false,
    ));
    assert_eq!(degraded.accent_tint, None);
    assert_ne!(
        degraded.material,
        DwmMaterial::Acrylic,
        "a failed accent channel must not be silently replaced by DWM acrylic"
    );
}

#[test]
fn the_illegal_combination_rule_matches_acrylic() {
    // 没有自绘变体：显式自绘必须回退到原生，而不是造一份假的色调亚克力。
    let combination = resolve_combination(
        BackdropSelection::new(MaterialKind::AcrylicTint, BackdropSource::SelfDrawn),
        &caps(26100, true),
    );
    assert_eq!(combination.material, MaterialKind::AcrylicTint);
    assert_eq!(combination.source, BackdropSource::Native);
    assert_eq!(
        combination.fallback,
        Some(unbound_backdrop::FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::AcrylicTint,
        })
    );
}

//! 沉浸式后景实验档与逐档门槛的模型层测试。
//!
//! 这些用例把「哪一档在什么来源与能力下可选、不可选时说什么原因」钉死在模型层。
//! 展台按钮的禁用态直接取自 [`selectable`]，因此这里的结论就是界面的结论。

use unbound_backdrop::{
    has_self_drawn_variant, native_available, selectable, unavailable_reason, BackdropCapabilities,
    BackdropSource, MaterialKind, UnavailableReason, IMMERSIVE_BACKDROP_MIN_BUILD,
};

fn capabilities(
    windows_build: Option<u32>,
    premultiplied_alpha_supported: bool,
) -> BackdropCapabilities {
    BackdropCapabilities {
        windows_build,
        premultiplied_alpha_supported,
    }
}

#[test]
fn immersive_is_the_only_experimental_material() {
    for material in MaterialKind::ALL {
        assert_eq!(
            material.is_experimental(),
            material == MaterialKind::Immersive,
            "only the immersive material must be marked experimental for {material:?}"
        );
    }
    assert!(
        MaterialKind::Immersive
            .description()
            .contains("Experimental"),
        "the experimental material must say so in its description"
    );
}

#[test]
fn an_unknown_build_disables_every_native_tier() {
    let caps = capabilities(None, true);
    for material in MaterialKind::ALL {
        assert_eq!(
            native_available(material, &caps),
            material == MaterialKind::None,
            "an unknown build must never count as native availability for {material:?}"
        );
    }
}

#[test]
fn the_native_source_disables_tiers_the_machine_cannot_host() {
    let caps = capabilities(Some(22621), true);
    for material in [
        MaterialKind::FollowSystem,
        MaterialKind::Mica,
        MaterialKind::MicaAlt,
        MaterialKind::Acrylic,
    ] {
        assert!(
            selectable(material, BackdropSource::Native, &caps),
            "a capable build must keep {material:?} selectable on the native source"
        );
    }
    assert!(
        !selectable(MaterialKind::Immersive, BackdropSource::Native, &caps),
        "the immersive tier must stay disabled until its system value is confirmed"
    );
    assert!(
        selectable(MaterialKind::None, BackdropSource::Native, &caps),
        "the no-material tier must never be gated"
    );
}

#[test]
fn the_auto_source_keeps_self_drawn_fallbacks_alive_but_hides_the_rest() {
    // 构建号低于原生门槛，自动来源只能靠自绘变体兜底。
    let caps = capabilities(Some(19045), true);
    for material in [MaterialKind::Mica, MaterialKind::MicaAlt] {
        assert!(
            selectable(material, BackdropSource::Auto, &caps),
            "{material:?} has a self-drawn variant, so auto must keep it selectable"
        );
        assert!(has_self_drawn_variant(material));
    }
    for material in [
        MaterialKind::FollowSystem,
        MaterialKind::Acrylic,
        MaterialKind::Immersive,
    ] {
        assert!(
            !has_self_drawn_variant(material),
            "{material:?} must not claim a self-drawn variant"
        );
        assert!(
            !selectable(material, BackdropSource::Auto, &caps),
            "{material:?} has no fallback, so auto must disable it"
        );
    }
    assert!(selectable(MaterialKind::None, BackdropSource::Auto, &caps));
}

#[test]
fn the_auto_source_still_offers_self_drawn_variants_on_an_unknown_build() {
    let caps = capabilities(None, false);
    assert!(
        selectable(MaterialKind::Mica, BackdropSource::Auto, &caps),
        "Mica can fall back to self-drawn even when the build is unknown"
    );
    assert!(selectable(
        MaterialKind::MicaAlt,
        BackdropSource::Auto,
        &caps
    ));
    assert!(
        !selectable(MaterialKind::Acrylic, BackdropSource::Auto, &caps),
        "Acrylic has no self-drawn variant, so auto must disable it"
    );
}

#[test]
fn the_self_drawn_source_never_disables_a_tier_for_lacking_a_variant() {
    let caps = capabilities(None, false);
    for material in MaterialKind::ALL {
        assert!(
            selectable(material, BackdropSource::SelfDrawn, &caps),
            "the self-drawn source must keep {material:?} selectable; a missing variant is not a machine limit"
        );
    }
}

#[test]
fn the_immersive_reason_names_the_missing_channel_after_the_build_gate() {
    let build_ok = capabilities(Some(IMMERSIVE_BACKDROP_MIN_BUILD), true);
    assert!(
        !native_available(MaterialKind::Immersive, &build_ok),
        "a high enough build must still not enable the immersive native path"
    );
    assert_eq!(
        unavailable_reason(MaterialKind::Immersive, &build_ok),
        Some(UnavailableReason::SystemChannelUnconfirmed),
        "a capable build must report the unconfirmed system channel"
    );

    let build_unknown = capabilities(None, true);
    assert_eq!(
        unavailable_reason(MaterialKind::Immersive, &build_unknown),
        Some(UnavailableReason::BuildUnknown),
        "an unknown build must be reported before the channel"
    );

    let build_too_low = capabilities(Some(IMMERSIVE_BACKDROP_MIN_BUILD - 1), true);
    assert_eq!(
        unavailable_reason(MaterialKind::Immersive, &build_too_low),
        Some(UnavailableReason::BuildTooLow {
            required: IMMERSIVE_BACKDROP_MIN_BUILD
        }),
        "an old build must be reported before the channel"
    );

    let no_premultiplied = capabilities(Some(IMMERSIVE_BACKDROP_MIN_BUILD), false);
    assert_eq!(
        unavailable_reason(MaterialKind::Immersive, &no_premultiplied),
        Some(UnavailableReason::PremultipliedAlphaUnsupported),
        "a surface without premultiplied alpha must be reported before the channel"
    );
}

#[test]
fn the_unavailable_reason_is_displayable_english() {
    let caps = capabilities(Some(IMMERSIVE_BACKDROP_MIN_BUILD), true);
    let reason = unavailable_reason(MaterialKind::Immersive, &caps)
        .expect("the immersive tier must be unavailable");
    assert!(!reason.label().is_empty(), "a reason must expose a label");
    assert!(
        reason.description().contains("unconfirmed"),
        "the immersive reason must explain the unconfirmed system channel"
    );
}

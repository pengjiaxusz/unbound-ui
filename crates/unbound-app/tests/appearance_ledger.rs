//! 外观意图账本集成测试（纯值进、Diff 出，无真实窗口与 GPU 依赖）。
//!
//! 覆盖 Issue #55 与 #56 的 10+1 项断言契约：
//! 1. 默认配置（意图关闭）：不产生任何系统写入，清屏 alpha 为 1；
//! 2. 云母意图：恰好一条 DWM 写入，外加一次初始深色标记；
//! 3. 相同输入连续同步两次：第二次没有任何写入；
//! 4. 表面非预乘：不发材质，清屏 alpha 被夹到 1；
//! 5. 系统构建号不足：不发材质，清屏 alpha 为 1；
//! 6. 主题翻转：只发深色标记，材质不重发；
//! 7. accent 色调意图：只发 accent 通道，不发 DWM；
//! 8. 通道切换（DWM → accent → DWM）：每次只发一条通道写入；
//! 9. 运行期关闭：发一次关闭写入、清屏 alpha 恢复 1，之后不再写；
//! 10. 宿主显式设过清屏 alpha 而意图开启：派生值覆盖宿主值；
//! 11. `Some(WindowBackdropMaterial::None)` 与 `None` 语义一致。

use unbound_app::appearance::{AppearanceCaps, AppearanceDiff, BackdropChannel, WindowAppearance};
use unbound_platform::WindowBackdropMaterial;
use unbound_theme::ThemeKind;

const CAPS_WIN11_NATIVE_OK: AppearanceCaps = AppearanceCaps::new(Some(22621), true, true);

#[test]
fn default_config_closed_produces_no_writes() {
    let mut appearance = WindowAppearance::new();
    let diff = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: None,
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );
    assert!(!diff.has_changes());
    assert_eq!(appearance.applied_clear_alpha(), 1.0);
}

#[test]
fn mica_intent_produces_dwm_write_and_initial_dark_mode() {
    let mut appearance = WindowAppearance::new();
    let diff = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: Some(BackdropChannel::Dwm(WindowBackdropMaterial::Mica)),
            dark_mode: Some(true),
            lock_mica: Some(true),
            clear_alpha: Some(0.0),
        }
    );
    assert!(diff.has_changes());
    assert_eq!(appearance.applied_clear_alpha(), 0.0);
}

#[test]
fn duplicate_sync_produces_no_writes() {
    let mut appearance = WindowAppearance::new();
    let first = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert!(first.has_changes());

    let second = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        second,
        AppearanceDiff {
            channel: None,
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );
    assert!(!second.has_changes());
}

#[test]
fn non_premultiplied_surface_clamps_clear_alpha_and_no_material() {
    let mut appearance = WindowAppearance::new();
    let caps_non_premultiplied = AppearanceCaps::new(Some(22621), false, true);

    let diff = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        caps_non_premultiplied,
        ThemeKind::Dark,
    );

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: None,
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );
    assert!(!diff.has_changes());
    assert_eq!(appearance.applied_clear_alpha(), 1.0);
}

#[test]
fn insufficient_build_number_produces_no_material() {
    let mut appearance = WindowAppearance::new();
    let caps_win10 = AppearanceCaps::new(Some(19045), true, true);

    let diff = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        caps_win10,
        ThemeKind::Dark,
    );

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: None,
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );
    assert!(!diff.has_changes());
    assert_eq!(appearance.applied_clear_alpha(), 1.0);
}

#[test]
fn theme_flip_only_emits_dark_mode() {
    let mut appearance = WindowAppearance::new();
    let first = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(first.dark_mode, Some(true));

    let diff = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Light,
    );

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: None,
            dark_mode: Some(false),
            lock_mica: None,
            clear_alpha: None,
        }
    );
}

#[test]
fn accent_tint_intent_only_emits_accent_channel() {
    let mut appearance = WindowAppearance::new();
    let tint_argb = 0xCCD07800;
    let diff = appearance.set_accent_tint(tint_argb, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: Some(BackdropChannel::AccentTint(tint_argb)),
            dark_mode: Some(true),
            lock_mica: Some(true),
            clear_alpha: Some(0.0),
        }
    );
}

#[test]
fn channel_switching_dwm_to_accent_to_dwm() {
    let mut appearance = WindowAppearance::new();
    let tint_argb = 0xCCD07800;

    let d1 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d1.channel,
        Some(BackdropChannel::Dwm(WindowBackdropMaterial::Mica))
    );

    let d2 = appearance.set_accent_tint(tint_argb, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(d2.channel, Some(BackdropChannel::AccentTint(tint_argb)));

    let d3 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d3.channel,
        Some(BackdropChannel::Dwm(WindowBackdropMaterial::Mica))
    );
}

#[test]
fn runtime_disable_emits_off_and_restores_clear_alpha() {
    let mut appearance = WindowAppearance::new();

    let d1 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert!(d1.has_changes());

    let d2 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(
        d2,
        AppearanceDiff {
            channel: Some(BackdropChannel::Off),
            dark_mode: None,
            lock_mica: Some(false),
            clear_alpha: Some(1.0),
        }
    );
    assert_eq!(appearance.applied_clear_alpha(), 1.0);

    let d3 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert!(!d3.has_changes());
}

#[test]
fn explicit_host_clear_alpha_overridden_when_intent_active() {
    let mut appearance = WindowAppearance::new();
    appearance.set_host_clear_alpha(0.5);

    let d1 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(d1.clear_alpha, Some(0.5));
    assert_eq!(appearance.applied_clear_alpha(), 0.5);

    let d2 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(d2.clear_alpha, Some(0.0));
    assert_eq!(appearance.applied_clear_alpha(), 0.0);

    let d3 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(d3.clear_alpha, Some(0.5));
    assert_eq!(appearance.applied_clear_alpha(), 0.5);
}

#[test]
fn none_variant_same_as_none_option() {
    let mut appearance = WindowAppearance::new();
    let diff = appearance.set(
        Some(WindowBackdropMaterial::None),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );

    assert_eq!(
        diff,
        AppearanceDiff {
            channel: None,
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );
    assert!(!diff.has_changes());
}

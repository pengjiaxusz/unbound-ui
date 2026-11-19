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

#[test]
fn runtime_continuous_material_switching_and_deduplication() {
    let mut appearance = WindowAppearance::new();

    // 1. 跟随系统 (FollowSystem)
    let d1 = appearance.set(
        Some(WindowBackdropMaterial::System),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d1,
        AppearanceDiff {
            channel: Some(BackdropChannel::Dwm(WindowBackdropMaterial::System)),
            dark_mode: Some(true),
            lock_mica: Some(true),
            clear_alpha: Some(0.0),
        }
    );

    // 2. 切到云母 (Mica)：仅通道变化，深色/锁定/清屏均保持不变
    let d2 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d2,
        AppearanceDiff {
            channel: Some(BackdropChannel::Dwm(WindowBackdropMaterial::Mica)),
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );

    // 3. 切到 Mica Alt (Tabbed)：仅通道变化
    let d3 = appearance.set(
        Some(WindowBackdropMaterial::Tabbed),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d3,
        AppearanceDiff {
            channel: Some(BackdropChannel::Dwm(WindowBackdropMaterial::Tabbed)),
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );

    // 4. 切到亚克力 (Acrylic)：仅通道变化
    let d4 = appearance.set(
        Some(WindowBackdropMaterial::Acrylic),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d4,
        AppearanceDiff {
            channel: Some(BackdropChannel::Dwm(WindowBackdropMaterial::Acrylic)),
            dark_mode: None,
            lock_mica: None,
            clear_alpha: None,
        }
    );

    // 5. 重复设置亚克力：零写入
    let d5 = appearance.set(
        Some(WindowBackdropMaterial::Acrylic),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert!(!d5.has_changes());

    // 6. 可逆关闭：切回 None，发出 Off、清屏恢复 1.0、失焦锁定关闭
    let d6 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(
        d6,
        AppearanceDiff {
            channel: Some(BackdropChannel::Off),
            dark_mode: None,
            lock_mica: Some(false),
            clear_alpha: Some(1.0),
        }
    );

    // 7. 重复关闭：零写入
    let d7 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert!(!d7.has_changes());

    // 8. 重复使用 Some(WindowBackdropMaterial::None) 关闭：零写入
    let d8 = appearance.set(
        Some(WindowBackdropMaterial::None),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert!(!d8.has_changes());

    // 9. 重新开启云母：恢复云母材质通道、清屏透明与失焦锁定
    let d9 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(
        d9,
        AppearanceDiff {
            channel: Some(BackdropChannel::Dwm(WindowBackdropMaterial::Mica)),
            dark_mode: None,
            lock_mica: Some(true),
            clear_alpha: Some(0.0),
        }
    );

    // 10. 重复开启云母：零写入
    let d10 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert!(!d10.has_changes());
}

#[test]
fn accent_policy_unavailable_does_not_fallback_to_dwm_acrylic() {
    let mut appearance = WindowAppearance::new();
    let caps_no_accent = AppearanceCaps::new(Some(22621), true, false);
    let tint_argb = 0xCCD07800;

    // 当 accent policy 导出不可用时，自定义色调按不可用处理，绝不回退到 DWM 亚克力
    let diff = appearance.set_accent_tint(tint_argb, caps_no_accent, ThemeKind::Dark);
    assert!(!diff.has_changes());
    assert_eq!(appearance.applied_clear_alpha(), 1.0);

    // 从云母开启态切到不可用的自定义色调：应直接关闭后景，绝不回退到 DWM 亚克力
    let d1 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        caps_no_accent,
        ThemeKind::Dark,
    );
    assert_eq!(
        d1.channel,
        Some(BackdropChannel::Dwm(WindowBackdropMaterial::Mica))
    );

    let d2 = appearance.set_accent_tint(tint_argb, caps_no_accent, ThemeKind::Dark);
    assert_eq!(d2.channel, Some(BackdropChannel::Off));
    assert_eq!(d2.lock_mica, Some(false));
    assert_eq!(d2.clear_alpha, Some(1.0));
    assert_ne!(
        d2.channel,
        Some(BackdropChannel::Dwm(WindowBackdropMaterial::Acrylic))
    );
}

#[test]
fn lock_mica_strictly_tracks_effective_native_material() {
    let mut appearance = WindowAppearance::new();

    // 1. 原生云母在场：锁定
    let d1 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(d1.lock_mica, Some(true));

    // 2. 切回无后景：不锁定
    let d2 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(d2.lock_mica, Some(false));

    // 3. 原生亚克力在场：锁定
    let d3 = appearance.set(
        Some(WindowBackdropMaterial::Acrylic),
        CAPS_WIN11_NATIVE_OK,
        ThemeKind::Dark,
    );
    assert_eq!(d3.lock_mica, Some(true));

    // 4. 原生自定义色调在场：锁定
    let d4 = appearance.set_accent_tint(0xCCD07800, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    // 从 Acrylic (lock=true) 到 AccentTint (lock=true)，锁定状态无变化所以 diff 为 None
    assert_eq!(d4.lock_mica, None);

    // 5. 切到关闭：取消锁定
    let d5 = appearance.set(None, CAPS_WIN11_NATIVE_OK, ThemeKind::Dark);
    assert_eq!(d5.lock_mica, Some(false));

    // 6. 构建号过低 (Win10 19045) 请求云母：不满足原生门槛，不锁定
    let caps_win10 = AppearanceCaps::new(Some(19045), true, true);
    let d6 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        caps_win10,
        ThemeKind::Dark,
    );
    assert_eq!(d6.lock_mica, None); // 未从 false 变化

    // 7. 表面不支持预乘 alpha 请求云母：不满足原生门槛，不锁定
    let caps_no_pm = AppearanceCaps::new(Some(22621), false, true);
    let d7 = appearance.set(
        Some(WindowBackdropMaterial::Mica),
        caps_no_pm,
        ThemeKind::Dark,
    );
    assert_eq!(d7.lock_mica, None);
}

struct DummyApp;
impl unbound_app::Application for DummyApp {
    type State = ();
    fn init(&mut self, _ctx: &mut unbound_app::AppContext) -> Self::State {}
    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: unbound_app::AppEvent,
        _ctx: &mut unbound_app::AppContext,
    ) -> unbound_app::EventOutcome {
        unbound_app::EventOutcome::new(unbound_app::DirtyFlags::NONE)
    }
    fn view(&mut self, _state: &Self::State, _view: &mut unbound_app::ViewBuilder<'_>) {}
}

#[test]
fn app_runtime_backdrop_apis_roundtrip() {
    use unbound_app::{AppRuntime, AppearanceIntent};
    use unbound_math::Vec2;

    let mut runtime = AppRuntime::new(DummyApp, Vec2::new(800.0, 600.0));
    assert_eq!(runtime.backdrop(), None);
    assert_eq!(runtime.backdrop_intent(), AppearanceIntent::Material(None));

    runtime.set_backdrop(Some(WindowBackdropMaterial::Mica));
    assert_eq!(runtime.backdrop(), Some(WindowBackdropMaterial::Mica));
    assert_eq!(
        runtime.backdrop_intent(),
        AppearanceIntent::Material(Some(WindowBackdropMaterial::Mica))
    );

    runtime.set_backdrop(Some(WindowBackdropMaterial::Acrylic));
    assert_eq!(runtime.backdrop(), Some(WindowBackdropMaterial::Acrylic));

    runtime.set_backdrop(None);
    assert_eq!(runtime.backdrop(), None);
    assert_eq!(runtime.backdrop_intent(), AppearanceIntent::Material(None));

    let tint = 0xCCD07800;
    runtime.set_accent_tint(tint);
    assert_eq!(runtime.backdrop(), None);
    assert_eq!(
        runtime.backdrop_intent(),
        AppearanceIntent::AccentTint(tint)
    );

    runtime.set_backdrop(Some(WindowBackdropMaterial::System));
    assert_eq!(runtime.backdrop(), Some(WindowBackdropMaterial::System));
}

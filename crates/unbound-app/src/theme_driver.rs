//! 框架级统一主题过渡驱动器（ADR 0034）。
//!
//! 所有交互式宿主（单窗运行器 `WindowedRunner`、多窗协调器 `MultiWindowCoordinator`、
//! 展厅 `apps/showcase` 及自定义主循环宿主）均通过 [`ThemeTransitionDriver`] 驱动主题切换与
//! 4 KiB LUT 上传，避免各自重复维护 `ThemeState` / 时钟 / 减弱动画逻辑而漏掉过渡动画。

use std::time::Instant;

use unbound_gpu::frame::Renderer;
use unbound_math::color::Color;
use unbound_theme::{
    AccentBrand, Palette, ProductPreset, ThemeKind, ThemeLut, ThemeSet, ThemeState,
    ThemeTransition, ThemeTransitionConfig,
};

/// 主题快照识别键（明暗种类、产品预设、品牌强调色与系统强调色比特表示）。
pub type ThemeSnapshotKey = (ThemeKind, ProductPreset, AccentBrand, Option<[u32; 4]>);

/// 从主题配置构造标准 [`ThemeSnapshotKey`]。
#[must_use]
pub fn make_theme_snapshot_key(
    theme: ThemeKind,
    preset: ProductPreset,
    accent: AccentBrand,
    system_accent: Option<Color>,
) -> ThemeSnapshotKey {
    let sys_bits =
        system_accent.map(|c| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()]);
    (theme, preset, accent, sys_bits)
}

/// 单次主题同步的结果摘要。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ThemeSyncOutcome {
    /// 本帧是否需要向渲染器上传（或已上传）4 KiB LUT。
    pub lut_updated: bool,
    /// 主题过渡是否仍在进行中（为 `true` 时宿主必须继续请求下一帧）。
    pub animating: bool,
    /// 当前过渡剩余时长（秒），供自绘后景双纹理叠化继承。
    pub remaining_secs: f32,
    /// 原生后景（如 DWM `DWMWA_USE_IMMERSIVE_DARK_MODE`）应同步的目标主题。
    pub backdrop_theme: ThemeKind,
}

/// 统一主题过渡驱动器。
///
/// 持有内部 [`ThemeTransition`] 与单调时钟，支持两种使用模式：
/// - 通用预设模式（[`Self::sync_preset`]）：面向 `WindowedRunner` / `MultiWindowCoordinator`；
/// - 自定义调色板模式（[`Self::sync_palette`]）：面向持有自有 `ThemeSet` 的自定义宿主（如 `apps/showcase`）。
#[derive(Clone, Debug)]
pub struct ThemeTransitionDriver {
    theme: ThemeState,
    transition: ThemeTransition,
    clock_epoch: Instant,
    /// 最近一次已同步的通用预设快照键；`None` 表示尚未完成首帧同步（首帧强制瞬切）。
    uploaded_preset: Option<ThemeSnapshotKey>,
    /// 最近一次已同步的自定义调色板标识；`None` 表示尚未完成首帧同步（首帧强制瞬切）。
    uploaded_custom_id: Option<u64>,
}

impl ThemeTransitionDriver {
    /// 使用指定 [`ThemeSet`] 与初始主题构造驱动器（初始处于静止态，首帧强制瞬切）。
    #[must_use]
    pub fn new(set: ThemeSet, initial: ThemeKind) -> Self {
        let theme = ThemeState::new(set, initial);
        let transition = ThemeTransition::new(theme.active_palette());
        Self {
            theme,
            transition,
            clock_epoch: Instant::now(),
            uploaded_preset: None,
            uploaded_custom_id: None,
        }
    }

    /// 标记已上传快照失效（如 Vulkan 设备重建后），确保下一帧强制瞬切重传 4 KiB LUT。
    pub fn invalidate_uploaded(&mut self) {
        self.uploaded_preset = None;
        self.uploaded_custom_id = None;
    }

    /// 当前主题过渡是否正在进行中。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.transition.is_active()
    }

    /// 当前线性进度（`0.0..=1.0`，静止时恒为 `1.0`）。
    #[must_use]
    pub fn progress(&self) -> f32 {
        self.transition.progress()
    }

    /// 当前目标主题种类。
    #[must_use]
    pub const fn target_kind(&self) -> ThemeKind {
        self.transition.target_kind()
    }

    /// 当前原生后景应采用的主题（按 ADR 0034 修订版，从 `t = 0` 即刻返回目标主题）。
    #[must_use]
    pub const fn effective_backdrop_theme(&self) -> ThemeKind {
        self.transition.effective_backdrop_theme()
    }

    /// 当前帧可直接上传至 GPU 的 4 KiB 主题 LUT。
    #[must_use]
    pub const fn lut(&self) -> &ThemeLut {
        self.transition.lut()
    }

    /// 返回自驱动器创建以来的单调时间（秒）。
    #[must_use]
    pub fn elapsed_secs(&self) -> f64 {
        self.clock_epoch.elapsed().as_secs_f64()
    }

    /// 立即将主题与过渡状态吸附至给定预设快照（用于 `Suspended` 恢复或无头/差分强制瞬切）。
    pub fn snap_preset(
        &mut self,
        key: ThemeSnapshotKey,
        system_accent: Option<Color>,
        renderer: Option<&mut Renderer>,
    ) {
        self.theme = ThemeState::new(
            ThemeSet::for_product_with_system_accent(key.1, key.2, system_accent),
            key.0,
        );
        self.transition.snap_to(self.theme.active_palette());
        self.uploaded_preset = Some(key);
        if let Some(renderer) = renderer {
            let _ = renderer.write_theme_lut(self.transition.lut());
        }
    }

    /// 同步通用预设主题并在需要时推进过渡（不绑定真实渲染器，便于无头单测与多窗复用）。
    pub fn sync_preset_at(
        &mut self,
        key: ThemeSnapshotKey,
        system_accent: Option<Color>,
        config: ThemeTransitionConfig,
        now_secs: f64,
        trigger_secs: f64,
        reduced_motion: bool,
    ) -> ThemeSyncOutcome {
        let key_changed = self.uploaded_preset != Some(key);
        if !key_changed && !self.transition.is_active() {
            return ThemeSyncOutcome {
                lut_updated: false,
                animating: false,
                remaining_secs: 0.0,
                backdrop_theme: self.transition.effective_backdrop_theme(),
            };
        }

        if key_changed {
            let prev_preset_accent = self.uploaded_preset.map(|(_, p, a, sa)| (p, a, sa));
            if prev_preset_accent != Some((key.1, key.2, key.3)) {
                self.theme = ThemeState::new(
                    ThemeSet::for_product_with_system_accent(key.1, key.2, system_accent),
                    key.0,
                );
            } else {
                let _ = self.theme.switch(key.0);
            }
            if self.uploaded_preset.is_none() {
                self.transition.snap_to(self.theme.active_palette());
            } else {
                let duration = config.effective_duration_secs(reduced_motion);
                let _ = self
                    .transition
                    .begin(self.theme.active_palette(), trigger_secs, duration);
            }
        }

        let still_active = self.transition.advance(now_secs);
        self.uploaded_preset = Some(key);
        ThemeSyncOutcome {
            lut_updated: still_active || key_changed,
            animating: still_active,
            remaining_secs: self.transition.remaining_secs(now_secs),
            backdrop_theme: self.transition.effective_backdrop_theme(),
        }
    }

    /// 按显式 `reduced_motion` 策略同步通用预设主题并直接上传 4 KiB LUT 至渲染器。
    pub fn sync_preset_with_policy(
        &mut self,
        key: ThemeSnapshotKey,
        system_accent: Option<Color>,
        config: ThemeTransitionConfig,
        reduced_motion: bool,
        renderer: Option<&mut Renderer>,
    ) -> ThemeSyncOutcome {
        let now_secs = self.elapsed_secs();
        let outcome = self.sync_preset_at(
            key,
            system_accent,
            config,
            now_secs,
            now_secs,
            reduced_motion,
        );
        if outcome.lut_updated {
            if let Some(renderer) = renderer {
                if let Err(error) = renderer.write_theme_lut(self.transition.lut()) {
                    eprintln!("[unbound-app] Could not upload the theme LUT: {error:?}");
                }
            }
        }
        outcome
    }

    /// 同步通用预设主题并直接上传 4 KiB LUT 至渲染器。
    pub fn sync_preset(
        &mut self,
        key: ThemeSnapshotKey,
        system_accent: Option<Color>,
        config: ThemeTransitionConfig,
        renderer: Option<&mut Renderer>,
    ) -> ThemeSyncOutcome {
        self.sync_preset_with_policy(
            key,
            system_accent,
            config,
            unbound_platform::prefers_reduced_motion(),
            renderer,
        )
    }

    /// 同步自定义调色板（供 `apps/showcase` 等持有自有 `ThemeSet` 的宿主使用）并推进过渡。
    ///
    /// 当 `force_instant` 为 `true`（如屏幕捕获对照或差分采集）或首帧同步时强制瞬切。
    pub fn sync_palette_at(
        &mut self,
        id: u64,
        palette: &Palette,
        config: ThemeTransitionConfig,
        now_secs: f64,
        reduced_motion: bool,
        force_instant: bool,
    ) -> ThemeSyncOutcome {
        let id_changed = self.uploaded_custom_id != Some(id);
        if !id_changed && !self.transition.is_active() {
            return ThemeSyncOutcome {
                lut_updated: false,
                animating: false,
                remaining_secs: 0.0,
                backdrop_theme: self.transition.effective_backdrop_theme(),
            };
        }

        if id_changed {
            if self.uploaded_custom_id.is_none() || force_instant {
                self.transition.snap_to(palette);
            } else {
                let duration = config.effective_duration_secs(reduced_motion);
                let _ = self.transition.begin(palette, now_secs, duration);
            }
        }

        let still_active = self.transition.advance(now_secs);
        self.uploaded_custom_id = Some(id);
        ThemeSyncOutcome {
            lut_updated: still_active || id_changed,
            animating: still_active,
            remaining_secs: self.transition.remaining_secs(now_secs),
            backdrop_theme: self.transition.effective_backdrop_theme(),
        }
    }

    /// 同步自定义调色板并直接上传 4 KiB LUT 至渲染器。
    pub fn sync_palette(
        &mut self,
        id: u64,
        palette: &Palette,
        config: ThemeTransitionConfig,
        force_instant: bool,
        renderer: Option<&mut Renderer>,
    ) -> ThemeSyncOutcome {
        let now_secs = self.elapsed_secs();
        let reduced_motion = unbound_platform::prefers_reduced_motion();
        let outcome =
            self.sync_palette_at(id, palette, config, now_secs, reduced_motion, force_instant);
        if outcome.lut_updated {
            if let Some(renderer) = renderer {
                if let Err(error) = renderer.write_theme_lut(self.transition.lut()) {
                    eprintln!("[unbound-app] Could not upload the theme LUT: {error:?}");
                }
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_palette_driver_snaps_first_frame_and_animates_runtime_switch() {
        let set = ThemeSet::default_set();
        let light = set.palette(ThemeKind::Light);
        let dark = set.palette(ThemeKind::Dark);
        let cfg = ThemeTransitionConfig::default();

        let mut driver = ThemeTransitionDriver::new(set.clone(), ThemeKind::Dark);
        // 首帧强制瞬切。
        let first = driver.sync_palette_at(1, dark, cfg, 0.0, false, false);
        assert!(first.lut_updated);
        assert!(!first.animating);
        assert_eq!(first.backdrop_theme, ThemeKind::Dark);

        // 运行期切至浅色：启动 240ms 过渡，且原生后景在 t = 0 立即返回 Light。
        let start = driver.sync_palette_at(2, light, cfg, 1.0, false, false);
        assert!(start.lut_updated);
        assert!(start.animating);
        assert_eq!(start.backdrop_theme, ThemeKind::Light);

        // 中段仍在动画中。
        let mid = driver.sync_palette_at(2, light, cfg, 1.12, false, false);
        assert!(mid.lut_updated);
        assert!(mid.animating);
        assert!(mid.remaining_secs > 0.0);

        // 收敛后停止唤醒。
        let end = driver.sync_palette_at(2, light, cfg, 1.25, false, false);
        assert!(!end.animating);
        assert_eq!(end.remaining_secs, 0.0);

        // 稳态帧不再上传 LUT。
        let idle = driver.sync_palette_at(2, light, cfg, 1.30, false, false);
        assert!(!idle.lut_updated);
        assert!(!idle.animating);
    }

    #[test]
    fn force_instant_bypasses_animation_for_capture_compare_mode() {
        let set = ThemeSet::default_set();
        let light = set.palette(ThemeKind::Light);
        let dark = set.palette(ThemeKind::Dark);
        let cfg = ThemeTransitionConfig::default();

        let mut driver = ThemeTransitionDriver::new(set.clone(), ThemeKind::Dark);
        let _ = driver.sync_palette_at(1, dark, cfg, 0.0, false, false);
        let snapped = driver.sync_palette_at(2, light, cfg, 1.0, false, true);
        assert!(snapped.lut_updated);
        assert!(!snapped.animating);
        assert_eq!(snapped.backdrop_theme, ThemeKind::Light);
    }
}

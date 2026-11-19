//! 深度窗口后景控制模块（Window Backdrop Controller）。
//! lang-allow: Chinese backdrop localized names and descriptions
//!
//! 内聚平台能力探测（构建号、强调色通道导出、预乘 Alpha 支持）、
//! 双路径计划推导（原生与自绘）、系统写入互斥下发与去重（DWM 材质、Accent Policy、深色模式），
//! 以及壁纸纹理生成与视图提交生命周期驱动。

use unbound_backdrop::{
    BackdropInputs, BackdropPlan, BackdropSelection, BackdropSource, FallbackReason, MaterialKind,
    PixelRect, DEFAULT_ACRYLIC_TINT_ARGB,
};
use unbound_theme::ThemeKind;

use crate::appearance::{
    apply_appearance_diff_to_platform, AppearanceCaps, AppearanceDiff, WindowAppearance,
};
use crate::{MicaBackdrop, ViewBuilder};

/// 获取材质种类的界面中文名。
#[must_use]
pub const fn material_name_zh(material: MaterialKind) -> &'static str {
    match material {
        MaterialKind::None => "纯色",
        MaterialKind::FollowSystem => "跟随系统",
        MaterialKind::Mica => "云母",
        MaterialKind::MicaAlt => "云母变体",
        MaterialKind::Acrylic => "亚克力",
        MaterialKind::Immersive => "沉浸式后景",
        MaterialKind::AcrylicTint => "亚克力（自定义色调）",
    }
}

/// 获取回退原因的中文说明。
#[must_use]
pub fn fallback_reason_zh(reason: FallbackReason) -> String {
    match reason {
        FallbackReason::NativeUnavailable { material } => {
            format!(
                "{} 无法由系统原生合成，已改走自绘",
                material_name_zh(material)
            )
        }
        FallbackReason::NoSelfDrawnVariant { material } => {
            format!(
                "{} 没有自绘变体，已按更接近的材质回退",
                material_name_zh(material)
            )
        }
    }
}

/// 将后景计划翻译为落点与回退说明（中文）。
#[must_use]
pub fn plan_summary_zh(plan: &BackdropPlan) -> String {
    if plan.material_kind == MaterialKind::None {
        return "纯色 · 不生成后景纹理".to_owned();
    }
    let source_str = match plan.source {
        BackdropSource::Native => "系统原生",
        BackdropSource::SelfDrawn => "引擎自绘",
        BackdropSource::Auto => "自动",
    };
    let mut summary = format!("{} · {source_str}", material_name_zh(plan.material_kind));
    if let Some(reason) = plan.fallback {
        summary.push('（');
        summary.push_str(&fallback_reason_zh(reason));
        summary.push('）');
    }
    summary
}

/// 深度窗口后景控制器。
#[derive(Debug)]
pub struct WindowBackdropController {
    caps: AppearanceCaps,
    appearance: WindowAppearance,
    mica: MicaBackdrop,
    plan: BackdropPlan,
    dark: bool,
    accent_tint_argb: u32,
}

impl Default for WindowBackdropController {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowBackdropController {
    /// 创建后景控制器，自动探测当前宿主 Windows 构建号与 accent policy 导出。
    #[must_use]
    pub fn new() -> Self {
        Self::with_caps(AppearanceCaps::detect(false))
    }

    /// 使用指定平台能力集创建后景控制器（供测试与离线模拟使用）。
    #[must_use]
    pub fn with_caps(caps: AppearanceCaps) -> Self {
        Self {
            caps,
            appearance: WindowAppearance::new(),
            mica: MicaBackdrop::new(),
            plan: BackdropPlan::default(),
            dark: false,
            accent_tint_argb: DEFAULT_ACRYLIC_TINT_ARGB,
        }
    }

    /// 设置表面合成是否支持预乘 Alpha。
    pub fn set_premultiplied_alpha(&mut self, supported: bool) {
        self.caps.premultiplied_alpha_supported = supported;
    }

    /// 当前生效的平台能力快照。
    #[must_use]
    pub const fn caps(&self) -> AppearanceCaps {
        self.caps
    }

    /// 设置自定义强调色色调（ARGB 格式）。
    pub fn set_accent_tint_argb(&mut self, argb: u32) {
        self.accent_tint_argb = argb;
    }

    /// 当前配置的强调色色调。
    #[must_use]
    pub const fn accent_tint_argb(&self) -> u32 {
        self.accent_tint_argb
    }

    /// 推导后景计划的唯一接缝（纯函数逻辑）。
    #[must_use]
    pub fn derive_plan(
        &self,
        selection: BackdropSelection,
        dark: bool,
        window_rect: Option<PixelRect>,
        self_drawn_ready: bool,
    ) -> BackdropPlan {
        BackdropPlan::derive(BackdropInputs {
            selection,
            windows_build: self.caps.windows_build,
            premultiplied_alpha_supported: self.caps.premultiplied_alpha_supported,
            accent_policy_export: self.caps.accent_policy_export,
            accent_tint_argb: self.accent_tint_argb,
            window_screen_rect: window_rect,
            primary_screen_rect: unbound_backdrop::primary_screen_rect(),
            primary_screen_scale: unbound_backdrop::primary_screen_scale(),
            dark,
            self_drawn_ready,
        })
    }

    /// 刷新当前后景计划，返回计划是否发生变化。
    pub fn update_plan(
        &mut self,
        selection: BackdropSelection,
        dark: bool,
        window_rect: Option<PixelRect>,
    ) -> bool {
        self.dark = dark;
        let next_plan = self.derive_plan(selection, dark, window_rect, self.mica.visible());
        let changed = next_plan != self.plan;
        self.plan = next_plan;
        changed
    }

    /// 当前环境是否为深色模式。
    #[must_use]
    pub const fn dark(&self) -> bool {
        self.dark
    }

    /// 材质种类快捷刷新：来源自动取 Auto。
    pub fn update_plan_for_material(
        &mut self,
        material: MaterialKind,
        dark: bool,
        window_rect: Option<PixelRect>,
    ) -> bool {
        self.update_plan(
            BackdropSelection::new(material, BackdropSource::Auto),
            dark,
            window_rect,
        )
    }

    /// 最近一次推导的后景计划。
    #[must_use]
    pub const fn plan(&self) -> &BackdropPlan {
        &self.plan
    }

    /// 当前计划是否需要引擎自绘壁纸纹理。
    #[must_use]
    pub fn needs_self_drawn(&self) -> bool {
        self.plan.material_kind != MaterialKind::None
            && self.plan.source == BackdropSource::SelfDrawn
    }

    /// 壁纸背板生成与提交组件的不可变引用。
    #[must_use]
    pub const fn mica(&self) -> &MicaBackdrop {
        &self.mica
    }

    /// 壁纸背板生成与提交组件的可变引用。
    pub fn mica_mut(&mut self) -> &mut MicaBackdrop {
        &mut self.mica
    }

    /// 是否还需要后续帧才能完成云母呈现。
    #[must_use]
    pub fn needs_frame(&self) -> bool {
        self.mica.needs_frame()
    }

    /// 将计划推导出的系统材质、强调色调及深色模式设置下发到原生 Win32 窗口。
    ///
    /// 内部借助 [`WindowAppearance`] 账本执行增量去重，相同取值不产生多余的 Win32 API 调用。
    pub fn apply_to_window(&mut self, hwnd: isize, dark: bool) -> AppearanceDiff {
        if hwnd == 0 {
            return AppearanceDiff {
                channel: None,
                dark_mode: None,
                lock_mica: None,
                clear_alpha: None,
            };
        }
        let theme = if dark {
            ThemeKind::Dark
        } else {
            ThemeKind::Light
        };
        let diff = self.appearance.set_plan(&self.plan, self.caps, theme);
        apply_appearance_diff_to_platform(&diff, hwnd, None);
        diff
    }

    /// 快捷下发：按指定材质选择与明暗更新计划并立即下发到窗口。
    pub fn apply_material(
        &mut self,
        hwnd: isize,
        selection: BackdropSelection,
        dark: bool,
    ) -> AppearanceDiff {
        self.update_plan(selection, dark, None);
        self.apply_to_window(hwnd, dark)
    }

    /// 同步深色模式到原生窗口（内部自动按已下发值去重）。
    pub fn sync_dark_mode(&mut self, hwnd: isize, dark: bool) {
        if hwnd == 0 {
            return;
        }
        let theme = if dark {
            ThemeKind::Dark
        } else {
            ThemeKind::Light
        };
        let diff = self.appearance.set_plan(&self.plan, self.caps, theme);
        if let Some(target_dark) = diff.dark_mode {
            unbound_platform::set_window_dark_mode(hwnd, target_dark);
        }
    }

    /// 视图阶段快捷驱动方法：更新计划、下发系统设置、同步自绘纹理并在整窗底层绘制。
    ///
    /// 返回 `(mica_visible, plan_changed)`。
    pub fn view_frame(
        &mut self,
        view: &mut ViewBuilder<'_>,
        key: u64,
        hwnd: isize,
        selection: BackdropSelection,
        dark: bool,
        window_rect: Option<PixelRect>,
    ) -> (bool, bool) {
        let plan_changed = self.update_plan(selection, dark, window_rect);
        if hwnd != 0 {
            self.apply_to_window(hwnd, dark);
        }
        let self_drawn = self.needs_self_drawn();
        self.mica.set_enabled(self_drawn);
        if self_drawn {
            let _ = self
                .mica
                .sync(self.plan.texture, self.plan.uv, view.event_sender());
        }
        let visible = self.mica.draw(view, key);
        (visible, plan_changed)
    }

    /// 获取当前计划的落点与生效路径说明（中文）。
    #[must_use]
    pub fn summary_zh(&self) -> String {
        plan_summary_zh(&self.plan)
    }

    /// 生成指定选择与明暗下的落点说明（中文）。
    #[must_use]
    pub fn destination_summary(&self, selection: BackdropSelection, dark: bool) -> String {
        let plan = self.derive_plan(selection, dark, None, false);
        plan_summary_zh(&plan)
    }

    /// 静态辅助方法：快速推导给定选择、平台能力与明暗环境下的落点说明（中文）。
    #[must_use]
    pub fn explain_destination(
        selection: BackdropSelection,
        caps: AppearanceCaps,
        dark: bool,
    ) -> String {
        let plan = BackdropPlan::derive(BackdropInputs {
            selection,
            windows_build: caps.windows_build,
            premultiplied_alpha_supported: caps.premultiplied_alpha_supported,
            accent_policy_export: caps.accent_policy_export,
            accent_tint_argb: DEFAULT_ACRYLIC_TINT_ARGB,
            window_screen_rect: None,
            primary_screen_rect: unbound_backdrop::primary_screen_rect(),
            primary_screen_scale: unbound_backdrop::primary_screen_scale(),
            dark,
            self_drawn_ready: false,
        });
        plan_summary_zh(&plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_initialization_and_plan_derivation() {
        let caps = AppearanceCaps::new(Some(22631), true, true);
        let mut ctrl = WindowBackdropController::with_caps(caps);
        assert_eq!(ctrl.caps().windows_build, Some(22631));
        assert!(ctrl.caps().premultiplied_alpha_supported);

        let selection = BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto);
        assert!(ctrl.update_plan(selection, true, None));
        assert_eq!(ctrl.plan().material_kind, MaterialKind::Mica);
        assert_eq!(ctrl.plan().source, BackdropSource::Native);
        assert!(!ctrl.needs_self_drawn());
        assert_eq!(ctrl.plan().clear_alpha, 0.0);
    }

    #[test]
    fn controller_fallback_to_self_drawn_when_not_premultiplied() {
        let caps = AppearanceCaps::new(Some(22631), false, true);
        let mut ctrl = WindowBackdropController::with_caps(caps);
        let selection = BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto);
        assert!(ctrl.update_plan(selection, false, None));
        assert_eq!(ctrl.plan().material_kind, MaterialKind::Mica);
        assert_eq!(ctrl.plan().source, BackdropSource::SelfDrawn);
        assert!(ctrl.needs_self_drawn());
        assert_eq!(ctrl.plan().clear_alpha, 1.0);
    }

    #[test]
    fn controller_idempotent_plan_updates() {
        let caps = AppearanceCaps::new(Some(22631), true, true);
        let mut ctrl = WindowBackdropController::with_caps(caps);
        let selection = BackdropSelection::new(MaterialKind::MicaAlt, BackdropSource::Auto);
        assert!(ctrl.update_plan(selection, true, None));
        assert!(!ctrl.update_plan(selection, true, None));
    }

    #[test]
    fn controller_destination_summary_zh() {
        let caps = AppearanceCaps::new(Some(22631), true, true);
        let ctrl = WindowBackdropController::with_caps(caps);
        let summary = ctrl.destination_summary(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto),
            true,
        );
        assert!(summary.contains("云母"));
        assert!(summary.contains("系统原生"));

        let none_summary = ctrl.destination_summary(
            BackdropSelection::new(MaterialKind::None, BackdropSource::Auto),
            true,
        );
        assert!(none_summary.contains("纯色"));
    }

    #[test]
    fn controller_explain_destination_static_helper() {
        let caps = AppearanceCaps::new(Some(22631), true, true);
        let summary = WindowBackdropController::explain_destination(
            BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::Auto),
            caps,
            false,
        );
        assert!(summary.contains("亚克力"));
        assert!(summary.contains("系统原生"));
    }
}

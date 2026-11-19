//! 窗口外观意图账本与系统写入去重接缝。
//!
//! 职责恰好三件：
//! 1. 把外观意图、平台能力与当前主题收敛成一份后景计划（复用既有的 [`BackdropPlan::derive`]）；
//! 2. 把后景计划翻译成系统写入请求与状态标记；
//! 3. 按「已下发值」逐帧去重，仅输出需要执行的写入增量（[`AppearanceDiff`]）。
//!
//! 组件本身纯值进、Diff 出，不执行任何系统调用或 GPU 操作，在无窗口、无 GPU 环境下可完全自动化断言。

use unbound_backdrop::{
    BackdropInputs, BackdropPlan, BackdropSelection, BackdropSource, DwmMaterial, MaterialKind,
};
use unbound_platform::WindowBackdropMaterial;
use unbound_theme::ThemeKind;

/// 平台后景通道请求。
///
/// 具备互斥语义：
/// - `Dwm(material)`：走 `DWMWA_SYSTEMBACKDROP_TYPE` 通道，执行方先清除 accent policy 再设置 DWM；
/// - `AccentTint(argb)`：走 `SetWindowCompositionAttribute` 的 accent policy 通道，执行方不碰 DWM；
/// - `Off`：关闭后景，执行方清除 accent policy 并将 DWM 材质设为 None。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BackdropChannel {
    /// 走 `DWMWA_SYSTEMBACKDROP_TYPE` 系统材质通道。
    Dwm(WindowBackdropMaterial),
    /// 走 `SetWindowCompositionAttribute` 的 accent policy 通道（ABGR 格式）。
    AccentTint(u32),
    /// 关闭后景通道（恢复不透明）。
    Off,
}

/// 窗口外观意图账本所需的平台能力输入（纯值，无系统查询）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AppearanceCaps {
    /// Windows 系统内部版本构建号（如 22621）。
    pub windows_build: Option<u32>,
    /// 当前 Vulkan 表面是否支持/启用了预乘 Alpha 合成。
    pub premultiplied_alpha_supported: bool,
    /// `SetWindowCompositionAttribute`（accent policy）导出函数是否可用。
    pub accent_policy_export: bool,
}

impl AppearanceCaps {
    /// 构造新的外观能力集。
    #[must_use]
    pub const fn new(
        windows_build: Option<u32>,
        premultiplied_alpha_supported: bool,
        accent_policy_export: bool,
    ) -> Self {
        Self {
            windows_build,
            premultiplied_alpha_supported,
            accent_policy_export,
        }
    }
}

/// 一次外观同步需要执行的系统写入；`None` 表示与已下发值一致，不必再写。
#[derive(Clone, Debug, PartialEq)]
pub struct AppearanceDiff {
    /// 系统后景通道变更；`None` 表示通道与参数无变化。
    pub channel: Option<BackdropChannel>,
    /// 沉浸式深色模式标记；`None` 表示深色状态无变化。
    pub dark_mode: Option<bool>,
    /// 失焦激活态锁定；`None` 表示锁定状态无变化。
    pub lock_mica: Option<bool>,
    /// 清屏 Alpha；`None` 表示清屏透明度无变化。
    pub clear_alpha: Option<f32>,
}

impl AppearanceDiff {
    /// 是否存在任何需要执行的变更。
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.channel.is_some()
            || self.dark_mode.is_some()
            || self.lock_mica.is_some()
            || self.clear_alpha.is_some()
    }
}

/// 外观意图种类。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AppearanceIntent {
    /// 标准 DWM 材质意图（`None` 或 `Some(WindowBackdropMaterial::None)` 表示关闭）。
    Material(Option<WindowBackdropMaterial>),
    /// 亚克力自定义色调（走 accent policy 通道，ABGR 格式）。
    AccentTint(u32),
}

/// 将 DWM 内部材质枚举映射为平台层公开的 [`WindowBackdropMaterial`]。
#[must_use]
pub const fn dwm_material_to_platform(dwm: DwmMaterial) -> WindowBackdropMaterial {
    match dwm {
        DwmMaterial::None => WindowBackdropMaterial::None,
        DwmMaterial::System => WindowBackdropMaterial::System,
        DwmMaterial::Mica => WindowBackdropMaterial::Mica,
        DwmMaterial::MicaAlt => WindowBackdropMaterial::Tabbed,
        DwmMaterial::Acrylic => WindowBackdropMaterial::Acrylic,
        DwmMaterial::Immersive => WindowBackdropMaterial::None,
    }
}

/// 把后景计划翻译为后景通道与失焦锁定状态。
#[must_use]
pub fn backdrop_channel_request(plan: &BackdropPlan) -> (BackdropChannel, bool) {
    let channel = if let Some(argb) = plan.accent_tint {
        BackdropChannel::AccentTint(argb)
    } else if plan.source == BackdropSource::Native && plan.material != DwmMaterial::None {
        let mat = dwm_material_to_platform(plan.material);
        if mat == WindowBackdropMaterial::None {
            BackdropChannel::Off
        } else {
            BackdropChannel::Dwm(mat)
        }
    } else {
        BackdropChannel::Off
    };
    let lock_mica =
        plan.source == BackdropSource::Native && plan.material_kind != MaterialKind::None;
    (channel, lock_mica)
}

/// 窗口外观意图账本。
///
/// 维护当前窗口的外观意图与已下发状态，通过 [`Self::set`] 纯值输入推导并产生增量 Diff。
#[derive(Clone, Debug)]
pub struct WindowAppearance {
    applied_channel: BackdropChannel,
    applied_dark_mode: Option<bool>,
    applied_lock_mica: bool,
    applied_clear_alpha: f32,
    host_clear_alpha: Option<f32>,
    plan: Option<BackdropPlan>,
}

impl Default for WindowAppearance {
    fn default() -> Self {
        Self {
            applied_channel: BackdropChannel::Off,
            applied_dark_mode: None,
            applied_lock_mica: false,
            applied_clear_alpha: 1.0,
            host_clear_alpha: None,
            plan: None,
        }
    }
}

impl WindowAppearance {
    /// 创建初始外观意图账本。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置宿主显式指定的清屏 Alpha（供兼容与自绘路径使用；当意图开启且生效时被计划派生值覆盖）。
    pub fn set_host_clear_alpha(&mut self, alpha: f32) {
        self.host_clear_alpha = Some(alpha.clamp(0.0, 1.0));
    }

    /// 最近一次推导的后景计划引用（供自绘路径判断是否需要纹理）。
    #[must_use]
    pub fn plan(&self) -> Option<&BackdropPlan> {
        self.plan.as_ref()
    }

    /// 当前实际生效的清屏 Alpha。
    #[must_use]
    pub const fn applied_clear_alpha(&self) -> f32 {
        self.applied_clear_alpha
    }

    /// 核心同步入口：输入意图、能力与主题，计算并返回本次需要执行的系统写入增量。
    pub fn set(
        &mut self,
        intent: Option<WindowBackdropMaterial>,
        caps: AppearanceCaps,
        theme: ThemeKind,
    ) -> AppearanceDiff {
        self.set_intent(AppearanceIntent::Material(intent), caps, theme)
    }

    /// 亚克力自定义色调快捷入口。
    pub fn set_accent_tint(
        &mut self,
        argb: u32,
        caps: AppearanceCaps,
        theme: ThemeKind,
    ) -> AppearanceDiff {
        self.set_intent(AppearanceIntent::AccentTint(argb), caps, theme)
    }

    /// 全功能意图同步入口。
    pub fn set_intent(
        &mut self,
        intent: AppearanceIntent,
        caps: AppearanceCaps,
        theme: ThemeKind,
    ) -> AppearanceDiff {
        let (selection, accent_tint_argb, is_open) = match intent {
            AppearanceIntent::Material(None | Some(WindowBackdropMaterial::None)) => (
                BackdropSelection {
                    material: MaterialKind::None,
                    source: BackdropSource::Auto,
                },
                unbound_backdrop::DEFAULT_ACRYLIC_TINT_ARGB,
                false,
            ),
            AppearanceIntent::Material(Some(WindowBackdropMaterial::System)) => (
                BackdropSelection {
                    material: MaterialKind::FollowSystem,
                    source: BackdropSource::Native,
                },
                unbound_backdrop::DEFAULT_ACRYLIC_TINT_ARGB,
                true,
            ),
            AppearanceIntent::Material(Some(WindowBackdropMaterial::Mica)) => (
                BackdropSelection {
                    material: MaterialKind::Mica,
                    source: BackdropSource::Native,
                },
                unbound_backdrop::DEFAULT_ACRYLIC_TINT_ARGB,
                true,
            ),
            AppearanceIntent::Material(Some(WindowBackdropMaterial::Tabbed)) => (
                BackdropSelection {
                    material: MaterialKind::MicaAlt,
                    source: BackdropSource::Native,
                },
                unbound_backdrop::DEFAULT_ACRYLIC_TINT_ARGB,
                true,
            ),
            AppearanceIntent::Material(Some(WindowBackdropMaterial::Acrylic)) => (
                BackdropSelection {
                    material: MaterialKind::Acrylic,
                    source: BackdropSource::Native,
                },
                unbound_backdrop::DEFAULT_ACRYLIC_TINT_ARGB,
                true,
            ),
            AppearanceIntent::AccentTint(argb) => (
                BackdropSelection {
                    material: MaterialKind::AcrylicTint,
                    source: BackdropSource::Native,
                },
                argb,
                true,
            ),
        };

        let dark = matches!(theme, ThemeKind::Dark);
        let plan = BackdropPlan::derive(BackdropInputs {
            selection,
            windows_build: caps.windows_build,
            premultiplied_alpha_supported: caps.premultiplied_alpha_supported,
            accent_policy_export: caps.accent_policy_export,
            accent_tint_argb,
            window_screen_rect: None,
            primary_screen_rect: None,
            primary_screen_scale: 1.0,
            dark,
            self_drawn_ready: false,
        });

        let (target_channel, target_lock_mica) = backdrop_channel_request(&plan);

        // 优先级：意图关闭 -> 保留宿主设置值（默认 1.0）；意图开启 -> 计划派生值，且非预乘强制 1.0。
        let target_clear_alpha = if is_open {
            if !caps.premultiplied_alpha_supported {
                1.0
            } else {
                plan.clear_alpha
            }
        } else {
            self.host_clear_alpha.unwrap_or(1.0)
        };

        // 只有后景通道处于激活态（或者曾经被激活）时，才同步深色标记；
        // 意图关闭且从前未激活时不发送深色标记，与改造前行为一致。
        let is_active = target_channel != BackdropChannel::Off;
        let target_dark_mode = if is_active || self.applied_dark_mode.is_some() {
            Some(dark)
        } else {
            None
        };

        let diff_channel = if target_channel != self.applied_channel {
            Some(target_channel)
        } else {
            None
        };

        let diff_dark_mode = if target_dark_mode != self.applied_dark_mode {
            target_dark_mode
        } else {
            None
        };

        let diff_lock_mica = if target_lock_mica != self.applied_lock_mica {
            Some(target_lock_mica)
        } else {
            None
        };

        let diff_clear_alpha = if (target_clear_alpha - self.applied_clear_alpha).abs() > 1e-6 {
            Some(target_clear_alpha)
        } else {
            None
        };

        self.applied_channel = target_channel;
        if target_dark_mode.is_some() {
            self.applied_dark_mode = target_dark_mode;
        }
        self.applied_lock_mica = target_lock_mica;
        self.applied_clear_alpha = target_clear_alpha;
        self.plan = Some(plan);

        AppearanceDiff {
            channel: diff_channel,
            dark_mode: diff_dark_mode,
            lock_mica: diff_lock_mica,
            clear_alpha: diff_clear_alpha,
        }
    }
}

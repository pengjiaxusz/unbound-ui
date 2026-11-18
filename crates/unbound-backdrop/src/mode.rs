//! 后景模式：用户选择、平台能力解析与两条语义查询。
//!
//! 本模块是旧的用户选择入口，在两轴模型落地期间继续可用：它只表达
//! 「整窗后景走哪条路径」，并翻译成新模型的一组选择（标准云母 + 某个来源）。
//! 新的组合解析接缝在 [`crate::selection`]。

use crate::selection::{BackdropCapabilities, BackdropSelection, BackdropSource, MaterialKind};

/// 原生云母路径要求的最低 Windows 构建号（Windows 11 22H2）。
pub use crate::selection::NATIVE_MICA_MIN_BUILD;

/// 判定原生后景路径是否可用。
///
/// 只有构建号已知且不低于 [`NATIVE_MICA_MIN_BUILD`]、同时图形表面支持
/// 预乘 alpha 合成时，才能启用 DWM 原生材质。未知构建号（非 Windows 或
/// 探测失败）一律视为不可用，从而回退到自绘路径。
///
/// 这是平台**基线**能力，与逐档材质门槛是两件事：它只回答「这台机器能不能
/// 承载原生后景」，不回答「某一档材质能不能走原生路径」。
#[must_use]
pub fn native_backdrop_available(
    windows_build: Option<u32>,
    premultiplied_alpha_supported: bool,
) -> bool {
    BackdropCapabilities {
        windows_build,
        premultiplied_alpha_supported,
    }
    .baseline_native_available()
}

/// 用户可选的整窗后景路径。
///
/// `Auto` 是默认值：宿主根据平台能力把它解析为 [`ResolvedBackdrop`]。
/// 显式 `SelfDrawn` 与 `Off` 不会被能力探测改写；显式 `Native` 在平台
/// 无法承载原生材质时会回退为自绘，避免使用不透明交换链却清屏为全透明。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BackdropMode {
    /// 按平台能力自动选择原生或自绘。
    #[default]
    Auto,
    /// DWM 系统背景材质（Win11 标准云母）。
    Native,
    /// 应用自己生成的壁纸模糊纹理。
    SelfDrawn,
    /// 不启用任何后景材质。
    Off,
}

impl BackdropMode {
    /// 全部模式，顺序与 [`Self::index`] 一致。
    pub const ALL: [BackdropMode; 4] = [Self::Auto, Self::Native, Self::SelfDrawn, Self::Off];

    /// 稳定的索引（用于界面部件编号与持久化）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn index(self) -> usize {
        match self {
            Self::Auto => 0,
            Self::Native => 1,
            Self::SelfDrawn => 2,
            Self::Off => 3,
        }
    }

    /// 按索引取回模式；越界时回退到 [`Self::Auto`]。
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Auto,
            1 => Self::Native,
            2 => Self::SelfDrawn,
            3 => Self::Off,
            _ => Self::Auto,
        }
    }

    /// 面向用户的短名称。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Native => "Native",
            Self::SelfDrawn => "SelfDrawn",
            Self::Off => "Off",
        }
    }

    /// 该模式的一句话说明。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Auto => "Resolve from the platform capability probe",
            Self::Native => "DWM system Mica material",
            Self::SelfDrawn => "Wallpaper blur drawn by the application",
            Self::Off => "No backdrop material at all",
        }
    }

    /// 翻译成两轴模型的用户选择。
    ///
    /// 旧入口只表达路径，材质种类一律取标准云母；「关闭」翻译成材质种类为
    /// 「无」，来源轴随之折叠。翻译后的实际生效结果与改造前逐项一致。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn to_selection(self) -> BackdropSelection {
        match self {
            Self::Auto => BackdropSelection {
                material: MaterialKind::Mica,
                source: BackdropSource::Auto,
            },
            Self::Native => BackdropSelection {
                material: MaterialKind::Mica,
                source: BackdropSource::Native,
            },
            Self::SelfDrawn => BackdropSelection {
                material: MaterialKind::Mica,
                source: BackdropSource::SelfDrawn,
            },
            Self::Off => BackdropSelection {
                material: MaterialKind::None,
                source: BackdropSource::Auto,
            },
        }
    }
}

/// 平台能力解析后的实际生效路径。
///
/// 与 [`BackdropMode`] 不同，本类型不含 `Auto`：宿主一旦解析就只面对具体路径，
/// 上层只需依赖 [`Self::uses_native_path`] 与 [`Self::draws_self_drawn_texture`]。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResolvedBackdrop {
    /// 使用 DWM 原生背景材质。
    Native,
    /// 绘制应用自生成的壁纸纹理。
    SelfDrawn,
    /// 不绘制任何后景。
    Off,
}

impl ResolvedBackdrop {
    /// 将用户选择解析为实际路径。
    ///
    /// 原生不可用时，`Auto` 与显式 `Native` 都解析为 [`Self::SelfDrawn`]：
    /// 平台无法承载 DWM 材质，继续走原生只会得到一张空透明窗口。
    /// `SelfDrawn` 与 `Off` 永不改写。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn resolve(choice: BackdropMode, native_available: bool) -> Self {
        match choice {
            BackdropMode::Auto | BackdropMode::Native => {
                if native_available {
                    Self::Native
                } else {
                    Self::SelfDrawn
                }
            }
            BackdropMode::SelfDrawn => Self::SelfDrawn,
            BackdropMode::Off => Self::Off,
        }
    }

    /// 对应的具体模式（永不返回 [`BackdropMode::Auto`]）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn mode(self) -> BackdropMode {
        match self {
            Self::Native => BackdropMode::Native,
            Self::SelfDrawn => BackdropMode::SelfDrawn,
            Self::Off => BackdropMode::Off,
        }
    }

    /// 是否走 DWM 原生背景材质路径。
    #[must_use]
    pub const fn uses_native_path(self) -> bool {
        matches!(self, Self::Native)
    }

    /// 是否需要绘制应用自生成的云母纹理。
    #[must_use]
    pub const fn draws_self_drawn_texture(self) -> bool {
        matches!(self, Self::SelfDrawn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_choice_is_auto() {
        assert_eq!(
            BackdropMode::default(),
            BackdropMode::Auto,
            "a missing user choice must default to Auto"
        );
    }

    #[test]
    fn auto_resolves_to_native_when_native_is_available() {
        assert_eq!(
            ResolvedBackdrop::resolve(BackdropMode::Auto, true),
            ResolvedBackdrop::Native,
            "Auto must pick the native path when the platform can host it"
        );
    }

    #[test]
    fn auto_resolves_to_self_drawn_when_native_is_unavailable() {
        assert_eq!(
            ResolvedBackdrop::resolve(BackdropMode::Auto, false),
            ResolvedBackdrop::SelfDrawn,
            "Auto must fall back to the self-drawn texture without native support"
        );
    }

    #[test]
    fn explicit_self_drawn_and_off_ignore_availability() {
        for available in [false, true] {
            assert_eq!(
                ResolvedBackdrop::resolve(BackdropMode::SelfDrawn, available),
                ResolvedBackdrop::SelfDrawn,
                "an explicit SelfDrawn choice must stay self-drawn"
            );
            assert_eq!(
                ResolvedBackdrop::resolve(BackdropMode::Off, available),
                ResolvedBackdrop::Off,
                "an explicit Off choice must stay off"
            );
        }
    }

    #[test]
    fn explicit_native_falls_back_when_native_is_unavailable() {
        assert_eq!(
            ResolvedBackdrop::resolve(BackdropMode::Native, false),
            ResolvedBackdrop::SelfDrawn,
            "an explicit Native choice must fall back to the self-drawn texture when the platform cannot host it"
        );
        assert_eq!(
            ResolvedBackdrop::resolve(BackdropMode::Native, true),
            ResolvedBackdrop::Native,
            "an explicit Native choice must stay native when the platform can host it"
        );
    }

    #[test]
    fn native_availability_requires_a_known_win11_build() {
        for build in [None, Some(10), Some(19045), Some(22000), Some(22620)] {
            assert!(
                !native_backdrop_available(build, true),
                "build {build:?} must not enable the native backdrop"
            );
        }
        assert!(
            native_backdrop_available(Some(NATIVE_MICA_MIN_BUILD), true),
            "the 22621 boundary with driver support must enable the native backdrop"
        );
        assert!(
            native_backdrop_available(Some(26100), true),
            "later Win11 builds with driver support must enable the native backdrop"
        );
    }

    #[test]
    fn native_availability_requires_premultiplied_alpha_support() {
        assert!(
            !native_backdrop_available(Some(26100), false),
            "a Win11 build without premultiplied alpha composition must not enable the native backdrop"
        );
    }

    #[test]
    fn semantic_queries_match_the_resolved_mode() {
        let native = ResolvedBackdrop::resolve(BackdropMode::Auto, true);
        assert!(
            native.uses_native_path(),
            "the native result must use the native path"
        );
        assert!(
            !native.draws_self_drawn_texture(),
            "the native result must not draw the self-drawn texture"
        );

        let self_drawn = ResolvedBackdrop::resolve(BackdropMode::Auto, false);
        assert!(
            !self_drawn.uses_native_path(),
            "the self-drawn result must not use the native path"
        );
        assert!(
            self_drawn.draws_self_drawn_texture(),
            "the self-drawn result must draw the self-drawn texture"
        );

        let off = ResolvedBackdrop::resolve(BackdropMode::Off, true);
        assert!(
            !off.uses_native_path() && !off.draws_self_drawn_texture(),
            "the off result must use neither backdrop path"
        );
    }

    #[test]
    fn every_mode_translates_into_the_two_axis_model() {
        assert_eq!(
            BackdropMode::Auto.to_selection(),
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto),
            "Auto must translate to Mica with the auto source"
        );
        assert_eq!(
            BackdropMode::Native.to_selection(),
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Native),
            "Native must translate to Mica with the native source"
        );
        assert_eq!(
            BackdropMode::SelfDrawn.to_selection(),
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::SelfDrawn),
            "SelfDrawn must translate to Mica with the self-drawn source"
        );
        assert_eq!(
            BackdropMode::Off.to_selection(),
            BackdropSelection::new(MaterialKind::None, BackdropSource::Auto),
            "Off must translate to the None material"
        );
    }

    #[test]
    fn every_mode_exposes_stable_metadata() {
        assert_eq!(
            BackdropMode::ALL.len(),
            4,
            "all four backdrop modes must be enumerated once"
        );
        for (index, mode) in BackdropMode::ALL.into_iter().enumerate() {
            assert_eq!(
                mode.index(),
                index,
                "BackdropMode::ALL order must match BackdropMode::index()"
            );
            assert_eq!(
                BackdropMode::from_index(index),
                mode,
                "from_index must round-trip every mode"
            );
            assert!(
                !mode.label().is_empty(),
                "every mode must expose a display label"
            );
            assert!(
                !mode.description().is_empty(),
                "every mode must expose a description"
            );
        }
    }
}

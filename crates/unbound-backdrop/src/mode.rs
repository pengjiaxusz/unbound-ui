//! 后景模式：用户选择、平台能力解析与两条语义查询。

/// 用户可选的整窗后景路径。
///
/// `Auto` 是默认值：宿主根据平台能力把它解析为 [`ResolvedBackdrop`]；
/// 其余三种是用户的显式选择，不会被能力探测改写。
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
    /// `native_available` 由平台能力探测提供（本工单尚未实现，调用方传 `false`
    /// 即可保持无头与非 Windows 环境下的自绘行为）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn resolve(choice: BackdropMode, native_available: bool) -> Self {
        match choice {
            BackdropMode::Auto => {
                if native_available {
                    Self::Native
                } else {
                    Self::SelfDrawn
                }
            }
            BackdropMode::Native => Self::Native,
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
    fn explicit_choices_ignore_availability() {
        for available in [false, true] {
            assert_eq!(
                ResolvedBackdrop::resolve(BackdropMode::Native, available),
                ResolvedBackdrop::Native,
                "an explicit Native choice must never be rewritten by the probe"
            );
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

//! 后景两轴模型：材质种类、来源、平台能力与组合解析接缝。
//!
//! 后景由两个彼此正交的维度共同决定：**材质种类**描述「看起来是什么」，
//! **来源**描述「由谁把它画出来」。同一材质种类可以有两种来源，观感应当接近，
//! 但能力前提与失效条件不同。
//!
//! 本模块把两轴的全部组合收敛成一个纯函数接缝 [`resolve_combination`]：
//! 输入用户选择与平台能力，输出实际生效的组合、回退原因与机器能力不足的原因。
//! 解析过程不做任何系统调用，也不读取任何全局状态，因此可以在单元测试里
//! 穷举「材质种类 × 来源 × 平台能力」的全部组合。
//!
//! 两类原因必须分开：**回退原因**说明用户的请求被降级成了什么，
//! **不可用原因**说明这台机器根本承载不了该材质。界面据此分别处理——
//! 机器能力不足时禁用组合，组合本身非法时允许选中并展示回退结果。

/// 标准云母材质要求的最低系统构建号（Windows 11 22H2）。
///
/// 跟随系统、标准云母、云母变体与亚克力四档共用这一门槛。
pub const NATIVE_MICA_MIN_BUILD: u32 = 22621;

/// 沉浸式后景要求的最低系统构建号。
///
/// 该档是公开文档未收录的实验档，只在较新系统版本上生效。
pub const IMMERSIVE_BACKDROP_MIN_BUILD: u32 = 26100;

/// 材质种类：后景在视觉上呈现哪一种系统材质。
///
/// 这是封闭的六档集合：新增一档必须同时给出它的门槛、它的系统材质请求，
/// 以及它在没有自绘变体时的回退行为。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    /// 不启用任何后景材质；来源轴随之折叠。
    None,
    /// 跟随系统默认策略。
    FollowSystem,
    /// 标准云母材质。
    #[default]
    Mica,
    /// 云母变体材质。
    MicaAlt,
    /// 亚克力材质。
    Acrylic,
    /// 沉浸式后景（实验档）。
    Immersive,
}

impl MaterialKind {
    /// 全部材质种类，顺序与 [`Self::index`] 一致。
    pub const ALL: [MaterialKind; 6] = [
        Self::None,
        Self::FollowSystem,
        Self::Mica,
        Self::MicaAlt,
        Self::Acrylic,
        Self::Immersive,
    ];

    /// 稳定的索引（用于界面部件编号与持久化）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn index(self) -> usize {
        match self {
            Self::None => 0,
            Self::FollowSystem => 1,
            Self::Mica => 2,
            Self::MicaAlt => 3,
            Self::Acrylic => 4,
            Self::Immersive => 5,
        }
    }

    /// 按索引取回材质种类；越界时回退到 [`Self::Mica`]。
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::None,
            1 => Self::FollowSystem,
            2 => Self::Mica,
            3 => Self::MicaAlt,
            4 => Self::Acrylic,
            5 => Self::Immersive,
            _ => Self::Mica,
        }
    }

    /// 面向用户的短名称。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::FollowSystem => "FollowSystem",
            Self::Mica => "Mica",
            Self::MicaAlt => "MicaAlt",
            Self::Acrylic => "Acrylic",
            Self::Immersive => "Immersive",
        }
    }

    /// 该材质种类的一句话说明。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn description(self) -> &'static str {
        match self {
            Self::None => "No backdrop material at all",
            Self::FollowSystem => "Let the system pick the backdrop material",
            Self::Mica => "Standard system Mica material",
            Self::MicaAlt => "System Mica variant material",
            Self::Acrylic => "System acrylic material",
            Self::Immersive => "Experimental immersive backdrop material",
        }
    }

    /// 该材质种类是否是实验档。
    ///
    /// 实验档表示「尚未被公开文档或实测证实为稳定可用」的材质：取值可能在任何
    /// 一次系统更新里改变，界面必须把它与常规档显式区分开。当前只有沉浸式后景
    /// 是实验档。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn is_experimental(self) -> bool {
        matches!(self, Self::Immersive)
    }
}

/// 来源：后景材质由哪条路径产出。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BackdropSource {
    /// 由平台能力在原生与自绘之间取舍，始终落到一个可用组合。
    #[default]
    Auto,
    /// 系统合成器直接生成材质。
    Native,
    /// 引擎自绘壁纸纹理。
    SelfDrawn,
}

impl BackdropSource {
    /// 全部来源，顺序与 [`Self::index`] 一致。
    pub const ALL: [BackdropSource; 3] = [Self::Auto, Self::Native, Self::SelfDrawn];

    /// 稳定的索引（用于界面部件编号与持久化）。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn index(self) -> usize {
        match self {
            Self::Auto => 0,
            Self::Native => 1,
            Self::SelfDrawn => 2,
        }
    }

    /// 按索引取回来源；越界时回退到 [`Self::Auto`]。
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Auto,
            1 => Self::Native,
            2 => Self::SelfDrawn,
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
        }
    }

    /// 该来源的一句话说明。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Auto => "Pick the native or self-drawn source from the platform capability",
            Self::Native => "The system compositor draws the backdrop material",
            Self::SelfDrawn => "The engine draws the wallpaper-derived texture",
        }
    }
}

/// 平台能力：判断某一档材质能否走原生路径所需的全部事实。
///
/// 能力数据必须由调用方探测后传入；本模块不做任何系统查询。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BackdropCapabilities {
    /// 系统构建号；`None` 表示非系统环境或探测失败。
    pub windows_build: Option<u32>,
    /// 图形表面是否支持预乘 alpha 合成。
    pub premultiplied_alpha_supported: bool,
}

impl BackdropCapabilities {
    /// 平台基线能力：能否承载原生后景。
    ///
    /// 这是表面合成方式的唯一依据——交换链在启动时按它一次性固定，
    /// 此后任何材质种类或来源的切换都不得改变它。它与逐档门槛的区别在于：
    /// 逐档门槛还会为实验档抬高构建号要求。
    #[must_use]
    pub const fn baseline_native_available(&self) -> bool {
        match self.windows_build {
            Some(build) => build >= NATIVE_MICA_MIN_BUILD && self.premultiplied_alpha_supported,
            None => false,
        }
    }
}

/// 用户的选择：两个正交维度。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BackdropSelection {
    /// 想要哪种材质。
    pub material: MaterialKind,
    /// 想要哪条来源。
    pub source: BackdropSource,
}

impl BackdropSelection {
    /// 构造一个选择。
    #[must_use]
    pub const fn new(material: MaterialKind, source: BackdropSource) -> Self {
        Self { material, source }
    }
}

/// 请求被降级的原因。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FallbackReason {
    /// 原生路径不可用，改走自绘。
    NativeUnavailable {
        /// 被降级的材质种类。
        material: MaterialKind,
    },
    /// 该材质种类没有自绘变体。
    NoSelfDrawnVariant {
        /// 缺少自绘变体的材质种类。
        material: MaterialKind,
    },
}

impl FallbackReason {
    /// 面向用户的短名称。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NativeUnavailable { .. } => "Native unavailable",
            Self::NoSelfDrawnVariant { .. } => "No self-drawn variant",
        }
    }

    /// 该回退原因的一句话说明。
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::NativeUnavailable { material } => format!(
                "The system cannot host the {} material, so the self-drawn source took over",
                material.label()
            ),
            Self::NoSelfDrawnVariant { material } => format!(
                "The {} material has no self-drawn variant, so the combination fell back",
                material.label()
            ),
        }
    }
}

/// 机器能力不足的原因。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum UnavailableReason {
    /// 系统构建号不够。
    BuildTooLow {
        /// 该材质种类要求的最低构建号。
        required: u32,
    },
    /// 构建号未知（非系统环境或探测失败）。
    BuildUnknown,
    /// 图形表面不支持预乘 alpha 合成。
    PremultipliedAlphaUnsupported,
    /// 该材质没有经过探测确认的系统取值，系统合成器无法把它画出来。
    ///
    /// 沉浸式后景属于这一档：它的 `DWM_SYSTEMBACKDROP_TYPE` 取值从未公开，
    /// 平台层的「写入后回读」探测也无法把真实接受与单纯回显区分开，因此宿主
    /// 没有可下发的系统材质请求。该原因必须让用户在界面上看得见。
    SystemChannelUnconfirmed,
}

impl UnavailableReason {
    /// 面向用户的短名称。
    #[must_use]
    #[deny(unreachable_patterns)]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::BuildTooLow { .. } => "System build too low",
            Self::BuildUnknown => "System build unknown",
            Self::PremultipliedAlphaUnsupported => "Premultiplied alpha unsupported",
            Self::SystemChannelUnconfirmed => "System channel unconfirmed",
        }
    }

    /// 该不可用原因的一句话说明。
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::BuildTooLow { required } => {
                format!("Requires Windows build {required} or newer")
            }
            Self::BuildUnknown => {
                "The Windows build number is unknown (non-Windows host or probe failure)"
                    .to_string()
            }
            Self::PremultipliedAlphaUnsupported => {
                "The graphics surface does not support premultiplied alpha composition".to_string()
            }
            Self::SystemChannelUnconfirmed => {
                "The system backdrop value for this material is unconfirmed, so the system compositor cannot draw it"
                    .to_string()
            }
        }
    }
}

/// 解析后的实际生效组合。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BackdropCombination {
    /// 实际生效的材质种类；`None` 表示完全没有后景。
    pub material: MaterialKind,
    /// 实际生效的来源；没有后景时保留用户请求的来源。
    pub source: BackdropSource,
    /// 请求被降级的原因；`None` 表示原样满足。
    pub fallback: Option<FallbackReason>,
    /// 机器能力不足的原因；`None` 表示机器能力没问题。
    pub unavailable: Option<UnavailableReason>,
}

/// 某一材质种类要求的最低系统构建号。
///
/// 「无」没有门槛，返回 `0`；其余各档按逐档门槛声明。
#[must_use]
#[deny(unreachable_patterns)]
pub const fn required_build(material: MaterialKind) -> u32 {
    match material {
        MaterialKind::None => 0,
        MaterialKind::FollowSystem
        | MaterialKind::Mica
        | MaterialKind::MicaAlt
        | MaterialKind::Acrylic => NATIVE_MICA_MIN_BUILD,
        MaterialKind::Immersive => IMMERSIVE_BACKDROP_MIN_BUILD,
    }
}

/// 某一材质种类在当前机器上能否走原生路径。
///
/// 门槛只作用于原生路径：自绘路径在任何环境都可用。
/// 门槛分三档——「无」不受约束；跟随系统、标准云母、云母变体与亚克力要求
/// 构建号不低于 [`NATIVE_MICA_MIN_BUILD`] **且**表面支持预乘合成。
///
/// 沉浸式后景是例外：它的系统取值从未公开，平台层的写入回读探测也确认不了
/// 真实取值（详见 `unbound_platform::win32_frame::probe_immersive_backdrop` 的文档），
/// 因此宿主没有可下发的系统材质请求，**任何环境都不能走原生路径**。
/// 构建号未知同样一律视为不可用。
#[must_use]
pub fn native_available(material: MaterialKind, caps: &BackdropCapabilities) -> bool {
    match material {
        // 无材质不受任何门槛约束：它不请求任何系统材质。
        MaterialKind::None => true,
        // 实验档没有可下发的系统取值，原生通道不存在。
        MaterialKind::Immersive => false,
        MaterialKind::FollowSystem
        | MaterialKind::Mica
        | MaterialKind::MicaAlt
        | MaterialKind::Acrylic => {
            caps.windows_build
                .is_some_and(|build| build >= required_build(material))
                && caps.premultiplied_alpha_supported
        }
    }
}

/// 某一材质种类在当前机器上无法走原生路径的原因。
///
/// 可用时返回 `None`；不可用时给出可展示的原因。沉浸式后景先报构建号门槛，
/// 通过门槛后再报「系统取值未确认」，避免把实验档的问题误报成机器太旧。
/// 其余各档的构建号不足优先于预乘合成不支持。
#[must_use]
pub fn unavailable_reason(
    material: MaterialKind,
    caps: &BackdropCapabilities,
) -> Option<UnavailableReason> {
    if material == MaterialKind::Immersive {
        return Some(match caps.windows_build {
            None => UnavailableReason::BuildUnknown,
            Some(build) if build < IMMERSIVE_BACKDROP_MIN_BUILD => UnavailableReason::BuildTooLow {
                required: IMMERSIVE_BACKDROP_MIN_BUILD,
            },
            Some(_) if !caps.premultiplied_alpha_supported => {
                UnavailableReason::PremultipliedAlphaUnsupported
            }
            Some(_) => UnavailableReason::SystemChannelUnconfirmed,
        });
    }
    if native_available(material, caps) {
        return None;
    }
    let required = required_build(material);
    Some(match caps.windows_build {
        None => UnavailableReason::BuildUnknown,
        Some(build) if build < required => UnavailableReason::BuildTooLow { required },
        Some(_) => UnavailableReason::PremultipliedAlphaUnsupported,
    })
}

/// 材质种类在自绘路径上的覆盖边界。
///
/// 这是「该材质有没有自绘变体」的唯一声明：布尔视图 [`has_self_drawn_variant`]
/// 与组合解析 [`resolve_combination`] 都从它派生，不得再写第二份判断。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SelfDrawnCoverage {
    /// 自绘路径能等价重现该材质的观感：自绘变体就是该材质本身。
    Equivalent,
    /// 自绘路径没有该材质的变体：显式选择自绘来源时组合非法，只能回退。
    NoVariant,
}

/// 某一材质种类在自绘路径上的覆盖声明。
///
/// 只有标准云母与云母变体属于「等价重现」：它们本身就是壁纸模糊加主题色调的
/// 产物，自绘路径复刻的是同一套配方。其余三档各有一条明确的「无变体」理由：
///
/// - **亚克力**的辨识特征是噪点纹理与半透明层叠感，壁纸模糊做不出这两点，
///   自造变体只会得到一份误导性的对照；
/// - **沉浸式后景**是系统里并不存在的取值，没有可对照的原生外观，自然也无从
///   自绘；
/// - **跟随系统**是一条选择策略而不是一种外观，自绘路径无法预知系统会选什么。
///
/// 材质为「无」时来源轴整体折叠，覆盖声明不参与解析，但这里仍诚实声明无变体。
#[must_use]
#[deny(unreachable_patterns)]
pub const fn self_drawn_coverage(material: MaterialKind) -> SelfDrawnCoverage {
    match material {
        MaterialKind::Mica | MaterialKind::MicaAlt => SelfDrawnCoverage::Equivalent,
        MaterialKind::None
        | MaterialKind::FollowSystem
        | MaterialKind::Acrylic
        | MaterialKind::Immersive => SelfDrawnCoverage::NoVariant,
    }
}

/// 该材质种类是否有自绘变体。
///
/// 这是 [`self_drawn_coverage`] 的布尔视图，不得独立判断。
#[must_use]
pub const fn has_self_drawn_variant(material: MaterialKind) -> bool {
    matches!(self_drawn_coverage(material), SelfDrawnCoverage::Equivalent)
}

/// 用户选择是否构成「组合非法但允许选中」。
///
/// 只有显式选择自绘来源、且该材质在 [`self_drawn_coverage`] 里声明为无变体时，
/// 组合才非法。它不同于机器能力不足：非法组合仍然可选，由解析给出回退原因；
/// 自动来源由平台在两条路径间取舍，任何材质都不算非法；材质为「无」时来源轴
/// 折叠，同样不算非法。
#[must_use]
pub fn is_illegal_combination(selection: BackdropSelection) -> bool {
    selection.material != MaterialKind::None
        && selection.source == BackdropSource::SelfDrawn
        && !has_self_drawn_variant(selection.material)
}

/// 在给定来源下，某一材质种类是否能被用户选中。
///
/// 这是展台逐档门槛的唯一裁决点，展台按钮的禁用状态直接取自它：
///
/// - **来源为「系统原生」**：必须能走原生路径，否则禁用；
/// - **来源为「自动」**：原生可用，或该材质有自绘变体（自动会回退到自绘），
///   否则禁用；
/// - **来源为「引擎自绘」**：永远可选——缺少自绘变体只是让组合非法并展示回退，
///   不是机器能力不足，不能因此禁用（见 #37 的细化）；
/// - **材质为「无」**永远可选，因为它不请求任何系统材质。
///
/// 「禁用」只表达机器承载不了，不表达组合非法。
#[must_use]
pub fn selectable(
    material: MaterialKind,
    source: BackdropSource,
    caps: &BackdropCapabilities,
) -> bool {
    if material == MaterialKind::None {
        return true;
    }
    match source {
        BackdropSource::Native => native_available(material, caps),
        BackdropSource::Auto => {
            native_available(material, caps) || has_self_drawn_variant(material)
        }
        BackdropSource::SelfDrawn => true,
    }
}

/// 解析用户选择与平台能力，给出实际生效的后景组合。
///
/// 解析阶梯（每一档来源各自成条）：
///
/// - **材质种类为「无」**：来源轴折叠。实际生效组合是「无材质 + 用户请求的来源」，
///   不请求任何系统材质，也不生成任何纹理。
/// - **来源为「系统原生」**：原生路径可用时原样满足。不可用时这是**机器能力不足**
///   而不是回退——必须给出不可用原因，回退原因保持为空，界面据此禁用该组合。
///   此时若该材质种类有自绘变体，实际生效组合保留材质种类、来源改为自绘；
///   否则完全没有后景。
/// - **来源为「引擎自绘」**：材质种类有自绘变体时原样满足。没有时该组合**非法**
///   但仍可选中，回退原因是「没有自绘变体」。回退先保材质种类、再保来源：
///   原生路径可用时保留材质种类、来源改为原生；否则保留自绘来源、材质种类
///   换成有自绘变体的标准云母。
/// - **来源为「自动」**：始终落到一个可用组合。原生路径可用时走原生；不可用时，
///   若材质种类有自绘变体则改走自绘（回退原因是「原生不可用」），
///   否则改走标准云母的自绘（回退原因是「没有自绘变体」）。
#[must_use]
pub fn resolve_combination(
    selection: BackdropSelection,
    caps: &BackdropCapabilities,
) -> BackdropCombination {
    let BackdropSelection { material, source } = selection;
    if material == MaterialKind::None {
        return BackdropCombination {
            material: MaterialKind::None,
            source,
            fallback: None,
            unavailable: None,
        };
    }
    match source {
        BackdropSource::Native => resolve_native(material, caps),
        BackdropSource::SelfDrawn => resolve_self_drawn(material, caps),
        BackdropSource::Auto => resolve_auto(material, caps),
    }
}

/// 来源为「系统原生」时的解析。
fn resolve_native(material: MaterialKind, caps: &BackdropCapabilities) -> BackdropCombination {
    if native_available(material, caps) {
        return BackdropCombination {
            material,
            source: BackdropSource::Native,
            fallback: None,
            unavailable: None,
        };
    }
    // 原生不可用属于机器能力不足：给出不可用原因，回退原因保持为空。
    let unavailable = unavailable_reason(material, caps);
    if has_self_drawn_variant(material) {
        BackdropCombination {
            material,
            source: BackdropSource::SelfDrawn,
            fallback: None,
            unavailable,
        }
    } else {
        // 该材质没有自绘变体，原生又不可用：完全没有后景。
        BackdropCombination {
            material: MaterialKind::None,
            source: BackdropSource::Native,
            fallback: None,
            unavailable,
        }
    }
}

/// 来源为「引擎自绘」时的解析。
fn resolve_self_drawn(material: MaterialKind, caps: &BackdropCapabilities) -> BackdropCombination {
    if has_self_drawn_variant(material) {
        return BackdropCombination {
            material,
            source: BackdropSource::SelfDrawn,
            fallback: None,
            unavailable: None,
        };
    }
    // 组合非法但仍可选中：回退先保材质种类、再保来源。
    let fallback = Some(FallbackReason::NoSelfDrawnVariant { material });
    if native_available(material, caps) {
        BackdropCombination {
            material,
            source: BackdropSource::Native,
            fallback,
            unavailable: None,
        }
    } else {
        BackdropCombination {
            material: MaterialKind::Mica,
            source: BackdropSource::SelfDrawn,
            fallback,
            unavailable: None,
        }
    }
}

/// 来源为「自动」时的解析。
fn resolve_auto(material: MaterialKind, caps: &BackdropCapabilities) -> BackdropCombination {
    if native_available(material, caps) {
        return BackdropCombination {
            material,
            source: BackdropSource::Native,
            fallback: None,
            unavailable: None,
        };
    }
    if has_self_drawn_variant(material) {
        BackdropCombination {
            material,
            source: BackdropSource::SelfDrawn,
            fallback: Some(FallbackReason::NativeUnavailable { material }),
            unavailable: None,
        }
    } else {
        BackdropCombination {
            material: MaterialKind::Mica,
            source: BackdropSource::SelfDrawn,
            fallback: Some(FallbackReason::NoSelfDrawnVariant { material }),
            unavailable: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一档平台能力用例。
    ///
    /// 期望的不可用原因都是手写字面量，独立于被测实现。沉浸式后景的门槛
    /// 高于其余四档，因此两者的不可用原因分开声明。
    struct CapabilityCase {
        /// 用例名称，仅用于断言消息。
        name: &'static str,
        /// 系统构建号。
        build: Option<u32>,
        /// 表面是否支持预乘合成。
        premultiplied: bool,
        /// 跟随系统、标准云母、云母变体与亚克力四档的不可用原因；`None` 表示可用。
        unavailable_standard: Option<UnavailableReason>,
        /// 沉浸式后景的不可用原因；`None` 表示可用。
        unavailable_immersive: Option<UnavailableReason>,
    }

    /// 能力矩阵：覆盖构建号未知、门槛下沿、门槛上沿与实验档门槛两侧。
    const CAPABILITY_CASES: [CapabilityCase; 10] = [
        CapabilityCase {
            name: "unknown build",
            build: None,
            premultiplied: true,
            unavailable_standard: Some(UnavailableReason::BuildUnknown),
            unavailable_immersive: Some(UnavailableReason::BuildUnknown),
        },
        CapabilityCase {
            name: "unknown build without premultiplied alpha",
            build: None,
            premultiplied: false,
            unavailable_standard: Some(UnavailableReason::BuildUnknown),
            unavailable_immersive: Some(UnavailableReason::BuildUnknown),
        },
        CapabilityCase {
            name: "build just below the mica gate",
            build: Some(22620),
            premultiplied: true,
            unavailable_standard: Some(UnavailableReason::BuildTooLow {
                required: NATIVE_MICA_MIN_BUILD,
            }),
            unavailable_immersive: Some(UnavailableReason::BuildTooLow {
                required: IMMERSIVE_BACKDROP_MIN_BUILD,
            }),
        },
        CapabilityCase {
            name: "build just below the mica gate without premultiplied alpha",
            build: Some(22620),
            premultiplied: false,
            unavailable_standard: Some(UnavailableReason::BuildTooLow {
                required: NATIVE_MICA_MIN_BUILD,
            }),
            unavailable_immersive: Some(UnavailableReason::BuildTooLow {
                required: IMMERSIVE_BACKDROP_MIN_BUILD,
            }),
        },
        CapabilityCase {
            name: "build on the mica gate",
            build: Some(22621),
            premultiplied: true,
            unavailable_standard: None,
            unavailable_immersive: Some(UnavailableReason::BuildTooLow {
                required: IMMERSIVE_BACKDROP_MIN_BUILD,
            }),
        },
        CapabilityCase {
            name: "build on the mica gate without premultiplied alpha",
            build: Some(22621),
            premultiplied: false,
            unavailable_standard: Some(UnavailableReason::PremultipliedAlphaUnsupported),
            unavailable_immersive: Some(UnavailableReason::BuildTooLow {
                required: IMMERSIVE_BACKDROP_MIN_BUILD,
            }),
        },
        CapabilityCase {
            name: "build on the immersive gate",
            build: Some(26100),
            premultiplied: true,
            unavailable_standard: None,
            // 构建号已够，但沉浸式后景的系统取值未被探测确认，仍然不可用。
            unavailable_immersive: Some(UnavailableReason::SystemChannelUnconfirmed),
        },
        CapabilityCase {
            name: "build on the immersive gate without premultiplied alpha",
            build: Some(26100),
            premultiplied: false,
            unavailable_standard: Some(UnavailableReason::PremultipliedAlphaUnsupported),
            unavailable_immersive: Some(UnavailableReason::PremultipliedAlphaUnsupported),
        },
        CapabilityCase {
            name: "build above the immersive gate",
            build: Some(26101),
            premultiplied: true,
            unavailable_standard: None,
            unavailable_immersive: Some(UnavailableReason::SystemChannelUnconfirmed),
        },
        CapabilityCase {
            name: "build above the immersive gate without premultiplied alpha",
            build: Some(26101),
            premultiplied: false,
            unavailable_standard: Some(UnavailableReason::PremultipliedAlphaUnsupported),
            unavailable_immersive: Some(UnavailableReason::PremultipliedAlphaUnsupported),
        },
    ];

    /// 手写的“该材质有没有自绘变体”期望，独立于被测实现。
    fn expected_variant(material: MaterialKind) -> bool {
        matches!(material, MaterialKind::Mica | MaterialKind::MicaAlt)
    }

    /// 手写的“该材质在该档能力下能否走原生路径”期望，独立于被测实现。
    fn expected_native_ok(material: MaterialKind, case: &CapabilityCase) -> bool {
        expected_unavailable(material, case).is_none()
    }

    /// 手写的“该材质在该档能力下的不可用原因”期望，独立于被测实现。
    fn expected_unavailable(
        material: MaterialKind,
        case: &CapabilityCase,
    ) -> Option<UnavailableReason> {
        if material == MaterialKind::Immersive {
            case.unavailable_immersive
        } else {
            case.unavailable_standard
        }
    }

    /// 手写的解析阶梯期望，独立于被测实现。
    fn expected_combination(
        material: MaterialKind,
        source: BackdropSource,
        case: &CapabilityCase,
    ) -> BackdropCombination {
        if material == MaterialKind::None {
            return BackdropCombination {
                material: MaterialKind::None,
                source,
                fallback: None,
                unavailable: None,
            };
        }
        let native_ok = expected_native_ok(material, case);
        let unavailable = expected_unavailable(material, case);
        match source {
            BackdropSource::Native => {
                if native_ok {
                    BackdropCombination {
                        material,
                        source: BackdropSource::Native,
                        fallback: None,
                        unavailable: None,
                    }
                } else if expected_variant(material) {
                    BackdropCombination {
                        material,
                        source: BackdropSource::SelfDrawn,
                        fallback: None,
                        unavailable,
                    }
                } else {
                    BackdropCombination {
                        material: MaterialKind::None,
                        source: BackdropSource::Native,
                        fallback: None,
                        unavailable,
                    }
                }
            }
            BackdropSource::SelfDrawn => {
                if expected_variant(material) {
                    BackdropCombination {
                        material,
                        source: BackdropSource::SelfDrawn,
                        fallback: None,
                        unavailable: None,
                    }
                } else if native_ok {
                    BackdropCombination {
                        material,
                        source: BackdropSource::Native,
                        fallback: Some(FallbackReason::NoSelfDrawnVariant { material }),
                        unavailable: None,
                    }
                } else {
                    BackdropCombination {
                        material: MaterialKind::Mica,
                        source: BackdropSource::SelfDrawn,
                        fallback: Some(FallbackReason::NoSelfDrawnVariant { material }),
                        unavailable: None,
                    }
                }
            }
            BackdropSource::Auto => {
                if native_ok {
                    BackdropCombination {
                        material,
                        source: BackdropSource::Native,
                        fallback: None,
                        unavailable: None,
                    }
                } else if expected_variant(material) {
                    BackdropCombination {
                        material,
                        source: BackdropSource::SelfDrawn,
                        fallback: Some(FallbackReason::NativeUnavailable { material }),
                        unavailable: None,
                    }
                } else {
                    BackdropCombination {
                        material: MaterialKind::Mica,
                        source: BackdropSource::SelfDrawn,
                        fallback: Some(FallbackReason::NoSelfDrawnVariant { material }),
                        unavailable: None,
                    }
                }
            }
        }
    }

    fn caps_of(case: &CapabilityCase) -> BackdropCapabilities {
        BackdropCapabilities {
            windows_build: case.build,
            premultiplied_alpha_supported: case.premultiplied,
        }
    }

    #[test]
    fn material_kinds_expose_stable_metadata() {
        assert_eq!(
            MaterialKind::ALL.len(),
            6,
            "all six material kinds must be enumerated once"
        );
        for (index, material) in MaterialKind::ALL.into_iter().enumerate() {
            assert_eq!(
                material.index(),
                index,
                "MaterialKind::ALL order must match MaterialKind::index()"
            );
            assert_eq!(
                MaterialKind::from_index(index),
                material,
                "from_index must round-trip every material kind"
            );
            assert!(
                !material.label().is_empty(),
                "every material kind must expose a display label"
            );
            assert!(
                !material.description().is_empty(),
                "every material kind must expose a description"
            );
            assert_eq!(
                material.is_experimental(),
                material == MaterialKind::Immersive,
                "only the immersive material must be marked experimental"
            );
        }
        assert_eq!(
            MaterialKind::from_index(99),
            MaterialKind::Mica,
            "an out-of-range index must fall back to the default material kind"
        );
    }

    #[test]
    fn sources_expose_stable_metadata() {
        assert_eq!(
            BackdropSource::ALL.len(),
            3,
            "all three sources must be enumerated once"
        );
        for (index, source) in BackdropSource::ALL.into_iter().enumerate() {
            assert_eq!(
                source.index(),
                index,
                "BackdropSource::ALL order must match BackdropSource::index()"
            );
            assert_eq!(
                BackdropSource::from_index(index),
                source,
                "from_index must round-trip every source"
            );
            assert!(
                !source.label().is_empty(),
                "every source must expose a display label"
            );
            assert!(
                !source.description().is_empty(),
                "every source must expose a description"
            );
        }
        assert_eq!(
            BackdropSource::from_index(99),
            BackdropSource::Auto,
            "an out-of-range index must fall back to the default source"
        );
    }

    #[test]
    fn required_build_matches_the_declared_tier_gates() {
        assert_eq!(
            required_build(MaterialKind::None),
            0,
            "the None material must not declare a build gate"
        );
        for material in [
            MaterialKind::FollowSystem,
            MaterialKind::Mica,
            MaterialKind::MicaAlt,
            MaterialKind::Acrylic,
        ] {
            assert_eq!(
                required_build(material),
                22621,
                "{material:?} must require Windows build 22621"
            );
        }
        assert_eq!(
            required_build(MaterialKind::Immersive),
            26100,
            "the immersive material must require Windows build 26100"
        );
    }

    #[test]
    fn native_availability_follows_the_per_tier_gates() {
        let standard = [
            MaterialKind::FollowSystem,
            MaterialKind::Mica,
            MaterialKind::MicaAlt,
            MaterialKind::Acrylic,
        ];
        // 22621 是四档的门槛下沿：低一个构建号即不可用。
        for material in standard {
            assert!(
                !native_available(
                    material,
                    &BackdropCapabilities {
                        windows_build: Some(22620),
                        premultiplied_alpha_supported: true,
                    }
                ),
                "{material:?} must not be native below build 22621"
            );
            assert!(
                native_available(
                    material,
                    &BackdropCapabilities {
                        windows_build: Some(22621),
                        premultiplied_alpha_supported: true,
                    }
                ),
                "{material:?} must be native from build 22621 on"
            );
            assert!(
                !native_available(
                    material,
                    &BackdropCapabilities {
                        windows_build: Some(22621),
                        premultiplied_alpha_supported: false,
                    }
                ),
                "{material:?} must not be native without premultiplied alpha support"
            );
            assert!(
                !native_available(
                    material,
                    &BackdropCapabilities {
                        windows_build: None,
                        premultiplied_alpha_supported: true,
                    }
                ),
                "{material:?} must not be native when the build number is unknown"
            );
        }
        // 沉浸式后景额外要求 26100。
        assert!(
            !native_available(
                MaterialKind::Immersive,
                &BackdropCapabilities {
                    windows_build: Some(22621),
                    premultiplied_alpha_supported: true,
                }
            ),
            "the immersive material must not be native below build 26100"
        );
        // 沉浸式后景即便构建号够高，也因为没有可下发的系统取值而不可走原生。
        assert!(
            !native_available(
                MaterialKind::Immersive,
                &BackdropCapabilities {
                    windows_build: Some(26100),
                    premultiplied_alpha_supported: true,
                }
            ),
            "the immersive material must never be native without a confirmed system value"
        );
        assert_eq!(
            unavailable_reason(
                MaterialKind::Immersive,
                &BackdropCapabilities {
                    windows_build: Some(26100),
                    premultiplied_alpha_supported: true,
                }
            ),
            Some(UnavailableReason::SystemChannelUnconfirmed),
            "the immersive material must report the unconfirmed system channel"
        );
        assert!(
            !native_available(
                MaterialKind::Immersive,
                &BackdropCapabilities {
                    windows_build: Some(26100),
                    premultiplied_alpha_supported: false,
                }
            ),
            "the immersive material must not be native without premultiplied alpha support"
        );
    }

    #[test]
    fn the_none_material_is_free_of_any_gate() {
        let caps = BackdropCapabilities {
            windows_build: None,
            premultiplied_alpha_supported: false,
        };
        assert!(
            native_available(MaterialKind::None, &caps),
            "the None material must not be blocked by a platform gate"
        );
        assert_eq!(
            unavailable_reason(MaterialKind::None, &caps),
            None,
            "the None material must never report an unavailable reason"
        );
    }

    #[test]
    fn the_unavailable_reason_names_the_failing_gate() {
        let unknown = BackdropCapabilities {
            windows_build: None,
            premultiplied_alpha_supported: true,
        };
        assert_eq!(
            unavailable_reason(MaterialKind::Mica, &unknown),
            Some(UnavailableReason::BuildUnknown),
            "an unknown build number must be reported as unknown"
        );

        let too_low = BackdropCapabilities {
            windows_build: Some(19045),
            premultiplied_alpha_supported: true,
        };
        assert_eq!(
            unavailable_reason(MaterialKind::Mica, &too_low),
            Some(UnavailableReason::BuildTooLow {
                required: NATIVE_MICA_MIN_BUILD
            }),
            "a build number below the gate must report the required build"
        );
        assert_eq!(
            unavailable_reason(MaterialKind::Immersive, &too_low),
            Some(UnavailableReason::BuildTooLow {
                required: IMMERSIVE_BACKDROP_MIN_BUILD
            }),
            "the immersive material must report its own higher gate"
        );

        let no_premultiplied = BackdropCapabilities {
            windows_build: Some(26100),
            premultiplied_alpha_supported: false,
        };
        assert_eq!(
            unavailable_reason(MaterialKind::Mica, &no_premultiplied),
            Some(UnavailableReason::PremultipliedAlphaUnsupported),
            "a surface without premultiplied alpha must report that gate"
        );

        let available = BackdropCapabilities {
            windows_build: Some(26100),
            premultiplied_alpha_supported: true,
        };
        assert_eq!(
            unavailable_reason(MaterialKind::Mica, &available),
            None,
            "an available material must not report any reason"
        );
    }

    #[test]
    fn only_mica_and_its_variant_have_a_self_drawn_variant() {
        assert!(
            has_self_drawn_variant(MaterialKind::Mica),
            "standard Mica must have a self-drawn variant"
        );
        assert!(
            has_self_drawn_variant(MaterialKind::MicaAlt),
            "the Mica variant must have a self-drawn variant"
        );
        for material in [
            MaterialKind::None,
            MaterialKind::FollowSystem,
            MaterialKind::Acrylic,
            MaterialKind::Immersive,
        ] {
            assert!(
                !has_self_drawn_variant(material),
                "{material:?} must not have a self-drawn variant"
            );
        }
    }

    #[test]
    fn every_combination_matches_the_declared_ladder() {
        let mut checked = 0usize;
        for material in MaterialKind::ALL {
            for source in BackdropSource::ALL {
                for case in &CAPABILITY_CASES {
                    let caps = caps_of(case);
                    let selection = BackdropSelection::new(material, source);
                    let actual = resolve_combination(selection, &caps);
                    let expected = expected_combination(material, source, case);
                    assert_eq!(
                        actual, expected,
                        "material {material:?} source {source:?} on the {} case must resolve as declared",
                        case.name
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(
            checked, 180,
            "the ladder must be pinned for every material, source and capability case"
        );
    }

    #[test]
    fn the_none_material_folds_the_source_axis() {
        let caps = BackdropCapabilities {
            windows_build: Some(26100),
            premultiplied_alpha_supported: true,
        };
        for source in BackdropSource::ALL {
            let combination =
                resolve_combination(BackdropSelection::new(MaterialKind::None, source), &caps);
            assert_eq!(
                combination,
                BackdropCombination {
                    material: MaterialKind::None,
                    source,
                    fallback: None,
                    unavailable: None,
                },
                "the None material must fold the source axis and keep the requested source"
            );
        }
    }

    #[test]
    fn an_explicit_native_choice_reports_the_capability_shortfall() {
        let caps = BackdropCapabilities {
            windows_build: Some(19045),
            premultiplied_alpha_supported: true,
        };
        let combination = resolve_combination(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Native),
            &caps,
        );
        assert_eq!(
            combination,
            BackdropCombination {
                material: MaterialKind::Mica,
                source: BackdropSource::SelfDrawn,
                fallback: None,
                unavailable: Some(UnavailableReason::BuildTooLow {
                    required: NATIVE_MICA_MIN_BUILD
                }),
            },
            "a capability shortfall must be reported as unavailable, not as a fallback"
        );

        let no_variant = resolve_combination(
            BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::Native),
            &caps,
        );
        assert_eq!(
            no_variant,
            BackdropCombination {
                material: MaterialKind::None,
                source: BackdropSource::Native,
                fallback: None,
                unavailable: Some(UnavailableReason::BuildTooLow {
                    required: NATIVE_MICA_MIN_BUILD
                }),
            },
            "a material without a self-drawn variant leaves no backdrop at all"
        );
    }

    #[test]
    fn an_illegal_self_drawn_choice_keeps_the_material_first() {
        let caps = BackdropCapabilities {
            windows_build: Some(26100),
            premultiplied_alpha_supported: true,
        };
        let combination = resolve_combination(
            BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::SelfDrawn),
            &caps,
        );
        assert_eq!(
            combination,
            BackdropCombination {
                material: MaterialKind::Acrylic,
                source: BackdropSource::Native,
                fallback: Some(FallbackReason::NoSelfDrawnVariant {
                    material: MaterialKind::Acrylic
                }),
                unavailable: None,
            },
            "an illegal combination must keep the material and switch the source"
        );

        let weak = BackdropCapabilities {
            windows_build: Some(19045),
            premultiplied_alpha_supported: true,
        };
        let fallback = resolve_combination(
            BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::SelfDrawn),
            &weak,
        );
        assert_eq!(
            fallback,
            BackdropCombination {
                material: MaterialKind::Mica,
                source: BackdropSource::SelfDrawn,
                fallback: Some(FallbackReason::NoSelfDrawnVariant {
                    material: MaterialKind::Acrylic
                }),
                unavailable: None,
            },
            "without a native path the fallback must keep the source and switch the material"
        );
    }

    #[test]
    fn an_auto_choice_always_lands_on_a_usable_combination() {
        let weak = BackdropCapabilities {
            windows_build: Some(19045),
            premultiplied_alpha_supported: true,
        };
        for material in MaterialKind::ALL {
            let combination = resolve_combination(
                BackdropSelection::new(material, BackdropSource::Auto),
                &weak,
            );
            if material == MaterialKind::None {
                // 材质为「无」时来源轴折叠，保留用户请求的来源。
                assert_eq!(combination.source, BackdropSource::Auto);
                assert_eq!(combination.material, MaterialKind::None);
                assert_eq!(combination.fallback, None);
                continue;
            }
            assert_ne!(
                combination.source,
                BackdropSource::Auto,
                "the auto source must always resolve to a concrete source"
            );
            assert!(
                combination.material != MaterialKind::None,
                "the auto source must always land on a material, got {combination:?}"
            );
            assert!(
                combination.fallback.is_some(),
                "the auto source must report why it fell back, got {combination:?}"
            );
        }
    }

    #[test]
    fn reasons_expose_displayable_english_text() {
        let fallback = FallbackReason::NativeUnavailable {
            material: MaterialKind::Mica,
        };
        assert!(!fallback.label().is_empty());
        assert!(!fallback.description().is_empty());
        assert!(
            fallback.description().contains("Mica"),
            "the fallback description must name the material"
        );

        let illegal = FallbackReason::NoSelfDrawnVariant {
            material: MaterialKind::Acrylic,
        };
        assert!(!illegal.label().is_empty());
        assert!(!illegal.description().is_empty());
        assert!(
            illegal.description().contains("Acrylic"),
            "the fallback description must name the material"
        );

        for reason in [
            UnavailableReason::BuildTooLow { required: 22621 },
            UnavailableReason::BuildUnknown,
            UnavailableReason::PremultipliedAlphaUnsupported,
            UnavailableReason::SystemChannelUnconfirmed,
        ] {
            assert!(!reason.label().is_empty());
            assert!(!reason.description().is_empty());
        }
        assert!(
            UnavailableReason::BuildTooLow { required: 22621 }
                .description()
                .contains("22621"),
            "the build gate description must name the required build"
        );
    }

    #[test]
    fn the_baseline_capability_follows_the_legacy_gate() {
        assert!(
            BackdropCapabilities {
                windows_build: Some(22621),
                premultiplied_alpha_supported: true,
            }
            .baseline_native_available(),
            "the baseline must be available from build 22621 with premultiplied alpha"
        );
        assert!(
            !BackdropCapabilities {
                windows_build: Some(22620),
                premultiplied_alpha_supported: true,
            }
            .baseline_native_available(),
            "the baseline must be unavailable below build 22621"
        );
        assert!(
            !BackdropCapabilities {
                windows_build: None,
                premultiplied_alpha_supported: true,
            }
            .baseline_native_available(),
            "the baseline must be unavailable when the build number is unknown"
        );
        assert!(
            !BackdropCapabilities {
                windows_build: Some(26100),
                premultiplied_alpha_supported: false,
            }
            .baseline_native_available(),
            "the baseline must be unavailable without premultiplied alpha"
        );
    }
}

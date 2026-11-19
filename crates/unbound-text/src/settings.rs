//! 文本渲染设置收敛与三档失效判定模型。
//!
//! 将散落在各个子系统的开关收敛为单一可序列化纯值结构 [`TextRenderSettings`]。
//! 写入设置后通过字段级差异比对推导 [`InvalidationTier`]，避免多余失效或部分失效。

use serde::{Deserialize, Serialize};

use crate::glyph::RasterBackend;

/// 文本渲染设置变更引发的失效级别。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvalidationTier {
    /// 预览样式变更或无变化：无需清空任何全局缓存。
    None,
    /// 排版语义变更：清空塑形与换行缓存，不清除字形位图。
    Typographic,
    /// 位图内容变更：清空整张字形图集与全部已光栅化位图，整表重建。
    Bitmap,
}

/// 文本塑形器后端类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TextShaper {
    /// 基于 `cosmic-text` 的高级塑形引擎。
    #[default]
    Cosmic,
}

/// 文本断行模式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LineBreakMode {
    /// 按单词边界换行。
    #[default]
    Word,
    /// 允许在任意字符处换行。
    Anywhere,
    /// 不自动换行。
    None,
}

/// 亚像素相位档位。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PhaseMode {
    /// 8 相位（⅛ 像素细度，掩码 0b111）。
    #[default]
    Eight = 8,
    /// 4 相位（¼ 像素细度，掩码 0b11）。
    Four = 4,
    /// 1 相位（整像素对齐，相位固定为 0）。
    One = 1,
}

impl PhaseMode {
    /// 当前档位包含的相位总数。
    #[must_use]
    pub fn phases(self) -> u8 {
        self as u8
    }

    /// 提取该档位对应的位掩码。
    #[must_use]
    pub fn mask(self) -> u8 {
        match self {
            Self::Eight => 0b111,
            Self::Four => 0b011,
            Self::One => 0,
        }
    }

    /// 是否强制整像素对齐（即 snap_phases == true）。
    #[must_use]
    pub fn snap_phases(self) -> bool {
        matches!(self, Self::One)
    }

    /// 将画笔水平坐标量化为整像素原点与亚像素相位。
    #[must_use]
    pub fn quantize_x(self, x: f32) -> (f32, u8) {
        match self {
            Self::One => (x.round(), 0),
            Self::Four => {
                let fourths = (x * 4.0).round();
                (
                    fourths.div_euclid(4.0),
                    (fourths.rem_euclid(4.0) as u8) & 0b11,
                )
            }
            Self::Eight => {
                let eighths = (x * 8.0).round();
                (
                    eighths.div_euclid(8.0),
                    (eighths.rem_euclid(8.0) as u8) & 0b111,
                )
            }
        }
    }

    /// 当前相位档位下，指定相位对应的水平亚像素偏移量（逻辑像素）。
    #[must_use]
    pub fn phase_offset(self, phase: u8) -> f32 {
        match self {
            Self::One => 0.0,
            Self::Four => (phase & 0b11) as f32 * 0.25,
            Self::Eight => (phase & 0b111) as f32 * 0.125,
        }
    }
}

impl From<bool> for PhaseMode {
    fn from(snap_phases: bool) -> Self {
        if snap_phases {
            Self::One
        } else {
            Self::Eight
        }
    }
}

/// Emoji 呈现模式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EmojiPresentation {
    /// 彩色 Emoji（写入 RGBA8 图集）。
    #[default]
    Color,
    /// 单色 Emoji（写入 R8 图集，以文字颜色染色）。
    Mono,
}

/// Emoji 光栅化后端。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EmojiBackend {
    /// 跟随正文光栅化后端配置。
    #[default]
    Follow,
    /// 强制使用 Windows DirectWrite。
    DirectWrite,
    /// 强制使用 Cosmic (swash)。
    Cosmic,
}

/// Emoji 来源。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EmojiSource {
    #[default]
    System,
    Builtin,
    Grid,
}

/// Emoji 失败回退策略。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EmojiFallback {
    /// 回退到 swash 彩色。此构建不含 swash 时运行期按 `Mono` 执行并记一次回退，
    /// 不静默忽略——否则状态栏的「实际生效」会再次失真（ADR 0017 第 9 条）。
    Swash,
    /// 回退到基字形剪影（单色覆盖率）。
    Mono,
    /// 回退到占位框；尚未实现，当前按 `Mono` 执行并记账。
    Placeholder,
}

impl Default for EmojiFallback {
    /// 默认回退随构建能力翻转：不含 swash 的构建默认 `Mono`。
    ///
    /// 默认值指向一个此构建根本执行不了的策略，会让第一次彩色合成失败就
    /// 落进「回退到不存在的后端」这一分支。
    fn default() -> Self {
        if cfg!(feature = "backend-swash") {
            Self::Swash
        } else {
            Self::Mono
        }
    }
}

/// 距离场生成模式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DistanceFieldMode {
    /// 关闭距离场（普通覆盖率位图）。
    #[default]
    Off,
    /// 单通道欧氏距离场（SDF）。
    Sdf,
    /// 多通道距离场（MTSDF/MSDF，管线未接线）。
    Mtsdf,
}

/// 文本混合色彩空间。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BlendSpace {
    /// sRGB 色彩空间混合。
    #[default]
    Srgb,
    /// 物理线性色彩空间混合。
    Linear,
}

/// 文本合成图层模式。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LayerMode {
    /// 直接绘制到当前目标表面（单层合并）。
    #[default]
    Direct,
    /// 独立透明中间层（离屏图层隔离后合成）。
    Isolated,
}

/// 文本渲染全局与预览设置纯值结构。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextRenderSettings {
    // === 预览样式档 (Preview Style Tier) ===
    /// 字体家族名称（None 表示使用系统默认 UI 字体）。
    pub family: Option<String>,
    /// 字体字重（1..=1000，标准为 400）。
    pub weight: u16,
    /// 字号（单位：逻辑像素）。
    pub size: f32,
    /// 是否斜体。
    pub italic: bool,

    // === 排版语义档 (Typographic Tier) ===
    /// 文本塑形器后端。
    pub shaper: TextShaper,
    /// 换行模式。
    pub line_break: LineBreakMode,
    /// 是否启用双向文本重排（BiDi）。
    pub bidi: bool,
    /// 是否启用连字（Ligatures）。
    pub ligatures: bool,
    /// 字距调整（单位：逻辑像素）。
    pub letter_spacing: f32,
    /// 行高（单位：逻辑像素，0.0 表示使用默认字体度量）。
    pub line_height: f32,

    // === 位图内容档 (Bitmap Content Tier) ===
    /// 光栅化后端。
    pub raster_backend: RasterBackend,
    /// 亚像素相位档位（8 / 4 / 1）。
    pub phase_mode: PhaseMode,
    /// 距离场生成模式（关 / SDF / MTSDF）。
    pub distance_field: DistanceFieldMode,
    /// 是否输出欧氏距离场（SDF）而非单通道覆盖率。
    pub use_sdf: bool,
    /// 距离场扩散范围（单位：纹素）。
    pub sdf_spread: f32,
    /// 每个字形四周的边距纹素。
    pub padding: u32,
    /// Emoji 呈现模式（彩色 / 单色）。
    pub emoji_presentation: EmojiPresentation,
    /// Emoji 光栅化后端（跟随正文 / DirectWrite / Cosmic）。
    pub emoji_backend: EmojiBackend,
    /// Emoji 来源（系统 / 内置 / 网格）。
    pub emoji_source: EmojiSource,
    /// Emoji 回退策略。
    pub emoji_fallback: EmojiFallback,
    /// Emoji 调色板索引。
    pub palette_index: u16,

    // === 合成组 (Composition Group) ===
    /// 混合色彩空间（sRGB / 线性）。
    pub blend_space: BlendSpace,
    /// 图层模式（直接绘制 / 独立图层）。
    pub layer_mode: LayerMode,
}

impl Default for TextRenderSettings {
    fn default() -> Self {
        Self {
            family: None,
            weight: 400,
            size: 14.0,
            italic: false,
            shaper: TextShaper::default(),
            line_break: LineBreakMode::default(),
            bidi: true,
            ligatures: true,
            letter_spacing: 0.0,
            line_height: 0.0,
            raster_backend: RasterBackend::default(),
            phase_mode: PhaseMode::default(),
            distance_field: DistanceFieldMode::default(),
            use_sdf: false,
            sdf_spread: 4.0,
            padding: 2,
            emoji_presentation: EmojiPresentation::default(),
            emoji_backend: EmojiBackend::default(),
            emoji_source: EmojiSource::default(),
            emoji_fallback: EmojiFallback::default(),
            palette_index: 0,
            blend_space: BlendSpace::default(),
            layer_mode: LayerMode::default(),
        }
    }
}

impl PartialEq for TextRenderSettings {
    fn eq(&self, other: &Self) -> bool {
        self.family == other.family
            && self.weight == other.weight
            && self.size.to_bits() == other.size.to_bits()
            && self.italic == other.italic
            && self.shaper == other.shaper
            && self.line_break == other.line_break
            && self.bidi == other.bidi
            && self.ligatures == other.ligatures
            && self.letter_spacing.to_bits() == other.letter_spacing.to_bits()
            && self.line_height.to_bits() == other.line_height.to_bits()
            && self.raster_backend == other.raster_backend
            && self.phase_mode == other.phase_mode
            && self.distance_field == other.distance_field
            && self.use_sdf == other.use_sdf
            && self.sdf_spread.to_bits() == other.sdf_spread.to_bits()
            && self.padding == other.padding
            && self.emoji_presentation == other.emoji_presentation
            && self.emoji_backend == other.emoji_backend
            && self.emoji_source == other.emoji_source
            && self.emoji_fallback == other.emoji_fallback
            && self.palette_index == other.palette_index
            && self.blend_space == other.blend_space
            && self.layer_mode == other.layer_mode
    }
}

impl Eq for TextRenderSettings {}

impl TextRenderSettings {
    /// 应用文本渲染互斥与约束规则。
    ///
    /// 规则 1：距离场开启（SDF）时，相位档位强制锁定为 1（整像素对齐）。
    pub fn enforce_constraints(&mut self) {
        if self.use_sdf || self.distance_field == DistanceFieldMode::Sdf {
            self.use_sdf = true;
            self.distance_field = DistanceFieldMode::Sdf;
            self.phase_mode = PhaseMode::One;
        } else if self.distance_field == DistanceFieldMode::Off {
            self.use_sdf = false;
        }
    }

    /// 是否强制整像素对齐。
    #[must_use]
    pub fn snap_phases(&self) -> bool {
        self.phase_mode.snap_phases()
    }

    /// 比对两份设置，计算失效层级。
    ///
    /// 优先级由高到低：Bitmap > Typographic > None。
    /// 若完全一致或仅调整预览样式，返回 None。
    #[must_use]
    pub fn invalidation_tier(&self, other: &Self) -> InvalidationTier {
        // 首先比对位图内容档：若有任何一项变更，必须整表失效
        if self.raster_backend != other.raster_backend
            || self.phase_mode != other.phase_mode
            || self.distance_field != other.distance_field
            || self.use_sdf != other.use_sdf
            || self.sdf_spread.to_bits() != other.sdf_spread.to_bits()
            || self.padding != other.padding
            || self.emoji_presentation != other.emoji_presentation
            || self.emoji_backend != other.emoji_backend
            || self.emoji_source != other.emoji_source
            || self.emoji_fallback != other.emoji_fallback
            || self.palette_index != other.palette_index
        {
            return InvalidationTier::Bitmap;
        }

        // 其次比对排版语义档：若有任何一项变更，清空排版与换行缓存
        if self.shaper != other.shaper
            || self.line_break != other.line_break
            || self.bidi != other.bidi
            || self.ligatures != other.ligatures
            || self.letter_spacing.to_bits() != other.letter_spacing.to_bits()
            || self.line_height.to_bits() != other.line_height.to_bits()
        {
            return InvalidationTier::Typographic;
        }

        // 预览样式档（family, weight, size, italic）变更，或完全无变更：不清任何全局缓存
        InvalidationTier::None
    }
}

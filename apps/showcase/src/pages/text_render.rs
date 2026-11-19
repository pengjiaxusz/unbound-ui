//! 文本渲染控制台（Text Render Console）展台页面。
//! lang-allow: live preview text specimens; CJK sample text is the data
//!
//! 承载文本渲染管线运行期参数调整、三态能力灰化、实时预览与底层实际生效状态报告。
//! 页面分为三大区域：
//! 1. 左侧：六组折叠参数容器（字体、排版、像素、距离场、合成、Emoji）；
//! 2. 右侧：三段对照预览区（中英混排、纯汉字、Emoji 符号）；
//! 3. 底部：横跨整宽的实际生效状态栏，读取只读快照报告。

use accesskit::Role;
use unbound_math::Rect;
use unbound_text::{
    BlendSpace, CapabilityStatus, DistanceFieldMode, EmojiBackend, EmojiPresentation, EmojiSource,
    LayerMode, LineBreakMode, PhaseMode, RasterBackend, TextCapabilities, TextRenderReport,
    TextRenderSettings, TextShaper,
};

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

/// 六组折叠框架的部件起始编号。
pub const GROUP_PART_BASE: u16 = 10;
/// 状态栏部件编号。
pub const STATUS_BAR_PART: u16 = 20;
/// 字体家族选项起始部件编号。
pub const FONT_FAMILY_PART_BASE: u16 = 100;
/// 字体字重选项起始部件编号。
pub const FONT_WEIGHT_PART_BASE: u16 = 110;
/// 字体字号选项起始部件编号。
pub const FONT_SIZE_PART_BASE: u16 = 120;
/// 字体斜体切换开关部件编号。
pub const FONT_ITALIC_PART: u16 = 130;

/// 字体家族候选选项表。
pub const FONT_FAMILY_OPTIONS: &[(&str, Option<&str>)] = &[
    ("Default", None),
    ("Segoe UI", Some("Segoe UI")),
    ("Cascadia", Some("Cascadia Mono")),
    ("Arial", Some("Arial")),
];

/// 字体字重候选选项表。
pub const FONT_WEIGHT_OPTIONS: &[(&str, u16)] = &[
    ("Regular 400", 400),
    ("Medium 500", 500),
    ("SemiBold 600", 600),
    ("Bold 700", 700),
];

/// 字体字号候选选项表。
pub const FONT_SIZE_OPTIONS: &[(&str, f32)] = &[
    ("12px", 12.0),
    ("14px", 14.0),
    ("18px", 18.0),
    ("24px", 24.0),
];

/// 相位档位选项部件起始编号（8 / 4 / 1）。
pub const PHASE_PART_BASE: u16 = 30;

/// DirectWrite 光栅化后端切换部件编号。
pub const PART_BACKEND_DW: u16 = 40;
/// Cosmic (Swash) 光栅化后端切换部件编号。
pub const PART_BACKEND_COSMIC: u16 = 41;

/// 相位档位定义。
pub struct PhaseOptionDef {
    pub mode: unbound_text::PhaseMode,
    pub key: TextFeatureKey,
    pub label: &'static str,
    pub description: &'static str,
}

/// 亚像素相位选项列表。
pub const PHASE_OPTIONS: [PhaseOptionDef; 3] = [
    PhaseOptionDef {
        mode: unbound_text::PhaseMode::Eight,
        key: TextFeatureKey::PhaseEight,
        label: "8 Phases",
        description: "1/8 px subpixel precision (recommended)",
    },
    PhaseOptionDef {
        mode: unbound_text::PhaseMode::Four,
        key: TextFeatureKey::PhaseFour,
        label: "4 Phases",
        description: "1/4 px subpixel precision",
    },
    PhaseOptionDef {
        mode: unbound_text::PhaseMode::One,
        key: TextFeatureKey::PhaseOne,
        label: "1 Phase",
        description: "Pixel-snapped (phase 0 only)",
    },
];

/// 距离场调节部件编号。
pub const PART_DF_OFF: u16 = 50;
pub const PART_DF_SDF: u16 = 51;
pub const PART_DF_MTSDF: u16 = 52;

/// 合成组调节部件编号。
pub const PART_COMP_SRGB: u16 = 60;
pub const PART_COMP_LINEAR: u16 = 61;
pub const PART_COMP_LAYER_DIRECT: u16 = 62;
pub const PART_COMP_LAYER_ISOLATED: u16 = 63;

// === 排版组各控件部件编号 ===
pub const PART_SHAPER_COSMIC: u16 = 201;
pub const PART_SHAPER_NATIVE: u16 = 202;

pub const PART_LINEBREAK_WORD: u16 = 211;
pub const PART_LINEBREAK_ANYWHERE: u16 = 212;
pub const PART_LINEBREAK_NONE: u16 = 213;

pub const PART_BIDI_ON: u16 = 221;
pub const PART_BIDI_OFF: u16 = 222;

pub const PART_LIGATURES_ON: u16 = 231;
pub const PART_LIGATURES_OFF: u16 = 232;

pub const PART_LETTER_SPACING_0: u16 = 241;
pub const PART_LETTER_SPACING_1: u16 = 242;
pub const PART_LETTER_SPACING_2: u16 = 243;

pub const PART_LINE_HEIGHT_AUTO: u16 = 251;
pub const PART_LINE_HEIGHT_20: u16 = 252;
pub const PART_LINE_HEIGHT_28: u16 = 253;

// Emoji 组部件编号 (301..350)
pub const PART_EMOJI_PRES_COLOR: u16 = 301;
pub const PART_EMOJI_PRES_MONO: u16 = 302;
pub const PART_EMOJI_BACKEND_FOLLOW: u16 = 310;
pub const PART_EMOJI_BACKEND_DW: u16 = 311;
pub const PART_EMOJI_BACKEND_COSMIC: u16 = 312;
pub const PART_EMOJI_SOURCE_SYSTEM: u16 = 320;
pub const PART_EMOJI_SOURCE_BUILTIN: u16 = 321;
pub const PART_EMOJI_SOURCE_GRID: u16 = 322;

// 预设方案部件编号 (401..404)
pub const PART_PRESET_BASE: u16 = 400;
pub const PART_PRESET_WINUI: u16 = 401;
pub const PART_PRESET_CROSS_PLATFORM: u16 = 402;
pub const PART_PRESET_PIXEL: u16 = 403;
pub const PART_PRESET_CUSTOM: u16 = 404;

/// 控制台六大功能参数分组。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextConsoleGroup {
    /// 字体样式组（预览样式：家族、字重、字号、斜体）。
    Font = 0,
    /// 排版语义组（塑形器、断行、双向、连字、字距、行高）。
    Typography = 1,
    /// 像素与相位组（光栅器、亚像素相位、抗锯齿、网格拟合）。
    Pixel = 2,
    /// 距离场组（SDF、MTSDF、扩散半径）。
    DistanceField = 3,
    /// 合成组（颜色空间、混合模式、图层）。
    Composition = 4,
    /// Emoji 组（来源、呈现、后端、调色板、回退）。
    Emoji = 5,
}

impl TextConsoleGroup {
    /// 全部六个分组的切片数组。
    pub const ALL: [TextConsoleGroup; 6] = [
        Self::Font,
        Self::Typography,
        Self::Pixel,
        Self::DistanceField,
        Self::Composition,
        Self::Emoji,
    ];

    /// 分组英文标题。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Font => "Font & Style",
            Self::Typography => "Typography",
            Self::Pixel => "Pixel & Phase",
            Self::DistanceField => "Distance Field",
            Self::Composition => "Composition",
            Self::Emoji => "Emoji",
        }
    }

    /// 分组简短描述。
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Font => "Preview font family, weight, size and slant",
            Self::Typography => "Shaper backend, line breaking, BiDi and ligatures",
            Self::Pixel => "Rasterizer backend, subpixel phases and hinting",
            Self::DistanceField => "Signed distance field and pipeline mode",
            Self::Composition => "Blending space and render layers",
            Self::Emoji => "Presentation mode, backend and color palettes",
        }
    }
}

/// 预设渲染方案枚举。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextRenderPreset {
    /// Windows WinUI 3 原生渲染风格（DirectWrite 覆盖率 + 8 亚像素相位 + sRGB + Segoe UI）。
    #[default]
    WinuiNative,
    /// 跨平台视觉一致风格（Swash + 8 亚像素相位 + 线性合成 + 图层隔离）。
    CrossPlatform,
    /// 像素风与低开销（Cosmic + 1 相位整数栅格对齐 + 单色 Emoji + 无连字）。
    Pixel,
    /// 用户自定义调节（手动修改任意项后自动落入此档）。
    Custom,
}

impl TextRenderPreset {
    pub const ALL: [TextRenderPreset; 4] = [
        Self::WinuiNative,
        Self::CrossPlatform,
        Self::Pixel,
        Self::Custom,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WinuiNative => "WinUI Native",
            Self::CrossPlatform => "Cross-Platform",
            Self::Pixel => "Pixel Style",
            Self::Custom => "Custom",
        }
    }
}

/// 文本渲染控制台的持久页面状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRenderPageState {
    /// 当前选中的预设方案。
    pub preset: TextRenderPreset,
    /// 六个分组的展开/折叠状态（true 为展开，false 为折叠）。
    pub group_expanded: [bool; 6],
    /// 状态栏回退原因详情是否展开。
    pub fallback_details_expanded: bool,
    /// 运行期文本渲染设置纯值快照。
    pub settings: TextRenderSettings,
    /// 文本渲染实际生效报告快照。
    pub report: TextRenderReport,
}

thread_local! {
    static CUSTOM_PERSISTENCE_PATH: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

impl Default for TextRenderPageState {
    fn default() -> Self {
        Self::fresh_default()
    }
}

impl TextRenderPageState {
    /// 默认初始空白状态。
    #[must_use]
    pub fn fresh_default() -> Self {
        Self {
            preset: TextRenderPreset::WinuiNative,
            group_expanded: [true; 6],
            fallback_details_expanded: false,
            settings: TextRenderSettings::default(),
            report: TextRenderReport::default(),
        }
    }

    /// 设置线程级持久化路径覆盖（供并发测试线程隔离使用）。
    pub fn set_thread_persistence_path(path: Option<std::path::PathBuf>) {
        CUSTOM_PERSISTENCE_PATH.with(|p| {
            *p.borrow_mut() = path;
        });
    }
    /// 切换指定分组的折叠状态。
    pub fn toggle_group(&mut self, index: usize) {
        if index < self.group_expanded.len() {
            self.group_expanded[index] = !self.group_expanded[index];
            self.persist();
        }
    }

    /// 切换状态栏回退原因展开状态。
    pub fn toggle_fallback_details(&mut self) {
        self.fallback_details_expanded = !self.fallback_details_expanded;
        self.persist();
    }

    /// 设置字体家族。
    pub fn set_family(&mut self, family: Option<String>) {
        self.settings.family = family;
        self.mark_custom();
        self.persist();
    }

    /// 设置字体字重。
    pub fn set_weight(&mut self, weight: u16) {
        self.settings.weight = weight;
        self.mark_custom();
        self.persist();
    }

    /// 设置字号。
    pub fn set_size(&mut self, size: f32) {
        self.settings.size = size;
        self.mark_custom();
        self.persist();
    }

    /// 切换斜体状态。
    pub fn toggle_italic(&mut self) {
        self.settings.italic = !self.settings.italic;
        self.mark_custom();
        self.persist();
    }

    /// 标记为自定义档位。
    pub fn mark_custom(&mut self) {
        self.preset = TextRenderPreset::Custom;
    }

    /// 应用预设方案（按机器能力自动降级）。
    pub fn apply_preset(&mut self, preset: TextRenderPreset, caps: &TextCapabilities) {
        self.preset = preset;
        match preset {
            TextRenderPreset::WinuiNative => {
                let raster_backend = if caps.directwrite.is_available() {
                    RasterBackend::DirectWrite
                } else {
                    RasterBackend::Cosmic
                };
                self.settings.raster_backend = raster_backend;
                self.settings.phase_mode = PhaseMode::Eight;
                self.settings.distance_field = DistanceFieldMode::Off;
                self.settings.use_sdf = false;
                self.settings.blend_space = BlendSpace::Srgb;
                self.settings.layer_mode = LayerMode::Direct;
                self.settings.emoji_presentation = EmojiPresentation::Color;
                self.settings.emoji_backend = EmojiBackend::Follow;
                self.settings.shaper = TextShaper::Cosmic;
                self.settings.line_break = LineBreakMode::Word;
                self.settings.bidi = true;
                self.settings.ligatures = true;
                self.settings.letter_spacing = 0.0;
                self.settings.line_height = 0.0;
                self.settings.family = Some("Segoe UI".to_string());
                self.settings.weight = 400;
                self.settings.size = 14.0;
                self.settings.italic = false;
            }
            TextRenderPreset::CrossPlatform => {
                self.settings.raster_backend = RasterBackend::Cosmic;
                self.settings.phase_mode = PhaseMode::Eight;
                self.settings.distance_field = DistanceFieldMode::Off;
                self.settings.use_sdf = false;
                self.settings.blend_space = BlendSpace::Linear;
                self.settings.layer_mode = LayerMode::Isolated;
                self.settings.emoji_presentation = EmojiPresentation::Color;
                self.settings.emoji_backend = EmojiBackend::Cosmic;
                self.settings.shaper = TextShaper::Cosmic;
                self.settings.line_break = LineBreakMode::Word;
                self.settings.bidi = true;
                self.settings.ligatures = true;
                self.settings.letter_spacing = 0.0;
                self.settings.line_height = 0.0;
                self.settings.family = None;
                self.settings.weight = 400;
                self.settings.size = 14.0;
                self.settings.italic = false;
            }
            TextRenderPreset::Pixel => {
                self.settings.raster_backend = RasterBackend::Cosmic;
                self.settings.phase_mode = PhaseMode::One;
                self.settings.distance_field = DistanceFieldMode::Off;
                self.settings.use_sdf = false;
                self.settings.blend_space = BlendSpace::Srgb;
                self.settings.layer_mode = LayerMode::Direct;
                self.settings.emoji_presentation = EmojiPresentation::Mono;
                self.settings.emoji_backend = EmojiBackend::Cosmic;
                self.settings.shaper = TextShaper::Cosmic;
                self.settings.line_break = LineBreakMode::Word;
                self.settings.bidi = true;
                self.settings.ligatures = false;
                self.settings.letter_spacing = 0.0;
                self.settings.line_height = 0.0;
                self.settings.family = Some("Cascadia Mono".to_string());
                self.settings.weight = 400;
                self.settings.size = 14.0;
                self.settings.italic = false;
            }
            TextRenderPreset::Custom => {}
        }
        self.persist();
    }

    /// 序列化状态为紧凑纯文本。
    #[must_use]
    pub fn serialize_to_string(&self) -> String {
        let groups = self
            .group_expanded
            .iter()
            .map(|&b| if b { '1' } else { '0' })
            .collect::<String>();
        format!(
            "preset={:?}\ngroups={}\nfallback={}\nbackend={:?}\nphase={:?}\nsdf={}\nblend={:?}\nlayer={:?}\nemoji_pres={:?}\nemoji_be={:?}\nemoji_src={:?}\nshaper={:?}\nlinebreak={:?}\nbidi={}\nligatures={}\nspacing={}\nline_h={}\nsize={}\nweight={}\nitalic={}\n",
            self.preset,
            groups,
            self.fallback_details_expanded,
            self.settings.raster_backend,
            self.settings.phase_mode,
            self.settings.distance_field != DistanceFieldMode::Off || self.settings.use_sdf,
            self.settings.blend_space,
            self.settings.layer_mode,
            self.settings.emoji_presentation,
            self.settings.emoji_backend,
            self.settings.emoji_source,
            self.settings.shaper,
            self.settings.line_break,
            self.settings.bidi,
            self.settings.ligatures,
            self.settings.letter_spacing,
            self.settings.line_height,
            self.settings.size,
            self.settings.weight,
            self.settings.italic,
        )
    }

    /// 从纯文本反序列化状态。
    #[must_use]
    pub fn deserialize_from_str(s: &str) -> Option<Self> {
        let mut state = Self {
            preset: TextRenderPreset::WinuiNative,
            group_expanded: [true; 6],
            fallback_details_expanded: false,
            settings: TextRenderSettings::default(),
            report: TextRenderReport::default(),
        };

        for line in s.lines() {
            let (k, v) = line.split_once('=')?;
            match k.trim() {
                "preset" => {
                    state.preset = match v.trim() {
                        "WinuiNative" => TextRenderPreset::WinuiNative,
                        "CrossPlatform" => TextRenderPreset::CrossPlatform,
                        "Pixel" => TextRenderPreset::Pixel,
                        _ => TextRenderPreset::Custom,
                    };
                }
                "groups" => {
                    for (i, ch) in v.trim().chars().take(6).enumerate() {
                        state.group_expanded[i] = ch == '1';
                    }
                }
                "fallback" => {
                    state.fallback_details_expanded = v.trim() == "true";
                }
                "backend" => {
                    if v.trim().contains("DirectWrite") {
                        state.settings.raster_backend = RasterBackend::DirectWrite;
                    } else {
                        state.settings.raster_backend = RasterBackend::Cosmic;
                    }
                }
                "phase" => {
                    state.settings.phase_mode = if v.trim().contains("Four") {
                        PhaseMode::Four
                    } else if v.trim().contains("One") {
                        PhaseMode::One
                    } else {
                        PhaseMode::Eight
                    };
                }
                "sdf" => {
                    let on = v.trim() == "true";
                    state.settings.use_sdf = on;
                    state.settings.distance_field = if on {
                        DistanceFieldMode::Sdf
                    } else {
                        DistanceFieldMode::Off
                    };
                }
                "blend" => {
                    state.settings.blend_space = if v.trim().contains("Linear") {
                        BlendSpace::Linear
                    } else {
                        BlendSpace::Srgb
                    };
                }
                "layer" => {
                    state.settings.layer_mode = if v.trim().contains("Isolated") {
                        LayerMode::Isolated
                    } else {
                        LayerMode::Direct
                    };
                }
                "emoji_pres" => {
                    state.settings.emoji_presentation = if v.trim().contains("Mono") {
                        EmojiPresentation::Mono
                    } else {
                        EmojiPresentation::Color
                    };
                }
                "emoji_be" => {
                    state.settings.emoji_backend = if v.trim().contains("DirectWrite") {
                        EmojiBackend::DirectWrite
                    } else if v.trim().contains("Cosmic") {
                        EmojiBackend::Cosmic
                    } else {
                        EmojiBackend::Follow
                    };
                }
                "emoji_src" => {
                    state.settings.emoji_source = if v.trim().contains("Builtin") {
                        EmojiSource::Builtin
                    } else if v.trim().contains("Grid") {
                        EmojiSource::Grid
                    } else {
                        EmojiSource::System
                    };
                }
                "shaper" => {
                    state.settings.shaper = TextShaper::Cosmic;
                }
                "linebreak" => {
                    state.settings.line_break = if v.trim().contains("Anywhere") {
                        LineBreakMode::Anywhere
                    } else if v.trim().contains("None") {
                        LineBreakMode::None
                    } else {
                        LineBreakMode::Word
                    };
                }
                "bidi" => {
                    state.settings.bidi = v.trim() == "true";
                }
                "ligatures" => {
                    state.settings.ligatures = v.trim() == "true";
                }
                "spacing" => {
                    if let Ok(val) = v.trim().parse::<f32>() {
                        state.settings.letter_spacing = val;
                    }
                }
                "line_h" => {
                    if let Ok(val) = v.trim().parse::<f32>() {
                        state.settings.line_height = val;
                    }
                }
                "size" => {
                    if let Ok(val) = v.trim().parse::<f32>() {
                        state.settings.size = val;
                    }
                }
                "weight" => {
                    if let Ok(val) = v.trim().parse::<u16>() {
                        state.settings.weight = val;
                    }
                }
                "italic" => {
                    state.settings.italic = v.trim() == "true";
                }
                _ => {}
            }
        }

        Some(state)
    }

    /// 持久化状态至临时目录配置文件。
    pub fn persist(&self) {
        let path = Self::persistence_path();
        let _ = std::fs::write(path, self.serialize_to_string());
    }

    /// 尝试从持久化配置文件恢复。
    #[must_use]
    pub fn try_restore() -> Option<Self> {
        let path = Self::persistence_path();
        let content = std::fs::read_to_string(path).ok()?;
        Self::deserialize_from_str(&content)
    }

    /// 持久化文件存储路径。
    #[must_use]
    pub fn persistence_path() -> std::path::PathBuf {
        CUSTOM_PERSISTENCE_PATH.with(|p| {
            if let Some(custom) = p.borrow().as_ref() {
                return custom.clone();
            }
            std::env::temp_dir().join("unbound_showcase_text_render_state.txt")
        })
    }

    /// 清理持久化文件（供测试重置环境）。
    pub fn clear_persisted() {
        let _ = std::fs::remove_file(Self::persistence_path());
    }
}

/// 表驱动能力灰化与互斥检查项。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextFeatureKey {
    // 预览样式：字体四项
    FontFamily,
    FontWeight,
    FontSize,
    FontItalic,
    // 光栅化后端
    BackendDirectWrite,
    BackendCosmic,
    // 相位档位
    PhaseEight,
    PhaseFour,
    PhaseOne,
    // 距离场
    DistanceFieldOff,
    DistanceFieldSdf,
    DistanceFieldMtsdf,
    // 拟合与抗锯齿
    Hinting,
    SubpixelAntialiasing,
    // Emoji 来源
    EmojiSourceSystem,
    EmojiSourceBuiltin,
    EmojiSourceGrid,
    // Emoji 呈现
    EmojiPresentationColor,
    EmojiPresentationMono,
    // Emoji 后端
    EmojiBackendFollow,
    EmojiBackendDirectWrite,
    EmojiBackendCosmic,
    // Emoji 调色板
    EmojiPalette,
    // 塑形器
    ShaperCosmic,
    ShaperNative,
    // 合成色彩空间
    CompositionSrgb,
    CompositionLinear,
    // 合成图层模式
    CompositionLayerDirect,
    CompositionLayerIsolated,
}

/// 控件特性状态决议结果。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FeatureResolution {
    /// 控件是否被禁用。
    pub disabled: bool,
    /// 禁用原因（若启用则为 None）。
    pub reason: Option<&'static str>,
}

impl FeatureResolution {
    /// 可用状态常量。
    pub const AVAILABLE: Self = Self {
        disabled: false,
        reason: None,
    };

    /// 禁用状态构造。
    #[must_use]
    pub const fn disabled(reason: &'static str) -> Self {
        Self {
            disabled: true,
            reason: Some(reason),
        }
    }
}

/// 静态表驱动规则项。
pub struct FeatureRule {
    /// 规则对应的特性标识。
    pub key: TextFeatureKey,
    /// 能力查询函数：从三态能力中提取当前底层支持情况。
    pub query_capability: fn(&TextCapabilities) -> CapabilityStatus,
    /// 设置上下文互斥与约束检查。
    pub check_constraint:
        Option<fn(&TextCapabilities, &TextRenderSettings) -> Option<&'static str>>,
}

/// 全局表驱动灰化规则表。
pub const FEATURE_RULES: &[FeatureRule] = &[
    // 预览样式：字体家族
    FeatureRule {
        key: TextFeatureKey::FontFamily,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 预览样式：字重
    FeatureRule {
        key: TextFeatureKey::FontWeight,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 预览样式：字号
    FeatureRule {
        key: TextFeatureKey::FontSize,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 预览样式：斜体
    FeatureRule {
        key: TextFeatureKey::FontItalic,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // DirectWrite 后端
    FeatureRule {
        key: TextFeatureKey::BackendDirectWrite,
        query_capability: |caps| caps.directwrite,
        check_constraint: None,
    },
    // Cosmic (swash) 后端
    FeatureRule {
        key: TextFeatureKey::BackendCosmic,
        query_capability: |caps| caps.cosmic,
        check_constraint: None,
    },
    // 8 相位
    FeatureRule {
        key: TextFeatureKey::PhaseEight,
        query_capability: |caps| caps.subpixel_phase_8,
        check_constraint: Some(|_, settings| {
            if settings.use_sdf || settings.distance_field != DistanceFieldMode::Off {
                Some("Phase locked to 1 when distance field is active")
            } else {
                None
            }
        }),
    },
    // 4 相位
    FeatureRule {
        key: TextFeatureKey::PhaseFour,
        query_capability: |caps| caps.subpixel_phase_4,
        check_constraint: Some(|_, settings| {
            if settings.use_sdf || settings.distance_field != DistanceFieldMode::Off {
                Some("Phase locked to 1 when distance field is active")
            } else {
                None
            }
        }),
    },
    // 1 相位
    FeatureRule {
        key: TextFeatureKey::PhaseOne,
        query_capability: |caps| caps.subpixel_phase_1,
        check_constraint: None,
    },
    // 距离场：关
    FeatureRule {
        key: TextFeatureKey::DistanceFieldOff,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 距离场：SDF
    FeatureRule {
        key: TextFeatureKey::DistanceFieldSdf,
        query_capability: |caps| caps.sdf,
        check_constraint: None,
    },
    // 距离场：MTSDF
    FeatureRule {
        key: TextFeatureKey::DistanceFieldMtsdf,
        query_capability: |caps| caps.mtsdf,
        check_constraint: None,
    },
    // 网格拟合 (Hinting)
    FeatureRule {
        key: TextFeatureKey::Hinting,
        query_capability: |caps| caps.hinting,
        check_constraint: Some(|_, settings| {
            if settings.phase_mode.phases() > 1 {
                Some("Hinting conflicts with subpixel phase quantization")
            } else {
                None
            }
        }),
    },
    // 子像素抗锯齿 (RGB / BGR)
    FeatureRule {
        key: TextFeatureKey::SubpixelAntialiasing,
        query_capability: |caps| caps.subpixel_antialiasing,
        check_constraint: None,
    },
    // Emoji 来源：系统
    FeatureRule {
        key: TextFeatureKey::EmojiSourceSystem,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // Emoji 来源：内置
    FeatureRule {
        key: TextFeatureKey::EmojiSourceBuiltin,
        query_capability: |_| CapabilityStatus::Unavailable("Bundled fonts not implemented"),
        check_constraint: None,
    },
    // Emoji 来源：网格
    FeatureRule {
        key: TextFeatureKey::EmojiSourceGrid,
        query_capability: |_| CapabilityStatus::Unavailable("Grid atlas fonts not implemented"),
        check_constraint: None,
    },
    // Emoji 呈现：彩色
    FeatureRule {
        key: TextFeatureKey::EmojiPresentationColor,
        query_capability: |caps| caps.color_glyphs,
        check_constraint: Some(|caps, settings| {
            if settings.emoji_backend == unbound_text::EmojiBackend::DirectWrite {
                caps.color_directwrite.reason()
            } else {
                None
            }
        }),
    },
    // Emoji 呈现：单色
    FeatureRule {
        key: TextFeatureKey::EmojiPresentationMono,
        query_capability: |caps| caps.mono_fallback,
        check_constraint: None,
    },
    // Emoji 后端：跟随正文
    FeatureRule {
        key: TextFeatureKey::EmojiBackendFollow,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // Emoji 后端：DirectWrite
    FeatureRule {
        key: TextFeatureKey::EmojiBackendDirectWrite,
        query_capability: |caps| caps.directwrite,
        check_constraint: Some(|caps, settings| {
            if settings.emoji_presentation == unbound_text::EmojiPresentation::Color {
                caps.color_directwrite.reason()
            } else {
                caps.mono_directwrite.reason()
            }
        }),
    },
    // Emoji 后端：Cosmic
    FeatureRule {
        key: TextFeatureKey::EmojiBackendCosmic,
        query_capability: |caps| caps.cosmic,
        check_constraint: None,
    },
    // Emoji 调色板：字体只提供一个调色板时该维度无处可选
    FeatureRule {
        key: TextFeatureKey::EmojiPalette,
        query_capability: |caps| caps.color_palettes,
        check_constraint: None,
    },
    // 塑形器：Cosmic
    FeatureRule {
        key: TextFeatureKey::ShaperCosmic,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 塑形器：原生
    FeatureRule {
        key: TextFeatureKey::ShaperNative,
        query_capability: |_| {
            CapabilityStatus::Unavailable("Native and simple shapers not implemented")
        },
        check_constraint: None,
    },
    // 合成色彩空间：sRGB
    FeatureRule {
        key: TextFeatureKey::CompositionSrgb,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 合成色彩空间：线性
    FeatureRule {
        key: TextFeatureKey::CompositionLinear,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 合成图层模式：Direct
    FeatureRule {
        key: TextFeatureKey::CompositionLayerDirect,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
    // 合成图层模式：Isolated
    FeatureRule {
        key: TextFeatureKey::CompositionLayerIsolated,
        query_capability: |_| CapabilityStatus::Available,
        check_constraint: None,
    },
];

/// 通过能力表与当前设置决议指定特性的禁用状态与原因。
#[must_use]
pub fn resolve_feature(
    key: TextFeatureKey,
    caps: &TextCapabilities,
    settings: &TextRenderSettings,
) -> FeatureResolution {
    let rule = FEATURE_RULES.iter().find(|r| r.key == key);
    let Some(rule) = rule else {
        return FeatureResolution::AVAILABLE;
    };

    // 1. 底层能力探测：Unavailable 或 NotCompiled 必然导致禁用
    let cap_status = (rule.query_capability)(caps);
    match cap_status {
        CapabilityStatus::Unavailable(reason) => return FeatureResolution::disabled(reason),
        CapabilityStatus::NotCompiled => {
            return FeatureResolution::disabled("Feature not included in this build");
        }
        CapabilityStatus::Available => {}
    }

    // 2. 上下文互斥约束判定
    if let Some(check_constraint) = rule.check_constraint {
        if let Some(reason) = check_constraint(caps, settings) {
            return FeatureResolution::disabled(reason);
        }
    }

    FeatureResolution::AVAILABLE
}

/// 启用状态下补充的行为说明（悬停与读屏提示）。
///
/// 只有「选项名本身说不清实际行为」的维度才有说明，因此不是每条规则都配一条。
/// 说明与 [`FEATURE_RULES`] 同源：都只读 [`TextCapabilities`] 与当前设置，
/// 面板不另存一份判定。
#[must_use]
pub fn feature_hint(
    key: TextFeatureKey,
    caps: &TextCapabilities,
    settings: &TextRenderSettings,
) -> Option<&'static str> {
    // 彩色 + DirectWrite 从「回退 swash」变成「层合成」，行为与代价都要讲清楚：
    // 层数决定 analysis 次数，因此相位锁 0；翻译不出层的簇逐簇回退（ADR 0017）。
    // 悬停说明描述「选上之后会发生什么」，所以只要求另一维已经就位，不要求
    // 本维已经被选中——否则只有选完才看得到解释。
    let color = settings.emoji_presentation == unbound_text::EmojiPresentation::Color;
    let directwrite = settings.emoji_backend == unbound_text::EmojiBackend::DirectWrite;
    let color_layers = caps.color_directwrite.is_available();
    match key {
        TextFeatureKey::EmojiPresentationColor if color_layers && directwrite => {
            Some("Color layers composited per cluster, phase locked to 0, per-cluster fallback")
        }
        TextFeatureKey::EmojiBackendDirectWrite if color_layers && color => {
            Some("DirectWrite layer traversal: one quad per cluster, phase 0, per-cluster fallback")
        }
        TextFeatureKey::EmojiPalette => {
            Some("Palette index is passed through; source fonts ship a single palette")
        }
        _ => None,
    }
}

pub const TEXT_RENDER_INFO: PageInfo = PageInfo {
    title: "Text Rendering Pipeline & Rasterizer Console",
    short: "Text Render",
    description: "Interactive text rasterization, subpixel phase quantization, distance field generation and emoji pipeline inspection console.",
    crates: &[
        "unbound-text",
        "unbound-app",
        "unbound-platform",
    ],
    keywords: "text render directwrite swash cosmic sdf mtsdf phase subpixel emoji glyph rasterizer",
};

pub const TEXT_RENDER_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Text Render Pipeline Console",
    "Toggle rasterizer backends, subpixel phase modes and distance field settings. The status bar displays effective values resolved by the host.",
    &["unbound-text", "unbound-app"],
    r#"let settings = TextRenderSettings::default();
let report = TextRenderReport::default();
let resolution = resolve_feature(TextFeatureKey::BackendDirectWrite, &capabilities, &settings);"#,
    "Every collapsible group header publishes the Button role, an accessible name and the expanded state; the status bar is accessible and reports effective values.",
    &[],
    DemoKind::TextRender,
)
.with_height(840.0)];

/// 渲染文本渲染控制台舞台主入口。
pub fn text_render(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let state = &ctx.state.text_render;

    // 底部状态栏高度
    let status_bar_h = 48.0;
    let status_bar_margin = 8.0;
    let main_h = (rect.size.y - status_bar_h - status_bar_margin).max(100.0);

    // 左右两栏布局
    let column_gap = 12.0;
    let left_w = (rect.size.x * 0.44).clamp(260.0, 420.0);
    let right_w = (rect.size.x - left_w - column_gap).max(200.0);

    let left_rect = Rect::new(rect.origin.x, rect.origin.y, left_w, main_h);
    let right_rect = Rect::new(
        rect.origin.x + left_w + column_gap,
        rect.origin.y,
        right_w,
        main_h,
    );
    let status_bar_rect = Rect::new(
        rect.origin.x,
        rect.origin.y + rect.size.y - status_bar_h,
        rect.size.x,
        status_bar_h,
    );

    // 1. 绘制左侧六组折叠框架
    draw_groups_panel(ctx, left_rect, state);

    // 2. 绘制右侧文本预览区
    draw_preview_panel(ctx, right_rect, &state.settings);

    // 3. 绘制底部实际生效状态栏
    draw_status_bar(
        ctx,
        status_bar_rect,
        &state.report,
        state.fallback_details_expanded,
    );
}

/// 绘制左侧六组折叠容器。
fn draw_groups_panel(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    let header_h = 28.0;
    let item_gap = 6.0;
    let mut current_y = rect.origin.y;

    // 0. 顶部预设方案切换栏
    let preset_h = 26.0;
    let preset_rect = Rect::new(rect.origin.x, current_y, rect.size.x, preset_h);
    draw_preset_bar(ctx, preset_rect, state);
    current_y += preset_h + item_gap;

    for (index, group) in TextConsoleGroup::ALL.iter().enumerate() {
        let is_expanded = state.group_expanded[index];
        let part = GROUP_PART_BASE + index as u16;

        let content_h = if is_expanded {
            match group {
                TextConsoleGroup::Font => 78.0,
                TextConsoleGroup::Typography => 176.0,
                TextConsoleGroup::Pixel => 58.0,
                TextConsoleGroup::DistanceField => 58.0,
                TextConsoleGroup::Composition => 68.0,
                TextConsoleGroup::Emoji => 78.0,
            }
        } else {
            0.0
        };
        let total_group_h = header_h + content_h;

        let header_rect = Rect::new(rect.origin.x, current_y, rect.size.x, header_h);
        ctx.canvas.rect(
            header_rect,
            4.0,
            palette.card_bg,
            palette.card_border,
            1.0,
            0.0,
        );

        // 展开箭头指示
        let arrow = if is_expanded { "v" } else { ">" };
        let header_title = format!("{}  {}", arrow, group.label());

        ctx.canvas.text(
            &header_title,
            Rect::new(
                header_rect.origin.x + 8.0,
                header_rect.origin.y,
                header_rect.size.x - 16.0,
                header_h,
            ),
            11.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );

        // 无障碍：全量注册折叠标题按钮与展开状态位
        let a11y_label = format!("Group: {} — {}", group.label(), group.description());
        ctx.control_expandable(
            part,
            header_rect,
            Role::Button,
            &a11y_label,
            false,
            is_expanded,
        );

        // 若展开，绘制内容容器
        if is_expanded {
            let content_rect = Rect::new(
                rect.origin.x + 8.0,
                current_y + header_h,
                rect.size.x - 16.0,
                content_h - 4.0,
            );
            if *group == TextConsoleGroup::Font {
                draw_font_controls(ctx, content_rect, state);
            } else if *group == TextConsoleGroup::Typography {
                draw_typography_group(ctx, content_rect, state);
            } else if *group == TextConsoleGroup::Pixel {
                ctx.canvas.rect(
                    content_rect,
                    3.0,
                    palette.card_border,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                draw_pixel_group_content(ctx, content_rect, state);
            } else if *group == TextConsoleGroup::DistanceField {
                ctx.canvas.rect(
                    content_rect,
                    3.0,
                    palette.card_border,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                draw_distance_field_controls(ctx, content_rect, state);
            } else if *group == TextConsoleGroup::Composition {
                ctx.canvas.rect(
                    content_rect,
                    3.0,
                    palette.card_border,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                draw_composition_controls(ctx, content_rect, state);
            } else if *group == TextConsoleGroup::Emoji {
                ctx.canvas.rect(
                    content_rect,
                    3.0,
                    palette.card_border,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                draw_emoji_controls(ctx, content_rect, state);
            } else {
                ctx.canvas.rect(
                    content_rect,
                    3.0,
                    palette.card_border,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                ctx.canvas.text(
                    group.description(),
                    Rect::new(
                        content_rect.origin.x + 6.0,
                        content_rect.origin.y,
                        content_rect.size.x - 12.0,
                        content_rect.size.y,
                    ),
                    9.5,
                    palette.text_muted,
                    AlignX::Start,
                    AlignY::Center,
                );
            }
        }

        current_y += total_group_h + item_gap;
    }
}

/// 绘制顶部预设方案切换栏（WinUI 原生 / 跨平台一致 / 像素风 / 自定义）。
fn draw_preset_bar(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    ctx.canvas
        .rect(rect, 4.0, palette.surface, palette.card_border, 0.5, 0.0);

    let padding = 4.0;
    let label_w = 48.0;
    let label_rect = Rect::new(rect.origin.x + padding, rect.origin.y, label_w, rect.size.y);
    ctx.canvas.text(
        "Preset:",
        label_rect,
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let presets_x = rect.origin.x + padding + label_w;
    let presets_w = (rect.size.x - padding * 2.0 - label_w).max(20.0);
    let gap = 4.0;
    let items = [
        ("WinUI", PART_PRESET_WINUI, TextRenderPreset::WinuiNative),
        (
            "Consistent",
            PART_PRESET_CROSS_PLATFORM,
            TextRenderPreset::CrossPlatform,
        ),
        ("Pixel", PART_PRESET_PIXEL, TextRenderPreset::Pixel),
        ("Custom", PART_PRESET_CUSTOM, TextRenderPreset::Custom),
    ];
    let btn_w = (presets_w - gap * (items.len() as f32 - 1.0)) / items.len() as f32;

    for (idx, (label, part, preset)) in items.iter().enumerate() {
        let btn_rect = Rect::new(
            presets_x + idx as f32 * (btn_w + gap),
            rect.origin.y + 3.0,
            btn_w,
            rect.size.y - 6.0,
        );

        let selected = state.preset == *preset;
        let a11y_label = format!("Preset: {} (Selected: {})", label, selected);

        ctx.control_state(
            *part,
            btn_rect,
            Role::RadioButton,
            &a11y_label,
            false,
            selected,
        );

        let (bg, border, text_color) = if selected {
            (palette.primary, palette.primary, palette.card_bg)
        } else {
            (palette.card_bg, palette.card_border, palette.text)
        };

        ctx.canvas.rect(btn_rect, 3.0, bg, border, 0.5, 0.0);
        ctx.canvas.text(
            label,
            btn_rect,
            10.0,
            text_color,
            AlignX::Center,
            AlignY::Center,
        );
    }
}

/// 绘制字体组四项调节控件（家族、字重、字号、斜体）。
fn draw_font_controls(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    ctx.canvas
        .rect(rect, 4.0, palette.card_bg, palette.card_border, 0.5, 0.0);

    let caps = TextCapabilities::probe();
    let family_res = resolve_feature(TextFeatureKey::FontFamily, &caps, &state.settings);
    let weight_res = resolve_feature(TextFeatureKey::FontWeight, &caps, &state.settings);
    let size_res = resolve_feature(TextFeatureKey::FontSize, &caps, &state.settings);
    let italic_res = resolve_feature(TextFeatureKey::FontItalic, &caps, &state.settings);

    let pad_x = 8.0;
    let pad_y = 4.0;
    let inner_w = rect.size.x - pad_x * 2.0;
    let col_gap = 8.0;
    let col_w = ((inner_w - col_gap) * 0.5).max(40.0);

    // Row 1: Family (left) & Weight (right)
    let row1_y = rect.origin.y + pad_y;
    ctx.canvas.text(
        "FAMILY",
        Rect::new(rect.origin.x + pad_x, row1_y, col_w, 10.0),
        8.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.text(
        "WEIGHT",
        Rect::new(rect.origin.x + pad_x + col_w + col_gap, row1_y, col_w, 10.0),
        8.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let family_rect = Rect::new(rect.origin.x + pad_x, row1_y + 11.0, col_w, 22.0);
    let family_ids: Vec<u64> = (0..FONT_FAMILY_OPTIONS.len())
        .map(|i| ctx.part(FONT_FAMILY_PART_BASE + i as u16))
        .collect();
    let family_labels: Vec<&str> = FONT_FAMILY_OPTIONS.iter().map(|(lbl, _)| *lbl).collect();
    let selected_family = FONT_FAMILY_OPTIONS
        .iter()
        .position(|(_, opt)| *opt == state.settings.family.as_deref())
        .unwrap_or(0);
    crate::widgets::segmented(
        ctx.canvas,
        &family_ids,
        family_rect,
        &family_labels,
        selected_family,
        family_res.disabled,
        None,
    );

    let weight_rect = Rect::new(
        rect.origin.x + pad_x + col_w + col_gap,
        row1_y + 11.0,
        col_w,
        22.0,
    );
    let weight_ids: Vec<u64> = (0..FONT_WEIGHT_OPTIONS.len())
        .map(|i| ctx.part(FONT_WEIGHT_PART_BASE + i as u16))
        .collect();
    let weight_labels: Vec<&str> = FONT_WEIGHT_OPTIONS.iter().map(|(lbl, _)| *lbl).collect();
    let selected_weight = FONT_WEIGHT_OPTIONS
        .iter()
        .position(|(_, w)| *w == state.settings.weight)
        .unwrap_or(0);
    crate::widgets::segmented(
        ctx.canvas,
        &weight_ids,
        weight_rect,
        &weight_labels,
        selected_weight,
        weight_res.disabled,
        None,
    );

    // Row 2: Size (left) & Italic (right)
    let row2_y = row1_y + 37.0;
    ctx.canvas.text(
        "SIZE",
        Rect::new(rect.origin.x + pad_x, row2_y, col_w, 10.0),
        8.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let size_rect = Rect::new(rect.origin.x + pad_x, row2_y + 11.0, col_w, 22.0);
    let size_ids: Vec<u64> = (0..FONT_SIZE_OPTIONS.len())
        .map(|i| ctx.part(FONT_SIZE_PART_BASE + i as u16))
        .collect();
    let size_labels: Vec<&str> = FONT_SIZE_OPTIONS.iter().map(|(lbl, _)| *lbl).collect();
    let selected_size = FONT_SIZE_OPTIONS
        .iter()
        .position(|(_, s)| (*s - state.settings.size).abs() < 0.1)
        .unwrap_or(1);
    crate::widgets::segmented(
        ctx.canvas,
        &size_ids,
        size_rect,
        &size_labels,
        selected_size,
        size_res.disabled,
        None,
    );

    let italic_rect = Rect::new(
        rect.origin.x + pad_x + col_w + col_gap,
        row2_y + 11.0,
        col_w,
        22.0,
    );
    let italic_node = ctx.part(FONT_ITALIC_PART);
    crate::widgets::toggle_switch(
        ctx.canvas,
        italic_node,
        italic_rect,
        "Italic Slant",
        state.settings.italic,
        italic_res.disabled,
    );
}

/// 绘制像素与相位组的内容：光栅器后端（DirectWrite / Cosmic）与亚像素相位（8 / 4 / 1）。
fn draw_pixel_group_content(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    let padding = 6.0;
    let caps = TextCapabilities::probe();

    // 1. 光栅化后端切换（DirectWrite / Cosmic）
    let btn_gap = 6.0;
    let btn_w = (rect.size.x - padding * 2.0 - btn_gap) * 0.5;
    let btn_h = 22.0;
    let btn_y = rect.origin.y + 4.0;

    let dw_rect = Rect::new(rect.origin.x + padding, btn_y, btn_w, btn_h);
    let cosmic_rect = Rect::new(
        rect.origin.x + padding + btn_w + btn_gap,
        btn_y,
        btn_w,
        btn_h,
    );

    let dw_res = resolve_feature(TextFeatureKey::BackendDirectWrite, &caps, &state.settings);
    let cosmic_res = resolve_feature(TextFeatureKey::BackendCosmic, &caps, &state.settings);
    let dw_selected = state.settings.raster_backend == RasterBackend::DirectWrite;
    let cosmic_selected = state.settings.raster_backend == RasterBackend::Cosmic;

    // DirectWrite 按钮绘制
    let (dw_bg, dw_border, dw_text_color) = if dw_res.disabled {
        (
            palette.disabled_fill,
            palette.card_border,
            palette.text_muted,
        )
    } else if dw_selected {
        (palette.primary, palette.primary, palette.on_primary)
    } else {
        (palette.card_bg, palette.card_border, palette.text)
    };
    ctx.canvas.rect(dw_rect, 3.0, dw_bg, dw_border, 1.0, 0.0);
    let dw_btn_title = if dw_selected {
        "[•] DirectWrite"
    } else {
        "DirectWrite"
    };
    ctx.canvas.text(
        dw_btn_title,
        dw_rect,
        10.5,
        dw_text_color,
        AlignX::Center,
        AlignY::Center,
    );
    let a11y_dw = format!(
        "Backend: DirectWrite{}{}",
        if dw_selected { " (Selected)" } else { "" },
        dw_res.reason.map_or(String::new(), |r| format!(" - {}", r))
    );
    ctx.control(
        PART_BACKEND_DW,
        dw_rect,
        Role::RadioButton,
        &a11y_dw,
        dw_selected,
    );

    // Cosmic 按钮绘制
    let (cosmic_bg, cosmic_border, cosmic_text_color) = if cosmic_res.disabled {
        (
            palette.disabled_fill,
            palette.card_border,
            palette.text_muted,
        )
    } else if cosmic_selected {
        (palette.primary, palette.primary, palette.on_primary)
    } else {
        (palette.card_bg, palette.card_border, palette.text)
    };
    ctx.canvas
        .rect(cosmic_rect, 3.0, cosmic_bg, cosmic_border, 1.0, 0.0);
    let cosmic_btn_title = if cosmic_selected {
        "[•] Cosmic"
    } else {
        "Cosmic"
    };
    ctx.canvas.text(
        cosmic_btn_title,
        cosmic_rect,
        10.5,
        cosmic_text_color,
        AlignX::Center,
        AlignY::Center,
    );
    let a11y_cosmic = format!(
        "Backend: Cosmic Swash{}{}",
        if cosmic_selected { " (Selected)" } else { "" },
        cosmic_res
            .reason
            .map_or(String::new(), |r| format!(" - {}", r))
    );
    ctx.control(
        PART_BACKEND_COSMIC,
        cosmic_rect,
        Role::RadioButton,
        &a11y_cosmic,
        cosmic_selected,
    );

    // 2. 亚像素相位选项（8 / 4 / 1）
    let phase_btn_top = btn_y + btn_h + 4.0;
    let phase_btn_h = 22.0;
    let phase_gap = 4.0;
    let phase_item_w = (rect.size.x - padding * 2.0 - phase_gap * 2.0) / 3.0;

    for (i, opt) in PHASE_OPTIONS.iter().enumerate() {
        let part = PHASE_PART_BASE + i as u16;
        let opt_rect = Rect::new(
            rect.origin.x + padding + i as f32 * (phase_item_w + phase_gap),
            phase_btn_top,
            phase_item_w,
            phase_btn_h,
        );

        let res = resolve_feature(opt.key, &caps, &state.settings);
        let selected = state.settings.phase_mode == opt.mode;
        let disabled = res.disabled;

        let (bg, border, text_color) = if disabled {
            (
                palette.disabled_fill,
                palette.card_border,
                palette.text_muted,
            )
        } else if selected {
            (palette.primary, palette.primary, palette.on_primary)
        } else {
            (palette.card_bg, palette.card_border, palette.text)
        };

        ctx.canvas.rect(opt_rect, 3.0, bg, border, 1.0, 0.0);

        ctx.canvas.text(
            opt.label,
            opt_rect,
            10.0,
            text_color,
            AlignX::Center,
            AlignY::Center,
        );

        let mut a11y_label = format!("Phase Mode: {} — {}", opt.label, opt.description);
        if let Some(reason) = res.reason {
            a11y_label.push_str(&format!(" (Disabled: {})", reason));
        }

        ctx.control(part, opt_rect, Role::RadioButton, &a11y_label, selected);
    }
}

/// 绘制单个选项按钮并注册无障碍控件。
fn draw_option_button(
    ctx: &mut DemoCtx<'_, '_>,
    part: u16,
    rect: Rect,
    label: &str,
    selected: bool,
    disabled: bool,
    reason: Option<&str>,
) {
    let palette = *ctx.canvas.palette;
    let (bg, border, text_color) = if disabled {
        (palette.card_bg, palette.card_border, palette.text_muted)
    } else if selected {
        (palette.primary, palette.primary, palette.card_bg)
    } else {
        (palette.surface, palette.card_border, palette.text)
    };

    ctx.canvas.rect(rect, 3.0, bg, border, 1.0, 0.0);
    ctx.canvas.text(
        label,
        Rect::new(
            rect.origin.x + 2.0,
            rect.origin.y,
            rect.size.x - 4.0,
            rect.size.y,
        ),
        10.5,
        text_color,
        AlignX::Center,
        AlignY::Center,
    );

    let accessible_label = if let Some(r) = reason {
        format!("{label} (Disabled: {r})")
    } else {
        label.to_string()
    };
    ctx.control(part, rect, Role::Button, &accessible_label, selected);
}

/// 绘制“距离场”组控件。
fn draw_distance_field_controls(
    ctx: &mut DemoCtx<'_, '_>,
    rect: Rect,
    state: &TextRenderPageState,
) {
    let palette = *ctx.canvas.palette;
    let btn_gap = 6.0;
    let btn_w = ((rect.size.x - 16.0 - btn_gap * 2.0) / 3.0).max(40.0);
    let btn_h = 24.0;
    let btn_y = rect.origin.y + 6.0;

    let is_sdf = state.settings.use_sdf || state.settings.distance_field == DistanceFieldMode::Sdf;
    let is_off = !is_sdf && state.settings.distance_field == DistanceFieldMode::Off;

    // Off 按钮
    let off_rect = Rect::new(rect.origin.x + 8.0, btn_y, btn_w, btn_h);
    draw_option_button(ctx, PART_DF_OFF, off_rect, "Off", is_off, false, None);

    // SDF 按钮
    let sdf_rect = Rect::new(rect.origin.x + 8.0 + btn_w + btn_gap, btn_y, btn_w, btn_h);
    draw_option_button(ctx, PART_DF_SDF, sdf_rect, "SDF", is_sdf, false, None);

    // MTSDF 按钮（管线未接线，灰化）
    let mtsdf_rect = Rect::new(
        rect.origin.x + 8.0 + (btn_w + btn_gap) * 2.0,
        btn_y,
        btn_w,
        btn_h,
    );
    draw_option_button(
        ctx,
        PART_DF_MTSDF,
        mtsdf_rect,
        "MTSDF",
        false,
        true,
        Some("Pipeline not wired up"),
    );

    // 互斥说明或提示文案
    let note = if is_sdf {
        "Phase locked to 1 (Snapped)"
    } else {
        "Signed distance field generation"
    };
    ctx.canvas.text(
        note,
        Rect::new(
            rect.origin.x + 8.0,
            btn_y + btn_h + 3.0,
            rect.size.x - 16.0,
            16.0,
        ),
        9.0,
        if is_sdf {
            palette.primary
        } else {
            palette.text_muted
        },
        AlignX::Start,
        AlignY::Center,
    );
}

/// 绘制“合成”组控件。
fn draw_composition_controls(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    let row_h = 24.0;
    let label_w = 46.0;
    let btn_gap = 6.0;

    // 第一行：混合空间 (Blend Space)
    let row1_y = rect.origin.y + 6.0;
    ctx.canvas.text(
        "Space:",
        Rect::new(rect.origin.x + 8.0, row1_y, label_w, row_h),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    let space_btn_w = ((rect.size.x - 16.0 - label_w - btn_gap) / 2.0).max(40.0);

    let srgb_rect = Rect::new(rect.origin.x + 8.0 + label_w, row1_y, space_btn_w, row_h);
    let is_srgb = state.settings.blend_space == BlendSpace::Srgb;
    draw_option_button(ctx, PART_COMP_SRGB, srgb_rect, "sRGB", is_srgb, false, None);

    let linear_rect = Rect::new(
        rect.origin.x + 8.0 + label_w + space_btn_w + btn_gap,
        row1_y,
        space_btn_w,
        row_h,
    );
    let is_linear = state.settings.blend_space == BlendSpace::Linear;
    draw_option_button(
        ctx,
        PART_COMP_LINEAR,
        linear_rect,
        "Linear",
        is_linear,
        false,
        None,
    );

    // 第二行：图层模式 (Layer Mode)
    let row2_y = row1_y + row_h + 6.0;
    ctx.canvas.text(
        "Layer:",
        Rect::new(rect.origin.x + 8.0, row2_y, label_w, row_h),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let direct_rect = Rect::new(rect.origin.x + 8.0 + label_w, row2_y, space_btn_w, row_h);
    let is_direct = state.settings.layer_mode == LayerMode::Direct;
    draw_option_button(
        ctx,
        PART_COMP_LAYER_DIRECT,
        direct_rect,
        "Direct",
        is_direct,
        false,
        None,
    );

    let isolated_rect = Rect::new(
        rect.origin.x + 8.0 + label_w + space_btn_w + btn_gap,
        row2_y,
        space_btn_w,
        row_h,
    );
    let is_isolated = state.settings.layer_mode == LayerMode::Isolated;
    draw_option_button(
        ctx,
        PART_COMP_LAYER_ISOLATED,
        isolated_rect,
        "Isolated",
        is_isolated,
        false,
        None,
    );
}

/// 选项单选框条目描述。
struct OptionItem {
    label: &'static str,
    part: u16,
    feature_key: Option<TextFeatureKey>,
    selected: bool,
}

/// 绘制排版语义组六项可切换与灰化控件。
fn draw_typography_group(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    let caps = TextCapabilities::probe();

    ctx.canvas
        .rect(rect, 4.0, palette.surface, palette.card_border, 0.5, 0.0);

    let padding = 4.0;
    let row_h = 24.0;
    let row_gap = 4.0;
    let title_w = 84.0;
    let mut current_y = rect.origin.y + padding;

    let rows: [(&'static str, &[OptionItem]); 6] = [
        // 1. 塑形器
        (
            "Shaper",
            &[
                OptionItem {
                    label: "Cosmic",
                    part: PART_SHAPER_COSMIC,
                    feature_key: Some(TextFeatureKey::ShaperCosmic),
                    selected: state.settings.shaper == TextShaper::Cosmic,
                },
                OptionItem {
                    label: "Native",
                    part: PART_SHAPER_NATIVE,
                    feature_key: Some(TextFeatureKey::ShaperNative),
                    selected: false,
                },
            ],
        ),
        // 2. 断行
        (
            "Line Break",
            &[
                OptionItem {
                    label: "Word",
                    part: PART_LINEBREAK_WORD,
                    feature_key: None,
                    selected: state.settings.line_break == LineBreakMode::Word,
                },
                OptionItem {
                    label: "Anywhere",
                    part: PART_LINEBREAK_ANYWHERE,
                    feature_key: None,
                    selected: state.settings.line_break == LineBreakMode::Anywhere,
                },
                OptionItem {
                    label: "None",
                    part: PART_LINEBREAK_NONE,
                    feature_key: None,
                    selected: state.settings.line_break == LineBreakMode::None,
                },
            ],
        ),
        // 3. 双向文本
        (
            "BiDi",
            &[
                OptionItem {
                    label: "On",
                    part: PART_BIDI_ON,
                    feature_key: None,
                    selected: state.settings.bidi,
                },
                OptionItem {
                    label: "Off",
                    part: PART_BIDI_OFF,
                    feature_key: None,
                    selected: !state.settings.bidi,
                },
            ],
        ),
        // 4. 连字
        (
            "Ligatures",
            &[
                OptionItem {
                    label: "On",
                    part: PART_LIGATURES_ON,
                    feature_key: None,
                    selected: state.settings.ligatures,
                },
                OptionItem {
                    label: "Off",
                    part: PART_LIGATURES_OFF,
                    feature_key: None,
                    selected: !state.settings.ligatures,
                },
            ],
        ),
        // 5. 字距
        (
            "Spacing",
            &[
                OptionItem {
                    label: "0 px",
                    part: PART_LETTER_SPACING_0,
                    feature_key: None,
                    selected: state.settings.letter_spacing == 0.0,
                },
                OptionItem {
                    label: "1 px",
                    part: PART_LETTER_SPACING_1,
                    feature_key: None,
                    selected: (state.settings.letter_spacing - 1.0).abs() < 0.01,
                },
                OptionItem {
                    label: "2 px",
                    part: PART_LETTER_SPACING_2,
                    feature_key: None,
                    selected: (state.settings.letter_spacing - 2.0).abs() < 0.01,
                },
            ],
        ),
        // 6. 行高
        (
            "Line Height",
            &[
                OptionItem {
                    label: "Auto",
                    part: PART_LINE_HEIGHT_AUTO,
                    feature_key: None,
                    selected: state.settings.line_height == 0.0,
                },
                OptionItem {
                    label: "20 px",
                    part: PART_LINE_HEIGHT_20,
                    feature_key: None,
                    selected: (state.settings.line_height - 20.0).abs() < 0.01,
                },
                OptionItem {
                    label: "28 px",
                    part: PART_LINE_HEIGHT_28,
                    feature_key: None,
                    selected: (state.settings.line_height - 28.0).abs() < 0.01,
                },
            ],
        ),
    ];

    for (row_title, options) in rows {
        if current_y + row_h > rect.max().y {
            break;
        }

        let title_rect = Rect::new(rect.origin.x + padding, current_y, title_w, row_h);
        ctx.canvas.text(
            row_title,
            title_rect,
            10.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );

        let options_x = rect.origin.x + padding + title_w;
        let options_w = (rect.size.x - padding * 2.0 - title_w).max(20.0);
        let btn_gap = 4.0;
        let btn_w = (options_w - btn_gap * (options.len() as f32 - 1.0)) / options.len() as f32;

        for (opt_idx, opt) in options.iter().enumerate() {
            let btn_x = options_x + opt_idx as f32 * (btn_w + btn_gap);
            let btn_rect = Rect::new(btn_x, current_y, btn_w, row_h);

            // 表驱动灰化决议
            let res = opt.feature_key.map_or(FeatureResolution::AVAILABLE, |key| {
                resolve_feature(key, &caps, &state.settings)
            });
            let hint = opt
                .feature_key
                .and_then(|key| feature_hint(key, &caps, &state.settings));

            let a11y_label = if let Some(reason) = res.reason {
                format!("{} — Disabled: {}", opt.label, reason)
            } else if let Some(hint) = hint {
                format!("{}: {} — {}", row_title, opt.label, hint)
            } else {
                format!("{}: {}", row_title, opt.label)
            };

            ctx.control_state(
                opt.part,
                btn_rect,
                Role::Button,
                &a11y_label,
                res.disabled,
                opt.selected,
            );

            // 视觉呈现
            if res.disabled {
                ctx.canvas.rect(
                    btn_rect,
                    3.0,
                    palette.surface,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                ctx.canvas.text(
                    opt.label,
                    btn_rect,
                    10.0,
                    palette.text_muted,
                    AlignX::Center,
                    AlignY::Center,
                );
            } else if opt.selected {
                ctx.canvas
                    .rect(btn_rect, 3.0, palette.primary, palette.primary, 1.0, 0.0);
                ctx.canvas.text(
                    opt.label,
                    btn_rect,
                    10.0,
                    palette.card_bg,
                    AlignX::Center,
                    AlignY::Center,
                );
            } else {
                ctx.canvas.rect(
                    btn_rect,
                    3.0,
                    palette.card_bg,
                    palette.card_border,
                    0.5,
                    0.0,
                );
                ctx.canvas.text(
                    opt.label,
                    btn_rect,
                    10.0,
                    palette.text,
                    AlignX::Center,
                    AlignY::Center,
                );
            }
        }

        current_y += row_h + row_gap;
    }
}

/// 绘制右侧文本实时对照预览区。
fn draw_preview_panel(ctx: &mut DemoCtx<'_, '_>, rect: Rect, settings: &TextRenderSettings) {
    let palette = *ctx.canvas.palette;
    ctx.canvas
        .rect(rect, 8.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    let padding = 12.0;
    let inner_w = (rect.size.x - padding * 2.0).max(40.0);
    let sample_gap = 10.0;
    let header_h = 24.0;

    // 标题与当前预览样式数值摘要
    let family_display = settings.family.as_deref().unwrap_or("Default");
    let summary_text = format!(
        "Family: {}  Weight: {}  Size: {:.1}px  Italic: {}",
        family_display,
        settings.weight,
        settings.size,
        if settings.italic { "Yes" } else { "No" }
    );

    ctx.canvas.text(
        "LIVE PREVIEW SPECIMENS",
        Rect::new(
            rect.origin.x + padding,
            rect.origin.y + padding,
            160.0,
            header_h,
        ),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    ctx.canvas.text(
        &summary_text,
        Rect::new(
            rect.origin.x + padding + 160.0,
            rect.origin.y + padding,
            (inner_w - 160.0).max(20.0),
            header_h,
        ),
        10.0,
        palette.primary,
        AlignX::End,
        AlignY::Center,
    );

    let preview_style = unbound_text::TextStyle {
        font_size: settings.size,
        weight: settings.weight,
        italic: settings.italic,
        family: settings.family.clone(),
        line_height: 0.0,
        letter_spacing: 0.0,
    };

    let start_y = rect.origin.y + padding + header_h + 4.0;

    // 样本 1：中英混排样例
    let sample1_h = 72.0;
    let sample1_rect = Rect::new(rect.origin.x + padding, start_y, inner_w, sample1_h);
    ctx.canvas.rect(
        sample1_rect,
        4.0,
        palette.surface,
        palette.card_border,
        0.5,
        0.0,
    );
    ctx.canvas.text_styled(
        "Unbound UI 文本渲染引擎 DirectWrite & Swash 灰度抗锯齿 The quick brown fox jumps over the lazy dog 1234567890.",
        Rect::new(
            sample1_rect.origin.x + 8.0,
            sample1_rect.origin.y + 6.0,
            sample1_rect.size.x - 16.0,
            sample1_rect.size.y - 12.0,
        ),
        &preview_style,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    // 样本 2：纯汉字一行
    let sample2_y = start_y + sample1_h + sample_gap;
    let sample2_h = 48.0;
    let sample2_rect = Rect::new(rect.origin.x + padding, sample2_y, inner_w, sample2_h);
    ctx.canvas.rect(
        sample2_rect,
        4.0,
        palette.surface,
        palette.card_border,
        0.5,
        0.0,
    );
    ctx.canvas.text_styled(
        "天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏",
        Rect::new(
            sample2_rect.origin.x + 8.0,
            sample2_rect.origin.y + 4.0,
            sample2_rect.size.x - 16.0,
            sample2_rect.size.y - 8.0,
        ),
        &preview_style,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    // 样本 3：Emoji 符号一行
    let sample3_y = sample2_y + sample2_h + sample_gap;
    let sample3_h = 48.0;
    let sample3_rect = Rect::new(rect.origin.x + padding, sample3_y, inner_w, sample3_h);
    ctx.canvas.rect(
        sample3_rect,
        4.0,
        palette.surface,
        palette.card_border,
        0.5,
        0.0,
    );
    ctx.canvas.text_styled(
        "😀 😃 😄 😁 🎨 🚀 🦀 ⚡ 🌈 💻 📦 ⚙️ 🔍 🔬 💡",
        Rect::new(
            sample3_rect.origin.x + 8.0,
            sample3_rect.origin.y + 4.0,
            sample3_rect.size.x - 16.0,
            sample3_rect.size.y - 8.0,
        ),
        &preview_style,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
}

/// 绘制底部横跨整宽的状态栏（读取实际生效报告）。
fn draw_status_bar(
    ctx: &mut DemoCtx<'_, '_>,
    rect: Rect,
    report: &TextRenderReport,
    fallback_expanded: bool,
) {
    let palette = *ctx.canvas.palette;
    ctx.canvas
        .rect(rect, 6.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    let backend_text = match report.backend {
        RasterBackend::DirectWrite => "DirectWrite (Coverage)",
        RasterBackend::Cosmic => "Cosmic (Swash)",
    };
    let phases_text = match report.phases {
        8 => "8 Subpixel Phases",
        4 => "4 Subpixel Phases",
        1 => "1 Phase (Snapped)",
        _ => "Phases",
    };
    let fallback_text = if report.fallback_count == 0 {
        "0 fallbacks".to_string()
    } else if let Some(ref reason) = report.last_fallback_reason {
        format!("{} fallbacks (Recent: {})", report.fallback_count, reason)
    } else {
        format!("{} fallbacks", report.fallback_count)
    };

    let items = [
        ("EFFECTIVE BACKEND", backend_text),
        ("EFFECTIVE PHASES", phases_text),
        ("EMOJI PATH", &report.emoji_path),
        ("FALLBACKS", &fallback_text),
    ];

    let col_w = (rect.size.x - 16.0) / 4.0;
    for (i, (title, value)) in items.iter().enumerate() {
        let x = rect.origin.x + 8.0 + i as f32 * col_w;
        let cell = Rect::new(x, rect.origin.y + 4.0, col_w - 8.0, rect.size.y - 8.0);

        ctx.canvas.text(
            title,
            Rect::new(cell.origin.x, cell.origin.y, cell.size.x, 12.0),
            8.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            value,
            Rect::new(
                cell.origin.x,
                cell.origin.y + 13.0,
                cell.size.x,
                cell.size.y - 13.0,
            ),
            11.5,
            palette.primary,
            AlignX::Start,
            AlignY::Center,
        );
    }

    // 状态栏语义与键盘可达（Button Role / 可聚焦），点击展开详细回退诊断。
    // 已知差异一旦存在就必须进摘要：面板不能把设置说成完全按字面生效。
    let deviation_suffix = report
        .known_deviation
        .as_ref()
        .map_or(String::new(), |dev| format!(". Known deviation: {dev}"));
    let a11y_summary = format!(
        "Effective Report: Backend: {}, Phases: {}, Emoji: {}, Fallbacks: {}{}",
        backend_text, phases_text, report.emoji_path, fallback_text, deviation_suffix
    );
    ctx.control_expandable(
        STATUS_BAR_PART,
        rect,
        Role::Status,
        &a11y_summary,
        false,
        fallback_expanded,
    );

    // 若展开回退详情，在状态栏上方绘制诊断横幅
    if fallback_expanded {
        let details_h = 24.0;
        let details_rect = Rect::new(
            rect.origin.x,
            rect.origin.y - details_h - 4.0,
            rect.size.x,
            details_h,
        );
        ctx.canvas.rect(
            details_rect,
            4.0,
            palette.card_bg,
            palette.primary,
            1.0,
            0.0,
        );
        let detail_text = match (
            report.last_fallback_reason.as_ref(),
            report.known_deviation.as_ref(),
        ) {
            (Some(reason), Some(dev)) => {
                format!("Fallback: {reason}  |  Known deviation: {dev}")
            }
            (Some(reason), None) => format!("Recent Fallback Diagnostic: {reason}"),
            (None, Some(dev)) => format!("Known deviation: {dev}"),
            (None, None) => "No fallback recorded in current session".to_string(),
        };
        ctx.canvas.text(
            &detail_text,
            Rect::new(
                details_rect.origin.x + 8.0,
                details_rect.origin.y,
                details_rect.size.x - 16.0,
                details_h,
            ),
            10.0,
            palette.primary,
            AlignX::Start,
            AlignY::Center,
        );
    }
}

/// 绘制 Emoji 组五维度控件（呈现、光栅器、来源等）。
fn draw_emoji_controls(ctx: &mut DemoCtx<'_, '_>, rect: Rect, state: &TextRenderPageState) {
    let palette = *ctx.canvas.palette;
    let caps = TextCapabilities::probe();

    let padding = 6.0;
    let row_h = 20.0;
    let row_gap = 4.0;
    let title_w = 76.0;

    let rows: [(&str, &[OptionItem]); 3] = [
        (
            "Presentation",
            &[
                OptionItem {
                    label: "Color",
                    part: PART_EMOJI_PRES_COLOR,
                    feature_key: Some(TextFeatureKey::EmojiPresentationColor),
                    selected: state.settings.emoji_presentation
                        == unbound_text::EmojiPresentation::Color,
                },
                OptionItem {
                    label: "Mono",
                    part: PART_EMOJI_PRES_MONO,
                    feature_key: Some(TextFeatureKey::EmojiPresentationMono),
                    selected: state.settings.emoji_presentation
                        == unbound_text::EmojiPresentation::Mono,
                },
            ],
        ),
        (
            "Rasterizer",
            &[
                OptionItem {
                    label: "Follow",
                    part: PART_EMOJI_BACKEND_FOLLOW,
                    feature_key: Some(TextFeatureKey::EmojiBackendFollow),
                    selected: state.settings.emoji_backend == unbound_text::EmojiBackend::Follow,
                },
                OptionItem {
                    label: "DW",
                    part: PART_EMOJI_BACKEND_DW,
                    feature_key: Some(TextFeatureKey::EmojiBackendDirectWrite),
                    selected: state.settings.emoji_backend
                        == unbound_text::EmojiBackend::DirectWrite,
                },
                OptionItem {
                    label: "Cosmic",
                    part: PART_EMOJI_BACKEND_COSMIC,
                    feature_key: Some(TextFeatureKey::EmojiBackendCosmic),
                    selected: state.settings.emoji_backend == unbound_text::EmojiBackend::Cosmic,
                },
            ],
        ),
        (
            "Source",
            &[
                OptionItem {
                    label: "System",
                    part: PART_EMOJI_SOURCE_SYSTEM,
                    feature_key: Some(TextFeatureKey::EmojiSourceSystem),
                    selected: state.settings.emoji_source == unbound_text::EmojiSource::System,
                },
                OptionItem {
                    label: "Builtin",
                    part: PART_EMOJI_SOURCE_BUILTIN,
                    feature_key: Some(TextFeatureKey::EmojiSourceBuiltin),
                    selected: state.settings.emoji_source == unbound_text::EmojiSource::Builtin,
                },
                OptionItem {
                    label: "Mesh",
                    part: PART_EMOJI_SOURCE_GRID,
                    feature_key: Some(TextFeatureKey::EmojiSourceGrid),
                    selected: state.settings.emoji_source == unbound_text::EmojiSource::Grid,
                },
            ],
        ),
    ];

    let mut current_y = rect.origin.y + padding;
    for (row_title, options) in rows {
        if current_y + row_h > rect.max().y {
            break;
        }

        let title_rect = Rect::new(rect.origin.x + padding, current_y, title_w, row_h);
        ctx.canvas.text(
            row_title,
            title_rect,
            10.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );

        let options_x = rect.origin.x + padding + title_w;
        let options_w = (rect.size.x - padding * 2.0 - title_w).max(20.0);
        let btn_gap = 4.0;
        let btn_w = (options_w - btn_gap * (options.len() as f32 - 1.0)) / options.len() as f32;

        for (opt_idx, opt) in options.iter().enumerate() {
            let btn_x = options_x + opt_idx as f32 * (btn_w + btn_gap);
            let btn_rect = Rect::new(btn_x, current_y, btn_w, row_h);

            // 表驱动灰化决议
            let res = opt.feature_key.map_or(FeatureResolution::AVAILABLE, |key| {
                resolve_feature(key, &caps, &state.settings)
            });
            let hint = opt
                .feature_key
                .and_then(|key| feature_hint(key, &caps, &state.settings));

            let a11y_label = if let Some(reason) = res.reason {
                format!("{} — Disabled: {}", opt.label, reason)
            } else if let Some(hint) = hint {
                format!("{}: {} — {}", row_title, opt.label, hint)
            } else {
                format!("{}: {}", row_title, opt.label)
            };

            ctx.control_state(
                opt.part,
                btn_rect,
                Role::Button,
                &a11y_label,
                res.disabled,
                opt.selected,
            );

            // 绘制按钮外观
            let (bg, border, text_color) = if res.disabled {
                (palette.surface, palette.card_border, palette.text_muted)
            } else if opt.selected {
                (palette.primary, palette.primary, palette.card_bg)
            } else {
                (palette.card_bg, palette.card_border, palette.text)
            };

            ctx.canvas.rect(btn_rect, 3.0, bg, border, 0.5, 0.0);
            ctx.canvas.text(
                opt.label,
                btn_rect,
                10.0,
                text_color,
                AlignX::Center,
                AlignY::Center,
            );
        }

        current_y += row_h + row_gap;
    }
}

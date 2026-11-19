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
    CapabilityStatus, LineBreakMode, RasterBackend, TextCapabilities, TextRenderReport,
    TextRenderSettings, TextShaper,
};

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

/// 六组折叠框架的部件起始编号。
pub const GROUP_PART_BASE: u16 = 10;
/// 状态栏部件编号。
pub const STATUS_BAR_PART: u16 = 20;

// === 排版组各控件部件编号 ===
pub const PART_SHAPER_COSMIC: u16 = 101;
pub const PART_SHAPER_NATIVE: u16 = 102;

pub const PART_LINEBREAK_WORD: u16 = 111;
pub const PART_LINEBREAK_ANYWHERE: u16 = 112;
pub const PART_LINEBREAK_NONE: u16 = 113;

pub const PART_BIDI_ON: u16 = 121;
pub const PART_BIDI_OFF: u16 = 122;

pub const PART_LIGATURES_ON: u16 = 131;
pub const PART_LIGATURES_OFF: u16 = 132;

pub const PART_LETTER_SPACING_0: u16 = 141;
pub const PART_LETTER_SPACING_1: u16 = 142;
pub const PART_LETTER_SPACING_2: u16 = 143;

pub const PART_LINE_HEIGHT_AUTO: u16 = 151;
pub const PART_LINE_HEIGHT_20: u16 = 152;
pub const PART_LINE_HEIGHT_28: u16 = 153;

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

/// 文本渲染控制台的持久页面状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRenderPageState {
    /// 六个分组的展开/折叠状态（true 为展开，false 为折叠）。
    pub group_expanded: [bool; 6],
    /// 状态栏回退原因详情是否展开。
    pub fallback_details_expanded: bool,
    /// 运行期文本渲染设置纯值快照。
    pub settings: TextRenderSettings,
    /// 文本渲染实际生效报告快照。
    pub report: TextRenderReport,
}

impl Default for TextRenderPageState {
    fn default() -> Self {
        Self {
            group_expanded: [true; 6],
            fallback_details_expanded: false,
            settings: TextRenderSettings::default(),
            report: TextRenderReport::default(),
        }
    }
}

impl TextRenderPageState {
    /// 切换指定分组的折叠状态。
    pub fn toggle_group(&mut self, index: usize) {
        if index < self.group_expanded.len() {
            self.group_expanded[index] = !self.group_expanded[index];
        }
    }

    /// 切换状态栏回退原因展开状态。
    pub fn toggle_fallback_details(&mut self) {
        self.fallback_details_expanded = !self.fallback_details_expanded;
    }
}

/// 表驱动能力灰化与互斥检查项。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextFeatureKey {
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
    // 塑形器
    ShaperCosmic,
    ShaperNative,
    // 合成色彩空间
    CompositionSrgb,
    CompositionLinear,
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
            if settings.use_sdf {
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
            if settings.use_sdf {
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
        check_constraint: None,
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
.with_height(520.0)];

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
    draw_preview_panel(ctx, right_rect);

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

    for (index, group) in TextConsoleGroup::ALL.iter().enumerate() {
        let is_expanded = state.group_expanded[index];
        let part = GROUP_PART_BASE + index as u16;

        let content_h = if is_expanded {
            if *group == TextConsoleGroup::Typography {
                180.0
            } else {
                28.0
            }
        } else {
            0.0
        };
        let total_group_h = header_h + content_h;
        let header_rect = Rect::new(rect.origin.x, current_y, rect.size.x, header_h);

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

        // 如果未超出左侧容器底部，绘制视觉部件
        if current_y + header_h <= rect.max().y {
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

            // 若展开，绘制内容容器
            if is_expanded && current_y + total_group_h <= rect.max().y {
                let content_rect = Rect::new(
                    rect.origin.x + 8.0,
                    current_y + header_h,
                    rect.size.x - 16.0,
                    content_h - 4.0,
                );
                if *group == TextConsoleGroup::Typography {
                    draw_typography_group(ctx, content_rect, state);
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
        }

        current_y += total_group_h + item_gap;
    }
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

    struct OptionItem {
        label: &'static str,
        part: u16,
        feature_key: Option<TextFeatureKey>,
        selected: bool,
    }

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

            let a11y_label = if let Some(reason) = res.reason {
                format!("{} — Disabled: {}", opt.label, reason)
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
fn draw_preview_panel(ctx: &mut DemoCtx<'_, '_>, rect: Rect) {
    let palette = *ctx.canvas.palette;
    ctx.canvas
        .rect(rect, 8.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    let padding = 12.0;
    let inner_w = (rect.size.x - padding * 2.0).max(40.0);
    let sample_gap = 10.0;
    let header_h = 24.0;

    // 标题
    ctx.canvas.text(
        "LIVE PREVIEW SPECIMENS",
        Rect::new(
            rect.origin.x + padding,
            rect.origin.y + padding,
            inner_w,
            header_h,
        ),
        10.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    let start_y = rect.origin.y + padding + header_h + 4.0;

    // 样本 1：中英混排样例
    let sample1_h = 64.0;
    let sample1_rect = Rect::new(rect.origin.x + padding, start_y, inner_w, sample1_h);
    ctx.canvas.rect(
        sample1_rect,
        4.0,
        palette.surface,
        palette.card_border,
        0.5,
        0.0,
    );
    ctx.canvas.text(
        "Unbound UI 文本渲染引擎 DirectWrite & Swash 灰度抗锯齿 The quick brown fox jumps over the lazy dog 1234567890.",
        Rect::new(sample1_rect.origin.x + 8.0, sample1_rect.origin.y + 6.0, sample1_rect.size.x - 16.0, sample1_rect.size.y - 12.0),
        13.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    // 样本 2：纯汉字一行
    let sample2_y = start_y + sample1_h + sample_gap;
    let sample2_h = 44.0;
    let sample2_rect = Rect::new(rect.origin.x + padding, sample2_y, inner_w, sample2_h);
    ctx.canvas.rect(
        sample2_rect,
        4.0,
        palette.surface,
        palette.card_border,
        0.5,
        0.0,
    );
    ctx.canvas.text(
        "天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏",
        Rect::new(
            sample2_rect.origin.x + 8.0,
            sample2_rect.origin.y + 4.0,
            sample2_rect.size.x - 16.0,
            sample2_rect.size.y - 8.0,
        ),
        14.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );

    // 样本 3：Emoji 符号一行
    let sample3_y = sample2_y + sample2_h + sample_gap;
    let sample3_h = 44.0;
    let sample3_rect = Rect::new(rect.origin.x + padding, sample3_y, inner_w, sample3_h);
    ctx.canvas.rect(
        sample3_rect,
        4.0,
        palette.surface,
        palette.card_border,
        0.5,
        0.0,
    );
    ctx.canvas.text(
        "😀 😃 😄 😁 🎨 🚀 🦀 ⚡ 🌈 💻 📦 ⚙️ 🔍 🔬 💡",
        Rect::new(
            sample3_rect.origin.x + 8.0,
            sample3_rect.origin.y + 4.0,
            sample3_rect.size.x - 16.0,
            sample3_rect.size.y - 8.0,
        ),
        15.0,
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

    // 状态栏语义与键盘可达（Button Role / 可聚焦），点击展开详细回退诊断
    let a11y_summary = format!(
        "Effective Report: Backend: {}, Phases: {}, Emoji: {}, Fallbacks: {}",
        backend_text, phases_text, report.emoji_path, fallback_text
    );
    ctx.control_expandable(
        STATUS_BAR_PART,
        rect,
        Role::Status,
        &a11y_summary,
        false,
        fallback_expanded,
    );
}

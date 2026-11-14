//! 语义设计令牌（Design Tokens）与扁平颜色查找表（LUT，`docs` §4.5）。
//!
//! 设计系统分为两部分：
//!
//! * **令牌（Tokens）** 是稳定的 `u16` 标识符。图元实例只存储令牌，
//!   从不存储具体颜色，因此切换主题绝不会触动实例数据。
//! * **调色板（Palettes）** 将每个令牌映射到具体颜色。切换主题时
//!   只需重建一张 4 KiB 的*扁平*表（512 个 RGBA16F 表项）并单次写入上传。
//!
//!
//! 整个设计赖以成立的不变量：**切换主题不得触发重新布局或重新上传实例**。
//! 两者均通过结构约束在此强制保证——[`ThemeState::switch`] 只会生成新的 LUT。
//!

pub mod snapping;
pub mod zoom;

pub use snapping::{edge_screen, edge_screen_snapped, jitter, snap_error, snap_screen};
pub use zoom::ZoomLayers;

use unbound_math::color::Color;

/// 扁平颜色表中的表项数量。
///
/// 512 个令牌 × 8 字节（RGBA16F）= 4 KiB，恰好符合设计文档的预算。
///
pub const MAX_TOKENS: usize = 512;

/// 打包后颜色表的字节大小。
pub const LUT_BYTES: usize = MAX_TOKENS * 8;

/// 语义颜色令牌。
///
/// 其数值属于 ABI 的一部分：会随 `GpuPrimitiveInstance::fill_token_or_uv` 传往 GPU，
/// 因此绝不可重排。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(u16);

impl Token {
    /// 包装原始令牌 ID，越界时饱和截断而非 panic。
    #[must_use]
    pub const fn new(raw: u16) -> Self {
        Self(if (raw as usize) < MAX_TOKENS {
            raw
        } else {
            MAX_TOKENS as u16 - 1
        })
    }

    /// 存储在图元实例中的原始 ID。
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// 在调色板中的索引位置。
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// 保留的“无颜色”槽位。
    ///
    /// GPU 的 `common.glsl` 将槽位 0 硬编码为全透明，因此“无填充”与“无边框”零开销：
    /// 图元只需指定槽位 0，着色器即读取到 alpha 为 0 的颜色。
    ///
    pub const NONE: Self = Self(0);
    /// 全局阴影颜色。
    ///
    /// 阴影是*主题*级概念——所有图元共享——因此存放在固定槽位而非每个图元的字段中。
    /// 该槽位索引属于引擎与 LUT 填充方之间的契约
    /// （对应 `unbound-gpu/shaders/common.glsl` 中的 `SHADOW_TOKEN`）。
    ///
    pub const SHADOW: Self = Self(1);
    pub const SURFACE: Self = Self(2);
    pub const SURFACE_RAISED: Self = Self(3);
    pub const SURFACE_SUNKEN: Self = Self(4);
    pub const PRIMARY: Self = Self(5);
    pub const ON_PRIMARY: Self = Self(6);
    pub const ACCENT: Self = Self(7);
    pub const ON_ACCENT: Self = Self(8);
    pub const OUTLINE: Self = Self(9);
    pub const OUTLINE_STRONG: Self = Self(10);
    pub const ON_SURFACE: Self = Self(11);
    pub const ON_SURFACE_MUTED: Self = Self(12);
    pub const ERROR: Self = Self(13);
    pub const ON_ERROR: Self = Self(14);
    pub const WARNING: Self = Self(15);
    pub const SUCCESS: Self = Self(16);
    pub const SELECTION: Self = Self(17);
    pub const ON_SELECTION: Self = Self(18);
    pub const HOVER: Self = Self(19);
    pub const PRESSED: Self = Self(20);
    pub const DISABLED: Self = Self(21);
    pub const ON_DISABLED: Self = Self(22);
    pub const GRID: Self = Self(23);
    pub const CANVAS_BACKDROP: Self = Self(24);
    pub const CANVAS_NODE: Self = Self(25);
    pub const CANVAS_NODE_SELECTED: Self = Self(26);
    pub const SNAP_GUIDE: Self = Self(27);
    pub const TEXT_PRIMARY: Self = Self(28);
    pub const TEXT_SECONDARY: Self = Self(29);
    pub const TEXT_DISABLED: Self = Self(30);
    pub const SCRIM: Self = Self(31);

    // ---- 控件状态与容器（32..=60）----------------------------------------
    //
    // 前 32 个槽位属于*基础图元*词汇：表面、强调色与文本。
    // 下方槽位属于*控件*词汇：标准组件的状态颜色及外壳绘制的容器。
    //
    //
    // 每个控件状态各自占用独立槽位，因此悬停（hover）从不重写实例数据——
    // 它仅改变令牌解析出的颜色。这正是主题切换与悬停无需重新上传实例的原因。
    //
    //

    /// 悬停时的主强调填充色。
    pub const PRIMARY_HOVER: Self = Self(32);
    /// 按下时的主强调填充色。
    pub const PRIMARY_PRESSED: Self = Self(33);
    /// 低饱和度主强调浅色背景。
    pub const PRIMARY_SOFT: Self = Self(34);
    /// 低饱和度次强调浅色背景。
    pub const SECONDARY_SOFT: Self = Self(35);
    /// 低饱和度成功状态浅色背景。
    pub const SUCCESS_SOFT: Self = Self(36);
    /// 低饱和度警告状态浅色背景。
    pub const WARNING_SOFT: Self = Self(37);
    /// 低饱和度错误状态浅色背景。
    pub const ERROR_SOFT: Self = Self(38);
    /// 键盘焦点环颜色。
    pub const FOCUS_RING: Self = Self(39);
    /// 滑块或开关的轨道颜色。
    pub const CONTROL_TRACK: Self = Self(40);
    /// 滑块拇指或开关旋钮颜色。
    pub const CONTROL_THUMB: Self = Self(41);
    /// 悬停时的控件表面颜色。
    pub const CONTROL_HOVER: Self = Self(42);
    /// 标题栏表面颜色。
    pub const HEADER_BG: Self = Self(43);
    /// 导航面板表面颜色。
    pub const NAV_BG: Self = Self(44);
    /// 悬停时的导航行颜色。
    pub const NAV_HOVER: Self = Self(45);
    /// 选中的导航行颜色。
    pub const NAV_SELECTED: Self = Self(46);
    /// 极细分隔线颜色。
    pub const DIVIDER: Self = Self(47);
    /// 代码面板背景色。
    pub const CODE_BG: Self = Self(48);
    /// 代码面板前景色。
    pub const CODE_TEXT: Self = Self(49);
    /// 浮层表面颜色。
    pub const OVERLAY_BG: Self = Self(50);
    /// 主视觉渐变的首个色标。
    pub const HERO_START: Self = Self(51);
    /// 主视觉渐变的末尾色标。
    pub const HERO_END: Self = Self(52);
    /// 绘制在主视觉渐变上的文本与图标颜色。
    pub const ON_HERO: Self = Self(53);
    /// 主视觉表面上行动号召按钮背后的遮罩层颜色。
    pub const HERO_SCRIM: Self = Self(54);
    /// 性能 HUD 背景色。
    pub const HUD_BG: Self = Self(55);
    /// 性能 HUD 前景色。
    pub const HUD_TEXT: Self = Self(56);
    /// 低饱和度强调浅色。
    pub const ACCENT_SOFT: Self = Self(57);
    /// 画布或舞台网格的点阵颜色。
    pub const GRID_DOT: Self = Self(58);
    /// 导航行的主文本颜色。
    pub const NAV_TEXT: Self = Self(59);
    /// 导航行的次文本颜色。
    pub const NAV_TEXT_MUTED: Self = Self(60);
    /// 与 [`Token::PRIMARY`] 并列的次强调色。
    ///
    /// 主强调色用于核心行动号召；次强调色用于界面的辅助高亮。
    /// 两者均为*实色*——浅色变体分别为 [`Token::PRIMARY_SOFT`] 与
    /// [`Token::SECONDARY_SOFT`]。
    ///
    pub const SECONDARY: Self = Self(61);
    /// 置于 [`Token::SURFACE`] 之上的卡片表面颜色。
    pub const CARD_BG: Self = Self(62);
    /// 置于 [`Token::SURFACE`] 之上的卡片轮廓颜色。
    pub const CARD_BORDER: Self = Self(63);

    // ---- 禁用 / 输入 / 编辑状态（64..=73）--------------------------------

    /// 禁用状态按钮、输入框或下拉框的填充色。
    pub const CONTROL_DISABLED_BG: Self = Self(64);
    /// 禁用状态控件的轮廓颜色。
    pub const CONTROL_DISABLED_BORDER: Self = Self(65);
    /// 禁用状态滑块或开关的滑块头/旋钮颜色。
    pub const CONTROL_DISABLED_THUMB: Self = Self(66);
    /// 不确定态复选框或混合态进度条的填充色。
    pub const INDETERMINATE_FILL: Self = Self(67);
    /// 文本输入框背景色。
    pub const INPUT_BG: Self = Self(68);
    /// 悬停时的输入框边框颜色。
    pub const INPUT_BORDER_HOVER: Self = Self(69);
    /// 校验错误状态下的输入框边框颜色。
    pub const INPUT_BORDER_ERROR: Self = Self(70);
    /// 校验警告状态下的输入框边框颜色。
    pub const INPUT_BORDER_WARNING: Self = Self(71);
    /// 输入法（IME）预编辑下划线颜色。
    pub const PREEDIT_UNDERLINE: Self = Self(72);
    /// 子菜单行的展开箭头颜色。
    pub const SUBMENU_ARROW: Self = Self(73);
}

/// 设计系统定义的全部语义令牌集合。
///
/// 将列表集中在一处，可避免令牌在某个调色板中重命名而在另一个中被遗漏。
///
///
/// [`Token::NONE`] 不在此列表中：它是引擎保留槽位，而非语义颜色。
///
pub const TOKENS: [Token; 73] = [
    Token::SURFACE,
    Token::SURFACE_RAISED,
    Token::SURFACE_SUNKEN,
    Token::ON_SURFACE,
    Token::ON_SURFACE_MUTED,
    Token::PRIMARY,
    Token::ON_PRIMARY,
    Token::ACCENT,
    Token::ON_ACCENT,
    Token::OUTLINE,
    Token::OUTLINE_STRONG,
    Token::SHADOW,
    Token::ERROR,
    Token::ON_ERROR,
    Token::WARNING,
    Token::SUCCESS,
    Token::SELECTION,
    Token::ON_SELECTION,
    Token::HOVER,
    Token::PRESSED,
    Token::DISABLED,
    Token::ON_DISABLED,
    Token::GRID,
    Token::CANVAS_BACKDROP,
    Token::CANVAS_NODE,
    Token::CANVAS_NODE_SELECTED,
    Token::SNAP_GUIDE,
    Token::TEXT_PRIMARY,
    Token::TEXT_SECONDARY,
    Token::TEXT_DISABLED,
    Token::SCRIM,
    Token::PRIMARY_HOVER,
    Token::PRIMARY_PRESSED,
    Token::PRIMARY_SOFT,
    Token::SECONDARY_SOFT,
    Token::SUCCESS_SOFT,
    Token::WARNING_SOFT,
    Token::ERROR_SOFT,
    Token::FOCUS_RING,
    Token::CONTROL_TRACK,
    Token::CONTROL_THUMB,
    Token::CONTROL_HOVER,
    Token::HEADER_BG,
    Token::NAV_BG,
    Token::NAV_HOVER,
    Token::NAV_SELECTED,
    Token::DIVIDER,
    Token::CODE_BG,
    Token::CODE_TEXT,
    Token::OVERLAY_BG,
    Token::HERO_START,
    Token::HERO_END,
    Token::ON_HERO,
    Token::HERO_SCRIM,
    Token::HUD_BG,
    Token::HUD_TEXT,
    Token::ACCENT_SOFT,
    Token::GRID_DOT,
    Token::NAV_TEXT,
    Token::NAV_TEXT_MUTED,
    Token::SECONDARY,
    Token::CARD_BG,
    Token::CARD_BORDER,
    Token::CONTROL_DISABLED_BG,
    Token::CONTROL_DISABLED_BORDER,
    Token::CONTROL_DISABLED_THUMB,
    Token::INDETERMINATE_FILL,
    Token::INPUT_BG,
    Token::INPUT_BORDER_HOVER,
    Token::INPUT_BORDER_ERROR,
    Token::INPUT_BORDER_WARNING,
    Token::PREEDIT_UNDERLINE,
    Token::SUBMENU_ARROW,
];

/// 当前激活的主题。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    Light,
    Dark,
    HighContrast,
}

impl ThemeKind {
    /// 所有主题，按稳定顺序排列。
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Light, ThemeKind::Dark, ThemeKind::HighContrast];

    /// 用于诊断的简短标识符。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ThemeKind::Light => "light",
            ThemeKind::Dark => "dark",
            ThemeKind::HighContrast => "high-contrast",
        }
    }
}

/// 每个令牌的完整颜色映射。
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    kind: ThemeKind,
    entries: Vec<Color>,
}

impl Palette {
    /// 根据令牌 → 颜色映射构建调色板。
    ///
    /// 未提及的令牌回退为 [`Color::TRANSPARENT`]；表长始终严格为 [`MAX_TOKENS`]。
    ///
    #[must_use]
    pub fn new(kind: ThemeKind, colors: &[(Token, Color)]) -> Self {
        let mut entries = vec![Color::TRANSPARENT; MAX_TOKENS];
        for (token, color) in colors {
            entries[token.index()] = *color;
        }
        Self { kind, entries }
    }

    /// 该调色板所属的主题。
    #[must_use]
    pub const fn kind(&self) -> ThemeKind {
        self.kind
    }

    /// 查询分配给指定令牌的颜色。
    #[must_use]
    pub fn color(&self, token: Token) -> Color {
        self.entries[token.index()]
    }

    /// 按令牌顺序返回所有表项。
    #[must_use]
    pub fn entries(&self) -> &[Color] {
        &self.entries
    }

    /// 从 8 位 sRGB 分量构建调色板。
    ///
    /// 供应用侧编写调色板的便捷形式：每个主题对应一组扁平的 `(token, rgba)` 对，
    /// 未提及的令牌保持透明。
    ///
    #[must_use]
    pub fn from_srgb8(kind: ThemeKind, colors: &[(Token, [u8; 4])]) -> Self {
        let mapped: Vec<(Token, Color)> = colors
            .iter()
            .map(|(token, rgba)| {
                (
                    *token,
                    Color::from_srgb8(rgba[0], rgba[1], rgba[2], rgba[3]),
                )
            })
            .collect();
        Self::new(kind, &mapped)
    }
}

/// 内置的三个主题。
#[derive(Clone, Debug)]
pub struct ThemeSet {
    light: Palette,
    dark: Palette,
    high_contrast: Palette,
}

impl ThemeSet {
    /// 根据三个显式调色板构建主题集。
    ///
    /// 内置 [`ThemeSet::default_set`] 只是其中一种预设；携带自有品牌色的应用
    /// 可自行构建主题集并交给 [`ThemeState::new`]。引擎不关心颜色来源，
    /// 只要求每个令牌都能解析出一个颜色。
    ///
    #[must_use]
    pub fn from_palettes(light: Palette, dark: Palette, high_contrast: Palette) -> Self {
        Self {
            light,
            dark,
            high_contrast,
        }
    }

    /// 默认的 Material 风格浅色/深色主题对及高对比度变体。
    #[must_use]
    pub fn default_set() -> Self {
        let light = Palette::new(
            ThemeKind::Light,
            &[
                (Token::SURFACE, Color::from_srgb8(255, 255, 255, 255)),
                (Token::SURFACE_RAISED, Color::from_srgb8(248, 249, 251, 255)),
                (Token::SURFACE_SUNKEN, Color::from_srgb8(238, 240, 244, 255)),
                (Token::ON_SURFACE, Color::from_srgb8(20, 22, 26, 255)),
                (
                    Token::ON_SURFACE_MUTED,
                    Color::from_srgb8(96, 101, 110, 255),
                ),
                (Token::PRIMARY, Color::from_srgb8(38, 110, 255, 255)),
                (Token::ON_PRIMARY, Color::from_srgb8(255, 255, 255, 255)),
                (Token::ACCENT, Color::from_srgb8(255, 106, 61, 255)),
                (Token::ON_ACCENT, Color::from_srgb8(255, 255, 255, 255)),
                (Token::OUTLINE, Color::from_srgb8(206, 210, 218, 255)),
                (Token::OUTLINE_STRONG, Color::from_srgb8(140, 146, 158, 255)),
                (Token::SHADOW, Color::from_srgb8(0, 0, 0, 64)),
                (Token::ERROR, Color::from_srgb8(206, 47, 47, 255)),
                (Token::ON_ERROR, Color::from_srgb8(255, 255, 255, 255)),
                (Token::WARNING, Color::from_srgb8(214, 137, 16, 255)),
                (Token::SUCCESS, Color::from_srgb8(35, 148, 78, 255)),
                (Token::SELECTION, Color::from_srgb8(38, 110, 255, 64)),
                (Token::ON_SELECTION, Color::from_srgb8(12, 16, 24, 255)),
                (Token::HOVER, Color::from_srgb8(20, 22, 26, 16)),
                (Token::PRESSED, Color::from_srgb8(20, 22, 26, 32)),
                (Token::DISABLED, Color::from_srgb8(20, 22, 26, 24)),
                (Token::ON_DISABLED, Color::from_srgb8(96, 101, 110, 96)),
                (Token::GRID, Color::from_srgb8(206, 210, 218, 128)),
                (
                    Token::CANVAS_BACKDROP,
                    Color::from_srgb8(246, 247, 250, 255),
                ),
                (Token::CANVAS_NODE, Color::from_srgb8(255, 255, 255, 255)),
                (
                    Token::CANVAS_NODE_SELECTED,
                    Color::from_srgb8(38, 110, 255, 255),
                ),
                (Token::SNAP_GUIDE, Color::from_srgb8(255, 45, 85, 255)),
                (Token::TEXT_PRIMARY, Color::from_srgb8(20, 22, 26, 255)),
                (Token::TEXT_SECONDARY, Color::from_srgb8(96, 101, 110, 255)),
                (Token::TEXT_DISABLED, Color::from_srgb8(120, 125, 133, 128)),
                (Token::SCRIM, Color::from_srgb8(0, 0, 0, 96)),
                (Token::PRIMARY_HOVER, Color::from_srgb8(58, 126, 255, 255)),
                (Token::PRIMARY_PRESSED, Color::from_srgb8(22, 88, 224, 255)),
                (Token::PRIMARY_SOFT, Color::from_srgb8(232, 238, 254, 255)),
                (Token::SECONDARY_SOFT, Color::from_srgb8(240, 234, 254, 255)),
                (Token::SUCCESS_SOFT, Color::from_srgb8(228, 246, 236, 255)),
                (Token::WARNING_SOFT, Color::from_srgb8(252, 242, 223, 255)),
                (Token::ERROR_SOFT, Color::from_srgb8(253, 233, 234, 255)),
                (Token::FOCUS_RING, Color::from_srgb8(38, 110, 255, 255)),
                (Token::CONTROL_TRACK, Color::from_srgb8(228, 231, 239, 255)),
                (Token::CONTROL_THUMB, Color::from_srgb8(255, 255, 255, 255)),
                (Token::CONTROL_HOVER, Color::from_srgb8(236, 239, 246, 255)),
                (Token::HEADER_BG, Color::from_srgb8(255, 255, 255, 255)),
                (Token::NAV_BG, Color::from_srgb8(247, 248, 251, 255)),
                (Token::NAV_HOVER, Color::from_srgb8(236, 238, 244, 255)),
                (Token::NAV_SELECTED, Color::from_srgb8(228, 235, 253, 255)),
                (Token::DIVIDER, Color::from_srgb8(234, 236, 242, 255)),
                (Token::CODE_BG, Color::from_srgb8(28, 32, 41, 255)),
                (Token::CODE_TEXT, Color::from_srgb8(215, 220, 234, 255)),
                (Token::OVERLAY_BG, Color::from_srgb8(255, 255, 255, 255)),
                (Token::HERO_START, Color::from_srgb8(59, 110, 246, 255)),
                (Token::HERO_END, Color::from_srgb8(138, 92, 246, 255)),
                (Token::ON_HERO, Color::from_srgb8(255, 255, 255, 255)),
                (Token::HERO_SCRIM, Color::from_srgb8(15, 18, 28, 200)),
                (Token::HUD_BG, Color::from_srgb8(255, 255, 255, 246)),
                (Token::HUD_TEXT, Color::from_srgb8(28, 30, 38, 255)),
                (Token::ACCENT_SOFT, Color::from_srgb8(255, 240, 232, 255)),
                (Token::GRID_DOT, Color::from_srgb8(216, 220, 232, 255)),
                (Token::NAV_TEXT, Color::from_srgb8(42, 47, 58, 255)),
                (Token::NAV_TEXT_MUTED, Color::from_srgb8(122, 128, 144, 255)),
                (Token::SECONDARY, Color::from_srgb8(124, 106, 240, 255)),
                (Token::CARD_BG, Color::from_srgb8(255, 255, 255, 255)),
                (Token::CARD_BORDER, Color::from_srgb8(228, 230, 238, 255)),
                (
                    Token::CONTROL_DISABLED_BG,
                    Color::from_srgb8(241, 242, 246, 255),
                ),
                (
                    Token::CONTROL_DISABLED_BORDER,
                    Color::from_srgb8(213, 216, 224, 255),
                ),
                (
                    Token::CONTROL_DISABLED_THUMB,
                    Color::from_srgb8(196, 200, 210, 255),
                ),
                (
                    Token::INDETERMINATE_FILL,
                    Color::from_srgb8(38, 110, 255, 255),
                ),
                (Token::INPUT_BG, Color::WHITE),
                (
                    Token::INPUT_BORDER_HOVER,
                    Color::from_srgb8(154, 163, 181, 255),
                ),
                (
                    Token::INPUT_BORDER_ERROR,
                    Color::from_srgb8(229, 72, 77, 255),
                ),
                (
                    Token::INPUT_BORDER_WARNING,
                    Color::from_srgb8(217, 119, 6, 255),
                ),
                (
                    Token::PREEDIT_UNDERLINE,
                    Color::from_srgb8(38, 110, 255, 255),
                ),
                (Token::SUBMENU_ARROW, Color::from_srgb8(107, 114, 128, 255)),
            ],
        );

        let dark = Palette::new(
            ThemeKind::Dark,
            &[
                (Token::SURFACE, Color::from_srgb8(24, 26, 31, 255)),
                (Token::SURFACE_RAISED, Color::from_srgb8(32, 35, 42, 255)),
                (Token::SURFACE_SUNKEN, Color::from_srgb8(16, 17, 21, 255)),
                (Token::ON_SURFACE, Color::from_srgb8(233, 236, 241, 255)),
                (
                    Token::ON_SURFACE_MUTED,
                    Color::from_srgb8(160, 166, 176, 255),
                ),
                (Token::PRIMARY, Color::from_srgb8(120, 165, 255, 255)),
                (Token::ON_PRIMARY, Color::from_srgb8(10, 18, 34, 255)),
                (Token::ACCENT, Color::from_srgb8(255, 148, 104, 255)),
                (Token::ON_ACCENT, Color::from_srgb8(34, 14, 4, 255)),
                (Token::OUTLINE, Color::from_srgb8(64, 68, 78, 255)),
                (Token::OUTLINE_STRONG, Color::from_srgb8(108, 114, 126, 255)),
                (Token::SHADOW, Color::from_srgb8(0, 0, 0, 160)),
                (Token::ERROR, Color::from_srgb8(255, 120, 120, 255)),
                (Token::ON_ERROR, Color::from_srgb8(40, 8, 8, 255)),
                (Token::WARNING, Color::from_srgb8(240, 180, 70, 255)),
                (Token::SUCCESS, Color::from_srgb8(110, 214, 150, 255)),
                (Token::SELECTION, Color::from_srgb8(120, 165, 255, 56)),
                (Token::ON_SELECTION, Color::from_srgb8(240, 245, 255, 255)),
                (Token::HOVER, Color::from_srgb8(233, 236, 241, 18)),
                (Token::PRESSED, Color::from_srgb8(233, 236, 241, 38)),
                (Token::DISABLED, Color::from_srgb8(233, 236, 241, 32)),
                (Token::ON_DISABLED, Color::from_srgb8(160, 166, 176, 96)),
                (Token::GRID, Color::from_srgb8(64, 68, 78, 140)),
                (Token::CANVAS_BACKDROP, Color::from_srgb8(18, 20, 24, 255)),
                (Token::CANVAS_NODE, Color::from_srgb8(44, 48, 56, 255)),
                (
                    Token::CANVAS_NODE_SELECTED,
                    Color::from_srgb8(120, 165, 255, 255),
                ),
                (Token::SNAP_GUIDE, Color::from_srgb8(255, 92, 122, 255)),
                (Token::TEXT_PRIMARY, Color::from_srgb8(233, 236, 241, 255)),
                (Token::TEXT_SECONDARY, Color::from_srgb8(160, 166, 176, 255)),
                (Token::TEXT_DISABLED, Color::from_srgb8(120, 126, 134, 128)),
                (Token::SCRIM, Color::from_srgb8(0, 0, 0, 160)),
                (Token::PRIMARY_HOVER, Color::from_srgb8(140, 180, 255, 255)),
                (Token::PRIMARY_PRESSED, Color::from_srgb8(88, 132, 235, 255)),
                (Token::PRIMARY_SOFT, Color::from_srgb8(34, 48, 79, 255)),
                (Token::SECONDARY_SOFT, Color::from_srgb8(46, 37, 66, 255)),
                (Token::SUCCESS_SOFT, Color::from_srgb8(28, 58, 42, 255)),
                (Token::WARNING_SOFT, Color::from_srgb8(59, 47, 27, 255)),
                (Token::ERROR_SOFT, Color::from_srgb8(61, 34, 38, 255)),
                (Token::FOCUS_RING, Color::from_srgb8(150, 195, 255, 255)),
                (Token::CONTROL_TRACK, Color::from_srgb8(45, 51, 65, 255)),
                (Token::CONTROL_THUMB, Color::from_srgb8(237, 241, 250, 255)),
                (Token::CONTROL_HOVER, Color::from_srgb8(56, 64, 82, 255)),
                (Token::HEADER_BG, Color::from_srgb8(23, 26, 33, 255)),
                (Token::NAV_BG, Color::from_srgb8(20, 23, 29, 255)),
                (Token::NAV_HOVER, Color::from_srgb8(35, 40, 51, 255)),
                (Token::NAV_SELECTED, Color::from_srgb8(38, 51, 79, 255)),
                (Token::DIVIDER, Color::from_srgb8(38, 44, 56, 255)),
                (Token::CODE_BG, Color::from_srgb8(14, 16, 21, 255)),
                (Token::CODE_TEXT, Color::from_srgb8(201, 210, 228, 255)),
                (Token::OVERLAY_BG, Color::from_srgb8(32, 37, 47, 255)),
                (Token::HERO_START, Color::from_srgb8(76, 125, 255, 255)),
                (Token::HERO_END, Color::from_srgb8(160, 107, 255, 255)),
                (Token::ON_HERO, Color::from_srgb8(255, 255, 255, 255)),
                (Token::HERO_SCRIM, Color::from_srgb8(8, 10, 16, 200)),
                (Token::HUD_BG, Color::from_srgb8(8, 10, 14, 224)),
                (Token::HUD_TEXT, Color::from_srgb8(220, 227, 242, 255)),
                (Token::ACCENT_SOFT, Color::from_srgb8(58, 42, 34, 255)),
                (Token::GRID_DOT, Color::from_srgb8(51, 58, 72, 255)),
                (Token::NAV_TEXT, Color::from_srgb8(228, 233, 243, 255)),
                (Token::NAV_TEXT_MUTED, Color::from_srgb8(138, 147, 166, 255)),
                (Token::SECONDARY, Color::from_srgb8(160, 130, 255, 255)),
                (Token::CARD_BG, Color::from_srgb8(32, 35, 42, 255)),
                (Token::CARD_BORDER, Color::from_srgb8(52, 56, 66, 255)),
                (
                    Token::CONTROL_DISABLED_BG,
                    Color::from_srgb8(42, 44, 51, 255),
                ),
                (
                    Token::CONTROL_DISABLED_BORDER,
                    Color::from_srgb8(74, 77, 87, 255),
                ),
                (
                    Token::CONTROL_DISABLED_THUMB,
                    Color::from_srgb8(90, 94, 105, 255),
                ),
                (
                    Token::INDETERMINATE_FILL,
                    Color::from_srgb8(122, 162, 255, 255),
                ),
                (Token::INPUT_BG, Color::from_srgb8(32, 34, 41, 255)),
                (
                    Token::INPUT_BORDER_HOVER,
                    Color::from_srgb8(108, 114, 128, 255),
                ),
                (
                    Token::INPUT_BORDER_ERROR,
                    Color::from_srgb8(255, 107, 107, 255),
                ),
                (
                    Token::INPUT_BORDER_WARNING,
                    Color::from_srgb8(255, 178, 36, 255),
                ),
                (
                    Token::PREEDIT_UNDERLINE,
                    Color::from_srgb8(122, 162, 255, 255),
                ),
                (Token::SUBMENU_ARROW, Color::from_srgb8(160, 166, 180, 255)),
            ],
        );

        // 高对比度并非“对比度更高的深色主题”：它会彻底移除半透明效果，
        // 使每个令牌都完全不透明。
        let high_contrast = Palette::new(
            ThemeKind::HighContrast,
            &[
                (Token::SURFACE, Color::BLACK),
                (Token::SURFACE_RAISED, Color::BLACK),
                (Token::SURFACE_SUNKEN, Color::BLACK),
                (Token::ON_SURFACE, Color::WHITE),
                (Token::ON_SURFACE_MUTED, Color::WHITE),
                (Token::PRIMARY, Color::from_srgb8(255, 230, 0, 255)),
                (Token::ON_PRIMARY, Color::BLACK),
                (Token::ACCENT, Color::from_srgb8(0, 230, 255, 255)),
                (Token::ON_ACCENT, Color::BLACK),
                (Token::OUTLINE, Color::WHITE),
                (Token::OUTLINE_STRONG, Color::WHITE),
                (Token::SHADOW, Color::BLACK),
                (Token::ERROR, Color::from_srgb8(255, 80, 80, 255)),
                (Token::ON_ERROR, Color::BLACK),
                (Token::WARNING, Color::from_srgb8(255, 200, 0, 255)),
                (Token::SUCCESS, Color::from_srgb8(0, 255, 128, 255)),
                (Token::SELECTION, Color::from_srgb8(255, 230, 0, 255)),
                (Token::ON_SELECTION, Color::BLACK),
                (Token::HOVER, Color::from_srgb8(255, 255, 255, 48)),
                (Token::PRESSED, Color::from_srgb8(255, 255, 255, 96)),
                (Token::DISABLED, Color::from_srgb8(255, 255, 255, 64)),
                (Token::ON_DISABLED, Color::from_srgb8(255, 255, 255, 128)),
                (Token::GRID, Color::WHITE),
                (Token::CANVAS_BACKDROP, Color::BLACK),
                (Token::CANVAS_NODE, Color::from_srgb8(16, 16, 16, 255)),
                (
                    Token::CANVAS_NODE_SELECTED,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (Token::SNAP_GUIDE, Color::from_srgb8(255, 0, 128, 255)),
                (Token::TEXT_PRIMARY, Color::WHITE),
                (Token::TEXT_SECONDARY, Color::WHITE),
                (Token::TEXT_DISABLED, Color::from_srgb8(255, 255, 255, 160)),
                (Token::SCRIM, Color::BLACK),
                (Token::PRIMARY_HOVER, Color::from_srgb8(255, 245, 120, 255)),
                (Token::PRIMARY_PRESSED, Color::from_srgb8(200, 190, 0, 255)),
                (Token::PRIMARY_SOFT, Color::from_srgb8(60, 58, 0, 255)),
                (Token::SECONDARY_SOFT, Color::from_srgb8(0, 58, 66, 255)),
                (Token::SUCCESS_SOFT, Color::from_srgb8(0, 60, 32, 255)),
                (Token::WARNING_SOFT, Color::from_srgb8(60, 48, 0, 255)),
                (Token::ERROR_SOFT, Color::from_srgb8(66, 16, 16, 255)),
                (Token::FOCUS_RING, Color::from_srgb8(255, 230, 0, 255)),
                (Token::CONTROL_TRACK, Color::from_srgb8(48, 48, 48, 255)),
                (Token::CONTROL_THUMB, Color::WHITE),
                (Token::CONTROL_HOVER, Color::from_srgb8(64, 64, 64, 255)),
                (Token::HEADER_BG, Color::BLACK),
                (Token::NAV_BG, Color::BLACK),
                (Token::NAV_HOVER, Color::from_srgb8(40, 40, 40, 255)),
                (Token::NAV_SELECTED, Color::from_srgb8(255, 230, 0, 255)),
                (Token::DIVIDER, Color::WHITE),
                (Token::CODE_BG, Color::BLACK),
                (Token::CODE_TEXT, Color::WHITE),
                (Token::OVERLAY_BG, Color::BLACK),
                (Token::HERO_START, Color::from_srgb8(0, 120, 255, 255)),
                (Token::HERO_END, Color::from_srgb8(120, 0, 255, 255)),
                (Token::ON_HERO, Color::BLACK),
                (Token::HERO_SCRIM, Color::BLACK),
                (Token::HUD_BG, Color::BLACK),
                (Token::HUD_TEXT, Color::WHITE),
                (Token::ACCENT_SOFT, Color::from_srgb8(60, 32, 16, 255)),
                (Token::GRID_DOT, Color::from_srgb8(64, 64, 64, 255)),
                (Token::NAV_TEXT, Color::WHITE),
                (Token::NAV_TEXT_MUTED, Color::from_srgb8(200, 200, 200, 255)),
                (Token::SECONDARY, Color::from_srgb8(200, 180, 255, 255)),
                (Token::CARD_BG, Color::BLACK),
                (Token::CARD_BORDER, Color::WHITE),
                (
                    Token::CONTROL_DISABLED_BG,
                    Color::from_srgb8(32, 32, 32, 255),
                ),
                (Token::CONTROL_DISABLED_BORDER, Color::WHITE),
                (Token::CONTROL_DISABLED_THUMB, Color::WHITE),
                (
                    Token::INDETERMINATE_FILL,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (Token::INPUT_BG, Color::BLACK),
                (
                    Token::INPUT_BORDER_HOVER,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (
                    Token::INPUT_BORDER_ERROR,
                    Color::from_srgb8(255, 80, 80, 255),
                ),
                (
                    Token::INPUT_BORDER_WARNING,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (
                    Token::PREEDIT_UNDERLINE,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (Token::SUBMENU_ARROW, Color::WHITE),
            ],
        );

        Self {
            light,
            dark,
            high_contrast,
        }
    }

    /// 指定主题对应的调色板。
    #[must_use]
    pub fn palette(&self, kind: ThemeKind) -> &Palette {
        match kind {
            ThemeKind::Light => &self.light,
            ThemeKind::Dark => &self.dark,
            ThemeKind::HighContrast => &self.high_contrast,
        }
    }
}

/// 已打包的 4 KiB 颜色表，可直接上传至 GPU。
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeLut {
    /// 按令牌顺序排列的 512 × RGBA16F 数据。
    entries: Vec<[u16; 4]>,
    /// 同一张表序列化后的小端序字节流。
    bytes: Vec<u8>,
}

impl ThemeLut {
    /// 将调色板打包为扁平颜色表。
    #[must_use]
    pub fn from_palette(palette: &Palette) -> Self {
        let entries: Vec<[u16; 4]> = palette.entries().iter().map(|c| c.to_rgba16f()).collect();
        let mut bytes = Vec::with_capacity(LUT_BYTES);
        for entry in &entries {
            for channel in entry {
                bytes.extend_from_slice(&channel.to_le_bytes());
            }
        }
        Self { entries, bytes }
    }

    /// 原始 RGBA16F 表项切片。
    #[must_use]
    pub fn entries(&self) -> &[[u16; 4]] {
        &self.entries
    }

    /// 按 GPU 期望格式排列的字节切片。
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 将令牌颜色解码回线性 RGBA。
    #[must_use]
    pub fn color(&self, token: Token) -> Color {
        let e = self.entries[token.index()];
        Color::new(
            unbound_math::f16_to_f32(e[0]),
            unbound_math::f16_to_f32(e[1]),
            unbound_math::f16_to_f32(e[2]),
            unbound_math::f16_to_f32(e[3]),
        )
    }
}

/// 主题切换的开销统计。
///
/// 其中两个 `false` 字段正是整个设计的核心目标：主题切换既不得触发重新布局，
/// 也不得重新上传图元实例。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SwitchOutcome {
    /// 写入颜色表的字节数。
    pub bytes_uploaded: u32,
    /// 布局引擎是否需要重新运行。
    pub relayout_required: bool,
    /// 图元实例是否需要重新上传。
    pub instance_reupload_required: bool,
    /// 本次切换是否实际改变了主题。
    pub changed: bool,
}

/// 应用的实时主题状态。
#[derive(Clone, Debug)]
pub struct ThemeState {
    set: ThemeSet,
    current: ThemeKind,
    lut: ThemeLut,
    /// 已执行的切换次数，用于诊断。
    switches: u64,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self::new(ThemeSet::default_set(), ThemeKind::Light)
    }
}

impl ThemeState {
    /// 使用显式主题集初始化状态。
    #[must_use]
    pub fn new(set: ThemeSet, kind: ThemeKind) -> Self {
        let lut = ThemeLut::from_palette(set.palette(kind));
        Self {
            set,
            current: kind,
            lut,
            switches: 0,
        }
    }

    /// 当前激活的主题。
    #[must_use]
    pub const fn current(&self) -> ThemeKind {
        self.current
    }

    /// 已打包的颜色表。
    #[must_use]
    pub const fn lut(&self) -> &ThemeLut {
        &self.lut
    }

    /// 累计发生的切换次数。
    #[must_use]
    pub const fn switch_count(&self) -> u64 {
        self.switches
    }

    /// 切换到另一个主题。
    ///
    /// 唯一的副作用是生成新的颜色表：无重新布局、无实例上传、无重排序——
    /// 图元引用的是令牌而非具体颜色。
    pub fn switch(&mut self, kind: ThemeKind) -> SwitchOutcome {
        if kind == self.current {
            return SwitchOutcome {
                bytes_uploaded: 0,
                relayout_required: false,
                instance_reupload_required: false,
                changed: false,
            };
        }
        self.lut = ThemeLut::from_palette(self.set.palette(kind));
        self.current = kind;
        self.switches += 1;
        SwitchOutcome {
            bytes_uploaded: LUT_BYTES as u32,
            relayout_required: false,
            instance_reupload_required: false,
            changed: true,
        }
    }

    /// 在浅色与深色主题间切换，保持高对比度主题不变。
    pub fn toggle_light_dark(&mut self) -> SwitchOutcome {
        match self.current {
            ThemeKind::Light => self.switch(ThemeKind::Dark),
            ThemeKind::Dark => self.switch(ThemeKind::Light),
            ThemeKind::HighContrast => SwitchOutcome {
                bytes_uploaded: 0,
                relayout_required: false,
                instance_reupload_required: false,
                changed: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lut_is_exactly_four_kibibytes() {
        let state = ThemeState::default();
        assert_eq!(state.lut().bytes().len(), 4096);
        assert_eq!(LUT_BYTES, 4096);
        assert_eq!(state.lut().entries().len(), MAX_TOKENS);
    }

    #[test]
    fn every_token_has_a_colour_in_every_theme() {
        let set = ThemeSet::default_set();
        for kind in ThemeKind::ALL {
            let palette = set.palette(kind);
            for token in TOKENS {
                assert!(
                    palette
                        .color(token)
                        .to_array()
                        .iter()
                        .all(|c| c.is_finite()),
                    "token {} has no finite colour in {}",
                    token.raw(),
                    kind.name()
                );
            }
        }
    }

    /// GPU 保留两个槽位：0 为“无颜色”，1 为阴影。
    ///
    /// 若调色板在槽位 0 放入真实颜色，所有“无填充”图元都会绘制该颜色；
    /// 若在槽位 1 放入其他颜色，所有解析阴影都会变成实心色块。
    ///
    #[test]
    fn slot_zero_is_transparent_and_slot_one_is_shadow() {
        let set = ThemeSet::default_set();
        for kind in ThemeKind::ALL {
            let palette = set.palette(kind);
            let none = palette.color(Token::NONE);
            assert_eq!(
                none.to_array()[3],
                0.0,
                "slot 0 must be fully transparent in {}",
                kind.name()
            );
            assert_eq!(
                palette.color(Token::SHADOW).to_array(),
                palette.entries()[Token::SHADOW.index()].to_array(),
                "slot 1 must be the shadow colour in {}",
                kind.name()
            );
            // 阴影不得为透明：不可见的阴影等于永不渲染。
            //
            assert!(palette.color(Token::SHADOW).to_array()[3] > 0.0);
            assert_eq!(Token::NONE.raw(), 0);
            assert_eq!(Token::SHADOW.raw(), 1);
        }
    }

    /// 每个语义令牌都拥有各自独立的槽位。
    #[test]
    fn the_semantic_tokens_never_share_a_slot() {
        let mut seen = std::collections::HashSet::new();
        for token in TOKENS {
            assert!(
                seen.insert(token.raw()),
                "token {} shares a slot with another token",
                token.raw()
            );
        }
        // 引擎保留槽位必须保持保留状态。
        assert!(!seen.contains(&Token::NONE.raw()));
    }

    #[test]
    fn a_switch_uploads_four_kibibytes_and_nothing_else() {
        let mut state = ThemeState::default();
        let outcome = state.switch(ThemeKind::Dark);
        assert_eq!(outcome.bytes_uploaded, 4096);
        assert!(
            !outcome.relayout_required,
            "a theme switch must not re-layout"
        );
        assert!(
            !outcome.instance_reupload_required,
            "a theme switch must not re-upload instances"
        );
        assert!(outcome.changed);
        assert_eq!(state.current(), ThemeKind::Dark);
        assert_eq!(state.switch_count(), 1);
    }

    #[test]
    fn switching_to_the_active_theme_is_a_no_op() {
        let mut state = ThemeState::default();
        let outcome = state.switch(ThemeKind::Light);
        assert_eq!(outcome.bytes_uploaded, 0);
        assert!(!outcome.changed);
        assert_eq!(state.switch_count(), 0);
    }

    #[test]
    fn the_colour_table_actually_changes_with_the_theme() {
        let mut state = ThemeState::default();
        let light = state.lut().color(Token::SURFACE);
        state.switch(ThemeKind::Dark);
        let dark = state.lut().color(Token::SURFACE);
        assert_ne!(light.to_array(), dark.to_array());
        assert!(light.luminance_perceptual() > dark.luminance_perceptual());
    }

    #[test]
    fn the_lut_round_trips_through_half_floats() {
        let state = ThemeState::default();
        let original = state.set.palette(ThemeKind::Light).color(Token::PRIMARY);
        let decoded = state.lut().color(Token::PRIMARY);
        let a = original.to_array();
        let b = decoded.to_array();
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 0.005, "{x} vs {y}");
        }
    }

    /// 主题模块曾在其内部维护与 `unbound-math` 规范解码器并行的半精度浮点解码器。
    /// 同一格式存在两个解码器就有两处产生分歧的风险——事实也确实如此：
    /// 旧解码器把所有指数为 31 的位模式（包括 NaN）都映射成了无穷大，
    /// 而规范解码器对 NaN 会正确返回 NaN。
    ///
    ///
    /// 主题模块现已直接使用规范解码器。本测试防止两者再次偏离，并锁定 NaN 行为。
    ///
    #[test]
    fn the_theme_uses_the_canonical_half_float_decoder() {
        // 一个信号 NaN（signalling NaN）与一个安静 NaN（quiet NaN）。
        for bits in [0x7c01u16, 0x7e00, 0xfc01] {
            let decoded = unbound_math::f16_to_f32(bits);
            assert!(
                decoded.is_nan(),
                "pattern {bits:#06x} must decode to NaN, got {decoded}"
            );
        }
        // 普通数值仍按 LUT 预期的方式解码。
        assert_eq!(unbound_math::f16_to_f32(0x3c00), 1.0);
        assert_eq!(unbound_math::f16_to_f32(0x0000), 0.0);
    }

    #[test]
    fn high_contrast_removes_translucency() {
        let set = ThemeSet::default_set();
        let palette = set.palette(ThemeKind::HighContrast);
        // 仅三种状态叠加层允许保持半透明；所有内容颜色必须完全不透明，
        // 否则高对比度主题在任意壁纸上都会发虚。
        //
        let translucent = [
            Token::HOVER,
            Token::PRESSED,
            Token::DISABLED,
            Token::ON_DISABLED,
            Token::TEXT_DISABLED,
        ];
        for token in TOKENS {
            let alpha = palette.color(token).to_array()[3];
            if translucent.contains(&token) {
                assert!(alpha < 1.0, "token {} should be translucent", token.raw());
            } else {
                assert_eq!(alpha, 1.0, "token {} must be opaque", token.raw());
            }
        }
    }

    #[test]
    fn token_ids_are_stable_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for token in TOKENS {
            assert!(seen.insert(token.raw()), "duplicate token {}", token.raw());
        }
        // 最低的两个 ID 已冻结，因为内置着色器直接引用它们：槽位 0 为“无颜色”，槽位 1 为阴影。
        //
        assert_eq!(Token::NONE.raw(), 0);
        assert_eq!(Token::SHADOW.raw(), 1);
        assert_eq!(Token::PRIMARY.raw(), 5);
    }

    #[test]
    fn a_token_out_of_range_saturates_instead_of_panicking() {
        assert_eq!(Token::new(u16::MAX).index(), MAX_TOKENS - 1);
    }
}

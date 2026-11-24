//! WinUI 3 规范导航视图（NavigationView）：完整应用级顶层导航壳控件。
//!
//! 本模块严格对齐 WinUI 3 `Microsoft.UI.Xaml.Controls.NavigationView` 官方规格与
//! Rust 编码规范，统一承载五种显示模式（`Auto`、`Left`、`LeftCompact`、`LeftMinimal`、`Top`）、
//! 多级层级菜单与折叠态祖先选中回退、紧凑/顶部子菜单 Flyout、`Top` 模式自动溢出菜单、
//! 窗格独立垂直滚动、`AutoSuggestBox` 搜索槽位、`TitleBar` 双向联动、原生 Mica 透出与
//! `BackdropRequest` 局部毛玻璃 Overlay 浮层、`unbound-anim` 弹簧动效以及键盘树导航与
//! AccessKit 无障碍语义。
//!
//! 设计遵循全库三条契约：
//! 1. **跨状态图元数量恒定**：在固定描述符与状态快照下，24 种 [`ControlState`] 组合产出完全一致的图元数量。
//! 2. **颜色全部走语义令牌**：严格使用 [`Token`]，零硬编码色值，圆角遵循 `0 / 4 / 8`（指示条胶囊除外）。
//! 3. **纯净导航职责**：不持有页面路由历史栈，仅维护导航 UI 状态并输出强类型 [`NavigationViewEvent`]。

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yActions, A11yNode};
use unbound_anim::{Spring, SpringPreset};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
use crate::overlay::{place_anchored_popup_detailed, BackdropRequest, Placement};
use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::scroll::{emit_scrollbar, ScrollBarDesc, ScrollBarVisibility, SCROLLBAR_HOLD};
use crate::shell::{emit_shell_icon_primitives, ShellIcon};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 展开模式下左侧窗格默认宽度（对齐 WinUI `NavigationView.OpenPaneLength` = 320）。
pub const NAVIGATION_VIEW_OPEN_PANE_LENGTH: f32 = 320.0;

/// 紧凑模式下左侧图标轨默认宽度（对齐 WinUI `NavigationViewCompactPaneLength` = 48）。
pub const NAVIGATION_VIEW_COMPACT_PANE_LENGTH: f32 = 48.0;

/// `Auto` 模式下从 `Minimal` 切换为 `Compact` 的窗口宽度阈值（对齐 WinUI 默认 640）。
pub const NAVIGATION_VIEW_COMPACT_THRESHOLD_WIDTH: f32 = 640.0;

/// `Auto` 模式下从 `Compact` 切换为 `Expanded` 的窗口宽度阈值（对齐 WinUI 默认 1008）。
pub const NAVIGATION_VIEW_EXPANDED_THRESHOLD_WIDTH: f32 = 1008.0;

/// 左侧导航条目行高（对齐 WinUI `NavigationViewItemOnLeftMinHeight` = 36）。
pub const NAVIGATION_VIEW_ITEM_HEIGHT: f32 = 36.0;

/// 左侧导航条目垂直外边距（对齐 WinUI `NavigationViewItemButtonMargin` 上下各 2px）。
pub const NAVIGATION_VIEW_ITEM_MARGIN_Y: f32 = 2.0;

/// 顶部导航栏高度（对齐 WinUI `NavigationViewTopPaneHeight` = 48）。
pub const NAVIGATION_VIEW_TOP_PANE_HEIGHT: f32 = 48.0;

/// 紧凑/最小模式浮层阴影的水平偏移（参考图左右各溢出 3px，故不偏移）。
pub const NAVIGATION_VIEW_OVERLAY_SHADOW_OFFSET_X: f32 = 0.0;

/// 紧凑/最小模式浮层阴影的垂直偏移（参考图顶部不外溢、底部溢出 8px）。
pub const NAVIGATION_VIEW_OVERLAY_SHADOW_OFFSET_Y: f32 = 4.0;

/// 紧凑/最小模式浮层的阴影模糊半径。
pub const NAVIGATION_VIEW_OVERLAY_SHADOW_BLUR: f32 = 6.0;

/// 页头固定高度（对齐 WinUI `NavigationView` Header 规范 = 52）。
pub const NAVIGATION_VIEW_HEADER_HEIGHT: f32 = 52.0;

/// 左侧选中指示条宽度（对齐 WinUI `NavigationViewSelectionIndicatorWidth` = 3）。
pub const NAVIGATION_VIEW_INDICATOR_WIDTH: f32 = 3.0;

/// 左侧选中指示条高度（对齐 WinUI `NavigationViewSelectionIndicatorHeight` = 16）。
pub const NAVIGATION_VIEW_INDICATOR_HEIGHT: f32 = 16.0;

/// 选中指示条胶囊圆角（对齐 WinUI `NavigationViewSelectionIndicatorRadius` = 1.5）。
pub const NAVIGATION_VIEW_INDICATOR_RADIUS: f32 = 1.5;

/// 顶部模式底边水平选中指示条宽度。
pub const NAVIGATION_VIEW_TOP_INDICATOR_WIDTH: f32 = 16.0;

/// 顶部模式底边水平选中指示条高度。
pub const NAVIGATION_VIEW_TOP_INDICATOR_HEIGHT: f32 = 3.0;

/// 窗格水平内边距（对齐 WinUI `NavigationViewItemButtonMargin` 左右 4px）。
pub const NAVIGATION_VIEW_PANE_PADDING: f32 = 4.0;

/// 窗格顶部按钮行（返回键 / 汉堡键）的纵向补偿。
///
/// 参考图里这两个按钮的图标中心比「顶边 + 内边距」低 2px：返回键中心 y=39.5、
/// 汉堡键中心 y=79.5（控件原点 16），故整行统一下移 2px。
pub const NAVIGATION_VIEW_PANE_HEADER_TOP_INSET: f32 = 2.0;

/// `PaneTitle` 相对汉堡按钮右缘的水平外边距（参考图文字左缘 = 按钮右缘 + 3）。
pub const NAVIGATION_VIEW_PANE_TITLE_INSET_X: f32 = 3.0;

/// AutoSuggestBox 相对窗格内容区的额外内缩。
///
/// 参考图搜索框 x=32..219（窗格内容区 20..231），即左右各再缩 12px；纵向比
/// 光标基准低 4px。
pub const NAVIGATION_VIEW_SEARCH_INSET_X: f32 = 12.0;

/// AutoSuggestBox 相对光标基准的纵向偏移。
pub const NAVIGATION_VIEW_SEARCH_INSET_Y: f32 = 4.0;

/// AutoSuggestBox 之后到首条菜单项的上间距。
///
/// 参考图搜索框下缘 135 → 首条菜单行顶 150，故为「32 行高 + 16」。
pub const NAVIGATION_VIEW_SEARCH_GAP_AFTER: f32 = 16.0;

/// 分组标题行高。
///
/// WinUI 的 `NavigationViewItemHeader` 走标准 40px 行高（与菜单项同高），
/// 此前取 28px 会让整段主菜单上移 12px。
pub const NAVIGATION_VIEW_SECTION_HEADER_HEIGHT: f32 = 40.0;

/// 分隔线行高。
pub const NAVIGATION_VIEW_SEPARATOR_HEIGHT: f32 = 9.0;

/// 顶部模式下 Settings 页脚条目的宽度（齿轮 + 文字整块）。
pub const NAVIGATION_VIEW_TOP_FOOTER_WIDTH: f32 = 98.0;

/// 内容卡片内页头标题的左侧内缩（WinUI 实测标题文字左缘距卡片左缘 58px）。
pub const NAVIGATION_VIEW_CONTENT_HEADER_INSET_X: f32 = 58.0;

/// 内容卡片内页头标题的顶部内缩（页头 52px 行垂直居中后文字中心落在卡片顶 +66）。
pub const NAVIGATION_VIEW_CONTENT_HEADER_INSET_Y: f32 = 40.0;

/// 每级层级子项的左侧缩进增量。
///
/// 参考图里父项图标左缘 32、子项图标左缘 65（父项 = 控件原点 + 16 + 12），
/// 故子项相对父项额外缩进 32px；选中强调条也跟着缩到子项自己的按钮左缘（51）。
pub const NAVIGATION_VIEW_CHILD_INDENT: f32 = 32.0;

/// 内容区容器圆角半径（对齐 Windows 11 `OverlayCornerRadius` = 8）。
pub const NAVIGATION_VIEW_CONTENT_CORNER_RADIUS: f32 = 8.0;

/// Overlay 浮层与 Flyout 的 Dual-Kawase 局部毛玻璃模糊半径（逻辑像素）。
pub const NAVIGATION_VIEW_OVERLAY_BLUR: f32 = 24.0;

/// 尾部 `InfoBadge` 胶囊默认宽度。
pub const NAVIGATION_VIEW_BADGE_WIDTH: f32 = 26.0;

/// 尾部 `InfoBadge` 胶囊默认高度（对齐 WinUI `InfoBadgeMaxHeight` = 16）。
pub const NAVIGATION_VIEW_BADGE_HEIGHT: f32 = 16.0;

/// 内置 `Settings` 项的保留标识。
pub const NAVIGATION_VIEW_SETTINGS_ID: u64 = u64::MAX - 1;

/// 内置返回按钮的保留标识。
pub const NAVIGATION_VIEW_BACK_BUTTON_ID: u64 = u64::MAX - 2;

/// 内置汉堡折叠按钮的保留标识。
pub const NAVIGATION_VIEW_PANE_TOGGLE_ID: u64 = u64::MAX - 3;

/// 紧凑态搜索图标按钮的保留标识。
pub const NAVIGATION_VIEW_SEARCH_BUTTON_ID: u64 = u64::MAX - 4;

/// 顶部模式溢出 `...` 按钮的保留标识。
pub const NAVIGATION_VIEW_OVERFLOW_BUTTON_ID: u64 = u64::MAX - 5;

/// 导航视图的期望窗格显示模式（对齐 WinUI `NavigationViewPaneDisplayMode`）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum NavigationViewPaneDisplayMode {
    /// 按窗口宽度在 `Minimal` / `Compact` / `Expanded` 间自动切换。
    #[default]
    Auto,
    /// 窗格常驻左侧，可折叠为紧凑图标轨。
    Left,
    /// 窗格默认仅显示 48px 图标轨，打开时以 Overlay 浮层覆盖内容区。
    LeftCompact,
    /// 仅显示菜单按钮，打开时以 Overlay 浮层覆盖内容区。
    LeftMinimal,
    /// 窗格位于内容区上方，水平排布并支持自动溢出菜单。
    Top,
}

/// 导航视图当前实际生效的显示模式（对齐 WinUI `NavigationViewDisplayMode`）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum NavigationViewDisplayMode {
    /// 极简模式：仅显示菜单按钮。
    Minimal,
    /// 紧凑模式：显示左侧 48px 图标轨。
    Compact,
    /// 展开模式：窗格完全展开（含 `Left` 与 `Top`）。
    #[default]
    Expanded,
}

/// 返回按钮可见性策略（对齐 WinUI `NavigationViewBackButtonVisible`）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum NavigationViewBackButtonVisible {
    /// 隐藏且不保留布局空间。
    Collapsed,
    /// 始终显示。
    Visible,
    /// 跟随模式自动决定（桌面默认显示）。
    #[default]
    Auto,
}

/// 顶部模式溢出按钮标签策略（对齐 WinUI `NavigationViewOverflowLabelMode`）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum NavigationViewOverflowLabelMode {
    /// 显示 `More` 文案与图标。
    #[default]
    MoreLabel,
    /// 仅显示 `...` 省略号图标。
    NoLabel,
}

/// 导航项图标来源：支持矢量外壳图标或 Segoe Fluent Icons 字形。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationViewIcon {
    /// 引擎内置几何矢量图标。
    Shell(ShellIcon),
    /// 字体图标字形字符串（如 Segoe Fluent Icons 的 `"\u{E80F}"`）。
    Glyph(String),
}

impl From<ShellIcon> for NavigationViewIcon {
    fn from(icon: ShellIcon) -> Self {
        Self::Shell(icon)
    }
}

/// 单个导航条目（对齐 WinUI `NavigationViewItem`）。
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationViewItem {
    /// 稳定条目标识。
    pub id: u64,
    /// 显示文本。
    pub content: String,
    /// 可选前置图标。
    pub icon: Option<NavigationViewIcon>,
    /// 可选尾部 `InfoBadge` 文本。
    pub info_badge: Option<String>,
    /// 嵌套子项（支持多级层级菜单）。
    pub children: Vec<NavigationViewMenuItem>,
    /// 点击时是否允许成为选中项（纯目录分组父项可设为 `false`）。
    pub selects_on_invoked: bool,
    /// 是否可交互。
    pub is_enabled: bool,
    /// 可选工具提示文本。
    pub tooltip: Option<String>,
}

impl NavigationViewItem {
    /// 创建一个可选择的导航项。
    #[must_use]
    pub fn new(id: u64, content: impl Into<String>) -> Self {
        Self {
            id,
            content: content.into(),
            icon: None,
            info_badge: None,
            children: Vec::new(),
            selects_on_invoked: true,
            is_enabled: true,
            tooltip: None,
        }
    }

    /// 设置前置图标。
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<NavigationViewIcon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// 设置 Segoe Fluent Icons 字形图标。
    #[must_use]
    pub fn with_glyph(mut self, glyph: impl Into<String>) -> Self {
        self.icon = Some(NavigationViewIcon::Glyph(glyph.into()));
        self
    }

    /// 附加尾部 `InfoBadge` 文本。
    #[must_use]
    pub fn with_info_badge(mut self, badge: impl Into<String>) -> Self {
        self.info_badge = Some(badge.into());
        self
    }

    /// 追加嵌套子项。
    #[must_use]
    pub fn with_children(mut self, children: Vec<NavigationViewMenuItem>) -> Self {
        self.children = children;
        self
    }

    /// 设置点击时是否自动选中。
    #[must_use]
    pub const fn selects_on_invoked(mut self, selects: bool) -> Self {
        self.selects_on_invoked = selects;
        self
    }

    /// 设置启用状态。
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.is_enabled = enabled;
        self
    }

    /// 设置工具提示文本。
    #[must_use]
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// 是否拥有可展开的子菜单项。
    #[must_use]
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// 分组标题项（对齐 WinUI `NavigationViewItemHeader`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationViewItemHeader {
    /// 分组标题文本。
    pub content: String,
}

impl NavigationViewItemHeader {
    /// 创建分组标题。
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// 分隔线项（对齐 WinUI `NavigationViewItemSeparator`）。
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationViewItemSeparator {
    /// 不透明度；设为 `0.0` 时渲染为纯留白分隔。
    pub opacity: f32,
}

impl Default for NavigationViewItemSeparator {
    fn default() -> Self {
        Self { opacity: 1.0 }
    }
}

/// 导航菜单项联合体（对齐 `NavigationView.MenuItems` 的三种标准元素）。
#[derive(Clone, Debug, PartialEq)]
pub enum NavigationViewMenuItem {
    /// 可交互导航项。
    Item(NavigationViewItem),
    /// 分组标题。
    Header(NavigationViewItemHeader),
    /// 分隔线。
    Separator(NavigationViewItemSeparator),
}

impl NavigationViewMenuItem {
    /// 快捷包装导航项。
    #[must_use]
    pub const fn item(item: NavigationViewItem) -> Self {
        Self::Item(item)
    }

    /// 快捷创建分组标题。
    #[must_use]
    pub fn header(content: impl Into<String>) -> Self {
        Self::Header(NavigationViewItemHeader::new(content))
    }

    /// 快捷创建默认分隔线。
    #[must_use]
    pub fn separator() -> Self {
        Self::Separator(NavigationViewItemSeparator::default())
    }
}

/// 导航视图强类型事件队列项（对齐 WinUI `NavigationView` 全部核心事件）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationViewEvent {
    /// 用户激活了一个导航项（即使已选中也会触发）。
    ItemInvoked {
        /// 被激活的条目标识。
        item_id: u64,
        /// 被激活的是否为内置 `Settings` 项。
        is_settings_invoked: bool,
    },
    /// 当前选中项发生变更。
    SelectionChanged {
        /// 新选中条目标识。
        selected_item_id: Option<u64>,
        /// 新选中的是否为内置 `Settings` 项。
        is_settings_selected: bool,
    },
    /// 用户点击返回按钮且当前无浮层可关闭时触发。
    BackRequested,
    /// 窗格即将打开。
    PaneOpening,
    /// 窗格即将关闭。
    PaneClosing,
    /// 实际生效的显示模式发生变化。
    DisplayModeChanged {
        /// 新生效的显示模式。
        display_mode: NavigationViewDisplayMode,
    },
    /// 带子项的父节点展开。
    Expanding {
        /// 展开的父节点标识。
        item_id: u64,
    },
    /// 带子项的父节点折叠。
    Collapsed {
        /// 折叠的父节点标识。
        item_id: u64,
    },
    /// 用户点击紧凑态搜索按钮或在搜索框提交查询。
    SearchRequested {
        /// 当前搜索文本。
        query: String,
    },
}

/// 与 `TitleBarDesc` 双向联动时对外暴露的同步快照。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NavigationViewTitleBarSync {
    /// 标题栏是否应显示窗格折叠按钮（`Top` 模式下自动为 `false`）。
    pub is_pane_toggle_button_visible: bool,
    /// 当前窗格是否处于打开态。
    pub is_pane_open: bool,
    /// 标题栏是否应显示返回按钮。
    pub is_back_button_visible: bool,
    /// 返回按钮是否可点击。
    pub is_back_enabled: bool,
}

/// 排版后的单行类型。
#[derive(Clone, Debug, PartialEq)]
pub enum NavigationViewRowKind {
    /// 分组标题。
    Header { content: String },
    /// 分隔线。
    Separator { opacity: f32 },
    /// 主列表或层级子项。
    Item {
        item: NavigationViewItem,
        depth: usize,
        is_expanded: bool,
    },
    /// 吸底页脚项。
    FooterItem { item: NavigationViewItem },
    /// 内置 `Settings` 项。
    Settings,
}

/// 已完成几何排版的单行记录。
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationViewRow {
    /// 行的类型与元数据。
    pub kind: NavigationViewRowKind,
    /// 该行在视口中的矩形。
    pub rect: Rect,
}

impl NavigationViewRow {
    /// 若该行对应可交互条目，返回其稳定标识。
    #[must_use]
    pub fn item_id(&self) -> Option<u64> {
        match &self.kind {
            NavigationViewRowKind::Item { item, .. }
            | NavigationViewRowKind::FooterItem { item } => Some(item.id),
            NavigationViewRowKind::Settings => Some(NAVIGATION_VIEW_SETTINGS_ID),
            NavigationViewRowKind::Header { .. } | NavigationViewRowKind::Separator { .. } => None,
        }
    }

    /// 该行是否可点击交互。
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        match &self.kind {
            NavigationViewRowKind::Item { item, .. }
            | NavigationViewRowKind::FooterItem { item } => item.is_enabled,
            NavigationViewRowKind::Settings => true,
            NavigationViewRowKind::Header { .. } | NavigationViewRowKind::Separator { .. } => false,
        }
    }
}

/// `NavigationView` 整壳布局求解结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigationViewLayout {
    /// 当前生效的显示模式。
    pub display_mode: NavigationViewDisplayMode,
    /// 当前是否处于顶部水平模式。
    pub is_top: bool,
    /// 当前展开的窗格是否为覆盖式 Overlay 浮层。
    pub is_overlay: bool,
    /// 窗格占用的布局矩形（常驻或折叠轨）。
    pub pane_rect: Rect,
    /// 当 `is_overlay` 且窗格打开时，覆盖浮层的完整矩形。
    pub overlay_rect: Option<Rect>,
    /// 顶部页头（`Header`）矩形。
    pub header_rect: Option<Rect>,
    /// 右侧/下方主内容区外壳矩形（圆角卡片层）。
    pub content_rect: Rect,
    /// 扣除推荐内边距（`Minimal` 12px，其余 24px）后的内容区内部矩形。
    pub content_inner_rect: Rect,
    /// 内置返回按钮矩形。
    pub back_button_rect: Option<Rect>,
    /// 内置汉堡折叠按钮矩形。
    pub pane_toggle_rect: Option<Rect>,
    /// 内置 `AutoSuggestBox` 搜索框（或紧凑态搜索按钮）矩形。
    pub search_rect: Option<Rect>,
    /// 主菜单列表的可滚动视口矩形。
    pub menu_viewport_rect: Rect,
    /// 底部页脚固定区矩形。
    pub footer_rect: Rect,
    /// 顶部模式溢出 `...` 按钮矩形。
    pub overflow_button_rect: Option<Rect>,
    /// 顶部模式溢出下拉菜单矩形。
    pub overflow_popup_rect: Option<Rect>,
    /// `LeftCompact` / `Top` 子菜单 Flyout 矩形。
    pub flyout_rect: Option<Rect>,
    /// 主菜单与页脚的可见排版行。
    pub rows: Vec<NavigationViewRow>,
    /// 顶部模式被收纳进溢出菜单的行。
    pub overflow_rows: Vec<NavigationViewRow>,
    /// 当前打开的子菜单 Flyout 内的行。
    pub flyout_rows: Vec<NavigationViewRow>,
    /// 主菜单内容总高度（用于计算 `scroll_max`）。
    pub menu_content_height: f32,
    /// 窗格展开动画进度（`0.0` = 紧凑态，`1.0` = 完全展开或 `Top` 模式）。
    pub pane_expand_progress: f32,
}

/// `NavigationView` 发射输出（包含标准图元/语义与局部毛玻璃请求）。
#[derive(Clone, Debug, Default)]
pub struct NavigationViewOutput {
    /// 标准控件图元、载荷、命中区与 AccessKit 语义输出。
    pub output: WidgetOutput,
    /// 当 Overlay 浮层、子菜单 Flyout 或顶部溢出菜单打开时发出的局部毛玻璃请求。
    pub backdrop: Option<BackdropRequest>,
    /// 本帧求解出的完整布局几何。
    pub layout: NavigationViewLayout,
}

/// `NavigationView` 的持久化交互与动效状态（DOD 状态机）。
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationViewState {
    /// 当前实际生效的显示模式。
    pub display_mode: NavigationViewDisplayMode,
    /// 窗格当前是否打开。
    pub is_pane_open: bool,
    /// 当前选中条目标识（含 `NAVIGATION_VIEW_SETTINGS_ID`）。
    pub selected_item_id: Option<u64>,
    /// 当前悬停条目标识。
    pub hovered_item_id: Option<u64>,
    /// 当前键盘焦点所在条目标识。
    pub focused_item_id: Option<u64>,
    /// 已展开的层级父项标识列表。
    pub expanded_item_ids: Vec<u64>,
    /// `LeftCompact` / `Top` 模式下当前打开子菜单 Flyout 的父项标识。
    pub open_flyout_parent_id: Option<u64>,
    /// `Top` 模式下溢出下拉菜单是否打开。
    pub overflow_menu_open: bool,
    /// 主菜单区独立垂直滚动偏移（逻辑像素）。
    pub scroll_offset: f32,
    /// 主菜单区最大滚动偏移（由布局回写）。
    pub scroll_max: f32,
    /// 指针当前是否悬停在 Left 窗格（含 Overlay 浮层）区域内。
    pub pane_hovered: bool,
    /// 指针当前是否悬停在 Left 窗格菜单区滚动条热区（轨道或滑块）上。
    pub scrollbar_hovered: bool,
    /// Left 窗格菜单区滚动条滑块拖拽起始状态 `(start_pointer_y, start_scroll_offset)`。
    pub scrollbar_drag: Option<(f32, f32)>,
    /// 近期滚轮滚动后在紧凑折叠态按需保持滚动条可见的剩余时长（秒）。
    pub scrollbar_recent_scroll_timer: f32,
    /// 搜索框当前文本。
    pub search_query: String,
    /// 搜索框是否请求聚焦（例如紧凑态点击搜索图标后自动展开并聚焦）。
    pub search_focused: bool,
    /// 是否启用弹簧动效（无头测试与 Parity 可关闭或调用 `snap_animations`）。
    pub animations_enabled: bool,
    /// 窗格宽度弹簧。
    pub pane_width_spring: Spring,
    /// 选中指示条主轴起点弹簧。
    pub indicator_pos_spring: Spring,
    /// 选中指示条主轴长度弹簧（滑动时短暂伸长再回弹）。
    pub indicator_len_spring: Spring,
    /// Overlay 浮层展开进度弹簧（`0.0..=1.0`）。
    pub overlay_progress_spring: Spring,
    /// 待应用层消费的强类型事件队列。
    pub events: Vec<NavigationViewEvent>,
}

impl Default for NavigationViewState {
    fn default() -> Self {
        Self {
            display_mode: NavigationViewDisplayMode::Expanded,
            is_pane_open: true,
            selected_item_id: None,
            hovered_item_id: None,
            focused_item_id: None,
            expanded_item_ids: Vec::new(),
            open_flyout_parent_id: None,
            overflow_menu_open: false,
            scroll_offset: 0.0,
            scroll_max: 0.0,
            pane_hovered: false,
            scrollbar_hovered: false,
            scrollbar_drag: None,
            scrollbar_recent_scroll_timer: 0.0,
            search_query: String::new(),
            search_focused: false,
            animations_enabled: true,
            pane_width_spring: Spring::new(SpringPreset::Gentle, NAVIGATION_VIEW_OPEN_PANE_LENGTH),
            indicator_pos_spring: Spring::new(SpringPreset::Snappy, 0.0),
            indicator_len_spring: Spring::new(
                SpringPreset::Snappy,
                NAVIGATION_VIEW_INDICATOR_HEIGHT,
            ),
            overlay_progress_spring: Spring::new(SpringPreset::Gentle, 0.0),
            events: Vec::new(),
        }
    }
}

impl NavigationViewState {
    /// 按给定描述符初始化状态并立即收敛弹簧到终态。
    #[must_use]
    pub fn from_desc(desc: &NavigationViewDesc) -> Self {
        let mut state = Self {
            is_pane_open: desc.is_pane_open,
            selected_item_id: desc.selected_item_id,
            hovered_item_id: desc.hovered_item_id,
            ..Self::default()
        };
        state.sync_viewport(desc, desc.rect.width());
        state.snap_animations(desc);
        state.events.clear();
        state
    }

    /// 启用或关闭动效（关闭时自动将所有弹簧吸附到目标值）。
    pub fn set_animations_enabled(&mut self, enabled: bool, desc: &NavigationViewDesc) {
        self.animations_enabled = enabled;
        if !enabled {
            self.snap_animations(desc);
        }
    }

    /// 立即将全部动效弹簧收敛到终态（供无头测试与双端 Parity 零抖动快照使用）。
    pub fn snap_animations(&mut self, desc: &NavigationViewDesc) {
        let target_w = self.target_pane_width(desc);
        self.pane_width_spring.snap_to(target_w);
        let target_overlay = if self.is_overlay_open(desc) { 1.0 } else { 0.0 };
        self.overlay_progress_spring.snap_to(target_overlay);
        let target_len = if desc.pane_display_mode == NavigationViewPaneDisplayMode::Top {
            NAVIGATION_VIEW_TOP_INDICATOR_WIDTH
        } else {
            NAVIGATION_VIEW_INDICATOR_HEIGHT
        };
        self.indicator_len_spring.snap_to(target_len);
        self.indicator_pos_spring
            .snap_to(self.indicator_pos_spring.target);
    }

    /// 推进所有弹簧动效 `dt` 秒；若仍有动效在运行返回 `true`。
    pub fn tick(&mut self, dt: f32, desc: &NavigationViewDesc) -> bool {
        if !self.animations_enabled {
            self.snap_animations(desc);
            return false;
        }
        self.pane_width_spring
            .set_target(self.target_pane_width(desc));
        self.overlay_progress_spring
            .set_target(if self.is_overlay_open(desc) { 1.0 } else { 0.0 });
        let a = self.pane_width_spring.step(dt);
        let b = self.indicator_pos_spring.step(dt);
        let c = self.indicator_len_spring.step(dt);
        let d = self.overlay_progress_spring.step(dt);
        let mut timer_active = false;
        if self.scrollbar_recent_scroll_timer > 0.0
            && !self.pane_hovered
            && !self.scrollbar_hovered
            && self.scrollbar_drag.is_none()
        {
            self.scrollbar_recent_scroll_timer =
                (self.scrollbar_recent_scroll_timer - dt.max(0.0)).max(0.0);
            timer_active = true;
        }
        a || b || c || d || timer_active
    }

    /// 取出并清空待处理事件队列。
    pub fn drain_events(&mut self) -> Vec<NavigationViewEvent> {
        core::mem::take(&mut self.events)
    }

    /// 根据窗口宽度与 `PaneDisplayMode` 同步实际生效的 [`NavigationViewDisplayMode`]。
    pub fn sync_viewport(&mut self, desc: &NavigationViewDesc, viewport_width: f32) {
        let resolved = desc.resolve_display_mode(viewport_width);
        if resolved != self.display_mode {
            self.display_mode = resolved;
            // `Auto` 降级为 Compact 或 Minimal 时自动收起覆盖浮层，升至 Expanded 时自动展开。
            if desc.pane_display_mode == NavigationViewPaneDisplayMode::Auto {
                self.is_pane_open = resolved == NavigationViewDisplayMode::Expanded;
            }
            self.open_flyout_parent_id = None;
            self.overflow_menu_open = false;
            self.events.push(NavigationViewEvent::DisplayModeChanged {
                display_mode: resolved,
            });
        }
        let target_w = self.target_pane_width(desc);
        self.pane_width_spring.set_target(target_w);
        if !self.animations_enabled {
            self.pane_width_spring.snap_to(target_w);
        }
    }

    /// 切换窗格展开/收起状态。
    pub fn toggle_pane(&mut self, desc: &NavigationViewDesc) {
        self.set_pane_open(!self.is_pane_open, desc);
    }

    /// 显式设置窗格开关状态并触发 `PaneOpening` / `PaneClosing`。
    pub fn set_pane_open(&mut self, open: bool, desc: &NavigationViewDesc) {
        if self.is_pane_open == open {
            return;
        }
        if open {
            self.events.push(NavigationViewEvent::PaneOpening);
        } else {
            self.events.push(NavigationViewEvent::PaneClosing);
            self.open_flyout_parent_id = None;
            self.search_focused = false;
        }
        self.is_pane_open = open;
        let target_w = self.target_pane_width(desc);
        self.pane_width_spring.set_target(target_w);
        self.overlay_progress_spring
            .set_target(if self.is_overlay_open(desc) { 1.0 } else { 0.0 });
        if !self.animations_enabled {
            self.snap_animations(desc);
        }
    }

    /// 响应返回按钮请求：若当前存在 Overlay 浮层、子菜单 Flyout 或溢出菜单则优先关闭浮层，否则发出 `BackRequested`。
    pub fn request_back(&mut self, desc: &NavigationViewDesc) -> bool {
        if self.open_flyout_parent_id.is_some() {
            self.open_flyout_parent_id = None;
            return true;
        }
        if self.overflow_menu_open {
            self.overflow_menu_open = false;
            return true;
        }
        if self.is_overlay_open(desc) {
            self.set_pane_open(false, desc);
            return true;
        }
        if desc.is_back_enabled {
            self.events.push(NavigationViewEvent::BackRequested);
            return true;
        }
        false
    }

    /// 触发点击某个导航项（含层级展开、Compact/Top Flyout、选中与事件发射）。
    pub fn invoke_item(&mut self, desc: &NavigationViewDesc, item_id: u64) -> bool {
        if item_id == NAVIGATION_VIEW_BACK_BUTTON_ID {
            return self.request_back(desc);
        }
        if item_id == NAVIGATION_VIEW_PANE_TOGGLE_ID {
            self.toggle_pane(desc);
            return true;
        }
        if item_id == NAVIGATION_VIEW_SEARCH_BUTTON_ID {
            if !self.is_pane_open {
                self.set_pane_open(true, desc);
            }
            self.search_focused = true;
            self.events.push(NavigationViewEvent::SearchRequested {
                query: self.search_query.clone(),
            });
            return true;
        }
        if item_id == NAVIGATION_VIEW_OVERFLOW_BUTTON_ID {
            self.overflow_menu_open = !self.overflow_menu_open;
            self.open_flyout_parent_id = None;
            return true;
        }
        if item_id == NAVIGATION_VIEW_SETTINGS_ID {
            self.events.push(NavigationViewEvent::ItemInvoked {
                item_id,
                is_settings_invoked: true,
            });
            self.select_item(Some(item_id), true);
            self.close_transient_surfaces(desc);
            return true;
        }

        let Some(item) = desc.find_item(item_id) else {
            return false;
        };
        if !item.is_enabled {
            return false;
        }

        let is_top = desc.pane_display_mode == NavigationViewPaneDisplayMode::Top;
        let is_compact_closed = !is_top
            && !self.is_pane_open
            && self.display_mode != NavigationViewDisplayMode::Minimal;

        if item.has_children() {
            if is_top || is_compact_closed {
                if self.open_flyout_parent_id == Some(item_id) {
                    self.open_flyout_parent_id = None;
                } else {
                    self.open_flyout_parent_id = Some(item_id);
                    self.overflow_menu_open = false;
                }
            } else {
                self.toggle_item_expanded(item_id);
            }
        }

        self.events.push(NavigationViewEvent::ItemInvoked {
            item_id,
            is_settings_invoked: false,
        });

        if item.selects_on_invoked {
            self.select_item(Some(item_id), false);
            if !item.has_children() {
                self.close_transient_surfaces(desc);
            }
        }
        true
    }

    /// 编程式更新选中项（不触发 `ItemInvoked`，仅在变化时触发 `SelectionChanged`）。
    pub fn select_item(&mut self, item_id: Option<u64>, is_settings: bool) {
        if self.selected_item_id == item_id {
            return;
        }
        self.selected_item_id = item_id;
        // 触发选中指示条连接滑动弹性伸长。
        self.indicator_len_spring.velocity += 220.0;
        self.events.push(NavigationViewEvent::SelectionChanged {
            selected_item_id: item_id,
            is_settings_selected: is_settings || item_id == Some(NAVIGATION_VIEW_SETTINGS_ID),
        });
    }

    /// 切换层级父项展开/折叠状态。
    pub fn toggle_item_expanded(&mut self, item_id: u64) {
        if let Some(pos) = self.expanded_item_ids.iter().position(|&id| id == item_id) {
            self.expanded_item_ids.remove(pos);
            self.events.push(NavigationViewEvent::Collapsed { item_id });
        } else {
            self.expanded_item_ids.push(item_id);
            self.events.push(NavigationViewEvent::Expanding { item_id });
        }
    }

    /// 判断父项是否处于展开状态。
    #[must_use]
    pub fn is_item_expanded(&self, item_id: u64) -> bool {
        self.expanded_item_ids.contains(&item_id)
    }

    /// 计算当前应绘制选中指示条的「有效可见项 ID」：
    /// 若真实选中项位于已折叠的祖先分支内，自动回退到其第一个可见祖先。
    #[must_use]
    pub fn effective_selected_id(&self, desc: &NavigationViewDesc) -> Option<u64> {
        let selected = self.selected_item_id?;
        if selected == NAVIGATION_VIEW_SETTINGS_ID {
            return desc.is_settings_visible.then_some(selected);
        }
        if let Some(chain) = desc.ancestor_chain(selected) {
            // 自顶向下找到第一个被折叠的祖先；若全部祖先都已展开则返回真实选中项。
            for &ancestor in &chain[..chain.len().saturating_sub(1)] {
                if !self.is_item_expanded(ancestor) {
                    return Some(ancestor);
                }
            }
            return Some(selected);
        }
        // 检查页脚菜单项。
        if desc
            .footer_menu_items
            .iter()
            .any(|m| matches!(m, NavigationViewMenuItem::Item(it) if it.id == selected))
        {
            return Some(selected);
        }
        None
    }

    /// 滚动主菜单列表。
    pub fn scroll_by(&mut self, delta_y: f32) -> bool {
        let next = (self.scroll_offset + delta_y).clamp(0.0, self.scroll_max.max(0.0));
        if (next - self.scroll_offset).abs() > f32::EPSILON {
            self.scroll_offset = next;
            self.scrollbar_recent_scroll_timer = SCROLLBAR_HOLD;
            true
        } else {
            false
        }
    }

    /// 更新 Left 窗格（含 Overlay 浮层）悬停状态（状态变化时返回 `true`）。
    pub fn set_pane_hovered(&mut self, hovered: bool) -> bool {
        if self.pane_hovered != hovered {
            self.pane_hovered = hovered;
            true
        } else {
            false
        }
    }

    /// 更新 Left 窗格菜单区滚动条热区悬停状态（状态变化时返回 `true`）。
    pub fn set_scrollbar_hovered(&mut self, hovered: bool) -> bool {
        if self.scrollbar_hovered != hovered {
            self.scrollbar_hovered = hovered;
            true
        } else {
            false
        }
    }

    /// 处理指针移动：若正在拖拽滑块则更新滚动偏移，否则同步窗格与滚动条悬停态。
    pub fn handle_pointer_move(
        &mut self,
        desc: &NavigationViewDesc,
        layout: &NavigationViewLayout,
        pos: Vec2,
    ) -> bool {
        let active_pane_rect = layout.overlay_rect.unwrap_or(layout.pane_rect);
        let in_pane = !layout.is_top
            && !(layout.display_mode == NavigationViewDisplayMode::Minimal && !self.is_pane_open)
            && active_pane_rect.contains_point(pos);
        let pane_changed = self.set_pane_hovered(in_pane);
        if self.scrollbar_drag.is_some() {
            let dragged = self.handle_pointer_drag(desc, layout, pos);
            return pane_changed || dragged;
        }
        let hovered = desc
            .menu_scrollbar_desc(layout, self, false)
            .is_some_and(|bar| bar.is_visible() && bar.track_rect().contains_point(pos));
        let bar_changed = self.set_scrollbar_hovered(hovered);
        pane_changed || bar_changed
    }

    /// 处理指针拖拽：拖拽 Left 窗格滚动条滑块时同步更新垂直滚动偏移。
    pub fn handle_pointer_drag(
        &mut self,
        desc: &NavigationViewDesc,
        layout: &NavigationViewLayout,
        pos: Vec2,
    ) -> bool {
        let Some((start_pointer_y, start_offset)) = self.scrollbar_drag else {
            return false;
        };
        let Some(bar) = desc.menu_scrollbar_desc(layout, self, false) else {
            self.scrollbar_drag = None;
            return false;
        };
        if !bar.is_visible() {
            self.scrollbar_drag = None;
            return false;
        }
        self.scroll_max = bar.max_offset();
        self.scroll_offset = bar.offset_for_drag(start_offset, start_pointer_y, pos.y);
        true
    }

    /// 处理指针释放：结束 Left 窗格滚动条滑块拖拽。
    pub fn handle_pointer_up(&mut self, _pos: Vec2) -> bool {
        self.scrollbar_drag.take().is_some()
    }

    /// 计算供 `TitleBarDesc` 双向消费的同步状态。
    #[must_use]
    pub fn title_bar_sync(&self, desc: &NavigationViewDesc) -> NavigationViewTitleBarSync {
        let is_top = desc.pane_display_mode == NavigationViewPaneDisplayMode::Top;
        let back_visible = match desc.is_back_button_visible {
            NavigationViewBackButtonVisible::Collapsed => false,
            NavigationViewBackButtonVisible::Visible | NavigationViewBackButtonVisible::Auto => {
                true
            }
        };
        NavigationViewTitleBarSync {
            is_pane_toggle_button_visible: !is_top,
            is_pane_open: self.is_pane_open,
            is_back_button_visible: back_visible,
            is_back_enabled: desc.is_back_enabled || self.is_overlay_open(desc),
        }
    }

    /// 窗格当前是否以覆盖浮层形态打开。
    #[must_use]
    pub fn is_overlay_open(&self, desc: &NavigationViewDesc) -> bool {
        if desc.pane_display_mode == NavigationViewPaneDisplayMode::Top {
            return false;
        }
        self.is_pane_open
            && matches!(
                self.display_mode,
                NavigationViewDisplayMode::Compact | NavigationViewDisplayMode::Minimal
            )
    }

    /// 当前是否有任一瞬态浮层（Overlay / Flyout / 溢出菜单）处于打开态。
    #[must_use]
    pub fn has_open_transient_surface(&self, desc: &NavigationViewDesc) -> bool {
        self.is_overlay_open(desc)
            || self.open_flyout_parent_id.is_some()
            || self.overflow_menu_open
    }

    /// 关闭所有瞬态浮层（在选中叶子节点后自动收起 Overlay / Flyout / 溢出菜单）。
    fn close_transient_surfaces(&mut self, desc: &NavigationViewDesc) {
        self.open_flyout_parent_id = None;
        self.overflow_menu_open = false;
        if self.is_overlay_open(desc) {
            self.set_pane_open(false, desc);
        }
    }

    /// 窗格目标宽度。
    #[must_use]
    fn target_pane_width(&self, desc: &NavigationViewDesc) -> f32 {
        if desc.pane_display_mode == NavigationViewPaneDisplayMode::Top {
            return desc.rect.width();
        }
        match self.display_mode {
            NavigationViewDisplayMode::Expanded => {
                if self.is_pane_open {
                    desc.open_pane_length
                } else {
                    desc.compact_pane_length
                }
            }
            NavigationViewDisplayMode::Compact => desc.compact_pane_length,
            NavigationViewDisplayMode::Minimal => 0.0,
        }
    }

    /// 处理指针按下（命中行激活、外部点击轻闭浮层）。
    pub fn handle_pointer_down(
        &mut self,
        desc: &NavigationViewDesc,
        layout: &NavigationViewLayout,
        pos: Vec2,
    ) -> bool {
        // 1. 优先命中子菜单 Flyout。
        if let Some(flyout_r) = layout.flyout_rect {
            if flyout_r.contains_point(pos) {
                for row in &layout.flyout_rows {
                    if row.rect.contains_point(pos) {
                        if let Some(id) = row.item_id() {
                            return self.invoke_item(desc, id);
                        }
                    }
                }
                return true;
            }
            // 点击 Flyout 外部自动轻闭。
            self.open_flyout_parent_id = None;
            return true;
        }

        // 2. 优先命中 Top 溢出下拉菜单。
        if let Some(popup_r) = layout.overflow_popup_rect {
            if popup_r.contains_point(pos) {
                for row in &layout.overflow_rows {
                    if row.rect.contains_point(pos) {
                        if let Some(id) = row.item_id() {
                            return self.invoke_item(desc, id);
                        }
                    }
                }
                return true;
            }
            self.overflow_menu_open = false;
            return true;
        }

        // 3. Overlay 浮层外部点击轻闭。
        if let Some(overlay_r) = layout.overlay_rect {
            if !overlay_r.contains_point(pos) {
                self.set_pane_open(false, desc);
                return true;
            }
        }

        // 4. 命中内置按钮与搜索槽位。
        if layout
            .back_button_rect
            .is_some_and(|r| r.contains_point(pos))
        {
            return self.invoke_item(desc, NAVIGATION_VIEW_BACK_BUTTON_ID);
        }
        if layout
            .pane_toggle_rect
            .is_some_and(|r| r.contains_point(pos))
        {
            return self.invoke_item(desc, NAVIGATION_VIEW_PANE_TOGGLE_ID);
        }
        if layout
            .overflow_button_rect
            .is_some_and(|r| r.contains_point(pos))
        {
            return self.invoke_item(desc, NAVIGATION_VIEW_OVERFLOW_BUTTON_ID);
        }
        if layout.search_rect.is_some_and(|r| r.contains_point(pos)) {
            return self.invoke_item(desc, NAVIGATION_VIEW_SEARCH_BUTTON_ID);
        }

        // 4.5 命中 Left 窗格菜单区叠加滚动条（优先于下方菜单行：滑块启动拖拽，轨道翻页）。
        if let Some(bar) = desc.menu_scrollbar_desc(layout, self, false) {
            if bar.is_visible() {
                if bar.thumb_rect().contains_point(pos) {
                    self.scroll_max = bar.max_offset();
                    self.scrollbar_drag = Some((pos.y, self.scroll_offset));
                    self.scrollbar_hovered = true;
                    return true;
                }
                if let Some(paged) = bar.page_offset_for_track_click(pos) {
                    self.scroll_max = bar.max_offset();
                    self.scroll_offset = paged;
                    self.scrollbar_hovered = true;
                    return true;
                }
            }
        }

        // 5. 命中主菜单与页脚条目。
        for row in &layout.rows {
            if row.rect.contains_point(pos) {
                if let Some(id) = row.item_id() {
                    self.focused_item_id = Some(id);
                    return self.invoke_item(desc, id);
                }
            }
        }

        false
    }

    /// 处理键盘事件（完整覆盖 Left / Top / Flyout 方向键、Space/Enter、Esc）。
    pub fn handle_key_down(
        &mut self,
        desc: &NavigationViewDesc,
        layout: &NavigationViewLayout,
        chord: KeyChord,
    ) -> bool {
        let KeyName::Named(name) = chord.key else {
            return false;
        };

        // Esc 优先关闭 Flyout / 溢出菜单 / Overlay 浮层。
        if name == "Escape" {
            if let Some(parent_id) = self.open_flyout_parent_id.take() {
                self.focused_item_id = Some(parent_id);
                return true;
            }
            if self.overflow_menu_open {
                self.overflow_menu_open = false;
                self.focused_item_id = Some(NAVIGATION_VIEW_OVERFLOW_BUTTON_ID);
                return true;
            }
            if self.is_overlay_open(desc) {
                self.set_pane_open(false, desc);
                return true;
            }
            return false;
        }

        // 确定当前焦点导航范围（Flyout / 溢出菜单优先于主窗格）。
        let active_rows: &[NavigationViewRow] = if layout.flyout_rect.is_some() {
            &layout.flyout_rows
        } else if layout.overflow_popup_rect.is_some() {
            &layout.overflow_rows
        } else {
            &layout.rows
        };

        let focusable_ids: Vec<u64> = active_rows
            .iter()
            .filter(|row| row.is_enabled())
            .filter_map(NavigationViewRow::item_id)
            .collect();
        if focusable_ids.is_empty() {
            return false;
        }

        let in_flyout = layout.flyout_rect.is_some() || layout.overflow_popup_rect.is_some();
        let is_top_bar = layout.is_top && !in_flyout;

        let step: Option<isize> = match (is_top_bar, name) {
            (false, "ArrowUp") => Some(-1),
            (false, "ArrowDown") => Some(1),
            (true, "ArrowLeft") => Some(-1),
            (true, "ArrowRight") => Some(1),
            _ => None,
        };

        if let Some(delta) = step {
            let current_idx = self
                .focused_item_id
                .or(self.selected_item_id)
                .and_then(|id| focusable_ids.iter().position(|&cand| cand == id));
            let next_idx = match current_idx {
                Some(idx) => {
                    let len = focusable_ids.len() as isize;
                    ((idx as isize + delta).rem_euclid(len)) as usize
                }
                None => 0,
            };
            self.focused_item_id = Some(focusable_ids[next_idx]);
            return true;
        }

        if name == "Enter" || name == "Space" {
            let target = self
                .focused_item_id
                .filter(|id| focusable_ids.contains(id))
                .or_else(|| focusable_ids.first().copied());
            if let Some(id) = target {
                let invoked = self.invoke_item(desc, id);
                // Top 模式下展开子菜单 Flyout 时自动将焦点移入首项。
                if is_top_bar && self.open_flyout_parent_id == Some(id) {
                    if let Some(first_child) = desc.find_item(id).and_then(|parent| {
                        parent.children.iter().find_map(|c| match c {
                            NavigationViewMenuItem::Item(it) if it.is_enabled => Some(it.id),
                            _ => None,
                        })
                    }) {
                        self.focused_item_id = Some(first_child);
                    }
                }
                return invoked;
            }
        }

        false
    }
}

/// 导航视图的纯配置与布局描述符（对齐 WinUI `NavigationView` 全部公开属性）。
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationViewDesc {
    /// 整个导航壳占用的视口矩形。
    pub rect: Rect,
    /// 期望窗格显示模式。
    pub pane_display_mode: NavigationViewPaneDisplayMode,
    /// 展开窗格宽度（默认 320）。
    pub open_pane_length: f32,
    /// 紧凑图标轨宽度（默认 48）。
    pub compact_pane_length: f32,
    /// `Auto` 模式下进入 `Compact` 的最小窗口宽度（默认 640）。
    pub compact_mode_threshold_width: f32,
    /// `Auto` 模式下进入 `Expanded` 的最小窗口宽度（默认 1008）。
    pub expanded_mode_threshold_width: f32,
    /// 窗格初始或受控开关状态。
    pub is_pane_open: bool,
    /// 是否显示内置汉堡折叠按钮（托管给 `TitleBar` 时设为 `false`）。
    pub is_pane_toggle_button_visible: bool,
    /// 内置返回按钮可见性（托管给 `TitleBar` 时设为 `Collapsed`）。
    pub is_back_button_visible: NavigationViewBackButtonVisible,
    /// 返回按钮是否可点击。
    pub is_back_enabled: bool,
    /// 是否显示底部内置 `Settings` 项。
    pub is_settings_visible: bool,
    /// 是否启用自定义标题栏顶部自动留白避让。
    pub is_title_bar_auto_padding_enabled: bool,
    /// 非 Minimal 模式下是否始终显示顶部 `Header` 页标题区。
    pub always_show_header: bool,
    /// 顶部模式溢出按钮标签样式。
    pub overflow_label_mode: NavigationViewOverflowLabelMode,
    /// 窗格标题（紧邻汉堡按钮右侧显示）。
    pub pane_title: Option<String>,
    /// 窗格头部自定义标签（紧邻 `pane_title` 显示）。
    pub pane_header_label: Option<String>,
    /// 窗格中部自由内容区标签。
    pub pane_custom_content_label: Option<String>,
    /// 窗格底部自由内容区标签。
    pub pane_footer_label: Option<String>,
    /// 页面主标题（渲染在内容区顶部 52px 页头中）。
    pub header: Option<String>,
    /// 是否显示内置 `AutoSuggestBox` 搜索槽位。
    pub show_auto_suggest_box: bool,
    /// 搜索框占位文本。
    pub auto_suggest_placeholder: String,
    /// 主导航菜单项列表（支持 `Item` / `Header` / `Separator` 与多级嵌套）。
    pub menu_items: Vec<NavigationViewMenuItem>,
    /// 底部固定导航项列表（位于 `Settings` 之前）。
    pub footer_menu_items: Vec<NavigationViewMenuItem>,
    /// 初始或受控选中条目标识。
    pub selected_item_id: Option<u64>,
    /// 当前指针悬停条目标识。
    pub hovered_item_id: Option<u64>,
    /// Left 窗格菜单区滚动条可见性策略（默认 [`ScrollBarVisibility::Auto`]）。
    pub scrollbar_visibility: ScrollBarVisibility,
    /// 无障碍标签。
    pub label: String,
    /// 基础绘制深度。
    pub z: u16,
}

impl NavigationViewDesc {
    /// 创建默认 `NavigationViewDesc`（对齐 WinUI 3 官方默认值）。
    #[must_use]
    pub fn new(rect: Rect, menu_items: Vec<NavigationViewMenuItem>) -> Self {
        Self {
            rect,
            pane_display_mode: NavigationViewPaneDisplayMode::Auto,
            open_pane_length: NAVIGATION_VIEW_OPEN_PANE_LENGTH,
            compact_pane_length: NAVIGATION_VIEW_COMPACT_PANE_LENGTH,
            compact_mode_threshold_width: NAVIGATION_VIEW_COMPACT_THRESHOLD_WIDTH,
            expanded_mode_threshold_width: NAVIGATION_VIEW_EXPANDED_THRESHOLD_WIDTH,
            is_pane_open: true,
            is_pane_toggle_button_visible: true,
            is_back_button_visible: NavigationViewBackButtonVisible::Auto,
            is_back_enabled: false,
            is_settings_visible: true,
            is_title_bar_auto_padding_enabled: false,
            always_show_header: true,
            overflow_label_mode: NavigationViewOverflowLabelMode::MoreLabel,
            pane_title: None,
            pane_header_label: None,
            pane_custom_content_label: None,
            pane_footer_label: None,
            header: None,
            show_auto_suggest_box: false,
            auto_suggest_placeholder: "Search".to_owned(),
            menu_items,
            footer_menu_items: Vec::new(),
            selected_item_id: None,
            hovered_item_id: None,
            scrollbar_visibility: ScrollBarVisibility::Auto,
            label: "NavigationView".to_owned(),
            z: 610,
        }
    }

    /// 设置窗格显示模式。
    #[must_use]
    pub const fn pane_display_mode(mut self, mode: NavigationViewPaneDisplayMode) -> Self {
        self.pane_display_mode = mode;
        self
    }

    /// 设置展开窗格宽度。
    #[must_use]
    pub const fn open_pane_length(mut self, width: f32) -> Self {
        self.open_pane_length = width;
        self
    }

    /// 设置紧凑图标轨宽度。
    #[must_use]
    pub const fn compact_pane_length(mut self, width: f32) -> Self {
        self.compact_pane_length = width;
        self
    }

    /// 设置窗格开关态。
    #[must_use]
    pub const fn is_pane_open(mut self, open: bool) -> Self {
        self.is_pane_open = open;
        self
    }

    /// 设置内置汉堡按钮可见性。
    #[must_use]
    pub const fn is_pane_toggle_button_visible(mut self, visible: bool) -> Self {
        self.is_pane_toggle_button_visible = visible;
        self
    }

    /// 设置内置返回按钮可见性。
    #[must_use]
    pub const fn is_back_button_visible(
        mut self,
        visible: NavigationViewBackButtonVisible,
    ) -> Self {
        self.is_back_button_visible = visible;
        self
    }

    /// 设置返回按钮启用态。
    #[must_use]
    pub const fn is_back_enabled(mut self, enabled: bool) -> Self {
        self.is_back_enabled = enabled;
        self
    }

    /// 设置内置 `Settings` 项可见性。
    #[must_use]
    pub const fn is_settings_visible(mut self, visible: bool) -> Self {
        self.is_settings_visible = visible;
        self
    }

    /// 设置页头标题。
    #[must_use]
    pub fn header(mut self, header: impl Into<String>) -> Self {
        self.header = Some(header.into());
        self
    }

    /// 设置窗格标题。
    #[must_use]
    pub fn pane_title(mut self, title: impl Into<String>) -> Self {
        self.pane_title = Some(title.into());
        self
    }

    /// 设置顶部溢出按钮的标签形态（`More` 文字或纯三点图标）。
    #[must_use]
    pub const fn overflow_label_mode(mut self, mode: NavigationViewOverflowLabelMode) -> Self {
        self.overflow_label_mode = mode;
        self
    }

    /// 设置内置搜索框开关与占位符。
    #[must_use]
    pub fn with_auto_suggest_box(mut self, placeholder: impl Into<String>) -> Self {
        self.show_auto_suggest_box = true;
        self.auto_suggest_placeholder = placeholder.into();
        self
    }

    /// 设置底部固定导航项。
    #[must_use]
    pub fn footer_menu_items(mut self, items: Vec<NavigationViewMenuItem>) -> Self {
        self.footer_menu_items = items;
        self
    }

    /// 设置受控选中项。
    #[must_use]
    pub const fn selected_item(mut self, item_id: Option<u64>) -> Self {
        self.selected_item_id = item_id;
        self
    }

    /// 设置当前悬停项。
    #[must_use]
    pub const fn hovered_item(mut self, item_id: Option<u64>) -> Self {
        self.hovered_item_id = item_id;
        self
    }

    /// 设置 Left 窗格菜单区滚动条可见性策略。
    #[must_use]
    pub const fn scrollbar_visibility(mut self, visibility: ScrollBarVisibility) -> Self {
        self.scrollbar_visibility = visibility;
        self
    }

    /// 构造 Left 窗格菜单区的垂直按需叠加滚动条描述符。
    ///
    /// `Top` 模式（水平溢出走溢出按钮）与 `Minimal` 关闭态返回 `None`。
    #[must_use]
    pub fn menu_scrollbar_desc(
        &self,
        layout: &NavigationViewLayout,
        state: &NavigationViewState,
        disabled: bool,
    ) -> Option<ScrollBarDesc> {
        if layout.is_top
            || (layout.display_mode == NavigationViewDisplayMode::Minimal && !state.is_pane_open)
        {
            return None;
        }
        let active_z = if layout.overlay_rect.is_some() {
            self.z + 22
        } else {
            self.z + 2
        };
        // 对齐 WinUI 3 `MenuItemsScrollViewer` 铺满 `PaneContentGrid` 宽度：
        // 滚动条宿主右缘直接对齐 `active_pane_rect.max().x`，不再受 `NAVIGATION_VIEW_PANE_PADDING` 二次内缩。
        let active_pane_rect = layout.overlay_rect.unwrap_or(layout.pane_rect);
        let scrollbar_host_rect = Rect::new(
            active_pane_rect.origin.x,
            layout.menu_viewport_rect.origin.y,
            active_pane_rect.width(),
            layout.menu_viewport_rect.height(),
        );
        let mut bar = ScrollBarDesc::vertical(
            scrollbar_host_rect,
            layout.menu_content_height,
            state.scroll_offset,
        );
        // 紧凑折叠态（`pane_expand_progress <= 0.01`）在 `Auto` 策略下默认隐藏滚动条，
        // 保持 48px 图标轨洁净；仅在指针悬停窗格/滚动条、正在拖拽或近期滚轮滚动时按需显现。
        let is_compact_collapsed = layout.pane_expand_progress <= 0.01;
        let compact_revealed = state.pane_hovered
            || state.scrollbar_hovered
            || state.scrollbar_drag.is_some()
            || state.scrollbar_recent_scroll_timer > 0.0;
        bar.visibility = match self.scrollbar_visibility {
            ScrollBarVisibility::Auto if is_compact_collapsed && !compact_revealed => {
                ScrollBarVisibility::Hidden
            }
            other => other,
        };
        bar.z = active_z + 6;
        bar.layer = CONTROL_LAYER;
        bar.hovered = state.scrollbar_hovered;
        bar.dragging = state.scrollbar_drag.is_some();
        bar.disabled = disabled;
        Some(bar)
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 根据视口宽度解析当前实际生效的 [`NavigationViewDisplayMode`]。
    #[must_use]
    pub fn resolve_display_mode(&self, viewport_width: f32) -> NavigationViewDisplayMode {
        match self.pane_display_mode {
            NavigationViewPaneDisplayMode::Left | NavigationViewPaneDisplayMode::Top => {
                NavigationViewDisplayMode::Expanded
            }
            NavigationViewPaneDisplayMode::LeftCompact => NavigationViewDisplayMode::Compact,
            NavigationViewPaneDisplayMode::LeftMinimal => NavigationViewDisplayMode::Minimal,
            NavigationViewPaneDisplayMode::Auto => {
                if viewport_width >= self.expanded_mode_threshold_width {
                    NavigationViewDisplayMode::Expanded
                } else if viewport_width > self.compact_mode_threshold_width {
                    NavigationViewDisplayMode::Compact
                } else {
                    NavigationViewDisplayMode::Minimal
                }
            }
        }
    }

    /// 在主菜单树与页脚中递归查找指定 ID 的导航项。
    #[must_use]
    pub fn find_item(&self, item_id: u64) -> Option<&NavigationViewItem> {
        fn search(items: &[NavigationViewMenuItem], target: u64) -> Option<&NavigationViewItem> {
            for entry in items {
                if let NavigationViewMenuItem::Item(item) = entry {
                    if item.id == target {
                        return Some(item);
                    }
                    if let Some(found) = search(&item.children, target) {
                        return Some(found);
                    }
                }
            }
            None
        }
        search(&self.menu_items, item_id).or_else(|| search(&self.footer_menu_items, item_id))
    }

    /// 返回从根到目标项的祖先标识链（包含目标自身），供折叠态祖先选中回退使用。
    #[must_use]
    pub fn ancestor_chain(&self, target_id: u64) -> Option<Vec<u64>> {
        fn walk(items: &[NavigationViewMenuItem], target: u64, path: &mut Vec<u64>) -> bool {
            for entry in items {
                if let NavigationViewMenuItem::Item(item) = entry {
                    path.push(item.id);
                    if item.id == target || walk(&item.children, target, path) {
                        return true;
                    }
                    path.pop();
                }
            }
            false
        }
        let mut path = Vec::new();
        if walk(&self.menu_items, target_id, &mut path) {
            Some(path)
        } else {
            None
        }
    }

    /// 求解完整导航壳的几何布局（支持全部 5 种 `PaneDisplayMode`、层级、滚动、溢出与 Flyout）。
    #[must_use]
    pub fn compute_layout(&self, state: &NavigationViewState) -> NavigationViewLayout {
        let mut layout = NavigationViewLayout::default();
        let rect = self.rect;
        let is_top = self.pane_display_mode == NavigationViewPaneDisplayMode::Top;
        let display_mode = self.resolve_display_mode(rect.width());
        layout.display_mode = display_mode;
        layout.is_top = is_top;

        let top_pad = if self.is_title_bar_auto_padding_enabled {
            32.0
        } else {
            0.0
        };
        let back_visible = match self.is_back_button_visible {
            NavigationViewBackButtonVisible::Collapsed => false,
            NavigationViewBackButtonVisible::Visible | NavigationViewBackButtonVisible::Auto => {
                true
            }
        };

        if is_top {
            self.layout_top_mode(state, &mut layout, top_pad, back_visible);
            return layout;
        }

        let is_overlay = state.is_pane_open
            && matches!(
                display_mode,
                NavigationViewDisplayMode::Compact | NavigationViewDisplayMode::Minimal
            );
        layout.is_overlay = is_overlay;

        let base_pane_w = match display_mode {
            NavigationViewDisplayMode::Expanded => {
                if state.animations_enabled {
                    state
                        .pane_width_spring
                        .value
                        .clamp(self.compact_pane_length, self.open_pane_length)
                } else if state.is_pane_open {
                    self.open_pane_length
                } else {
                    self.compact_pane_length
                }
            }
            // Compact 展开时窗格是浮在内容之上的覆盖层，内容区不为其让位（WinUI 实测如此）；
            // 折叠态才占 48px 图标轨。
            NavigationViewDisplayMode::Compact => {
                if state.is_pane_open {
                    0.0
                } else {
                    self.compact_pane_length
                }
            }
            NavigationViewDisplayMode::Minimal => 0.0,
        }
        .min(rect.width().max(0.0));

        layout.pane_rect = Rect::new(
            rect.origin.x,
            rect.origin.y + top_pad,
            base_pane_w,
            (rect.height() - top_pad).max(0.0),
        );

        let active_pane_rect = if is_overlay {
            let progress = if state.animations_enabled {
                state.overlay_progress_spring.value.clamp(0.0, 1.0)
            } else {
                1.0
            };
            let overlay_w = (self.open_pane_length * progress).clamp(
                self.compact_pane_length,
                rect.width().max(self.compact_pane_length),
            );
            let ov = Rect::new(
                rect.origin.x,
                rect.origin.y + top_pad,
                overlay_w,
                (rect.height() - top_pad).max(0.0),
            );
            layout.overlay_rect = Some(ov);
            ov
        } else {
            layout.pane_rect
        };

        // 顶部控制按钮（返回键 + 汉堡键）。
        let mut cursor_y = rect.origin.y + top_pad + NAVIGATION_VIEW_PANE_PADDING;
        let header_y = cursor_y + NAVIGATION_VIEW_PANE_HEADER_TOP_INSET;
        if display_mode == NavigationViewDisplayMode::Minimal && !state.is_pane_open {
            let mut btn_x = rect.origin.x + NAVIGATION_VIEW_PANE_PADDING;
            if back_visible {
                layout.back_button_rect = Some(Rect::new(btn_x, header_y, 40.0, 36.0));
                btn_x += 40.0;
            }
            if self.is_pane_toggle_button_visible {
                layout.pane_toggle_rect = Some(Rect::new(btn_x, header_y, 40.0, 36.0));
            }
        } else {
            if back_visible {
                // 返回键贴窗格左缘（参考图箭头中心 x=36.5，即控件原点 + 20.5），
                // 与汉堡键一样宽 40，但不再叠加 `NAVIGATION_VIEW_PANE_PADDING`。
                layout.back_button_rect =
                    Some(Rect::new(active_pane_rect.origin.x, header_y, 40.0, 36.0));
                cursor_y += 40.0;
            }
            if self.is_pane_toggle_button_visible {
                layout.pane_toggle_rect = Some(Rect::new(
                    active_pane_rect.origin.x + NAVIGATION_VIEW_PANE_PADDING,
                    cursor_y + NAVIGATION_VIEW_PANE_HEADER_TOP_INSET,
                    40.0,
                    36.0,
                ));
                cursor_y += 40.0;
            }
        }

        // 右侧页头与圆角内容区容器。
        let content_x = rect.origin.x + base_pane_w;
        let content_w = (rect.width() - base_pane_w).max(0.0);
        let show_header = self.header.is_some()
            && (self.always_show_header || display_mode == NavigationViewDisplayMode::Minimal);
        let header_h = if show_header {
            NAVIGATION_VIEW_HEADER_HEIGHT
        } else {
            0.0
        };
        // WinUI 的页头标题画在内容卡片**内部**顶端，卡片本身仍从控件顶部铺满。
        let content_y = rect.origin.y + top_pad;
        let content_h = (rect.height() - top_pad).max(0.0);
        layout.content_rect = Rect::new(content_x, content_y, content_w, content_h);
        if show_header {
            layout.header_rect = Some(Rect::new(
                content_x,
                content_y + NAVIGATION_VIEW_CONTENT_HEADER_INSET_Y,
                content_w,
                header_h,
            ));
        }
        let pad = if display_mode == NavigationViewDisplayMode::Minimal {
            12.0
        } else {
            24.0
        };
        layout.content_inner_rect = Rect::new(
            content_x + pad,
            content_y + pad,
            (content_w - pad * 2.0).max(0.0),
            (content_h - pad * 2.0).max(0.0),
        );

        // Minimal 关闭态不排布窗格内行。
        if display_mode == NavigationViewDisplayMode::Minimal && !state.is_pane_open {
            layout.pane_expand_progress = 0.0;
            return layout;
        }

        // 依据实际窗格宽度计算动画展开进度 `t`，避免折叠第 0 帧按布尔开关硬切布局。
        let span = self.open_pane_length - self.compact_pane_length;
        let pane_expand_progress = if span > f32::EPSILON {
            ((active_pane_rect.width() - self.compact_pane_length) / span).clamp(0.0, 1.0)
        } else if state.is_pane_open {
            1.0
        } else {
            0.0
        };
        layout.pane_expand_progress = pane_expand_progress;
        let pane_is_collapsed = pane_expand_progress <= 0.01;
        let inner_x = active_pane_rect.origin.x + NAVIGATION_VIEW_PANE_PADDING;
        let inner_w = (active_pane_rect.width() - NAVIGATION_VIEW_PANE_PADDING * 2.0).max(1.0);

        // 可选 AutoSuggestBox 搜索槽位（随窗格宽度平滑过渡，避免第 0 帧瞬切）。
        if self.show_auto_suggest_box {
            let search_h = 32.0;
            let inset_x = NAVIGATION_VIEW_SEARCH_INSET_X * pane_expand_progress;
            let search_x = inner_x + inset_x;
            let search_w = if pane_is_collapsed {
                40.0_f32.min(inner_w)
            } else {
                (inner_w - inset_x * 2.0).max(40.0_f32.min(inner_w))
            };
            layout.search_rect = Some(Rect::new(
                search_x,
                cursor_y + NAVIGATION_VIEW_SEARCH_INSET_Y,
                search_w,
                search_h,
            ));
            cursor_y += search_h + NAVIGATION_VIEW_SEARCH_GAP_AFTER;
        }

        // 计算吸底页脚高度（FooterMenuItems + 可选 Settings）。
        let footer_count = self.footer_menu_items.len() + usize::from(self.is_settings_visible);
        let pitch = NAVIGATION_VIEW_ITEM_HEIGHT + NAVIGATION_VIEW_ITEM_MARGIN_Y * 2.0;
        let footer_h = if footer_count == 0 {
            0.0
        } else {
            8.0 + footer_count as f32 * pitch
        };
        let footer_top =
            (active_pane_rect.max().y - NAVIGATION_VIEW_PANE_PADDING - footer_h).max(cursor_y);
        layout.footer_rect = Rect::new(inner_x, footer_top, inner_w, footer_h);
        layout.menu_viewport_rect =
            Rect::new(inner_x, cursor_y, inner_w, (footer_top - cursor_y).max(0.0));

        // 递归展开主菜单项并应用垂直滚动偏移。
        let mut content_y_cursor = 0.0_f32;
        let mut flat_main: Vec<(NavigationViewRowKind, f32, f32)> = Vec::new();
        self.flatten_menu_items(
            &self.menu_items,
            0,
            pane_is_collapsed,
            state,
            &mut content_y_cursor,
            &mut flat_main,
        );
        layout.menu_content_height = content_y_cursor;

        let scroll = state.scroll_offset.clamp(
            0.0,
            (content_y_cursor - layout.menu_viewport_rect.height()).max(0.0),
        );

        for (kind, local_y, h) in flat_main {
            let row_y = layout.menu_viewport_rect.origin.y + local_y - scroll;
            // WinUI 的分组分隔线横跨整个窗格，不跟随条目内边距。
            let row_r = if matches!(kind, NavigationViewRowKind::Separator { .. }) {
                Rect::new(
                    active_pane_rect.origin.x,
                    row_y,
                    active_pane_rect.width(),
                    h,
                )
            } else {
                Rect::new(inner_x, row_y, inner_w, h)
            };
            if row_r.max().y > layout.menu_viewport_rect.origin.y
                && row_r.origin.y < layout.menu_viewport_rect.max().y
            {
                layout.rows.push(NavigationViewRow { kind, rect: row_r });
            }
        }

        // 吸底固定页脚区（不随主列表滚动）。
        let mut fy = footer_top + 8.0;
        for entry in &self.footer_menu_items {
            let row_r = Rect::new(
                inner_x,
                fy + NAVIGATION_VIEW_ITEM_MARGIN_Y,
                inner_w,
                NAVIGATION_VIEW_ITEM_HEIGHT,
            );
            match entry {
                NavigationViewMenuItem::Item(item) => {
                    layout.rows.push(NavigationViewRow {
                        kind: NavigationViewRowKind::FooterItem { item: item.clone() },
                        rect: row_r,
                    });
                }
                NavigationViewMenuItem::Separator(sep) => {
                    layout.rows.push(NavigationViewRow {
                        kind: NavigationViewRowKind::Separator {
                            opacity: sep.opacity,
                        },
                        rect: Rect::new(inner_x, fy, inner_w, NAVIGATION_VIEW_SEPARATOR_HEIGHT),
                    });
                }
                NavigationViewMenuItem::Header(hdr) => {
                    if !pane_is_collapsed {
                        layout.rows.push(NavigationViewRow {
                            kind: NavigationViewRowKind::Header {
                                content: hdr.content.clone(),
                            },
                            rect: Rect::new(
                                inner_x,
                                fy,
                                inner_w,
                                NAVIGATION_VIEW_SECTION_HEADER_HEIGHT,
                            ),
                        });
                    }
                }
            }
            fy += pitch;
        }
        if self.is_settings_visible {
            layout.rows.push(NavigationViewRow {
                kind: NavigationViewRowKind::Settings,
                rect: Rect::new(
                    inner_x,
                    fy + NAVIGATION_VIEW_ITEM_MARGIN_Y,
                    inner_w,
                    NAVIGATION_VIEW_ITEM_HEIGHT,
                ),
            });
        }

        // Compact 折叠态子菜单 Flyout 定位（防越界翻转）。
        if !state.is_pane_open {
            if let Some(parent_id) = state.open_flyout_parent_id {
                self.layout_submenu_flyout(parent_id, Placement::RightStart, &mut layout);
            }
        }

        layout
    }

    /// 递归扁平化主菜单项。
    fn flatten_menu_items(
        &self,
        items: &[NavigationViewMenuItem],
        depth: usize,
        pane_is_narrow: bool,
        state: &NavigationViewState,
        cursor_y: &mut f32,
        out: &mut Vec<(NavigationViewRowKind, f32, f32)>,
    ) {
        let pitch = NAVIGATION_VIEW_ITEM_HEIGHT + NAVIGATION_VIEW_ITEM_MARGIN_Y * 2.0;
        for entry in items {
            match entry {
                NavigationViewMenuItem::Header(hdr) => {
                    if !pane_is_narrow {
                        out.push((
                            NavigationViewRowKind::Header {
                                content: hdr.content.clone(),
                            },
                            *cursor_y,
                            NAVIGATION_VIEW_SECTION_HEADER_HEIGHT,
                        ));
                        *cursor_y += NAVIGATION_VIEW_SECTION_HEADER_HEIGHT;
                    }
                }
                NavigationViewMenuItem::Separator(sep) => {
                    out.push((
                        NavigationViewRowKind::Separator {
                            opacity: sep.opacity,
                        },
                        *cursor_y,
                        NAVIGATION_VIEW_SEPARATOR_HEIGHT,
                    ));
                    *cursor_y += NAVIGATION_VIEW_SEPARATOR_HEIGHT;
                }
                NavigationViewMenuItem::Item(item) => {
                    let expanded =
                        !pane_is_narrow && item.has_children() && state.is_item_expanded(item.id);
                    out.push((
                        NavigationViewRowKind::Item {
                            item: item.clone(),
                            depth,
                            is_expanded: expanded,
                        },
                        *cursor_y + NAVIGATION_VIEW_ITEM_MARGIN_Y,
                        NAVIGATION_VIEW_ITEM_HEIGHT,
                    ));
                    *cursor_y += pitch;
                    if expanded {
                        self.flatten_menu_items(
                            &item.children,
                            depth + 1,
                            pane_is_narrow,
                            state,
                            cursor_y,
                            out,
                        );
                    }
                }
            }
        }
    }

    /// `Top` 顶部水平导航模式布局（含自动溢出 `...` 按钮与下拉菜单及子菜单 Flyout）。
    fn layout_top_mode(
        &self,
        state: &NavigationViewState,
        layout: &mut NavigationViewLayout,
        top_pad: f32,
        back_visible: bool,
    ) {
        let rect = self.rect;
        let bar_y = rect.origin.y + top_pad;
        layout.pane_expand_progress = 1.0;
        layout.pane_rect = Rect::new(
            rect.origin.x,
            bar_y,
            rect.width(),
            NAVIGATION_VIEW_TOP_PANE_HEIGHT,
        );

        let mut x = rect.origin.x + 8.0;
        let row_y = bar_y + (NAVIGATION_VIEW_TOP_PANE_HEIGHT - NAVIGATION_VIEW_ITEM_HEIGHT) * 0.5;
        if back_visible {
            layout.back_button_rect = Some(Rect::new(x, row_y, 40.0, NAVIGATION_VIEW_ITEM_HEIGHT));
            x += 44.0;
        }

        // 右侧预留搜索框、页脚与 Settings 宽度。
        let mut right_limit = rect.max().x - 8.0;
        if self.is_settings_visible {
            // 顶部模式的 Settings 是「齿轮 + 文字」整块条目（12 + 16 + 8 + 50 + 12 ≈ 98），
            // 不是左侧轨道的纯图标按钮。
            let settings_w = NAVIGATION_VIEW_TOP_FOOTER_WIDTH;
            let settings_r = Rect::new(
                right_limit - settings_w,
                row_y,
                settings_w,
                NAVIGATION_VIEW_ITEM_HEIGHT,
            );
            layout.rows.push(NavigationViewRow {
                kind: NavigationViewRowKind::Settings,
                rect: settings_r,
            });
            right_limit -= settings_w + 4.0;
        }
        if self.show_auto_suggest_box && right_limit - x > 220.0 {
            let sw = 180.0;
            layout.search_rect = Some(Rect::new(right_limit - sw, bar_y + 8.0, sw, 32.0));
            right_limit -= sw + 8.0;
        }

        let overflow_reserve = match self.overflow_label_mode {
            NavigationViewOverflowLabelMode::MoreLabel => 76.0,
            NavigationViewOverflowLabelMode::NoLabel => 44.0,
        };
        layout.menu_viewport_rect = Rect::new(
            x,
            row_y,
            (right_limit - x).max(0.0),
            NAVIGATION_VIEW_ITEM_HEIGHT,
        );

        let mut overflow_items: Vec<NavigationViewItem> = Vec::new();
        let estimate = |item: &NavigationViewItem| -> f32 {
            let chars = item.content.chars().count() as f32;
            if item.icon.is_some() {
                // WinUI 顶部带图标项：左 12 + 图标 16 + 间距 8 + 文字估算 + 右 13（"Home" 恰为 85px）。
                (chars * 9.0 + 49.0).clamp(72.0, 180.0)
            } else {
                (chars * 9.0 + 24.0).clamp(56.0, 180.0)
            }
        };
        let items: Vec<&NavigationViewItem> = self
            .menu_items
            .iter()
            .filter_map(|entry| match entry {
                NavigationViewMenuItem::Item(item) => Some(item),
                _ => None,
            })
            .collect();
        let total_w = items.iter().map(|item| estimate(item) + 4.0).sum::<f32>();
        if !items.is_empty() && x + total_w - 4.0 <= right_limit {
            // 全部条目都放得下：顺序平铺，不出现溢出按钮。
            for item in &items {
                let est_w = estimate(item);
                layout.rows.push(NavigationViewRow {
                    kind: NavigationViewRowKind::Item {
                        item: (*item).clone(),
                        depth: 0,
                        is_expanded: state.open_flyout_parent_id == Some(item.id),
                    },
                    rect: Rect::new(x, row_y, est_w, NAVIGATION_VIEW_ITEM_HEIGHT),
                });
                x += est_w + 4.0;
            }
        } else if !items.is_empty() {
            // WinUI 顶部模式放不下时只保留选中项（保证选中项始终可见），其余收进溢出菜单。
            let selected_id = state.effective_selected_id(self);
            let keep = items
                .iter()
                .position(|item| Some(item.id) == selected_id)
                .unwrap_or(0);
            for (index, item) in items.iter().enumerate() {
                if index == keep {
                    let est_w = estimate(item);
                    layout.rows.push(NavigationViewRow {
                        kind: NavigationViewRowKind::Item {
                            item: (*item).clone(),
                            depth: 0,
                            is_expanded: state.open_flyout_parent_id == Some(item.id),
                        },
                        rect: Rect::new(x, row_y, est_w, NAVIGATION_VIEW_ITEM_HEIGHT),
                    });
                    x += est_w + 4.0;
                } else {
                    overflow_items.push((*item).clone());
                }
            }
        }

        if !overflow_items.is_empty() {
            let ov_btn = Rect::new(
                x.min((right_limit - overflow_reserve).max(rect.origin.x + 8.0)),
                row_y,
                overflow_reserve,
                NAVIGATION_VIEW_ITEM_HEIGHT,
            );
            layout.overflow_button_rect = Some(ov_btn);
            if state.overflow_menu_open {
                let popup_w = 200.0;
                let popup_h = overflow_items.len() as f32 * NAVIGATION_VIEW_ITEM_HEIGHT + 8.0;
                let placed = place_anchored_popup_detailed(
                    ov_btn,
                    Vec2::new(popup_w, popup_h),
                    rect,
                    Placement::BottomEnd,
                );
                layout.overflow_popup_rect = Some(placed.rect);
                for (idx, item) in overflow_items.into_iter().enumerate() {
                    layout.overflow_rows.push(NavigationViewRow {
                        kind: NavigationViewRowKind::Item {
                            item,
                            depth: 0,
                            is_expanded: false,
                        },
                        rect: Rect::new(
                            placed.rect.origin.x + 4.0,
                            placed.rect.origin.y + 4.0 + idx as f32 * NAVIGATION_VIEW_ITEM_HEIGHT,
                            popup_w - 8.0,
                            NAVIGATION_VIEW_ITEM_HEIGHT,
                        ),
                    });
                }
            }
        }

        // Top 模式下子菜单 Flyout。
        if let Some(parent_id) = state.open_flyout_parent_id {
            self.layout_submenu_flyout(parent_id, Placement::BottomStart, layout);
        }

        // 下方页头与内容区。
        let show_header = self.header.is_some() && self.always_show_header;
        let header_h = if show_header {
            NAVIGATION_VIEW_HEADER_HEIGHT
        } else {
            0.0
        };
        let after_bar_y = bar_y + NAVIGATION_VIEW_TOP_PANE_HEIGHT;
        if show_header {
            layout.header_rect = Some(Rect::new(
                rect.origin.x,
                after_bar_y,
                rect.width(),
                header_h,
            ));
        }
        let content_y = after_bar_y + header_h;
        let content_h = (rect.max().y - content_y).max(0.0);
        layout.content_rect = Rect::new(rect.origin.x, content_y, rect.width(), content_h);
        layout.content_inner_rect = Rect::new(
            rect.origin.x + 24.0,
            content_y + 24.0,
            (rect.width() - 48.0).max(0.0),
            (content_h - 48.0).max(0.0),
        );
    }

    /// 计算子菜单 Flyout 的防越界矩形及内部子行。
    fn layout_submenu_flyout(
        &self,
        parent_id: u64,
        preferred: Placement,
        layout: &mut NavigationViewLayout,
    ) {
        let Some(parent) = self.find_item(parent_id) else {
            return;
        };
        if parent.children.is_empty() {
            return;
        }
        let Some(anchor) = layout
            .rows
            .iter()
            .find(|row| row.item_id() == Some(parent_id))
            .map(|row| row.rect)
        else {
            return;
        };
        let child_items: Vec<&NavigationViewItem> = parent
            .children
            .iter()
            .filter_map(|m| match m {
                NavigationViewMenuItem::Item(it) => Some(it),
                _ => None,
            })
            .collect();
        if child_items.is_empty() {
            return;
        }
        let flyout_w = 216.0;
        let flyout_h = child_items.len() as f32 * NAVIGATION_VIEW_ITEM_HEIGHT + 8.0;
        let placed = place_anchored_popup_detailed(
            anchor,
            Vec2::new(flyout_w, flyout_h),
            self.rect,
            preferred,
        );
        layout.flyout_rect = Some(placed.rect);
        for (idx, item) in child_items.into_iter().enumerate() {
            layout.flyout_rows.push(NavigationViewRow {
                kind: NavigationViewRowKind::Item {
                    item: item.clone(),
                    depth: 0,
                    is_expanded: false,
                },
                rect: Rect::new(
                    placed.rect.origin.x + 4.0,
                    placed.rect.origin.y + 4.0 + idx as f32 * NAVIGATION_VIEW_ITEM_HEIGHT,
                    flyout_w - 8.0,
                    NAVIGATION_VIEW_ITEM_HEIGHT,
                ),
            });
        }
    }
}

/// 生成导航行的确定性子控件节点 ID。
#[must_use]
pub const fn navigation_view_row_node(node: u64, item_id: u64) -> u64 {
    node.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ item_id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// 发射完整的 `NavigationView`（图元、载荷、命中区、AccessKit 语义树与局部毛玻璃请求）。
///
/// 在固定 `desc` 与 `state` 下，跨 24 种 [`ControlState`] 组合产出完全恒定的图元数量。
#[must_use]
pub fn emit_navigation_view(
    node: u64,
    desc: &NavigationViewDesc,
    control: ControlState,
    state: &NavigationViewState,
    text_ctx: &mut TextCtx<'_>,
) -> NavigationViewOutput {
    let layout = desc.compute_layout(state);
    let mut out = WidgetOutput::default();
    let effective_selected = state.effective_selected_id(desc);
    let hovered_id = state.hovered_item_id.or(desc.hovered_item_id);
    let disabled = control.disabled;
    let z = desc.z;

    // 1. 窗格背景：Left 常驻与 Top 保持透明以透出窗口级原生 Mica，绝不铺实色遮挡后景。
    out.primitives
        .push(surface(layout.pane_rect, 0.0, Token::NONE, z));

    // 2. 右侧/下方内容区圆角表面（对齐 Windows 11 LayerFillColorDefault + 1px 描边）。
    let content_radius = if layout.is_top {
        0.0
    } else {
        NAVIGATION_VIEW_CONTENT_CORNER_RADIUS
    };
    out.primitives.push(outlined(
        layout.content_rect,
        content_radius,
        Token::NAV_CONTENT_BG,
        Token::NAV_CONTENT_STROKE,
        1.0,
        0.0,
        z,
    ));

    // 3. 顶部页头标题（若存在）。
    if let (Some(header_r), Some(header_text)) = (layout.header_rect, desc.header.as_deref()) {
        let inset_x = if layout.is_top {
            24.0
        } else {
            NAVIGATION_VIEW_CONTENT_HEADER_INSET_X
        };
        let title_r = Rect::new(
            header_r.origin.x + inset_x,
            header_r.origin.y,
            (header_r.width() - inset_x * 2.0).max(0.0),
            header_r.height(),
        );
        let quads = text_ctx.shape_line_styled(
            header_text,
            title_r,
            28.0,
            600,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        glyphs(
            &quads,
            if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            },
            z + 1,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 4. Overlay 浮层背景（原生 In-App Acrylic：Dual-Kawase BackdropRequest + 半透明叠色 + 描边 + 阴影）。
    let mut backdrop = None;
    let active_z = if let Some(ov_r) = layout.overlay_rect {
        backdrop = BackdropRequest::new(node, ov_r, NAVIGATION_VIEW_OVERLAY_BLUR);
        // 阴影单独一层并向右下偏移 2px：WinUI 的 `ThemeShadow` 只在下/右侧可见，
        // 左侧与顶部被控件边界裁掉。居中的模糊阴影会向上溢到 (10,10)，
        // 让内容包围盒比参考图大 5~6px，直接卡住包围盒门禁。
        out.primitives.push(instance(
            Rect::new(
                ov_r.origin.x + NAVIGATION_VIEW_OVERLAY_SHADOW_OFFSET_X,
                ov_r.origin.y + NAVIGATION_VIEW_OVERLAY_SHADOW_OFFSET_Y,
                ov_r.width(),
                ov_r.height(),
            ),
            [NAVIGATION_VIEW_CONTENT_CORNER_RADIUS; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            NAVIGATION_VIEW_OVERLAY_SHADOW_BLUR,
            PrimType::Shadow,
            SubElement::Shadow,
            z + 19,
        ));
        out.primitives.push(outlined(
            ov_r,
            NAVIGATION_VIEW_CONTENT_CORNER_RADIUS,
            Token::OVERLAY_BG,
            Token::CARD_BORDER,
            1.0,
            0.0,
            z + 20,
        ));
        z + 22
    } else {
        z + 2
    };

    let active_pane_rect = layout.overlay_rect.unwrap_or(layout.pane_rect);
    let pane_right_edge = active_pane_rect.max().x;
    let pane_t = if layout.is_top {
        1.0
    } else {
        layout.pane_expand_progress
    };

    // 5. 内置返回按钮与汉堡折叠按钮。
    if let Some(back_r) = layout.back_button_rect {
        let back_enabled = !disabled && (desc.is_back_enabled || layout.is_overlay);
        let back_hovered = back_enabled && hovered_id == Some(NAVIGATION_VIEW_BACK_BUTTON_ID);
        out.primitives.push(surface(
            back_r,
            4.0,
            if back_hovered {
                Token::NAV_HOVER
            } else {
                Token::NONE
            },
            active_z,
        ));
        // WinUI 3 的返回键是纯 v 形箭头（参考图墨迹仅 6x10px，无长杆），
        // `\u{E72B}` 在多数图标字体里带长杆，字形与参考不符，故改画箭头几何。
        emit_shell_icon_primitives(
            ShellIcon::Back,
            back_r,
            if back_enabled {
                Token::TEXT_PRIMARY
            } else {
                Token::TEXT_DISABLED
            },
            Token::NONE,
            active_z + 1,
            &mut out.primitives,
            &mut out.payloads,
        );
        let key = navigation_view_row_node(node, NAVIGATION_VIEW_BACK_BUTTON_ID);
        let mut a11y = A11yNode::new(Role::Button, "Back", back_r)
            .with_actions(A11yActions::single(A11yAction::Click));
        a11y.disabled = !back_enabled;
        a11y.focusable = back_enabled;
        out.with_sub_control(
            key,
            HitRegion::ui(key, u32::from(active_z) + 2, back_r).with_cursor(CursorKind::Pointer),
            a11y,
            back_enabled,
        );
    }

    if let Some(toggle_r) = layout.pane_toggle_rect {
        let toggle_hovered = !disabled && hovered_id == Some(NAVIGATION_VIEW_PANE_TOGGLE_ID);
        out.primitives.push(surface(
            toggle_r,
            4.0,
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else if toggle_hovered {
                Token::NAV_HOVER
            } else {
                Token::NONE
            },
            active_z,
        ));
        let hamburger_x = toggle_r.origin.x + (toggle_r.width() - 14.0) * 0.5;
        let base_y = toggle_r.origin.y + (toggle_r.height() - 10.0) * 0.5;
        let bar_ink = if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        for dy in [0.0_f32, 4.5, 9.0] {
            out.primitives.push(surface(
                Rect::new(hamburger_x, base_y + dy, 14.0, 1.0),
                0.0,
                bar_ink,
                active_z + 1,
            ));
        }
        // 紧邻汉堡按钮右侧绘制 PaneTitle（随窗格展开进度平滑淡出/淡入并受右缘裁剪）。
        if !layout.is_top && pane_t > 0.01 {
            if let Some(title) = desc.pane_title.as_deref() {
                let title_x = toggle_r.max().x + NAVIGATION_VIEW_PANE_TITLE_INSET_X;
                let max_right = (pane_right_edge - NAVIGATION_VIEW_PANE_PADDING).max(title_x);
                let pt_r = Rect::new(
                    title_x,
                    toggle_r.origin.y,
                    (desc.open_pane_length - 64.0).max(0.0),
                    toggle_r.height(),
                );
                let pt_quads = text_ctx.shape_line_styled(
                    title,
                    pt_r,
                    14.0,
                    600,
                    None,
                    AlignX::Left,
                    AlignY::Middle,
                );
                let clipped = clip_quads_to_right_edge(&pt_quads, max_right);
                let start = out.primitives.len();
                glyphs(
                    &clipped,
                    bar_ink,
                    active_z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                apply_opacity_since(&mut out.primitives, start, pane_t);
            }
        }
        let key = navigation_view_row_node(node, NAVIGATION_VIEW_PANE_TOGGLE_ID);
        let mut a11y = A11yNode::new(Role::Button, "Toggle navigation", toggle_r)
            .expanded(state.is_pane_open)
            .with_actions(A11yActions::single(A11yAction::Click));
        a11y.disabled = disabled;
        a11y.focusable = !disabled;
        out.with_sub_control(
            key,
            HitRegion::ui(key, u32::from(active_z) + 2, toggle_r).with_cursor(CursorKind::Pointer),
            a11y,
            !disabled,
        );
    }

    // 6. 可选 AutoSuggestBox 搜索槽位（搜索图标锚定在左侧 48px 紧凑列固定偏移，0px 位移）。
    if let Some(search_r) = layout.search_rect {
        let is_compact_icon = !layout.is_top && pane_t <= 0.01;
        let box_start = out.primitives.len();
        out.primitives.push(outlined(
            search_r,
            4.0,
            if is_compact_icon {
                Token::NONE
            } else {
                Token::INPUT_BG
            },
            if is_compact_icon {
                Token::NONE
            } else if state.search_focused {
                Token::PRIMARY
            } else {
                // WinUI 的 TextBox 描边是 `TextControlElevationBorderBrush`（顶浅底深的
                // 竖向渐变），统一形状由 `CONTROL_STROKE` 承担；用 `CARD_BORDER`
                // 会得到四边同色的深灰框，与参考图顶 229 / 底 134 不符。
                Token::CONTROL_STROKE
            },
            if is_compact_icon { 0.0 } else { 1.0 },
            0.0,
            active_z,
        ));
        if !is_compact_icon && !layout.is_top {
            apply_opacity_since(&mut out.primitives, box_start, pane_t);
        }
        // 紧凑态及动画过渡期发射搜索图标，始终锚定在左侧 48px 紧凑列固定偏移（inner_x + 12.0）。
        if !layout.is_top && pane_t < 0.99 {
            let fixed_icon_x = active_pane_rect.origin.x + NAVIGATION_VIEW_PANE_PADDING + 12.0;
            let icon_r = Rect::new(
                fixed_icon_x,
                search_r.origin.y + (search_r.height() - 16.0) * 0.5,
                16.0,
                16.0,
            );
            let icon_start = out.primitives.len();
            emit_shell_icon_primitives(
                ShellIcon::Search,
                icon_r,
                if disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_PRIMARY
                },
                Token::SURFACE_RAISED,
                active_z + 1,
                &mut out.primitives,
                &mut out.payloads,
            );
            let icon_opacity = if is_compact_icon { 1.0 } else { 1.0 - pane_t };
            apply_opacity_since(&mut out.primitives, icon_start, icon_opacity);
        }
        if !is_compact_icon {
            let label = if state.search_query.is_empty() {
                desc.auto_suggest_placeholder.as_str()
            } else {
                state.search_query.as_str()
            };
            let text_r = Rect::new(
                search_r.origin.x + 12.0,
                search_r.origin.y,
                (search_r.width() - 32.0).max(0.0),
                search_r.height(),
            );
            let quads = text_ctx.shape_line_styled(
                label,
                text_r,
                13.0,
                400,
                None,
                AlignX::Left,
                AlignY::Middle,
            );
            let clipped = if layout.is_top {
                quads
            } else {
                clip_quads_to_right_edge(&quads, search_r.max().x.min(pane_right_edge))
            };
            let text_start = out.primitives.len();
            glyphs(
                &clipped,
                if disabled {
                    Token::TEXT_DISABLED
                } else if state.search_query.is_empty() {
                    Token::TEXT_SECONDARY
                } else {
                    Token::TEXT_PRIMARY
                },
                active_z + 1,
                &mut out.primitives,
                &mut out.payloads,
            );
            if !layout.is_top {
                apply_opacity_since(&mut out.primitives, text_start, pane_t);
            }
        }
        let key = navigation_view_row_node(node, NAVIGATION_VIEW_SEARCH_BUTTON_ID);
        let mut a11y = A11yNode::new(Role::Search, &desc.auto_suggest_placeholder, search_r)
            .with_value(&state.search_query)
            .with_actions(A11yActions::single(A11yAction::Click));
        a11y.disabled = disabled;
        a11y.focusable = !disabled;
        out.with_sub_control(
            key,
            HitRegion::ui(key, u32::from(active_z) + 2, search_r).with_cursor(if is_compact_icon {
                CursorKind::Pointer
            } else {
                CursorKind::Text
            }),
            a11y,
            !disabled,
        );
    }

    // 7. 发射所有可见主菜单与页脚行。
    let row_clip_right = (!layout.is_top).then_some(pane_right_edge);
    for row in &layout.rows {
        emit_navigation_row(
            node,
            row,
            layout.is_top,
            pane_t,
            row_clip_right,
            effective_selected,
            hovered_id,
            state.focused_item_id,
            disabled,
            active_z,
            text_ctx,
            &mut out,
        );
    }

    // 7.5 Left 窗格菜单区按需叠加滚动条（Top 模式与未溢出态均不发射图元与命中区）。
    if let Some(bar_desc) = desc.menu_scrollbar_desc(&layout, state, disabled) {
        let bar_out = emit_scrollbar(node, &bar_desc);
        if !bar_out.is_empty() {
            out.primitives.extend(bar_out.output.primitives);
            out.sub_hits.extend(bar_out.hits);
        }
    }

    // 8. 顶部模式溢出 `...` 按钮与下拉菜单。
    if let Some(ov_btn) = layout.overflow_button_rect {
        let ov_hovered = !disabled && hovered_id == Some(NAVIGATION_VIEW_OVERFLOW_BUTTON_ID);
        out.primitives.push(surface(
            ov_btn,
            4.0,
            if ov_hovered || state.overflow_menu_open {
                Token::NAV_HOVER
            } else {
                Token::NONE
            },
            active_z,
        ));
        match desc.overflow_label_mode {
            NavigationViewOverflowLabelMode::NoLabel => {
                // 无标签模式：只画水平三点图标（与 WinUI 顶部溢出按钮一致）。
                let icon_r = Rect::new(
                    ov_btn.origin.x + (ov_btn.width() - 16.0) * 0.5,
                    ov_btn.origin.y + (ov_btn.height() - 16.0) * 0.5,
                    16.0,
                    16.0,
                );
                emit_shell_icon_primitives(
                    ShellIcon::Ellipsis,
                    icon_r,
                    if disabled {
                        Token::TEXT_DISABLED
                    } else {
                        Token::TEXT_PRIMARY
                    },
                    Token::SURFACE_RAISED,
                    active_z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }
            NavigationViewOverflowLabelMode::MoreLabel => {
                let quads = text_ctx.shape_line_styled(
                    "More",
                    ov_btn,
                    13.0,
                    400,
                    None,
                    AlignX::Center,
                    AlignY::Middle,
                );
                glyphs(
                    &quads,
                    if disabled {
                        Token::TEXT_DISABLED
                    } else {
                        Token::TEXT_PRIMARY
                    },
                    active_z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }
        }
        let key = navigation_view_row_node(node, NAVIGATION_VIEW_OVERFLOW_BUTTON_ID);
        let mut a11y = A11yNode::new(Role::Button, "More", ov_btn)
            .expanded(state.overflow_menu_open)
            .with_actions(A11yActions::single(A11yAction::Click));
        a11y.disabled = disabled;
        a11y.focusable = !disabled;
        out.with_sub_control(
            key,
            HitRegion::ui(key, u32::from(active_z) + 2, ov_btn).with_cursor(CursorKind::Pointer),
            a11y,
            !disabled,
        );
    }

    if let Some(popup_r) = layout.overflow_popup_rect {
        if backdrop.is_none() {
            backdrop = BackdropRequest::new(node, popup_r, NAVIGATION_VIEW_OVERLAY_BLUR);
        }
        let popup_z = active_z + 30;
        out.primitives.push(outlined(
            popup_r,
            NAVIGATION_VIEW_CONTENT_CORNER_RADIUS,
            Token::OVERLAY_BG,
            Token::CARD_BORDER,
            1.0,
            12.0,
            popup_z,
        ));
        for row in &layout.overflow_rows {
            emit_navigation_row(
                node,
                row,
                false,
                1.0,
                None,
                effective_selected,
                hovered_id,
                state.focused_item_id,
                disabled,
                popup_z + 1,
                text_ctx,
                &mut out,
            );
        }
    }

    // 9. Compact / Top 子菜单 Flyout（原生毛玻璃 + 防越界定位）。
    if let Some(flyout_r) = layout.flyout_rect {
        if backdrop.is_none() {
            backdrop = BackdropRequest::new(node, flyout_r, NAVIGATION_VIEW_OVERLAY_BLUR);
        }
        let flyout_z = active_z + 32;
        out.primitives.push(outlined(
            flyout_r,
            NAVIGATION_VIEW_CONTENT_CORNER_RADIUS,
            Token::OVERLAY_BG,
            Token::CARD_BORDER,
            1.0,
            12.0,
            flyout_z,
        ));
        for row in &layout.flyout_rows {
            emit_navigation_row(
                node,
                row,
                false,
                1.0,
                None,
                effective_selected,
                hovered_id,
                state.focused_item_id,
                disabled,
                flyout_z + 1,
                text_ctx,
                &mut out,
            );
        }
    }

    // 10. 控件级外框焦点环（始终发射以保证跨 24 态图元数恒定，非聚焦时通过 focus_ring 隐藏）。
    out.primitives.push(focus_ring(
        desc.rect.inflate(3.0),
        8.0,
        Token::FOCUS_RING,
        2.0,
        active_z + 40,
        control.focused && !disabled,
    ));

    out.hit = Some(
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_cursor(CursorKind::Default),
    );
    let mut root_a11y = A11yNode::new(Role::Navigation, &desc.label, desc.rect);
    root_a11y.disabled = disabled;
    root_a11y.focusable = !disabled;
    out.a11y = Some(root_a11y);
    out.focusable = !disabled;

    NavigationViewOutput {
        output: out,
        backdrop,
        layout,
    }
}

/// 对 `primitives[start..]` 中新发射的图元叠乘不透明度（用于窗格展开/折叠平滑淡出与淡入）。
fn apply_opacity_since(primitives: &mut [GpuPrimitiveInstance], start: usize, opacity: f32) {
    let clamped = opacity.clamp(0.0, 1.0);
    if (clamped - 1.0).abs() <= f32::EPSILON {
        return;
    }
    for prim in &mut primitives[start..] {
        let combined = (prim.opacity() * clamped).clamp(0.0, 1.0);
        prim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(prim.depth_u16(), combined);
    }
}

/// 按窗格右缘水平裁剪字形四边形，确保折叠/展开中间帧文字自然裁切、不溢出窗格右缘。
fn clip_quads_to_right_edge(quads: &[GlyphQuad], max_x: f32) -> Vec<GlyphQuad> {
    let mut clipped = Vec::with_capacity(quads.len());
    for quad in quads {
        if quad.rect.origin.x >= max_x {
            continue;
        }
        if quad.rect.max().x <= max_x {
            clipped.push(*quad);
            continue;
        }
        let keep_w = (max_x - quad.rect.origin.x).max(0.0);
        if keep_w <= 0.0 {
            continue;
        }
        let ratio = (keep_w / quad.rect.size.x.max(f32::EPSILON)).clamp(0.0, 1.0);
        let mut partial = *quad;
        partial.rect.size.x = keep_w;
        partial.uv[2] = partial.uv[0] + (partial.uv[2] - partial.uv[0]) * ratio;
        clipped.push(partial);
    }
    clipped
}

/// 发射单行导航条目、分组标题、分隔线或 `Settings` 项（保持跨状态图元数恒定）。
#[allow(clippy::too_many_arguments)]
fn emit_navigation_row(
    parent_node: u64,
    row: &NavigationViewRow,
    is_top: bool,
    text_opacity: f32,
    clip_right: Option<f32>,
    effective_selected: Option<u64>,
    hovered_id: Option<u64>,
    focused_id: Option<u64>,
    disabled: bool,
    z: u16,
    text_ctx: &mut TextCtx<'_>,
    out: &mut WidgetOutput,
) {
    match &row.kind {
        NavigationViewRowKind::Header { content } => {
            if text_opacity <= 0.01 {
                return;
            }
            let label_r = Rect::new(
                row.rect.origin.x + 12.0,
                row.rect.origin.y,
                (row.rect.width() - 16.0).max(0.0),
                row.rect.height(),
            );
            let quads = text_ctx.shape_line_styled(
                content,
                label_r,
                14.0,
                600,
                None,
                AlignX::Left,
                AlignY::Middle,
            );
            let quads = match clip_right {
                Some(max_x) => clip_quads_to_right_edge(&quads, max_x),
                None => quads,
            };
            let start = out.primitives.len();
            glyphs(
                &quads,
                if disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::NAV_TEXT_MUTED
                },
                z + 1,
                &mut out.primitives,
                &mut out.payloads,
            );
            apply_opacity_since(&mut out.primitives, start, text_opacity);
        }
        NavigationViewRowKind::Separator { opacity } => {
            let mut line = surface(
                Rect::new(
                    row.rect.origin.x,
                    // WinUI 的分隔线在 9px 行内偏上 3px（实测分隔行 180..189 时线在 183）。
                    row.rect.origin.y + 3.0,
                    row.rect.width().max(0.0),
                    1.0,
                ),
                0.0,
                Token::DIVIDER,
                z + 1,
            );
            line.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(line.depth_u16(), opacity.clamp(0.0, 1.0));
            out.primitives.push(line);
        }
        NavigationViewRowKind::Item {
            item,
            depth,
            is_expanded,
        } => {
            emit_interactive_nav_item(
                parent_node,
                row.rect,
                item.id,
                &item.content,
                item.icon.as_ref(),
                item.info_badge.as_deref(),
                *depth,
                item.has_children(),
                *is_expanded,
                is_top,
                text_opacity,
                clip_right,
                effective_selected == Some(item.id),
                hovered_id == Some(item.id),
                focused_id == Some(item.id),
                disabled || !item.is_enabled,
                z,
                text_ctx,
                out,
            );
        }
        NavigationViewRowKind::FooterItem { item } => {
            emit_interactive_nav_item(
                parent_node,
                row.rect,
                item.id,
                &item.content,
                item.icon.as_ref(),
                item.info_badge.as_deref(),
                0,
                false,
                false,
                is_top,
                text_opacity,
                clip_right,
                effective_selected == Some(item.id),
                hovered_id == Some(item.id),
                focused_id == Some(item.id),
                disabled || !item.is_enabled,
                z,
                text_ctx,
                out,
            );
        }
        NavigationViewRowKind::Settings => {
            // 优先使用平台符号字体齿轮字形（U+E713），无符号字体时由 Glyph 分支优雅回退到 8 齿 ShellIcon::Settings。
            let settings_icon = NavigationViewIcon::Glyph("\u{E713}".into());
            emit_interactive_nav_item(
                parent_node,
                row.rect,
                NAVIGATION_VIEW_SETTINGS_ID,
                "Settings",
                Some(&settings_icon),
                None,
                0,
                false,
                false,
                is_top,
                text_opacity,
                clip_right,
                effective_selected == Some(NAVIGATION_VIEW_SETTINGS_ID),
                hovered_id == Some(NAVIGATION_VIEW_SETTINGS_ID),
                focused_id == Some(NAVIGATION_VIEW_SETTINGS_ID),
                disabled,
                z,
                text_ctx,
                out,
            );
        }
    }
}

/// 将常见 WinUI 3 符号字体 PUA 码点映射为内置矢量图标，供无符号字体环境（如 Linux）优雅回退。
fn fallback_shell_icon_for_glyph(glyph: &str) -> Option<ShellIcon> {
    match glyph {
        "\u{E80F}" => Some(ShellIcon::Home),
        "\u{E713}" => Some(ShellIcon::Settings),
        "\u{E8F1}" => Some(ShellIcon::Layers),
        "\u{E908}" | "\u{E9D2}" => Some(ShellIcon::Chart),
        "\u{E8A5}" | "\u{E7C3}" => Some(ShellIcon::Documents),
        "\u{E8B7}" => Some(ShellIcon::Folder),
        "\u{E8E5}" => Some(ShellIcon::FolderOpen),
        "\u{E787}" => Some(ShellIcon::Calendar),
        "\u{E721}" => Some(ShellIcon::Search),
        "\u{E74E}" => Some(ShellIcon::Save),
        "\u{E74D}" => Some(ShellIcon::Delete),
        "\u{E71D}" => Some(ShellIcon::AllApps),
        "\u{E712}" | "\u{E10C}" => Some(ShellIcon::Ellipsis),
        "\u{E762}" => Some(ShellIcon::DetailsView),
        "\u{E7B8}" => Some(ShellIcon::Package),
        _ => None,
    }
}

/// 发射单个可交互导航条目（背景胶囊 + 选中指示条 + 图标 + 文本 + 徽标 + 折叠箭头 + 焦点环）。
#[allow(clippy::too_many_arguments)]
fn emit_interactive_nav_item(
    parent_node: u64,
    rect: Rect,
    item_id: u64,
    label: &str,
    icon: Option<&NavigationViewIcon>,
    badge: Option<&str>,
    depth: usize,
    has_children: bool,
    is_expanded: bool,
    is_top: bool,
    text_opacity: f32,
    clip_right: Option<f32>,
    selected: bool,
    hovered: bool,
    focused: bool,
    disabled: bool,
    z: u16,
    text_ctx: &mut TextCtx<'_>,
    out: &mut WidgetOutput,
) {
    let collapsed = text_opacity <= 0.01;
    let row_fill = if disabled {
        Token::NONE
    } else if selected && !is_top {
        // 顶部模式的选中态只画底部强调条，不铺胶囊底（与 WinUI 顶部导航一致）。
        Token::NAV_SELECTED
    } else if hovered {
        Token::NAV_HOVER
    } else {
        Token::NONE
    };
    out.primitives.push(surface(rect, 4.0, row_fill, z));

    // 选中指示条：Left 在左缘 3×16 竖胶囊，Top 在底边 16×3 水平胶囊；未选中以不透明度 0 保持恒定图元数。
    let indicator_rect = if is_top {
        Rect::new(
            rect.origin.x + (rect.width() - NAVIGATION_VIEW_TOP_INDICATOR_WIDTH) * 0.5,
            rect.max().y - NAVIGATION_VIEW_TOP_INDICATOR_HEIGHT,
            NAVIGATION_VIEW_TOP_INDICATOR_WIDTH,
            NAVIGATION_VIEW_TOP_INDICATOR_HEIGHT,
        )
    } else {
        // 层级子项的强调条落在**子项自己的按钮左缘**（父项缩进宽度之后），
        // 而不是整条窗格的左缘：参考图父项 x=20、子项 x=51。
        let indicator_indent = if collapsed {
            0.0
        } else {
            depth as f32 * NAVIGATION_VIEW_CHILD_INDENT
        };
        Rect::new(
            rect.origin.x + indicator_indent,
            rect.origin.y + (rect.height() - NAVIGATION_VIEW_INDICATOR_HEIGHT) * 0.5,
            NAVIGATION_VIEW_INDICATOR_WIDTH,
            NAVIGATION_VIEW_INDICATOR_HEIGHT,
        )
    };
    let mut indicator = surface(
        indicator_rect,
        NAVIGATION_VIEW_INDICATOR_RADIUS,
        Token::PRIMARY,
        z + 2,
    );
    if !selected {
        indicator.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(indicator.depth_u16(), 0.0);
    } else if disabled {
        // WinUI 禁用态仍保留强调条，但压到约 30% 不透明度（实测浅色 #A9C5E0 叠在 #F3F3F3 上）。
        indicator.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(indicator.depth_u16(), 0.30);
    }
    out.primitives.push(indicator);

    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::NAV_TEXT
    };

    let indent = if is_top || collapsed {
        0.0
    } else {
        depth as f32 * NAVIGATION_VIEW_CHILD_INDENT
    };

    // 图标区域：一级图标始终锚定在左侧 48px 紧凑列固定偏移（rect.origin.x + 12.0 + indent），
    // 无论窗格展开/折叠动画处于何种中间宽度，水平 X 坐标全程严格保持 0px 位移。
    let icon_x = rect.origin.x + 12.0 + indent;
    if let Some(ic) = icon {
        let icon_r = Rect::new(
            icon_x,
            rect.origin.y + (rect.height() - 16.0) * 0.5,
            16.0,
            16.0,
        );
        match ic {
            NavigationViewIcon::Shell(shell_icon) => {
                emit_shell_icon_primitives(
                    *shell_icon,
                    icon_r,
                    ink,
                    Token::SURFACE_RAISED,
                    z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }
            NavigationViewIcon::Glyph(glyph) => {
                if let Some(quads) = text_ctx.shape_symbol_glyph(glyph, icon_r, 16.0) {
                    glyphs(&quads, ink, z + 1, &mut out.primitives, &mut out.payloads);
                } else if let Some(fallback) = fallback_shell_icon_for_glyph(glyph) {
                    emit_shell_icon_primitives(
                        fallback,
                        icon_r,
                        ink,
                        Token::SURFACE_RAISED,
                        z + 1,
                        &mut out.primitives,
                        &mut out.payloads,
                    );
                } else {
                    let quads = text_ctx.shape_line_styled(
                        glyph,
                        Rect::new(icon_x, rect.origin.y, 16.0, rect.height()),
                        16.0,
                        400,
                        None,
                        AlignX::Center,
                        AlignY::Middle,
                    );
                    glyphs(&quads, ink, z + 1, &mut out.primitives, &mut out.payloads);
                }
            }
        }
    }

    // 文本标签、子项折叠箭头与徽标：随窗格展开进度平滑淡出/淡入并受窗格右缘裁剪，`t <= 0.01` 时完全不画。
    if !collapsed {
        let text_x = if icon.is_some() {
            if is_top {
                // WinUI 顶部模式图标盒 16px 后留 8px 间距（icon_x + 24.0），对齐 Home 与 Settings 文字左缘。
                icon_x + 24.0
            } else {
                // WinUI 左侧模式图标盒 16px 后留 16px 间距（实测文字左缘 = 行左 + 45）。
                icon_x + 32.0
            }
        } else {
            rect.origin.x + 12.0 + indent
        };
        let pane_right = clip_right.unwrap_or(rect.max().x);
        let mut right_edge = (rect.max().x - 12.0).min(pane_right);
        if has_children {
            let chevron_r = Rect::new(
                right_edge - 16.0,
                rect.origin.y + (rect.height() - 16.0) * 0.5,
                16.0,
                16.0,
            );
            if chevron_r.origin.x >= icon_x + 20.0 && chevron_r.max().x <= pane_right {
                let chevron = if is_expanded {
                    ShellIcon::ChevronUp
                } else {
                    ShellIcon::ChevronDown
                };
                let start = out.primitives.len();
                emit_shell_icon_primitives(
                    chevron,
                    chevron_r,
                    if disabled {
                        Token::TEXT_DISABLED
                    } else {
                        Token::NAV_TEXT_MUTED
                    },
                    Token::SURFACE_RAISED,
                    z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                apply_opacity_since(&mut out.primitives, start, text_opacity);
            }
            right_edge -= 22.0;
        }
        if let Some(badge_text) = badge {
            let badge_r = Rect::new(
                right_edge - NAVIGATION_VIEW_BADGE_WIDTH,
                rect.origin.y + (rect.height() - NAVIGATION_VIEW_BADGE_HEIGHT) * 0.5,
                NAVIGATION_VIEW_BADGE_WIDTH,
                NAVIGATION_VIEW_BADGE_HEIGHT,
            );
            if badge_r.origin.x >= icon_x + 20.0 && badge_r.max().x <= pane_right {
                let start = out.primitives.len();
                out.primitives.push(surface(
                    badge_r,
                    NAVIGATION_VIEW_BADGE_HEIGHT * 0.5,
                    if disabled {
                        Token::CONTROL_DISABLED_BG
                    } else {
                        Token::PRIMARY_SOFT
                    },
                    z + 1,
                ));
                let bq = text_ctx.shape_line_styled(
                    badge_text,
                    badge_r,
                    11.0,
                    600,
                    None,
                    AlignX::Center,
                    AlignY::Middle,
                );
                glyphs(
                    &bq,
                    if disabled {
                        Token::TEXT_DISABLED
                    } else {
                        Token::PRIMARY
                    },
                    z + 2,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                apply_opacity_since(&mut out.primitives, start, text_opacity);
            }
            right_edge -= NAVIGATION_VIEW_BADGE_WIDTH + 6.0;
        }
        let text_r = Rect::new(
            text_x,
            rect.origin.y,
            (right_edge - text_x).max(0.0),
            rect.height(),
        );
        let weight = if selected { 600 } else { 400 };
        let quads = text_ctx.shape_line_styled(
            label,
            text_r,
            14.0,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        let quads = match clip_right {
            Some(max_x) => clip_quads_to_right_edge(&quads, max_x),
            None => quads,
        };
        let start = out.primitives.len();
        glyphs(&quads, ink, z + 1, &mut out.primitives, &mut out.payloads);
        apply_opacity_since(&mut out.primitives, start, text_opacity);
    }

    // 单项键盘焦点环（rect.inflate(3.0)，未聚焦时保持恒定图元数）。
    out.primitives.push(focus_ring(
        rect.inflate(3.0),
        4.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        focused && !disabled,
    ));

    let row_key = navigation_view_row_node(parent_node, item_id);
    let role = if has_children || depth > 0 {
        Role::TreeItem
    } else {
        Role::Tab
    };
    let mut a11y = A11yNode::new(role, label, rect)
        .selected(selected)
        .with_level((depth + 1) as u32)
        .with_actions(A11yActions::single(A11yAction::Click));
    if has_children {
        a11y = a11y.expanded(is_expanded);
    }
    a11y.disabled = disabled;
    a11y.focusable = !disabled;
    out.with_sub_control(
        row_key,
        HitRegion::ui(row_key, u32::from(z) + 3, rect).with_cursor(if disabled {
            CursorKind::Default
        } else {
            CursorKind::Pointer
        }),
        a11y,
        !disabled,
    );
}

impl UiWidget for NavigationViewDesc {
    type State = NavigationViewState;

    fn initial_state(&self) -> Self::State {
        NavigationViewState::from_desc(self)
    }

    fn sync_state(&self, state: &mut Self::State) {
        if let Some(sel) = self.selected_item_id {
            if state.selected_item_id.is_none() {
                state.selected_item_id = Some(sel);
            }
        }
        state.hovered_item_id = self.hovered_item_id;
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let w = if self.rect.width() > 0.0 {
            self.rect.width()
        } else {
            self.open_pane_length + 320.0
        };
        let h = if self.rect.height() > 0.0 {
            self.rect.height()
        } else {
            360.0
        };
        Vec2::new(w, h)
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let mut laid = self.clone();
        laid.rect = rect;
        state.sync_viewport(&laid, rect.width());
        let layout = laid.compute_layout(state);
        state.scroll_max =
            (layout.menu_content_height - layout.menu_viewport_rect.height()).max(0.0);

        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                state.handle_pointer_down(&laid, &layout, *pos)
            }
            WidgetInputEvent::PointerDrag { pos } => {
                state.handle_pointer_drag(&laid, &layout, *pos)
            }
            WidgetInputEvent::PointerUp { pos } => state.handle_pointer_up(*pos),
            WidgetInputEvent::Wheel { delta } => state.scroll_by(-delta.y),
            WidgetInputEvent::KeyDown(chord) => state.handle_key_down(&laid, &layout, *chord),
            WidgetInputEvent::TextInput(text) if state.search_focused => {
                state.search_query.push_str(text);
                state.events.push(NavigationViewEvent::SearchRequested {
                    query: state.search_query.clone(),
                });
                true
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut laid = self.clone();
        laid.rect = rect;
        emit_navigation_view(node, &laid, control, state, text_ctx).output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scroll::{scrollbar_track_node, ScrollbarAxis, SCROLLBAR_EDGE_INSET};
    use unbound_text::{GlyphCache, SymbolFontPolicy, TextEngine};

    #[test]
    fn back_visible_false_places_pane_toggle_above_first_row_without_overlap() {
        let rect = Rect::new(16.0, 16.0, 360.0, 280.0);
        let items = vec![
            NavigationViewMenuItem::header("Workspace"),
            NavigationViewMenuItem::item(
                NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
            ),
        ];

        // back_visible == false：汉堡按钮应落在 y=22，位于首行菜单之上且无重叠。
        let desc_no_back = NavigationViewDesc::new(rect, items.clone())
            .pane_display_mode(NavigationViewPaneDisplayMode::Left)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(1));
        let mut state_no_back = NavigationViewState::from_desc(&desc_no_back);
        state_no_back.set_animations_enabled(false, &desc_no_back);
        let layout_no_back = desc_no_back.compute_layout(&state_no_back);

        assert!(layout_no_back.back_button_rect.is_none());
        let toggle_no_back = layout_no_back
            .pane_toggle_rect
            .expect("pane toggle rect must exist");
        let first_row = layout_no_back
            .rows
            .first()
            .expect("menu must contain at least one row");
        assert!((toggle_no_back.origin.y - 22.0).abs() < 1e-3);
        assert!(
            toggle_no_back.origin.y < first_row.rect.origin.y,
            "pane toggle y {} must be strictly above first row y {}",
            toggle_no_back.origin.y,
            first_row.rect.origin.y
        );
        assert!(
            toggle_no_back.max().y <= first_row.rect.origin.y,
            "pane toggle bottom {} must not overlap first row top {}",
            toggle_no_back.max().y,
            first_row.rect.origin.y
        );

        // back_visible == true：返回键占首槽（y=22），汉堡按钮保持 y=62（与 NavigationViewChrome 一致）。
        let desc_with_back = NavigationViewDesc::new(rect, items)
            .pane_display_mode(NavigationViewPaneDisplayMode::Left)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_back_button_visible(NavigationViewBackButtonVisible::Visible)
            .selected_item(Some(1));
        let mut state_with_back = NavigationViewState::from_desc(&desc_with_back);
        state_with_back.set_animations_enabled(false, &desc_with_back);
        let layout_with_back = desc_with_back.compute_layout(&state_with_back);

        let back_rect = layout_with_back
            .back_button_rect
            .expect("back button rect must exist");
        let toggle_with_back = layout_with_back
            .pane_toggle_rect
            .expect("pane toggle rect must exist");
        let first_row_back = layout_with_back
            .rows
            .first()
            .expect("menu must contain at least one row");
        assert!((back_rect.origin.y - 22.0).abs() < 1e-3);
        assert!((toggle_with_back.origin.y - 62.0).abs() < 1e-3);
        assert!(toggle_with_back.max().y <= first_row_back.rect.origin.y);
    }

    #[test]
    fn top_mode_aligns_home_text_and_centers_selection_indicator() {
        let rect = Rect::new(16.0, 16.0, 360.0, 280.0);
        let desc = NavigationViewDesc::new(
            rect,
            vec![
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                ),
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(2, "Library").with_glyph("\u{E8F1}"),
                ),
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(3, "Analytics").with_glyph("\u{E908}"),
                ),
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(4, "Reports").with_glyph("\u{E8A5}"),
                ),
            ],
        )
        .pane_display_mode(NavigationViewPaneDisplayMode::Top)
        .overflow_label_mode(NavigationViewOverflowLabelMode::NoLabel)
        .is_settings_visible(true)
        .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
        .selected_item(Some(1));

        let mut state = NavigationViewState::from_desc(&desc);
        state.set_animations_enabled(false, &desc);

        let mut engine = TextEngine::new();
        engine.set_symbol_font_policy(SymbolFontPolicy::Disabled);
        let mut cache = GlyphCache::new();
        let mut ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        };

        let emitted = emit_navigation_view(201, &desc, ControlState::default(), &state, &mut ctx);
        let home_row = emitted
            .layout
            .rows
            .iter()
            .find(|r| r.item_id() == Some(1))
            .expect("Home row must exist");

        // Home 项宽为 85.0（origin.x = 24.0），icon_x = 36.0，Top 模式文字起点为 icon_x + 24.0 = 60.0。
        assert!((home_row.rect.origin.x - 24.0).abs() < 1e-3);
        assert!((home_row.rect.width() - 85.0).abs() < 1e-3);

        let icon_x = home_row.rect.origin.x + 12.0;
        let expected_text_x = icon_x + 24.0;
        let home_text_min_x = emitted
            .output
            .primitives
            .iter()
            .filter(|p| {
                p.prim_type() == PrimType::Glyph
                    && p.local_pos[0] >= icon_x + 18.0
                    && p.local_pos[0] <= home_row.rect.max().x
                    && p.local_pos[1] >= home_row.rect.origin.y
                    && p.local_pos[1] <= home_row.rect.max().y
            })
            .map(|p| p.local_pos[0])
            .fold(f32::INFINITY, f32::min);
        assert!(
            home_text_min_x.is_finite(),
            "Home label glyphs must be emitted in Top mode"
        );
        assert!(
            (home_text_min_x - expected_text_x).abs() <= 3.0,
            "Home text left edge {home_text_min_x} must align near icon_x + 24.0 ({expected_text_x})"
        );
        assert!(
            home_text_min_x < icon_x + 30.0,
            "Home text left edge {home_text_min_x} must not suffer +8px Left-mode offset"
        );

        // 底部蓝色选中指示条应水平居中于 Home 项矩形（中心 66.5，区间 58.5..74.5）。
        let indicator = emitted
            .output
            .primitives
            .iter()
            .find(|p| {
                p.prim_type() == PrimType::Rect
                    && (p.size[0] - NAVIGATION_VIEW_TOP_INDICATOR_WIDTH).abs() < 1e-3
                    && (p.size[1] - NAVIGATION_VIEW_TOP_INDICATOR_HEIGHT).abs() < 1e-3
                    && p.opacity() > 0.5
                    && p.local_pos[0] >= home_row.rect.origin.x
                    && p.local_pos[0] <= home_row.rect.max().x
            })
            .expect("selected Top Home indicator primitive must exist");
        let indicator_cx = indicator.local_pos[0] + indicator.size[0] * 0.5;
        assert!(
            (indicator_cx - home_row.rect.center().x).abs() < 1e-3,
            "indicator center {indicator_cx} must match Home rect center {}",
            home_row.rect.center().x
        );

        // 溢出按钮紧随 Home 之后（109 + 4 = 113，中心 135）。
        let ov_btn = emitted
            .layout
            .overflow_button_rect
            .expect("Top mode must emit overflow button for 4 items in 360px");
        assert!((ov_btn.center().x - 135.0).abs() < 1e-3);
    }

    #[test]
    fn symbol_font_disabled_falls_back_to_vector_primitives_without_panic() {
        let rect = Rect::new(16.0, 16.0, 360.0, 280.0);
        let desc = NavigationViewDesc::new(
            rect,
            vec![
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(1, "Home").with_glyph("\u{E80F}"),
                ),
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(2, "Analytics").with_glyph("\u{E908}"),
                ),
                NavigationViewMenuItem::item(
                    NavigationViewItem::new(3, "Custom").with_glyph("\u{E999}"),
                ),
            ],
        )
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .open_pane_length(220.0)
        .is_pane_open(true)
        .is_settings_visible(true)
        .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
        .selected_item(Some(1));

        let mut state = NavigationViewState::from_desc(&desc);
        state.set_animations_enabled(false, &desc);

        let mut engine = TextEngine::new();
        engine.set_symbol_font_policy(SymbolFontPolicy::Disabled);
        let mut cache = GlyphCache::new();
        let mut ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        };

        let emitted = emit_navigation_view(202, &desc, ControlState::default(), &state, &mut ctx);

        for target_id in [1_u64, 2_u64, NAVIGATION_VIEW_SETTINGS_ID] {
            let row = emitted
                .layout
                .rows
                .iter()
                .find(|r| r.item_id() == Some(target_id))
                .expect("expected nav row to exist");
            let icon_x = row.rect.origin.x + 12.0;
            let icon_rect = Rect::new(icon_x - 1.0, row.rect.origin.y + 4.0, 18.0, 28.0);

            let vector_count = emitted
                .output
                .primitives
                .iter()
                .filter(|p| {
                    matches!(p.prim_type(), PrimType::Rect | PrimType::Curve)
                        && p.local_pos[0] >= icon_rect.origin.x
                        && p.local_pos[0] <= icon_rect.max().x
                        && p.local_pos[1] >= icon_rect.origin.y
                        && p.local_pos[1] <= icon_rect.max().y
                        && p.size[0] <= 18.0
                        && p.size[1] <= 18.0
                })
                .count();
            assert!(
                vector_count > 0,
                "row {target_id} must fall back to vector primitives when symbol font is disabled"
            );

            let icon_glyph_count = emitted
                .output
                .primitives
                .iter()
                .filter(|p| {
                    p.prim_type() == PrimType::Glyph
                        && p.local_pos[0] >= icon_rect.origin.x
                        && p.local_pos[0] <= icon_rect.max().x
                        && p.local_pos[1] >= icon_rect.origin.y
                        && p.local_pos[1] <= icon_rect.max().y
                })
                .count();
            assert_eq!(
                icon_glyph_count, 0,
                "row {target_id} must not emit tofu glyphs in icon slot when vector fallback matches"
            );
        }
    }

    #[test]
    fn expand_animation_mid_frames_preserve_icon_anchor_header_clip_and_scrollbar_bounds() {
        let rect = Rect::new(16.0, 16.0, 640.0, 260.0);
        let owner_node = 203_u64;
        let track_node = scrollbar_track_node(owner_node, ScrollbarAxis::Vertical);
        let open = 220.0_f32;
        let compact = NAVIGATION_VIEW_COMPACT_PANE_LENGTH;
        let expected_icon_x = rect.origin.x + NAVIGATION_VIEW_PANE_PADDING + 12.0;

        let mut items = vec![
            NavigationViewMenuItem::header("Workspace Navigation Section"),
            NavigationViewMenuItem::item(
                NavigationViewItem::new(1, "Home").with_icon(ShellIcon::Grid),
            ),
        ];
        for i in 2..=12_u64 {
            items.push(NavigationViewMenuItem::item(
                NavigationViewItem::new(i, format!("Section {i}")).with_icon(ShellIcon::Layers),
            ));
        }
        let desc = NavigationViewDesc::new(rect, items)
            .pane_display_mode(NavigationViewPaneDisplayMode::Left)
            .open_pane_length(open)
            .is_pane_open(true)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .selected_item(Some(1));

        let mut state = NavigationViewState::from_desc(&desc);
        state.set_animations_enabled(true, &desc);

        let mut engine = TextEngine::new();
        engine.set_symbol_font_policy(SymbolFontPolicy::Disabled);
        let mut cache = GlyphCache::new();
        let mut ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        };

        let mut prev_header_max_right = 0.0_f32;
        for t in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            state.pane_width_spring.value = compact + (open - compact) * t;
            let emitted =
                emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
            let pane_right = emitted.layout.pane_rect.max().x;

            // 1. 一级图标水平锚点 0px 跳变。
            let home_row = emitted
                .layout
                .rows
                .iter()
                .find(|r| r.item_id() == Some(1))
                .expect("Home row must exist");
            let grid_cell = emitted
                .output
                .primitives
                .iter()
                .find(|p| {
                    (p.size[0] - 5.6).abs() < 0.05
                        && (p.size[1] - 5.6).abs() < 0.05
                        && p.local_pos[1] >= home_row.rect.origin.y
                        && p.local_pos[1] <= home_row.rect.max().y
                })
                .expect("Home ShellIcon::Grid cell must exist");
            let primary_icon_x = grid_cell.local_pos[0] - 1.5;
            assert!(
                (primary_icon_x - expected_icon_x).abs() < 1e-3,
                "primary icon drifted at t={t}: got {primary_icon_x}, expected {expected_icon_x}"
            );

            // 2. 分组标题右缘裁剪连续且不越过窗格右缘。
            let header_row = emitted
                .layout
                .rows
                .iter()
                .find(|r| matches!(r.kind, NavigationViewRowKind::Header { .. }))
                .expect("Header row must exist");
            let header_rights: Vec<f32> = emitted
                .output
                .primitives
                .iter()
                .filter(|p| {
                    p.prim_type() == PrimType::Glyph
                        && p.local_pos[1] >= header_row.rect.origin.y
                        && p.local_pos[1] <= header_row.rect.max().y
                        && p.local_pos[0] < pane_right + 1.0
                })
                .map(|p| p.local_pos[0] + p.size[0])
                .collect();
            if t <= 0.01 {
                assert!(
                    header_rights.is_empty(),
                    "header glyphs must be hidden at collapsed frame t={t}"
                );
            } else {
                assert!(
                    !header_rights.is_empty(),
                    "header glyphs must be emitted at expand frame t={t}"
                );
                let max_right = header_rights
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    max_right <= pane_right + 0.5,
                    "header right edge {max_right} exceeded pane right {pane_right} at t={t}"
                );
                assert!(
                    max_right + 0.5 >= prev_header_max_right,
                    "header clipped right edge regressed from {prev_header_max_right} to {max_right} at t={t}"
                );
                prev_header_max_right = max_right;
            }

            // 3. 滚动条贴合窗格右缘且无越界（t > 0.01 时常显细指示条，t == 0.0 静止折叠态自动隐藏）。
            let track_hit = emitted
                .output
                .sub_hits
                .iter()
                .find(|h| h.node == track_node);
            if t <= 0.01 {
                assert!(
                    track_hit.is_none(),
                    "collapsed idle pane must auto-hide scrollbar at t={t}"
                );
            } else {
                let track = track_hit.expect("overflowing expanding pane must emit scrollbar track");
                assert!(
                    (track.bounds.max().x - (pane_right - SCROLLBAR_EDGE_INSET)).abs() < 1e-3,
                    "scrollbar track right {} must hug pane right {} - inset at t={t}",
                    track.bounds.max().x,
                    pane_right
                );
                assert!(
                    track.bounds.max().x <= pane_right,
                    "scrollbar track right {} overflowed pane right {pane_right} at t={t}",
                    track.bounds.max().x
                );
            }
        }
    }
}

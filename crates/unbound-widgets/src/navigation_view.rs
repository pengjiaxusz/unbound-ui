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
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
use crate::overlay::{place_anchored_popup_detailed, BackdropRequest, Placement};
use crate::primitive::{focus_ring, glyphs, outlined, surface};
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

/// 分组标题行高。
pub const NAVIGATION_VIEW_SECTION_HEADER_HEIGHT: f32 = 28.0;

/// 分隔线行高。
pub const NAVIGATION_VIEW_SEPARATOR_HEIGHT: f32 = 9.0;

/// 每级层级子项的左侧缩进增量。
pub const NAVIGATION_VIEW_CHILD_INDENT: f32 = 28.0;

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
        a || b || c || d
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
            true
        } else {
            false
        }
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
            NavigationViewDisplayMode::Compact => self.compact_pane_length,
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
        if display_mode == NavigationViewDisplayMode::Minimal && !state.is_pane_open {
            let mut btn_x = rect.origin.x + NAVIGATION_VIEW_PANE_PADDING;
            if back_visible {
                layout.back_button_rect = Some(Rect::new(btn_x, cursor_y, 40.0, 36.0));
                btn_x += 40.0;
            }
            if self.is_pane_toggle_button_visible {
                layout.pane_toggle_rect = Some(Rect::new(btn_x, cursor_y, 40.0, 36.0));
            }
        } else {
            if back_visible {
                layout.back_button_rect = Some(Rect::new(
                    active_pane_rect.origin.x + NAVIGATION_VIEW_PANE_PADDING,
                    cursor_y,
                    40.0,
                    36.0,
                ));
                cursor_y += 40.0;
            }
            if self.is_pane_toggle_button_visible {
                layout.pane_toggle_rect = Some(Rect::new(
                    active_pane_rect.origin.x + NAVIGATION_VIEW_PANE_PADDING,
                    cursor_y,
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
        if show_header {
            layout.header_rect = Some(Rect::new(
                content_x,
                rect.origin.y + top_pad,
                content_w,
                header_h,
            ));
        }
        let content_y = rect.origin.y + top_pad + header_h;
        let content_h = (rect.height() - top_pad - header_h).max(0.0);
        layout.content_rect = Rect::new(content_x, content_y, content_w, content_h);
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
            return layout;
        }

        let pane_is_narrow = !state.is_pane_open;
        let inner_x = active_pane_rect.origin.x + NAVIGATION_VIEW_PANE_PADDING;
        let inner_w = (active_pane_rect.width() - NAVIGATION_VIEW_PANE_PADDING * 2.0).max(1.0);

        // 可选 AutoSuggestBox 搜索槽位。
        if self.show_auto_suggest_box {
            let search_h = 32.0;
            let search_w = if pane_is_narrow { 40.0 } else { inner_w };
            layout.search_rect = Some(Rect::new(inner_x, cursor_y + 2.0, search_w, search_h));
            cursor_y += search_h + 8.0;
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
            pane_is_narrow,
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
            let row_r = Rect::new(inner_x, row_y, inner_w, h);
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
                    if !pane_is_narrow {
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
        if pane_is_narrow {
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
            let settings_r =
                Rect::new(right_limit - 40.0, row_y, 40.0, NAVIGATION_VIEW_ITEM_HEIGHT);
            layout.rows.push(NavigationViewRow {
                kind: NavigationViewRowKind::Settings,
                rect: settings_r,
            });
            right_limit -= 44.0;
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
        for entry in &self.menu_items {
            let NavigationViewMenuItem::Item(item) = entry else {
                continue;
            };
            let est_w = (item.content.chars().count() as f32 * 8.0 + 48.0).clamp(72.0, 180.0);
            let needs_space = if overflow_items.is_empty() {
                right_limit - overflow_reserve
            } else {
                0.0
            };
            if overflow_items.is_empty() && x + est_w <= needs_space {
                layout.rows.push(NavigationViewRow {
                    kind: NavigationViewRowKind::Item {
                        item: item.clone(),
                        depth: 0,
                        is_expanded: state.open_flyout_parent_id == Some(item.id),
                    },
                    rect: Rect::new(x, row_y, est_w, NAVIGATION_VIEW_ITEM_HEIGHT),
                });
                x += est_w + 4.0;
            } else {
                overflow_items.push(item.clone());
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
        Token::CARD_BG,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 3. 顶部页头标题（若存在）。
    if let (Some(header_r), Some(header_text)) = (layout.header_rect, desc.header.as_deref()) {
        let title_r = Rect::new(
            header_r.origin.x + 24.0,
            header_r.origin.y,
            (header_r.width() - 48.0).max(0.0),
            header_r.height(),
        );
        let quads = text_ctx.shape_line_styled(
            header_text,
            title_r,
            24.0,
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
        out.primitives.push(outlined(
            ov_r,
            NAVIGATION_VIEW_CONTENT_CORNER_RADIUS,
            Token::OVERLAY_BG,
            Token::CARD_BORDER,
            1.0,
            16.0,
            z + 20,
        ));
        z + 22
    } else {
        z + 2
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
        let back_quads = text_ctx.shape_line_styled(
            "\u{E72B}",
            back_r,
            14.0,
            400,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        glyphs(
            &back_quads,
            if back_enabled {
                Token::TEXT_PRIMARY
            } else {
                Token::TEXT_DISABLED
            },
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
        let hamburger_x = toggle_r.origin.x + (toggle_r.width() - 12.0) * 0.5;
        let base_y = toggle_r.origin.y + (toggle_r.height() - 10.0) * 0.5;
        let bar_ink = if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        for dy in [0.0_f32, 4.5, 9.0] {
            out.primitives.push(surface(
                Rect::new(hamburger_x, base_y + dy, 12.0, 1.0),
                0.0,
                bar_ink,
                active_z + 1,
            ));
        }
        // 紧邻汉堡按钮右侧绘制 PaneTitle（展开态可见）。
        if state.is_pane_open {
            if let Some(title) = desc.pane_title.as_deref() {
                let pt_r = Rect::new(
                    toggle_r.max().x + 8.0,
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
                glyphs(
                    &pt_quads,
                    bar_ink,
                    active_z + 1,
                    &mut out.primitives,
                    &mut out.payloads,
                );
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

    // 6. 可选 AutoSuggestBox 搜索槽位。
    if let Some(search_r) = layout.search_rect {
        let is_compact_icon = !layout.is_top && !state.is_pane_open;
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
                Token::CARD_BORDER
            },
            if is_compact_icon { 0.0 } else { 1.0 },
            0.0,
            active_z,
        ));
        if is_compact_icon {
            let icon_r = Rect::new(
                search_r.origin.x + (search_r.width() - 16.0) * 0.5,
                search_r.origin.y + (search_r.height() - 16.0) * 0.5,
                16.0,
                16.0,
            );
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
        } else {
            let label = if state.search_query.is_empty() {
                desc.auto_suggest_placeholder.as_str()
            } else {
                state.search_query.as_str()
            };
            let text_r = Rect::new(
                search_r.origin.x + 10.0,
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
            glyphs(
                &quads,
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
    let pane_is_narrow = !layout.is_top && !state.is_pane_open;
    for row in &layout.rows {
        emit_navigation_row(
            node,
            row,
            layout.is_top,
            pane_is_narrow,
            effective_selected,
            hovered_id,
            state.focused_item_id,
            disabled,
            active_z,
            text_ctx,
            &mut out,
        );
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
        let label = match desc.overflow_label_mode {
            NavigationViewOverflowLabelMode::MoreLabel => "... More",
            NavigationViewOverflowLabelMode::NoLabel => "...",
        };
        let quads = text_ctx.shape_line_styled(
            label,
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
                false,
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
                false,
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

/// 发射单行导航条目、分组标题、分隔线或 `Settings` 项（保持跨状态图元数恒定）。
#[allow(clippy::too_many_arguments)]
fn emit_navigation_row(
    parent_node: u64,
    row: &NavigationViewRow,
    is_top: bool,
    pane_is_narrow: bool,
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
            let label_r = Rect::new(
                row.rect.origin.x + 12.0,
                row.rect.origin.y,
                (row.rect.width() - 16.0).max(0.0),
                row.rect.height(),
            );
            let quads = text_ctx.shape_line_styled(
                content,
                label_r,
                12.0,
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
                    Token::NAV_TEXT_MUTED
                },
                z + 1,
                &mut out.primitives,
                &mut out.payloads,
            );
        }
        NavigationViewRowKind::Separator { opacity } => {
            let mut line = surface(
                Rect::new(
                    row.rect.origin.x + 12.0,
                    row.rect.origin.y + row.rect.height() * 0.5,
                    (row.rect.width() - 24.0).max(0.0),
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
                pane_is_narrow,
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
                pane_is_narrow,
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
            let settings_icon = NavigationViewIcon::Glyph("\u{E713}".to_owned());
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
                pane_is_narrow || is_top,
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
    icon_only: bool,
    selected: bool,
    hovered: bool,
    focused: bool,
    disabled: bool,
    z: u16,
    text_ctx: &mut TextCtx<'_>,
    out: &mut WidgetOutput,
) {
    let row_fill = if disabled {
        Token::NONE
    } else if selected {
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
        Rect::new(
            rect.origin.x,
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
    if !selected || disabled {
        indicator.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(indicator.depth_u16(), 0.0);
    }
    out.primitives.push(indicator);

    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::NAV_TEXT
    };

    let indent = if is_top || icon_only {
        0.0
    } else {
        depth as f32 * NAVIGATION_VIEW_CHILD_INDENT
    };

    // 图标区域。
    let icon_x = if icon_only {
        rect.origin.x + (rect.width() - 16.0) * 0.5
    } else {
        rect.origin.x + 12.0 + indent
    };
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

    // 文本标签（非仅图标态）。
    if !icon_only {
        let text_x = if icon.is_some() {
            icon_x + 28.0
        } else {
            rect.origin.x + 12.0 + indent
        };
        let mut right_edge = rect.max().x - 12.0;
        if has_children {
            let chevron_r = Rect::new(
                right_edge - 16.0,
                rect.origin.y + (rect.height() - 16.0) * 0.5,
                16.0,
                16.0,
            );
            let chevron = if is_expanded {
                ShellIcon::ChevronDown
            } else {
                ShellIcon::ChevronRight
            };
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
            right_edge -= 22.0;
        }
        if let Some(badge_text) = badge {
            let badge_r = Rect::new(
                right_edge - NAVIGATION_VIEW_BADGE_WIDTH,
                rect.origin.y + (rect.height() - NAVIGATION_VIEW_BADGE_HEIGHT) * 0.5,
                NAVIGATION_VIEW_BADGE_WIDTH,
                NAVIGATION_VIEW_BADGE_HEIGHT,
            );
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
        glyphs(&quads, ink, z + 1, &mut out.primitives, &mut out.payloads);
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

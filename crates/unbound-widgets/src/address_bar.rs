//! 双模路径导航地址栏（`AddressBar`）：
//! 外部 `← → ↑ ↻` 导航按钮簇 + 一体化面包屑/原位文本编辑双模地址框 + 右侧可折叠内联搜索框。

use std::ops::Deref;

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::shell::{
    emit_shell_icon_primitives, emit_widget_icon_primitives, ShellIcon, WidgetIcon,
};
use crate::text_input::{
    floor_boundary, focus_underline, reduce_text_input, TextEditState, TextInputDesc,
    TextInputEvent, TextInputOutcome, TextInputVisual,
};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 地址栏标准高度（对齐 Win11 Fluent 胶囊尺度 `36px`，落在原生 `34–36px` 区间）。
pub const ADDRESS_BAR_HEIGHT: f32 = 36.0;
/// 地址栏、面包屑与内联搜索框默认正文基准字号（`14px`，对齐 Win11 `font_size::BODY`）。
pub const ADDRESS_BAR_FONT_SIZE: f32 = unbound_theme::typography::font_size::BODY;
/// 外部导航按钮尺寸（`32×32px`）。
const NAV_BUTTON_SIZE: f32 = 32.0;
/// 外部导航按钮间距（`4px`）。
const NAV_BUTTON_GAP: f32 = 4.0;
/// 三段区域之间的水平间距（`8px`）。
const SECTION_GAP: f32 = 8.0;
/// 传统描边地址框与搜索框圆角（`4px`）。
const BAR_RADIUS: f32 = 4.0;
/// Windows 11 无描边胶囊地址框与搜索框圆角（`6px`）。
const PILL_BAR_RADIUS: f32 = 6.0;
/// 内部按钮与分段胶囊圆角（`4px`）。
const INNER_RADIUS: f32 = 4.0;
/// 地址框左侧根图标尺寸（`16×16px`）。
const ROOT_ICON_SIZE: f32 = 16.0;
/// 面包屑末段右侧保留的最小空白点击编辑触发区宽度（`36px`）。
const MIN_EDIT_TRIGGER_WIDTH: f32 = 36.0;
/// 溢出折叠按钮（`«`）宽度（`24px`）。
const OVERFLOW_BUTTON_WIDTH: f32 = 24.0;
/// 根图标右侧分隔箭头（`›`）宽度（`14px`）。
const ROOT_CHEVRON_WIDTH: f32 = 14.0;
/// 分段右侧子目录展开箭头（`›`）宽度（`18px`）。
const CHEVRON_WIDTH: f32 = 18.0;
/// 地址栏默认深度键（与 `TreeViewDesc` 对齐，确保绘制在容器 `BoxSurface` 背景之上）。
const DEFAULT_ADDRESS_BAR_Z: u16 = 450;
/// 启用搜索框时，低于该总宽则自动折叠为图标按钮。
const SEARCH_COLLAPSE_THRESHOLD: f32 = 420.0;
/// 编辑态主地址框右侧清除按钮（`×`）尺寸（`22×22px`）。
const EDIT_CLEAR_SIZE: f32 = 22.0;
/// 主地址框最右侧历史下拉箭头按钮（`⌄`）宽度（`24px`）。
const HISTORY_DROPDOWN_BUTTON_WIDTH: f32 = 24.0;
/// 历史/补全下拉浮层单行高度（`32px`，适配 `14px` 正文字号）。
const DROPDOWN_ROW_HEIGHT: f32 = 32.0;
/// 历史/补全下拉浮层内边距（`4px`）。
const DROPDOWN_PADDING: f32 = 4.0;
/// 历史/补全下拉浮层与地址框之间的垂直间距（`4px`）。
const DROPDOWN_GAP: f32 = 4.0;
/// 历史/补全下拉浮层最大同时可见条目数。
const DROPDOWN_MAX_VISIBLE_ITEMS: usize = 8;

/// 面包屑路径分段描述。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSegment {
    /// 展示名称（如本地化名称或目录名）。
    pub label: String,
    /// 该段对应的完整规范路径。
    pub path: String,
    /// 可选图标（首段常用于标识根/驱动器）。
    pub icon: Option<ShellIcon>,
    /// 统一图标槽（支持内置矢量与外部彩色纹理，优先于 `icon`）。
    pub widget_icon: WidgetIcon,
    /// 是否存在子目录（决定是否渲染可点击的 `›` 箭头）。
    pub has_subfolders: bool,
}

impl PathSegment {
    /// 创建带标签与规范路径的面包屑分段（默认 `has_subfolders = true`）。
    #[must_use]
    pub fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
            icon: None,
            widget_icon: WidgetIcon::None,
            has_subfolders: true,
        }
    }

    /// 设置该分段的可选图标。
    #[must_use]
    pub fn with_icon(mut self, icon: ShellIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 设置该分段的统一图标（支持内置矢量或外部彩色纹理）。
    #[must_use]
    pub fn with_widget_icon(mut self, icon: WidgetIcon) -> Self {
        self.widget_icon = icon;
        self
    }

    /// 解析该分段实际生效的图标：优先返回 `widget_icon`，其次回退至 `icon`。
    #[must_use]
    pub const fn effective_icon(&self) -> WidgetIcon {
        match self.widget_icon {
            WidgetIcon::None => match self.icon {
                Some(shell_icon) => WidgetIcon::Vector(shell_icon),
                None => WidgetIcon::None,
            },
            other => other,
        }
    }

    /// 设置该分段是否存在子目录（控制 `›` 箭头显隐）。
    #[must_use]
    pub fn with_subfolders(mut self, has_subfolders: bool) -> Self {
        self.has_subfolders = has_subfolders;
        self
    }
}

impl Deref for PathSegment {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.label
    }
}

impl AsRef<str> for PathSegment {
    fn as_ref(&self) -> &str {
        &self.label
    }
}

/// 地址栏下拉浮层条目来源模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressBarDropdownMode {
    /// 最近访问的历史路径列表。
    History,
    /// 路径前缀自动补全建议列表。
    Completion,
}

/// 双模地址栏离散交互事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressBarEvent {
    /// 点击外部后退按钮（`←`）。
    NavigateBack,
    /// 点击外部前进按钮（`→`）。
    NavigateForward,
    /// 点击外部向上一级按钮（`↑`）。
    NavigateUp,
    /// 点击外部刷新按钮（`↻`）。
    Refresh,
    /// 点击根图标右侧分隔箭头（`›`），请求在指定锚点矩形弹出顶级目录菜单。
    RootChevronDropdown { anchor_rect: [i32; 4] },
    /// 点击指定面包屑分段，跳转到对应路径。
    SegmentClicked { index: usize, path: String },
    /// 点击面包屑 `›` 箭头或 `«` 溢出按钮，请求在指定锚点矩形弹出菜单。
    SubfolderMenuRequested {
        index: usize,
        path: String,
        anchor_rect: [i32; 4],
    },
    /// 在原位编辑态按 `Enter` 提交路径文本。
    PathSubmitted(String),
    /// 取消原位路径编辑并恢复面包屑态。
    EditCancelled,
    /// 在编辑态点击右侧清除按钮（`×`）清空了编辑缓冲。
    EditCleared,
    /// 点击历史下拉按钮展开/收起（或在非编辑态按 `Esc` 收起）历史下拉浮层。
    HistoryDropdownToggled(bool),
    /// 在历史下拉浮层中点击或按 `Enter` 选中了历史路径。
    HistorySelected { index: usize, path: String },
    /// 在编辑态按 `Tab`（或点击补全建议项）接受了路径前缀自动补全建议。
    CompletionAccepted(String),
    /// 右侧内联搜索框内容发生变化。
    SearchChanged(String),
    /// 在右侧内联搜索框中按 `Enter` 提交搜索。
    SearchSubmitted(String),
}

/// 地址栏精确命中区域类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressBarHitKind {
    /// 外部后退按钮（`←`）。
    NavBack,
    /// 外部前进按钮（`→`）。
    NavForward,
    /// 外部向上一级按钮（`↑`）。
    NavUp,
    /// 外部刷新按钮（`↻`）。
    Refresh,
    /// 根图标右侧分隔箭头（`›`）。
    RootChevron,
    /// 面包屑前段溢出折叠按钮（`«`）。
    OverflowButton,
    /// 第 `index` 个面包屑分段标签热区。
    Segment(usize),
    /// 第 `index` 个面包屑分段右侧 `›` 子目录箭头热区。
    SegmentChevron(usize),
    /// 末段右侧空白编辑触发区（或编辑态文本框主体）。
    EditTrigger,
    /// 编辑态主地址框右侧清除按钮（`×`）。
    EditClear,
    /// 主地址框最右侧历史下拉箭头按钮（`⌄`）。
    HistoryDropdownButton,
    /// 历史/补全下拉浮层中的第 `index` 行条目。
    DropdownItem(usize),
    /// 右侧内联搜索框主体。
    SearchBox,
    /// 右侧内联搜索框清除按钮（`×`）。
    SearchClear,
}

/// 单个可见面包屑分段的排版矩形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentLayout {
    /// 在 `desc.segments` 中的原始索引。
    pub index: usize,
    /// 段名按钮矩形。
    pub label_rect: Rect,
    /// 可选 `›` 子目录展开箭头矩形（仅 `has_subfolders == true` 时为 `Some`）。
    pub chevron_rect: Option<Rect>,
}

/// 地址栏三段式确定性几何布局（供绘制器与命中测试共用）。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarLayout {
    /// 后退按钮矩形。
    pub back_rect: Option<Rect>,
    /// 前进按钮矩形。
    pub forward_rect: Option<Rect>,
    /// 向上按钮矩形。
    pub up_rect: Option<Rect>,
    /// 刷新按钮矩形。
    pub refresh_rect: Option<Rect>,
    /// 一体化双模主地址框矩形。
    pub address_rect: Rect,
    /// 左侧 `16×16` 根图标矩形。
    pub root_icon_rect: Rect,
    /// 根图标右侧与首个分段之间的分隔箭头（`›`）矩形。
    pub root_chevron_rect: Option<Rect>,
    /// 被折叠的前段数量（大于 0 时渲染 `«` 按钮）。
    pub overflow_count: usize,
    /// `«` 溢出按钮矩形。
    pub overflow_rect: Option<Rect>,
    /// 可见面包屑分段列表。
    pub segments: Vec<SegmentLayout>,
    /// 末段右侧空白编辑触发区矩形。
    pub edit_trigger_rect: Rect,
    /// 编辑态主地址框右侧清除 `×` 按钮矩形。
    pub edit_clear_rect: Option<Rect>,
    /// 主地址框右侧历史下拉箭头 `⌄` 按钮矩形（仅历史路径非空时分配）。
    pub history_dropdown_rect: Option<Rect>,
    /// 可选右侧内联搜索框矩形。
    pub search_rect: Option<Rect>,
    /// 搜索框是否因窄窗折叠为图标按钮。
    pub search_collapsed: bool,
    /// 搜索框放大镜图标矩形。
    pub search_icon_rect: Option<Rect>,
    /// 搜索框清除 `×` 按钮矩形。
    pub search_clear_rect: Option<Rect>,
}

/// 地址栏历史/补全下拉浮层确定性几何布局。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarDropdownLayout {
    /// 当前浮层展示模式（历史路径或自动补全建议）。
    pub mode: AddressBarDropdownMode,
    /// 浮层整体容器矩形（锚定于 `address_rect` 正下方）。
    pub popup_rect: Rect,
    /// 每个可见条目的行矩形列表（索引与源列表前缀一一对应）。
    pub item_rects: Vec<Rect>,
}

/// 地址栏与内联搜索框容器视觉风格。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AddressBarChromeStyle {
    /// Windows 11 风格无外描边半透明圆角胶囊填充（默认）。
    #[default]
    BorderlessPill,
    /// 传统 1px 外描边输入框风格。
    Outlined,
}

/// 内联搜索框放大镜图标置放位置。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SearchIconPlacement {
    /// 图标置放于搜索框右侧（与 Windows 11 资源管理器一致，默认）。
    #[default]
    Right,
    /// 图标置放于搜索框左侧。
    Left,
}

impl AddressBarLayout {
    /// 计算编辑态主地址框内部可用的文本显示与输入区域（自动避让左侧根图标、右侧清除 `×` 与历史下拉按钮）。
    #[must_use]
    pub fn edit_text_rect(&self, show_clear: bool) -> Rect {
        let left_x = self.root_icon_rect.max().x + 6.0;
        let right_x = if show_clear {
            self.edit_clear_rect
                .map_or(self.address_rect.max().x - 8.0, |cr| cr.origin.x - 4.0)
        } else if let Some(hr) = self.history_dropdown_rect {
            hr.origin.x - 4.0
        } else {
            self.address_rect.max().x - 8.0
        };
        Rect::new(
            left_x,
            self.address_rect.origin.y,
            (right_x - left_x).max(0.0),
            self.address_rect.size.y.max(0.0),
        )
    }

    /// 根据当前描述符与编辑/展开状态计算历史或补全下拉浮层的确定性几何布局。
    #[must_use]
    pub fn compute_dropdown_layout(
        &self,
        desc: &AddressBarDesc,
        editing: bool,
        dropdown_open: bool,
    ) -> Option<AddressBarDropdownLayout> {
        if !dropdown_open || self.address_rect.width() <= 0.0 {
            return None;
        }
        let (mode, items) = desc.active_dropdown_items(editing)?;
        let count = items.len().min(DROPDOWN_MAX_VISIBLE_ITEMS);
        if count == 0 {
            return None;
        }
        let popup_rect = Rect::new(
            self.address_rect.origin.x,
            self.address_rect.max().y + DROPDOWN_GAP,
            self.address_rect.width(),
            count as f32 * DROPDOWN_ROW_HEIGHT + DROPDOWN_PADDING * 2.0,
        );
        let row_w = (popup_rect.width() - DROPDOWN_PADDING * 2.0).max(16.0);
        let mut item_rects = Vec::with_capacity(count);
        for idx in 0..count {
            item_rects.push(Rect::new(
                popup_rect.origin.x + DROPDOWN_PADDING,
                popup_rect.origin.y + DROPDOWN_PADDING + idx as f32 * DROPDOWN_ROW_HEIGHT,
                row_w,
                DROPDOWN_ROW_HEIGHT - 2.0,
            ));
        }
        Some(AddressBarDropdownLayout {
            mode,
            popup_rect,
            item_rects,
        })
    }

    /// 计算非折叠内联搜索框内部可用的文本显示与输入区域。
    ///
    /// 自动适配放大镜图标位于左侧或右侧、以及清除 `×` 按钮显隐时的边界留白。
    #[must_use]
    pub fn search_text_rect(&self, show_clear: bool) -> Option<Rect> {
        let sr = self.search_rect?;
        if self.search_collapsed {
            return None;
        }
        let icon_on_right = self
            .search_icon_rect
            .is_some_and(|ir| ir.center().x >= sr.center().x);
        let (left_x, right_x) = if icon_on_right {
            let left = sr.origin.x + 10.0;
            let right = if show_clear {
                self.search_clear_rect
                    .map_or(sr.max().x - 8.0, |cr| cr.origin.x - 4.0)
            } else {
                self.search_icon_rect
                    .map_or(sr.max().x - 8.0, |ir| ir.origin.x - 6.0)
            };
            (left, right)
        } else {
            let left = self
                .search_icon_rect
                .map_or(sr.origin.x + 8.0, |ir| ir.max().x + 6.0);
            let right = if show_clear {
                self.search_clear_rect
                    .map_or(sr.max().x - 6.0, |cr| cr.origin.x - 6.0)
            } else {
                sr.max().x - 6.0
            };
            (left, right)
        };
        let width = (right_x - left_x).max(0.0);
        Some(Rect::new(left_x, sr.origin.y, width, sr.size.y.max(0.0)))
    }
}

/// 双模地址栏描述符（平时显示分段路径面包屑，点击空白或按下 `Ctrl+L` / `Alt+D` 切换为完整路径编辑框）。
#[derive(Clone, Debug, PartialEq)]
pub struct AddressBarDesc {
    pub rect: Rect,
    pub full_path: String,
    pub segments: Vec<PathSegment>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub can_go_up: bool,
    pub show_nav_buttons: bool,
    pub show_refresh: bool,
    pub show_search: bool,
    pub search_placeholder: String,
    /// 容器视觉风格（默认 [`AddressBarChromeStyle::BorderlessPill`]）。
    pub chrome_style: AddressBarChromeStyle,
    /// 搜索框放大镜图标置放位置（默认 [`SearchIconPlacement::Right`]）。
    pub search_icon_placement: SearchIconPlacement,
    /// 最近访问的历史路径列表（非空时显示历史下拉按钮并支持编辑态/点击展开下拉浮层）。
    pub history_paths: Vec<String>,
    /// 路径前缀自动补全建议列表（编辑态非空时优先展示于下拉浮层并支持 `Tab` 接受）。
    pub completion_suggestions: Vec<String>,
    /// 可选单条行内自动补全建议槽位（支持次级色幽灵后缀展示与 `Tab` 接受）。
    pub completion_hint: Option<String>,
    pub z: u16,
}

impl AddressBarDesc {
    /// 从字符串分段列表构建地址栏描述符（保持既有调用点向后兼容）。
    #[must_use]
    pub fn new(rect: Rect, segments: Vec<String>) -> Self {
        let mut path_segments = Vec::with_capacity(segments.len());
        let mut acc = String::new();
        for (idx, label) in segments.into_iter().enumerate() {
            if idx > 0 {
                acc.push('/');
            }
            acc.push_str(&label);
            let mut seg = PathSegment::new(label, acc.clone());
            if idx == 0 {
                seg.icon = Some(ShellIcon::Folder);
            }
            path_segments.push(seg);
        }
        let full_path = if acc.is_empty() { "/".to_owned() } else { acc };
        let can_go_up = path_segments.len() > 1;
        Self {
            rect,
            full_path,
            segments: path_segments,
            can_go_back: true,
            can_go_forward: false,
            can_go_up,
            show_nav_buttons: true,
            show_refresh: true,
            show_search: false,
            search_placeholder: String::new(),
            chrome_style: AddressBarChromeStyle::BorderlessPill,
            search_icon_placement: SearchIconPlacement::Right,
            history_paths: Vec::new(),
            completion_suggestions: Vec::new(),
            completion_hint: None,
            z: DEFAULT_ADDRESS_BAR_Z,
        }
    }

    /// 使用结构化 [`PathSegment`] 列表与完整路径构造地址栏描述符。
    #[must_use]
    pub fn with_path_segments(
        rect: Rect,
        full_path: impl Into<String>,
        segments: Vec<PathSegment>,
    ) -> Self {
        let can_go_up = segments.len() > 1;
        Self {
            rect,
            full_path: full_path.into(),
            segments,
            can_go_back: false,
            can_go_forward: false,
            can_go_up,
            show_nav_buttons: true,
            show_refresh: true,
            show_search: false,
            search_placeholder: String::new(),
            chrome_style: AddressBarChromeStyle::BorderlessPill,
            search_icon_placement: SearchIconPlacement::Right,
            history_paths: Vec::new(),
            completion_suggestions: Vec::new(),
            completion_hint: None,
            z: DEFAULT_ADDRESS_BAR_Z,
        }
    }

    /// 配置后退、前进与向上导航按钮的可用状态。
    #[must_use]
    pub fn with_nav_state(
        mut self,
        can_go_back: bool,
        can_go_forward: bool,
        can_go_up: bool,
    ) -> Self {
        self.can_go_back = can_go_back;
        self.can_go_forward = can_go_forward;
        self.can_go_up = can_go_up;
        self
    }

    /// 控制是否显示外部导航按钮簇（后退/前进/向上）。
    #[must_use]
    pub fn with_nav_buttons(mut self, show: bool) -> Self {
        self.show_nav_buttons = show;
        self
    }

    /// 控制是否显示外部刷新按钮。
    #[must_use]
    pub fn with_refresh(mut self, show: bool) -> Self {
        self.show_refresh = show;
        self
    }

    /// 启用右侧内联搜索框并设置动态占位文本。
    #[must_use]
    pub fn with_search(mut self, placeholder: impl Into<String>) -> Self {
        self.show_search = true;
        self.search_placeholder = placeholder.into();
        self
    }

    /// 隐藏右侧内联搜索框。
    #[must_use]
    pub fn without_search(mut self) -> Self {
        self.show_search = false;
        self
    }

    /// 设置地址栏与搜索框容器视觉风格。
    #[must_use]
    pub fn with_chrome_style(mut self, style: AddressBarChromeStyle) -> Self {
        self.chrome_style = style;
        self
    }

    /// 设置内联搜索框放大镜图标的置放位置（左侧或右侧）。
    #[must_use]
    pub fn with_search_icon_placement(mut self, placement: SearchIconPlacement) -> Self {
        self.search_icon_placement = placement;
        self
    }

    /// 设置最近访问的历史路径列表（用于历史下拉浮层）。
    #[must_use]
    pub fn with_history_paths(mut self, history_paths: Vec<String>) -> Self {
        self.history_paths = history_paths;
        self
    }

    /// 设置路径前缀自动补全建议列表（用于编辑态补全浮层与 `Tab` 接受）。
    #[must_use]
    pub fn with_completion_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.completion_suggestions = suggestions;
        self
    }

    /// 设置行内路径前缀自动补全建议槽位（用于幽灵后缀展示与 `Tab` 接受）。
    #[must_use]
    pub fn with_completion_hint(mut self, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        self.completion_hint = (!hint.is_empty()).then_some(hint);
        self
    }

    /// 解析当前应展示在下拉浮层中的条目模式与只读切片。
    ///
    /// 编辑态下若 `completion_suggestions` 非空则优先展示补全建议，否则回退展示 `history_paths`。
    #[must_use]
    pub fn active_dropdown_items(
        &self,
        editing: bool,
    ) -> Option<(AddressBarDropdownMode, &[String])> {
        if editing && !self.completion_suggestions.is_empty() {
            Some((
                AddressBarDropdownMode::Completion,
                &self.completion_suggestions,
            ))
        } else if !self.history_paths.is_empty() {
            Some((AddressBarDropdownMode::History, &self.history_paths))
        } else {
            None
        }
    }

    /// 计算编辑态下可紧接当前输入展示的自动补全幽灵后缀（优先取 `completion_hint`，其次取 `completion_suggestions` 首项）。
    #[must_use]
    pub fn inline_completion_suffix<'a>(&'a self, current_text: &str) -> Option<&'a str> {
        if current_text.is_empty() {
            return None;
        }
        let candidate = self
            .completion_hint
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.completion_suggestions.first().map(String::as_str))?;
        if candidate.len() <= current_text.len() {
            return None;
        }
        if candidate.starts_with(current_text)
            || (candidate.is_char_boundary(current_text.len())
                && candidate[..current_text.len()].eq_ignore_ascii_case(current_text))
        {
            Some(&candidate[current_text.len()..])
        } else {
            None
        }
    }

    /// 设置绘制深度键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 解析地址栏左侧根图标：优先取首个分段的 [`PathSegment::effective_icon`]，未配置时回退 [`ShellIcon::Folder`]。
    #[must_use]
    pub fn root_icon(&self) -> WidgetIcon {
        match self.segments.first().map(PathSegment::effective_icon) {
            Some(WidgetIcon::None) | None => WidgetIcon::Vector(ShellIcon::Folder),
            Some(icon) => icon,
        }
    }

    /// 完整路径字符串（优先返回显式 `full_path`，否则以 `/` 连接分段）。
    #[must_use]
    pub fn full_path(&self) -> String {
        if !self.full_path.is_empty() {
            return self.full_path.clone();
        }
        if self.segments.is_empty() {
            "/".to_owned()
        } else {
            self.segments
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>()
                .join("/")
        }
    }
}

/// 估算面包屑分段标签的逻辑像素宽度（非 ASCII / CJK 宽字符按 `13.8px`、ASCII 按 `7.5px` 估算）。
#[must_use]
fn estimate_segment_label_width(label: &str) -> f32 {
    let text_w: f32 = label
        .chars()
        .map(|ch| if ch.is_ascii() { 7.5 } else { 13.8 })
        .sum();
    (text_w + 16.0).clamp(32.0, 220.0)
}

/// 计算地址栏三段式确定性几何布局。
#[must_use]
pub fn compute_layout(desc: &AddressBarDesc) -> AddressBarLayout {
    let r = desc.rect;
    let btn_size = NAV_BUTTON_SIZE.min(r.size.y.max(16.0));
    let btn_y = r.origin.y + (r.size.y - btn_size) * 0.5;
    let mut left_x = r.origin.x;

    let (back_rect, forward_rect, up_rect) = if desc.show_nav_buttons {
        let back = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        let forward = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        let up = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        (Some(back), Some(forward), Some(up))
    } else {
        (None, None, None)
    };

    let refresh_rect = if desc.show_refresh {
        let refresh = Rect::new(left_x, btn_y, btn_size, btn_size);
        left_x += btn_size + NAV_BUTTON_GAP;
        Some(refresh)
    } else {
        None
    };

    if desc.show_nav_buttons || desc.show_refresh {
        left_x += SECTION_GAP - NAV_BUTTON_GAP;
    }

    let (search_rect, search_collapsed, search_icon_rect, search_clear_rect, right_limit) = if desc
        .show_search
    {
        let collapsed = r.size.x < SEARCH_COLLAPSE_THRESHOLD;
        let sw = if collapsed {
            r.size.y.clamp(28.0, ADDRESS_BAR_HEIGHT)
        } else {
            (r.size.x * 0.28).clamp(220.0, 340.0)
        };
        let sx = (r.max().x - sw).max(left_x);
        let sr = Rect::new(sx, r.origin.y, (r.max().x - sx).max(0.0), r.size.y);
        let (icon_r, clear_r) = if collapsed {
            let icon_s = 24.0f32.min(sr.size.y).min(sr.size.x);
            let icon_y = sr.origin.y + (sr.size.y - icon_s) * 0.5;
            let ir = Rect::new(
                sr.origin.x + (sr.size.x - icon_s) * 0.5,
                icon_y,
                icon_s,
                icon_s,
            );
            (ir, None)
        } else {
            let icon_s = 16.0f32.min(sr.size.y).min(sr.size.x);
            let icon_y = sr.origin.y + (sr.size.y - icon_s) * 0.5;
            let clear_s = 22.0f32.min(sr.size.y).min(sr.size.x);
            let clear_y = sr.origin.y + (sr.size.y - clear_s) * 0.5;
            match desc.search_icon_placement {
                SearchIconPlacement::Right => {
                    // Windows 11 资源管理器风格：放大镜位于搜索框最右侧，清除按钮紧靠放大镜左侧
                    let icon_x = (sr.max().x - icon_s - 8.0).max(sr.origin.x);
                    let ir = Rect::new(icon_x, icon_y, icon_s, icon_s);
                    let clear_x = (ir.origin.x - clear_s - 2.0).max(sr.origin.x + 4.0);
                    (ir, Some(Rect::new(clear_x, clear_y, clear_s, clear_s)))
                }
                SearchIconPlacement::Left => {
                    let icon_x = (sr.origin.x + 8.0).min((sr.max().x - icon_s).max(sr.origin.x));
                    let ir = Rect::new(icon_x, icon_y, icon_s, icon_s);
                    let clear_x = (sr.max().x - clear_s - 4.0)
                        .max(ir.max().x + 4.0)
                        .min((sr.max().x - clear_s).max(sr.origin.x));
                    (ir, Some(Rect::new(clear_x, clear_y, clear_s, clear_s)))
                }
            }
        };
        (
            Some(sr),
            collapsed,
            Some(icon_r),
            clear_r,
            (sx - SECTION_GAP).max(left_x),
        )
    } else {
        (None, false, None, None, r.max().x)
    };

    let address_rect = Rect::new(
        left_x,
        r.origin.y,
        (right_limit - left_x).max(0.0),
        r.size.y,
    );
    let root_icon_rect = if address_rect.width() < 24.0 {
        Rect::new(
            address_rect.origin.x,
            address_rect.origin.y + (address_rect.size.y - ROOT_ICON_SIZE) * 0.5,
            0.0,
            0.0,
        )
    } else {
        let icon_w = ROOT_ICON_SIZE.min((address_rect.width() - 8.0).max(0.0));
        Rect::new(
            address_rect.origin.x + 8.0,
            address_rect.origin.y + (address_rect.size.y - ROOT_ICON_SIZE) * 0.5,
            icon_w,
            ROOT_ICON_SIZE,
        )
    };

    let inner_y = address_rect.origin.y + 3.0;
    let inner_h = (address_rect.size.y - 6.0).max(16.0);
    let history_dropdown_rect = if !desc.history_paths.is_empty()
        && address_rect.width() >= ROOT_ICON_SIZE + HISTORY_DROPDOWN_BUTTON_WIDTH + 16.0
    {
        let hd_x =
            (address_rect.max().x - HISTORY_DROPDOWN_BUTTON_WIDTH - 4.0).max(address_rect.origin.x);
        Some(Rect::new(
            hd_x,
            inner_y,
            HISTORY_DROPDOWN_BUTTON_WIDTH,
            inner_h,
        ))
    } else {
        None
    };
    let clear_s = EDIT_CLEAR_SIZE
        .min(address_rect.size.y)
        .min(address_rect.size.x);
    let clear_y = address_rect.origin.y + (address_rect.size.y - clear_s) * 0.5;
    let clear_right =
        history_dropdown_rect.map_or(address_rect.max().x - 4.0, |hr| hr.origin.x - 2.0);
    let clear_x = (clear_right - clear_s).max(root_icon_rect.max().x + 4.0);
    let edit_clear_rect = (clear_s > 0.0 && clear_x + clear_s <= address_rect.max().x)
        .then_some(Rect::new(clear_x, clear_y, clear_s, clear_s));
    let right_content_edge =
        history_dropdown_rect.map_or(address_rect.max().x, |hr| hr.origin.x - 2.0);

    // 根图标后置 `›` 分隔符，使面包屑呈现 `[图标] > 分段1 > 分段2 >`
    let root_chevron_rect = if !desc.segments.is_empty()
        && root_icon_rect.max().x + 2.0 + ROOT_CHEVRON_WIDTH <= right_content_edge
    {
        Some(Rect::new(
            root_icon_rect.max().x + 2.0,
            inner_y,
            ROOT_CHEVRON_WIDTH,
            inner_h,
        ))
    } else {
        None
    };
    let content_start_x =
        root_chevron_rect.map_or(root_icon_rect.max().x + 4.0, |rc| rc.max().x + 2.0);
    let available_total = (right_content_edge - content_start_x - MIN_EDIT_TRIGGER_WIDTH).max(0.0);

    // 预估每段所需宽度（标签 + 可选 `›` 箭头），对 CJK 宽字符按双倍宽度系数估算
    let seg_widths: Vec<(f32, f32)> = desc
        .segments
        .iter()
        .map(|seg| {
            let label_w = estimate_segment_label_width(&seg.label);
            let chev_w = if seg.has_subfolders {
                CHEVRON_WIDTH
            } else {
                0.0
            };
            (label_w, chev_w)
        })
        .collect();

    let total_segs_w: f32 = seg_widths.iter().map(|(lw, cw)| lw + cw + 2.0).sum();

    let mut overflow_count = 0usize;
    if total_segs_w > available_total && desc.segments.len() > 1 {
        let budget_with_overflow = (available_total - OVERFLOW_BUTTON_WIDTH - 2.0).max(0.0);
        let mut remaining_w = total_segs_w;
        while overflow_count + 1 < desc.segments.len() && remaining_w > budget_with_overflow {
            let (lw, cw) = seg_widths[overflow_count];
            remaining_w -= lw + cw + 2.0;
            overflow_count += 1;
        }
    }

    let mut cursor_x = content_start_x;
    let overflow_rect = if overflow_count > 0 {
        let ov = Rect::new(cursor_x, inner_y, OVERFLOW_BUTTON_WIDTH, inner_h);
        cursor_x = ov.max().x + 2.0;
        Some(ov)
    } else {
        None
    };

    let mut segments = Vec::with_capacity(desc.segments.len().saturating_sub(overflow_count));
    let seg_right_limit = (right_content_edge - MIN_EDIT_TRIGGER_WIDTH).max(cursor_x);
    for (idx, &(label_w, chev_w)) in seg_widths.iter().enumerate().skip(overflow_count) {
        let max_w = (seg_right_limit - cursor_x).max(0.0);
        if max_w < 16.0 && !segments.is_empty() {
            break;
        }
        let actual_label_w = label_w.min((max_w - chev_w).max(16.0));
        let label_rect = Rect::new(cursor_x, inner_y, actual_label_w, inner_h);
        cursor_x = label_rect.max().x;
        let chevron_rect = if chev_w > 0.0 && cursor_x + chev_w <= right_content_edge {
            let cr = Rect::new(cursor_x, inner_y, chev_w, inner_h);
            cursor_x = cr.max().x;
            Some(cr)
        } else {
            None
        };
        cursor_x += 2.0;
        segments.push(SegmentLayout {
            index: idx,
            label_rect,
            chevron_rect,
        });
    }

    let edit_trigger_rect = Rect::new(
        cursor_x.min(right_content_edge),
        address_rect.origin.y,
        (right_content_edge - cursor_x).max(0.0),
        address_rect.size.y,
    );

    AddressBarLayout {
        back_rect,
        forward_rect,
        up_rect,
        refresh_rect,
        address_rect,
        root_icon_rect,
        root_chevron_rect,
        overflow_count,
        overflow_rect,
        segments,
        edit_trigger_rect,
        edit_clear_rect,
        history_dropdown_rect,
        search_rect,
        search_collapsed,
        search_icon_rect,
        search_clear_rect,
    }
}

/// 双模地址栏交互状态（内嵌复用 [`TextEditState`] 编辑核心）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddressBarState {
    /// 是否处于全路径文本编辑模式。
    pub editing: bool,
    /// 编辑缓冲区内容（与 `edit_core.text()` 同步，保持向后兼容）。
    pub edit_buffer: String,
    /// 历史后退栈。
    pub back_stack: Vec<Vec<String>>,
    /// 历史前进栈。
    pub forward_stack: Vec<Vec<String>>,
    /// 全路径原位编辑核心（支持光标、选区、撤销重做、剪贴板与 IME）。
    pub edit_core: TextEditState,
    /// 右侧内联搜索框编辑核心。
    pub search_core: TextEditState,
    /// 右侧内联搜索框是否处于聚焦态。
    pub search_focused: bool,
    /// 待宿主消费的离散交互事件队列。
    pub pending_events: Vec<AddressBarEvent>,
    /// 缓存的后退可用状态。
    pub can_go_back: bool,
    /// 缓存的前进可用状态。
    pub can_go_forward: bool,
    /// 缓存的向上一级可用状态。
    pub can_go_up: bool,
    /// 历史/补全下拉浮层是否处于展开状态。
    pub dropdown_open: bool,
    /// 历史/补全下拉浮层当前键盘高亮条目索引。
    pub dropdown_highlighted: Option<usize>,
    /// 当前指针悬停的子热区（用于高亮对应按钮或分段）。
    pub hovered_hit: Option<AddressBarHitKind>,
    /// 当前指针按下的子热区（用于高亮对应按钮或分段的按压态）。
    pub pressed_hit: Option<AddressBarHitKind>,
}

impl AddressBarState {
    /// 创建默认地址栏状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出并清空待处理的地址栏交互事件。
    pub fn drain_events(&mut self) -> Vec<AddressBarEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// 返回当前活动编辑缓冲区文本（兼容仅写入 `edit_buffer` 的既有调用点）。
    #[must_use]
    pub fn current_edit_text(&self) -> &str {
        if self.edit_core.text().is_empty() && !self.edit_buffer.is_empty() {
            self.edit_buffer.as_str()
        } else {
            self.edit_core.text()
        }
    }

    /// 切换进入全路径编辑态并全选路径文本（同时打开下拉浮层以便有历史/补全条目时立即展示）。
    pub fn begin_edit(&mut self, current_path: &str) {
        self.editing = true;
        self.search_focused = false;
        self.dropdown_open = true;
        self.dropdown_highlighted = None;
        self.edit_buffer = current_path.to_owned();
        self.edit_core.set_text(current_path);
        self.edit_core.select_all();
    }

    /// 清空编辑态路径缓冲并派发 [`AddressBarEvent::EditCleared`]。
    pub fn clear_edit(&mut self) -> bool {
        if !self.editing || self.current_edit_text().is_empty() {
            return false;
        }
        self.edit_core.set_text("");
        self.edit_buffer.clear();
        self.dropdown_highlighted = None;
        self.pending_events.push(AddressBarEvent::EditCleared);
        true
    }

    /// 显式展开历史路径下拉浮层并发射 [`AddressBarEvent::HistoryDropdownToggled`]。
    pub fn open_history_dropdown(&mut self, desc: &AddressBarDesc) -> bool {
        if desc.history_paths.is_empty() {
            return false;
        }
        self.dropdown_open = true;
        self.dropdown_highlighted = None;
        self.search_focused = false;
        self.pending_events
            .push(AddressBarEvent::HistoryDropdownToggled(true));
        true
    }

    /// 收起当前展开的历史或补全下拉浮层。
    pub fn close_dropdown(&mut self) -> bool {
        if !self.dropdown_open {
            return false;
        }
        self.dropdown_open = false;
        self.dropdown_highlighted = None;
        true
    }

    /// 解析当前 `Tab` 按键应接受的路径前缀自动补全目标。
    #[must_use]
    pub fn resolve_completion(&self, desc: &AddressBarDesc) -> Option<String> {
        if !self.editing {
            return None;
        }
        if let Some((AddressBarDropdownMode::Completion, items)) = desc.active_dropdown_items(true)
        {
            return self
                .dropdown_highlighted
                .and_then(|idx| items.get(idx))
                .or_else(|| items.first())
                .cloned();
        }
        if let Some(hint) = desc.completion_hint.as_deref().filter(|s| !s.is_empty()) {
            return Some(hint.to_owned());
        }
        if self.dropdown_open {
            if let Some(idx) = self.dropdown_highlighted {
                if let Some(path) = desc.history_paths.get(idx) {
                    return Some(path.clone());
                }
            }
        }
        None
    }

    /// 接受当前路径前缀自动补全建议，写回编辑缓冲并派发 [`AddressBarEvent::CompletionAccepted`]。
    pub fn accept_completion(&mut self, desc: &AddressBarDesc) -> bool {
        let Some(accepted) = self.resolve_completion(desc) else {
            return false;
        };
        self.edit_core.set_text(&accepted);
        self.edit_buffer = accepted.clone();
        self.dropdown_highlighted = None;
        self.pending_events
            .push(AddressBarEvent::CompletionAccepted(accepted));
        true
    }

    fn move_dropdown_highlight(
        &mut self,
        desc: &AddressBarDesc,
        editing: bool,
        delta: isize,
    ) -> bool {
        let Some((_, items)) = desc.active_dropdown_items(editing) else {
            return false;
        };
        let visible_len = items.len().min(DROPDOWN_MAX_VISIBLE_ITEMS);
        if visible_len == 0 {
            return false;
        }
        self.dropdown_open = true;
        self.dropdown_highlighted = Some(match self.dropdown_highlighted {
            None => {
                if delta >= 0 {
                    0
                } else {
                    visible_len - 1
                }
            }
            Some(curr) => {
                let clamped = curr.min(visible_len - 1) as isize;
                (clamped + delta).rem_euclid(visible_len as isize) as usize
            }
        });
        true
    }

    /// 提交编辑路径、发射 [`AddressBarEvent::PathSubmitted`] 并解析为分段列表。
    pub fn commit_edit(&mut self) -> Vec<String> {
        self.editing = false;
        self.dropdown_open = false;
        self.dropdown_highlighted = None;
        let text = self.current_edit_text().to_owned();
        self.edit_buffer = text.clone();
        self.pending_events
            .push(AddressBarEvent::PathSubmitted(text.clone()));
        text.split(['/', '\\'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// 取消全路径编辑并恢复面包屑态，发射 [`AddressBarEvent::EditCancelled`]。
    pub fn cancel_edit(&mut self) {
        if self.editing {
            self.editing = false;
            self.dropdown_open = false;
            self.dropdown_highlighted = None;
            self.pending_events.push(AddressBarEvent::EditCancelled);
        }
    }

    /// 失焦处理：若正在编辑路径则取消编辑；同时取消搜索框聚焦并收起下拉浮层。
    pub fn handle_blur(&mut self) {
        self.cancel_edit();
        self.search_focused = false;
        self.dropdown_open = false;
        self.dropdown_highlighted = None;
    }

    /// 清空右侧内联搜索框内容并发射 `SearchChanged("")`。
    pub fn clear_search(&mut self) -> bool {
        if self.search_core.text().is_empty() {
            return false;
        }
        self.search_core.set_text("");
        self.pending_events
            .push(AddressBarEvent::SearchChanged(String::new()));
        true
    }

    /// 对指定坐标执行精确命中测试。
    #[must_use]
    pub fn hit_test(&self, desc: &AddressBarDesc, point: Vec2) -> Option<AddressBarHitKind> {
        let layout = compute_layout(desc);
        if let Some(dd) = layout.compute_dropdown_layout(desc, self.editing, self.dropdown_open) {
            if dd.popup_rect.contains_point(point) {
                for (idx, row_r) in dd.item_rects.iter().enumerate() {
                    if row_r.contains_point(point) {
                        return Some(AddressBarHitKind::DropdownItem(idx));
                    }
                }
            }
        }
        if layout.back_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::NavBack);
        }
        if layout.forward_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::NavForward);
        }
        if layout.up_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::NavUp);
        }
        if layout.refresh_rect.is_some_and(|r| r.contains_point(point)) {
            return Some(AddressBarHitKind::Refresh);
        }
        if let Some(sr) = layout.search_rect {
            if sr.contains_point(point) {
                if !layout.search_collapsed
                    && !self.search_core.text().is_empty()
                    && layout
                        .search_clear_rect
                        .is_some_and(|cr| cr.contains_point(point))
                {
                    return Some(AddressBarHitKind::SearchClear);
                }
                return Some(AddressBarHitKind::SearchBox);
            }
        }
        if layout.address_rect.contains_point(point) {
            if layout
                .history_dropdown_rect
                .is_some_and(|r| r.contains_point(point))
            {
                return Some(AddressBarHitKind::HistoryDropdownButton);
            }
            if self.editing {
                if !self.current_edit_text().is_empty()
                    && layout
                        .edit_clear_rect
                        .is_some_and(|r| r.contains_point(point))
                {
                    return Some(AddressBarHitKind::EditClear);
                }
                return Some(AddressBarHitKind::EditTrigger);
            }
            if layout
                .root_chevron_rect
                .is_some_and(|r| r.contains_point(point))
            {
                return Some(AddressBarHitKind::RootChevron);
            }
            if layout
                .overflow_rect
                .is_some_and(|r| r.contains_point(point))
            {
                return Some(AddressBarHitKind::OverflowButton);
            }
            for seg in &layout.segments {
                if seg.chevron_rect.is_some_and(|r| r.contains_point(point)) {
                    return Some(AddressBarHitKind::SegmentChevron(seg.index));
                }
                if seg.label_rect.contains_point(point) {
                    return Some(AddressBarHitKind::Segment(seg.index));
                }
            }
            return Some(AddressBarHitKind::EditTrigger);
        }
        None
    }

    /// 处理指针左键点击事件（纯状态机，返回是否消费该点击）。
    pub fn handle_click(&mut self, desc: &AddressBarDesc, point: Vec2) -> bool {
        self.can_go_back = desc.can_go_back;
        self.can_go_forward = desc.can_go_forward;
        self.can_go_up = desc.can_go_up;

        let layout = compute_layout(desc);
        let hit = self.hit_test(desc, point);
        match hit {
            Some(AddressBarHitKind::NavBack) => {
                if !desc.can_go_back {
                    return false;
                }
                self.close_dropdown();
                self.pending_events.push(AddressBarEvent::NavigateBack);
                true
            }
            Some(AddressBarHitKind::NavForward) => {
                if !desc.can_go_forward {
                    return false;
                }
                self.close_dropdown();
                self.pending_events.push(AddressBarEvent::NavigateForward);
                true
            }
            Some(AddressBarHitKind::NavUp) => {
                if !desc.can_go_up {
                    return false;
                }
                self.close_dropdown();
                self.pending_events.push(AddressBarEvent::NavigateUp);
                true
            }
            Some(AddressBarHitKind::Refresh) => {
                self.close_dropdown();
                self.pending_events.push(AddressBarEvent::Refresh);
                true
            }
            Some(AddressBarHitKind::RootChevron) => {
                if let Some(rc_r) = layout.root_chevron_rect {
                    self.close_dropdown();
                    self.pending_events
                        .push(AddressBarEvent::RootChevronDropdown {
                            anchor_rect: rect_to_i32(rc_r),
                        });
                    return true;
                }
                false
            }
            Some(AddressBarHitKind::OverflowButton) => {
                if let (Some(ov_r), Some(first)) = (layout.overflow_rect, desc.segments.first()) {
                    self.close_dropdown();
                    self.pending_events
                        .push(AddressBarEvent::SubfolderMenuRequested {
                            index: 0,
                            path: first.path.clone(),
                            anchor_rect: rect_to_i32(ov_r),
                        });
                    return true;
                }
                false
            }
            Some(AddressBarHitKind::Segment(index)) => {
                if let Some(seg) = desc.segments.get(index) {
                    self.close_dropdown();
                    self.pending_events.push(AddressBarEvent::SegmentClicked {
                        index,
                        path: seg.path.clone(),
                    });
                    return true;
                }
                false
            }
            Some(AddressBarHitKind::SegmentChevron(index)) => {
                if let (Some(seg), Some(seg_layout)) = (
                    desc.segments.get(index),
                    layout.segments.iter().find(|s| s.index == index),
                ) {
                    if let Some(chev_r) = seg_layout.chevron_rect {
                        self.close_dropdown();
                        self.pending_events
                            .push(AddressBarEvent::SubfolderMenuRequested {
                                index,
                                path: seg.path.clone(),
                                anchor_rect: rect_to_i32(chev_r),
                            });
                        return true;
                    }
                }
                false
            }
            Some(AddressBarHitKind::EditTrigger) => {
                if !self.editing {
                    self.begin_edit(&desc.full_path());
                }
                true
            }
            Some(AddressBarHitKind::EditClear) => self.clear_edit(),
            Some(AddressBarHitKind::HistoryDropdownButton) => {
                self.search_focused = false;
                self.dropdown_open = !self.dropdown_open;
                self.dropdown_highlighted = None;
                self.pending_events
                    .push(AddressBarEvent::HistoryDropdownToggled(self.dropdown_open));
                true
            }
            Some(AddressBarHitKind::DropdownItem(index)) => {
                if let Some((mode, items)) = desc.active_dropdown_items(self.editing) {
                    if let Some(path) = items.get(index).cloned() {
                        match mode {
                            AddressBarDropdownMode::History => {
                                self.editing = false;
                                self.dropdown_open = false;
                                self.dropdown_highlighted = None;
                                self.edit_buffer = path.clone();
                                self.edit_core.set_text(&path);
                                self.pending_events
                                    .push(AddressBarEvent::HistorySelected { index, path });
                                return true;
                            }
                            AddressBarDropdownMode::Completion => {
                                self.edit_core.set_text(&path);
                                self.edit_buffer = path.clone();
                                self.dropdown_highlighted = None;
                                self.pending_events
                                    .push(AddressBarEvent::CompletionAccepted(path));
                                return true;
                            }
                        }
                    }
                }
                false
            }
            Some(AddressBarHitKind::SearchClear) => self.clear_search(),
            Some(AddressBarHitKind::SearchBox) => {
                self.cancel_edit();
                self.close_dropdown();
                self.search_focused = true;
                true
            }
            None => {
                let was_active = self.editing || self.search_focused || self.dropdown_open;
                self.handle_blur();
                was_active
            }
        }
    }

    /// 处理键盘快捷键与文本编辑按键（含 `Ctrl+L`、`Alt+D`、`Enter`、`Esc`）。
    pub fn handle_key(&mut self, desc: &AddressBarDesc, chord: KeyChord) -> bool {
        // `Ctrl+L` 或 `Alt+D`：随时切入全路径原位编辑态并全选
        if (chord.ctrl && matches!(chord.key, KeyName::Char('l' | 'L')))
            || (chord.alt && matches!(chord.key, KeyName::Char('d' | 'D')))
        {
            self.begin_edit(&desc.full_path());
            return true;
        }

        if self.editing {
            match chord.key {
                KeyName::Named("Tab") if !chord.ctrl && !chord.alt && !chord.meta => {
                    return self.accept_completion(desc);
                }
                KeyName::Named("ArrowDown") if !chord.ctrl && !chord.alt && !chord.meta => {
                    if self.move_dropdown_highlight(desc, true, 1) {
                        return true;
                    }
                }
                KeyName::Named("ArrowUp") if !chord.ctrl && !chord.alt && !chord.meta => {
                    if self.move_dropdown_highlight(desc, true, -1) {
                        return true;
                    }
                }
                KeyName::Named("Enter") => {
                    if self.dropdown_open {
                        if let Some(idx) = self.dropdown_highlighted {
                            if let Some((mode, items)) = desc.active_dropdown_items(true) {
                                if let Some(path) = items.get(idx).cloned() {
                                    self.editing = false;
                                    self.dropdown_open = false;
                                    self.dropdown_highlighted = None;
                                    self.edit_buffer = path.clone();
                                    self.edit_core.set_text(&path);
                                    match mode {
                                        AddressBarDropdownMode::History => {
                                            self.pending_events.push(
                                                AddressBarEvent::HistorySelected {
                                                    index: idx,
                                                    path,
                                                },
                                            );
                                        }
                                        AddressBarDropdownMode::Completion => {
                                            self.pending_events
                                                .push(AddressBarEvent::PathSubmitted(path));
                                        }
                                    }
                                    return true;
                                }
                            }
                        }
                    }
                    self.commit_edit();
                    return true;
                }
                KeyName::Named("Escape") => {
                    self.cancel_edit();
                    return true;
                }
                _ => {
                    if let Some(ev) = chord_to_text_event(chord) {
                        let outcome = self.handle_text_event(desc, &ev);
                        return outcome.changed
                            || outcome.copied.is_some()
                            || outcome.cut.is_some()
                            || outcome.paste_requested;
                    }
                }
            }
            return false;
        }

        if self.dropdown_open {
            match chord.key {
                KeyName::Named("ArrowDown") if !chord.ctrl && !chord.alt && !chord.meta => {
                    return self.move_dropdown_highlight(desc, false, 1);
                }
                KeyName::Named("ArrowUp") if !chord.ctrl && !chord.alt && !chord.meta => {
                    return self.move_dropdown_highlight(desc, false, -1);
                }
                KeyName::Named("Enter") => {
                    if let Some(idx) = self.dropdown_highlighted {
                        if let Some((_, items)) = desc.active_dropdown_items(false) {
                            if let Some(path) = items.get(idx).cloned() {
                                self.dropdown_open = false;
                                self.dropdown_highlighted = None;
                                self.edit_buffer = path.clone();
                                self.edit_core.set_text(&path);
                                self.pending_events
                                    .push(AddressBarEvent::HistorySelected { index: idx, path });
                                return true;
                            }
                        }
                    }
                }
                KeyName::Named("Escape") => {
                    self.dropdown_open = false;
                    self.dropdown_highlighted = None;
                    self.pending_events
                        .push(AddressBarEvent::HistoryDropdownToggled(false));
                    return true;
                }
                _ => {}
            }
        }

        if self.search_focused {
            match chord.key {
                KeyName::Named("Enter") => {
                    self.pending_events.push(AddressBarEvent::SearchSubmitted(
                        self.search_core.text().to_owned(),
                    ));
                    return true;
                }
                KeyName::Named("Escape") => {
                    self.search_focused = false;
                    return true;
                }
                _ => {
                    if let Some(ev) = chord_to_text_event(chord) {
                        let outcome = self.handle_text_event(desc, &ev);
                        return outcome.changed
                            || outcome.copied.is_some()
                            || outcome.cut.is_some()
                            || outcome.paste_requested;
                    }
                }
            }
        }

        false
    }

    /// 将完整文本输入事件（含 IME 预编辑/提交/取消、剪贴板粘贴/复制）分发至当前激活的 `TextEditState`。
    pub fn handle_text_event(
        &mut self,
        desc: &AddressBarDesc,
        event: &TextInputEvent,
    ) -> TextInputOutcome {
        if self.editing {
            let editor_desc = TextInputDesc::new(desc.rect);
            let outcome = reduce_text_input(&editor_desc, &mut self.edit_core, event);
            if outcome.changed {
                self.edit_buffer = self.edit_core.text().to_owned();
                self.dropdown_highlighted = None;
                self.dropdown_open = true;
            }
            return outcome;
        }
        if self.search_focused {
            let prev = self.search_core.text().to_owned();
            let editor_desc = TextInputDesc::new(desc.rect);
            let outcome = reduce_text_input(&editor_desc, &mut self.search_core, event);
            if self.search_core.text() != prev {
                self.pending_events.push(AddressBarEvent::SearchChanged(
                    self.search_core.text().to_owned(),
                ));
            }
            return outcome;
        }
        TextInputOutcome::default()
    }
}

/// 为地址栏子热区生成确定性子节点键。
#[must_use]
pub const fn address_bar_sub_node(node: u64, kind: AddressBarHitKind) -> u64 {
    let salt: u64 = match kind {
        AddressBarHitKind::NavBack => 0x4142_0001,
        AddressBarHitKind::NavForward => 0x4142_0002,
        AddressBarHitKind::NavUp => 0x4142_0003,
        AddressBarHitKind::Refresh => 0x4142_0004,
        AddressBarHitKind::RootChevron => 0x4142_0009,
        AddressBarHitKind::OverflowButton => 0x4142_0005,
        AddressBarHitKind::EditTrigger => 0x4142_0006,
        AddressBarHitKind::SearchBox => 0x4142_0007,
        AddressBarHitKind::SearchClear => 0x4142_0008,
        AddressBarHitKind::EditClear => 0x4142_000A,
        AddressBarHitKind::HistoryDropdownButton => 0x4142_000B,
        AddressBarHitKind::Segment(idx) => 0x4142_1000 + idx as u64,
        AddressBarHitKind::SegmentChevron(idx) => 0x4142_2000 + idx as u64,
        AddressBarHitKind::DropdownItem(idx) => 0x4142_3000 + idx as u64,
    };
    node ^ salt
}

fn rect_to_i32(r: Rect) -> [i32; 4] {
    [
        r.origin.x.round() as i32,
        r.origin.y.round() as i32,
        r.size.x.round() as i32,
        r.size.y.round() as i32,
    ]
}

fn chord_to_text_event(chord: KeyChord) -> Option<TextInputEvent> {
    if chord.ctrl && !chord.alt && !chord.meta {
        return match chord.key {
            KeyName::Char('a' | 'A') => Some(TextInputEvent::SelectAll),
            KeyName::Char('c' | 'C') => Some(TextInputEvent::Copy),
            KeyName::Char('x' | 'X') => Some(TextInputEvent::Cut),
            KeyName::Char('z' | 'Z') if chord.shift => Some(TextInputEvent::Redo),
            KeyName::Char('z' | 'Z') => Some(TextInputEvent::Undo),
            KeyName::Char('y' | 'Y') => Some(TextInputEvent::Redo),
            KeyName::Named("ArrowLeft") => Some(TextInputEvent::MoveWordLeft {
                extend: chord.shift,
            }),
            KeyName::Named("ArrowRight") => Some(TextInputEvent::MoveWordRight {
                extend: chord.shift,
            }),
            KeyName::Named("Backspace") => Some(TextInputEvent::DeleteWordBack),
            KeyName::Named("Delete") => Some(TextInputEvent::DeleteWordForward),
            _ => None,
        };
    }
    if chord.alt || chord.meta {
        return None;
    }
    match chord.key {
        KeyName::Named("Backspace") => Some(TextInputEvent::Backspace),
        KeyName::Named("Delete") => Some(TextInputEvent::Delete),
        KeyName::Named("ArrowLeft") => Some(TextInputEvent::MoveLeft {
            extend: chord.shift,
        }),
        KeyName::Named("ArrowRight") => Some(TextInputEvent::MoveRight {
            extend: chord.shift,
        }),
        KeyName::Named("Home") => Some(TextInputEvent::Home {
            extend: chord.shift,
        }),
        KeyName::Named("End") => Some(TextInputEvent::End {
            extend: chord.shift,
        }),
        KeyName::Char(ch) if !ch.is_control() => Some(TextInputEvent::Insert(ch.to_string())),
        _ => None,
    }
}

/// 绘制双左折线溢出图标 `«`（恒定 4 条线段）。
fn emit_overflow_chevrons(
    rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    for dx in [-2.8f32, 2.2] {
        let cx = c.x + dx;
        curve_stroke(
            [cx + 2.0, c.y - 3.2],
            [cx - 1.8, c.y],
            1.4,
            color,
            z,
            primitives,
            payloads,
        );
        curve_stroke(
            [cx - 1.8, c.y],
            [cx + 2.0, c.y + 3.2],
            1.4,
            color,
            z,
            primitives,
            payloads,
        );
    }
}

/// 绘制单个分段右侧子目录展开箭头 `›`（恒定 2 条线段）。
fn emit_segment_chevron(
    rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    curve_stroke(
        [c.x - 1.8, c.y - 3.2],
        [c.x + 2.0, c.y],
        1.4,
        color,
        z,
        primitives,
        payloads,
    );
    curve_stroke(
        [c.x + 2.0, c.y],
        [c.x - 1.8, c.y + 3.2],
        1.4,
        color,
        z,
        primitives,
        payloads,
    );
}

/// 地址栏编辑态与内联搜索框的已测量文本视觉信息（光标、选区与 IME 预编辑下划线）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AddressBarTextVisual {
    /// 主地址栏编辑态光标、选区与 IME 几何信息（仅 `state.editing == true` 时生效）。
    pub edit_visual: TextInputVisual,
    /// 右侧内联搜索框光标、选区与 IME 几何信息（仅 `state.search_focused == true` 且非折叠时生效）。
    pub search_visual: TextInputVisual,
}

impl Default for AddressBarTextVisual {
    fn default() -> Self {
        let hidden = TextInputVisual {
            caret: Rect::ZERO,
            selection: None,
            preedit: None,
            caret_visible: false,
        };
        Self {
            edit_visual: hidden,
            search_visual: hidden,
        }
    }
}

/// 根据编辑核心状态与文本测量回调计算光标竖线、选区高亮矩形与 IME 预编辑下划线矩形。
#[must_use]
pub fn compute_text_edit_visual<F>(
    edit: &TextEditState,
    text_rect: Rect,
    active: bool,
    mut measure_width: F,
) -> TextInputVisual
where
    F: FnMut(&str) -> f32,
{
    let text = edit.text();
    let scroll_x = edit.scroll_x();
    let x_at = |byte_offset: usize, measure: &mut F| -> f32 {
        let clamped = floor_boundary(text, byte_offset.min(text.len()));
        let prefix_w = if clamped == 0 {
            0.0
        } else {
            measure(&text[..clamped])
        };
        (text_rect.origin.x + prefix_w - scroll_x).clamp(text_rect.origin.x, text_rect.max().x)
    };

    let caret_x = x_at(edit.cursor(), &mut measure_width);
    let rect_h = text_rect.size.y.max(0.0);
    let caret_h = (rect_h - 12.0).max(12.0).min(rect_h);
    let caret_y = text_rect.origin.y + (rect_h - caret_h) * 0.5;
    let caret = Rect::new(caret_x, caret_y, 1.5, caret_h);

    let selection = edit.selection().and_then(|(start, end)| {
        let x0 = x_at(start, &mut measure_width);
        let x1 = x_at(end, &mut measure_width);
        let w = (x1 - x0).max(0.0);
        (w > 0.5).then_some(Rect::new(x0, caret_y, w, caret_h))
    });

    let preedit = edit.preedit().and_then(|(start, end)| {
        let x0 = x_at(start, &mut measure_width);
        let x1 = x_at(end, &mut measure_width);
        let w = (x1 - x0).max(0.0);
        (w > 0.5).then_some(Rect::new(x0, caret_y + caret_h, w, 1.5))
    });

    TextInputVisual {
        caret,
        selection,
        preedit,
        caret_visible: active,
    }
}

/// 发射编辑态选区高亮矩形（槽位恒定，通过不透明度切换显隐）。
fn push_selection_overlay(
    visual: &TextInputVisual,
    active: bool,
    disabled: bool,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
) {
    let mut selection = surface(
        visual.selection.unwrap_or(Rect::ZERO),
        2.0,
        Token::SELECTION,
        z,
    );
    selection.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::InnerShadow as u16,
    );
    selection.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z,
        if active && !disabled && visual.selection.is_some() {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(selection);
}

/// 发射编辑态光标竖线与 IME 预编辑下划线（槽位恒定，通过不透明度切换显隐）。
fn push_caret_and_preedit_overlays(
    visual: &TextInputVisual,
    active: bool,
    has_preedit: bool,
    disabled: bool,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
) {
    let mut caret = surface(visual.caret, 0.0, Token::TEXT_PRIMARY, z);
    caret.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    caret.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z,
        if active && !disabled && visual.caret_visible {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    let mut preedit = surface(
        visual.preedit.unwrap_or(Rect::ZERO),
        0.0,
        Token::PREEDIT_UNDERLINE,
        z,
    );
    preedit.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    preedit.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        z,
        if active && !disabled && has_preedit && visual.preedit.is_some() {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(preedit);
}

/// 发射地址栏单个文本裁剪层（用于路径区或搜索区在独立 `push_clip` 作用域内绘制主文本、次级占位文本与光标/选区/IME）。
#[must_use]
pub fn emit_address_bar_text_layer(
    primary_glyphs: &[GlyphQuad],
    placeholder_glyphs: &[GlyphQuad],
    visual: &TextInputVisual,
    active: bool,
    has_preedit: bool,
    disabled: bool,
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + primary_glyphs.len() + placeholder_glyphs.len());
    let mut payloads = Vec::with_capacity(primary_glyphs.len() + placeholder_glyphs.len());
    let text_z = z.saturating_add(2);

    push_selection_overlay(visual, active, disabled, text_z, &mut primitives);
    emit_glyphs(
        primary_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        text_z,
        &mut primitives,
        &mut payloads,
    );
    emit_glyphs(
        placeholder_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        },
        text_z,
        &mut primitives,
        &mut payloads,
    );
    push_caret_and_preedit_overlays(
        visual,
        active,
        has_preedit,
        disabled,
        text_z,
        &mut primitives,
    );

    WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    }
}

/// 发射地址栏历史/补全下拉浮层的条目文本裁剪层。
#[must_use]
pub fn emit_address_bar_dropdown_text_layer(
    dropdown_glyphs: &[GlyphQuad],
    disabled: bool,
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(dropdown_glyphs.len());
    let mut payloads = Vec::with_capacity(dropdown_glyphs.len());
    emit_glyphs(
        dropdown_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z.saturating_add(6),
        &mut primitives,
        &mut payloads,
    );
    WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    }
}

/// 发射双模复合地址栏（外部导航按钮簇 + 一体化双模地址框 + 可选内联搜索框，严格保持 24 态恒定图元数）。
#[must_use]
pub fn emit_address_bar(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    emit_address_bar_with_visual(
        node,
        desc,
        state,
        control,
        glyphs,
        &[],
        &AddressBarTextVisual::default(),
    )
}

/// 发射双模复合地址栏外框、导航按钮、图标、子热区与无障碍节点（不含文本与光标/选区/IME 占位图元）。
///
/// 适用于宿主在独立 `push_clip` 裁剪层中通过 [`emit_address_bar_text_layer`] 发射文本与编辑叠加层的场景，
/// 避免在底层外框中重复发射不可见的占位图元。
#[must_use]
pub fn emit_address_bar_chrome(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
) -> WidgetOutput {
    emit_address_bar_inner(
        node,
        desc,
        state,
        control,
        &[],
        &[],
        &[],
        &AddressBarTextVisual::default(),
        false,
    )
}

/// 发射双模复合地址栏，支持区分主文本与次级色占位文本字形，以及编辑态/搜索框的光标、选区与 IME 预编辑下划线。
#[must_use]
pub fn emit_address_bar_with_visual(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    primary_glyphs: &[GlyphQuad],
    placeholder_glyphs: &[GlyphQuad],
    visual: &AddressBarTextVisual,
) -> WidgetOutput {
    emit_address_bar_with_dropdown(
        node,
        desc,
        state,
        control,
        primary_glyphs,
        placeholder_glyphs,
        &[],
        visual,
    )
}

/// 发射双模复合地址栏，并在展开历史/补全下拉浮层时一并绘制条目文本字形。
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn emit_address_bar_with_dropdown(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    primary_glyphs: &[GlyphQuad],
    placeholder_glyphs: &[GlyphQuad],
    dropdown_glyphs: &[GlyphQuad],
    visual: &AddressBarTextVisual,
) -> WidgetOutput {
    emit_address_bar_inner(
        node,
        desc,
        state,
        control,
        primary_glyphs,
        placeholder_glyphs,
        dropdown_glyphs,
        visual,
        true,
    )
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn emit_address_bar_inner(
    node: u64,
    desc: &AddressBarDesc,
    state: &AddressBarState,
    control: ControlState,
    primary_glyphs: &[GlyphQuad],
    placeholder_glyphs: &[GlyphQuad],
    dropdown_glyphs: &[GlyphQuad],
    visual: &AddressBarTextVisual,
    emit_text_overlays: bool,
) -> WidgetOutput {
    let layout = compute_layout(desc);
    let z = desc.z;
    let total_glyphs = primary_glyphs.len() + placeholder_glyphs.len() + dropdown_glyphs.len();
    let mut primitives = Vec::with_capacity(36 + layout.segments.len() * 4 + total_glyphs);
    let mut payloads = Vec::with_capacity(20 + layout.segments.len() * 2 + total_glyphs);

    // 1. 左段：外部 4 枚导航按钮槽位（后退/前进/向上/刷新，位于地址框之外）
    let nav_slots: [(Option<Rect>, AddressBarHitKind, bool); 4] = [
        (
            layout.back_rect,
            AddressBarHitKind::NavBack,
            desc.can_go_back,
        ),
        (
            layout.forward_rect,
            AddressBarHitKind::NavForward,
            desc.can_go_forward,
        ),
        (layout.up_rect, AddressBarHitKind::NavUp, desc.can_go_up),
        (layout.refresh_rect, AddressBarHitKind::Refresh, true),
    ];
    for (slot_rect, kind, enabled) in nav_slots {
        let Some(btn_r) = slot_rect else {
            continue;
        };
        let active = !control.disabled && enabled;
        // 仅在 `pressed_hit` / `hovered_hit` 精确命中对应按钮时才高亮，避免空白悬停误高亮后退按钮
        let is_pressed = active
            && (state.pressed_hit == Some(kind)
                || (state.hovered_hit == Some(kind) && control.phase == PointerPhase::Pressed));
        let is_hovered = active && state.hovered_hit == Some(kind);
        let fill = if is_pressed {
            Token::PRESSED
        } else if is_hovered {
            Token::HOVER
        } else {
            Token::NONE
        };
        let icon_color = if active {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_DISABLED
        };
        primitives.push(surface(btn_r, INNER_RADIUS, fill, z + 1));
        match kind {
            AddressBarHitKind::NavBack => {
                emit_shell_icon_primitives(
                    ShellIcon::ArrowLeft,
                    btn_r,
                    icon_color,
                    fill,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
            AddressBarHitKind::NavForward => {
                emit_shell_icon_primitives(
                    ShellIcon::ArrowRight,
                    btn_r,
                    icon_color,
                    fill,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
            AddressBarHitKind::NavUp => {
                emit_shell_icon_primitives(
                    ShellIcon::ArrowUp,
                    btn_r,
                    icon_color,
                    fill,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
            AddressBarHitKind::Refresh => {
                emit_shell_icon_primitives(
                    ShellIcon::Refresh,
                    btn_r,
                    icon_color,
                    fill,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
            _ => {}
        }
    }

    // 2. 中段：一体化双模地址框
    let addr_r = layout.address_rect;
    let is_borderless_pill = desc.chrome_style == AddressBarChromeStyle::BorderlessPill;
    let bar_radius = if is_borderless_pill {
        PILL_BAR_RADIUS
    } else {
        BAR_RADIUS
    };
    // 悬停或按压地址框内部任意子热区时，外框/胶囊底色保持悬停高亮不闪烁
    let addr_sub_active = matches!(
        state.hovered_hit,
        Some(
            AddressBarHitKind::RootChevron
                | AddressBarHitKind::OverflowButton
                | AddressBarHitKind::Segment(_)
                | AddressBarHitKind::SegmentChevron(_)
                | AddressBarHitKind::EditTrigger
                | AddressBarHitKind::EditClear
                | AddressBarHitKind::HistoryDropdownButton
        )
    ) || matches!(
        state.pressed_hit,
        Some(
            AddressBarHitKind::RootChevron
                | AddressBarHitKind::OverflowButton
                | AddressBarHitKind::Segment(_)
                | AddressBarHitKind::SegmentChevron(_)
                | AddressBarHitKind::EditTrigger
                | AddressBarHitKind::EditClear
                | AddressBarHitKind::HistoryDropdownButton
        )
    );
    let bg = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if is_borderless_pill {
        if control.phase == PointerPhase::Hovered || addr_sub_active || state.editing {
            Token::CONTROL_HOVER
        } else {
            Token::PILL_INPUT_FILL
        }
    } else {
        Token::CONTROL_FILL
    };
    let (border, border_width) = if is_borderless_pill {
        (Token::NONE, 0.0)
    } else if control.disabled {
        (Token::CONTROL_DISABLED_BORDER, 1.0)
    } else if control.phase == PointerPhase::Hovered || addr_sub_active || state.editing {
        (Token::OUTLINE_STRONG, 1.0)
    } else {
        (Token::CONTROL_STROKE, 1.0)
    };
    primitives.push(outlined(
        addr_r,
        bar_radius,
        bg,
        border,
        border_width,
        0.0,
        z,
    ));

    // 聚焦/编辑态底部 2px 强调线（图元始终存在，通过透明度切换保证 24 态图元数恒定）
    primitives.push(focus_underline(
        addr_r,
        bar_radius,
        z,
        if !control.disabled && (state.editing || control.focused) {
            1.0
        } else {
            0.0
        },
    ));

    // 左侧固定 16×16 根目录/驱动器图标（支持彩色外部纹理或单色矢量回退）
    let root_icon = desc.root_icon();
    let root_icon_color = if control.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    if layout.root_icon_rect.width() > 0.0 && layout.root_icon_rect.height() > 0.0 {
        emit_widget_icon_primitives(
            root_icon,
            layout.root_icon_rect,
            root_icon_color,
            bg,
            control.disabled,
            z + 1,
            &mut primitives,
            &mut payloads,
        );
    }

    // 根图标后置 `›` 分隔箭头（支持独立悬停/按压反馈；编辑态置透明以保持图元数恒定）
    if let Some(rc_r) = layout.root_chevron_rect {
        let rc_fill = if state.editing || control.disabled {
            Token::NONE
        } else if state.pressed_hit == Some(AddressBarHitKind::RootChevron)
            || (state.hovered_hit == Some(AddressBarHitKind::RootChevron)
                && control.phase == PointerPhase::Pressed)
        {
            Token::PRESSED
        } else if state.hovered_hit == Some(AddressBarHitKind::RootChevron) {
            Token::HOVER
        } else {
            Token::NONE
        };
        let rc_color = if state.editing {
            Token::NONE
        } else if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        };
        primitives.push(surface(rc_r, INNER_RADIUS, rc_fill, z + 1));
        emit_segment_chevron(rc_r, rc_color, z + 2, &mut primitives, &mut payloads);
    }

    // 溢出折叠 `«` 按钮（编辑态置透明以保持图元数恒定）
    if let Some(ov_r) = layout.overflow_rect {
        let ov_fill = if state.editing || control.disabled {
            Token::NONE
        } else if state.pressed_hit == Some(AddressBarHitKind::OverflowButton)
            || (state.hovered_hit == Some(AddressBarHitKind::OverflowButton)
                && control.phase == PointerPhase::Pressed)
        {
            Token::PRESSED
        } else if state.hovered_hit == Some(AddressBarHitKind::OverflowButton) {
            Token::HOVER
        } else {
            Token::NONE
        };
        let ov_color = if state.editing {
            Token::NONE
        } else if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        };
        primitives.push(surface(ov_r, INNER_RADIUS, ov_fill, z + 1));
        emit_overflow_chevrons(ov_r, ov_color, z + 2, &mut primitives, &mut payloads);
    }

    // 面包屑分段胶囊与条件性 `›` 展开箭头（常态透明，悬停/按下呈现圆角高亮；编辑态隐藏但保持图元数恒定）
    for seg in &layout.segments {
        let seg_kind = AddressBarHitKind::Segment(seg.index);
        let seg_fill = if state.editing || control.disabled {
            Token::NONE
        } else if state.pressed_hit == Some(seg_kind)
            || (state.hovered_hit == Some(seg_kind) && control.phase == PointerPhase::Pressed)
        {
            Token::PRESSED
        } else if state.hovered_hit == Some(seg_kind) {
            Token::HOVER
        } else {
            Token::NONE
        };
        primitives.push(surface(seg.label_rect, INNER_RADIUS, seg_fill, z + 1));

        if let Some(chev_r) = seg.chevron_rect {
            let chev_kind = AddressBarHitKind::SegmentChevron(seg.index);
            let chev_fill = if state.editing || control.disabled {
                Token::NONE
            } else if state.pressed_hit == Some(chev_kind)
                || (state.hovered_hit == Some(chev_kind) && control.phase == PointerPhase::Pressed)
            {
                Token::PRESSED
            } else if state.hovered_hit == Some(chev_kind) {
                Token::HOVER
            } else {
                Token::NONE
            };
            let chev_color = if state.editing {
                Token::NONE
            } else if control.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };
            primitives.push(surface(chev_r, INNER_RADIUS, chev_fill, z + 1));
            emit_segment_chevron(chev_r, chev_color, z + 2, &mut primitives, &mut payloads);
        }
    }

    // 编辑态右侧清除 `×` 按钮（图元槽位恒定，非编辑态或缓冲为空时置透明以保持 24 态图元数恒定）
    if let Some(clear_r) = layout.edit_clear_rect {
        let show_edit_clear =
            !control.disabled && state.editing && !state.current_edit_text().is_empty();
        let clear_fill = if !show_edit_clear {
            Token::NONE
        } else if state.pressed_hit == Some(AddressBarHitKind::EditClear)
            || (state.hovered_hit == Some(AddressBarHitKind::EditClear)
                && control.phase == PointerPhase::Pressed)
        {
            Token::PRESSED
        } else if state.hovered_hit == Some(AddressBarHitKind::EditClear) {
            Token::HOVER
        } else {
            Token::NONE
        };
        let clear_color = if show_edit_clear {
            Token::TEXT_SECONDARY
        } else {
            Token::NONE
        };
        primitives.push(surface(clear_r, INNER_RADIUS, clear_fill, z + 1));
        emit_shell_icon_primitives(
            ShellIcon::Close,
            clear_r,
            clear_color,
            clear_fill,
            z + 2,
            &mut primitives,
            &mut payloads,
        );
    }

    // 主地址框最右侧历史下拉箭头 `⌄` 按钮（仅历史路径非空时绘制）
    if let Some(hd_r) = layout.history_dropdown_rect {
        let active = !control.disabled;
        let hd_fill = if !active {
            Token::NONE
        } else if state.pressed_hit == Some(AddressBarHitKind::HistoryDropdownButton)
            || (state.hovered_hit == Some(AddressBarHitKind::HistoryDropdownButton)
                && control.phase == PointerPhase::Pressed)
        {
            Token::PRESSED
        } else if state.hovered_hit == Some(AddressBarHitKind::HistoryDropdownButton)
            || state.dropdown_open
        {
            Token::HOVER
        } else {
            Token::NONE
        };
        let hd_color = if active {
            Token::TEXT_SECONDARY
        } else {
            Token::TEXT_DISABLED
        };
        primitives.push(surface(hd_r, INNER_RADIUS, hd_fill, z + 1));
        emit_shell_icon_primitives(
            ShellIcon::ChevronDown,
            hd_r,
            hd_color,
            hd_fill,
            z + 2,
            &mut primitives,
            &mut payloads,
        );
    }

    // 3. 右段：可选可折叠内联搜索框
    if let Some(sr) = layout.search_rect {
        let search_sub_active = matches!(
            state.hovered_hit,
            Some(AddressBarHitKind::SearchBox | AddressBarHitKind::SearchClear)
        ) || matches!(
            state.pressed_hit,
            Some(AddressBarHitKind::SearchBox | AddressBarHitKind::SearchClear)
        );
        let search_bg = if control.disabled {
            Token::CONTROL_DISABLED_BG
        } else if is_borderless_pill {
            if control.phase == PointerPhase::Hovered || search_sub_active || state.search_focused {
                Token::CONTROL_HOVER
            } else {
                Token::PILL_INPUT_FILL
            }
        } else {
            Token::CONTROL_FILL
        };
        let (search_border, search_border_width) = if is_borderless_pill {
            (Token::NONE, 0.0)
        } else if control.disabled {
            (Token::CONTROL_DISABLED_BORDER, 1.0)
        } else if state.search_focused || search_sub_active {
            (Token::OUTLINE_STRONG, 1.0)
        } else {
            (Token::CONTROL_STROKE, 1.0)
        };
        primitives.push(outlined(
            sr,
            bar_radius,
            search_bg,
            search_border,
            search_border_width,
            0.0,
            z,
        ));
        primitives.push(focus_underline(
            sr,
            bar_radius,
            z,
            if !control.disabled && state.search_focused {
                1.0
            } else {
                0.0
            },
        ));
        if let Some(icon_r) = layout.search_icon_rect {
            emit_shell_icon_primitives(
                ShellIcon::Search,
                icon_r,
                if control.disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_SECONDARY
                },
                bg,
                z + 2,
                &mut primitives,
                &mut payloads,
            );
        }
        if let Some(clear_r) = layout.search_clear_rect {
            let show_clear = !control.disabled && !state.search_core.text().is_empty();
            let clear_fill = if !show_clear {
                Token::NONE
            } else if state.pressed_hit == Some(AddressBarHitKind::SearchClear)
                || (state.hovered_hit == Some(AddressBarHitKind::SearchClear)
                    && control.phase == PointerPhase::Pressed)
            {
                Token::PRESSED
            } else if state.hovered_hit == Some(AddressBarHitKind::SearchClear) {
                Token::HOVER
            } else {
                Token::NONE
            };
            let clear_color = if show_clear {
                Token::TEXT_SECONDARY
            } else {
                Token::NONE
            };
            primitives.push(surface(clear_r, INNER_RADIUS, clear_fill, z + 1));
            emit_shell_icon_primitives(
                ShellIcon::Close,
                clear_r,
                clear_color,
                clear_fill,
                z + 2,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    // 4. 编辑态/搜索框选区高亮、主文本与次级占位文本字形、光标与 IME 预编辑下划线
    if emit_text_overlays {
        let edit_active = state.editing && !control.disabled;
        let search_active = layout.search_rect.is_some()
            && !layout.search_collapsed
            && state.search_focused
            && !control.disabled;

        push_selection_overlay(
            &visual.edit_visual,
            edit_active,
            control.disabled,
            z + 2,
            &mut primitives,
        );
        if layout.search_rect.is_some() {
            push_selection_overlay(
                &visual.search_visual,
                search_active,
                control.disabled,
                z + 2,
                &mut primitives,
            );
        }

        emit_glyphs(
            primary_glyphs,
            if control.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_PRIMARY
            },
            z + 2,
            &mut primitives,
            &mut payloads,
        );
        emit_glyphs(
            placeholder_glyphs,
            if control.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            },
            z + 2,
            &mut primitives,
            &mut payloads,
        );

        push_caret_and_preedit_overlays(
            &visual.edit_visual,
            edit_active,
            state.edit_core.preedit().is_some(),
            control.disabled,
            z + 2,
            &mut primitives,
        );
        if layout.search_rect.is_some() {
            push_caret_and_preedit_overlays(
                &visual.search_visual,
                search_active,
                state.search_core.preedit().is_some(),
                control.disabled,
                z + 2,
                &mut primitives,
            );
        }
    }
    primitives.push(focus_ring(
        addr_r.inflate(2.0),
        BAR_RADIUS + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| {
            HitRegion::new(node, LAYER, u32::from(z), desc.rect).with_cursor(if state.editing {
                CursorKind::Text
            } else {
                CursorKind::Pointer
            })
        }),
        a11y: Some(
            A11yNode::new(Role::Toolbar, desc.full_path(), desc.rect).disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    };

    // 注册独立子热区（便于输入路由器精确命中各按钮、分段与搜索框）
    if !control.disabled {
        for (slot_rect, kind, enabled, label) in [
            (
                layout.back_rect,
                AddressBarHitKind::NavBack,
                desc.can_go_back,
                "Back",
            ),
            (
                layout.forward_rect,
                AddressBarHitKind::NavForward,
                desc.can_go_forward,
                "Forward",
            ),
            (
                layout.up_rect,
                AddressBarHitKind::NavUp,
                desc.can_go_up,
                "Up",
            ),
            (
                layout.refresh_rect,
                AddressBarHitKind::Refresh,
                true,
                "Refresh",
            ),
        ] {
            if let Some(btn_r) = slot_rect {
                let sub = address_bar_sub_node(node, kind);
                out.with_sub_control(
                    sub,
                    HitRegion::new(sub, LAYER, u32::from(z) + 1, btn_r).with_cursor(if enabled {
                        CursorKind::Pointer
                    } else {
                        CursorKind::Default
                    }),
                    A11yNode::new(Role::Button, label, btn_r)
                        .clickable()
                        .disabled(!enabled),
                    enabled,
                );
            }
        }

        if !state.editing {
            if let Some(rc_r) = layout.root_chevron_rect {
                let rc_sub = address_bar_sub_node(node, AddressBarHitKind::RootChevron);
                out.with_sub_control(
                    rc_sub,
                    HitRegion::new(rc_sub, LAYER, u32::from(z) + 2, rc_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Root folders", rc_r).clickable(),
                    true,
                );
            }
            if let Some(ov_r) = layout.overflow_rect {
                let sub = address_bar_sub_node(node, AddressBarHitKind::OverflowButton);
                out.with_sub_control(
                    sub,
                    HitRegion::new(sub, LAYER, u32::from(z) + 2, ov_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Overflow path segments", ov_r).clickable(),
                    true,
                );
            }
            for seg in &layout.segments {
                let seg_label = desc
                    .segments
                    .get(seg.index)
                    .map_or("", |s| s.label.as_str());
                let seg_sub = address_bar_sub_node(node, AddressBarHitKind::Segment(seg.index));
                out.with_sub_control(
                    seg_sub,
                    HitRegion::new(seg_sub, LAYER, u32::from(z) + 2, seg.label_rect)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, seg_label, seg.label_rect).clickable(),
                    true,
                );
                if let Some(chev_r) = seg.chevron_rect {
                    let chev_sub =
                        address_bar_sub_node(node, AddressBarHitKind::SegmentChevron(seg.index));
                    out.with_sub_control(
                        chev_sub,
                        HitRegion::new(chev_sub, LAYER, u32::from(z) + 2, chev_r)
                            .with_cursor(CursorKind::Pointer),
                        A11yNode::new(Role::Button, "Subfolders", chev_r).clickable(),
                        true,
                    );
                }
            }
        }

        let edit_sub = address_bar_sub_node(node, AddressBarHitKind::EditTrigger);
        let edit_r = if state.editing {
            layout.address_rect
        } else {
            layout.edit_trigger_rect
        };
        out.with_sub_control(
            edit_sub,
            HitRegion::new(edit_sub, LAYER, u32::from(z) + 1, edit_r).with_cursor(CursorKind::Text),
            A11yNode::new(Role::TextInput, desc.full_path(), edit_r),
            true,
        );

        if state.editing && !state.current_edit_text().is_empty() {
            if let Some(clear_r) = layout.edit_clear_rect {
                let clear_sub = address_bar_sub_node(node, AddressBarHitKind::EditClear);
                out.with_sub_control(
                    clear_sub,
                    HitRegion::new(clear_sub, LAYER, u32::from(z) + 2, clear_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::Button, "Clear address", clear_r).clickable(),
                    true,
                );
            }
        }

        if let Some(hd_r) = layout.history_dropdown_rect {
            let hd_sub = address_bar_sub_node(node, AddressBarHitKind::HistoryDropdownButton);
            out.with_sub_control(
                hd_sub,
                HitRegion::new(hd_sub, LAYER, u32::from(z) + 2, hd_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, "Address history", hd_r).clickable(),
                true,
            );
        }

        if let Some(dd) = layout.compute_dropdown_layout(desc, state.editing, state.dropdown_open) {
            let items = desc
                .active_dropdown_items(state.editing)
                .map_or(&[][..], |(_, slice)| slice);
            let popup_z = z.saturating_add(4);
            let row_z = z.saturating_add(5);
            let glyph_z = z.saturating_add(6);
            out.primitives.push(outlined(
                dd.popup_rect,
                8.0,
                Token::OVERLAY_BG,
                Token::CARD_BORDER,
                1.0,
                6.0,
                popup_z,
            ));
            for (idx, &row_r) in dd.item_rects.iter().enumerate() {
                let item_kind = AddressBarHitKind::DropdownItem(idx);
                let row_fill = if state.pressed_hit == Some(item_kind)
                    || (state.hovered_hit == Some(item_kind)
                        && control.phase == PointerPhase::Pressed)
                {
                    Token::PRESSED
                } else if state.dropdown_highlighted == Some(idx) {
                    Token::SELECTION
                } else if state.hovered_hit == Some(item_kind) {
                    Token::HOVER
                } else {
                    Token::NONE
                };
                out.primitives
                    .push(surface(row_r, INNER_RADIUS, row_fill, row_z));
                let item_sub = address_bar_sub_node(node, item_kind);
                let item_label = items.get(idx).map_or("", String::as_str);
                out.with_sub_control(
                    item_sub,
                    HitRegion::new(item_sub, LAYER, u32::from(row_z), row_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::ListItem, item_label, row_r).clickable(),
                    true,
                );
            }
            if !dropdown_glyphs.is_empty() {
                emit_glyphs(
                    dropdown_glyphs,
                    Token::TEXT_PRIMARY,
                    glyph_z,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }
        }

        if let Some(sr) = layout.search_rect {
            let search_sub = address_bar_sub_node(node, AddressBarHitKind::SearchBox);
            let search_label = if desc.search_placeholder.is_empty() {
                "Search"
            } else {
                desc.search_placeholder.as_str()
            };
            out.with_sub_control(
                search_sub,
                HitRegion::new(search_sub, LAYER, u32::from(z) + 1, sr).with_cursor(
                    if layout.search_collapsed {
                        CursorKind::Pointer
                    } else {
                        CursorKind::Text
                    },
                ),
                A11yNode::new(Role::SearchInput, search_label, sr),
                true,
            );
            if let Some(clear_r) = layout.search_clear_rect {
                if !state.search_core.text().is_empty() {
                    let clear_sub = address_bar_sub_node(node, AddressBarHitKind::SearchClear);
                    out.with_sub_control(
                        clear_sub,
                        HitRegion::new(clear_sub, LAYER, u32::from(z) + 2, clear_r)
                            .with_cursor(CursorKind::Pointer),
                        A11yNode::new(Role::Button, "Clear search", clear_r).clickable(),
                        true,
                    );
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_desc() -> AddressBarDesc {
        AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 840.0, ADDRESS_BAR_HEIGHT),
            "C:/Workspace/Assets/Shaders",
            vec![
                PathSegment::new("C:", "C:/").with_icon(ShellIcon::Folder),
                PathSegment::new("Workspace", "C:/Workspace"),
                PathSegment::new("Assets", "C:/Workspace/Assets"),
                PathSegment::new("Shaders", "C:/Workspace/Assets/Shaders").with_subfolders(false),
            ],
        )
        .with_nav_state(true, false, true)
        .with_search("Search Shaders")
    }

    #[test]
    fn address_bar_keeps_constant_primitives_across_24_states() {
        let desc = sample_desc();
        for editing in [false, true] {
            let mut ab_state = AddressBarState::new();
            if editing {
                ab_state.begin_edit(&desc.full_path());
            }
            let base = emit_address_bar(1, &desc, &ab_state, ControlState::default(), &[])
                .primitives
                .len();
            for phase in [
                PointerPhase::Idle,
                PointerPhase::Hovered,
                PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let count = emit_address_bar(1, &desc, &ab_state, control, &[])
                                .primitives
                                .len();
                            assert_eq!(
                                count, base,
                                "primitive count drifted in editing={editing} state={control:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nav_buttons_are_outside_address_box_and_respect_disabled_state() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let back_r = layout.back_rect.expect("back button");
        let fwd_r = layout.forward_rect.expect("forward button");
        let up_r = layout.up_rect.expect("up button");
        let ref_r = layout.refresh_rect.expect("refresh button");

        assert!(ref_r.max().x < layout.address_rect.origin.x);

        let mut state = AddressBarState::new();
        assert!(state.handle_click(&desc, back_r.center()));
        assert!(
            !state.handle_click(&desc, fwd_r.center()),
            "disabled forward button must not emit event"
        );
        assert!(state.handle_click(&desc, up_r.center()));
        assert!(state.handle_click(&desc, ref_r.center()));

        assert_eq!(
            state.drain_events(),
            vec![
                AddressBarEvent::NavigateBack,
                AddressBarEvent::NavigateUp,
                AddressBarEvent::Refresh,
            ]
        );
    }

    #[test]
    fn segments_click_and_chevrons_emit_anchor_rect_and_overflow_folds() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        assert_eq!(layout.overflow_count, 0);
        assert_eq!(layout.segments.len(), 4);
        assert!(layout.segments[3].chevron_rect.is_none());

        let mut state = AddressBarState::new();
        assert!(state.handle_click(&desc, layout.segments[1].label_rect.center()));
        let chev_r = layout.segments[1].chevron_rect.expect("chevron");
        assert!(state.handle_click(&desc, chev_r.center()));

        assert_eq!(
            state.drain_events(),
            vec![
                AddressBarEvent::SegmentClicked {
                    index: 1,
                    path: "C:/Workspace".to_owned(),
                },
                AddressBarEvent::SubfolderMenuRequested {
                    index: 1,
                    path: "C:/Workspace".to_owned(),
                    anchor_rect: rect_to_i32(chev_r),
                },
            ]
        );

        // 窄宽下自动将前段折叠为 `«` 溢出按钮
        let mut narrow = desc.clone();
        narrow.rect = Rect::new(0.0, 0.0, 320.0, ADDRESS_BAR_HEIGHT);
        narrow.show_search = false;
        let narrow_layout = compute_layout(&narrow);
        assert!(narrow_layout.overflow_count > 0);
        let ov_r = narrow_layout.overflow_rect.expect("overflow button");
        assert!(state.handle_click(&narrow, ov_r.center()));
        assert!(matches!(
            state.drain_events().as_slice(),
            [AddressBarEvent::SubfolderMenuRequested { index: 0, .. }]
        ));
    }

    #[test]
    fn blank_click_and_ctrl_l_enter_edit_mode_with_enter_and_esc() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let mut state = AddressBarState::new();

        // 点击末段右侧空白区切入编辑态并全选完整路径
        assert!(state.handle_click(&desc, layout.edit_trigger_rect.center()));
        assert!(state.editing);
        assert_eq!(state.edit_core.text(), "C:/Workspace/Assets/Shaders");
        assert_eq!(
            state.edit_core.selected_text(),
            Some("C:/Workspace/Assets/Shaders")
        );

        // Esc 取消编辑
        assert!(state.handle_key(&desc, KeyChord::named("Escape")));
        assert!(!state.editing);
        assert_eq!(state.drain_events(), vec![AddressBarEvent::EditCancelled]);

        // Ctrl+L 切入编辑态，输入新路径后按 Enter 提交
        assert!(state.handle_key(&desc, KeyChord::character('l').with_ctrl()));
        assert!(state.editing);
        state.handle_text_event(&desc, &TextInputEvent::Insert("D:/Projects".to_owned()));
        assert!(state.handle_key(&desc, KeyChord::named("Enter")));
        assert!(!state.editing);
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::PathSubmitted("D:/Projects".to_owned())]
        );
    }

    #[test]
    fn inline_search_box_typing_submit_and_clear() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let sr = layout.search_rect.expect("search box");
        let clear_r = layout.search_clear_rect.expect("clear button");
        let mut state = AddressBarState::new();

        assert!(state.handle_click(&desc, sr.center()));
        assert!(state.search_focused);

        assert!(state.handle_key(&desc, KeyChord::character('p')));
        assert!(state.handle_key(&desc, KeyChord::character('b')));
        assert!(state.handle_key(&desc, KeyChord::character('r')));
        assert!(state.handle_key(&desc, KeyChord::named("Enter")));

        assert_eq!(
            state.drain_events(),
            vec![
                AddressBarEvent::SearchChanged("p".to_owned()),
                AddressBarEvent::SearchChanged("pb".to_owned()),
                AddressBarEvent::SearchChanged("pbr".to_owned()),
                AddressBarEvent::SearchSubmitted("pbr".to_owned()),
            ]
        );

        assert!(state.handle_click(&desc, clear_r.center()));
        assert!(state.search_core.text().is_empty());
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::SearchChanged(String::new())]
        );
    }

    #[test]
    fn legacy_address_bar_api_remains_compatible() {
        let desc = AddressBarDesc::new(
            Rect::new(0.0, 0.0, 360.0, 34.0),
            vec!["Workspace".to_owned(), "Assets".to_owned()],
        );
        assert_eq!(desc.full_path(), "Workspace/Assets");
        let mut state = AddressBarState::new();
        state.begin_edit(&desc.full_path());
        assert_eq!(state.commit_edit(), vec!["Workspace", "Assets"]);
    }

    #[test]
    fn default_z_is_above_container_surface_layer() {
        let legacy = AddressBarDesc::new(
            Rect::new(0.0, 0.0, 600.0, ADDRESS_BAR_HEIGHT),
            vec!["C:".to_owned(), "Workspace".to_owned()],
        );
        let structured = sample_desc();
        assert!(
            legacy.z >= 400,
            "legacy default z must be >= 400, got {}",
            legacy.z
        );
        assert!(
            structured.z >= 400,
            "structured default z must be >= 400, got {}",
            structured.z
        );
        assert_eq!(legacy.z, 450);
        assert_eq!(structured.z, 450);
    }

    #[test]
    fn win11_borderless_pill_and_right_search_icon_are_default() {
        let desc = sample_desc();
        assert_eq!(desc.chrome_style, AddressBarChromeStyle::BorderlessPill);
        assert_eq!(desc.search_icon_placement, SearchIconPlacement::Right);

        let layout = compute_layout(&desc);
        let sr = layout.search_rect.expect("search rect");
        let icon_r = layout.search_icon_rect.expect("search icon rect");
        let clear_r = layout.search_clear_rect.expect("search clear rect");

        // 默认搜索放大镜置放于搜索框右侧，清除按钮位于放大镜左侧
        assert!(
            icon_r.origin.x > sr.center().x && icon_r.max().x <= sr.max().x,
            "default search icon must be on the right side of search box"
        );
        assert!(
            clear_r.max().x <= icon_r.origin.x,
            "clear button must sit to the left of right-aligned search icon"
        );
        let text_r = layout
            .search_text_rect(true)
            .expect("search text rect with clear");
        assert!(
            text_r.origin.x >= sr.origin.x && text_r.max().x <= clear_r.origin.x,
            "search text rect must fit to the left of clear button"
        );

        // 默认无外描边胶囊填充：地址框与搜索框填充 PILL_INPUT_FILL 且无描边（border = NONE, width = 0）
        let state = AddressBarState::new();
        let out = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        let addr_prim = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - layout.address_rect.origin.x).abs() < 0.5
                    && (p.size[0] - layout.address_rect.size.x).abs() < 0.5
            })
            .expect("address container primitive");
        assert_eq!(
            addr_prim.fill_token_or_uv,
            u32::from(Token::PILL_INPUT_FILL.raw())
        );
        assert_eq!(addr_prim.border_token_or_tex, u32::from(Token::NONE.raw()));
        assert_eq!(addr_prim.border_width, 0);

        let search_prim = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - sr.origin.x).abs() < 0.5 && (p.size[0] - sr.size.x).abs() < 0.5
            })
            .expect("search container primitive");
        assert_eq!(
            search_prim.fill_token_or_uv,
            u32::from(Token::PILL_INPUT_FILL.raw())
        );
        assert_eq!(
            search_prim.border_token_or_tex,
            u32::from(Token::NONE.raw())
        );
        assert_eq!(search_prim.border_width, 0);

        // 显式切换为 Outlined 模式后恢复 1px 描边
        let outlined_desc = desc
            .clone()
            .with_chrome_style(AddressBarChromeStyle::Outlined);
        let outlined_out =
            emit_address_bar(1, &outlined_desc, &state, ControlState::default(), &[]);
        let outlined_addr = outlined_out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - layout.address_rect.origin.x).abs() < 0.5
                    && (p.size[0] - layout.address_rect.size.x).abs() < 0.5
            })
            .expect("outlined address primitive");
        assert!(outlined_addr.border_width > 0);
        assert_ne!(
            outlined_addr.border_token_or_tex,
            u32::from(Token::NONE.raw())
        );
    }

    #[test]
    fn search_icon_is_on_left_and_text_region_stays_inside_search_box() {
        let desc = sample_desc().with_search_icon_placement(SearchIconPlacement::Left);
        let layout = compute_layout(&desc);
        let sr = layout.search_rect.expect("search rect");
        let icon_r = layout.search_icon_rect.expect("search icon rect");
        let clear_r = layout.search_clear_rect.expect("search clear rect");

        assert!(!layout.search_collapsed);
        assert!(
            icon_r.origin.x >= sr.origin.x && icon_r.max().x < sr.center().x,
            "search icon must be positioned on the left side of search box"
        );
        assert!(
            clear_r.origin.x > sr.center().x && clear_r.max().x <= sr.max().x,
            "clear button must be positioned on the right side of search box"
        );
        let text_left = icon_r.max().x + 6.0;
        let text_right = clear_r.origin.x - 4.0;
        assert!(
            text_left < text_right && text_right <= sr.max().x,
            "search text region [{text_left}, {text_right}] must fit inside search_rect"
        );

        // 折叠态下图标居中且不分配清除按钮
        let mut narrow = desc.clone();
        narrow.rect = Rect::new(0.0, 0.0, 360.0, ADDRESS_BAR_HEIGHT);
        let narrow_layout = compute_layout(&narrow);
        assert!(narrow_layout.search_collapsed);
        assert!(narrow_layout.search_clear_rect.is_none());
        let collapsed_sr = narrow_layout.search_rect.expect("collapsed search rect");
        let collapsed_icon = narrow_layout.search_icon_rect.expect("collapsed icon");
        assert!((collapsed_icon.center().x - collapsed_sr.center().x).abs() < 0.5);
    }

    #[test]
    fn root_chevron_rect_sits_between_root_icon_and_first_segment() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let rc = layout.root_chevron_rect.expect("root chevron rect");
        let first_seg = layout.segments.first().expect("first segment");
        assert!(
            rc.origin.x >= layout.root_icon_rect.max().x,
            "root chevron must follow root icon"
        );
        assert!(
            rc.max().x <= first_seg.label_rect.origin.x,
            "root chevron must precede first segment"
        );
    }

    #[test]
    fn cjk_segment_width_exceeds_same_char_count_ascii_and_clamps_upper_bound() {
        // 使用 Unicode 转义构造 CJK 目录名，满足代码语言守卫同时覆盖宽字符估算
        let cjk_short = "\u{6B64}\u{7535}\u{8111}";
        let ascii_short = "abc";
        let cjk_long = "\u{672C}\u{5730}\u{78C1}\u{76D8}\u{5DE5}\u{4F5C}\u{533A}\u{8D44}\u{6E90}\u{7BA1}\u{7406}\u{5668}\u{957F}\u{76EE}\u{5F55}\u{540D}\u{79F0}\u{6D4B}\u{8BD5}\u{7528}\u{4F8B}";

        let desc_cjk = AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 900.0, ADDRESS_BAR_HEIGHT),
            cjk_short,
            vec![
                PathSegment::new(cjk_short, cjk_short),
                PathSegment::new(cjk_long, cjk_long).with_subfolders(false),
            ],
        )
        .without_search();
        let desc_ascii = AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 900.0, ADDRESS_BAR_HEIGHT),
            ascii_short,
            vec![PathSegment::new(ascii_short, ascii_short)],
        )
        .without_search();

        let layout_cjk = compute_layout(&desc_cjk);
        let layout_ascii = compute_layout(&desc_ascii);
        assert!(
            layout_cjk.segments[0].label_rect.width()
                > layout_ascii.segments[0].label_rect.width() + 10.0,
            "CJK segment width must exceed same-char-count ASCII width"
        );
        assert!(
            layout_cjk.segments[1].label_rect.width() <= 220.0 + 0.01,
            "long CJK segment width must clamp to 220px"
        );
    }

    #[test]
    fn idle_segments_are_transparent_and_none_hover_does_not_highlight_nav_back() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let mut state = AddressBarState::new();
        let hovered_control = ControlState {
            phase: PointerPhase::Hovered,
            ..ControlState::default()
        };

        // hovered_hit == None 且控件整体处于 Hovered 时：NavBack 不得被误高亮，且所有分段常态透明
        let out = emit_address_bar(1, &desc, &state, hovered_control, &[]);
        let back_r = layout.back_rect.expect("back rect");
        let back_bg = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - back_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - back_r.origin.y).abs() < 0.5
                    && (p.size[0] - back_r.size.x).abs() < 0.5
            })
            .expect("back button surface");
        assert_eq!(
            back_bg.fill_token_or_uv,
            u32::from(Token::NONE.raw()),
            "NavBack must remain transparent when hovered_hit is None"
        );

        let seg0_r = layout.segments[0].label_rect;
        let seg0_bg = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - seg0_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - seg0_r.origin.y).abs() < 0.5
                    && (p.size[0] - seg0_r.size.x).abs() < 0.5
            })
            .expect("segment 0 surface");
        assert_eq!(
            seg0_bg.fill_token_or_uv,
            u32::from(Token::NONE.raw()),
            "unhovered breadcrumb segment must be transparent"
        );

        // 精确悬停在 NavBack 或 Segment(0) 时应呈现 Token::HOVER，按下时呈现 Token::PRESSED
        state.hovered_hit = Some(AddressBarHitKind::NavBack);
        let out_back_hover = emit_address_bar(1, &desc, &state, hovered_control, &[]);
        let back_bg_hover = out_back_hover
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - back_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - back_r.origin.y).abs() < 0.5
                    && (p.size[0] - back_r.size.x).abs() < 0.5
            })
            .expect("hovered back button surface");
        assert_eq!(
            back_bg_hover.fill_token_or_uv,
            u32::from(Token::HOVER.raw())
        );

        state.hovered_hit = Some(AddressBarHitKind::Segment(0));
        let pressed_control = ControlState {
            phase: PointerPhase::Pressed,
            ..ControlState::default()
        };
        let out_seg_pressed = emit_address_bar(1, &desc, &state, pressed_control, &[]);
        let seg0_bg_pressed = out_seg_pressed
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - seg0_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - seg0_r.origin.y).abs() < 0.5
                    && (p.size[0] - seg0_r.size.x).abs() < 0.5
            })
            .expect("pressed segment 0 surface");
        assert_eq!(
            seg0_bg_pressed.fill_token_or_uv,
            u32::from(Token::PRESSED.raw())
        );
    }

    #[test]
    fn edit_and_search_overlays_emit_selection_caret_ime_and_secondary_placeholder() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let mut state = AddressBarState::new();
        state.begin_edit(&desc.full_path());
        let edit_r = Rect::new(
            layout.root_icon_rect.max().x + 6.0,
            layout.address_rect.origin.y,
            240.0,
            layout.address_rect.size.y,
        );
        // 刚进入编辑态时全选路径，验证选区高亮矩形发射
        let sel_visual = compute_text_edit_visual(&state.edit_core, edit_r, true, |s| {
            s.chars().count() as f32 * 8.0
        });
        assert!(sel_visual.selection.is_some());
        let out_sel = emit_address_bar_with_visual(
            1,
            &desc,
            &state,
            ControlState::default(),
            &[],
            &[],
            &AddressBarTextVisual {
                edit_visual: sel_visual,
                ..AddressBarTextVisual::default()
            },
        );
        assert!(
            out_sel
                .primitives
                .iter()
                .any(|p| p.fill_token_or_uv == u32::from(Token::SELECTION.raw())
                    && p.opacity() > 0.5),
            "active edit selection must emit visible Token::SELECTION overlay"
        );

        // 触发 IME 预编辑态以验证预编辑下划线发射
        let _ = state.handle_text_event(
            &desc,
            &TextInputEvent::ImePreedit {
                text: "src".to_owned(),
                cursor: Some(3),
            },
        );
        let edit_visual = compute_text_edit_visual(&state.edit_core, edit_r, true, |s| {
            s.chars().count() as f32 * 8.0
        });
        assert!(edit_visual.preedit.is_some());
        assert!(edit_visual.caret_visible);

        let placeholder_quad = GlyphQuad {
            rect: Rect::new(580.0, 8.0, 8.0, 14.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        };
        let visual = AddressBarTextVisual {
            edit_visual,
            ..AddressBarTextVisual::default()
        };
        let out = emit_address_bar_with_visual(
            1,
            &desc,
            &state,
            ControlState::default(),
            &[],
            &[placeholder_quad],
            &visual,
        );

        assert!(
            out.primitives.iter().any(|p| p.fill_token_or_uv
                == u32::from(Token::TEXT_SECONDARY.raw())
                && (p.local_pos[0] - 580.0).abs() < 0.5),
            "placeholder glyph must use Token::TEXT_SECONDARY"
        );
        assert!(
            out.primitives.iter().any(|p| p.fill_token_or_uv
                == u32::from(Token::PREEDIT_UNDERLINE.raw())
                && p.opacity() > 0.5),
            "active IME preedit must emit visible underline"
        );
        assert!(
            out.primitives.iter().any(|p| p.fill_token_or_uv
                == u32::from(Token::TEXT_PRIMARY.raw())
                && (p.size[0] - 1.5).abs() < 0.01
                && p.opacity() > 0.5),
            "active edit state must emit visible caret"
        );
    }

    #[test]
    fn compute_text_edit_visual_clamps_negative_height_and_chrome_skips_overlay_placeholders() {
        let state = AddressBarState::new();
        let neg_rect = Rect::new(10.0, 20.0, 120.0, -8.0);
        let visual = compute_text_edit_visual(&state.edit_core, neg_rect, true, |_| 0.0);
        assert!(
            visual.caret.size.y >= 0.0,
            "caret height must never be negative, got {}",
            visual.caret.size.y
        );
        assert!((visual.caret.origin.y - 20.0).abs() < 0.01);

        let desc = AddressBarDesc::new(
            Rect::new(0.0, 0.0, 760.0, ADDRESS_BAR_HEIGHT),
            vec!["C:".to_owned(), "Users".to_owned()],
        )
        .with_search("Search");
        let full = emit_address_bar_with_visual(
            1,
            &desc,
            &state,
            ControlState::default(),
            &[],
            &[],
            &AddressBarTextVisual::default(),
        );
        let chrome = emit_address_bar_chrome(1, &desc, &state, ControlState::default());
        assert_eq!(
            chrome.primitives.len() + 6,
            full.primitives.len(),
            "chrome output must omit the 6 redundant text overlay placeholder primitives"
        );
        assert_eq!(chrome.sub_hits.len(), full.sub_hits.len());
    }

    #[test]
    fn root_chevron_supports_hit_test_click_sub_control_and_hover_pressed_states() {
        let desc = sample_desc().with_chrome_style(AddressBarChromeStyle::Outlined);
        let layout = compute_layout(&desc);
        let rc_r = layout.root_chevron_rect.expect("root chevron rect");
        let mut state = AddressBarState::new();

        assert_eq!(
            state.hit_test(&desc, rc_r.center()),
            Some(AddressBarHitKind::RootChevron)
        );
        assert!(state.handle_click(&desc, rc_r.center()));
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::RootChevronDropdown {
                anchor_rect: rect_to_i32(rc_r),
            }]
        );

        let expected_sub = address_bar_sub_node(1, AddressBarHitKind::RootChevron);
        let out_idle = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        assert!(
            out_idle.sub_hits.iter().any(|h| h.node == expected_sub),
            "RootChevron must register a deterministic sub HitRegion"
        );

        state.hovered_hit = Some(AddressBarHitKind::RootChevron);
        let out_hover = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        let rc_hover = out_hover
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - rc_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - rc_r.origin.y).abs() < 0.5
                    && (p.size[0] - rc_r.size.x).abs() < 0.5
            })
            .expect("root chevron hover surface");
        assert_eq!(rc_hover.fill_token_or_uv, u32::from(Token::HOVER.raw()));

        // 悬停子热区时地址框外边框保持 OUTLINE_STRONG 不闪烁
        let addr_border = out_hover
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - layout.address_rect.origin.x).abs() < 0.5
                    && (p.size[0] - layout.address_rect.size.x).abs() < 0.5
                    && p.border_token_or_tex != u32::from(Token::NONE.raw())
            })
            .expect("address box border");
        assert_eq!(
            addr_border.border_token_or_tex,
            u32::from(Token::OUTLINE_STRONG.raw())
        );

        state.pressed_hit = Some(AddressBarHitKind::RootChevron);
        let out_pressed = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        let rc_pressed = out_pressed
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - rc_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - rc_r.origin.y).abs() < 0.5
                    && (p.size[0] - rc_r.size.x).abs() < 0.5
            })
            .expect("root chevron pressed surface");
        assert_eq!(rc_pressed.fill_token_or_uv, u32::from(Token::PRESSED.raw()));
    }

    #[test]
    fn search_clear_renders_pressed_token_and_keeps_search_border_hovered() {
        let desc = sample_desc().with_chrome_style(AddressBarChromeStyle::Outlined);
        let layout = compute_layout(&desc);
        let sr = layout.search_rect.expect("search rect");
        let clear_r = layout.search_clear_rect.expect("search clear rect");

        let mut state = AddressBarState::new();
        state.search_core.set_text("shader");
        state.hovered_hit = Some(AddressBarHitKind::SearchClear);
        state.pressed_hit = Some(AddressBarHitKind::SearchClear);

        let out = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        let clear_surface = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - clear_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - clear_r.origin.y).abs() < 0.5
                    && (p.size[0] - clear_r.size.x).abs() < 0.5
            })
            .expect("search clear surface");
        assert_eq!(
            clear_surface.fill_token_or_uv,
            u32::from(Token::PRESSED.raw())
        );

        let search_box = out
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - sr.origin.x).abs() < 0.5
                    && (p.size[0] - sr.size.x).abs() < 0.5
                    && p.border_token_or_tex != u32::from(Token::NONE.raw())
            })
            .expect("search box border");
        assert_eq!(
            search_box.border_token_or_tex,
            u32::from(Token::OUTLINE_STRONG.raw())
        );
    }

    #[test]
    fn path_segment_widget_icon_precedence_and_external_texture_emission() {
        let plain = PathSegment::new("Drive", "C:\\");
        assert_eq!(plain.effective_icon(), WidgetIcon::None);

        let vec_only = PathSegment::new("Drive", "C:\\").with_icon(ShellIcon::Drive);
        assert_eq!(
            vec_only.effective_icon(),
            WidgetIcon::Vector(ShellIcon::Drive)
        );

        let overridden = vec_only
            .clone()
            .with_widget_icon(WidgetIcon::ExternalTexture(64));
        assert_eq!(overridden.effective_icon(), WidgetIcon::ExternalTexture(64));

        let desc = AddressBarDesc::with_path_segments(
            Rect::new(0.0, 0.0, 520.0, ADDRESS_BAR_HEIGHT),
            "C:\\Users",
            vec![overridden, PathSegment::new("Users", "C:\\Users")],
        );
        assert_eq!(desc.root_icon(), WidgetIcon::ExternalTexture(64));

        let state = AddressBarState::new();
        let enabled = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        let img = enabled
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image)
            .expect("address bar external root icon");
        assert_eq!(img.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert_eq!(img.border_token_or_tex, 64);
        assert!((img.opacity() - 1.0).abs() < 0.01);

        let disabled = emit_address_bar(
            1,
            &desc,
            &state,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
        );
        let disabled_img = disabled
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image)
            .expect("disabled address bar external root icon");
        assert_eq!(disabled_img.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert_eq!(disabled_img.border_token_or_tex, 64);
        assert!((disabled_img.opacity() - crate::WINUI_DISABLED_OPACITY).abs() < 0.02);
    }

    #[test]
    fn edit_clear_button_renders_in_edit_mode_clears_buffer_and_emits_event() {
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        let clear_r = layout.edit_clear_rect.expect("edit clear rect");
        let clear_sub = address_bar_sub_node(1, AddressBarHitKind::EditClear);

        // 非编辑态：不命中清除按钮，也不注册 EditClear 子热区
        let mut state = AddressBarState::new();
        assert_ne!(
            state.hit_test(&desc, clear_r.center()),
            Some(AddressBarHitKind::EditClear)
        );
        let out_idle = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        assert!(
            !out_idle.sub_hits.iter().any(|h| h.node == clear_sub),
            "EditClear sub-control must not be registered outside edit mode"
        );

        // 进入编辑态：注册独立子热区并支持悬停/按压反馈
        state.begin_edit(&desc.full_path());
        assert_eq!(
            state.hit_test(&desc, clear_r.center()),
            Some(AddressBarHitKind::EditClear)
        );
        let edit_text_r = layout.edit_text_rect(true);
        assert!(
            edit_text_r.max().x <= clear_r.origin.x,
            "edit text rect must stay to the left of edit clear button"
        );

        state.hovered_hit = Some(AddressBarHitKind::EditClear);
        state.pressed_hit = Some(AddressBarHitKind::EditClear);
        let out_edit = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        assert!(
            out_edit.sub_hits.iter().any(|h| h.node == clear_sub),
            "EditClear sub-control must be registered in active edit mode"
        );
        let clear_surface = out_edit
            .primitives
            .iter()
            .find(|p| {
                (p.local_pos[0] - clear_r.origin.x).abs() < 0.5
                    && (p.local_pos[1] - clear_r.origin.y).abs() < 0.5
                    && (p.size[0] - clear_r.size.x).abs() < 0.5
            })
            .expect("edit clear surface primitive");
        assert_eq!(
            clear_surface.fill_token_or_uv,
            u32::from(Token::PRESSED.raw())
        );

        // 点击清除按钮：清空编辑缓冲、保持编辑态并派发 EditCleared
        assert!(state.handle_click(&desc, clear_r.center()));
        assert!(state.editing);
        assert!(state.edit_core.text().is_empty());
        assert!(state.edit_buffer.is_empty());
        assert_eq!(state.drain_events(), vec![AddressBarEvent::EditCleared]);

        // 清空后不再命中清除按钮，也不再注册其子热区
        assert_eq!(
            state.hit_test(&desc, clear_r.center()),
            Some(AddressBarHitKind::EditTrigger)
        );
        let out_empty = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        assert!(!out_empty.sub_hits.iter().any(|h| h.node == clear_sub));
    }

    #[test]
    fn history_dropdown_toggle_keyboard_navigation_and_click_selection() {
        let history = vec![
            "C:/Users/Alice".to_owned(),
            "D:/Projects".to_owned(),
            "E:/Archives".to_owned(),
        ];
        let desc = sample_desc().with_history_paths(history.clone());
        let layout = compute_layout(&desc);
        let hd_r = layout
            .history_dropdown_rect
            .expect("history dropdown button rect");
        let clear_r = layout.edit_clear_rect.expect("edit clear rect");
        assert!(
            clear_r.max().x <= hd_r.origin.x,
            "edit clear button must sit to the left of history dropdown button"
        );

        let mut state = AddressBarState::new();
        assert_eq!(
            state.hit_test(&desc, hd_r.center()),
            Some(AddressBarHitKind::HistoryDropdownButton)
        );

        // 点击下拉按钮展开历史浮层
        assert!(state.handle_click(&desc, hd_r.center()));
        assert!(state.dropdown_open);
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::HistoryDropdownToggled(true)]
        );

        let dd = layout
            .compute_dropdown_layout(&desc, state.editing, state.dropdown_open)
            .expect("dropdown layout");
        assert_eq!(dd.mode, AddressBarDropdownMode::History);
        assert_eq!(dd.item_rects.len(), 3);
        assert!(dd.popup_rect.origin.y > layout.address_rect.max().y);

        // 验证浮层图元与子热区注册
        let out = emit_address_bar(1, &desc, &state, ControlState::default(), &[]);
        assert!(
            out.primitives
                .iter()
                .any(|p| p.fill_token_or_uv == u32::from(Token::OVERLAY_BG.raw())),
            "open dropdown must emit OVERLAY_BG popup surface"
        );
        for idx in 0..3 {
            let item_sub = address_bar_sub_node(1, AddressBarHitKind::DropdownItem(idx));
            assert!(
                out.sub_hits.iter().any(|h| h.node == item_sub),
                "dropdown item {idx} must register sub HitRegion"
            );
        }

        // 键盘上下选择循环并按 Enter 提交
        assert!(state.handle_key(&desc, KeyChord::named("ArrowDown")));
        assert_eq!(state.dropdown_highlighted, Some(0));
        assert!(state.handle_key(&desc, KeyChord::named("ArrowDown")));
        assert_eq!(state.dropdown_highlighted, Some(1));
        assert!(state.handle_key(&desc, KeyChord::named("ArrowUp")));
        assert_eq!(state.dropdown_highlighted, Some(0));
        assert!(state.handle_key(&desc, KeyChord::named("ArrowUp")));
        assert_eq!(state.dropdown_highlighted, Some(2));
        assert!(state.handle_key(&desc, KeyChord::named("Enter")));
        assert!(!state.dropdown_open);
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::HistorySelected {
                index: 2,
                path: "E:/Archives".to_owned(),
            }]
        );

        // 进入编辑态自动展示历史浮层，并支持直接点击条目提交
        assert!(state.handle_key(&desc, KeyChord::character('l').with_ctrl()));
        assert!(state.editing);
        assert!(state.dropdown_open);
        let edit_dd = layout
            .compute_dropdown_layout(&desc, state.editing, state.dropdown_open)
            .expect("edit dropdown layout");
        assert_eq!(
            state.hit_test(&desc, edit_dd.item_rects[1].center()),
            Some(AddressBarHitKind::DropdownItem(1))
        );
        assert!(state.handle_click(&desc, edit_dd.item_rects[1].center()));
        assert!(!state.editing);
        assert!(!state.dropdown_open);
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::HistorySelected {
                index: 1,
                path: "D:/Projects".to_owned(),
            }]
        );

        // 非编辑态再次展开历史浮层，按 Esc 收起
        assert!(state.handle_click(&desc, hd_r.center()));
        let _ = state.drain_events();
        assert!(state.handle_key(&desc, KeyChord::named("Escape")));
        assert!(!state.dropdown_open);
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::HistoryDropdownToggled(false)]
        );
    }

    #[test]
    fn completion_suggestions_inline_suffix_and_tab_accept() {
        let desc = sample_desc()
            .with_history_paths(vec!["C:/Users/Alice".to_owned()])
            .with_completion_suggestions(vec![
                "C:/Workspace/Assets/Shaders".to_owned(),
                "C:/Workspace/Assets/Scripts".to_owned(),
            ]);

        assert_eq!(
            desc.inline_completion_suffix("C:/Workspace/Assets/S"),
            Some("haders")
        );
        assert_eq!(
            desc.inline_completion_suffix("c:/workspace/assets/s"),
            Some("haders")
        );
        assert_eq!(desc.inline_completion_suffix("D:/Other"), None);

        let mut state = AddressBarState::new();
        state.begin_edit("C:/Workspace/Assets/S");
        assert_eq!(
            desc.active_dropdown_items(state.editing).map(|(m, _)| m),
            Some(AddressBarDropdownMode::Completion)
        );

        // 下移选中第 2 条补全建议并按 Tab 接受
        assert!(state.handle_key(&desc, KeyChord::named("ArrowDown")));
        assert!(state.handle_key(&desc, KeyChord::named("ArrowDown")));
        assert_eq!(state.dropdown_highlighted, Some(1));
        assert!(state.handle_key(&desc, KeyChord::named("Tab")));
        assert!(state.editing);
        assert_eq!(state.edit_core.text(), "C:/Workspace/Assets/Scripts");
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::CompletionAccepted(
                "C:/Workspace/Assets/Scripts".to_owned()
            )]
        );

        // 点击补全列表首项同样派发 CompletionAccepted
        let layout = compute_layout(&desc);
        let dd = layout
            .compute_dropdown_layout(&desc, state.editing, state.dropdown_open)
            .expect("completion dropdown");
        assert_eq!(dd.mode, AddressBarDropdownMode::Completion);
        assert!(state.handle_click(&desc, dd.item_rects[0].center()));
        assert_eq!(state.edit_core.text(), "C:/Workspace/Assets/Shaders");
        assert_eq!(
            state.drain_events(),
            vec![AddressBarEvent::CompletionAccepted(
                "C:/Workspace/Assets/Shaders".to_owned()
            )]
        );

        // 仅配置单条 completion_hint 时，Tab 也可直接接受补全
        let hint_desc = sample_desc().with_completion_hint("D:/Work/Notes");
        let mut hint_state = AddressBarState::new();
        hint_state.begin_edit("D:/Work/");
        assert!(hint_state.handle_key(&hint_desc, KeyChord::named("Tab")));
        assert_eq!(hint_state.edit_core.text(), "D:/Work/Notes");
        assert_eq!(
            hint_state.drain_events(),
            vec![AddressBarEvent::CompletionAccepted(
                "D:/Work/Notes".to_owned()
            )]
        );

        // 无可用补全建议时，Tab 不消费按键且不派发事件
        let empty_desc = sample_desc();
        let mut empty_state = AddressBarState::new();
        empty_state.begin_edit("C:/");
        assert!(!empty_state.handle_key(&empty_desc, KeyChord::named("Tab")));
        assert!(empty_state.drain_events().is_empty());
    }

    #[test]
    fn fluent_address_bar_metrics_match_win11_scale() {
        assert!((34.0..=36.0).contains(&ADDRESS_BAR_HEIGHT));
        assert!((ADDRESS_BAR_FONT_SIZE - 14.0).abs() < f32::EPSILON);
        let desc = sample_desc();
        let layout = compute_layout(&desc);
        assert!((layout.address_rect.size.y - ADDRESS_BAR_HEIGHT).abs() < f32::EPSILON);
    }
}

//! WinUI 3 基准控件统一标准插桩辅助库与配方契约基础。
//!
//! 依据 ADR 0016 与 Spec #79，本模块提供容器与复合控件通用的标准插桩生成器：
//! 1. 通用路径/分段插桩辅助：固定输出 `Root > Folder > File.txt` 的标准文本与几何（用于 `BreadcrumbBar`、`SelectorBar` 等）；
//! 2. 通用树形展开插桩辅助：固定输出 2 级展开结构 `Root`（首项选中态）+ 子节点 `Child`（用于 `TreeView` 等）；
//! 3. 通用垂直列表/堆叠插桩辅助：固定输出统一字号与间距的 `Row 1`、`Row 2`、`Row 3`（用于 `StackPanel`、`ScrollView`、`ListView` 等）；
//! 4. 通用递归禁用态（`disabled`）传播机制：保证容器处于 `disabled` 状态时，所有子插桩文本、图标与指示器同步应用半透明度衰减与变灰。

use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::ergonomics::TextCtx;
use crate::primitive;
use crate::{AlignX, AlignY, ControlState, WidgetOutput};

/// 标准路径插桩文本项（固定 3 级）。
pub const CANONICAL_PATH_ITEMS: [&str; 3] = ["Root", "Folder", "File.txt"];

/// 标准路径分隔符文本。
pub const CANONICAL_PATH_SEPARATOR: &str = ">";

/// 标准分段插桩文本项（固定 3 段）。
pub const CANONICAL_SEGMENTED_ITEMS: [&str; 3] = ["Day", "Week", "Month"];

/// 标准树形根节点标签。
pub const CANONICAL_TREE_ROOT: &str = "Root";

/// 标准树形展开子节点标签。
pub const CANONICAL_TREE_CHILD: &str = "Child";

/// 标准垂直堆叠纯文本行项（固定 3 行）。
pub const CANONICAL_ROW_ITEMS: [&str; 3] = ["Row 1", "Row 2", "Row 3"];

/// 标准可交互列表条目项（固定 3 项）。
pub const CANONICAL_LIST_ITEMS: [&str; 3] = ["Item 1", "Item 2", "Item 3"];

/// 标准 PipsPager 页数（固定 5 页）。
pub const CANONICAL_PIPS_COUNT: usize = 5;

/// 标准 PipsPager 默认选中页索引（第 2 页，0-based 为 1）。
pub const CANONICAL_PIPS_SELECTED_INDEX: usize = 1;

/// 标准 PipsPager 未选中圆点直径（4px）。
pub const CANONICAL_PIP_SIZE_NORMAL: f32 = 4.0;

/// 标准 PipsPager 选中圆点直径（6px）。
pub const CANONICAL_PIP_SIZE_SELECTED: f32 = 6.0;

/// 标准 PipsPager 圆点中心间距（12px）。
pub const CANONICAL_PIP_SPACING: f32 = 12.0;

/// 标准 CommandBar 命令项（图标与文本标签，对齐 C# 宿主 Save / Open）。
pub const CANONICAL_COMMAND_ITEMS: [(&str, &str); 2] = [("\u{E74E}", "Save"), ("\u{E8E5}", "Open")];

/// 标准 CommandBar 更多命令按钮图标。
pub const CANONICAL_COMMAND_MORE_GLYPH: &str = "\u{E10C}";

/// 标准 MenuBar 菜单项（固定为 File 与 Edit，对齐 C# 宿主）。
pub const CANONICAL_MENU_ITEMS: [&str; 2] = ["File", "Edit"];

/// 标准 TitleBar 窗口标题。
pub const CANONICAL_TITLE_BAR_TITLE: &str = "Document";

/// 标准 TitleBar 窗口副标题。
pub const CANONICAL_TITLE_BAR_SUBTITLE: &str = "WinUiReference";

/// 标准 NavigationView 菜单项（图标与文本标签，对齐 C# 宿主 Home / Library）。
pub const CANONICAL_NAV_ITEMS: [(&str, &str); 2] = [("\u{E80F}", "Home"), ("\u{E8F1}", "Library")];

/// 标准 NavigationView 底部 Settings 项。
pub const CANONICAL_NAV_SETTINGS: (&str, &str) = ("\u{E713}", "\u{8BBE}\u{7F6E}"); // lang-ok: 复刻参考截图的 zh-CN 系统设置项

/// 标准 NavigationView 顶部折叠汉堡按钮图标。
pub const CANONICAL_NAV_TOGGLE_GLYPH: &str = "\u{E700}";

/// 标准 TabView 标签页项（固定为 Tab 1 与 Tab 2，对齐 C# 宿主）。
pub const CANONICAL_TAB_ITEMS: [&str; 2] = ["Tab 1", "Tab 2"];

/// 标准 TabView 选中页内部纯文本内容（对齐 C# 宿主 "Content"）。
pub const CANONICAL_TAB_CONTENT: &str = "Content";

/// 标准 TabView 标签页关闭按钮图标。
pub const CANONICAL_TAB_CLOSE_GLYPH: &str = "\u{E711}";

/// 标准网格条目项（固定 2×2，共 4 项）。
pub const CANONICAL_GRID_ITEMS: [&str; 4] = ["Item 1", "Item 2", "Item 3", "Item 4"];

/// 标准翻页视图页签项（固定 2 页）。
pub const CANONICAL_FLIP_PAGES: [&str; 2] = ["Page 1", "Page 2"];

/// 标准下拉刷新提示文本。
pub const CANONICAL_REFRESH_TEXT: &str = "Pull to refresh";

/// 标准 InfoBar 标题与消息文案。
pub const CANONICAL_INFOBAR_TITLE: &str = "InfoBar";
pub const CANONICAL_INFOBAR_MESSAGE: &str = "Message";
pub const CANONICAL_INFOBAR_WIDTH: f32 = 320.0;
pub const CANONICAL_INFOBAR_HEIGHT: f32 = 48.0;
pub const CANONICAL_INFOBAR_ICON_GLYPH: &str = "\u{E946}";
pub const CANONICAL_INFOBAR_CLOSE_GLYPH: &str = "\u{E711}";

/// 标准 Expander 标题与正文。
pub const CANONICAL_EXPANDER_HEADER: &str = "Expander";
pub const CANONICAL_EXPANDER_BODY: &str = "Body";
pub const CANONICAL_EXPANDER_WIDTH: f32 = 320.0;
pub const CANONICAL_EXPANDER_HEADER_HEIGHT: f32 = 48.0;
pub const CANONICAL_EXPANDER_CONTENT_HEIGHT: f32 = 52.0;
pub const CANONICAL_EXPANDER_CHEVRON_UP: &str = "\u{E70E}";
pub const CANONICAL_EXPANDER_CHEVRON_DOWN: &str = "\u{E70D}";

/// 标准 SplitView 左右分区文案。
pub const CANONICAL_SPLIT_VIEW_PANE_TEXT: &str = "Pane";
pub const CANONICAL_SPLIT_VIEW_CONTENT_TEXT: &str = "Content";
pub const CANONICAL_SPLIT_VIEW_WIDTH: f32 = 320.0;
pub const CANONICAL_SPLIT_VIEW_HEIGHT: f32 = 120.0;
pub const CANONICAL_SPLIT_VIEW_PANE_WIDTH: f32 = 120.0;

/// 标准 CalendarView 规范。
pub const CANONICAL_CALENDAR_WIDTH: f32 = 300.0;
pub const CANONICAL_CALENDAR_HEIGHT: f32 = 260.0;
pub const CANONICAL_CALENDAR_TITLE: &str = "January 2026";
pub const CANONICAL_CALENDAR_DAYS_HEADER: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
pub const CANONICAL_CALENDAR_SELECTED_DAY: u8 = 15;

/// 标准 TimePicker 规范。
pub const CANONICAL_TIME_PICKER_WIDTH: f32 = 200.0;
pub const CANONICAL_TIME_PICKER_HEIGHT: f32 = 32.0;
pub const CANONICAL_TIME_COLUMNS: [&str; 2] = ["13", "30"];

/// 标准 PersonPicture 规范。
pub const CANONICAL_PERSON_PICTURE_INITIALS: &str = "AB";
pub const CANONICAL_PERSON_PICTURE_SIZE: f32 = 48.0;

/// 标准 FontIcon 规范。
pub const CANONICAL_FONT_ICON_GLYPH: &str = "\u{E71D}";
pub const CANONICAL_FONT_ICON_SIZE: f32 = 32.0;
pub const CANONICAL_FONT_ICON_FONT_SIZE: f32 = 24.0;

/// 标准 Pivot 规范。
pub const CANONICAL_PIVOT_ITEMS: [&str; 2] = ["One", "Two"];
pub const CANONICAL_PIVOT_CONTENT: &str = "One";
pub const CANONICAL_PIVOT_WIDTH: f32 = 320.0;
pub const CANONICAL_PIVOT_HEIGHT: f32 = 120.0;
pub const CANONICAL_PIVOT_HEADER_HEIGHT: f32 = 48.0;

/// 标准 Image 规范。
pub const CANONICAL_IMAGE_SIZE: Vec2 = Vec2::new(64.0, 64.0);

/// 标准 Flyout 规范。
pub const CANONICAL_FLYOUT_TEXT: &str = "Flyout";
pub const CANONICAL_FLYOUT_WIDTH: f32 = 200.0;
pub const CANONICAL_FLYOUT_HEIGHT: f32 = 60.0;

/// 标准 MenuFlyout 规范。
pub const CANONICAL_MENU_FLYOUT_ITEMS: [&str; 2] = ["Item 1", "Item 2"];
pub const CANONICAL_MENU_FLYOUT_WIDTH: f32 = 200.0;
pub const CANONICAL_MENU_FLYOUT_ITEM_HEIGHT: f32 = 36.0;

/// 标准 AppBarButton 与 AppBarToggleButton 规范。
pub const CANONICAL_APP_BAR_BUTTON_LABEL: &str = "Save";
pub const CANONICAL_APP_BAR_BUTTON_GLYPH: &str = "\u{E74E}";
pub const CANONICAL_APP_BAR_BUTTON_WIDTH: f32 = 68.0;
pub const CANONICAL_APP_BAR_BUTTON_HEIGHT: f32 = 48.0;

pub const CANONICAL_APP_BAR_TOGGLE_BUTTON_LABEL: &str = "Bold";
pub const CANONICAL_APP_BAR_TOGGLE_BUTTON_GLYPH: &str = "\u{E8DD}";
pub const CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH: f32 = 68.0;
pub const CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT: f32 = 48.0;

/// 标准插桩文本字号（WinUI 3 默认 TextBlock 字号）。
pub const STANDARD_FONT_SIZE: f32 = 14.0;

/// 标准树形行高（WinUI 3 TreeView 标准行高）。
pub const STANDARD_TREE_ITEM_HEIGHT: f32 = 32.0;

/// 标准列表项行高（WinUI 3 ListView 标准行高）。
pub const STANDARD_LIST_ITEM_HEIGHT: f32 = 40.0;

/// 标准堆叠项垂直间距（WinUI 3 StackPanel 常用间距）。
pub const STANDARD_STACK_SPACING: f32 = 8.0;

/// 标准树形层级缩进量（WinUI 3 TreeView 子节点相对父节点的偏移）。
pub const STANDARD_TREE_INDENT: f32 = 40.0;

/// 标准网格视图固有宽度。
pub const STANDARD_GRID_WIDTH: f32 = 260.0;

/// 标准网格视图固有高度。
pub const STANDARD_GRID_HEIGHT: f32 = 140.0;

/// 标准网格单元圆角半径。
pub const STANDARD_GRID_CELL_RADIUS: f32 = 6.0;

/// 标准翻页视图固有宽度。
pub const STANDARD_FLIP_WIDTH: f32 = 220.0;

/// 标准翻页视图固有高度。
pub const STANDARD_FLIP_HEIGHT: f32 = 120.0;

/// 标准下拉刷新容器固有宽度。
pub const STANDARD_REFRESH_WIDTH: f32 = 260.0;

/// 标准下拉刷新容器固有高度。
pub const STANDARD_REFRESH_HEIGHT: f32 = 160.0;

/// 标准环形刷新指示器直径。
pub const STANDARD_REFRESH_INDICATOR_SIZE: f32 = 24.0;

/// 标准标注滚动条固有宽度。
pub const STANDARD_SCROLL_BAR_WIDTH: f32 = 16.0;

/// 标准标注滚动条固有高度。
pub const STANDARD_SCROLL_BAR_HEIGHT: f32 = 160.0;

/// 标准标注滚动条轨道宽度。
pub const STANDARD_SCROLL_BAR_TRACK_WIDTH: f32 = 8.0;

/// 标准选中指示胶囊条宽度。
pub const STANDARD_PILL_WIDTH: f32 = 3.0;

/// 标准选中指示胶囊条高度。
pub const STANDARD_PILL_HEIGHT: f32 = 16.0;

/// 标准选中指示胶囊条圆角半径。
pub const STANDARD_PILL_RADIUS: f32 = 1.5;

/// WinUI 3 官方禁用态不透明度衰减系数（36%）。
pub const WINUI_DISABLED_OPACITY: f32 = 0.36;

/// 插桩状态配置契约。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FixtureState {
    /// 是否处于禁用状态。
    pub disabled: bool,
    /// 是否处于键盘聚焦状态。
    pub focused: bool,
    /// 是否处于选中/勾选状态。
    pub checked: bool,
    /// 交互项初始选中索引（默认为 0）。
    pub selected_index: usize,
}

impl FixtureState {
    /// 默认常规态。
    #[must_use]
    pub const fn normal() -> Self {
        Self {
            disabled: false,
            focused: false,
            checked: false,
            selected_index: 0,
        }
    }

    /// 禁用态。
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            disabled: true,
            focused: false,
            checked: false,
            selected_index: 0,
        }
    }

    /// 选中/勾选态。
    #[must_use]
    pub const fn checked() -> Self {
        Self {
            disabled: false,
            focused: false,
            checked: true,
            selected_index: 0,
        }
    }

    /// 聚焦态。
    #[must_use]
    pub const fn focused() -> Self {
        Self {
            disabled: false,
            focused: true,
            checked: false,
            selected_index: 0,
        }
    }

    /// 设置禁用状态。
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 设置选中索引。
    #[must_use]
    pub const fn with_selected(mut self, selected_index: usize) -> Self {
        self.selected_index = selected_index;
        self
    }

    /// 从 ControlState 与 checked 标志派生插桩状态。
    #[must_use]
    pub const fn from_control_state(control: ControlState, checked: bool) -> Self {
        Self {
            disabled: control.disabled,
            focused: control.focused,
            checked,
            selected_index: 0,
        }
    }
}

/// 通用路径/分段插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct PathFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各路径项对应的包围盒矩形。
    pub item_rects: Vec<Rect>,
    /// 各分隔符对应的包围盒矩形。
    pub separator_rects: Vec<Rect>,
    /// 插桩内容测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用树形展开插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct TreeFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 根节点包围盒矩形。
    pub root_rect: Rect,
    /// 子节点包围盒矩形。
    pub child_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用垂直纯文本行插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct TextRowsFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各文本行对应的包围盒矩形。
    pub row_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用可交互列表行插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct ListRowsFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各行条目对应的包围盒矩形。
    pub item_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 PipsPager 插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct PipsPagerFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各圆点对应的包围盒矩形。
    pub pip_rects: Vec<Rect>,
    /// 选中的圆点索引。
    pub selected_index: usize,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 CommandBar 插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct CommandBarFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各命令按钮对应的包围盒矩形。
    pub button_rects: Vec<Rect>,
    /// 分隔线对应的包围盒矩形。
    pub separator_rect: Option<Rect>,
    /// 更多操作按钮对应的包围盒矩形。
    pub more_button_rect: Option<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 MenuBar 插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct MenuBarFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各菜单项对应的包围盒矩形。
    pub item_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 TitleBar 插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct TitleBarFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 标题对应的包围盒矩形。
    pub title_rect: Rect,
    /// 副标题对应的包围盒矩形。
    pub subtitle_rect: Rect,
    /// 窗口控制按钮对应的包围盒矩形。
    pub caption_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 NavigationView 插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct NavigationViewFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各菜单项对应的包围盒矩形。
    pub item_rects: Vec<Rect>,
    /// 底部设置项对应的包围盒矩形。
    pub settings_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 TabView 插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct TabViewFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各标签页对应的包围盒矩形。
    pub tab_rects: Vec<Rect>,
    /// 内容区对应的包围盒矩形。
    pub content_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用网格集合视图插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct GridFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各单元格对应的包围盒矩形。
    pub cell_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用翻页视图插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct FlipViewFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 内容卡片包围盒矩形。
    pub card_rect: Rect,
    /// 上一页导航按钮矩形。
    pub prev_button_rect: Rect,
    /// 下一页导航按钮矩形。
    pub next_button_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用下拉刷新容器插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct RefreshFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 顶部环形指示器外框矩形。
    pub indicator_rect: Rect,
    /// 下方内容卡片矩形。
    pub card_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用标注滚动条插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct AnnotatedScrollBarFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 滚动条轨道矩形。
    pub track_rect: Rect,
    /// 滑块矩形。
    pub thumb_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用 InfoBar 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct InfoBarFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 Expander 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct ExpanderFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 头部矩形。
    pub header_rect: Rect,
    /// 内容区矩形。
    pub content_rect: Rect,
}

/// 通用 SplitView 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct SplitViewFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 侧边窗格矩形。
    pub pane_rect: Rect,
    /// 主内容区矩形。
    pub content_rect: Rect,
}

/// 通用 CalendarView 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct CalendarViewFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 TimePicker 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct TimePickerFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 PersonPicture 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct PersonPictureFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 FontIcon 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct FontIconFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 Pivot 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct PivotFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 头部矩形。
    pub header_rect: Rect,
    /// 内容区矩形。
    pub content_rect: Rect,
}

/// 通用 Image 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct ImageFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 Flyout 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct FlyoutFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 MenuFlyout 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct MenuFlyoutFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 AppBarButton 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct AppBarButtonFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 通用 AppBarToggleButton 插桩输出。
#[derive(Clone, Debug, Default)]
pub struct AppBarToggleButtonFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 整体尺寸。
    pub total_size: Vec2,
    /// 整体包围矩形。
    pub rect: Rect,
}

/// 发射标准化 3 级路径插桩（`Root > Folder > File.txt`）。
///
/// 几何与排版规则：
/// - 水平顺序排布 `Root`、`>`、`Folder`、`>`、`File.txt`；
/// - 标签项与分隔符之间固定留有 4px 水平间距；
/// - 末尾项为当前项：未禁用时字重为 600、颜色为 [`Token::TEXT_PRIMARY`]；
/// - 前序项与分隔符：未禁用时字重为 400、颜色为 [`Token::TEXT_SECONDARY`]；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，所有文本与分隔符强制降级为字重 400 与 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_path_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> PathFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut item_rects = Vec::with_capacity(3);
    let mut separator_rects = Vec::with_capacity(2);

    let row_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        32.0
    };
    let gap = 5.0;
    let mut cur_x = rect.origin.x + 1.0;

    let sep_measure = text_ctx.measure_line(CANONICAL_PATH_SEPARATOR, 12.0, 400);
    let sep_w = sep_measure.x.ceil().max(8.0);

    for (idx, &item_text) in CANONICAL_PATH_ITEMS.iter().enumerate() {
        let is_last = idx == CANONICAL_PATH_ITEMS.len() - 1;
        let weight = 400;
        let text_color = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };

        let item_measure = text_ctx.measure_line(item_text, STANDARD_FONT_SIZE, weight);
        let item_w = item_measure.x.ceil().max(20.0);
        let item_r = Rect::new(cur_x, rect.origin.y, item_w, row_h);
        item_rects.push(item_r);

        let quads = text_ctx.shape_line_styled(
            item_text,
            item_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_color,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_x += item_w;

        if !is_last {
            cur_x += gap;
            let sep_r = Rect::new(cur_x, rect.origin.y, sep_w, row_h);
            separator_rects.push(sep_r);

            let sep_color = if state.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };

            let sep_quads = text_ctx.shape_line_styled(
                CANONICAL_PATH_SEPARATOR,
                sep_r,
                12.0,
                400,
                None,
                AlignX::Center,
                AlignY::Middle,
            );
            primitive::glyphs(
                &sep_quads,
                sep_color,
                z + 2,
                &mut out.primitives,
                &mut out.payloads,
            );

            cur_x += sep_w + gap;
        }
    }

    if state.focused && !state.disabled {
        let focus_idx = if state.selected_index < item_rects.len() {
            state.selected_index
        } else {
            item_rects.len().saturating_sub(1)
        };
        if let Some(r) = item_rects.get(focus_idx) {
            out.primitives.push(primitive::focus_ring(
                r.inflate(2.0),
                4.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }
    }

    let total_w = cur_x - rect.origin.x;
    PathFixtureOutput {
        output: out,
        item_rects,
        separator_rects,
        total_size: Vec2::new(total_w, row_h),
    }
}

/// 发射标准化分段选择条插桩（`Day | Week | Month`）。
///
/// 几何与排版规则：
/// - 将 `rect` 水平等分为 3 个槽位；
/// - 选中项（由 `state.selected_index` 决定，默认 0）：
///   - 未禁用时：绘制 [`Token::HOVER`] 背景表面与左侧 3×16px 胶囊指示条（[`Token::PRIMARY`]），文本字重 600、颜色 [`Token::TEXT_PRIMARY`]；
///   - 禁用时：抑制背景表面与胶囊条，文本字重 400、颜色 [`Token::TEXT_DISABLED`]；
/// - 未选中项：
///   - 未禁用时：字重 400、颜色 [`Token::TEXT_SECONDARY`]；
///   - 禁用时：字重 400、颜色 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_segmented_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let slot_count = CANONICAL_SEGMENTED_ITEMS.len() as f32;
    let pad = 2.0;
    let available_w = (rect.width() - pad * 2.0).max(1.0);
    let slot_w = available_w / slot_count;
    let slot_h = (rect.height() - pad * 2.0).max(1.0);

    for (idx, &text) in CANONICAL_SEGMENTED_ITEMS.iter().enumerate() {
        let slot_x = rect.origin.x + pad + idx as f32 * slot_w;
        let slot_r = Rect::new(slot_x, rect.origin.y + pad, slot_w, slot_h);
        let is_selected = idx == state.selected_index;

        if is_selected && !state.disabled {
            out.primitives
                .push(primitive::surface(slot_r, 4.0, Token::HOVER, z + 2));

            let pill = Rect::new(
                slot_r.origin.x + 3.0,
                slot_r.origin.y + (slot_h - STANDARD_PILL_HEIGHT) * 0.5,
                STANDARD_PILL_WIDTH,
                STANDARD_PILL_HEIGHT,
            );
            out.primitives.push(primitive::surface(
                pill,
                STANDARD_PILL_RADIUS,
                if state.disabled {
                    Token::CONTROL_DISABLED_THUMB
                } else {
                    Token::PRIMARY
                },
                z + 5,
            ));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    slot_r.inflate(1.0),
                    5.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 10,
                    true,
                ));
            }
        }

        let text_color = if state.disabled {
            Token::TEXT_DISABLED
        } else if is_selected {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        };

        let weight = if is_selected && !state.disabled {
            600
        } else {
            400
        };

        let quads = text_ctx.shape_line_styled(
            text,
            slot_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_color,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    out
}

/// 发射标准化选择条插桩（WinUI 3 SelectorBar: `Day | Week | Month`）。
///
/// 几何与排版规则（对齐 WinUI 3 参考呈现）：
/// - 不绘制任何外层背景表面或描边（完全透明透出窗口背景）；
/// - 项宽度按内容与 padding 排布：各项宽度分别为 48px、57px、69px；
/// - 各项高度 40px，文本使用 14px Regular 字形，水平居中对齐；
/// - 选中项（由 `state.selected_index` 决定，默认 0）：
///   - 选中项文本下方（y=40..43）绘制 16×3px 的胶囊指示条（圆角 1.5px）；
///   - normal 态：文本颜色 [`Token::TEXT_PRIMARY`]，指示条颜色 [`Token::PRIMARY`]；
///   - disabled 态：文本与指示条颜色均为 [`Token::TEXT_DISABLED`]；
/// - 未选中项：
///   - normal 态：文本颜色 [`Token::TEXT_SECONDARY`]；
///   - disabled 态：文本颜色 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_selector_bar_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    // 参考宿主中 Day、Week、Month 三项的实际左边界与指示条位置
    let item_offsets = [12.0_f32, 60.0, 119.0];
    let row_h = 40.0;

    if state.selected_index == 0 {
        let indicator_w = 16.0;
        let indicator_h = 3.0;
        let indicator_x = rect.origin.x + 16.0;
        let indicator_y = rect.origin.y + row_h;
        let indicator_r = Rect::new(indicator_x, indicator_y, indicator_w, indicator_h);
        let indicator_color = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::PRIMARY
        };
        out.primitives
            .push(primitive::surface(indicator_r, 1.5, indicator_color, z + 2));
    }

    let text_color = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    for (idx, &text) in CANONICAL_SEGMENTED_ITEMS.iter().enumerate() {
        let x = rect.origin.x + item_offsets.get(idx).copied().unwrap_or(0.0);
        let item_r = Rect::new(x, rect.origin.y + 4.0, 60.0, row_h);

        let quads =
            text_ctx.shape_line_styled(text, item_r, 14.0, 400, None, AlignX::Left, AlignY::Middle);
        primitive::glyphs(
            &quads,
            text_color,
            z + 1,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    out
}

/// 发射标准化 2 级展开树形插桩（根节点 `Root` 选中展开 + 子节点 `Child` 缩进）。
///
/// 几何与排版规则：
/// - 根节点（深度 0，行高 [`STANDARD_TREE_ITEM_HEIGHT`]）：
///   - 包含展开箭头（朝下 chevron，thickness 1.5px）；
///   - 首项处于选中态：左侧垂直居中绘制 3×16px、圆角 1.5px 的 Accent 胶囊条（[`Token::PRIMARY`]），填充 [`Token::HOVER`] 背景；
///   - 标签 `"Root"` 位于箭头右侧（x 偏移 32px）；
/// - 子节点（深度 1，缩进 [`STANDARD_TREE_INDENT`]，行高 [`STANDARD_TREE_ITEM_HEIGHT`]）：
///   - WinUI 参考形态没有深度引导线，只画缩进后的标签；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，抑制 Accent 胶囊条与选中背景，chevron 指示器与文本全量变灰衰减为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_tree_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TreeFixtureOutput {
    let mut out = WidgetOutput::default();
    // WinUI 的 TreeViewItem 参考行高是 32px（不是 36），行与行之间没有额外间距。
    let item_h = STANDARD_TREE_ITEM_HEIGHT;
    let tree_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        220.0
    };

    // 1. Root 根节点行（深度 0，首项选中；WinUI 在禁用态仍保留选中行底色与指示胶囊）
    let root_y = rect.origin.y + 4.0;
    let root_r = Rect::new(rect.origin.x + 4.0, root_y, (tree_w - 8.0).max(1.0), item_h);
    let root_selected = state.checked || state.selected_index == 0;

    if root_selected {
        out.primitives
            .push(primitive::surface(root_r, 4.0, Token::HOVER, z + 2));

        let pill = Rect::new(
            root_r.origin.x,
            root_y + (item_h - STANDARD_PILL_HEIGHT) * 0.5,
            STANDARD_PILL_WIDTH,
            STANDARD_PILL_HEIGHT,
        );
        out.primitives.push(primitive::surface(
            pill,
            STANDARD_PILL_RADIUS,
            if state.disabled {
                Token::CONTROL_DISABLED_THUMB
            } else {
                Token::PRIMARY
            },
            z + 5,
        ));

        if state.focused {
            out.primitives.push(primitive::focus_ring(
                root_r.inflate(1.0),
                5.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }
    }

    // 根节点折叠/展开指示箭头（已展开：朝下箭头）
    let chevron_cx = root_r.origin.x + 16.0;
    let chevron_cy = root_y + item_h * 0.5;
    let chevron_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    primitive::curve_stroke(
        [chevron_cx - 4.0, chevron_cy - 2.0],
        [chevron_cx, chevron_cy + 2.0],
        1.5,
        chevron_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );
    primitive::curve_stroke(
        [chevron_cx, chevron_cy + 2.0],
        [chevron_cx + 4.0, chevron_cy - 2.0],
        1.5,
        chevron_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 2. Child 子节点行（深度 1，缩进 40px）
    let indent = STANDARD_TREE_INDENT;
    let child_y = root_y + item_h - 2.0;
    let child_r = Rect::new(
        rect.origin.x + 4.0 + indent,
        child_y,
        (tree_w - 8.0 - indent).max(1.0),
        item_h,
    );

    // 文本排版
    let text_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let root_weight = if root_selected { 600 } else { 400 };
    let root_label_r = Rect::new(
        root_r.origin.x + 32.0,
        root_r.origin.y,
        (root_r.max().x - (root_r.origin.x + 32.0)).max(1.0),
        root_r.height(),
    );
    let root_quads = text_ctx.shape_line_styled(
        CANONICAL_TREE_ROOT,
        root_label_r,
        STANDARD_FONT_SIZE,
        root_weight,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &root_quads,
        text_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    let child_label_r = Rect::new(
        child_r.origin.x + 16.0,
        child_r.origin.y,
        (child_r.max().x - (child_r.origin.x + 16.0)).max(1.0),
        child_r.height(),
    );
    let child_quads = text_ctx.shape_line_styled(
        CANONICAL_TREE_CHILD,
        child_label_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &child_quads,
        text_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    TreeFixtureOutput {
        output: out,
        root_rect: root_r,
        child_rect: child_r,
        total_size: Vec2::new(tree_w, item_h * 2.0 + 2.0),
    }
}

/// 发射标准化垂直纯文本列表插桩（`Row 1`、`Row 2`、`Row 3`），用于 `StackPanel`、`ScrollView` 等。
///
/// 几何与排版规则：
/// - 固定输出 3 行纯文本，字号 14px，间距由 `spacing` 控制（默认建议 8px）；
/// - 未禁用状态：使用 [`Token::TEXT_PRIMARY`]；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，所有行文本同步应用 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_text_rows_fixture(
    rect: Rect,
    spacing: f32,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TextRowsFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut row_rects = Vec::with_capacity(3);

    // WinUI 3 的 `TextBlock` 在 14px 正文下自然行盒为 19px（Segoe UI Variable
    // 的 ascent/descent 合计取整），三行加 8px 间距的固有高度是 73px。
    // 此前按 20px 排布会累计多出 3px，使第三行整体下移 2px。
    let line_h = 19.0;
    let mut cur_y = rect.origin.y;
    let text_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let row_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        200.0
    };

    for &row_text in &CANONICAL_ROW_ITEMS {
        let row_r = Rect::new(rect.origin.x, cur_y, row_w, line_h);
        row_rects.push(row_r);

        let quads = text_ctx.shape_line_styled(
            row_text,
            row_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_y += line_h + spacing;
    }

    let total_h = (cur_y - spacing) - rect.origin.y;
    TextRowsFixtureOutput {
        output: out,
        row_rects,
        total_size: Vec2::new(row_w, total_h),
    }
}

/// 发射标准化可交互垂直列表行插桩，用于 `ListView`、`ListBox` 等。
///
/// 几何与排版规则：
/// - 固定输出 3 个标准行（标准行高 40px）；
/// - 首项处于选中态（`state.selected_index == 0`）：
///   - 未禁用时：绘制 [`Token::HOVER`] 背景表面与左侧 3×16px 胶囊指示条（[`Token::PRIMARY`]），文本字重 500、颜色 [`Token::TEXT_PRIMARY`]；
///   - 禁用时：抑制背景表面与胶囊指示条，文本字重 400、颜色 [`Token::TEXT_DISABLED`]；
/// - 其他行：文本字重 400，未禁用时为 [`Token::TEXT_PRIMARY`]，禁用时为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_list_rows_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> ListRowsFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut item_rects = Vec::with_capacity(3);

    let item_h = STANDARD_LIST_ITEM_HEIGHT;
    let list_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        220.0
    };

    for (idx, &text) in CANONICAL_LIST_ITEMS.iter().enumerate() {
        let y = rect.origin.y + 2.0 + idx as f32 * item_h;
        let row_r = Rect::new(
            rect.origin.x + 4.0,
            y + 2.0,
            (list_w - 8.0).max(1.0),
            item_h - 4.0,
        );
        item_rects.push(row_r);

        // 参考截图里 ListView 的选中高亮在禁用态仍然保留（只是整体变淡）。
        let is_selected = idx == state.selected_index;

        if is_selected {
            out.primitives
                .push(primitive::surface(row_r, 4.0, Token::HOVER, z + 2));

            let pill = Rect::new(
                row_r.origin.x,
                y + (item_h - STANDARD_PILL_HEIGHT) * 0.5,
                STANDARD_PILL_WIDTH,
                STANDARD_PILL_HEIGHT,
            );
            out.primitives.push(primitive::surface(
                pill,
                STANDARD_PILL_RADIUS,
                if state.disabled {
                    Token::CONTROL_DISABLED_THUMB
                } else {
                    Token::PRIMARY
                },
                z + 5,
            ));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    row_r.inflate(1.0),
                    5.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 10,
                    true,
                ));
            }
        }

        // WinUI 3 官方参考截图中 ListViewItem 的 TextBlock 墨水无论启用/禁用均保持主文本色。
        let text_ink = Token::TEXT_PRIMARY;

        let label_r = Rect::new(
            rect.origin.x + 16.0,
            row_r.origin.y,
            (row_r.max().x - (rect.origin.x + 16.0)).max(1.0),
            row_r.height(),
        );

        let weight = if is_selected { 500 } else { 400 };
        let mut quads = text_ctx.shape_line_styled(
            text,
            label_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        for quad in &mut quads {
            quad.rect.origin.y += 1.0;
        }
        primitive::glyphs(
            &quads,
            text_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    ListRowsFixtureOutput {
        output: out,
        item_rects,
        total_size: Vec2::new(list_w, item_h * 3.0),
    }
}

/// 发射 WinUI `ListBox` 的行插桩：选中行铺满控件宽度的强调色淡化底色，
/// 没有 ListView 的左侧胶囊指示条，其余行只有文本。
pub fn emit_canonical_listbox_rows_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> ListRowsFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut item_rects = Vec::with_capacity(3);
    let item_h = STANDARD_LIST_ITEM_HEIGHT;
    let list_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        220.0
    };

    let container_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        item_h * 3.0
    };
    out.primitives.push(primitive::surface(
        Rect::new(rect.origin.x, rect.origin.y, list_w, container_h),
        0.0,
        Token::CARD_BG,
        z,
    ));

    for (idx, &text) in CANONICAL_LIST_ITEMS.iter().enumerate() {
        let y = rect.origin.y + idx as f32 * item_h;
        let row_r = Rect::new(rect.origin.x, y, list_w, item_h);
        item_rects.push(row_r);

        let is_selected = !state.disabled && idx == state.selected_index;
        if is_selected {
            out.primitives
                .push(primitive::surface(row_r, 0.0, Token::PRIMARY_SOFT, z + 2));
        }

        let text_ink = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        let label_r = Rect::new(rect.origin.x + 12.0, y, (list_w - 12.0).max(1.0), item_h);
        let mut quads = text_ctx.shape_line_styled(
            text,
            label_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        for quad in &mut quads {
            quad.rect.origin.y -= 1.0;
        }
        primitive::glyphs(
            &quads,
            text_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    ListRowsFixtureOutput {
        output: out,
        item_rects,
        total_size: Vec2::new(list_w, item_h * 3.0),
    }
}

/// 发射标准化 PipsPager 分页指示点插桩（固定 5 页，第 2 页选中）。
///
/// 几何与排版规则：
/// - 水平居中排布 5 个圆点，中心间距 16px；
/// - 选中项（由 `state.selected_index` 决定，默认 1）：
///   - 未禁用时：直径 10px，填充 [`Token::PRIMARY`]；
///   - 禁用时：直径 10px，填充 [`Token::CONTROL_DISABLED_THUMB`]；
///   - 聚焦时：绘制跟随选中圆点的圆形焦点环；
/// - 未选中项：
///   - 未禁用时：直径 6px，填充 [`Token::CONTROL_TRACK`]；
///   - 禁用时：直径 6px，填充 [`Token::CONTROL_DISABLED_THUMB`]。
pub fn emit_canonical_pips_pager_fixture(
    rect: Rect,
    state: FixtureState,
    z: u16,
) -> PipsPagerFixtureOutput {
    let mut out = WidgetOutput::default();
    let count = CANONICAL_PIPS_COUNT;
    let mut pip_rects = Vec::with_capacity(count);

    let total_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        320.0
    };
    let total_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        32.0
    };
    let center_x = rect.origin.x + total_w * 0.5;
    let center_y = rect.origin.y + total_h * 0.5;

    let span = (count - 1) as f32 * CANONICAL_PIP_SPACING;
    let start_x = center_x - span * 0.5;

    let sel_idx = if state.selected_index == 0 {
        CANONICAL_PIPS_SELECTED_INDEX
    } else if state.selected_index < count {
        state.selected_index
    } else {
        CANONICAL_PIPS_SELECTED_INDEX
    };

    for idx in 0..count {
        let cx = start_x + idx as f32 * CANONICAL_PIP_SPACING;
        let is_selected = idx == sel_idx;
        let size = if is_selected {
            CANONICAL_PIP_SIZE_SELECTED
        } else {
            CANONICAL_PIP_SIZE_NORMAL
        };
        let pip_r = Rect::new(cx - size * 0.5, center_y - size * 0.5, size, size);
        pip_rects.push(pip_r);

        let fill = if state.disabled {
            // WinUI 的 PipsPager 禁用圆点整体降到 `SliderTrackValueFillDisabled`
            // （浅色 #A6A6A6），比 `ControlStrongFillColorDisabled` 更亮。
            Token::SLIDER_TRACK_DISABLED
        } else if is_selected {
            // WinUI PipsPager 选中圆点使用中性灰（TextFillColorSecondary / CONTROL_TRACK），而非强调蓝。
            Token::CONTROL_TRACK
        } else {
            Token::CONTROL_TRACK
        };

        out.primitives
            .push(primitive::surface(pip_r, size * 0.5, fill, z + 2));

        if is_selected && state.focused && !state.disabled {
            out.primitives.push(primitive::focus_ring(
                pip_r.inflate(3.0),
                (size * 0.5) + 3.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }
    }

    PipsPagerFixtureOutput {
        output: out,
        pip_rects,
        selected_index: sel_idx,
        total_size: Vec2::new(total_w, total_h),
    }
}

/// 发射标准化 CommandBar 命令栏插桩（`Save`、`Open`、竖向分隔符与更多按钮）。
///
/// 几何与排版规则：
/// - 整体高度 48px，宽度对齐 `rect.width()`（默认 320px）；
/// - 左侧依次排布 2 枚 `AppBarButton`（宽 68px，上部 16px 图标，下部 12px 标签文本）；
/// - 按钮后跟随 1px 宽竖向分隔符（高 24px，垂直居中）；
/// - 右侧对齐排布 `...` 更多操作按钮（宽 40px）；
/// - 未禁用时使用 [`Token::TEXT_PRIMARY`]，禁用时递归衰减为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_command_bar_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> CommandBarFixtureOutput {
    let mut out = WidgetOutput::default();
    let bar_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        320.0
    };
    let bar_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        48.0
    };
    let bar_r = Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h);

    // WinUI CommandBar 闭合态背景透明、命令右对齐且仅显示图标（隐藏底部标签）。
    let more_w = 48.0;
    let btn_w = 68.0;
    let sep_w = 1.0;
    let total_cmd_w = CANONICAL_COMMAND_ITEMS.len() as f32 * btn_w + sep_w + 8.0 + more_w;
    let mut button_rects = Vec::with_capacity(CANONICAL_COMMAND_ITEMS.len());
    let mut cur_x = (bar_r.max().x - total_cmd_w).max(bar_r.origin.x);

    let icon_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    for (idx, &(glyph, _label)) in CANONICAL_COMMAND_ITEMS.iter().enumerate() {
        let btn_r = Rect::new(cur_x, bar_r.origin.y + 2.0, btn_w, bar_h - 4.0);
        button_rects.push(btn_r);

        if idx == 0 && state.focused && !state.disabled {
            out.primitives.push(primitive::focus_ring(
                btn_r.inflate(1.0),
                4.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }

        let icon_quads = text_ctx.shape_line_styled(
            glyph,
            btn_r,
            16.0,
            400,
            Some("Segoe Fluent Icons"),
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &icon_quads,
            icon_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_x += btn_w;
    }

    let sep_r = Rect::new(
        cur_x + 4.0,
        bar_r.origin.y + (bar_h - 24.0) * 0.5,
        sep_w,
        24.0,
    );
    let sep_ink = if state.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::DIVIDER
    };
    out.primitives
        .push(primitive::surface(sep_r, 0.0, sep_ink, z + 2));

    let more_r = Rect::new(
        bar_r.max().x - more_w,
        bar_r.origin.y + 2.0,
        more_w,
        bar_h - 4.0,
    );
    let more_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let more_quads = text_ctx.shape_line_styled(
        CANONICAL_COMMAND_MORE_GLYPH,
        more_r,
        16.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &more_quads,
        more_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    CommandBarFixtureOutput {
        output: out,
        button_rects,
        separator_rect: Some(sep_r),
        more_button_rect: Some(more_r),
        total_size: Vec2::new(bar_w, bar_h),
    }
}

/// 发射标准化 MenuBar 菜单栏插桩（`File` 与 `Edit`）。
///
/// 几何与排版规则：
/// - 默认行高 40px，宽度对齐 `rect.width()`（默认 320px）；
/// - 左侧横向排列 `File` 与 `Edit` 两个顶层菜单项；
/// - 未禁用时使用 [`Token::TEXT_PRIMARY`]，禁用时统一降级为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_menu_bar_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> MenuBarFixtureOutput {
    let mut out = WidgetOutput::default();
    let bar_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        320.0
    };
    let bar_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        40.0
    };
    let bar_r = Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h);

    let text_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let mut item_rects = Vec::with_capacity(CANONICAL_MENU_ITEMS.len());
    let mut cur_x = bar_r.origin.x + 2.0;
    // WinUI 的 MenuBar 菜单项固定 40px 行高，并在菜单栏高度内垂直居中；
    // 绘制整条菜单栏高度会让文字随栏高下移。
    let row_h = 40.0_f32.min(bar_h);
    // 参考截图的菜单文字基线比通用 Middle 位置低 2px（菜单栏行盒按 40px 对齐）。
    let row_y = bar_r.origin.y + (bar_h - row_h) * 0.5 + 2.0;

    for (idx, &item_text) in CANONICAL_MENU_ITEMS.iter().enumerate() {
        let measured = text_ctx.measure_line(item_text, STANDARD_FONT_SIZE, 400);
        let item_w = (measured.x + 20.0).max(44.0);
        let item_r = Rect::new(cur_x, row_y, item_w, row_h);
        item_rects.push(item_r);

        if idx == 0 && state.focused && !state.disabled {
            out.primitives.push(primitive::focus_ring(
                item_r.inflate(1.0),
                4.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }

        let quads = text_ctx.shape_line_styled(
            item_text,
            item_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_x += item_w + 4.0;
    }

    MenuBarFixtureOutput {
        output: out,
        item_rects,
        total_size: Vec2::new(bar_w, bar_h),
    }
}

/// 发射标准化 TitleBar 标题栏插桩（`Document`、`WinUiReference` 与三枚控制按钮）。
///
/// 几何与排版规则：
/// - 默认高度 32px，宽度对齐 `rect.width()`（默认 420px）；
/// - 左侧垂直居中排布窗口主标题 `Document`（12px，SemiBold 600，[`Token::TEXT_PRIMARY`]）；
/// - 主标题右侧相隔 8px 紧随副标题 `WinUiReference`（12px，Regular 400，[`Token::TEXT_SECONDARY`]），两端保持完全统一的基线排版；
/// - 右侧自右向左紧贴排布三枚标准无边框窗口控制按钮（各 46px 宽）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，标题、副标题与控制按钮同步降级为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_title_bar_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TitleBarFixtureOutput {
    let mut out = WidgetOutput::default();
    let bar_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        420.0
    };
    let bar_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        32.0
    };
    let bar_r = Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h);

    // WinUI 独立 TitleBar 控件背景透明、无底部分隔线，窗口控制按钮由宿主窗口非客户区负责。
    let chrome_fill = Token::HEADER_BG;
    out.primitives
        .push(primitive::surface(bar_r, 0.0, chrome_fill, z));

    let title_x = bar_r.origin.x + 16.0;
    let title_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let subtitle_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };

    let title_measured = text_ctx.measure_line(CANONICAL_TITLE_BAR_TITLE, 12.0, 600);
    let title_w = title_measured.x.ceil().max(20.0);
    let title_r = Rect::new(title_x, bar_r.origin.y, title_w, bar_h);

    let title_quads = text_ctx.shape_line_styled(
        CANONICAL_TITLE_BAR_TITLE,
        title_r,
        12.0,
        600,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &title_quads,
        title_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    let sub_x = title_x + title_w + 8.0;
    let sub_measured = text_ctx.measure_line(CANONICAL_TITLE_BAR_SUBTITLE, 12.0, 400);
    let sub_w = sub_measured.x.ceil().max(20.0);
    let sub_r = Rect::new(sub_x, bar_r.origin.y, sub_w, bar_h);

    let sub_quads = text_ctx.shape_line_styled(
        CANONICAL_TITLE_BAR_SUBTITLE,
        sub_r,
        12.0,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &sub_quads,
        subtitle_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    let caption_w = crate::title_bar::TITLE_BAR_CAPTION_WIDTH;
    let kinds = [
        crate::title_bar::WindowCaptionKind::Minimize,
        crate::title_bar::WindowCaptionKind::Maximize { maximized: false },
        crate::title_bar::WindowCaptionKind::Close,
    ];
    let mut caption_rects = Vec::with_capacity(kinds.len());
    let count = kinds.len();
    for (index, _kind) in kinds.into_iter().enumerate() {
        let left = bar_r.max().x - caption_w * (count - index) as f32;
        let button_rect = Rect::new(left, bar_r.origin.y, caption_w, bar_h);
        caption_rects.push(button_rect);
    }

    TitleBarFixtureOutput {
        output: out,
        title_rect: title_r,
        subtitle_rect: sub_r,
        caption_rects,
        total_size: Vec2::new(bar_w, bar_h),
    }
}

/// 发射标准化 NavigationView 导航窗格插桩（`Home`、`Library` 与底部 `Settings`）。
///
/// 几何与排版规则：
/// - 默认宽度 300px，高度 200px；
/// - 左侧窗格整体铺 [`Token::CARD_BG`]，右沿带 1px [`Token::CARD_BORDER`] 分隔；
/// - 首项 `Home` 处于默认选中态：绘制 3×16px Accent 胶囊条与 [`Token::NAV_SELECTED`] 背景；
/// - 底部相隔 1px 细线排布 `Settings` 菜单项；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，抑制胶囊条与选中背景，所有图标与文本统一下发 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_navigation_view_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> NavigationViewFixtureOutput {
    let mut out = WidgetOutput::default();
    let panel_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        300.0
    };
    let panel_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        200.0
    };
    let panel_r = Rect::new(rect.origin.x, rect.origin.y, panel_w, panel_h);

    let item_h = 40.0;
    let pad_x = 4.0;
    let pad_y = 4.0;
    let row_w = panel_w - pad_x * 2.0;

    // 参考截图只绘制导航项本身：没有整块窗格底板，也没有右侧分隔线。
    // 顶部折叠按钮：禁用态在深色模式下保留 40×36 圆角底板（对齐 WinUI 实测包围盒 [20,22]）。
    if state.disabled {
        out.primitives.push(primitive::surface(
            Rect::new(panel_r.origin.x + 4.0, panel_r.origin.y + 6.0, 40.0, 36.0),
            4.0,
            Token::CONTROL_DISABLED_BG,
            z,
        ));
    }
    let hamburger_x = panel_r.origin.x + 17.0;
    for offset in [19.0_f32, 24.0, 28.0] {
        out.primitives.push(primitive::surface(
            Rect::new(hamburger_x, panel_r.origin.y + offset, 12.0, 1.0),
            0.0,
            Token::TEXT_PRIMARY,
            z + 1,
        ));
    }

    let disabled = state.disabled;
    let selected_idx = state.selected_index;
    let mut item_rects = Vec::with_capacity(CANONICAL_NAV_ITEMS.len());

    let icon_x = panel_r.origin.x + 19.0;
    let text_x = panel_r.origin.x + 47.0;

    for (idx, &(glyph, label)) in CANONICAL_NAV_ITEMS.iter().enumerate() {
        let is_selected = !disabled && (idx == selected_idx);
        let row_y = panel_r.origin.y + 44.0 + idx as f32 * item_h;
        let row_r = Rect::new(panel_r.origin.x + pad_x, row_y, row_w, item_h);
        item_rects.push(row_r);

        // 参考图的选中项只有左侧胶囊，没有整行底色。
        let row_fill = Token::NONE;
        out.primitives
            .push(primitive::surface(row_r, 4.0, row_fill, z + 2));

        if is_selected {
            let pill = Rect::new(
                panel_r.origin.x + 16.0,
                row_r.origin.y + (item_h - 16.0) * 0.5,
                3.0,
                16.0,
            );
            out.primitives
                .push(primitive::surface(pill, 1.5, Token::PRIMARY, z + 5));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    row_r.inflate(1.0),
                    5.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 10,
                    true,
                ));
            }
        }

        let icon_ink = if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        let text_ink = if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };

        let icon_r = Rect::new(icon_x, row_r.origin.y, 16.0, item_h);
        let icon_quads = text_ctx.shape_line_styled(
            glyph,
            icon_r,
            16.0,
            400,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &icon_quads,
            icon_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );

        let weight = if is_selected { 600 } else { 400 };
        let text_r = Rect::new(
            text_x,
            row_r.origin.y,
            row_w - (text_x - row_r.origin.x),
            item_h,
        );
        let text_quads = text_ctx.shape_line_styled(
            label,
            text_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &text_quads,
            text_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    let footer_y = panel_r.origin.y + panel_h - item_h - pad_y;
    // 参考截图的 Settings 前没有横贯窗格的分隔线。
    let settings_r = Rect::new(panel_r.origin.x + pad_x, footer_y, row_w, item_h);
    out.primitives
        .push(primitive::surface(settings_r, 4.0, Token::NONE, z + 2));

    let settings_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };

    let set_icon_r = Rect::new(icon_x, settings_r.origin.y, 16.0, item_h);
    let set_icon_quads = text_ctx.shape_line_styled(
        CANONICAL_NAV_SETTINGS.0,
        set_icon_r,
        16.0,
        400,
        None,
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &set_icon_quads,
        settings_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    let set_text_r = Rect::new(
        text_x,
        settings_r.origin.y,
        row_w - (text_x - settings_r.origin.x),
        item_h,
    );
    let set_text_quads = text_ctx.shape_line_styled(
        CANONICAL_NAV_SETTINGS.1,
        set_text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &set_text_quads,
        settings_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    NavigationViewFixtureOutput {
        output: out,
        item_rects,
        settings_rect: settings_r,
        total_size: Vec2::new(panel_w, panel_h),
    }
}

/// 发射标准化 TabView 标签页组件插桩（`Tab 1`、`Tab 2` 与内容区 `Content`）。
///
/// 几何与排版规则：
/// - 默认宽度 360px，高度 120px；
/// - 顶部为 36px 高度的标签条，底部垫 1px [`Token::DIVIDER`] 细线；
/// - 首项 `Tab 1` 处于激活态（[`Token::SURFACE`] 背景与底部 2px 强调色指示条），右侧带 `x` 关闭图标；
/// - 标签条下方为内容区（[`Token::CARD_BG`] 底板），左上角内边距 16px 排布纯文本 `Content`；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，抑制底色与指示条，标签页、关闭图标与正文全量变灰衰减为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_tab_view_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TabViewFixtureOutput {
    let mut out = WidgetOutput::default();
    let tab_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        360.0
    };
    let tab_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        120.0
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, tab_w, tab_h);

    let strip_h = 39.0;
    let tab_item_w = 176.0;
    let disabled = state.disabled;
    let selected_idx = state.selected_index;

    let mut tab_rects = Vec::with_capacity(CANONICAL_TAB_ITEMS.len());

    let line_r = Rect::new(total_r.origin.x, total_r.origin.y + strip_h, tab_w, 1.0);
    out.primitives
        .push(primitive::surface(line_r, 0.0, Token::CARD_BORDER, z + 1));

    let mut cur_x = total_r.origin.x + 4.0;
    for (idx, &title) in CANONICAL_TAB_ITEMS.iter().enumerate() {
        let is_selected = idx == selected_idx;
        let t_rect = Rect::new(cur_x, total_r.origin.y + 12.0, tab_item_w, strip_h - 11.0);
        tab_rects.push(t_rect);

        if is_selected {
            out.primitives
                .push(primitive::surface(t_rect, 7.0, Token::SURFACE, z + 2));
            out.primitives.push(primitive::outlined(
                t_rect,
                7.0,
                Token::NONE,
                Token::CARD_BORDER,
                1.0,
                0.0,
                z + 2,
            ));
        }

        if is_selected && state.focused && !disabled {
            out.primitives.push(primitive::focus_ring(
                t_rect.inflate(1.0),
                7.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }

        let text_ink = if disabled {
            Token::TEXT_DISABLED
        } else if is_selected {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        };

        let label_r = Rect::new(
            t_rect.origin.x + 12.0,
            t_rect.origin.y + 1.0,
            t_rect.width() - 36.0,
            t_rect.height(),
        );
        let weight = if is_selected { 600 } else { 400 };
        let quads = text_ctx.shape_line_styled(
            title,
            label_r,
            12.0,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );

        let close_ink = if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        };
        let close_r = Rect::new(
            t_rect.max().x - 26.0,
            t_rect.origin.y,
            16.0,
            t_rect.height(),
        );
        let close_quads = text_ctx.shape_line_styled(
            CANONICAL_TAB_CLOSE_GLYPH,
            close_r,
            10.0,
            400,
            Some("Segoe Fluent Icons"),
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &close_quads,
            close_ink,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_x += tab_item_w;
    }

    let content_r = Rect::new(
        total_r.origin.x,
        total_r.origin.y + strip_h,
        tab_w,
        (tab_h - strip_h).max(1.0),
    );
    // 参考截图的内容区没有独立底板，页面背景直接透出。

    let content_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let content_label_r = Rect::new(
        content_r.origin.x,
        content_r.origin.y + 1.0,
        content_r.width(),
        20.0,
    );
    let content_quads = text_ctx.shape_line_styled(
        CANONICAL_TAB_CONTENT,
        content_label_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &content_quads,
        content_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    TabViewFixtureOutput {
        output: out,
        tab_rects,
        content_rect: content_r,
        total_size: Vec2::new(tab_w, tab_h),
    }
}

/// 发射标准化 2D 网格单元插桩（`Item 1` 到 `Item 4`，2×2 网格）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 260px、高 140px；
/// - 外围绘制圆角 8px 的控件背景卡片；
/// - 内部等分为 2 列 × 2 行单元格，单元内边距 6px、单元间距 6px，单元圆角 6px；
/// - 首项处于选中态（`state.selected_index == 0`）：
///   - 未禁用时：绘制 [`Token::PRIMARY_SOFT`] 填充、[`Token::PRIMARY`] 2.0px 高亮边框与右上角勾选徽章；
///   - 禁用时：降级为 [`Token::CONTROL_DISABLED_BG`] 填充与 [`Token::CONTROL_DISABLED_BORDER`] 边框，抑制高亮边框与徽章；
/// - 其他未选中项：
///   - 未禁用时：绘制 [`Token::SURFACE_SUNKEN`] 填充与 [`Token::CONTROL_STROKE`] 1.0px 边框；
///   - 禁用时：绘制 [`Token::CONTROL_DISABLED_BG`] 填充与 [`Token::CONTROL_DISABLED_BORDER`] 边框；
/// - 单元文本：字号 14px，居中排版。选中项字重 600、未禁用时为 [`Token::TEXT_PRIMARY`]；禁用时统一为字重 400 与 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_grid_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> GridFixtureOutput {
    let mut out = WidgetOutput::default();
    let grid_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_GRID_WIDTH
    };
    let grid_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_GRID_HEIGHT
    };

    // WinUI GridView 默认使用 `ItemsWrapGrid`：条目按内容尺寸从左到右流式排布，
    // 没有外层卡片、也没有整块单元格底色。参考截图中只有选中项带 2px 强调色
    // 圆角描边，其余条目只显示文本。
    let cell_w = 42.0;
    let cell_h = 42.0;
    let gap = 8.0;
    let per_row = ((grid_w + gap) / (cell_w + gap)).floor().max(1.0) as usize;

    let mut cell_rects = Vec::with_capacity(CANONICAL_GRID_ITEMS.len());

    for (idx, &text) in CANONICAL_GRID_ITEMS.iter().enumerate() {
        let col = idx % per_row;
        let row = idx / per_row;
        let x = rect.origin.x + col as f32 * (cell_w + gap);
        let y = rect.origin.y + 3.0 + row as f32 * (cell_h + gap);
        let cell_r = Rect::new(x, y, cell_w, cell_h);
        cell_rects.push(cell_r);

        let is_selected = idx == state.selected_index;
        if is_selected {
            out.primitives.push(primitive::outlined(
                Rect::new(
                    cell_r.origin.x,
                    cell_r.origin.y,
                    cell_r.width() + 2.0,
                    cell_r.height() + 2.0,
                ),
                4.0,
                Token::HOVER,
                if state.disabled {
                    Token::CONTROL_DISABLED_THUMB
                } else {
                    Token::PRIMARY
                },
                2.0,
                0.0,
                z + 2,
            ));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    cell_r.inflate(1.0),
                    5.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 8,
                    true,
                ));
            }
        }

        let text_ink = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        let quads = text_ctx.shape_line_styled(
            text,
            cell_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    GridFixtureOutput {
        output: out,
        cell_rects,
        total_size: Vec2::new(grid_w, grid_h),
    }
}

/// 发射标准化翻页视图插桩（内容卡片 `Page 1` 与翻页导航前后按钮）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 220px、高 120px；
/// - 外框为圆角 4px 的内容卡片背景（[`Token::CARD_BG`]、边框 [`Token::CARD_BORDER`]）；
/// - 中央水平与垂直居中排版当前页文本 `"Page 1"`（未禁用时字重 400、颜色 [`Token::TEXT_PRIMARY`]）；
/// - 左侧垂直居中放置上一页导航按钮（28×28px，向左 chevron 折线，因处于首页故呈次级半透明或禁用态）；
/// - 右侧垂直居中放置下一页导航按钮（28×28px，向右 chevron 折线，未禁用时为活跃色 [`Token::TEXT_PRIMARY`]）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，卡片背景、边框、页文本与全部导航箭头均降级为变灰与衰减令牌。
pub fn emit_canonical_flip_view_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> FlipViewFixtureOutput {
    let mut out = WidgetOutput::default();
    let flip_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_FLIP_WIDTH
    };
    let flip_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_FLIP_HEIGHT
    };
    let card_r = Rect::new(rect.origin.x, rect.origin.y, flip_w, flip_h);

    // WinUI FlipView 静态无悬停时背景透明、不显示翻页按钮，当前页 TextBlock 左上对齐。
    if state.focused && !state.disabled {
        out.primitives.push(primitive::focus_ring(
            card_r.inflate(1.0),
            5.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    let btn_size = 28.0;
    let prev_x = card_r.origin.x + 8.0;
    let prev_y = card_r.origin.y + (flip_h - btn_size) * 0.5;
    let prev_btn_r = Rect::new(prev_x, prev_y, btn_size, btn_size);

    let next_x = card_r.max().x - 8.0 - btn_size;
    let next_y = prev_y;
    let next_btn_r = Rect::new(next_x, next_y, btn_size, btn_size);

    let current_page = CANONICAL_FLIP_PAGES[0];
    let page_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let page_r = Rect::new(card_r.origin.x, card_r.origin.y, flip_w, 20.0);
    let quads = text_ctx.shape_line_styled(
        current_page,
        page_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Top,
    );
    primitive::glyphs(
        &quads,
        page_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    FlipViewFixtureOutput {
        output: out,
        card_rect: card_r,
        prev_button_rect: prev_btn_r,
        next_button_rect: next_btn_r,
        total_size: Vec2::new(flip_w, flip_h),
    }
}

/// 发射标准化下拉刷新容器插桩（顶部环形指示器 + 下方内容卡片）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 260px、高 160px；
/// - 顶部包含 24px 直径的 ProgressRing 环形指示器（圆心位于水平居中、距顶 16px 处）：
///   - 未禁用时：绘制环形底轨、内部掏空与右上四分之一强调色圆弧（[`Token::PRIMARY`]，线宽 2.5px）；
///   - 禁用时：抑制强调色圆弧，底轨降级为变灰底色；
/// - 下方为内容卡片（宽 252px、高 120px，圆角 8px）：
///   - 位于 y = 36px 处，背景为 [`Token::CARD_BG`]、边框为 [`Token::CARD_BORDER`]；
///   - 内部左内边距 16px、垂直居中排版文本 `"Pull to refresh"`（字号 14px，字重 400）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，指示器与卡片内容全量半透明变灰衰减。
pub fn emit_canonical_refresh_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> RefreshFixtureOutput {
    let mut out = WidgetOutput::default();
    let refresh_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_REFRESH_WIDTH
    };
    let refresh_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_REFRESH_HEIGHT
    };
    let refresh_rect = Rect::new(rect.origin.x, rect.origin.y, refresh_w, refresh_h);

    // WinUI RefreshContainer 静态无卡片底板：左上显示内容文本，上方居中绘制浅灰刷新图标。
    let ring_size = STANDARD_REFRESH_INDICATOR_SIZE;
    let ring_cx = refresh_rect.origin.x + refresh_w * 0.5;
    let ring_cy = refresh_rect.origin.y + 46.0;
    let ring_rect = Rect::new(
        ring_cx - ring_size * 0.5,
        ring_cy - ring_size * 0.5,
        ring_size,
        ring_size,
    );
    let card_rect = refresh_rect;

    let refresh_icon_ink = Token::TEXT_DISABLED;
    let icon_quads = text_ctx.shape_line_styled(
        "\u{E72C}",
        ring_rect,
        20.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &icon_quads,
        refresh_icon_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if state.focused && !state.disabled {
        out.primitives.push(primitive::focus_ring(
            refresh_rect.inflate(1.0),
            5.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    let ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let label_r = Rect::new(
        refresh_rect.origin.x,
        refresh_rect.origin.y,
        refresh_w,
        20.0,
    );
    let quads = text_ctx.shape_line_styled(
        CANONICAL_REFRESH_TEXT,
        label_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Top,
    );
    primitive::glyphs(&quads, ink, z + 4, &mut out.primitives, &mut out.payloads);

    RefreshFixtureOutput {
        output: out,
        indicator_rect: ring_rect,
        card_rect,
        total_size: Vec2::new(refresh_w, refresh_h),
    }
}

/// 发射标准化标注滚动条插桩（8px 居中垂直轨道 + 32px 滑块 + 刻度线与标签）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 16px、高 160px；
/// - 轨道宽度 8px，水平居中于 16px 控件包围盒内（左右各留 4px 边距），圆角 4px；
/// - 滑块位于进度 0.4 处，高度 32px，宽度 8px，圆角 3px：
///   - 未禁用时：使用滑块色令牌 [`Token::CONTROL_TRACK_STROKE`]；
///   - 禁用时：降级为变灰滑块令牌 [`Token::CONTROL_DISABLED_THUMB`]；
/// - 沿轨道排布 5 个等分刻度线（高度 1px，线宽 4px，水平居中于轨道）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，轨道、滑块与全部刻度同步变灰。
pub fn emit_canonical_annotated_scroll_bar_fixture(
    rect: Rect,
    state: FixtureState,
    _text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> AnnotatedScrollBarFixtureOutput {
    let mut out = WidgetOutput::default();
    let bar_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_SCROLL_BAR_WIDTH
    };
    let bar_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_SCROLL_BAR_HEIGHT
    };

    // WinUI AnnotatedScrollBar 无垂直底轨，由顶部上三角、横向强调色指示条与底部下三角组成。
    let track_rect = Rect::new(rect.origin.x, rect.origin.y, bar_w, bar_h);
    let arrow_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let cx = rect.origin.x + bar_w * 0.5;

    // 1. 顶部上三角（y = origin.y + 6..10）
    for row in 0..4 {
        let w = (row * 2 + 2) as f32;
        out.primitives.push(primitive::surface(
            Rect::new(cx - w * 0.5, rect.origin.y + 6.0 + row as f32, w, 1.0),
            0.0,
            arrow_ink,
            z + 2,
        ));
    }

    // 2. 横向强调色滑块指示条（宽 30px、高 3px，位于 y = origin.y + 18）
    let thumb_w = (bar_w - 14.0).clamp(12.0, 30.0);
    let thumb_h = 3.0;
    let thumb_rect = Rect::new(cx - thumb_w * 0.5, rect.origin.y + 18.0, thumb_w, thumb_h);
    let thumb_token = if state.disabled {
        Token::CONTROL_DISABLED_THUMB
    } else {
        Token::PRIMARY
    };
    out.primitives
        .push(primitive::surface(thumb_rect, 1.5, thumb_token, z + 4));

    // 3. 底部下三角（y = max.y - 12..max.y - 8）
    for row in 0..4 {
        let w = ((3 - row) * 2 + 2) as f32;
        out.primitives.push(primitive::surface(
            Rect::new(
                cx - w * 0.5,
                rect.origin.y + bar_h - 12.0 + row as f32,
                w,
                1.0,
            ),
            0.0,
            arrow_ink,
            z + 2,
        ));
    }

    if state.focused && !state.disabled {
        out.primitives.push(primitive::focus_ring(
            thumb_rect.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 6,
            true,
        ));
    }

    AnnotatedScrollBarFixtureOutput {
        output: out,
        track_rect,
        thumb_rect,
        total_size: Vec2::new(bar_w, bar_h),
    }
}

/// 发射标准化 InfoBar 插桩。
pub fn emit_canonical_info_bar_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> InfoBarFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_INFOBAR_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_INFOBAR_HEIGHT
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;

    // 1. 底板与微描边
    out.primitives
        .push(primitive::surface(total_r, 4.0, Token::CARD_BG, z));
    out.primitives.push(primitive::outlined(
        total_r,
        4.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 2. 严重级别实心圆徽章图标（WinUI InfoBar 无左侧色条，使用 16px 实心强调圆 + 反色 i）
    let badge_fill = if disabled {
        Token::CONTROL_DISABLED_THUMB
    } else {
        Token::PRIMARY
    };
    let icon_r = Rect::new(
        total_r.origin.x + 16.0,
        total_r.origin.y + (h - 16.0) * 0.5,
        16.0,
        16.0,
    );
    out.primitives
        .push(primitive::surface(icon_r, 8.0, badge_fill, z + 1));
    let badge_ink = if disabled {
        Token::SURFACE
    } else {
        Token::ON_PRIMARY
    };
    let info_quads =
        text_ctx.shape_line_styled("i", icon_r, 11.0, 600, None, AlignX::Center, AlignY::Middle);
    primitive::glyphs(
        &info_quads,
        badge_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. Title 标题与 Message 消息文本（WinUI Informational 两者均为 TEXT_PRIMARY）
    let title_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let title_r = Rect::new(total_r.origin.x + 44.0, total_r.origin.y, 56.0, h);
    let title_quads = text_ctx.shape_line_styled(
        CANONICAL_INFOBAR_TITLE,
        title_r,
        STANDARD_FONT_SIZE,
        600,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &title_quads,
        title_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    let msg_r = Rect::new(
        total_r.origin.x + 102.0,
        total_r.origin.y,
        (total_r.width() - 142.0).max(10.0),
        h,
    );
    let msg_quads = text_ctx.shape_line_styled(
        CANONICAL_INFOBAR_MESSAGE,
        msg_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &msg_quads,
        title_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 4. 关闭按钮（Segoe Fluent Icons）
    let close_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let close_r = Rect::new(
        total_r.max().x - 32.0,
        total_r.origin.y + (h - 16.0) * 0.5,
        16.0,
        16.0,
    );
    let close_quads = text_ctx.shape_line_styled(
        CANONICAL_INFOBAR_CLOSE_GLYPH,
        close_r,
        12.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &close_quads,
        close_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 5. 焦点环
    if state.focused && !disabled {
        out.primitives.push(primitive::focus_ring(
            total_r.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    InfoBarFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 发射标准化 Expander 折叠展开面板插桩。
pub fn emit_canonical_expander_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> ExpanderFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_EXPANDER_WIDTH
    };
    let header_h = CANONICAL_EXPANDER_HEADER_HEIGHT;
    let content_h = CANONICAL_EXPANDER_CONTENT_HEIGHT;
    let total_h = header_h + content_h;
    let disabled = state.disabled;

    let header_r = Rect::new(rect.origin.x, rect.origin.y, w, header_h);
    let content_r = Rect::new(rect.origin.x, rect.origin.y + header_h, w, content_h);

    // 1. 头部背景与描边
    out.primitives
        .push(primitive::surface(header_r, 4.0, Token::CARD_BG, z));
    out.primitives.push(primitive::outlined(
        header_r,
        4.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 2. 头部文本 "Expander"
    let header_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let header_text_r = Rect::new(
        header_r.origin.x + 16.0,
        header_r.origin.y,
        w - 64.0,
        header_h,
    );
    let header_quads = text_ctx.shape_line_styled(
        CANONICAL_EXPANDER_HEADER,
        header_text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &header_quads,
        header_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. 头部展开折叠指示图标（向上箭头）
    let chevron_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    let chevron_r = Rect::new(
        header_r.max().x - 32.0,
        header_r.origin.y + (header_h - 16.0) * 0.5,
        16.0,
        16.0,
    );
    let chevron_quads = text_ctx.shape_line_styled(
        CANONICAL_EXPANDER_CHEVRON_UP,
        chevron_r,
        10.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &chevron_quads,
        chevron_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 4. 分隔边框线
    out.primitives.push(primitive::surface(
        Rect::new(header_r.origin.x, header_r.max().y - 1.0, w, 1.0),
        0.0,
        Token::CARD_BORDER,
        z + 1,
    ));

    // 5. 内容区背景与描边
    out.primitives
        .push(primitive::surface(content_r, 4.0, Token::CARD_BG, z));
    out.primitives.push(primitive::outlined(
        content_r,
        4.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 6. 内容区正文 "Body"
    let body_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let body_text_r = Rect::new(
        content_r.origin.x + 16.0,
        content_r.origin.y,
        w - 32.0,
        content_h,
    );
    let body_quads = text_ctx.shape_line_styled(
        CANONICAL_EXPANDER_BODY,
        body_text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &body_quads,
        body_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if state.focused && !disabled {
        out.primitives.push(primitive::focus_ring(
            header_r.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    ExpanderFixtureOutput {
        output: out,
        total_size: Vec2::new(w, total_h),
        header_rect: header_r,
        content_rect: content_r,
    }
}

/// 发射标准化 SplitView 分割视图插桩。
pub fn emit_canonical_split_view_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> SplitViewFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_SPLIT_VIEW_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_SPLIT_VIEW_HEIGHT
    };
    let pane_w = CANONICAL_SPLIT_VIEW_PANE_WIDTH;
    let disabled = state.disabled;

    let pane_r = Rect::new(rect.origin.x, rect.origin.y, pane_w, h);
    let content_r = Rect::new(
        rect.origin.x + pane_w,
        rect.origin.y,
        (w - pane_w).max(1.0),
        h,
    );

    // 1. 窗格底板（WinUI CompactInline SplitView 默认使用 CARD_BG，无硬分隔线）
    out.primitives
        .push(primitive::surface(pane_r, 0.0, Token::CARD_BG, z));

    // 2. 窗格文字 "Pane"（左上对齐）
    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let pane_text_r = Rect::new(pane_r.origin.x, pane_r.origin.y, pane_w, 20.0);
    let pane_quads = text_ctx.shape_line_styled(
        CANONICAL_SPLIT_VIEW_PANE_TEXT,
        pane_text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Top,
    );
    primitive::glyphs(
        &pane_quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. 内容区文字 "Content"（左上对齐）
    let content_text_r = Rect::new(
        content_r.origin.x,
        content_r.origin.y,
        content_r.width(),
        20.0,
    );
    let content_quads = text_ctx.shape_line_styled(
        CANONICAL_SPLIT_VIEW_CONTENT_TEXT,
        content_text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Top,
    );
    primitive::glyphs(
        &content_quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    SplitViewFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        pane_rect: pane_r,
        content_rect: content_r,
    }
}

/// 发射标准化 CalendarView 日历视图插桩。
pub fn emit_canonical_calendar_view_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> CalendarViewFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_CALENDAR_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_CALENDAR_HEIGHT
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;

    // 1. 外围表面卡片与描边（WinUI CalendarView 浅色为纯白 #FFFFFF，深色为 #2B2B2B）
    out.primitives
        .push(primitive::surface(total_r, 4.0, Token::SURFACE_RAISED, z));
    out.primitives.push(primitive::surface(
        Rect::new(total_r.origin.x, total_r.origin.y + 40.0, w, 1.0),
        0.0,
        Token::CARD_BORDER,
        z + 1,
    ));
    out.primitives.push(primitive::outlined(
        total_r,
        4.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 2. 顶部标题栏 "January 2026"
    let title_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let title_r = Rect::new(
        total_r.origin.x + 16.0,
        total_r.origin.y + 10.0,
        160.0,
        24.0,
    );
    let title_quads = text_ctx.shape_line_styled(
        CANONICAL_CALENDAR_TITLE,
        title_r,
        STANDARD_FONT_SIZE,
        600,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &title_quads,
        title_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 导航箭头
    let nav_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    let up_r = Rect::new(total_r.max().x - 48.0, total_r.origin.y + 12.0, 16.0, 16.0);
    let up_quads = text_ctx.shape_line_styled(
        "\u{EDDB}",
        up_r,
        10.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &up_quads,
        nav_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    let down_r = Rect::new(total_r.max().x - 24.0, total_r.origin.y + 12.0, 16.0, 16.0);
    let down_quads = text_ctx.shape_line_styled(
        "\u{EDDC}",
        down_r,
        10.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &down_quads,
        nav_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. 星期表头
    let col_w = (w - 16.0) / 7.0;
    let week_y = total_r.origin.y + 40.0;
    for (col, &day_name) in CANONICAL_CALENDAR_DAYS_HEADER.iter().enumerate() {
        let cell_r = Rect::new(
            total_r.origin.x + 8.0 + col as f32 * col_w,
            week_y,
            col_w,
            20.0,
        );
        let cell_quads = text_ctx.shape_line_styled(
            day_name,
            cell_r,
            12.0,
            400,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &cell_quads,
            nav_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 4. 日期网格（1 到 31 日，固定排布，15 日为选中态）
    let grid_y = total_r.origin.y + 64.0;
    let row_h = 28.0;
    let day_strings: [String; 31] = std::array::from_fn(|i| (i + 1).to_string());
    for day in 1..=31u8 {
        let idx = (day + 3) as usize; // 起始偏移 4（从周四开始）
        let col = idx % 7;
        let row = idx / 7;
        let cell_r = Rect::new(
            total_r.origin.x + 8.0 + col as f32 * col_w,
            grid_y + row as f32 * row_h,
            col_w,
            row_h,
        );

        let is_selected = !disabled && (day == CANONICAL_CALENDAR_SELECTED_DAY);
        if is_selected {
            let pill_size = 34.0;
            let pill_r = Rect::new(
                cell_r.origin.x + (cell_r.width() - pill_size) * 0.5,
                cell_r.origin.y + (cell_r.height() - pill_size) * 0.5,
                pill_size,
                pill_size,
            );
            out.primitives.push(primitive::surface(
                pill_r,
                pill_size * 0.5,
                Token::PRIMARY,
                z + 3,
            ));
        }

        let day_ink = if disabled {
            Token::TEXT_DISABLED
        } else if is_selected {
            Token::ON_PRIMARY
        } else {
            Token::TEXT_PRIMARY
        };
        let day_weight = if is_selected { 600 } else { 400 };
        let day_quads = text_ctx.shape_line_styled(
            &day_strings[(day - 1) as usize],
            cell_r,
            12.0,
            day_weight,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &day_quads,
            day_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    CalendarViewFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 发射标准化 TimePicker 时间选择器插桩。
pub fn emit_canonical_time_picker_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TimePickerFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_TIME_PICKER_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_TIME_PICKER_HEIGHT
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;

    // 1. 底板与微描边
    out.primitives
        .push(primitive::surface(total_r, 4.0, Token::CONTROL_FILL, z));
    out.primitives.push(primitive::outlined(
        total_r,
        4.0,
        Token::NONE,
        Token::CONTROL_STROKE,
        1.0,
        0.0,
        z,
    ));

    if !disabled && !state.focused {
        out.primitives.push(primitive::surface(
            Rect::new(
                total_r.origin.x + 3.0,
                total_r.max().y - 1.0,
                (w - 6.0).max(0.0),
                1.0,
            ),
            0.0,
            Token::CONTROL_STROKE_SECONDARY,
            z + 1,
        ));
    }

    // 2. WinUI TimePicker 24 小时制为 2 列分栏（时 | 分）
    let col_w = (w - 2.0) / CANONICAL_TIME_COLUMNS.len() as f32;
    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    for (col, &col_text) in CANONICAL_TIME_COLUMNS.iter().enumerate() {
        if col > 0 {
            out.primitives.push(primitive::surface(
                Rect::new(
                    total_r.origin.x + col as f32 * col_w,
                    total_r.origin.y + 4.0,
                    1.0,
                    h - 8.0,
                ),
                0.0,
                Token::DIVIDER,
                z + 1,
            ));
        }
        let col_r = Rect::new(
            total_r.origin.x + col as f32 * col_w,
            total_r.origin.y,
            col_w,
            h,
        );
        let col_quads = text_ctx.shape_line_styled(
            col_text,
            col_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &col_quads,
            text_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    if state.focused && !disabled {
        out.primitives.push(primitive::focus_ring(
            total_r.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    TimePickerFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 发射标准化 PersonPicture 人物头像插桩。
pub fn emit_canonical_person_picture_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> PersonPictureFixtureOutput {
    let mut out = WidgetOutput::default();
    let size = if rect.width() > 0.0 {
        rect.width().min(if rect.height() > 0.0 {
            rect.height()
        } else {
            rect.width()
        })
    } else {
        CANONICAL_PERSON_PICTURE_SIZE
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, size, size);
    let disabled = state.disabled;

    // 1. 圆形底板（对齐 WinUI PersonPicture 默认中性灰底）
    out.primitives
        .push(primitive::surface(total_r, size * 0.5, Token::OUTLINE, z));

    // 2. 居中双字母缩写 "AB"
    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let font_size = (size * 0.45).clamp(11.0, 24.0);
    let quads = text_ctx.shape_line_styled(
        CANONICAL_PERSON_PICTURE_INITIALS,
        total_r,
        font_size,
        600,
        None,
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    PersonPictureFixtureOutput {
        output: out,
        total_size: Vec2::new(size, size),
        rect: total_r,
    }
}

/// 发射标准化 FontIcon 字体图标插桩。
pub fn emit_canonical_font_icon_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> FontIconFixtureOutput {
    let mut out = WidgetOutput::default();
    let size = if rect.width() > 0.0 {
        rect.width().min(if rect.height() > 0.0 {
            rect.height()
        } else {
            rect.width()
        })
    } else {
        CANONICAL_FONT_ICON_SIZE
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, size, size);
    let disabled = state.disabled;

    // 居中绘制矢量字形
    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let quads = text_ctx.shape_line_styled(
        CANONICAL_FONT_ICON_GLYPH,
        total_r,
        CANONICAL_FONT_ICON_FONT_SIZE,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &quads,
        text_ink,
        z + 1,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    FontIconFixtureOutput {
        output: out,
        total_size: Vec2::new(size, size),
        rect: total_r,
    }
}

/// 发射标准化 Pivot 页签条插桩。
pub fn emit_canonical_pivot_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> PivotFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_PIVOT_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_PIVOT_HEIGHT
    };
    let header_h = 46.0;
    let disabled = state.disabled;

    let header_r = Rect::new(rect.origin.x, rect.origin.y, w, header_h);
    let content_r = Rect::new(
        rect.origin.x,
        rect.origin.y + header_h,
        w,
        (h - header_h).max(1.0),
    );

    // 参考截图的 Pivot 没有横贯全宽的分隔线，只有选中项下方的短指示条。

    // 2. 标签项排布（"One" 与 "Two"）
    let item_w = 44.0;
    let gap = 24.0;
    let mut cur_x = header_r.origin.x + 11.0;

    for (idx, &item_name) in CANONICAL_PIVOT_ITEMS.iter().enumerate() {
        let is_selected = !disabled && (idx == state.selected_index);
        let item_r = Rect::new(cur_x, header_r.origin.y + 2.0, item_w, header_h);

        let text_ink = if disabled {
            Token::TEXT_DISABLED
        } else if is_selected {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        };
        let weight = if is_selected { 600 } else { 400 };

        let quads = text_ctx.shape_line_styled(
            item_name,
            item_r,
            24.0,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        if is_selected {
            // 选中指示下划线（3px 厚度）
            let indicator_r =
                Rect::new(item_r.origin.x, header_r.max().y - 3.0, item_r.width(), 3.0);
            out.primitives
                .push(primitive::surface(indicator_r, 1.5, Token::PRIMARY, z + 5));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    item_r.inflate(1.0),
                    4.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 10,
                    true,
                ));
            }
        }

        cur_x += item_w + gap;
    }

    // 3. 内容区文本
    let content_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let content_text_r = Rect::new(
        content_r.origin.x + 12.0,
        content_r.origin.y,
        content_r.width() - 24.0,
        24.0,
    );
    let content_quads = text_ctx.shape_line_styled(
        CANONICAL_PIVOT_CONTENT,
        content_text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &content_quads,
        content_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    PivotFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        header_rect: header_r,
        content_rect: content_r,
    }
}

/// 发射标准化 Image 空图像占位盒插桩。
pub fn emit_canonical_image_fixture(
    rect: Rect,
    state: FixtureState,
    _text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> ImageFixtureOutput {
    let mut out = WidgetOutput::default();
    let size = if rect.width() > 0.0 && rect.height() > 0.0 {
        rect.size
    } else {
        CANONICAL_IMAGE_SIZE
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, size.x, size.y);
    let disabled = state.disabled;

    out.primitives
        .push(primitive::surface(total_r, 4.0, Token::SURFACE_SUNKEN, z));
    out.primitives.push(primitive::outlined(
        total_r,
        4.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    ImageFixtureOutput {
        output: out,
        total_size: size,
        rect: total_r,
    }
}

/// 发射标准化 Flyout 浮层呈现器插桩。
pub fn emit_canonical_flyout_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> FlyoutFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_FLYOUT_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_FLYOUT_HEIGHT
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;

    // 1. 12px 解析阴影（避免阴影外扩超出 WinUI 参考包围盒）
    out.primitives.push(primitive::instance(
        total_r,
        [8.0; 4],
        Token::NONE,
        Token::NONE,
        0.0,
        12.0,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));

    // 2. 浮层底板
    out.primitives
        .push(primitive::surface(total_r, 8.0, Token::OVERLAY_BG, z));

    // 3. 1px 微描边
    out.primitives.push(primitive::outlined(
        total_r,
        8.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 4. 内容文本 "Flyout"
    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let text_r = Rect::new(
        total_r.origin.x + 16.0,
        total_r.origin.y + 14.0,
        (w - 32.0).max(1.0),
        20.0,
    );
    let quads = text_ctx.shape_line_styled(
        CANONICAL_FLYOUT_TEXT,
        text_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Top,
    );
    primitive::glyphs(
        &quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    FlyoutFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 发射标准化 MenuFlyout 菜单浮层呈现器插桩。
pub fn emit_canonical_menu_flyout_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> MenuFlyoutFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_MENU_FLYOUT_WIDTH
    };
    let item_h = CANONICAL_MENU_FLYOUT_ITEM_HEIGHT;
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        item_h * 2.0 + 8.0
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;

    // 1. 12px 解析阴影（对齐 WinUI MenuFlyoutPresenter 参考包围盒）
    out.primitives.push(primitive::instance(
        total_r,
        [8.0; 4],
        Token::NONE,
        Token::NONE,
        0.0,
        12.0,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));

    // 2. 菜单表面底板
    out.primitives
        .push(primitive::surface(total_r, 8.0, Token::OVERLAY_BG, z));

    // 3. 1px 微描边
    out.primitives.push(primitive::outlined(
        total_r,
        8.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    // 4. 菜单项文本与选中背景
    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    for (idx, &item_name) in CANONICAL_MENU_FLYOUT_ITEMS.iter().enumerate() {
        let row_y = total_r.origin.y + 6.0 + idx as f32 * item_h;
        let row_r = Rect::new(total_r.origin.x + 4.0, row_y, w - 8.0, item_h);

        if idx == state.selected_index && state.checked && !disabled {
            out.primitives
                .push(primitive::surface(row_r, 4.0, Token::CONTROL_HOVER, z + 1));
        }

        let text_r = Rect::new(total_r.origin.x + 13.0, row_y, (w - 26.0).max(1.0), item_h);
        let quads = text_ctx.shape_line_styled(
            item_name,
            text_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    MenuFlyoutFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 发射标准化 AppBarButton 命令栏按钮插桩。
pub fn emit_canonical_app_bar_button_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> AppBarButtonFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_APP_BAR_BUTTON_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_APP_BAR_BUTTON_HEIGHT
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;

    // 1. 底板与焦点环
    out.primitives
        .push(primitive::surface(total_r, 4.0, Token::NONE, z));
    if state.focused && !disabled {
        out.primitives.push(primitive::focus_ring(
            total_r.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    // 2. 顶部图标
    //
    // WinUI 的 AppBarButton 把「16px 图标 + 2px 间隙 + 12px 标签」当成 30px 内容栈
    // 在控件高度内垂直居中；参考截图的图标顶端因此在 y=33（64px 高时为 17）。
    let stack_top = total_r.origin.y + ((h - 30.0) * 0.5).round();
    let icon_r = Rect::new(total_r.origin.x, stack_top - 3.0, w, 16.0);
    let icon_quads = text_ctx.shape_line_styled(
        CANONICAL_APP_BAR_BUTTON_GLYPH,
        icon_r,
        16.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &icon_quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. 底部文字
    let label_r = Rect::new(total_r.origin.x, stack_top + 18.0, w, 12.0);
    let label_quads = text_ctx.shape_line_styled(
        CANONICAL_APP_BAR_BUTTON_LABEL,
        label_r,
        12.0,
        400,
        None,
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &label_quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    AppBarButtonFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 发射标准化 AppBarToggleButton 命令栏切换按钮插桩。
pub fn emit_canonical_app_bar_toggle_button_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> AppBarToggleButtonFixtureOutput {
    let mut out = WidgetOutput::default();
    let w = if rect.width() > 0.0 {
        rect.width()
    } else {
        CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH
    };
    let h = if rect.height() > 0.0 {
        rect.height()
    } else {
        CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT
    };
    let total_r = Rect::new(rect.origin.x, rect.origin.y, w, h);
    let disabled = state.disabled;
    let checked = state.checked;

    // 1. 选中底板
    //
    // WinUI 的 AppBarToggleButton 选中态是整块强调色实底（左右内缩 2px、上下
    // 内缩 6px），文字与图标反色；不是底部指示条。
    let fill = if checked && !disabled {
        Token::PRIMARY
    } else {
        Token::NONE
    };
    let visual_r = if checked && !disabled {
        Rect::new(
            total_r.origin.x + 2.0,
            total_r.origin.y + 6.0,
            (w - 4.0).max(1.0),
            (h - 12.0).max(1.0),
        )
    } else {
        total_r
    };
    out.primitives
        .push(primitive::surface(visual_r, 4.0, fill, z));

    if state.focused && !disabled {
        out.primitives.push(primitive::focus_ring(
            total_r.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    let text_ink = if disabled {
        Token::TEXT_DISABLED
    } else if checked {
        Token::ON_PRIMARY
    } else {
        Token::TEXT_PRIMARY
    };

    // 2. 顶部图标（与 AppBarButton 相同的 30px 内容栈居中）
    let stack_top = total_r.origin.y + ((h - 30.0) * 0.5).round();
    let icon_r = Rect::new(total_r.origin.x, stack_top - 3.0, w, 16.0);
    let icon_quads = text_ctx.shape_line_styled(
        CANONICAL_APP_BAR_TOGGLE_BUTTON_GLYPH,
        icon_r,
        16.0,
        400,
        Some("Segoe Fluent Icons"),
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &icon_quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. 底部文字
    let label_r = Rect::new(total_r.origin.x, stack_top + 18.0, w, 12.0);
    let label_quads = text_ctx.shape_line_styled(
        CANONICAL_APP_BAR_TOGGLE_BUTTON_LABEL,
        label_r,
        12.0,
        400,
        None,
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &label_quads,
        text_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    if disabled {
        apply_recursive_disabled_propagation(&mut out, true);
    }

    AppBarToggleButtonFixtureOutput {
        output: out,
        total_size: Vec2::new(w, h),
        rect: total_r,
    }
}

/// 通用递归禁用态变灰与衰减映射器。
///
/// 针对颜色令牌执行语义衰减：
/// - 文本前景色统一映射为 [`Token::TEXT_DISABLED`]；
/// - 交互高亮与强调色映射为抑制态或 [`Token::CONTROL_DISABLED_THUMB`]；
/// - 悬停/选中背景映射为 [`Token::NONE`]；
/// - 边框映射为 [`Token::CONTROL_DISABLED_BORDER`]。
#[must_use]
pub fn remap_token_to_disabled(token: Token) -> Token {
    match token {
        Token::TEXT_PRIMARY
        | Token::TEXT_SECONDARY
        | Token::ON_SURFACE
        | Token::ON_PRIMARY
        | Token::ON_ACCENT
        | Token::ON_SELECTION => Token::TEXT_DISABLED,

        Token::PRIMARY
        | Token::ACCENT
        | Token::PRIMARY_HOVER
        | Token::PRIMARY_PRESSED
        | Token::CONTROL_TRACK_STROKE => Token::CONTROL_DISABLED_THUMB,

        Token::HOVER
        | Token::PRESSED
        | Token::NAV_SELECTED
        | Token::NAV_HOVER
        | Token::FOCUS_RING
        | Token::PRIMARY_SOFT => Token::NONE,

        Token::CONTROL_STROKE | Token::CARD_BORDER | Token::DIVIDER => {
            Token::CONTROL_DISABLED_BORDER
        }

        other => other,
    }
}

/// 通用递归禁用态图元后处理函数。
///
/// 遍历并重写 `output` 中所有图元的颜色令牌与不透明度：
/// 1. 将所有文本、图标、曲线与高亮图元的颜色令牌递归映射为变灰/衰减令牌；
/// 2. 抑制悬停、选中背景与焦点环；
/// 3. 若 `attenuate_opacity` 为 true，将所有图元的不透明度乘以 [`WINUI_DISABLED_OPACITY`]。
pub fn apply_recursive_disabled_propagation(output: &mut WidgetOutput, attenuate_opacity: bool) {
    for prim in &mut output.primitives {
        let prim_type = prim.prim_type();

        // 1. 字形文本与彩色字形处理
        if matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor) {
            let token = Token::new(prim.fill_token_or_uv as u16);
            let disabled_token = remap_token_to_disabled(token);
            prim.fill_token_or_uv = u32::from(disabled_token.raw());
        } else if prim_type == PrimType::Curve {
            // 2. 曲线描边（指示箭头、引导线等）
            let token = Token::new(prim.fill_token_or_uv as u16);
            let disabled_token = remap_token_to_disabled(token);
            prim.fill_token_or_uv = u32::from(disabled_token.raw());
        } else if matches!(prim_type, PrimType::RoundedRect | PrimType::Stroke) {
            // 3. 表面与边框处理
            let fill = Token::new(prim.fill_token_or_uv as u16);
            let border = Token::new(prim.border_token_or_tex as u16);

            let new_fill = remap_token_to_disabled(fill);
            let new_border = remap_token_to_disabled(border);

            prim.fill_token_or_uv = u32::from(new_fill.raw());
            prim.border_token_or_tex = u32::from(new_border.raw());
        }

        // 4. 不透明度衰减
        if attenuate_opacity {
            let cur_opacity = prim.opacity();
            let new_opacity = cur_opacity * WINUI_DISABLED_OPACITY;
            prim.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(prim.depth_u16(), new_opacity);
        }
    }
}

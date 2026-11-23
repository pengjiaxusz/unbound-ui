//! WinUI 3 规范名、目录分类与控件能力边界的唯一事实源。
//!
//! 目录枚举以 WinUI 3 官方规范名为主键（例如 `ToggleSwitch`、`ComboBox`），而差分系统、
//! 展台与设计规律巡检同样以官方名称寻址同一个控件。本模块承担四项编译期穷举职责：
//!
//! 1. [`WidgetCatalogId::winui3_name`] 返回该组件对应的官方规范名；
//! 2. [`WidgetCatalogId::from_winui3_name`] 按官方规范名反查目录标识；
//! 3. [`WidgetCatalogId::class`] 区分基准对齐控件、Community Toolkit 控件与
//!    自有独有控件，供批量差分与风格巡检筛选；
//! 4. [`WidgetCatalogId::capabilities`] 声明全部 133 个变体的 [`WidgetCapabilities`]
//!    （含 [`OverflowStrategy`]、文本截断、浮层贴边翻转与几何动画标记，遵循 ADR-0031）。
//!
//! 所有映射均为无通配符穷举 `match`，新增目录变体时若不在此处归类会直接触发 `E0004` 编译失败。

use crate::WidgetCatalogId;

/// 控件内容溢出时的官方处理策略（ADR-0031）。
///
/// 由 [`WidgetCatalogId::capabilities`] 在编译期穷举声明，并由运行期参数化契约测试
/// 自动遍历校验所有声明 `OverlayScrollbar*` 的组件。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum OverflowStrategy {
    /// 控件自身无内部滚动或溢出容器行为。
    None,
    /// 仅垂直方向溢出时渲染 Win11 按需叠加滚动条（复用 `emit_scrollbar`）。
    OverlayScrollbarVertical,
    /// 水平与垂直双轴溢出时渲染 Win11 按需叠加滚动条（如 `ScrollView`）。
    OverlayScrollbarBoth,
    /// 水平溢出走左右翻页箭头按钮，不绘制底边滚动条（如 `TabView`）。
    TabStripButtons,
    /// 溢出项收敛至弹出式溢出菜单（如 `CommandBar`、`MenuBar`）。
    MenuOverflowPopup,
    /// 自身不内置滚动容器，长内容由宿主外层 `ScrollView` 承载（如 `Form`）。
    ExternalScrollView,
}

impl OverflowStrategy {
    /// 该策略在内容溢出时是否渲染按需叠加滚动条（含单轴垂直与双轴）。
    #[must_use]
    pub const fn uses_overlay_scrollbar(self) -> bool {
        matches!(
            self,
            Self::OverlayScrollbarVertical | Self::OverlayScrollbarBoth
        )
    }

    /// 策略的稳定标识，用于测试诊断与合规报告。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OverlayScrollbarVertical => "overlay-scrollbar-vertical",
            Self::OverlayScrollbarBoth => "overlay-scrollbar-both",
            Self::TabStripButtons => "tab-strip-buttons",
            Self::MenuOverflowPopup => "menu-overflow-popup",
            Self::ExternalScrollView => "external-scroll-view",
        }
    }
}

/// 控件在能力边界（Widget Capability Boundary）上的正交维度声明。
///
/// 全部 [`WidgetCatalogId`] 变体均须通过无通配符的穷举 `match` 返回本结构体，
/// 确保未来新增控件不会在溢出滚动条、文本截断、贴边翻转或几何动画维度上静默漏检。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WidgetCapabilities {
    /// 内容溢出时的官方策略。
    pub overflow: OverflowStrategy,
    /// 文本标签或单元格在窄容器下是否需要截断省略或裁剪。
    pub text_elidable: bool,
    /// 浮层或弹出表面在贴近视口边缘时是否需要自动翻转或夹紧定位。
    pub overlay_flippable: bool,
    /// 是否包含驱动几何坐标或尺寸变化的过渡动画（须受中间帧几何不变量测试看守）。
    pub animates_geometry: bool,
}

impl WidgetCapabilities {
    /// 默认基础能力：无内部滚动、不自动截断、非贴边翻转浮层、无几何动画。
    pub const NONE: Self = Self {
        overflow: OverflowStrategy::None,
        text_elidable: false,
        overlay_flippable: false,
        animates_geometry: false,
    };

    /// 以指定溢出策略构造能力声明。
    #[must_use]
    pub const fn with_overflow(overflow: OverflowStrategy) -> Self {
        Self {
            overflow,
            ..Self::NONE
        }
    }

    /// 标记文本在窄容器下支持截断省略或裁剪。
    #[must_use]
    pub const fn elidable(mut self) -> Self {
        self.text_elidable = true;
        self
    }

    /// 标记浮层支持贴边自动翻转与视口夹紧。
    #[must_use]
    pub const fn flippable(mut self) -> Self {
        self.overlay_flippable = true;
        self
    }

    /// 标记控件包含几何动画（须通过中间帧几何不变量回归）。
    #[must_use]
    pub const fn animated_geometry(mut self) -> Self {
        self.animates_geometry = true;
        self
    }
}

/// 组件在三合一清单中的分类。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ControlClass {
    /// WinUI 3 官方存在的基准对齐控件，可进入一对一参考图差分。
    Baseline,
    /// Windows Community Toolkit 中的标准扩展控件。
    Toolkit,
    /// WinUI 3 不存在的 Unbound 自有独有控件，走 Fluent 规律巡检。
    Exclusive,
}

impl ControlClass {
    /// 该分类是否参与一对一参考图像素差分。
    #[must_use]
    pub const fn participates_in_parity(self) -> bool {
        matches!(self, Self::Baseline | Self::Toolkit)
    }

    /// 分类的稳定标识，用于报告与测试。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Toolkit => "toolkit",
            Self::Exclusive => "exclusive",
        }
    }
}

/// 基准对齐控件总数（含 Community Toolkit 之外的 WinUI 3 标准控件与变体）。
pub const BASELINE_COUNT: usize = 70;

/// Community Toolkit 对齐控件总数。
pub const TOOLKIT_COUNT: usize = 20;

/// Unbound 自有独有控件总数。
pub const EXCLUSIVE_COUNT: usize = 43;

impl WidgetCatalogId {
    /// 该组件的能力边界声明（编译期穷举，禁止通配符）。
    #[must_use]
    pub const fn capabilities(self) -> WidgetCapabilities {
        match self {
            // 1. 双轴按需叠加滚动条。
            Self::ScrollView => {
                WidgetCapabilities::with_overflow(OverflowStrategy::OverlayScrollbarBoth)
            }

            // 2. 垂直按需叠加滚动条（已统一接入 emit_scrollbar 的目录组件）。
            Self::NavigationView => {
                WidgetCapabilities::with_overflow(OverflowStrategy::OverlayScrollbarVertical)
                    .elidable()
                    .animated_geometry()
            }
            Self::TreeView | Self::Table | Self::GenericDataTable => {
                WidgetCapabilities::with_overflow(OverflowStrategy::OverlayScrollbarVertical)
                    .elidable()
            }
            Self::TextBoxMultiline => {
                WidgetCapabilities::with_overflow(OverflowStrategy::OverlayScrollbarVertical)
            }
            Self::MenuFlyout | Self::ContextMenu => {
                WidgetCapabilities::with_overflow(OverflowStrategy::OverlayScrollbarVertical)
                    .elidable()
                    .flippable()
            }

            // 3. 水平标签条翻页箭头按钮（显式排除底边滚动条）。
            Self::TabView => {
                WidgetCapabilities::with_overflow(OverflowStrategy::TabStripButtons).elidable()
            }

            // 4. 溢出项收敛至弹出式溢出菜单。
            Self::CommandBar | Self::MenuBar | Self::CommandBarFlyout => {
                WidgetCapabilities::with_overflow(OverflowStrategy::MenuOverflowPopup).flippable()
            }

            // 5. 无内置滚动容器，长内容由宿主外层 ScrollView 承载。
            Self::Form
            | Self::SchemaForm
            | Self::Descriptions
            | Self::Containers
            | Self::StackPanel
            | Self::Wrap
            | Self::Masonry
            | Self::VirtualMasonry
            | Self::RichEditBox
            | Self::CodeBlock
            | Self::CodeEditor
            | Self::TerminalView
            | Self::BladeView => {
                WidgetCapabilities::with_overflow(OverflowStrategy::ExternalScrollView)
            }
            Self::ExpanderGroup => {
                WidgetCapabilities::with_overflow(OverflowStrategy::ExternalScrollView)
                    .animated_geometry()
            }
            Self::DataGrid
            | Self::ListView
            | Self::GridView
            | Self::ListBox
            | Self::ItemsView
            | Self::TreeTable
            | Self::PropertyGrid
            | Self::NotificationCenter
            | Self::Timeline => {
                WidgetCapabilities::with_overflow(OverflowStrategy::ExternalScrollView).elidable()
            }

            // 6. 无内部滚动容器：带贴边翻转与几何动画的浮层。
            Self::Toast | Self::TeachingTip | Self::Tour => WidgetCapabilities::NONE
                .elidable()
                .flippable()
                .animated_geometry(),

            // 7. 无内部滚动容器：带贴边翻转与文本截断的弹出层/下拉触发器。
            Self::AutoSuggestBox
            | Self::CalendarDatePicker
            | Self::Callout
            | Self::Cascader
            | Self::ComboBox
            | Self::ComboBoxEditable
            | Self::CommandPalette
            | Self::DateRangePicker
            | Self::DropDownButton
            | Self::Flyout
            | Self::HoverCard
            | Self::Popconfirm
            | Self::SplitButton
            | Self::ToggleSplitButton
            | Self::ToolTip => WidgetCapabilities::NONE.elidable().flippable(),

            // 8. 无内部滚动容器：带贴边翻转的非文本浮层。
            Self::ColorPicker | Self::DatePicker | Self::Overlay | Self::TimePicker => {
                WidgetCapabilities::NONE.flippable()
            }

            // 9. 无内部滚动容器：带几何动画且文本可截断的控件。
            Self::Expander | Self::SplitView | Self::SwipeControl => {
                WidgetCapabilities::NONE.elidable().animated_geometry()
            }

            // 10. 无内部滚动容器：带几何动画的交互/指示控件。
            Self::FlipView
            | Self::LoadingOverlay
            | Self::ProgressBar
            | Self::ProgressRing
            | Self::RangeSelector
            | Self::RefreshContainer
            | Self::Skeleton
            | Self::Slider
            | Self::ToggleSwitch => WidgetCapabilities::NONE.animated_geometry(),

            // 11. 无内部滚动容器：文本可截断的标准内联/提示/输入控件。
            Self::AddressBar
            | Self::AlertDialog
            | Self::AppBarButton
            | Self::AppBarToggleButton
            | Self::Banner
            | Self::BreadcrumbBar
            | Self::Button
            | Self::CheckBox
            | Self::Chip
            | Self::ContentDialog
            | Self::HeaderedContentControl
            | Self::HyperlinkButton
            | Self::InfoBar
            | Self::PasswordBox
            | Self::Pivot
            | Self::RadioButton
            | Self::RepeatButton
            | Self::RichTextBlock
            | Self::SelectorBar
            | Self::ShortcutBar
            | Self::StatusBar
            | Self::Stepper
            | Self::TagInput
            | Self::TaskHud
            | Self::TextBox
            | Self::ToggleButton
            | Self::Transfer => WidgetCapabilities::NONE.elidable(),

            // 12. 其余固定尺寸、画布/图形/辅助类控件。
            Self::AnnotatedScrollBar
            | Self::AppBarSeparator
            | Self::AspectRatioBox
            | Self::AvatarGroup
            | Self::CalendarView
            | Self::CanvasToolbar
            | Self::Chart
            | Self::ColorSwatch
            | Self::CurveEditor
            | Self::Divider
            | Self::Dock
            | Self::EmptyState
            | Self::FileDrop
            | Self::FontIcon
            | Self::Gizmo
            | Self::GradientEditor
            | Self::HotkeyInput
            | Self::Image
            | Self::ImageCropper
            | Self::InfoBadge
            | Self::Minimap
            | Self::MultiSplit
            | Self::NinePatch
            | Self::NodeGraph
            | Self::NumberBox
            | Self::OrbitView
            | Self::PersonPicture
            | Self::PinInput
            | Self::PipelineView
            | Self::PipsPager
            | Self::QueryBuilder
            | Self::RadialGauge
            | Self::RatingControl
            | Self::Ruler
            | Self::SplitPane
            | Self::TimelineTrack
            | Self::Watermark => WidgetCapabilities::NONE,
        }
    }

    /// 迭代所有声明了按需叠加滚动条策略（单轴或双轴）的目录标识。
    pub fn ids_with_overlay_scrollbar() -> impl Iterator<Item = Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(|id| id.capabilities().overflow.uses_overlay_scrollbar())
    }

    /// 该组件在三合一清单中的分类。
    #[must_use]
    pub const fn class(self) -> ControlClass {
        match self {
            // Community Toolkit 对齐控件。
            Self::DataGrid
            | Self::Table
            | Self::GenericDataTable
            | Self::SplitPane
            | Self::MultiSplit
            | Self::TagInput
            | Self::Form
            | Self::Descriptions
            | Self::SchemaForm
            | Self::Wrap
            | Self::Masonry
            | Self::VirtualMasonry
            | Self::Skeleton
            | Self::ColorSwatch
            | Self::RangeSelector
            | Self::RadialGauge
            | Self::OrbitView
            | Self::BladeView
            | Self::HeaderedContentControl
            | Self::ImageCropper => ControlClass::Toolkit,

            // Unbound 自有独有控件。
            Self::NodeGraph
            | Self::CanvasToolbar
            | Self::Minimap
            | Self::Gizmo
            | Self::Ruler
            | Self::CurveEditor
            | Self::GradientEditor
            | Self::TimelineTrack
            | Self::CodeBlock
            | Self::CodeEditor
            | Self::TerminalView
            | Self::CommandPalette
            | Self::Dock
            | Self::StatusBar
            | Self::AddressBar
            | Self::Cascader
            | Self::DateRangePicker
            | Self::HotkeyInput
            | Self::PinInput
            | Self::ShortcutBar
            | Self::Stepper
            | Self::Transfer
            | Self::Chart
            | Self::PropertyGrid
            | Self::PipelineView
            | Self::QueryBuilder
            | Self::TaskHud
            | Self::TreeTable
            | Self::Timeline
            | Self::Chip
            | Self::Divider
            | Self::EmptyState
            | Self::FileDrop
            | Self::NotificationCenter
            | Self::Watermark
            | Self::AspectRatioBox
            | Self::NinePatch
            | Self::Overlay
            | Self::AvatarGroup
            | Self::Banner
            | Self::Callout
            | Self::Popconfirm
            | Self::Tour => ControlClass::Exclusive,

            // 其余全部为 WinUI 3 基准对齐控件。
            Self::ExpanderGroup
            | Self::InfoBar
            | Self::AlertDialog
            | Self::AutoSuggestBox
            | Self::PersonPicture
            | Self::InfoBadge
            | Self::BreadcrumbBar
            | Self::Button
            | Self::CalendarDatePicker
            | Self::FlipView
            | Self::CheckBox
            | Self::Expander
            | Self::ColorPicker
            | Self::ComboBoxEditable
            | Self::Containers
            | Self::ContextMenu
            | Self::CalendarView
            | Self::DatePicker
            | Self::SplitView
            | Self::DropDownButton
            | Self::HoverCard
            | Self::HyperlinkButton
            | Self::FontIcon
            | Self::Image
            | Self::LoadingOverlay
            | Self::MenuFlyout
            | Self::MenuBar
            | Self::ContentDialog
            | Self::NumberBox
            | Self::PipsPager
            | Self::PasswordBox
            | Self::Pivot
            | Self::Flyout
            | Self::ProgressBar
            | Self::RadioButton
            | Self::RatingControl
            | Self::RepeatButton
            | Self::RichTextBlock
            | Self::ScrollView
            | Self::SelectorBar
            | Self::ComboBox
            | Self::NavigationView
            | Self::Slider
            | Self::ProgressRing
            | Self::SplitButton
            | Self::StackPanel
            | Self::ToggleSwitch
            | Self::TabView
            | Self::TextBoxMultiline
            | Self::TextBox
            | Self::TimePicker
            | Self::Toast
            | Self::ToggleButton
            | Self::ToggleSplitButton
            | Self::CommandBar
            | Self::ToolTip
            | Self::TreeView
            | Self::ListView
            | Self::RichEditBox
            | Self::GridView
            | Self::ListBox
            | Self::CommandBarFlyout
            | Self::AppBarButton
            | Self::AppBarToggleButton
            | Self::AppBarSeparator
            | Self::SwipeControl
            | Self::RefreshContainer
            | Self::TeachingTip
            | Self::AnnotatedScrollBar
            | Self::ItemsView => ControlClass::Baseline,
        }
    }

    /// 该组件对应的 WinUI 3 官方规范名（自有独有控件返回自身显示名）。
    #[must_use]
    pub const fn winui3_name(self) -> &'static str {
        match self {
            Self::ExpanderGroup | Self::Expander => "Expander",
            Self::InfoBar => "InfoBar",
            Self::AlertDialog | Self::ContentDialog => "ContentDialog",
            Self::AutoSuggestBox => "AutoSuggestBox",
            Self::PersonPicture => "PersonPicture",
            Self::InfoBadge => "InfoBadge",
            Self::BreadcrumbBar => "BreadcrumbBar",
            Self::Button => "Button",
            Self::CalendarDatePicker => "CalendarDatePicker",
            Self::FlipView => "FlipView",
            Self::CheckBox => "CheckBox",
            Self::ColorPicker => "ColorPicker",
            Self::ComboBoxEditable | Self::ComboBox => "ComboBox",
            Self::Containers => "Border",
            Self::ContextMenu | Self::MenuFlyout => "MenuFlyout",
            Self::CalendarView => "CalendarView",
            Self::DatePicker => "DatePicker",
            Self::SplitView => "SplitView",
            Self::DropDownButton => "DropDownButton",
            Self::HoverCard | Self::Flyout => "Flyout",
            Self::HyperlinkButton => "HyperlinkButton",
            Self::FontIcon => "FontIcon",
            Self::Image => "Image",
            Self::LoadingOverlay | Self::ProgressRing => "ProgressRing",
            Self::MenuBar => "MenuBar",
            Self::NavigationView => "NavigationView",
            Self::NumberBox => "NumberBox",
            Self::PipsPager => "PipsPager",
            Self::PasswordBox => "PasswordBox",
            Self::Pivot => "Pivot",
            Self::ProgressBar => "ProgressBar",
            Self::RadioButton => "RadioButton",
            Self::RatingControl => "RatingControl",
            Self::RepeatButton => "RepeatButton",
            Self::RichTextBlock => "RichTextBlock",
            Self::ScrollView => "ScrollView",
            Self::SelectorBar => "SelectorBar",
            Self::Slider => "Slider",
            Self::SplitButton => "SplitButton",
            Self::StackPanel => "StackPanel",
            Self::ToggleSwitch => "ToggleSwitch",
            Self::TabView => "TabView",
            Self::TextBoxMultiline | Self::TextBox => "TextBox",
            Self::TimePicker => "TimePicker",
            Self::Toast => "TeachingTip",
            Self::ToggleButton => "ToggleButton",
            Self::ToggleSplitButton => "ToggleSplitButton",
            Self::CommandBar => "CommandBar",
            Self::ToolTip => "ToolTip",
            Self::TreeView => "TreeView",
            Self::ListView => "ListView",

            // Community Toolkit 规范名。
            Self::DataGrid | Self::GenericDataTable => "DataGrid",
            Self::Table => "TableView",
            Self::SplitPane | Self::MultiSplit => "GridSplitter",
            Self::TagInput => "TokenizingTextBox",
            Self::Form | Self::SchemaForm => "SettingsCard",
            Self::Descriptions => "SettingsExpander",
            Self::Wrap => "WrapPanel",
            Self::Masonry | Self::VirtualMasonry => "StaggeredPanel",
            Self::Skeleton => "Shimmer",
            Self::ColorSwatch => "ColorPalette",

            // 自有独有控件没有 WinUI 3 对应物，显示名保持自身标识。
            Self::AddressBar => "AddressBar",
            Self::AspectRatioBox => "AspectRatioBox",
            Self::AvatarGroup => "AvatarGroup",
            Self::Banner => "Banner",
            Self::Callout => "Callout",
            Self::CanvasToolbar => "CanvasToolbar",
            Self::Cascader => "Cascader",
            Self::Chart => "Chart",
            Self::Chip => "Chip",
            Self::CodeBlock => "CodeBlock",
            Self::CodeEditor => "CodeEditor",
            Self::CommandPalette => "CommandPalette",
            Self::CurveEditor => "CurveEditor",
            Self::DateRangePicker => "DateRangePicker",
            Self::Divider => "Divider",
            Self::Dock => "Dock",
            Self::EmptyState => "EmptyState",
            Self::FileDrop => "FileDrop",
            Self::Gizmo => "Gizmo",
            Self::GradientEditor => "GradientEditor",
            Self::HotkeyInput => "HotkeyInput",
            Self::Minimap => "Minimap",
            Self::NinePatch => "NinePatch",
            Self::NodeGraph => "NodeGraph",
            Self::NotificationCenter => "NotificationCenter",
            Self::Overlay => "Overlay",
            Self::PinInput => "PinInput",
            Self::PipelineView => "PipelineView",
            Self::Popconfirm => "Popconfirm",
            Self::PropertyGrid => "PropertyGrid",
            Self::QueryBuilder => "QueryBuilder",
            Self::Ruler => "Ruler",
            Self::ShortcutBar => "ShortcutBar",
            Self::StatusBar => "StatusBar",
            Self::Stepper => "Stepper",
            Self::TaskHud => "TaskHud",
            Self::TerminalView => "TerminalView",
            Self::Timeline => "Timeline",
            Self::TimelineTrack => "TimelineTrack",
            Self::Tour => "Tour",
            Self::Transfer => "Transfer",
            Self::TreeTable => "TreeTable",
            Self::Watermark => "Watermark",

            // P1：后续补齐的 WinUI 3 官方控件。
            Self::RichEditBox => "RichEditBox",
            Self::GridView => "GridView",
            Self::ListBox => "ListBox",
            Self::CommandBarFlyout => "CommandBarFlyout",
            Self::AppBarButton => "AppBarButton",
            Self::AppBarToggleButton => "AppBarToggleButton",
            Self::AppBarSeparator => "AppBarSeparator",
            Self::SwipeControl => "SwipeControl",
            Self::RefreshContainer => "RefreshContainer",
            Self::TeachingTip => "TeachingTip",
            Self::AnnotatedScrollBar => "AnnotatedScrollBar",
            Self::ItemsView => "ItemsView",

            // P2：Community Toolkit 选补控件。
            Self::RangeSelector => "RangeSelector",
            Self::RadialGauge => "RadialGauge",
            Self::OrbitView => "OrbitView",
            Self::BladeView => "BladeView",
            Self::HeaderedContentControl => "HeaderedContentControl",
            Self::ImageCropper => "ImageCropper",
        }
    }

    /// 按 WinUI 3 官方规范名（或自有控件显示名）反查目录标识。
    ///
    /// 同一规范名对应多个变体时返回一对一差分使用的主变体：
    /// `Expander → Expander`、`ContentDialog → ContentDialog`、`MenuFlyout → MenuFlyout`、
    /// `ComboBox → ComboBox`、`NavigationView → NavigationView`、`Flyout → Flyout`、
    /// `ProgressRing → ProgressRing`、`TextBox → TextBox`、`StackPanel → StackPanel`。
    #[must_use]
    pub fn from_winui3_name(name: &str) -> Option<Self> {
        let mut found = None;
        for &id in Self::ALL {
            if id.winui3_name().eq_ignore_ascii_case(name) {
                if id.is_primary_canonical() {
                    return Some(id);
                }
                if found.is_none() {
                    found = Some(id);
                }
            }
        }
        found
    }

    /// 该变体是否是同名规范名的主变体（一对一差分的默认落点）。
    #[must_use]
    pub const fn is_primary_canonical(self) -> bool {
        matches!(
            self,
            Self::Expander
                | Self::ContentDialog
                | Self::MenuFlyout
                | Self::ComboBox
                | Self::NavigationView
                | Self::Flyout
                | Self::ProgressRing
                | Self::TextBox
                | Self::StackPanel
                | Self::DataGrid
                | Self::SplitPane
                | Self::Form
                | Self::Masonry
                | Self::TeachingTip
        )
    }

    /// 是否属于 WinUI 3 基准对齐控件。
    #[must_use]
    pub const fn is_baseline_aligned(self) -> bool {
        matches!(self.class(), ControlClass::Baseline)
    }

    /// 是否属于 Unbound 自有独有控件。
    #[must_use]
    pub const fn is_exclusive(self) -> bool {
        matches!(self.class(), ControlClass::Exclusive)
    }

    /// 按分类筛选目录标识。
    pub fn ids_in_class(class: ControlClass) -> impl Iterator<Item = Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(move |id| id.class() == class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_variant_has_a_stable_display_name() {
        for &id in WidgetCatalogId::ALL {
            assert!(
                !id.winui3_name().is_empty(),
                "{} must expose a canonical name",
                id.module_name()
            );
        }
    }

    #[test]
    fn the_three_way_inventory_totals_match_the_spec() {
        let baseline = WidgetCatalogId::ids_in_class(ControlClass::Baseline).count();
        let toolkit = WidgetCatalogId::ids_in_class(ControlClass::Toolkit).count();
        let exclusive = WidgetCatalogId::ids_in_class(ControlClass::Exclusive).count();
        assert_eq!(baseline, BASELINE_COUNT);
        assert_eq!(toolkit, TOOLKIT_COUNT);
        assert_eq!(exclusive, EXCLUSIVE_COUNT);
        assert_eq!(baseline + toolkit + exclusive, WidgetCatalogId::COUNT);
    }

    #[test]
    fn canonical_names_resolve_back_to_a_catalog_id() {
        for &id in WidgetCatalogId::ALL {
            let resolved = WidgetCatalogId::from_winui3_name(id.winui3_name())
                .expect("every canonical name must resolve");
            assert_eq!(resolved.winui3_name(), id.winui3_name());
        }
    }

    #[test]
    fn renamed_controls_use_the_official_winui3_spelling() {
        assert_eq!(WidgetCatalogId::ToggleSwitch.winui3_name(), "ToggleSwitch");
        assert_eq!(WidgetCatalogId::CheckBox.winui3_name(), "CheckBox");
        assert_eq!(WidgetCatalogId::ComboBox.winui3_name(), "ComboBox");
        assert_eq!(WidgetCatalogId::TextBox.winui3_name(), "TextBox");
        assert_eq!(
            WidgetCatalogId::from_winui3_name("ToggleSwitch"),
            Some(WidgetCatalogId::ToggleSwitch)
        );
        assert_eq!(
            WidgetCatalogId::from_winui3_name("combobox"),
            Some(WidgetCatalogId::ComboBox)
        );
    }

    #[test]
    fn exclusive_controls_never_enter_one_to_one_parity() {
        for id in WidgetCatalogId::ids_in_class(ControlClass::Exclusive) {
            assert!(!id.class().participates_in_parity());
        }
    }

    #[test]
    fn capability_matrix_covers_all_133_variants_and_adr_0031_contracts() {
        assert_eq!(WidgetCatalogId::COUNT, 133);

        assert_eq!(
            WidgetCatalogId::ScrollView.capabilities().overflow,
            OverflowStrategy::OverlayScrollbarBoth
        );
        for id in [
            WidgetCatalogId::NavigationView,
            WidgetCatalogId::TreeView,
            WidgetCatalogId::Table,
            WidgetCatalogId::GenericDataTable,
            WidgetCatalogId::TextBoxMultiline,
            WidgetCatalogId::MenuFlyout,
            WidgetCatalogId::ContextMenu,
        ] {
            assert_eq!(
                id.capabilities().overflow,
                OverflowStrategy::OverlayScrollbarVertical,
                "{id:?} must declare OverlayScrollbarVertical"
            );
        }

        assert_eq!(
            WidgetCatalogId::TabView.capabilities().overflow,
            OverflowStrategy::TabStripButtons
        );
        assert!(!WidgetCatalogId::TabView
            .capabilities()
            .overflow
            .uses_overlay_scrollbar());
        assert_eq!(
            WidgetCatalogId::Form.capabilities().overflow,
            OverflowStrategy::ExternalScrollView
        );
        assert_eq!(
            WidgetCatalogId::SchemaForm.capabilities().overflow,
            OverflowStrategy::ExternalScrollView
        );
        assert_eq!(
            WidgetCatalogId::CommandBar.capabilities().overflow,
            OverflowStrategy::MenuOverflowPopup
        );
        assert!(
            WidgetCatalogId::NavigationView
                .capabilities()
                .animates_geometry
        );

        let overlay_ids: Vec<WidgetCatalogId> =
            WidgetCatalogId::ids_with_overlay_scrollbar().collect();
        assert_eq!(overlay_ids.len(), 8);
    }
}

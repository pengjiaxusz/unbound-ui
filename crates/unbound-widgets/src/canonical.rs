//! WinUI 3 规范名与目录标识之间的双向映射。
//!
//! 目录枚举以源码模块名为主键（例如 `Switch`、`Select`），而差分系统、展台与
//! 设计规律巡检需要以 WinUI 3 官方名称（例如 `ToggleSwitch`、`ComboBox`）寻址
//! 同一个控件。本模块是两者之间的唯一事实源：
//!
//! 1. [`WidgetCatalogId::winui3_name`] 返回该组件对应的官方规范名；
//! 2. [`WidgetCatalogId::from_winui3_name`] 按官方规范名反查目录标识；
//! 3. [`WidgetCatalogId::class`] 区分基准对齐控件、Community Toolkit 控件与
//!    自有独有控件，供批量差分与风格巡检筛选。
//!
//! 映射是穷举 `match`，新增目录变体时若不在此处归类会直接编译失败。

use crate::WidgetCatalogId;

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
pub const BASELINE_COUNT: usize = 71;

/// Community Toolkit 对齐控件总数。
pub const TOOLKIT_COUNT: usize = 20;

/// Unbound 自有独有控件总数。
pub const EXCLUSIVE_COUNT: usize = 43;

impl WidgetCatalogId {
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
            Self::Accordion
            | Self::Alert
            | Self::AlertDialog
            | Self::AutoComplete
            | Self::Avatar
            | Self::Badge
            | Self::Breadcrumb
            | Self::Button
            | Self::CalendarDatePicker
            | Self::Carousel
            | Self::Checkbox
            | Self::Collapsible
            | Self::ColorPicker
            | Self::Combobox
            | Self::Containers
            | Self::ContextMenu
            | Self::DatePicker
            | Self::DatePickerWheel
            | Self::Drawer
            | Self::DropDownButton
            | Self::HoverCard
            | Self::HyperlinkButton
            | Self::Icon
            | Self::Image
            | Self::LoadingOverlay
            | Self::Menu
            | Self::Menubar
            | Self::Modal
            | Self::NavigationRail
            | Self::NumberInput
            | Self::Pagination
            | Self::PasswordBox
            | Self::Pivot
            | Self::Popover
            | Self::Progress
            | Self::Radio
            | Self::Rating
            | Self::RepeatButton
            | Self::RichText
            | Self::Scroll
            | Self::Segmented
            | Self::Select
            | Self::Sidebar
            | Self::Slider
            | Self::Spinner
            | Self::SplitButton
            | Self::Stack
            | Self::Switch
            | Self::Tabs
            | Self::TextArea
            | Self::TextInput
            | Self::TimePicker
            | Self::Toast
            | Self::ToggleButton
            | Self::ToggleSplitButton
            | Self::Toolbar
            | Self::Tooltip
            | Self::TreeView
            | Self::VirtualViews
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
            Self::Accordion | Self::Collapsible => "Expander",
            Self::Alert => "InfoBar",
            Self::AlertDialog | Self::Modal => "ContentDialog",
            Self::AutoComplete => "AutoSuggestBox",
            Self::Avatar => "PersonPicture",
            Self::Badge => "InfoBadge",
            Self::Breadcrumb => "BreadcrumbBar",
            Self::Button => "Button",
            Self::CalendarDatePicker => "CalendarDatePicker",
            Self::Carousel => "FlipView",
            Self::Checkbox => "CheckBox",
            Self::ColorPicker => "ColorPicker",
            Self::Combobox | Self::Select => "ComboBox",
            Self::Containers => "Border",
            Self::ContextMenu | Self::Menu => "MenuFlyout",
            Self::DatePicker => "CalendarView",
            Self::DatePickerWheel => "DatePicker",
            Self::Drawer => "SplitView",
            Self::DropDownButton => "DropDownButton",
            Self::HoverCard | Self::Popover => "Flyout",
            Self::HyperlinkButton => "HyperlinkButton",
            Self::Icon => "FontIcon",
            Self::Image => "Image",
            Self::LoadingOverlay | Self::Spinner => "ProgressRing",
            Self::Menubar => "MenuBar",
            Self::NavigationRail | Self::Sidebar => "NavigationView",
            Self::NumberInput => "NumberBox",
            Self::Pagination => "PipsPager",
            Self::PasswordBox => "PasswordBox",
            Self::Pivot => "Pivot",
            Self::Progress => "ProgressBar",
            Self::Radio => "RadioButton",
            Self::Rating => "RatingControl",
            Self::RepeatButton => "RepeatButton",
            Self::RichText => "RichTextBlock",
            Self::Scroll => "ScrollView",
            Self::Segmented => "SelectorBar",
            Self::Slider => "Slider",
            Self::SplitButton => "SplitButton",
            Self::Stack => "StackPanel",
            Self::Switch => "ToggleSwitch",
            Self::Tabs => "TabView",
            Self::TextArea | Self::TextInput => "TextBox",
            Self::TimePicker => "TimePicker",
            Self::Toast => "TeachingTip",
            Self::ToggleButton => "ToggleButton",
            Self::ToggleSplitButton => "ToggleSplitButton",
            Self::Toolbar => "CommandBar",
            Self::Tooltip => "ToolTip",
            Self::TreeView => "TreeView",
            Self::VirtualViews => "ListView",

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
    /// `Expander → Collapsible`、`ContentDialog → Modal`、`MenuFlyout → Menu`、
    /// `ComboBox → Select`、`NavigationView → Sidebar`、`Flyout → Popover`、
    /// `ProgressRing → Spinner`、`TextBox → TextInput`、`StackPanel → Stack`。
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
            Self::Collapsible
                | Self::Modal
                | Self::Menu
                | Self::Select
                | Self::Sidebar
                | Self::Popover
                | Self::Spinner
                | Self::TextInput
                | Self::Stack
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
        assert_eq!(WidgetCatalogId::Switch.winui3_name(), "ToggleSwitch");
        assert_eq!(WidgetCatalogId::Checkbox.winui3_name(), "CheckBox");
        assert_eq!(WidgetCatalogId::Select.winui3_name(), "ComboBox");
        assert_eq!(WidgetCatalogId::TextInput.winui3_name(), "TextBox");
        assert_eq!(
            WidgetCatalogId::from_winui3_name("ToggleSwitch"),
            Some(WidgetCatalogId::Switch)
        );
        assert_eq!(
            WidgetCatalogId::from_winui3_name("combobox"),
            Some(WidgetCatalogId::Select)
        );
    }

    #[test]
    fn exclusive_controls_never_enter_one_to_one_parity() {
        for id in WidgetCatalogId::ids_in_class(ControlClass::Exclusive) {
            assert!(!id.class().participates_in_parity());
        }
    }
}

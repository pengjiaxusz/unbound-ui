//! 组件展厅：目录中的每个组件均由其真实发射器（emitter）绘制，并在侧边栏中一一列出。
//! lang-allow: bilingual widget catalog labels; Chinese is the zh-CN half
//!
//! 每张卡片驱动 [`unbound_widgets::catalog::emit_widget`]——与支撑 Stable Shape 探针的
//! 穷举式、无通配符匹配相同——因此表面、命中区域与语义节点全部来自真实的
//! `unbound-widgets` 发射器，绝非手绘替身。帧会将组件记录在 `Canvas::emitted_widgets` 中，
//! Showcase 覆盖率门禁读取该记录以证明目录已 100% 承载。
//!
//! [`showcase_entry_for_widget`]、[`SIDEBAR_COMPONENT_IDS`]、[`component_nav_label`] 与
//! [`COMPONENT_EXAMPLES`] 由同一份 ID 列表生成，因此无法在不加入侧边栏与展厅入口函数的前提下
//! 向 `WidgetCatalogId` 新增变体。下方的 `const` 断言在编译期将三者严格绑定到 `WidgetCatalogId::ALL`。

use unbound_math::Rect;
use unbound_widgets::catalog::{emit_widget_with_shaper, CatalogParams};
use unbound_widgets::WidgetCatalogId;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};

pub const COMPONENTS_INFO: PageInfo = PageInfo {
    title: "Component gallery",
    short: "All Components",
    description: "All 133 standard components, each listed in the sidebar and painted by its genuine unbound-widgets emitter with a real hit region, semantic node, and interactive knobs.",
    crates: &["unbound-widgets"],
    keywords: "component gallery catalog emitter coverage widget showcase every control tooltip",
};

const KNOBS_ACTION: &[KnobSpec] = &[
    KnobSpec::toggle("Disabled", false),
    KnobSpec::segmented("Variant", &["Primary", "Secondary", "Outline"], 0),
];

const KNOBS_NUMERIC: &[KnobSpec] = &[
    KnobSpec::toggle("Disabled", false),
    KnobSpec::slider("Value", 0.0, 100.0, 5.0, 50.0, "%"),
];

const KNOBS_DATA: &[KnobSpec] = &[
    KnobSpec::toggle("Compact", false),
    KnobSpec::segmented("Density", &["Comfort", "Normal", "Dense"], 1),
];

const KNOBS_FEEDBACK: &[KnobSpec] = &[
    KnobSpec::segmented("Severity", &["Info", "Success", "Warning", "Error"], 0),
    KnobSpec::toggle("Show Icon", true),
];

const KNOBS_NAV: &[KnobSpec] = &[
    KnobSpec::toggle("Disabled", false),
    KnobSpec::segmented("Size", &["Small", "Medium", "Large"], 1),
];

const KNOBS_LAYOUT: &[KnobSpec] = &[
    KnobSpec::toggle("Bordered", true),
    KnobSpec::slider("Padding", 8.0, 32.0, 4.0, 16.0, "px"),
];

const KNOBS_INPUT: &[KnobSpec] = &[
    KnobSpec::toggle("Disabled", false),
    KnobSpec::toggle("ReadOnly", false),
];

const KNOBS_VISUAL: &[KnobSpec] = &[
    KnobSpec::toggle("Active", true),
    KnobSpec::slider("Scale", 0.5, 2.0, 0.25, 1.0, "x"),
];

/// 为任意组件目录变体获取非空的属性旋钮。
#[must_use]
pub const fn knobs_for_id(id: WidgetCatalogId) -> &'static [KnobSpec] {
    match id {
        WidgetCatalogId::Button
        | WidgetCatalogId::SplitButton
        | WidgetCatalogId::ToggleButton
        | WidgetCatalogId::HyperlinkButton
        | WidgetCatalogId::RepeatButton
        | WidgetCatalogId::DropDownButton
        | WidgetCatalogId::ToggleSplitButton
        | WidgetCatalogId::InfoBadge
        | WidgetCatalogId::Chip
        | WidgetCatalogId::CheckBox
        | WidgetCatalogId::ToggleSwitch
        | WidgetCatalogId::RadioButton
        | WidgetCatalogId::SelectorBar
        | WidgetCatalogId::Stepper
        | WidgetCatalogId::RatingControl
        | WidgetCatalogId::TagInput
        | WidgetCatalogId::PinInput
        | WidgetCatalogId::CommandBarFlyout
        | WidgetCatalogId::AppBarButton
        | WidgetCatalogId::AppBarToggleButton
        | WidgetCatalogId::AppBarSeparator => KNOBS_ACTION,

        WidgetCatalogId::ProgressBar
        | WidgetCatalogId::Slider
        | WidgetCatalogId::NumberBox
        | WidgetCatalogId::ProgressRing
        | WidgetCatalogId::AspectRatioBox
        | WidgetCatalogId::RefreshContainer
        | WidgetCatalogId::AnnotatedScrollBar
        | WidgetCatalogId::RangeSelector
        | WidgetCatalogId::RadialGauge
        | WidgetCatalogId::ImageCropper
        | WidgetCatalogId::TaskHud => KNOBS_NUMERIC,

        WidgetCatalogId::Table
        | WidgetCatalogId::TreeTable
        | WidgetCatalogId::DataGrid
        | WidgetCatalogId::Chart
        | WidgetCatalogId::CodeBlock
        | WidgetCatalogId::CodeEditor
        | WidgetCatalogId::Descriptions
        | WidgetCatalogId::TerminalView
        | WidgetCatalogId::Timeline
        | WidgetCatalogId::TimelineTrack
        | WidgetCatalogId::ListView
        | WidgetCatalogId::Masonry
        | WidgetCatalogId::VirtualMasonry
        | WidgetCatalogId::AvatarGroup
        | WidgetCatalogId::GenericDataTable
        | WidgetCatalogId::GridView
        | WidgetCatalogId::ListBox
        | WidgetCatalogId::ItemsView
        | WidgetCatalogId::OrbitView
        | WidgetCatalogId::BladeView
        | WidgetCatalogId::SwipeControl
        | WidgetCatalogId::PipelineView => KNOBS_DATA,

        WidgetCatalogId::InfoBar
        | WidgetCatalogId::AlertDialog
        | WidgetCatalogId::Banner
        | WidgetCatalogId::Callout
        | WidgetCatalogId::Popconfirm
        | WidgetCatalogId::NotificationCenter
        | WidgetCatalogId::Toast
        | WidgetCatalogId::ContentDialog
        | WidgetCatalogId::SplitView
        | WidgetCatalogId::Flyout
        | WidgetCatalogId::ToolTip
        | WidgetCatalogId::HoverCard
        | WidgetCatalogId::TeachingTip
        | WidgetCatalogId::Tour => KNOBS_FEEDBACK,

        WidgetCatalogId::BreadcrumbBar
        | WidgetCatalogId::NavigationView
        | WidgetCatalogId::PipsPager
        | WidgetCatalogId::Pivot
        | WidgetCatalogId::TabView
        | WidgetCatalogId::TreeView
        | WidgetCatalogId::MenuFlyout
        | WidgetCatalogId::MenuBar
        | WidgetCatalogId::ContextMenu
        | WidgetCatalogId::CommandPalette
        | WidgetCatalogId::CommandBar
        | WidgetCatalogId::CanvasToolbar
        | WidgetCatalogId::StatusBar
        | WidgetCatalogId::AddressBar
        | WidgetCatalogId::ShortcutBar => KNOBS_NAV,

        WidgetCatalogId::Containers
        | WidgetCatalogId::SplitPane
        | WidgetCatalogId::MultiSplit
        | WidgetCatalogId::Dock
        | WidgetCatalogId::ExpanderGroup
        | WidgetCatalogId::Expander
        | WidgetCatalogId::EmptyState
        | WidgetCatalogId::Watermark
        | WidgetCatalogId::LoadingOverlay
        | WidgetCatalogId::NinePatch
        | WidgetCatalogId::StackPanel
        | WidgetCatalogId::Wrap
        | WidgetCatalogId::PropertyGrid
        | WidgetCatalogId::HeaderedContentControl
        | WidgetCatalogId::Divider => KNOBS_LAYOUT,

        WidgetCatalogId::TextBox
        | WidgetCatalogId::TextBoxMultiline
        | WidgetCatalogId::PasswordBox
        | WidgetCatalogId::DatePicker
        | WidgetCatalogId::CalendarDatePicker
        | WidgetCatalogId::AutoSuggestBox
        | WidgetCatalogId::Cascader
        | WidgetCatalogId::ComboBoxEditable
        | WidgetCatalogId::CalendarView
        | WidgetCatalogId::DateRangePicker
        | WidgetCatalogId::TimePicker
        | WidgetCatalogId::HotkeyInput
        | WidgetCatalogId::ColorPicker
        | WidgetCatalogId::ColorSwatch
        | WidgetCatalogId::GradientEditor
        | WidgetCatalogId::CurveEditor
        | WidgetCatalogId::Form
        | WidgetCatalogId::Transfer
        | WidgetCatalogId::FileDrop
        | WidgetCatalogId::ComboBox
        | WidgetCatalogId::QueryBuilder
        | WidgetCatalogId::RichEditBox
        | WidgetCatalogId::SchemaForm => KNOBS_INPUT,

        WidgetCatalogId::NodeGraph
        | WidgetCatalogId::Minimap
        | WidgetCatalogId::Ruler
        | WidgetCatalogId::Gizmo
        | WidgetCatalogId::Image
        | WidgetCatalogId::FontIcon
        | WidgetCatalogId::Skeleton
        | WidgetCatalogId::FlipView
        | WidgetCatalogId::PersonPicture
        | WidgetCatalogId::RichTextBlock
        | WidgetCatalogId::ScrollView
        | WidgetCatalogId::Overlay => KNOBS_VISUAL,
    }
}

/// 任意组件的惯用声明式 Unbound UI 代码。
#[must_use]
pub const fn declarative_code_for_id(id: WidgetCatalogId) -> &'static str {
    match id {
        WidgetCatalogId::ExpanderGroup => "view.accordion(ui_key!(\"acc\"), AccordionDesc::new(rect, sections));",
        WidgetCatalogId::InfoBar => "view.alert(ui_key!(\"alt\"), AlertDesc::message(\"Notice\").severity(Severity::Info));",
        WidgetCatalogId::AlertDialog => "view.add(ui_key!(\"dlg\"), AlertDialogDesc::new(rect, \"Confirm Action\"));",
        WidgetCatalogId::AspectRatioBox => "view.add(ui_key!(\"box\"), AspectRatioBoxDesc::new(rect, 16.0 / 9.0));",
        WidgetCatalogId::AutoSuggestBox => "view.add(ui_key!(\"ac\"), AutoCompleteDesc::new(rect, suggestions));",
        WidgetCatalogId::PersonPicture => "view.add(ui_key!(\"ava\"), AvatarDesc::initials(rect, \"JD\"));",
        WidgetCatalogId::AvatarGroup => "view.add(ui_key!(\"grp\"), AvatarGroupDesc::new(rect, avatars));",
        WidgetCatalogId::InfoBadge => "view.badge(ui_key!(\"bdg\"), BadgeDesc::label(\"New\"));",
        WidgetCatalogId::Banner => "view.add(ui_key!(\"bnr\"), BannerDesc::new(rect, \"System Banner\"));",
        WidgetCatalogId::BreadcrumbBar => "view.add(ui_key!(\"bc\"), BreadcrumbDesc::new(rect, crumbs));",
        WidgetCatalogId::Button => "view.button(ui_key!(\"btn\"), ButtonDesc::label(\"Click Me\").variant(ButtonVariant::Primary));",
        WidgetCatalogId::HyperlinkButton => "view.add(ui_key!(\"link\"), HyperlinkButtonDesc::new(rect, \"Open docs\"));",
        WidgetCatalogId::RepeatButton => "view.add(ui_key!(\"rep\"), RepeatButtonDesc::new(rect, \"Repeat\"));",
        WidgetCatalogId::DropDownButton => "view.add(ui_key!(\"ddb\"), DropDownButtonDesc::new(rect, \"Options\"));",
        WidgetCatalogId::ToggleSplitButton => "view.add(ui_key!(\"tsb\"), ToggleSplitButtonDesc::new(rect, \"Bold\"));",
        WidgetCatalogId::Callout => "view.callout(ui_key!(\"cal\"), CalloutDesc::new(rect, \"Tip\", \"GPU memory is optimal.\"));",
        WidgetCatalogId::CanvasToolbar => "view.add(ui_key!(\"tb\"), CanvasToolbarDesc::new(rect));",
        WidgetCatalogId::FlipView => "view.add(ui_key!(\"car\"), CarouselDesc::new(rect, slides));",
        WidgetCatalogId::Cascader => "view.add(ui_key!(\"cas\"), CascaderDesc::new(rect, tree));",
        WidgetCatalogId::Chart => "view.add(ui_key!(\"chr\"), ChartDesc::new(rect, series));",
        WidgetCatalogId::CheckBox => "view.checkbox(ui_key!(\"chk\"), CheckboxDesc::labeled(\"Accept terms\", true));",
        WidgetCatalogId::Chip => "view.badge(ui_key!(\"chp\"), BadgeDesc::label(\"Filter\"));",
        WidgetCatalogId::CodeBlock => "view.add(ui_key!(\"cb\"), CodeBlockDesc::new(rect, code, \"rust\"));",
        WidgetCatalogId::CodeEditor => "view.add(ui_key!(\"ce\"), CodeEditorDesc::new(rect, buffer));",
        WidgetCatalogId::Expander => "view.add(ui_key!(\"col\"), CollapsibleDesc::new(rect, \"Details\"));",
        WidgetCatalogId::ColorPicker => "view.add(ui_key!(\"cp\"), ColorPickerDesc::new(rect));",
        WidgetCatalogId::ColorSwatch => "view.color_swatch(ui_key!(\"csw\"), ColorSwatchDesc::swatch(Token::accent_swatch(0), \"Accent\"));",
        WidgetCatalogId::ComboBoxEditable => "view.add(ui_key!(\"cmb\"), ComboboxDesc::new(rect, items));",
        WidgetCatalogId::CommandPalette => "view.add(ui_key!(\"cmd\"), CommandPaletteDesc::new(rect));",
        WidgetCatalogId::Containers => "view.card(ui_key!(\"crd\"), CardDesc::default(), |card| { /* content */ });",
        WidgetCatalogId::ContextMenu => "view.add(ui_key!(\"ctx\"), ContextMenuDesc::new(rect, menu));",
        WidgetCatalogId::CurveEditor => "view.add(ui_key!(\"crv\"), CurveEditorDesc::new(rect));",
        WidgetCatalogId::DataGrid => "view.add(ui_key!(\"dg\"), DataGridDesc::new(rect, grid_model));",
        WidgetCatalogId::CalendarView => "view.add(ui_key!(\"dp\"), DatePickerDesc::new(rect));",
        WidgetCatalogId::DatePicker => "view.add(ui_key!(\"dpw\"), DatePickerWheelDesc::new(rect, date));",
        WidgetCatalogId::CalendarDatePicker => "view.add(ui_key!(\"cdp\"), CalendarDatePickerDesc::new(rect, date));",
        WidgetCatalogId::DateRangePicker => "view.add(ui_key!(\"drp\"), DateRangePickerDesc::new(rect));",
        WidgetCatalogId::Descriptions => "view.descriptions(ui_key!(\"desc\"), DescriptionsDesc::new(rect, items));",
        WidgetCatalogId::Divider => "view.divider(ui_key!(\"div\"), DividerDesc::rule());",
        WidgetCatalogId::Dock => "view.add(ui_key!(\"dck\"), DockDesc::new(rect, dock_tree));",
        WidgetCatalogId::SplitView => "view.add(ui_key!(\"drw\"), DrawerDesc::new(rect, DrawerSide::Right));",
        WidgetCatalogId::EmptyState => "view.add(ui_key!(\"emp\"), EmptyStateDesc::new(rect, \"No Items\"));",
        WidgetCatalogId::FileDrop => "view.add(ui_key!(\"fd\"), FileDropDesc::new(rect));",
        WidgetCatalogId::Form => "view.add(ui_key!(\"frm\"), FormDesc::new(rect, fields));",
        WidgetCatalogId::Gizmo => "view.add(ui_key!(\"gz\"), GizmoDesc::new(rect));",
        WidgetCatalogId::GradientEditor => "view.add(ui_key!(\"ge\"), GradientEditorDesc::new(rect));",
        WidgetCatalogId::HotkeyInput => "view.add(ui_key!(\"hk\"), HotkeyInputDesc::new(rect));",
        WidgetCatalogId::HoverCard => "view.add(ui_key!(\"hc\"), HoverCardDesc::new(rect));",
        WidgetCatalogId::FontIcon => "view.add(ui_key!(\"ic\"), IconDesc::new(rect, icon_name));",
        WidgetCatalogId::Image => "view.add(ui_key!(\"img\"), ImageDesc::new(rect, asset_id));",
        WidgetCatalogId::LoadingOverlay => "view.add(ui_key!(\"lo\"), LoadingOverlayDesc::new(rect));",
        WidgetCatalogId::Masonry => "view.add(ui_key!(\"msn\"), MasonryDesc::new(rect, items));",
        WidgetCatalogId::MenuFlyout => "view.add(ui_key!(\"mnu\"), MenuDesc::new(rect, items));",
        WidgetCatalogId::MenuBar => "view.add(ui_key!(\"mb\"), MenuBarDesc::new(rect, menus));",
        WidgetCatalogId::Minimap => "view.add(ui_key!(\"mm\"), MinimapDesc::new(rect));",
        WidgetCatalogId::ContentDialog => "view.add(ui_key!(\"mdl\"), ModalDesc::new(rect, \"Modal Dialog\"));",
        WidgetCatalogId::MultiSplit => "view.multi_split(ui_key!(\"msplit\"), MultiSplitDesc::new(rect, panes));",
        WidgetCatalogId::NinePatch => "view.add(ui_key!(\"np\"), NinePatchDesc::new(rect, asset_id));",
        WidgetCatalogId::NodeGraph => "view.add(ui_key!(\"ng\"), NodeGraphDesc::new(rect, graph));",
        WidgetCatalogId::NotificationCenter => "view.add(ui_key!(\"nc\"), NotificationCenterDesc::new(rect));",
        WidgetCatalogId::NumberBox => "view.add(ui_key!(\"ni\"), NumberInputDesc::new(rect).min(0.0).max(100.0));",
        WidgetCatalogId::Overlay => "view.add(ui_key!(\"ov\"), OverlayDesc::new(rect));",
        WidgetCatalogId::PipsPager => "view.add(ui_key!(\"pg\"), PaginationDesc::new(rect, 100).page_size(10));",
        WidgetCatalogId::PinInput => "view.add(ui_key!(\"pin\"), PinInputDesc::new(rect, 6));",
        WidgetCatalogId::Popconfirm => "view.add(ui_key!(\"pcf\"), PopconfirmDesc::new(rect, \"Delete item?\"));",
        WidgetCatalogId::Flyout => "view.add(ui_key!(\"pop\"), PopoverDesc::new(rect));",
        WidgetCatalogId::ProgressBar => "view.progress(ui_key!(\"prg\"), ProgressDesc::bar(0.75));",
        WidgetCatalogId::PropertyGrid => "view.add(ui_key!(\"pgd\"), PropertyGridDesc::new(rect, props));",
        WidgetCatalogId::RadioButton => "view.add(ui_key!(\"rad\"), RadioGroupDesc::new(rect, options));",
        WidgetCatalogId::RatingControl => "view.add(ui_key!(\"rtg\"), RatingDesc::new(rect, 5).value(4.5));",
        WidgetCatalogId::RichTextBlock => "view.add(ui_key!(\"rt\"), RichTextDesc::new(rect, markdown));",
        WidgetCatalogId::Ruler => "view.add(ui_key!(\"rlr\"), RulerDesc::new(rect));",
        WidgetCatalogId::ScrollView => "view.scroll_view(ui_key!(\"sc\"), true, |ui| { /* content */ });",
        WidgetCatalogId::SelectorBar => "view.add(ui_key!(\"seg\"), SegmentedDesc::new(rect, options));",
        WidgetCatalogId::ComboBox => "view.select(ui_key!(\"sel\"), SelectDesc::options(&[\"A\", \"B\"]).selected(0));",
        WidgetCatalogId::Skeleton => "view.add(ui_key!(\"skl\"), SkeletonDesc::new(rect));",
        WidgetCatalogId::Slider => "view.slider(ui_key!(\"sld\"), SliderDesc::range(0.0..=100.0).value(50.0));",
        WidgetCatalogId::ProgressRing => "view.add(ui_key!(\"spn\"), SpinnerDesc::new(rect));",
        WidgetCatalogId::SplitButton => "view.split_button(ui_key!(\"sbtn\"), SplitButtonDesc::new(rect, \"Run\", \"Build & Run\"));",
        WidgetCatalogId::SplitPane => "view.split_pane(ui_key!(\"sp\"), true, |left| {}, |right| {});",
        WidgetCatalogId::StackPanel => "view.add(ui_key!(\"stk\"), StackDesc::new(rect));",
        WidgetCatalogId::StatusBar => "view.add(ui_key!(\"sb\"), StatusBarDesc::new(rect, segments));",
        WidgetCatalogId::Stepper => "view.add(ui_key!(\"stp\"), StepperDesc::new(rect, steps));",
        WidgetCatalogId::ToggleSwitch => "view.switch(ui_key!(\"sw\"), SwitchDesc::on(true));",
        WidgetCatalogId::Table => "view.table(ui_key!(\"tbl\"), TableDesc::columns(cols).rows(rows));",
        WidgetCatalogId::TabView => "view.add(ui_key!(\"tb\"), TabsDesc::new(rect, tabs));",
        WidgetCatalogId::TagInput => "view.add(ui_key!(\"tag\"), TagInputDesc::new(rect, tags));",
        WidgetCatalogId::TerminalView => "view.terminal_view(ui_key!(\"term\"), TerminalViewDesc::new(rect, lines));",
        WidgetCatalogId::TextBoxMultiline => "view.add(ui_key!(\"ta\"), TextAreaDesc::new(rect));",
        WidgetCatalogId::TextBox => "view.text_input(ui_key!(\"txt\"), TextInputDesc::text(\"\").placeholder(\"Search...\"));",
        WidgetCatalogId::PasswordBox => "view.add(ui_key!(\"pwd\"), PasswordBoxDesc::new(rect, \"Password\"));",
        WidgetCatalogId::TimePicker => "view.add(ui_key!(\"tp\"), TimePickerDesc::new(rect));",
        WidgetCatalogId::Timeline => "view.add(ui_key!(\"tl\"), TimelineDesc::new(rect, events));",
        WidgetCatalogId::TimelineTrack => "view.add(ui_key!(\"tlt\"), TimelineTrackDesc::new(rect));",
        WidgetCatalogId::Toast => "view.add(ui_key!(\"tst\"), ToastDesc::new(rect, \"Notification\"));",
        WidgetCatalogId::ToggleButton => "view.toggle_button(ui_key!(\"tbtn\"), ToggleButtonDesc::new(rect, \"Pin\", true));",
        WidgetCatalogId::CommandBar => "view.add(ui_key!(\"tbr\"), ToolbarDesc::new(rect, tools));",
        WidgetCatalogId::ToolTip => "view.tooltip(ui_key!(\"ttp\"), TooltipDesc::labeled(anchor, viewport, \"Auto-flipping hint\"));",
        WidgetCatalogId::Tour => "view.add(ui_key!(\"tur\"), TourDesc::new(rect, step));",
        WidgetCatalogId::Transfer => "view.add(ui_key!(\"tr\"), TransferDesc::new(rect, items));",
        WidgetCatalogId::TreeTable => "view.tree_table(ui_key!(\"ttbl\"), TreeTableDesc::new(rect, rows));",
        WidgetCatalogId::TreeView => "view.add(ui_key!(\"tv\"), TreeViewDesc::new(rect, tree_nodes));",
        WidgetCatalogId::ListView => "view.add(ui_key!(\"vv\"), VirtualViewsDesc::new(rect));",
        WidgetCatalogId::VirtualMasonry => "view.virtual_masonry(ui_key!(\"vm\"), VirtualMasonryDesc::new(rect, 4, 1000));",
        WidgetCatalogId::Watermark => "view.add(ui_key!(\"wm\"), WatermarkDesc::new(rect, \"CONFIDENTIAL\"));",
        WidgetCatalogId::Wrap => "view.add(ui_key!(\"wrp\"), WrapDesc::new(rect));",
        WidgetCatalogId::AddressBar => "view.add(ui_key!(\"addr\"), AddressBarDesc::new(rect, segments));",
        WidgetCatalogId::GenericDataTable => "view.add(ui_key!(\"gdt\"), GenericDataTableDesc::new(rect, cols, 6));",
        WidgetCatalogId::PipelineView => "view.add(ui_key!(\"pipe\"), PipelineViewDesc::new(rect, stages));",
        WidgetCatalogId::Pivot => "view.add(ui_key!(\"pivot\"), PivotDesc::new(rect, &[\"Overview\", \"Activity\", \"Settings\"]).with_selected(0));",
        WidgetCatalogId::QueryBuilder => "view.add(ui_key!(\"qb\"), QueryBuilderDesc::new(rect, QueryCombinator::And, rules));",
        WidgetCatalogId::SchemaForm => "view.add(ui_key!(\"sf\"), SchemaFormDesc::new(rect, fields));",
        WidgetCatalogId::ShortcutBar => "view.add(ui_key!(\"scb\"), ShortcutBarDesc::new(rect, entries));",
        WidgetCatalogId::NavigationView => "view.add(ui_key!(\"nav\"), NavigationViewDesc::new(rect, menu_items));",
        WidgetCatalogId::TaskHud => "view.add(ui_key!(\"hud\"), TaskHudDesc::new(rect, tasks));",
        WidgetCatalogId::RichEditBox => "view.add(ui_key!(\"reb\"), RichEditBoxDesc::new(rect, \"Document\"));",
        WidgetCatalogId::GridView => "view.add(ui_key!(\"gv\"), GridViewDesc::new(rect, 3, 2).selected(0));",
        WidgetCatalogId::ListBox => "view.add(ui_key!(\"lb\"), ListBoxDesc::new(rect, items).selected(0));",
        WidgetCatalogId::CommandBarFlyout => "view.add(ui_key!(\"cbf\"), CommandBarFlyoutDesc::new(rect));",
        WidgetCatalogId::AppBarButton => "view.add(ui_key!(\"abb\"), AppBarButtonDesc::new(rect, \"Add\"));",
        WidgetCatalogId::AppBarToggleButton => "view.add(ui_key!(\"atb\"), AppBarToggleButtonDesc::new(rect, \"Bold\", true));",
        WidgetCatalogId::AppBarSeparator => "view.add(ui_key!(\"abs\"), AppBarSeparatorDesc::vertical(rect));",
        WidgetCatalogId::SwipeControl => "view.add(ui_key!(\"swp\"), SwipeControlDesc::new(rect).with_left(\"Delete\"));",
        WidgetCatalogId::RefreshContainer => "view.add(ui_key!(\"rfc\"), RefreshContainerDesc::new(rect).pull(0.6));",
        WidgetCatalogId::TeachingTip => "view.add(ui_key!(\"tip\"), TeachingTipDesc::new(rect, \"Title\", \"Body\"));",
        WidgetCatalogId::AnnotatedScrollBar => "view.add(ui_key!(\"asb\"), AnnotatedScrollBarDesc::new(rect).thumb(0.5));",
        WidgetCatalogId::ItemsView => "view.add(ui_key!(\"iv\"), ItemsViewDesc::new(rect, 6).grid(3));",
        WidgetCatalogId::RangeSelector => "view.add(ui_key!(\"rng\"), RangeSelectorDesc::new(rect, 0.0, 100.0).with_range(20.0, 80.0));",
        WidgetCatalogId::RadialGauge => "view.add(ui_key!(\"rg\"), RadialGaugeDesc::new(rect).value_norm(0.65));",
        WidgetCatalogId::OrbitView => "view.add(ui_key!(\"orb\"), OrbitViewDesc::new(rect).with_nodes(6));",
        WidgetCatalogId::BladeView => "view.add(ui_key!(\"blv\"), BladeViewDesc::new(rect).with_blades(3));",
        WidgetCatalogId::HeaderedContentControl => "view.add(ui_key!(\"hcc\"), HeaderedContentDesc::new(rect, \"Appearance\"));",
        WidgetCatalogId::ImageCropper => "view.add(ui_key!(\"crop\"), ImageCropperDesc::new(rect));",
    }
}

/// 由同一份 ID 列表生成展厅示例、侧边栏组件列表与穷举覆盖率门禁。
/// 新增目录变体时必须在此补充条目（否则下方 `const` 断言会失败），
/// 并在 `emit_widget` 中新增分支（`E0004`）。
macro_rules! define_showcase_components {
    ($( $id:ident => $title:literal, $desc:literal, $height:literal ),+ $(,)?) => {
        /// 侧边栏组件导航项顺序（与 `COMPONENT_EXAMPLES` 卡片一一对应）。
        pub const SIDEBAR_COMPONENT_IDS: [WidgetCatalogId; WidgetCatalogId::COUNT] = [
            $( WidgetCatalogId::$id, )+
        ];

        /// 按目录顺序为每个目录组件生成一张卡片。
        pub const COMPONENT_EXAMPLES: &[ExampleDef] = &[
            $(
                ExampleDef::new(
                    $title,
                    $desc,
                    &["unbound-widgets"],
                    declarative_code_for_id(WidgetCatalogId::$id),
                    concat!(
                        "The genuine ", stringify!($id),
                        " emitter paints this surface; its semantic node and hit region come from the component itself.",
                    ),
                    knobs_for_id(WidgetCatalogId::$id),
                    DemoKind::Component { id: WidgetCatalogId::$id },
                )
                .with_height($height),
            )+
        ];

        /// 将每个目录组件映射到承载它的页面和演示。
        ///
        /// 穷举匹配：禁止 `_ =>` 通配符分支。新增的 `WidgetCatalogId` 变体
        /// 在命名之前将无法通过此处编译，与 `unbound-widgets` 中的探针门禁一致。
        #[deny(unreachable_patterns)]
        pub const fn showcase_entry_for_widget(id: WidgetCatalogId) -> (super::PageId, DemoKind) {
            match id {
                $(
                    WidgetCatalogId::$id => (
                        super::PageId::Components,
                        DemoKind::Component { id: WidgetCatalogId::$id },
                    ),
                )+
            }
        }

        /// 获取每个目录组件在侧边栏中显示的导航名称（穷举匹配，禁止遗漏）。
        #[must_use]
        #[deny(unreachable_patterns)]
        pub const fn component_nav_label(id: WidgetCatalogId) -> &'static str {
            match id {
                $(
                    WidgetCatalogId::$id => $title,
                )+
            }
        }

        /// 获取每个目录组件的英文简介（穷举匹配，禁止遗漏）。
        #[must_use]
        #[deny(unreachable_patterns)]
        pub const fn component_description(id: WidgetCatalogId) -> &'static str {
            match id {
                $(
                    WidgetCatalogId::$id => $desc,
                )+
            }
        }
    };
}

define_showcase_components! {
    ExpanderGroup => "Expander group", "Collapsible sections with a stable header identity.", 320.0,
    InfoBar => "InfoBar", "Severity banner with an optional dismiss affordance.", 240.0,
    AlertDialog => "Alert dialog", "A destructive confirmation placed in the viewport.", 320.0,
    PersonPicture => "PersonPicture", "Initials avatar with a presence indicator.", 240.0,
    InfoBadge => "InfoBadge", "A closable chip — the interactive half of the badge module.", 240.0,
    BreadcrumbBar => "BreadcrumbBar", "Hierarchical trail with per-crumb hit slots.", 240.0,
    Button => "Button", "The flat button: shadow, surface, label, border, focus ring.", 240.0,
    CanvasToolbar => "Canvas toolbar", "Floating zoom and tool controls for the infinite canvas.", 280.0,
    Chart => "Chart", "Line series with axes and a hover crosshair.", 320.0,
    CheckBox => "CheckBox", "A checked box with a shaped mark and label.", 240.0,
    CodeBlock => "Code block", "Syntax-highlighted source with a copy affordance.", 300.0,
    ColorPicker => "Color picker", "HSVA field with HEX / OKLCH readouts.", 360.0,
    ColorSwatch => "Color swatch", "Accent palette tile with a selection ring.", 240.0,
    ComboBoxEditable => "ComboBox (editable)", "Editable trigger with a filtered suggestion list.", 260.0,
    CommandPalette => "Command palette", "Fuzzy command search with shortcuts.", 360.0,
    Containers => "Card container", "Header / body / footer surface from the containers module.", 300.0,
    ContextMenu => "Context menu", "Right-click placement state machine over the menu emitter.", 300.0,
    CurveEditor => "Curve editor", "Editable bezier curve with draggable handles.", 320.0,
    CalendarDatePicker => "Calendar date picker", "Collapsed date trigger with a calendar flyout affordance.", 320.0,
    CalendarView => "CalendarView", "Month calendar with single-date selection.", 360.0,
    DatePicker => "DatePicker", "Three-column month / day / year wheel selector.", 320.0,
    Divider => "Divider", "A hairline separator with an optional label.", 220.0,
    Dock => "Dock", "Tabbed panes with drag-to-dock layout.", 340.0,
    SplitView => "SplitView", "A side drawer with a scrim over the viewport.", 340.0,
    DropDownButton => "Drop-down button", "A button with a fixed trailing chevron that opens a menu.", 240.0,
    FileDrop => "File drop", "A drop target with idle and drag-over states.", 280.0,
    Form => "Form", "Field label with required marker and validation message.", 280.0,
    Gizmo => "Gizmo", "Eight-handle transform gizmo with a size bubble.", 300.0,
    HoverCard => "Hover card", "A rich preview anchored to a trigger.", 280.0,
    HyperlinkButton => "Hyperlink button", "Borderless accent text button with hover and pressed veils.", 240.0,
    Image => "Image", "A textured quad bound to an atlas layer.", 280.0,
    MenuFlyout => "MenuFlyout", "Action rows, separators and shortcuts.", 300.0,
    MenuBar => "MenuBar", "Top-level menu items with keyboard traversal.", 240.0,
    Minimap => "Minimap", "A viewport thumbnail over the world bounds.", 300.0,
    ContentDialog => "ContentDialog", "A centered dialog with a focus trap and scrim.", 360.0,
    NodeGraph => "Node graph", "Ported nodes connected by wires.", 340.0,
    NumberBox => "NumberBox", "Stepper with expression evaluation.", 240.0,
    Overlay => "Overlay stack", "Pure placement bookkeeping — emits no primitives of its own.", 220.0,
    Flyout => "Flyout", "An anchored popup with a directional arrow.", 280.0,
    ProgressBar => "ProgressBar", "A determinate bar driven by a value.", 220.0,
    PropertyGrid => "Property grid", "Sectioned inspector rows with reset points.", 320.0,
    RadioButton => "RadioButton", "A single-choice row in a group.", 240.0,
    RichTextBlock => "RichTextBlock", "Markdown parsed into styled runs.", 320.0,
    Ruler => "Ruler", "Canvas rulers with draggable guides.", 320.0,
    ScrollView => "ScrollView", "A scrollable viewport with a real scrollbar.", 320.0,
    SelectorBar => "SelectorBar", "An unselected slot of a single-choice pill.", 240.0,
    ComboBox => "ComboBox", "A closed select trigger with a value.", 240.0,
    Skeleton => "Skeleton", "An animated loading placeholder.", 220.0,
    Slider => "Slider", "A continuous track with a thumb and step.", 240.0,
    SplitPane => "Split pane", "A draggable divider between two panes.", 320.0,
    ToggleSwitch => "ToggleSwitch", "An on/off toggle with a shaped label.", 240.0,
    Table => "Table", "Column headers and a virtual row window.", 320.0,
    TabView => "TabView", "An inactive tab slot with close affordance.", 240.0,
    TagInput => "Tag input", "Removable tags with an inline editor.", 280.0,
    TextBoxMultiline => "TextBox (multiline)", "Multi-line editing with line numbers.", 300.0,
    TextBox => "TextBox", "A single-line field with caret and selection.", 240.0,
    Toast => "Toast", "A spring-in notification with a countdown.", 280.0,
    CommandBar => "CommandBar", "Button and toggle items in a row.", 240.0,
    ToolTip => "ToolTip", "A delayed, smart-flipping in-window hint.", 260.0,
    TreeView => "Tree view", "An expandable row with keyboard navigation.", 320.0,
    ListView => "ListView", "A Fenwick-backed variable-height virtual row.", 240.0,
    Expander => "Expander", "Single-section disclosure panel with smooth spring progress.", 240.0,
    Masonry => "Masonry grid", "Shortest-column adaptive waterfall grid and aspect-ratio box.", 280.0,
    PipsPager => "PipsPager", "Page buttons with ellipsis folding and page-size metadata.", 220.0,
    PasswordBox => "Password box", "Masked text field with a press-and-hold reveal button.", 240.0,
    Stepper => "Stepper", "Horizontal or vertical multi-step progress indicator.", 240.0,
    AutoSuggestBox => "AutoSuggestBox", "Prefix-filtered input with live suggestion badge.", 220.0,
    Banner => "Banner", "Full-width top global status and recovery banner.", 220.0,
    Cascader => "Cascader", "Multi-level hierarchical option selector.", 220.0,
    DateRangePicker => "Date range picker", "Dual-month calendar range selector with presets.", 260.0,
    HotkeyInput => "Hotkey input", "Live KeyChord shortcut recorder with conflict status.", 220.0,
    NotificationCenter => "Notification center", "Drawer aggregating toast history and unread status.", 260.0,
    PinInput => "PIN input", "Segmented verification code input with paste splitting.", 220.0,
    Popconfirm => "Popconfirm", "Anchored confirmation bubble for destructive actions.", 240.0,
    RatingControl => "RatingControl", "Five-star rating control with half-star precision.", 220.0,
    RepeatButton => "Repeat button", "Press and hold to fire repeated clicks after the first delay.", 240.0,
    ProgressRing => "ProgressRing", "Continuous loading spinner and overlay lock scrim.", 220.0,
    TimePicker => "Time picker", "Hour, minute, and second stepping time selector.", 220.0,
    Transfer => "Transfer", "Dual-column item transfer list with batch move.", 260.0,
    FlipView => "FlipView", "Spring-snapping slide deck with indicator dots.", 240.0,
    CodeEditor => "Code editor", "Editable code surface with line gutter and active line highlight.", 260.0,
    DataGrid => "Data grid", "Editable virtualized spreadsheet grid with cell selection.", 260.0,
    Timeline => "Timeline", "Chronological event stream with status nodes and timestamps.", 260.0,
    TimelineTrack => "Timeline track", "Multi-track keyframe dope sheet with playhead scrubber.", 260.0,
    Tour => "Tour", "Spotlight walkthrough overlay with target cutout and bubble.", 260.0,
    Watermark => "Watermark", "Non-interactive tiled security watermark overlay.", 240.0,
    AspectRatioBox => "Aspect ratio box", "Maintains fixed width/height proportions.", 260.0,
    AvatarGroup => "Avatar group", "Overlapping avatars with overflow counter.", 240.0,
    Chip => "Chip", "Interactive filter chip with close affordance.", 240.0,
    EmptyState => "Empty state", "Centred empty placeholder with action button.", 280.0,
    GradientEditor => "Gradient editor", "Colour ramp editor with draggable colour stops.", 300.0,
    FontIcon => "FontIcon", "Sized vector icon with rounded corners.", 220.0,
    LoadingOverlay => "Loading overlay", "Full-card scrim with centred spinning loader.", 260.0,
    NinePatch => "Nine-patch", "Nine-slice scaling image with fixed border corners.", 260.0,
    StackPanel => "StackPanel", "Layered container stacking children in z-order.", 260.0,
    StatusBar => "Status bar", "Desktop status strip with aligned segments.", 220.0,
    Wrap => "Wrap layout", "Flow layout wrapping items into lines.", 260.0,
    SplitButton => "Split button", "Primary button paired with a dropdown trigger menu.", 220.0,
    ToggleButton => "Toggle button", "Two-state push button with pressed state lock.", 220.0,
    ToggleSplitButton => "Toggle split button", "A split button whose primary half keeps a checked state.", 240.0,
    TreeTable => "Tree table", "Hierarchical data table with expandable rows and columns.", 280.0,
    MultiSplit => "Multi-split", "Resizable multi-pane container with draggable splitters.", 280.0,
    Descriptions => "Descriptions", "Label-value summary display for structured properties.", 260.0,
    Callout => "Callout", "High-visibility information or warning callout box.", 220.0,
    VirtualMasonry => "Virtual masonry", "Virtualized staggered column layout for heterogeneous items.", 280.0,
    TerminalView => "Terminal view", "ANSI-styled virtualized terminal console log viewer.", 280.0,
    AddressBar => "Address bar", "Dual-mode path breadcrumb and full-path text editor.", 240.0,
    GenericDataTable => "Generic data table", "Virtualized table with multi-column sort, Pinyin/Shuangpin filter, and summary footer.", 300.0,
    PipelineView => "Pipeline view", "Render pass and build pipeline stage duration inspector.", 240.0,
    Pivot => "Pivot", "Win11 secondary navigation: plain labels with an accent underline under the selected one.", 240.0,
    QueryBuilder => "Query builder", "Visual AND/OR filter rule builder with field, operator, and value rows.", 280.0,
    SchemaForm => "Schema form", "Declarative schema-driven inspector form with dirty tracking and conditional fields.", 280.0,
    ShortcutBar => "Shortcut bar", "Contextual keycap shortcut hint bar with conflict detection.", 220.0,
    TaskHud => "Task HUD", "Background task progress stack with cancellation affordances.", 260.0,
    NavigationView => "NavigationView", "Full-height navigation panel with grouped sections, active and hovered rows, badges and a pinned footer.", 420.0,
    RichEditBox => "Rich edit box", "WinUI 3 native rich text editor area.", 320.0,
    GridView => "Grid view", "Evenly sized cells with a selection frame and hover rounding.", 320.0,
    ListBox => "List box", "Classic single-selection list with a primary accent indicator.", 300.0,
    CommandBarFlyout => "Command bar flyout", "Mini icon toolbar with a trailing overflow menu.", 260.0,
    AppBarButton => "App bar button", "Compact command-bar icon button with hover and pressed veils.", 240.0,
    AppBarToggleButton => "App bar toggle button", "Command-bar toggle with a primary accent underline when checked.", 240.0,
    AppBarSeparator => "App bar separator", "One-pixel divider between command-bar groups.", 220.0,
    SwipeControl => "Swipe control", "List row that reveals leading and trailing action slots.", 300.0,
    RefreshContainer => "Refresh container", "Pull-to-refresh indicator above a scrollable content area.", 320.0,
    TeachingTip => "Teaching tip", "Coaching bubble with a tail, title, body and action buttons.", 340.0,
    AnnotatedScrollBar => "Annotated scroll bar", "Scroll bar with tick labels and a hover preview tag.", 340.0,
    ItemsView => "Items view", "Unified collection that switches between list and grid layouts.", 320.0,
    RangeSelector => "Range selector", "Dual-thumb slider for low / high value ranges.", 260.0,
    RadialGauge => "Radial gauge", "Circular dial with tick marks and a sweeping needle.", 320.0,
    OrbitView => "Orbit view", "Nodes arranged evenly on a circular orbit ring.", 320.0,
    BladeView => "Blade view", "Horizontal multi-level blade panels with active headers.", 320.0,
    HeaderedContentControl => "Headered content", "Shared header strip above a content region.", 280.0,
    ImageCropper => "Image cropper", "Crop frame with eight resize handles and corner scrim.", 340.0,
}

/// 按激活语言返回目录组件的侧边栏与卡片标题。
#[must_use]
#[deny(unreachable_patterns)]
pub const fn component_nav_label_localized(
    id: WidgetCatalogId,
    locale: unbound_app::Locale,
) -> &'static str {
    if !locale.is_zh() {
        return component_nav_label(id);
    }
    match id {
        WidgetCatalogId::ExpanderGroup => "折叠面板 (Expander Group)",
        WidgetCatalogId::AddressBar => "地址栏 (Address Bar)",
        WidgetCatalogId::InfoBar => "信息栏 (InfoBar)",
        WidgetCatalogId::AlertDialog => "确认弹窗 (Alert Dialog)",
        WidgetCatalogId::AspectRatioBox => "宽高比容器 (Aspect Ratio Box)",
        WidgetCatalogId::AutoSuggestBox => "自动建议框 (AutoSuggestBox)",
        WidgetCatalogId::PersonPicture => "个人头像 (PersonPicture)",
        WidgetCatalogId::AvatarGroup => "头像组 (Avatar Group)",
        WidgetCatalogId::InfoBadge => "信息徽章 (InfoBadge)",
        WidgetCatalogId::Banner => "全局横幅 (Banner)",
        WidgetCatalogId::BreadcrumbBar => "面包屑导航 (BreadcrumbBar)",
        WidgetCatalogId::Button => "按钮 (Button)",
        WidgetCatalogId::Callout => "高亮提示框 (Callout)",
        WidgetCatalogId::CanvasToolbar => "画布工具栏 (Canvas Toolbar)",
        WidgetCatalogId::FlipView => "翻转视图 (FlipView)",
        WidgetCatalogId::Cascader => "级联选择器 (Cascader)",
        WidgetCatalogId::Chart => "图表 (Chart)",
        WidgetCatalogId::CheckBox => "复选框 (CheckBox)",
        WidgetCatalogId::Chip => "过滤胶囊 (Chip)",
        WidgetCatalogId::CodeBlock => "代码块 (Code Block)",
        WidgetCatalogId::CodeEditor => "代码编辑器 (Code Editor)",
        WidgetCatalogId::Expander => "可展开面板 (Expander)",
        WidgetCatalogId::ColorPicker => "颜色选择器 (Color Picker)",
        WidgetCatalogId::ColorSwatch => "颜色色块 (Color Swatch)",
        WidgetCatalogId::ComboBoxEditable => "可编辑下拉框 (ComboBox)",
        WidgetCatalogId::CommandPalette => "命令面板 (Command Palette)",
        WidgetCatalogId::Containers => "卡片容器 (Card Container)",
        WidgetCatalogId::ContextMenu => "右键菜单 (Context Menu)",
        WidgetCatalogId::CurveEditor => "曲线编辑器 (Curve Editor)",
        WidgetCatalogId::DataGrid => "电子表格网格 (Data Grid)",
        WidgetCatalogId::CalendarDatePicker => "日期触发器 (Calendar Date Picker)",
        WidgetCatalogId::CalendarView => "日历视图 (CalendarView)",
        WidgetCatalogId::DatePicker => "日期选择器 (DatePicker)",
        WidgetCatalogId::DateRangePicker => "日期区间选择器 (Date Range Picker)",
        WidgetCatalogId::Descriptions => "描述列表 (Descriptions)",
        WidgetCatalogId::Divider => "分割线 (Divider)",
        WidgetCatalogId::Dock => "停靠面板 (Dock)",
        WidgetCatalogId::SplitView => "拆分视图 (SplitView)",
        WidgetCatalogId::DropDownButton => "下拉按钮 (Drop-Down Button)",
        WidgetCatalogId::EmptyState => "空状态占位 (Empty State)",
        WidgetCatalogId::FileDrop => "文件拖拽区 (File Drop)",
        WidgetCatalogId::Form => "表单 (Form)",
        WidgetCatalogId::GenericDataTable => "泛型数据表 (Generic Data Table)",
        WidgetCatalogId::Gizmo => "变换操控器 (Gizmo)",
        WidgetCatalogId::GradientEditor => "渐变编辑器 (Gradient Editor)",
        WidgetCatalogId::HotkeyInput => "快捷键录制框 (Hotkey Input)",
        WidgetCatalogId::HoverCard => "悬停预览卡 (Hover Card)",
        WidgetCatalogId::HyperlinkButton => "超链接按钮 (Hyperlink Button)",
        WidgetCatalogId::FontIcon => "字体图标 (FontIcon)",
        WidgetCatalogId::Image => "图像视图 (Image)",
        WidgetCatalogId::LoadingOverlay => "加载遮罩 (Loading Overlay)",
        WidgetCatalogId::Masonry => "瀑布流网格 (Masonry Grid)",
        WidgetCatalogId::MenuFlyout => "菜单浮出 (MenuFlyout)",
        WidgetCatalogId::MenuBar => "菜单栏 (MenuBar)",
        WidgetCatalogId::Minimap => "缩略小地图 (Minimap)",
        WidgetCatalogId::ContentDialog => "内容对话框 (ContentDialog)",
        WidgetCatalogId::MultiSplit => "多列分割面板 (Multi-Split)",
        WidgetCatalogId::NinePatch => "九宫格图像 (Nine-Patch)",
        WidgetCatalogId::NodeGraph => "节点蓝图 (Node Graph)",
        WidgetCatalogId::NotificationCenter => "通知中心 (Notification Center)",
        WidgetCatalogId::NumberBox => "数字框 (NumberBox)",
        WidgetCatalogId::Overlay => "浮层栈 (Overlay Stack)",
        WidgetCatalogId::PipsPager => "页码导航 (PipsPager)",
        WidgetCatalogId::PasswordBox => "密码输入框 (Password Box)",
        WidgetCatalogId::PinInput => "验证码输入框 (PIN Input)",
        WidgetCatalogId::PipelineView => "管线阶段视图 (Pipeline View)",
        WidgetCatalogId::Pivot => "页签条 (Pivot)",
        WidgetCatalogId::Popconfirm => "气泡确认框 (Popconfirm)",
        WidgetCatalogId::Flyout => "浮出控件 (Flyout)",
        WidgetCatalogId::ProgressBar => "进度条 (ProgressBar)",
        WidgetCatalogId::PropertyGrid => "属性检视网格 (Property Grid)",
        WidgetCatalogId::QueryBuilder => "查询构建器 (Query Builder)",
        WidgetCatalogId::RadioButton => "单选按钮 (RadioButton)",
        WidgetCatalogId::RatingControl => "评分控件 (RatingControl)",
        WidgetCatalogId::RepeatButton => "长按重复按钮 (Repeat Button)",
        WidgetCatalogId::RichTextBlock => "富文本块 (RichTextBlock)",
        WidgetCatalogId::Ruler => "标尺参考线 (Ruler)",
        WidgetCatalogId::SchemaForm => "Schema 表单 (Schema Form)",
        WidgetCatalogId::ScrollView => "滚动视图 (ScrollView)",
        WidgetCatalogId::SelectorBar => "选择条 (SelectorBar)",
        WidgetCatalogId::ComboBox => "下拉框 (ComboBox)",
        WidgetCatalogId::ShortcutBar => "快捷键提示栏 (Shortcut Bar)",
        WidgetCatalogId::NavigationView => "导航视图 (NavigationView)",
        WidgetCatalogId::Skeleton => "骨架占位屏 (Skeleton)",
        WidgetCatalogId::Slider => "滑块 (Slider)",
        WidgetCatalogId::ProgressRing => "进度环 (ProgressRing)",
        WidgetCatalogId::SplitButton => "分割按钮 (Split Button)",
        WidgetCatalogId::SplitPane => "双栏分割窗格 (Split Pane)",
        WidgetCatalogId::StackPanel => "堆叠面板 (StackPanel)",
        WidgetCatalogId::StatusBar => "状态栏 (Status Bar)",
        WidgetCatalogId::Stepper => "步骤条 (Stepper)",
        WidgetCatalogId::ToggleSwitch => "切换开关 (Toggle Switch)",
        WidgetCatalogId::Table => "虚拟数据表 (Table)",
        WidgetCatalogId::TabView => "选项卡视图 (TabView)",
        WidgetCatalogId::TagInput => "标签输入框 (Tag Input)",
        WidgetCatalogId::TaskHud => "任务进度坞 (Task HUD)",
        WidgetCatalogId::TerminalView => "终端日志视图 (Terminal View)",
        WidgetCatalogId::TextBoxMultiline => "多行文本框 (TextBox)",
        WidgetCatalogId::TextBox => "文本框 (TextBox)",
        WidgetCatalogId::TimePicker => "时间选择器 (Time Picker)",
        WidgetCatalogId::Timeline => "时间轴 (Timeline)",
        WidgetCatalogId::TimelineTrack => "关键帧轨道 (Timeline Track)",
        WidgetCatalogId::Toast => "吐司通知 (Toast)",
        WidgetCatalogId::ToggleButton => "双态切换按钮 (Toggle Button)",
        WidgetCatalogId::ToggleSplitButton => "切换分割按钮 (Toggle Split Button)",
        WidgetCatalogId::CommandBar => "命令栏 (CommandBar)",
        WidgetCatalogId::ToolTip => "工具提示 (ToolTip)",
        WidgetCatalogId::Tour => "漫游引导 (Tour)",
        WidgetCatalogId::Transfer => "双栏穿梭框 (Transfer)",
        WidgetCatalogId::TreeTable => "树形数据表 (Tree Table)",
        WidgetCatalogId::TreeView => "树形视图 (Tree View)",
        WidgetCatalogId::VirtualMasonry => "虚拟瀑布流 (Virtual Masonry)",
        WidgetCatalogId::ListView => "列表视图 (ListView)",
        WidgetCatalogId::Watermark => "安全水印 (Watermark)",
        WidgetCatalogId::Wrap => "自动换行流 (Wrap Layout)",
        WidgetCatalogId::RichEditBox => "富文本编辑框 (Rich Edit Box)",
        WidgetCatalogId::GridView => "网格视图 (Grid View)",
        WidgetCatalogId::ListBox => "列表框 (List Box)",
        WidgetCatalogId::CommandBarFlyout => "命令条浮层 (Command Bar Flyout)",
        WidgetCatalogId::AppBarButton => "命令栏按钮 (App Bar Button)",
        WidgetCatalogId::AppBarToggleButton => "命令栏切换按钮 (App Bar Toggle Button)",
        WidgetCatalogId::AppBarSeparator => "命令栏分隔线 (App Bar Separator)",
        WidgetCatalogId::SwipeControl => "滑动操作容器 (Swipe Control)",
        WidgetCatalogId::RefreshContainer => "下拉刷新容器 (Refresh Container)",
        WidgetCatalogId::TeachingTip => "教学提示气泡 (Teaching Tip)",
        WidgetCatalogId::AnnotatedScrollBar => "标注滚动条 (Annotated Scroll Bar)",
        WidgetCatalogId::ItemsView => "集合视图 (Items View)",
        WidgetCatalogId::RangeSelector => "区间选择器 (Range Selector)",
        WidgetCatalogId::RadialGauge => "环形仪表 (Radial Gauge)",
        WidgetCatalogId::OrbitView => "环绕视图 (Orbit View)",
        WidgetCatalogId::BladeView => "刀片面板 (Blade View)",
        WidgetCatalogId::HeaderedContentControl => "带标题内容容器 (Headered Content)",
        WidgetCatalogId::ImageCropper => "图像裁剪器 (Image Cropper)",
    }
}

/// 按激活语言返回目录组件的卡片简介（穷举覆盖全部目录组件）。
#[must_use]
#[deny(unreachable_patterns)]
pub const fn component_description_localized(
    id: WidgetCatalogId,
    locale: unbound_app::Locale,
) -> &'static str {
    if !locale.is_zh() {
        return component_description(id);
    }
    match id {
        WidgetCatalogId::ExpanderGroup => "带稳定标题节点标识的可折叠分组面板。",
        WidgetCatalogId::InfoBar => "带严重级别标识与可选关闭按钮的信息提示栏。",
        WidgetCatalogId::AlertDialog => "居中于视口的破坏性操作确认对话框。",
        WidgetCatalogId::PersonPicture => "带在线状态指示器的首字母个人头像。",
        WidgetCatalogId::InfoBadge => "可关闭的交互式徽章胶囊，构成徽章模块的交互部分。",
        WidgetCatalogId::BreadcrumbBar => "逐级可独立命中点击的层级面包屑导航条。",
        WidgetCatalogId::Button => "扁平按钮：包含阴影、表面、标签、边框与焦点环。",
        WidgetCatalogId::CanvasToolbar => "面向无限画布的悬浮缩放与工具切换栏。",
        WidgetCatalogId::Chart => "带坐标轴与悬停十字准线的折线数据图表。",
        WidgetCatalogId::CheckBox => "带塑形勾选记号与文字标签的复选框。",
        WidgetCatalogId::CodeBlock => "支持语法高亮与一键复制的代码展示块。",
        WidgetCatalogId::ColorPicker => "HSVA 取色区域并实时输出 HEX / OKLCH 色值。",
        WidgetCatalogId::ColorSwatch => "带选中高亮环的主题强调色块。",
        WidgetCatalogId::ComboBoxEditable => "带实时过滤建议列表的可编辑下拉组合框。",
        WidgetCatalogId::CommandPalette => "支持模糊搜索与快捷键提示的命令面板。",
        WidgetCatalogId::Containers => "由容器模块绘制的页眉 / 正文 / 页脚卡片表面。",
        WidgetCatalogId::ContextMenu => "驱动菜单发射器的右键定位上下文菜单状态机。",
        WidgetCatalogId::CurveEditor => "带可拖拽控制点手柄的贝塞尔曲线编辑器。",
        WidgetCatalogId::CalendarDatePicker => "折叠态日期触发器，点击可展开日历浮层。",
        WidgetCatalogId::CalendarView => "支持单日选择的月历视图控件。",
        WidgetCatalogId::DatePicker => "月 / 日 / 年三列滚轮式日期选择器。",
        WidgetCatalogId::Divider => "支持可选居中文字标签的细线分割符。",
        WidgetCatalogId::Dock => "支持拖拽停靠布局的多页签工作区面板。",
        WidgetCatalogId::SplitView => "带视口遮罩层的侧边抽屉拆分视图。",
        WidgetCatalogId::DropDownButton => "右侧带固定下拉箭头、用于展开菜单的按钮。",
        WidgetCatalogId::FileDrop => "具备空闲态与拖入悬停态的文件拖拽放置区。",
        WidgetCatalogId::Form => "带必填星号标记与校验反馈信息的表单字段。",
        WidgetCatalogId::Gizmo => "带尺寸气泡读数的八手柄变换操控器。",
        WidgetCatalogId::HoverCard => "锚定于触发控件的富内容悬停预览卡片。",
        WidgetCatalogId::HyperlinkButton => "无边框强调色文字按钮，带悬停与按压薄纱。",
        WidgetCatalogId::Image => "绑定至图集图层的纹理四边形视图。",
        WidgetCatalogId::MenuFlyout => "包含操作行、分隔线与快捷键标注的弹出菜单。",
        WidgetCatalogId::MenuBar => "支持键盘焦点遍历的顶层菜单栏。",
        WidgetCatalogId::Minimap => "覆盖世界边界的视口缩略导航小地图。",
        WidgetCatalogId::ContentDialog => "带焦点陷阱与背景遮罩的居中内容对话框。",
        WidgetCatalogId::NodeGraph => "带输入输出端口与贝塞尔连线的节点蓝图。",
        WidgetCatalogId::NumberBox => "支持数学表达式求值的步进数字输入框。",
        WidgetCatalogId::Overlay => "纯定位记账浮层栈，本身不发射可见图元。",
        WidgetCatalogId::Flyout => "带方向指向箭头的锚定浮出弹出层。",
        WidgetCatalogId::ProgressBar => "由当前数值驱动的确定态水平进度条。",
        WidgetCatalogId::PropertyGrid => "支持分组与属性重置点的检视器属性网格。",
        WidgetCatalogId::RadioButton => "单选组内的单行互斥单选按钮。",
        WidgetCatalogId::RichTextBlock => "将 Markdown 解析为多样式文本段的富文本块。",
        WidgetCatalogId::Ruler => "带可拖拽辅助参考线的画布标尺。",
        WidgetCatalogId::ScrollView => "配备真实可拖拽滚动条的可滚动视口。",
        WidgetCatalogId::SelectorBar => "单选分段胶囊中的未选中槽位控件。",
        WidgetCatalogId::ComboBox => "展示当前选中值的收起态下拉选择触发器。",
        WidgetCatalogId::Skeleton => "带微光动画的加载骨架占位符。",
        WidgetCatalogId::Slider => "带拇指滑块与步进刻度的连续轨道滑块。",
        WidgetCatalogId::SplitPane => "双窗格之间带可拖拽分割条的拆分容器。",
        WidgetCatalogId::ToggleSwitch => "带塑形文字标签的开 / 关切换开关。",
        WidgetCatalogId::Table => "带列头与虚拟化行窗口的基础数据表。",
        WidgetCatalogId::TabView => "带关闭按钮的选项卡槽位视图。",
        WidgetCatalogId::TagInput => "带行内文本编辑器的可移除标签输入控件。",
        WidgetCatalogId::TextBoxMultiline => "带行号边栏的多行文本编辑区域。",
        WidgetCatalogId::TextBox => "带光标闪烁与文本选区的单行输入框。",
        WidgetCatalogId::Toast => "带倒计时与弹簧入场动画的吐司通知。",
        WidgetCatalogId::CommandBar => "水平排列按钮与切换项的命令工具栏。",
        WidgetCatalogId::ToolTip => "带延迟显示与窗口边界自动翻转的工具提示。",
        WidgetCatalogId::TreeView => "支持展开折叠与键盘导航的树形目录行。",
        WidgetCatalogId::ListView => "基于树状数组（Fenwick）的变高虚拟列表行。",
        WidgetCatalogId::Expander => "带平滑弹簧展开进度的单节折叠面板。",
        WidgetCatalogId::Masonry => "最短列自适应瀑布流网格与固定比例盒。",
        WidgetCatalogId::PipsPager => "带省略号折叠与页容量信息的页码导航条。",
        WidgetCatalogId::PasswordBox => "带按住显密按钮的掩码密码输入框。",
        WidgetCatalogId::Stepper => "支持水平或垂直布局的多步骤进度指示器。",
        WidgetCatalogId::AutoSuggestBox => "带前缀过滤与实时建议徽章的自动建议输入框。",
        WidgetCatalogId::Banner => "通栏顶部全局状态通知与恢复操作横幅。",
        WidgetCatalogId::Cascader => "多层级联选项选择器。",
        WidgetCatalogId::DateRangePicker => "带快捷预设的双月历日期区间选择器。",
        WidgetCatalogId::HotkeyInput => "带键位冲突状态提示的实时 KeyChord 快捷键录制框。",
        WidgetCatalogId::NotificationCenter => "汇总吐司历史与未读状态的侧边通知中心抽屉。",
        WidgetCatalogId::PinInput => "支持整段粘贴自动拆分的分段验证码输入框。",
        WidgetCatalogId::Popconfirm => "面向破坏性操作的锚定气泡二次确认框。",
        WidgetCatalogId::RatingControl => "支持半星精度的五星评分控件。",
        WidgetCatalogId::RepeatButton => "长按并在初次延迟后持续触发重复点击的按钮。",
        WidgetCatalogId::ProgressRing => "连续旋转加载圆环与遮罩锁定层。",
        WidgetCatalogId::TimePicker => "支持时、分、秒步进调节的时间选择器。",
        WidgetCatalogId::Transfer => "支持批量转移条目的双栏穿梭列表。",
        WidgetCatalogId::FlipView => "带指示圆点与弹簧吸附切换的轮播视图。",
        WidgetCatalogId::CodeEditor => "带行号边栏与当前行高亮的可编辑代码表面。",
        WidgetCatalogId::DataGrid => "支持单元格选中的可编辑虚拟化电子表格网格。",
        WidgetCatalogId::Timeline => "带状态节点与时间戳的按序事件时间轴。",
        WidgetCatalogId::TimelineTrack => "带播放头拖动的动画多轨关键帧轨道表。",
        WidgetCatalogId::Tour => "带目标挖空聚光灯与说明气泡的漫游引导层。",
        WidgetCatalogId::Watermark => "不拦截交互的平铺安全水印覆盖层。",
        WidgetCatalogId::AspectRatioBox => "维持固定宽高比例的布局约束容器。",
        WidgetCatalogId::AvatarGroup => "带溢出计数徽章的叠放头像组。",
        WidgetCatalogId::Chip => "带关闭按钮的交互式过滤标签胶囊。",
        WidgetCatalogId::EmptyState => "带主操作按钮的居中空状态占位视图。",
        WidgetCatalogId::GradientEditor => "带可拖拽色标的渐变色阶编辑器。",
        WidgetCatalogId::FontIcon => "带圆角容器与尺寸控制的矢量字体图标。",
        WidgetCatalogId::LoadingOverlay => "铺满卡片并居中显示旋转加载器的等待遮罩。",
        WidgetCatalogId::NinePatch => "固定四角边框比例的九宫格拉伸图像。",
        WidgetCatalogId::StackPanel => "按 Z 轴顺序层叠排列子元素的堆叠容器。",
        WidgetCatalogId::StatusBar => "按段对齐排列的桌面底部状态栏。",
        WidgetCatalogId::Wrap => "超出宽度自动换行的流式布局容器。",
        WidgetCatalogId::SplitButton => "主操作按钮与右侧下拉菜单触发器的组合按钮。",
        WidgetCatalogId::ToggleButton => "带按下锁定态的双态切换按钮。",
        WidgetCatalogId::ToggleSplitButton => "主操作区保持选中切换状态的分割按钮。",
        WidgetCatalogId::TreeTable => "支持行展开折叠的多列层级树形数据表。",
        WidgetCatalogId::MultiSplit => "带可拖拽分割线的多栏尺寸可调容器。",
        WidgetCatalogId::Descriptions => "面向结构化属性的标签-数值摘要展示列表。",
        WidgetCatalogId::Callout => "高可见度的信息提示或警告高亮说明框。",
        WidgetCatalogId::VirtualMasonry => "面向异构条目的虚拟化交错多列瀑布流。",
        WidgetCatalogId::TerminalView => "支持 ANSI 样式着色的虚拟化终端日志视图。",
        WidgetCatalogId::AddressBar => "兼具路径面包屑与完整路径编辑的双模地址栏。",
        WidgetCatalogId::GenericDataTable => "支持多列排序、拼音/双拼过滤与底部汇总的虚拟数据表。",
        WidgetCatalogId::PipelineView => "渲染通道与构建流水线各阶段耗时检视器。",
        WidgetCatalogId::Pivot => "Win11 二级导航页签条：纯文字标签配选中强调下划线。",
        WidgetCatalogId::QueryBuilder => "带字段、操作符与值行的可视化 AND/OR 过滤规则构建器。",
        WidgetCatalogId::SchemaForm => "带脏状态跟踪与条件显隐的声明式 Schema 表单。",
        WidgetCatalogId::ShortcutBar => "带冲突检测的上下文按键帽快捷键提示栏。",
        WidgetCatalogId::TaskHud => "带取消操作入口的后台任务进度堆栈浮窗。",
        WidgetCatalogId::NavigationView => {
            "全高导航面板：包含分组标题、选中/悬停行、徽章与固定页脚。"
        }
        WidgetCatalogId::RichEditBox => "WinUI 3 风格的富文本编辑区域。",
        WidgetCatalogId::GridView => "等尺寸网格单元，带选中边框与悬停圆角反馈。",
        WidgetCatalogId::ListBox => "带主强调色指示条的经典单选列表框。",
        WidgetCatalogId::CommandBarFlyout => "带末尾溢出菜单的迷你图标工具条浮层。",
        WidgetCatalogId::AppBarButton => "带悬停与按压薄纱的紧凑命令栏图标按钮。",
        WidgetCatalogId::AppBarToggleButton => "选中时带主强调色下划线的命令栏切换按钮。",
        WidgetCatalogId::AppBarSeparator => "命令栏分组间的 1 像素垂直分隔线。",
        WidgetCatalogId::SwipeControl => "左右滑动可揭示前后操作槽位的列表行容器。",
        WidgetCatalogId::RefreshContainer => "位于可滚动内容区顶部的下拉刷新指示容器。",
        WidgetCatalogId::TeachingTip => "带指向尖角、标题、正文与操作按钮的教学引导气泡。",
        WidgetCatalogId::AnnotatedScrollBar => "带刻度标签与悬停预览气泡的标注滚动条。",
        WidgetCatalogId::ItemsView => "可在列表与网格布局间无缝切换的统一集合视图。",
        WidgetCatalogId::RangeSelector => "用于选择上下界区间的双拇指范围滑块。",
        WidgetCatalogId::RadialGauge => "带刻度线与扫描指针的环形仪表盘。",
        WidgetCatalogId::OrbitView => "将节点均匀排布在圆形轨道上的环绕视图。",
        WidgetCatalogId::BladeView => "带激活标题栏的水平多级刀片展开面板。",
        WidgetCatalogId::HeaderedContentControl => "内容区域上方带共享标题条的组合容器。",
        WidgetCatalogId::ImageCropper => "带八个缩放手柄与四角遮罩的图像裁剪框。",
    }
}

/// 每个目录组件的中英文搜索关键词（支持全拼、小鹤/多方案双拼与首字母检索）。
#[must_use]
#[deny(unreachable_patterns)]
pub const fn component_search_keywords(id: WidgetCatalogId) -> &'static str {
    match id {
        WidgetCatalogId::ExpanderGroup => "折叠面板 手风琴 expander group",
        WidgetCatalogId::AddressBar => "地址栏 路径导航 面包屑 address bar",
        WidgetCatalogId::InfoBar => "信息栏 警告提示 横幅 info bar",
        WidgetCatalogId::AlertDialog => "确认对话框 弹窗 alert dialog",
        WidgetCatalogId::AspectRatioBox => "宽高比容器 比例盒 aspect ratio",
        WidgetCatalogId::AutoSuggestBox => "自动建议框 自动补全输入建议 auto suggest box",
        WidgetCatalogId::PersonPicture => "个人头像 用户状态 person picture",
        WidgetCatalogId::AvatarGroup => "头像组 叠放头像 avatar group",
        WidgetCatalogId::InfoBadge => "信息徽章 标记 info badge",
        WidgetCatalogId::Banner => "全局横幅 顶部通知 banner",
        WidgetCatalogId::BreadcrumbBar => "面包屑 路径导航 breadcrumb bar",
        WidgetCatalogId::Button => "按钮 点击 触发器 button",
        WidgetCatalogId::Callout => "高亮提示框 说明框 callout",
        WidgetCatalogId::CanvasToolbar => "画布工具栏 缩放工具 canvas toolbar",
        WidgetCatalogId::FlipView => "翻转视图 轮播走马灯 flip view",
        WidgetCatalogId::Cascader => "级联选择器 多级下拉 cascader",
        WidgetCatalogId::Chart => "图表 折线图 渲染耗时曲线 chart",
        WidgetCatalogId::CheckBox => "复选框 勾选框 check box",
        WidgetCatalogId::Chip => "标签胶囊 过滤标签 chip",
        WidgetCatalogId::CodeBlock => "代码块 语法高亮 code block",
        WidgetCatalogId::CodeEditor => "代码编辑器 着色器编辑 code editor",
        WidgetCatalogId::Expander => "可展开面板 展开收起 expander",
        WidgetCatalogId::ColorPicker => "颜色选择器 调色盘 材质球色彩 color picker",
        WidgetCatalogId::ColorSwatch => "颜色色块 品牌色板 调色板方块 color swatch",
        WidgetCatalogId::ComboBoxEditable => "可编辑下拉框 组合框 combo box editable",
        WidgetCatalogId::CommandPalette => "命令面板 快捷指令搜索 command palette",
        WidgetCatalogId::Containers => "卡片容器 面板 containers card",
        WidgetCatalogId::ContextMenu => "右键菜单 上下文菜单 context menu",
        WidgetCatalogId::CurveEditor => "曲线编辑器 贝塞尔动画曲线 curve editor",
        WidgetCatalogId::DataGrid => "数据网格 电子表格 data grid",
        WidgetCatalogId::CalendarDatePicker => "日期触发器 日历弹层 calendar date picker",
        WidgetCatalogId::CalendarView => "日历视图 月历 calendar view",
        WidgetCatalogId::DatePicker => "日期选择器 滚轮日期 月日年 date picker",
        WidgetCatalogId::DateRangePicker => "日期区间选择器 date range picker",
        WidgetCatalogId::Descriptions => "描述列表 属性键值对 descriptions",
        WidgetCatalogId::Divider => "分割线 分隔符 divider",
        WidgetCatalogId::Dock => "停靠面板 工作区多窗格 dock",
        WidgetCatalogId::SplitView => "拆分视图 侧边抽屉 split view",
        WidgetCatalogId::DropDownButton => "下拉按钮 菜单触发器 drop down button",
        WidgetCatalogId::EmptyState => "空状态 占位提示 empty state",
        WidgetCatalogId::FileDrop => "文件拖拽上传 资产导入 file drop",
        WidgetCatalogId::Form => "表单 校验字段 form",
        WidgetCatalogId::GenericDataTable => {
            "泛型数据表格 拼音双拼过滤 排序汇总 generic data table"
        }
        WidgetCatalogId::Gizmo => "变换操控器 旋转缩放手柄 gizmo",
        WidgetCatalogId::GradientEditor => "渐变编辑器 色阶条 gradient editor",
        WidgetCatalogId::HotkeyInput => "快捷键录制 键位绑定 hotkey input",
        WidgetCatalogId::HoverCard => "悬停预览卡 浮窗卡片 hover card",
        WidgetCatalogId::HyperlinkButton => "超链接按钮 强调色文字 hyperlink button",
        WidgetCatalogId::FontIcon => "字体图标 矢量图标 font icon",
        WidgetCatalogId::Image => "图像 纹理贴图预览 image",
        WidgetCatalogId::LoadingOverlay => "加载遮罩 等待浮层 loading overlay",
        WidgetCatalogId::Masonry => "瀑布流布局 资产墙 masonry",
        WidgetCatalogId::MenuFlyout => "菜单浮出 下拉菜单 menu flyout",
        WidgetCatalogId::MenuBar => "菜单栏 顶部菜单 menu bar",
        WidgetCatalogId::Minimap => "小地图 缩略导航 minimap",
        WidgetCatalogId::ContentDialog => "内容对话框 模态框 弹窗 content dialog",
        WidgetCatalogId::MultiSplit => "多列分割面板 可调分栏 multi split",
        WidgetCatalogId::NinePatch => "九宫格拉伸图 nine patch",
        WidgetCatalogId::NodeGraph => "节点图 着色器管线蓝图 node graph",
        WidgetCatalogId::NotificationCenter => "通知中心 消息盒子 notification center",
        WidgetCatalogId::NumberBox => "数字框 数值输入 步进器 number box",
        WidgetCatalogId::Overlay => "浮层栈 弹出定位层 overlay",
        WidgetCatalogId::PipsPager => "页码导航 分页器 pips pager",
        WidgetCatalogId::PasswordBox => "密码输入框 掩码 显密 password box",
        WidgetCatalogId::PinInput => "验证码输入框 PIN码 pin input",
        WidgetCatalogId::PipelineView => "渲染管线视图 阶段耗时分析 着色器管线 pipeline view",
        WidgetCatalogId::Pivot => "页签条 二级导航 pivot tabs",
        WidgetCatalogId::Popconfirm => "气泡确认框 删除确认 popconfirm",
        WidgetCatalogId::Flyout => "浮出控件 锚定弹窗 flyout",
        WidgetCatalogId::ProgressBar => "进度条 progress bar",
        WidgetCatalogId::PropertyGrid => "属性网格 检视器面板 property grid",
        WidgetCatalogId::QueryBuilder => "查询构建器 过滤规则树 query builder",
        WidgetCatalogId::RadioButton => "单选按钮 单选组 radio button",
        WidgetCatalogId::RatingControl => "评分控件 星级评分 rating control",
        WidgetCatalogId::RepeatButton => "长按重复按钮 连续触发 repeat button",
        WidgetCatalogId::RichTextBlock => "富文本块 Markdown渲染 rich text block",
        WidgetCatalogId::Ruler => "标尺 辅助参考线 ruler",
        WidgetCatalogId::SchemaForm => "声明式配置表单 属性检查器 schema form",
        WidgetCatalogId::ScrollView => "滚动视图 平滑滚轮 scroll view",
        WidgetCatalogId::SelectorBar => "选择条 分段控制器 selector bar",
        WidgetCatalogId::ComboBox => "下拉框 下拉选择器 枚举选择 combo box",
        WidgetCatalogId::ShortcutBar => "快捷键提示栏 键位冲突检测 shortcut bar",
        WidgetCatalogId::NavigationView => "导航视图 侧边栏 导航面板 navigation view",
        WidgetCatalogId::Skeleton => "骨架屏 加载占位 skeleton",
        WidgetCatalogId::Slider => "滑块 区间滑块 磁吸刻度 slider range snap",
        WidgetCatalogId::ProgressRing => "进度环 加载转圈 progress ring",
        WidgetCatalogId::SplitButton => "分割按钮 下拉操作按钮 split button",
        WidgetCatalogId::SplitPane => "双栏分割窗格 split pane",
        WidgetCatalogId::StackPanel => "堆叠面板 层叠容器 stack panel",
        WidgetCatalogId::StatusBar => "底部状态栏 status bar",
        WidgetCatalogId::Stepper => "步骤条 向导进度 stepper",
        WidgetCatalogId::ToggleSwitch => "切换开关 开关 toggle switch",
        WidgetCatalogId::Table => "基础数据表 虚拟行 table",
        WidgetCatalogId::TabView => "选项卡视图 标签页 tab view",
        WidgetCatalogId::TagInput => "标签输入框 多标签编辑 tag input",
        WidgetCatalogId::TaskHud => "后台任务浮窗 烘焙进度栈 task hud",
        WidgetCatalogId::TerminalView => "终端控制台 日志视图 terminal view",
        WidgetCatalogId::TextBoxMultiline => "多行文本框 文本编辑 text box multiline",
        WidgetCatalogId::TextBox => "文本框 单行输入框 只读复制框 text box",
        WidgetCatalogId::TimePicker => "时间选择器 时分秒 time picker",
        WidgetCatalogId::Timeline => "时间轴 事件流 timeline",
        WidgetCatalogId::TimelineTrack => "关键帧轨道 动画时间线 timeline track",
        WidgetCatalogId::Toast => "轻量吐司通知 浮动提示 toast",
        WidgetCatalogId::ToggleButton => "双态切换按钮 按下锁定 toggle button",
        WidgetCatalogId::ToggleSplitButton => "切换分割按钮 选中态 toggle split button",
        WidgetCatalogId::CommandBar => "命令栏 工具栏 command bar",
        WidgetCatalogId::ToolTip => "工具提示 文字提示气泡 tool tip",
        WidgetCatalogId::Tour => "漫游引导 新手聚光灯 tour",
        WidgetCatalogId::Transfer => "穿梭框 双栏列表转移 transfer",
        WidgetCatalogId::TreeTable => "树形表格 层级数据表 tree table",
        WidgetCatalogId::TreeView => "树形视图 目录树 重命名文件 tree view",
        WidgetCatalogId::VirtualMasonry => "虚拟化瀑布流 大规模缩略图 virtual masonry",
        WidgetCatalogId::ListView => "列表视图 变高虚拟列表 list view",
        WidgetCatalogId::Watermark => "安全水印 平铺背景 watermark",
        WidgetCatalogId::Wrap => "自动换行流式布局 wrap",
        WidgetCatalogId::RichEditBox => "富文本编辑框 格式工具条 加粗斜体下划线 rich edit box",
        WidgetCatalogId::GridView => "网格视图 单元选中 悬停圆角 grid view",
        WidgetCatalogId::ListBox => "列表框 单选列表 list box",
        WidgetCatalogId::CommandBarFlyout => "命令条浮层 迷你工具条 更多菜单 command bar flyout",
        WidgetCatalogId::AppBarButton => "命令栏按钮 图标按钮 app bar button",
        WidgetCatalogId::AppBarToggleButton => "命令栏切换按钮 选中指示条 app bar toggle button",
        WidgetCatalogId::AppBarSeparator => "命令栏分隔线 app bar separator",
        WidgetCatalogId::SwipeControl => "滑动操作 左滑右滑删除归档 swipe control",
        WidgetCatalogId::RefreshContainer => "下拉刷新 刷新指示器 refresh container",
        WidgetCatalogId::TeachingTip => "教学提示 引导气泡 尖角 teaching tip",
        WidgetCatalogId::AnnotatedScrollBar => "标注滚动条 刻度标签 悬停预览 annotated scroll bar",
        WidgetCatalogId::ItemsView => "集合视图 列表网格切换 items view",
        WidgetCatalogId::RangeSelector => "区间选择器 双拇指 range selector slider",
        WidgetCatalogId::RadialGauge => "环形仪表 刻度指针 radial gauge dial",
        WidgetCatalogId::OrbitView => "环绕布局 圆形排列 orbit view",
        WidgetCatalogId::BladeView => "刀片面板 多级面板 blade view",
        WidgetCatalogId::HeaderedContentControl => "带标题内容容器 标题条 headered content control",
        WidgetCatalogId::ImageCropper => "图像裁剪 九宫格手柄 image cropper crop",
    }
}

/// 返回组件在 `COMPONENT_EXAMPLES` 中的卡片索引。
#[must_use]
pub const fn component_card_index(id: WidgetCatalogId) -> usize {
    let mut j = 0;
    while j < SIDEBAR_COMPONENT_IDS.len() {
        if SIDEBAR_COMPONENT_IDS[j] as usize == id as usize {
            return j;
        }
        j += 1;
    }
    0
}

/// 计算滚动到 `PageId::Components` 上指定组件卡片顶部的精确垂直滚动偏移（逻辑像素）。
#[must_use]
pub fn component_scroll_offset(id: WidgetCatalogId) -> f32 {
    let target_idx = component_card_index(id);
    if target_idx == 0 {
        return 0.0;
    }
    // 页头区高度：PAGE_PAD_TOP (26) + header (140) + PAGE_HEAD_GAP (6) = 172.0
    let mut offset = 172.0_f32;
    for example in COMPONENT_EXAMPLES.iter().take(target_idx) {
        // 单张卡片高度：CARD_PAD_TOP(12) + CARD_HEAD(64) + stage_height + CODE_GAP(6) + CODE_BAR(30) + CARD_PAD_BOTTOM(10) = 122 + stage_height
        // 加卡片间距 CARD_GAP(18) = 140 + stage_height
        offset += 140.0 + example.stage_height;
    }
    offset
}

/// `examples` 是否为 `id` 挂载了卡片（可 const 比较）。
const fn hosts(examples: &[ExampleDef], id: WidgetCatalogId) -> bool {
    let mut j = 0;
    while j < examples.len() {
        if let DemoKind::Component { id: other } = examples[j].demo {
            if other as usize == id as usize {
                return true;
            }
        }
        j += 1;
    }
    false
}

/// 编译期覆盖率门禁：每个目录组件都必须映射到目标页面实际挂载的演示、
/// 出现在侧边栏列表 `SIDEBAR_COMPONENT_IDS` 中，且拥有非空的交互旋钮与导航标签。
const _: () = {
    assert!(
        SIDEBAR_COMPONENT_IDS.len() == WidgetCatalogId::COUNT,
        "SIDEBAR_COMPONENT_IDS must contain every WidgetCatalogId",
    );
    let mut i = 0;
    while i < WidgetCatalogId::COUNT {
        let id = WidgetCatalogId::ALL[i];
        let (page, _demo) = showcase_entry_for_widget(id);
        assert!(
            hosts(page.examples(), id),
            "catalog component is not mounted by its showcase page",
        );
        let idx = component_card_index(id);
        assert!(
            SIDEBAR_COMPONENT_IDS[idx] as usize == id as usize,
            "every catalog component must be present in SIDEBAR_COMPONENT_IDS",
        );
        assert!(
            !component_nav_label(id).is_empty(),
            "every catalog component must have a non-empty sidebar label",
        );
        let knobs = knobs_for_id(id);
        assert!(
            !knobs.is_empty(),
            "every showcase component must have non-empty knobs",
        );
        i += 1;
    }
};

/// 使用真实发射器渲染单个组件卡片。
pub fn component(ctx: &mut DemoCtx<'_, '_>, id: WidgetCatalogId) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;

    let clicks = ctx.state.sandbox.get_clicks(id);
    let title_text = if clicks > 0 {
        format!(
            "{} • {} {}",
            id.module_name(),
            clicks,
            ctx.state.locale.pick("clicks", "次点击")
        )
    } else {
        id.module_name().to_string()
    };

    ctx.canvas.text(
        &title_text,
        Rect::new(
            rect.origin.x + 16.0,
            rect.origin.y + 12.0,
            rect.size.x - 32.0,
            18.0,
        ),
        11.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );

    let knobs = knobs_for_id(id);
    let mut pad_x = 24.0_f32;
    let mut bordered = true;
    let mut active = true;
    let mut compact = false;
    let mut show_icon = true;
    let mut read_only = false;
    let mut knob_variant = 0usize;
    let mut knob_value_norm = 0.5_f32;

    if let Some(k0) = knobs.first() {
        match k0 {
            KnobSpec::Segmented { .. } => {
                knob_variant = ctx.knob(0).round().max(0.0) as usize;
            }
            KnobSpec::Toggle {
                label: "Active", ..
            } => {
                active = ctx.knob_on(0);
            }
            KnobSpec::Toggle {
                label: "Bordered", ..
            } => {
                bordered = ctx.knob_on(0);
            }
            KnobSpec::Toggle {
                label: "Compact", ..
            } => {
                compact = ctx.knob_on(0);
            }
            _ => {}
        }
    }
    if let Some(k1) = knobs.get(1) {
        match k1 {
            KnobSpec::Slider {
                label, min, max, ..
            } => {
                let span = (max - min).max(1e-5);
                knob_value_norm = ((ctx.knob(1) - min) / span).clamp(0.0, 1.0);
                if *label == "Padding" {
                    pad_x = ctx.knob(1).clamp(8.0, 32.0);
                }
            }
            KnobSpec::Segmented { .. } => {
                knob_variant = ctx.knob(1).round().max(0.0) as usize;
            }
            KnobSpec::Toggle {
                label: "Show Icon", ..
            } => {
                show_icon = ctx.knob_on(1);
            }
            KnobSpec::Toggle {
                label: "ReadOnly", ..
            } => {
                read_only = ctx.knob_on(1);
            }
            _ => {}
        }
    }

    let stage = Rect::new(
        rect.origin.x + pad_x,
        rect.origin.y + 42.0,
        (rect.size.x - pad_x * 2.0).max(40.0),
        (rect.size.y - 66.0).max(40.0),
    );
    let node = ctx.part(1);
    let checked = ctx.state.sandbox.get_toggle(id);
    let selected_idx = ctx.state.sandbox.get_selected(id);
    let selected = checked || selected_idx > 0;
    let mut state = ctx.canvas.control_state(node, false, selected);
    if matches!(
        knobs.first(),
        Some(
            KnobSpec::Toggle {
                label: "Disabled",
                ..
            } | KnobSpec::CheckBox {
                label: "Disabled",
                ..
            }
        )
    ) && ctx.knob_on(0)
    {
        state.disabled = true;
        state.phase = unbound_widgets::PointerPhase::Idle;
    }

    let value_norm = if let Some(&sandbox_val) = ctx.state.sandbox.slider_values.get(&(id as u8)) {
        (sandbox_val / 100.0).clamp(0.0, 1.0)
    } else {
        knob_value_norm
    };
    let text_step = ctx
        .state
        .sandbox
        .text_buffers
        .get(&(id as u8))
        .map_or(0, |buf| buf.len() / 8);
    let variant_idx = selected_idx
        .wrapping_add(knob_variant)
        .wrapping_add(text_step);

    let params = CatalogParams {
        value_norm,
        variant_idx,
        show_icon,
        bordered,
        active,
        compact,
        read_only,
        toggled: checked,
        click_count: clicks,
    };
    let output = emit_widget_with_shaper(id, node, state, stage, params, |text, origin, size| {
        ctx.canvas.shape(text, origin, size)
    });
    ctx.canvas.widget_output(id, node, &output);
}


// ============================================================================
// 分类索引页（Category Hub）与直观控件卡片（ADR 0035）
// ============================================================================

const HUB_FILTER_H: f32 = 34.0;
const HUB_CAT_STRIP_H: f32 = 34.0;
const HUB_GAP: f32 = 12.0;
const HUB_CARD_H: f32 = 76.0;

fn hub_columns(width: f32) -> usize {
    if width >= 1040.0 {
        4
    } else if width >= 720.0 {
        3
    } else if width >= 440.0 {
        2
    } else {
        1
    }
}

/// 按当前分类与来源层级过滤控件列表。
#[must_use]
pub fn filtered_hub_controls(state: &crate::state::GalleryState) -> Vec<WidgetCatalogId> {
    WidgetCatalogId::ALL
        .iter()
        .copied()
        .filter(|id| {
            if let Some(cat) = state.active_category {
                if id.category() != cat {
                    return false;
                }
            }
            if let Some(class) = state.control_origin_filter {
                if id.class() != class {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// 返回分类索引页在指定宽度下的总高度。
#[must_use]
pub fn hub_height(width: f32, state: &crate::state::GalleryState) -> f32 {
    let items = filtered_hub_controls(state);
    let cols = hub_columns(width);
    let rows = items.len().max(1).div_ceil(cols);
    let cat_strip = if state.active_category.is_none() {
        HUB_CAT_STRIP_H + HUB_GAP
    } else {
        0.0
    };
    HUB_FILTER_H
        + HUB_GAP
        + cat_strip
        + rows as f32 * HUB_CARD_H
        + (rows.saturating_sub(1)) as f32 * HUB_GAP
        + 16.0
}

/// 绘制单张直观控件卡片（用于首页快捷区与分类索引页）。
pub fn draw_control_tile(
    canvas: &mut crate::canvas::Canvas<'_>,
    state: &crate::state::GalleryState,
    rect: Rect,
    id: WidgetCatalogId,
) {
    let palette = *canvas.palette;
    let nav_node = crate::target::id(crate::target::Target::NavComponent(id as u8));
    let visual = canvas.visual(nav_node, false);
    let bg = if visual.is_pressed() || visual.is_hovered() {
        palette.primary_soft
    } else {
        palette.card_bg
    };
    let border = if visual.is_hovered() {
        palette.primary
    } else {
        palette.card_border
    };
    canvas.rect(rect, 10.0, bg, border, 1.0, 0.0);

    // 左侧语义图标盒（取代原先单字母圆标）
    let cat = id.category();
    let (accent_fg, accent_bg) = super::home::category_accent(palette, cat.index());
    let icon_box = Rect::new(rect.origin.x + 10.0, rect.origin.y + 18.0, 36.0, 36.0);
    canvas.rect(icon_box, 8.0, accent_bg, 0, 0.0, 0.0);
    crate::widgets::icon(canvas, super::home::category_widget_icon(cat), icon_box, accent_fg, accent_bg);

    // 右上来源徽章（WinUI 3 / Toolkit / Unbound）与收藏星标
    let star_rect = Rect::new(rect.max().x - 28.0, rect.origin.y + 8.0, 20.0, 20.0);
    let starred = state.is_favorite(id);
    let star_node = crate::target::id(crate::target::Target::FavoriteToggle(id as u8));
    let star_visual = canvas.visual(star_node, starred);
    let star_fg = if starred || star_visual.is_hovered() {
        palette.warning
    } else {
        palette.text_muted
    };
    canvas.text(
        if starred { "★" } else { "☆" },
        star_rect,
        12.5,
        star_fg,
        AlignX::Center,
        AlignY::Center,
    );

    let badge_text = super::control_class_badge(id.class());
    let (b_fg, b_bg) = super::home::origin_badge_colors(palette, id.class());
    let badge_w = canvas.labels.measure(badge_text, 9.0).x + 12.0;
    let badge_rect = Rect::new(star_rect.origin.x - badge_w - 4.0, rect.origin.y + 10.0, badge_w, 16.0);
    crate::widgets::badge(canvas, badge_rect, badge_text, b_fg, b_bg);

    // 控件规范名 + 一句话用途
    let label = component_nav_label_localized(id, state.locale);
    let title_w = (badge_rect.origin.x - (icon_box.max().x + 10.0) - 6.0).max(40.0);
    canvas.text(
        label,
        Rect::new(icon_box.max().x + 10.0, rect.origin.y + 10.0, title_w, 18.0),
        12.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    let desc = super::category_localized_summary(cat, state.locale);
    canvas.text(
        desc,
        Rect::new(
            icon_box.max().x + 10.0,
            rect.origin.y + 34.0,
            (rect.size.x - 66.0).max(40.0),
            30.0,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );

    // 主卡片命中区排除右上角收藏星标，避免重叠误触
    let card_hit = Rect::new(rect.origin.x, rect.origin.y, rect.size.x - 30.0, rect.size.y);
    canvas.control(
        nav_node,
        card_hit,
        accesskit::Role::Link,
        label,
        unbound_input::CursorKind::Pointer,
        false,
        false,
    );
    canvas.control(
        star_node,
        star_rect,
        accesskit::Role::Button,
        if starred { "Remove favorite" } else { "Add favorite" },
        unbound_input::CursorKind::Pointer,
        starred,
        false,
    );
}

/// 绘制分类索引页 / 全部控件总览页（脱离 `ExampleCard` 外框）。
pub fn draw_category_hub(
    canvas: &mut crate::canvas::Canvas<'_>,
    state: &crate::state::GalleryState,
    body: Rect,
) {
    let palette = *canvas.palette;
    let zh = state.locale.is_zh();
    let mut y = body.origin.y;

    // 1. 来源层级筛选条：All / WinUI 3 / Toolkit / Unbound
    let scoped: Vec<WidgetCatalogId> = WidgetCatalogId::ALL
        .iter()
        .copied()
        .filter(|id| state.active_category.is_none_or(|c| id.category() == c))
        .collect();
    let n_all = scoped.len();
    let n_base = scoped
        .iter()
        .filter(|id| id.class() == unbound_widgets::ControlClass::Baseline)
        .count();
    let n_tk = scoped
        .iter()
        .filter(|id| id.class() == unbound_widgets::ControlClass::Toolkit)
        .count();
    let n_ex = scoped
        .iter()
        .filter(|id| id.class() == unbound_widgets::ControlClass::Exclusive)
        .count();

    let filters = [
        (0u8, None, if zh { format!("全部 ({n_all})") } else { format!("All ({n_all})") }),
        (1u8, Some(unbound_widgets::ControlClass::Baseline), format!("WinUI 3 ({n_base})")),
        (2u8, Some(unbound_widgets::ControlClass::Toolkit), format!("Toolkit ({n_tk})")),
        (3u8, Some(unbound_widgets::ControlClass::Exclusive), format!("Unbound ({n_ex})")),
    ];
    let mut fx = body.origin.x;
    for (idx, class_opt, label) in filters {
        let w = canvas.labels.measure(&label, 11.0).x + 20.0;
        let pill = Rect::new(fx, y, w, 28.0);
        let active = state.control_origin_filter == class_opt;
        let node = crate::target::id(crate::target::Target::ControlFilter(idx));
        let visual = canvas.visual(node, active);
        let fill = if active {
            palette.primary
        } else if visual.is_hovered() {
            palette.primary_soft
        } else {
            palette.card_bg
        };
        let fg = if active { palette.on_primary } else { palette.text };
        canvas.rect(pill, 14.0, fill, palette.card_border, if active { 0.0 } else { 1.0 }, 0.0);
        canvas.text(&label, pill, 11.0, fg, AlignX::Center, AlignY::Center);
        canvas.control(
            node,
            pill,
            accesskit::Role::Button,
            &label,
            unbound_input::CursorKind::Pointer,
            active,
            false,
        );
        fx += w + 8.0;
    }
    y += HUB_FILTER_H + HUB_GAP;

    // 2. 在 All Controls 总览页提供快捷分类跳转胶囊
    if state.active_category.is_none() {
        let mut cx = body.origin.x;
        for cat in unbound_widgets::ControlCategory::ALL {
            let label = super::category_localized_label(cat, state.locale);
            let w = canvas.labels.measure(label, 10.0).x + 16.0;
            if cx + w > body.max().x {
                break;
            }
            let pill = Rect::new(cx, y, w, 24.0);
            let node = crate::target::id(crate::target::Target::NavCategory(cat.index() as u8));
            let visual = canvas.visual(node, false);
            let fill = if visual.is_hovered() {
                palette.primary_soft
            } else {
                palette.surface
            };
            canvas.rect(pill, 12.0, fill, palette.card_border, 1.0, 0.0);
            canvas.text(label, pill, 10.0, palette.text, AlignX::Center, AlignY::Center);
            canvas.control(
                node,
                pill,
                accesskit::Role::Link,
                label,
                unbound_input::CursorKind::Pointer,
                false,
                false,
            );
            cx += w + 6.0;
        }
        y += HUB_CAT_STRIP_H + HUB_GAP;
    }

    // 3. 响应式控件卡片网格
    let items = filtered_hub_controls(state);
    if items.is_empty() {
        let empty = Rect::new(body.origin.x, y, body.size.x, 76.0);
        canvas.rect(empty, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        canvas.text(
            if zh {
                "当前来源筛选下暂无控件，点击上方“全部”可恢复显示。"
            } else {
                "No controls match this origin filter. Select 'All' above to view every control."
            },
            empty,
            12.0,
            palette.text_muted,
            AlignX::Center,
            AlignY::Center,
        );
        return;
    }

    let cols = hub_columns(body.size.x);
    let cell_w = (body.size.x - HUB_GAP * (cols - 1) as f32) / cols as f32;
    let visible_top = canvas.viewport.origin.y - 60.0;
    let visible_bottom = canvas.viewport.max().y + 60.0;

    for (idx, comp_id) in items.iter().copied().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cell = Rect::new(
            body.origin.x + col as f32 * (cell_w + HUB_GAP),
            y + row as f32 * (HUB_CARD_H + HUB_GAP),
            cell_w,
            HUB_CARD_H,
        );
        if cell.max().y < visible_top || cell.origin.y > visible_bottom {
            continue;
        }
        draw_control_tile(canvas, state, cell, comp_id);
    }
}

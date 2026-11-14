//! 组件展厅：目录中的每个组件均由其真实发射器（emitter）绘制，并在侧边栏中一一列出。
//!
//! 每张卡片驱动 [`unbound_widgets::catalog::emit_widget`]——与支撑 Stable Shape 探针的
//! 穷举式、无通配符匹配相同——因此表面、命中区域与语义节点全部来自真实的
//! `unbound-widgets` 发射器，绝非手绘替身。帧会将组件记录在 `Canvas::emitted_widgets` 中，
//! Showcase 覆盖率门禁读取该记录以证明目录已 100% 承载。
//!
//! [`showcase_entry_for_widget`]、[`SIDEBAR_COMPONENT_IDS`]、[`component_nav_label`] 与
//! [`COMPONENT_EXAMPLES`] 由同一份 ID 列表生成，因此无法在不加入侧边栏与展厅入口函数的前提下
//! 向 `WidgetCatalogId` 新增变体。下方的 `const` 断言在编译期将三者严格绑定到 `WidgetCatalogId::ALL`。

use unbound_math::{Rect, Vec2};
use unbound_widgets::catalog::emit_widget;
use unbound_widgets::WidgetCatalogId;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};

pub const COMPONENTS_INFO: PageInfo = PageInfo {
    title: "Component gallery",
    short: "All Components",
    description: "All 91 standard components, each listed in the sidebar and painted by its genuine unbound-widgets emitter with a real hit region, semantic node, and interactive knobs.",
    crates: &["unbound-widgets"],
    docs: "docs/architecture/widgets.md",
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
        | WidgetCatalogId::Badge
        | WidgetCatalogId::Chip
        | WidgetCatalogId::Checkbox
        | WidgetCatalogId::Switch
        | WidgetCatalogId::Radio
        | WidgetCatalogId::Segmented
        | WidgetCatalogId::Stepper
        | WidgetCatalogId::Rating
        | WidgetCatalogId::TagInput
        | WidgetCatalogId::PinInput => KNOBS_ACTION,

        WidgetCatalogId::Progress
        | WidgetCatalogId::Slider
        | WidgetCatalogId::NumberInput
        | WidgetCatalogId::Spinner
        | WidgetCatalogId::AspectRatioBox => KNOBS_NUMERIC,

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
        | WidgetCatalogId::VirtualViews
        | WidgetCatalogId::Masonry
        | WidgetCatalogId::VirtualMasonry
        | WidgetCatalogId::AvatarGroup => KNOBS_DATA,

        WidgetCatalogId::Alert
        | WidgetCatalogId::AlertDialog
        | WidgetCatalogId::Banner
        | WidgetCatalogId::Callout
        | WidgetCatalogId::Popconfirm
        | WidgetCatalogId::NotificationCenter
        | WidgetCatalogId::Toast
        | WidgetCatalogId::Modal
        | WidgetCatalogId::Drawer
        | WidgetCatalogId::Popover
        | WidgetCatalogId::Tooltip
        | WidgetCatalogId::HoverCard
        | WidgetCatalogId::Tour => KNOBS_FEEDBACK,

        WidgetCatalogId::Breadcrumb
        | WidgetCatalogId::NavigationRail
        | WidgetCatalogId::Pagination
        | WidgetCatalogId::Tabs
        | WidgetCatalogId::TreeView
        | WidgetCatalogId::Menu
        | WidgetCatalogId::Menubar
        | WidgetCatalogId::ContextMenu
        | WidgetCatalogId::CommandPalette
        | WidgetCatalogId::Toolbar
        | WidgetCatalogId::CanvasToolbar
        | WidgetCatalogId::StatusBar => KNOBS_NAV,

        WidgetCatalogId::Containers
        | WidgetCatalogId::SplitPane
        | WidgetCatalogId::MultiSplit
        | WidgetCatalogId::Dock
        | WidgetCatalogId::Accordion
        | WidgetCatalogId::Collapsible
        | WidgetCatalogId::EmptyState
        | WidgetCatalogId::Watermark
        | WidgetCatalogId::LoadingOverlay
        | WidgetCatalogId::NinePatch
        | WidgetCatalogId::Stack
        | WidgetCatalogId::Wrap
        | WidgetCatalogId::PropertyGrid
        | WidgetCatalogId::Divider => KNOBS_LAYOUT,

        WidgetCatalogId::TextInput
        | WidgetCatalogId::TextArea
        | WidgetCatalogId::AutoComplete
        | WidgetCatalogId::Cascader
        | WidgetCatalogId::Combobox
        | WidgetCatalogId::DatePicker
        | WidgetCatalogId::DateRangePicker
        | WidgetCatalogId::TimePicker
        | WidgetCatalogId::HotkeyInput
        | WidgetCatalogId::ColorPicker
        | WidgetCatalogId::GradientEditor
        | WidgetCatalogId::CurveEditor
        | WidgetCatalogId::Form
        | WidgetCatalogId::Transfer
        | WidgetCatalogId::FileDrop
        | WidgetCatalogId::Select => KNOBS_INPUT,

        WidgetCatalogId::NodeGraph
        | WidgetCatalogId::Minimap
        | WidgetCatalogId::Ruler
        | WidgetCatalogId::Gizmo
        | WidgetCatalogId::Image
        | WidgetCatalogId::Icon
        | WidgetCatalogId::Skeleton
        | WidgetCatalogId::Carousel
        | WidgetCatalogId::Avatar
        | WidgetCatalogId::RichText
        | WidgetCatalogId::Scroll
        | WidgetCatalogId::Overlay => KNOBS_VISUAL,
    }
}

/// 任意组件的惯用声明式 Unbound UI 代码。
#[must_use]
pub const fn declarative_code_for_id(id: WidgetCatalogId) -> &'static str {
    match id {
        WidgetCatalogId::Accordion => "view.accordion(ui_key!(\"acc\"), AccordionDesc::new(rect, sections));",
        WidgetCatalogId::Alert => "view.alert(ui_key!(\"alt\"), AlertDesc::message(\"Notice\").severity(Severity::Info));",
        WidgetCatalogId::AlertDialog => "view.add(ui_key!(\"dlg\"), AlertDialogDesc::new(rect, \"Confirm Action\"));",
        WidgetCatalogId::AspectRatioBox => "view.add(ui_key!(\"box\"), AspectRatioBoxDesc::new(rect, 16.0 / 9.0));",
        WidgetCatalogId::AutoComplete => "view.add(ui_key!(\"ac\"), AutoCompleteDesc::new(rect, suggestions));",
        WidgetCatalogId::Avatar => "view.add(ui_key!(\"ava\"), AvatarDesc::initials(rect, \"JD\"));",
        WidgetCatalogId::AvatarGroup => "view.add(ui_key!(\"grp\"), AvatarGroupDesc::new(rect, avatars));",
        WidgetCatalogId::Badge => "view.badge(ui_key!(\"bdg\"), BadgeDesc::label(\"New\"));",
        WidgetCatalogId::Banner => "view.add(ui_key!(\"bnr\"), BannerDesc::new(rect, \"System Banner\"));",
        WidgetCatalogId::Breadcrumb => "view.add(ui_key!(\"bc\"), BreadcrumbDesc::new(rect, crumbs));",
        WidgetCatalogId::Button => "view.button(ui_key!(\"btn\"), ButtonDesc::label(\"Click Me\").variant(ButtonVariant::Primary));",
        WidgetCatalogId::Callout => "view.callout(ui_key!(\"cal\"), CalloutDesc::new(rect, \"Tip\", \"GPU memory is optimal.\"));",
        WidgetCatalogId::CanvasToolbar => "view.add(ui_key!(\"tb\"), CanvasToolbarDesc::new(rect));",
        WidgetCatalogId::Carousel => "view.add(ui_key!(\"car\"), CarouselDesc::new(rect, slides));",
        WidgetCatalogId::Cascader => "view.add(ui_key!(\"cas\"), CascaderDesc::new(rect, tree));",
        WidgetCatalogId::Chart => "view.add(ui_key!(\"chr\"), ChartDesc::new(rect, series));",
        WidgetCatalogId::Checkbox => "view.checkbox(ui_key!(\"chk\"), CheckboxDesc::labeled(\"Accept terms\", true));",
        WidgetCatalogId::Chip => "view.badge(ui_key!(\"chp\"), BadgeDesc::label(\"Filter\"));",
        WidgetCatalogId::CodeBlock => "view.add(ui_key!(\"cb\"), CodeBlockDesc::new(rect, code, \"rust\"));",
        WidgetCatalogId::CodeEditor => "view.add(ui_key!(\"ce\"), CodeEditorDesc::new(rect, buffer));",
        WidgetCatalogId::Collapsible => "view.add(ui_key!(\"col\"), CollapsibleDesc::new(rect, \"Details\"));",
        WidgetCatalogId::ColorPicker => "view.add(ui_key!(\"cp\"), ColorPickerDesc::new(rect));",
        WidgetCatalogId::Combobox => "view.add(ui_key!(\"cmb\"), ComboboxDesc::new(rect, items));",
        WidgetCatalogId::CommandPalette => "view.add(ui_key!(\"cmd\"), CommandPaletteDesc::new(rect));",
        WidgetCatalogId::Containers => "view.card(ui_key!(\"crd\"), CardDesc::default(), |card| { /* content */ });",
        WidgetCatalogId::ContextMenu => "view.add(ui_key!(\"ctx\"), ContextMenuDesc::new(rect, menu));",
        WidgetCatalogId::CurveEditor => "view.add(ui_key!(\"crv\"), CurveEditorDesc::new(rect));",
        WidgetCatalogId::DataGrid => "view.add(ui_key!(\"dg\"), DataGridDesc::new(rect, grid_model));",
        WidgetCatalogId::DatePicker => "view.add(ui_key!(\"dp\"), DatePickerDesc::new(rect));",
        WidgetCatalogId::DateRangePicker => "view.add(ui_key!(\"drp\"), DateRangePickerDesc::new(rect));",
        WidgetCatalogId::Descriptions => "view.descriptions(ui_key!(\"desc\"), DescriptionsDesc::new(rect, items));",
        WidgetCatalogId::Divider => "view.divider(ui_key!(\"div\"), DividerDesc::rule());",
        WidgetCatalogId::Dock => "view.add(ui_key!(\"dck\"), DockDesc::new(rect, dock_tree));",
        WidgetCatalogId::Drawer => "view.add(ui_key!(\"drw\"), DrawerDesc::new(rect, DrawerSide::Right));",
        WidgetCatalogId::EmptyState => "view.add(ui_key!(\"emp\"), EmptyStateDesc::new(rect, \"No Items\"));",
        WidgetCatalogId::FileDrop => "view.add(ui_key!(\"fd\"), FileDropDesc::new(rect));",
        WidgetCatalogId::Form => "view.add(ui_key!(\"frm\"), FormDesc::new(rect, fields));",
        WidgetCatalogId::Gizmo => "view.add(ui_key!(\"gz\"), GizmoDesc::new(rect));",
        WidgetCatalogId::GradientEditor => "view.add(ui_key!(\"ge\"), GradientEditorDesc::new(rect));",
        WidgetCatalogId::HotkeyInput => "view.add(ui_key!(\"hk\"), HotkeyInputDesc::new(rect));",
        WidgetCatalogId::HoverCard => "view.add(ui_key!(\"hc\"), HoverCardDesc::new(rect));",
        WidgetCatalogId::Icon => "view.add(ui_key!(\"ic\"), IconDesc::new(rect, icon_name));",
        WidgetCatalogId::Image => "view.add(ui_key!(\"img\"), ImageDesc::new(rect, asset_id));",
        WidgetCatalogId::LoadingOverlay => "view.add(ui_key!(\"lo\"), LoadingOverlayDesc::new(rect));",
        WidgetCatalogId::Masonry => "view.add(ui_key!(\"msn\"), MasonryDesc::new(rect, items));",
        WidgetCatalogId::Menu => "view.add(ui_key!(\"mnu\"), MenuDesc::new(rect, items));",
        WidgetCatalogId::Menubar => "view.add(ui_key!(\"mb\"), MenuBarDesc::new(rect, menus));",
        WidgetCatalogId::Minimap => "view.add(ui_key!(\"mm\"), MinimapDesc::new(rect));",
        WidgetCatalogId::Modal => "view.add(ui_key!(\"mdl\"), ModalDesc::new(rect, \"Modal Dialog\"));",
        WidgetCatalogId::MultiSplit => "view.multi_split(ui_key!(\"msplit\"), MultiSplitDesc::new(rect, panes));",
        WidgetCatalogId::NavigationRail => "view.add(ui_key!(\"nr\"), NavigationRailDesc::new(rect, items));",
        WidgetCatalogId::NinePatch => "view.add(ui_key!(\"np\"), NinePatchDesc::new(rect, asset_id));",
        WidgetCatalogId::NodeGraph => "view.add(ui_key!(\"ng\"), NodeGraphDesc::new(rect, graph));",
        WidgetCatalogId::NotificationCenter => "view.add(ui_key!(\"nc\"), NotificationCenterDesc::new(rect));",
        WidgetCatalogId::NumberInput => "view.add(ui_key!(\"ni\"), NumberInputDesc::new(rect).min(0.0).max(100.0));",
        WidgetCatalogId::Overlay => "view.add(ui_key!(\"ov\"), OverlayDesc::new(rect));",
        WidgetCatalogId::Pagination => "view.add(ui_key!(\"pg\"), PaginationDesc::new(rect, 100).page_size(10));",
        WidgetCatalogId::PinInput => "view.add(ui_key!(\"pin\"), PinInputDesc::new(rect, 6));",
        WidgetCatalogId::Popconfirm => "view.add(ui_key!(\"pcf\"), PopconfirmDesc::new(rect, \"Delete item?\"));",
        WidgetCatalogId::Popover => "view.add(ui_key!(\"pop\"), PopoverDesc::new(rect));",
        WidgetCatalogId::Progress => "view.progress(ui_key!(\"prg\"), ProgressDesc::bar(0.75));",
        WidgetCatalogId::PropertyGrid => "view.add(ui_key!(\"pgd\"), PropertyGridDesc::new(rect, props));",
        WidgetCatalogId::Radio => "view.add(ui_key!(\"rad\"), RadioGroupDesc::new(rect, options));",
        WidgetCatalogId::Rating => "view.add(ui_key!(\"rtg\"), RatingDesc::new(rect, 5).value(4.5));",
        WidgetCatalogId::RichText => "view.add(ui_key!(\"rt\"), RichTextDesc::new(rect, markdown));",
        WidgetCatalogId::Ruler => "view.add(ui_key!(\"rlr\"), RulerDesc::new(rect));",
        WidgetCatalogId::Scroll => "view.scroll_view(ui_key!(\"sc\"), true, |ui| { /* content */ });",
        WidgetCatalogId::Segmented => "view.add(ui_key!(\"seg\"), SegmentedDesc::new(rect, options));",
        WidgetCatalogId::Select => "view.select(ui_key!(\"sel\"), SelectDesc::options(&[\"A\", \"B\"]).selected(0));",
        WidgetCatalogId::Skeleton => "view.add(ui_key!(\"skl\"), SkeletonDesc::new(rect));",
        WidgetCatalogId::Slider => "view.slider(ui_key!(\"sld\"), SliderDesc::range(0.0..=100.0).value(50.0));",
        WidgetCatalogId::Spinner => "view.add(ui_key!(\"spn\"), SpinnerDesc::new(rect));",
        WidgetCatalogId::SplitButton => "view.split_button(ui_key!(\"sbtn\"), SplitButtonDesc::new(rect, \"Run\", \"Build & Run\"));",
        WidgetCatalogId::SplitPane => "view.split_pane(ui_key!(\"sp\"), true, |left| {}, |right| {});",
        WidgetCatalogId::Stack => "view.add(ui_key!(\"stk\"), StackDesc::new(rect));",
        WidgetCatalogId::StatusBar => "view.add(ui_key!(\"sb\"), StatusBarDesc::new(rect, segments));",
        WidgetCatalogId::Stepper => "view.add(ui_key!(\"stp\"), StepperDesc::new(rect, steps));",
        WidgetCatalogId::Switch => "view.switch(ui_key!(\"sw\"), SwitchDesc::on(true));",
        WidgetCatalogId::Table => "view.table(ui_key!(\"tbl\"), TableDesc::columns(cols).rows(rows));",
        WidgetCatalogId::Tabs => "view.add(ui_key!(\"tb\"), TabsDesc::new(rect, tabs));",
        WidgetCatalogId::TagInput => "view.add(ui_key!(\"tag\"), TagInputDesc::new(rect, tags));",
        WidgetCatalogId::TerminalView => "view.terminal_view(ui_key!(\"term\"), TerminalViewDesc::new(rect, lines));",
        WidgetCatalogId::TextArea => "view.add(ui_key!(\"ta\"), TextAreaDesc::new(rect));",
        WidgetCatalogId::TextInput => "view.text_input(ui_key!(\"txt\"), TextInputDesc::text(\"\").placeholder(\"Search...\"));",
        WidgetCatalogId::TimePicker => "view.add(ui_key!(\"tp\"), TimePickerDesc::new(rect));",
        WidgetCatalogId::Timeline => "view.add(ui_key!(\"tl\"), TimelineDesc::new(rect, events));",
        WidgetCatalogId::TimelineTrack => "view.add(ui_key!(\"tlt\"), TimelineTrackDesc::new(rect));",
        WidgetCatalogId::Toast => "view.add(ui_key!(\"tst\"), ToastDesc::new(rect, \"Notification\"));",
        WidgetCatalogId::ToggleButton => "view.toggle_button(ui_key!(\"tbtn\"), ToggleButtonDesc::new(rect, \"Pin\", true));",
        WidgetCatalogId::Toolbar => "view.add(ui_key!(\"tbr\"), ToolbarDesc::new(rect, tools));",
        WidgetCatalogId::Tooltip => "view.tooltip(ui_key!(\"ttp\"), TooltipDesc::labeled(anchor, viewport, \"Auto-flipping hint\"));",
        WidgetCatalogId::Tour => "view.add(ui_key!(\"tur\"), TourDesc::new(rect, step));",
        WidgetCatalogId::Transfer => "view.add(ui_key!(\"tr\"), TransferDesc::new(rect, items));",
        WidgetCatalogId::TreeTable => "view.tree_table(ui_key!(\"ttbl\"), TreeTableDesc::new(rect, rows));",
        WidgetCatalogId::TreeView => "view.add(ui_key!(\"tv\"), TreeViewDesc::new(rect, tree_nodes));",
        WidgetCatalogId::VirtualViews => "view.add(ui_key!(\"vv\"), VirtualViewsDesc::new(rect));",
        WidgetCatalogId::VirtualMasonry => "view.virtual_masonry(ui_key!(\"vm\"), VirtualMasonryDesc::new(rect, 4, 1000));",
        WidgetCatalogId::Watermark => "view.add(ui_key!(\"wm\"), WatermarkDesc::new(rect, \"CONFIDENTIAL\"));",
        WidgetCatalogId::Wrap => "view.add(ui_key!(\"wrp\"), WrapDesc::new(rect));",
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
    };
}

define_showcase_components! {
    Accordion => "Accordion", "Collapsible sections with a stable header identity.", 320.0,
    Alert => "Alert", "Severity banner with an optional dismiss affordance.", 240.0,
    AlertDialog => "Alert dialog", "A destructive confirmation placed in the viewport.", 320.0,
    Avatar => "Avatar", "Initials avatar with a presence indicator.", 240.0,
    Badge => "Badge / chip", "A closable chip — the interactive half of the badge module.", 240.0,
    Breadcrumb => "Breadcrumb", "Hierarchical trail with per-crumb hit slots.", 240.0,
    Button => "Button", "The flat button: shadow, surface, label, border, focus ring.", 240.0,
    CanvasToolbar => "Canvas toolbar", "Floating zoom and tool controls for the infinite canvas.", 280.0,
    Chart => "Chart", "Line series with axes and a hover crosshair.", 320.0,
    Checkbox => "Checkbox", "A checked box with a shaped mark and label.", 240.0,
    CodeBlock => "Code block", "Syntax-highlighted source with a copy affordance.", 300.0,
    ColorPicker => "Color picker", "HSVA field with HEX / OKLCH readouts.", 360.0,
    Combobox => "Combobox", "Editable trigger with a filtered suggestion list.", 260.0,
    CommandPalette => "Command palette", "Fuzzy command search with shortcuts.", 360.0,
    Containers => "Card container", "Header / body / footer surface from the containers module.", 300.0,
    ContextMenu => "Context menu", "Right-click placement state machine over the menu emitter.", 300.0,
    CurveEditor => "Curve editor", "Editable bezier curve with draggable handles.", 320.0,
    DatePicker => "Date picker", "Month calendar with single-date selection.", 360.0,
    Divider => "Divider", "A hairline separator with an optional label.", 220.0,
    Dock => "Dock", "Tabbed panes with drag-to-dock layout.", 340.0,
    Drawer => "Drawer", "A side drawer with a scrim over the viewport.", 340.0,
    FileDrop => "File drop", "A drop target with idle and drag-over states.", 280.0,
    Form => "Form", "Field label with required marker and validation message.", 280.0,
    Gizmo => "Gizmo", "Eight-handle transform gizmo with a size bubble.", 300.0,
    HoverCard => "Hover card", "A rich preview anchored to a trigger.", 280.0,
    Image => "Image", "A textured quad bound to an atlas layer.", 280.0,
    Menu => "Menu", "Action rows, separators and shortcuts.", 300.0,
    Menubar => "Menubar", "Top-level menu items with keyboard traversal.", 240.0,
    Minimap => "Minimap", "A viewport thumbnail over the world bounds.", 300.0,
    Modal => "Modal", "A centered dialog with a focus trap and scrim.", 360.0,
    NodeGraph => "Node graph", "Ported nodes connected by wires.", 340.0,
    NumberInput => "Number input", "Stepper with expression evaluation.", 240.0,
    Overlay => "Overlay stack", "Pure placement bookkeeping — emits no primitives of its own.", 220.0,
    Popover => "Popover", "An anchored popup with a directional arrow.", 280.0,
    Progress => "Progress", "A determinate bar driven by a value.", 220.0,
    PropertyGrid => "Property grid", "Sectioned inspector rows with reset points.", 320.0,
    Radio => "Radio", "A single-choice row in a group.", 240.0,
    RichText => "Rich text", "Markdown parsed into styled runs.", 320.0,
    Ruler => "Ruler", "Canvas rulers with draggable guides.", 320.0,
    Scroll => "Scroll view", "A scrollable viewport with a real scrollbar.", 320.0,
    Segmented => "Segmented", "An unselected slot of a single-choice pill.", 240.0,
    Select => "Select", "A closed select trigger with a value.", 240.0,
    Skeleton => "Skeleton", "An animated loading placeholder.", 220.0,
    Slider => "Slider", "A continuous track with a thumb and step.", 240.0,
    SplitPane => "Split pane", "A draggable divider between two panes.", 320.0,
    Switch => "Switch", "An on/off toggle with a shaped label.", 240.0,
    Table => "Table", "Column headers and a virtual row window.", 320.0,
    Tabs => "Tabs", "An inactive tab slot with close affordance.", 240.0,
    TagInput => "Tag input", "Removable tags with an inline editor.", 280.0,
    TextArea => "Text area", "Multi-line editing with line numbers.", 300.0,
    TextInput => "Text input", "A single-line field with caret and selection.", 240.0,
    Toast => "Toast", "A spring-in notification with a countdown.", 280.0,
    Toolbar => "Toolbar", "Button and toggle items in a row.", 240.0,
    Tooltip => "Tooltip", "A delayed, smart-flipping in-window hint.", 260.0,
    TreeView => "Tree view", "An expandable row with keyboard navigation.", 320.0,
    VirtualViews => "Virtual row", "A Fenwick-backed variable-height virtual row.", 240.0,
    Collapsible => "Collapsible", "Single-section disclosure panel with smooth spring progress.", 240.0,
    Masonry => "Masonry grid", "Shortest-column adaptive waterfall grid and aspect-ratio box.", 280.0,
    NavigationRail => "Navigation rail", "Collapsible sidebar navigation rail with badges.", 280.0,
    Pagination => "Pagination", "Page buttons with ellipsis folding and page-size metadata.", 220.0,
    Stepper => "Stepper", "Horizontal or vertical multi-step progress indicator.", 240.0,
    AutoComplete => "AutoComplete", "Prefix-filtered input with live suggestion badge.", 220.0,
    Banner => "Banner", "Full-width top global status and recovery banner.", 220.0,
    Cascader => "Cascader", "Multi-level hierarchical option selector.", 220.0,
    DateRangePicker => "Date range picker", "Dual-month calendar range selector with presets.", 260.0,
    HotkeyInput => "Hotkey input", "Live KeyChord shortcut recorder with conflict status.", 220.0,
    NotificationCenter => "Notification center", "Drawer aggregating toast history and unread status.", 260.0,
    PinInput => "PIN input", "Segmented verification code input with paste splitting.", 220.0,
    Popconfirm => "Popconfirm", "Anchored confirmation bubble for destructive actions.", 240.0,
    Rating => "Rating", "Five-star rating control with half-star precision.", 220.0,
    Spinner => "Spinner", "Continuous loading spinner and overlay lock scrim.", 220.0,
    TimePicker => "Time picker", "Hour, minute, and second stepping time selector.", 220.0,
    Transfer => "Transfer", "Dual-column item transfer list with batch move.", 260.0,
    Carousel => "Carousel", "Spring-snapping slide deck with indicator dots.", 240.0,
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
    Icon => "Icon", "Sized vector icon with rounded corners.", 220.0,
    LoadingOverlay => "Loading overlay", "Full-card scrim with centred spinning loader.", 260.0,
    NinePatch => "Nine-patch", "Nine-slice scaling image with fixed border corners.", 260.0,
    Stack => "Stack", "Layered container stacking children in z-order.", 260.0,
    StatusBar => "Status bar", "Desktop status strip with aligned segments.", 220.0,
    Wrap => "Wrap layout", "Flow layout wrapping items into lines.", 260.0,
    SplitButton => "Split button", "Primary button paired with a dropdown trigger menu.", 220.0,
    ToggleButton => "Toggle button", "Two-state push button with pressed state lock.", 220.0,
    TreeTable => "Tree table", "Hierarchical data table with expandable rows and columns.", 280.0,
    MultiSplit => "Multi-split", "Resizable multi-pane container with draggable splitters.", 280.0,
    Descriptions => "Descriptions", "Label-value summary display for structured properties.", 260.0,
    Callout => "Callout", "High-visibility information or warning callout box.", 220.0,
    VirtualMasonry => "Virtual masonry", "Virtualized staggered column layout for heterogeneous items.", 280.0,
    TerminalView => "Terminal view", "ANSI-styled virtualized terminal console log viewer.", 280.0,
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

fn semantic_label_for_id(
    id: WidgetCatalogId,
    sandbox: &crate::state::ComponentSandboxState,
) -> String {
    match id {
        WidgetCatalogId::Button => format!("Button ({})", sandbox.get_clicks(id)),
        WidgetCatalogId::Badge => "Badge: Active".to_string(),
        WidgetCatalogId::Chip => "Chip: Selected".to_string(),
        WidgetCatalogId::Checkbox => {
            if sandbox.get_toggle(id) {
                "Checked".to_string()
            } else {
                "Unchecked".to_string()
            }
        }
        WidgetCatalogId::Switch => {
            if sandbox.get_toggle(id) {
                "Enabled".to_string()
            } else {
                "Disabled".to_string()
            }
        }
        WidgetCatalogId::TextInput => sandbox.get_text(id, "Search query...").to_string(),
        WidgetCatalogId::TextArea => sandbox
            .get_text(id, "Multi-line text editor\nLine 2")
            .to_string(),
        WidgetCatalogId::Slider => format!("Slider: {:.0}%", sandbox.get_slider(id, 50.0)),
        WidgetCatalogId::Progress => format!("Progress: {:.0}%", sandbox.get_slider(id, 65.0)),
        WidgetCatalogId::Table => "Table: 5 columns, 100 rows".to_string(),
        WidgetCatalogId::DataGrid => "DataGrid: Virtualized grid".to_string(),
        WidgetCatalogId::Chart => "Series: 120 FPS".to_string(),
        WidgetCatalogId::Select => format!("Option {}", sandbox.get_selected(id) + 1),
        WidgetCatalogId::Tabs => format!("Tab {}", sandbox.get_selected(id) + 1),
        WidgetCatalogId::Accordion => format!("Section {}", sandbox.get_selected(id) + 1),
        WidgetCatalogId::SplitButton => format!("SplitButton ({})", sandbox.get_clicks(id)),
        WidgetCatalogId::ToggleButton => {
            if sandbox.get_toggle(id) {
                "Active (ON)".to_string()
            } else {
                "Inactive (OFF)".to_string()
            }
        }
        WidgetCatalogId::TreeTable => "TreeTable: 12 nodes".to_string(),
        WidgetCatalogId::MultiSplit => "MultiSplit: 3 panes".to_string(),
        WidgetCatalogId::Descriptions => "Descriptions: 6 items".to_string(),
        WidgetCatalogId::Callout => "Callout: Tip".to_string(),
        WidgetCatalogId::VirtualMasonry => "VirtualMasonry: 1000 items".to_string(),
        WidgetCatalogId::TerminalView => "Terminal: Ready".to_string(),
        WidgetCatalogId::Tooltip => "Tooltip: In-window auto-flipped".to_string(),
        _ => id.module_name().to_string(),
    }
}

/// 使用真实发射器渲染单个组件卡片。
pub fn component(ctx: &mut DemoCtx<'_, '_>, id: WidgetCatalogId) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;

    let clicks = ctx.state.sandbox.get_clicks(id);
    let title_text = if clicks > 0 {
        format!("{} • {} clicks", id.module_name(), clicks)
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
    let stage = Rect::new(
        rect.origin.x + 24.0,
        rect.origin.y + 42.0,
        (rect.size.x - 48.0).max(40.0),
        (rect.size.y - 66.0).max(40.0),
    );
    let node = ctx.part(1);
    let mut state = ctx.canvas.control_state(node, false, false);
    if ctx.knob_on(0) {
        state.disabled = true;
    }

    let label_str = semantic_label_for_id(id, &ctx.state.sandbox);
    let mut glyphs = ctx.canvas.shape(
        &label_str,
        Vec2::new(stage.origin.x + 8.0, stage.origin.y + 8.0),
        13.0,
    );
    let output = emit_widget(id, node, state, stage, &mut glyphs);
    ctx.canvas.widget_output(id, node, &output);
}

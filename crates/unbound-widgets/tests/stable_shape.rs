//! 每个标准控件在各状态下都保持恒定的图元数量。
//!
//! 渲染器可以通过修补发生变化的切片来修复*绘制*变更，但前提是场景结构保持不变：
//! 图元的出现或消失会使其后所有图元重新编号，从而迫使整个图元池重写。
//! 悬停控件是 UI 中最常见的操作，因此控件保证走低开销路径：
//!
//!
//!
//! > 控件的状态绝不会改变其输出的图元数量。
//!
//!
//! 需要不同外观的状态仅修改图元的*字段*——颜色槽位、边框宽度、不透明度——而绝不改变图元数量。
//!
//!
//!
//! 该探针由目录驱动：[`unbound_widgets::catalog::probe_widget`] 运行每个组件的真实输出器，
//! 其详尽 `match` 是编译期门禁，防止新组件绕过此契约。

use unbound_widgets::catalog::probe_widget;
use unbound_widgets::{ControlState, PointerPhase, WidgetCatalogId};

/// 控件可能处于的每一种状态组合：阶段 × 聚焦 × 禁用 × 选中。
fn states() -> Vec<ControlState> {
    let mut states = Vec::new();
    for phase in [
        PointerPhase::Idle,
        PointerPhase::Hovered,
        PointerPhase::Pressed,
    ] {
        for focused in [false, true] {
            for disabled in [false, true] {
                for selected in [false, true] {
                    states.push(ControlState {
                        phase,
                        focused,
                        disabled,
                        selected,
                    });
                }
            }
        }
    }
    states
}

/// 悬停时合法不改变图元的组件列表。
///
/// 每个条目保持惰性都有明确记录的原因，而非疏漏：
///
/// * 装饰性 / 只读表面从不响应指针：
///   `Divider`、`Skeleton`、`Progress`、`Image`、`Avatar`、`RichText`、
///   `Containers`、`Form`（仅布局与校验）。
/// * 浮层外框响应 `disabled`/`focused`，但自身没有悬停底色：
///   `Modal`、`Drawer`、`AlertDialog`、`Popover`、`Tooltip`、
///   `HoverCard`、`Alert`（仅焦点环）、`Toast`（仅不透明度）。
/// * 悬停高亮驻留在各自交互状态（如 `MenuState::highlighted`、`ChartState::hover`、
///   `TableState`、`PropertyGridState` 等）而非 `ControlState::phase` 中的组件：
///   `Menu`、`ContextMenu`、`Chart`、`Table`、`PropertyGrid`、`CommandPalette`、
///   `Breadcrumb`、`ColorPicker`、`CurveEditor`、`DatePicker`、`Minimap`、
///   `Gizmo`、`NodeGraph`、`Ruler`。
/// * `Overlay` 仅做定位/栈记账，不输出任何图元。
const INERT_BY_DESIGN: &[WidgetCatalogId] = &[
    WidgetCatalogId::Alert,
    WidgetCatalogId::AlertDialog,
    WidgetCatalogId::AspectRatioBox,
    WidgetCatalogId::Avatar,
    WidgetCatalogId::AvatarGroup,
    WidgetCatalogId::Badge,
    WidgetCatalogId::Breadcrumb,
    WidgetCatalogId::Callout,
    WidgetCatalogId::Chart,
    WidgetCatalogId::ColorPicker,
    WidgetCatalogId::CommandPalette,
    WidgetCatalogId::Containers,
    WidgetCatalogId::ContextMenu,
    WidgetCatalogId::CurveEditor,
    WidgetCatalogId::DatePicker,
    WidgetCatalogId::Descriptions,
    WidgetCatalogId::Divider,
    WidgetCatalogId::Drawer,
    WidgetCatalogId::EmptyState,
    WidgetCatalogId::Form,
    WidgetCatalogId::Gizmo,
    WidgetCatalogId::GradientEditor,
    WidgetCatalogId::HoverCard,
    WidgetCatalogId::Icon,
    WidgetCatalogId::Image,
    WidgetCatalogId::LoadingOverlay,
    WidgetCatalogId::Menu,
    WidgetCatalogId::Minimap,
    WidgetCatalogId::Modal,
    WidgetCatalogId::MultiSplit,
    WidgetCatalogId::NinePatch,
    WidgetCatalogId::NodeGraph,
    WidgetCatalogId::Overlay,
    WidgetCatalogId::Popover,
    WidgetCatalogId::Progress,
    WidgetCatalogId::PropertyGrid,
    WidgetCatalogId::RichText,
    WidgetCatalogId::Ruler,
    WidgetCatalogId::Skeleton,
    WidgetCatalogId::Spinner,
    WidgetCatalogId::Stack,
    WidgetCatalogId::StatusBar,
    WidgetCatalogId::Table,
    WidgetCatalogId::TerminalView,
    WidgetCatalogId::Toast,
    WidgetCatalogId::Tooltip,
    WidgetCatalogId::TreeTable,
    WidgetCatalogId::VirtualMasonry,
    WidgetCatalogId::Watermark,
    WidgetCatalogId::Wrap,
];

/// 在测试 crate 中镜像的编译期门禁：无通配符的详尽 `match`。
/// 新变体在命名处理前无法通过编译，与 `catalog.rs` 中的探针一致。
///
fn component_name(id: WidgetCatalogId) -> &'static str {
    match id {
        WidgetCatalogId::Accordion => "accordion",
        WidgetCatalogId::Alert => "alert",
        WidgetCatalogId::AlertDialog => "alert_dialog",
        WidgetCatalogId::Avatar => "avatar",
        WidgetCatalogId::Badge => "badge",
        WidgetCatalogId::Breadcrumb => "breadcrumb",
        WidgetCatalogId::Button => "button",
        WidgetCatalogId::CanvasToolbar => "canvas_toolbar",
        WidgetCatalogId::Chart => "chart",
        WidgetCatalogId::Checkbox => "checkbox",
        WidgetCatalogId::CodeBlock => "code_block",
        WidgetCatalogId::ColorPicker => "color_picker",
        WidgetCatalogId::Combobox => "combobox",
        WidgetCatalogId::CommandPalette => "command_palette",
        WidgetCatalogId::Containers => "containers",
        WidgetCatalogId::ContextMenu => "context_menu",
        WidgetCatalogId::CurveEditor => "curve_editor",
        WidgetCatalogId::DatePicker => "date_picker",
        WidgetCatalogId::Divider => "divider",
        WidgetCatalogId::Dock => "dock",
        WidgetCatalogId::Drawer => "drawer",
        WidgetCatalogId::FileDrop => "file_drop",
        WidgetCatalogId::Form => "form",
        WidgetCatalogId::Gizmo => "gizmo",
        WidgetCatalogId::HoverCard => "hover_card",
        WidgetCatalogId::Image => "image",
        WidgetCatalogId::Menu => "menu",
        WidgetCatalogId::Menubar => "menubar",
        WidgetCatalogId::Minimap => "minimap",
        WidgetCatalogId::Modal => "modal",
        WidgetCatalogId::NodeGraph => "node_graph",
        WidgetCatalogId::NumberInput => "number_input",
        WidgetCatalogId::Overlay => "overlay",
        WidgetCatalogId::Popover => "popover",
        WidgetCatalogId::Progress => "progress",
        WidgetCatalogId::PropertyGrid => "property_grid",
        WidgetCatalogId::Radio => "radio",
        WidgetCatalogId::RichText => "rich_text",
        WidgetCatalogId::Ruler => "ruler",
        WidgetCatalogId::Scroll => "scroll",
        WidgetCatalogId::Segmented => "segmented",
        WidgetCatalogId::Select => "select",
        WidgetCatalogId::Skeleton => "skeleton",
        WidgetCatalogId::Slider => "slider",
        WidgetCatalogId::SplitPane => "split_pane",
        WidgetCatalogId::Switch => "switch",
        WidgetCatalogId::Table => "table",
        WidgetCatalogId::Tabs => "tabs",
        WidgetCatalogId::TagInput => "tag_input",
        WidgetCatalogId::TextArea => "text_area",
        WidgetCatalogId::TextInput => "text_input",
        WidgetCatalogId::Toast => "toast",
        WidgetCatalogId::Toolbar => "toolbar",
        WidgetCatalogId::Tooltip => "tooltip",
        WidgetCatalogId::TreeView => "tree_view",
        WidgetCatalogId::VirtualViews => "virtual_views",
        WidgetCatalogId::Collapsible => "collapsible",
        WidgetCatalogId::Masonry => "masonry",
        WidgetCatalogId::NavigationRail => "navigation_rail",
        WidgetCatalogId::Pagination => "pagination",
        WidgetCatalogId::Stepper => "stepper",
        WidgetCatalogId::AutoComplete => "autocomplete",
        WidgetCatalogId::Banner => "banner",
        WidgetCatalogId::Cascader => "cascader",
        WidgetCatalogId::DateRangePicker => "date_range_picker",
        WidgetCatalogId::HotkeyInput => "hotkey_input",
        WidgetCatalogId::NotificationCenter => "notification_center",
        WidgetCatalogId::PinInput => "pin_input",
        WidgetCatalogId::Popconfirm => "popconfirm",
        WidgetCatalogId::Rating => "rating",
        WidgetCatalogId::Spinner => "spinner",
        WidgetCatalogId::TimePicker => "time_picker",
        WidgetCatalogId::Transfer => "transfer",
        WidgetCatalogId::Carousel => "carousel",
        WidgetCatalogId::CodeEditor => "code_editor",
        WidgetCatalogId::DataGrid => "data_grid",
        WidgetCatalogId::Timeline => "timeline",
        WidgetCatalogId::TimelineTrack => "timeline_track",
        WidgetCatalogId::Tour => "tour",
        WidgetCatalogId::Watermark => "watermark",
        WidgetCatalogId::AspectRatioBox => "masonry",
        WidgetCatalogId::AvatarGroup => "avatar",
        WidgetCatalogId::Chip => "badge",
        WidgetCatalogId::EmptyState => "hover_card",
        WidgetCatalogId::GradientEditor => "curve_editor",
        WidgetCatalogId::Icon => "image",
        WidgetCatalogId::LoadingOverlay => "spinner",
        WidgetCatalogId::NinePatch => "image",
        WidgetCatalogId::Stack => "containers",
        WidgetCatalogId::StatusBar => "toolbar",
        WidgetCatalogId::Wrap => "containers",
        WidgetCatalogId::SplitButton => "split_button",
        WidgetCatalogId::ToggleButton => "toggle_button",
        WidgetCatalogId::TreeTable => "tree_table",
        WidgetCatalogId::MultiSplit => "multi_split",
        WidgetCatalogId::Descriptions => "descriptions",
        WidgetCatalogId::Callout => "callout",
        WidgetCatalogId::VirtualMasonry => "virtual_masonry",
        WidgetCatalogId::TerminalView => "terminal_view",
    }
}

#[test]
fn no_control_changes_its_primitive_count_between_states() {
    let mut failures = Vec::new();
    for &id in WidgetCatalogId::ALL {
        let baseline = probe_widget(id, ControlState::default()).primitives.len();
        for state in states() {
            let count = probe_widget(id, state).primitives.len();
            if count != baseline {
                failures.push(format!(
                    "{} emits {count} primitives in {state:?}, but {baseline} when idle",
                    id.module_name()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} control(s) changed shape with their state:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// 探针不得流于形式：悬停必须实际改变*某些字段*。
///
/// 若所有状态绘制完全相同，上方的形状测试虽会通过却毫无意义，因此本测试断言契约的另一半：
/// 状态变更即字段变更，这正是渲染器可直接修补的部分。
/// 上方 [`INERT_BY_DESIGN`] 中列出的组件因已记录的原因豁免。
///
///
#[test]
fn every_interactive_control_paints_differently_when_hovered() {
    let idle = ControlState {
        phase: PointerPhase::Idle,
        ..ControlState::default()
    };
    let hovered = ControlState {
        phase: PointerPhase::Hovered,
        ..ControlState::default()
    };
    let mut same = Vec::new();
    for &id in WidgetCatalogId::ALL {
        let idle_primitives = probe_widget(id, idle).primitives;
        let hovered_primitives = probe_widget(id, hovered).primitives;
        if idle_primitives == hovered_primitives && !INERT_BY_DESIGN.contains(&id) {
            same.push(id.module_name());
        }
    }
    assert!(
        same.is_empty(),
        "these controls paint the same idle and hovered: {same:?}"
    );
}

#[test]
fn the_probe_covers_every_component() {
    let mut probed = Vec::new();
    for &id in WidgetCatalogId::ALL {
        // 详尽 match 是门禁；witness 证明探针确实运行了该组件的输出器。
        //
        let name = component_name(id);
        assert_eq!(name, id.module_name());
        let probe = probe_widget(id, ControlState::default());
        assert_eq!(probe.witness.id(), id);
        probed.push(id);
    }
    probed.sort_unstable_by_key(|id| *id as usize);
    probed.dedup();
    assert_eq!(
        probed.len(),
        WidgetCatalogId::COUNT,
        "the probe must cover every component exactly once"
    );
    assert_eq!(WidgetCatalogId::COUNT, 99);
}

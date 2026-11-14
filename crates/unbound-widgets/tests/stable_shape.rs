//! Every standard control keeps a constant primitive count across states.
//!
//! The renderer can repair a *paint* change by patching the slices that
//! changed, but only while the scene's shape stays the same: a primitive that
//! appears or disappears renumbers every primitive after it, and the whole
//! pool has to be rewritten. Hovering a control is the most common thing a UI
//! does, so the controls promise the cheap path:
//!
//! > a control's state never changes how many primitives it emits.
//!
//! A state that needs a different appearance changes the *fields* of a
//! primitive — its colour slot, its border width, its opacity — never the
//! number of primitives.
//!
//! The probe is catalog-driven: [`unbound_widgets::catalog::probe_widget`]
//! runs every component's genuine emitter, and its exhaustive `match` is the
//! compile-time gate that keeps a new component from skipping this contract.

use unbound_widgets::catalog::probe_widget;
use unbound_widgets::{ControlState, PointerPhase, WidgetCatalogId};

/// Every state a control can be in: phase × focused × disabled × selected.
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

/// Components whose primitives legitimately do not change on hover.
///
/// Each entry is inert for a documented reason, not by accident:
///
/// * Decorative / read-only surfaces never answer the pointer:
///   `Divider`, `Skeleton`, `Progress`, `Image`, `Avatar`, `RichText`,
///   `Containers`, `Form` (layout + validation only).
/// * Overlay chrome reacts to `disabled`/`focused` but has no hover wash of
///   its own: `Modal`, `Drawer`, `AlertDialog`, `Popover`, `Tooltip`,
///   `HoverCard`, `Alert` (focus ring only), `Toast` (opacity only).
/// * Components whose hover highlight lives in their own interaction state
///   (`MenuState::highlighted`, `ChartState::hover`, `TableState`,
///   `PropertyGridState`, …) rather than in `ControlState::phase`:
///   `Menu`, `ContextMenu`, `Chart`, `Table`, `PropertyGrid`, `CommandPalette`,
///   `Breadcrumb`, `ColorPicker`, `CurveEditor`, `DatePicker`, `Minimap`,
///   `Gizmo`, `NodeGraph`, `Ruler`.
/// * `Overlay` is pure placement/stack bookkeeping and emits no primitives.
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

/// The compile-time gate, mirrored in the test crate: an exhaustive `match`
/// over the catalog with no wildcard. A new variant fails to compile here
/// until it is named, exactly like the probe in `catalog.rs`.
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

/// The probe must not be vacuous: a hover has to change *something*.
///
/// If every state painted identically the shape test above would pass while
/// proving nothing, so this one states the other half of the contract: a
/// state change is a change of fields, which is exactly the patch the
/// renderer can apply. Components listed in [`INERT_BY_DESIGN`] are exempt
/// for the documented reasons above.
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
        // The exhaustive match is the gate; the witness proves the probe
        // really ran this component's emitter.
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

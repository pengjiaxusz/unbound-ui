//! The component gallery: every catalog component painted by its genuine
//! emitter.
//!
//! Each card drives [`unbound_widgets::catalog::emit_widget`] — the same
//! exhaustive, wildcard-free match that backs the stable-shape probe — so the
//! surface, hit region and semantic node all come from the real
//! `unbound-widgets` emitter, never a hand-drawn stand-in. The frame records
//! the component in `Canvas::emitted_widgets`, which the Showcase coverage
//! gate reads to prove 100% of the catalog is hosted.
//!
//! [`showcase_entry_for_widget`] and [`COMPONENT_EXAMPLES`] are generated from
//! one id list, so a new catalog variant cannot be added to the gallery
//! without also being named in the exhaustive entry function (and vice
//! versa). A `const` assertion below ties both to `WidgetCatalogId::ALL`.

use unbound_math::{Rect, Vec2};
use unbound_widgets::catalog::emit_widget;
use unbound_widgets::WidgetCatalogId;

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

pub const COMPONENTS_INFO: PageInfo = PageInfo {
    title: "Component gallery",
    short: "Components",
    description: "All 56 standard components, each painted by its genuine unbound-widgets emitter with a real hit region and semantic node.",
    crates: &["unbound-widgets"],
    docs: "docs/05-agent/layering.md",
    keywords: "component gallery catalog emitter coverage widget showcase every control",
};

/// Generate the gallery examples and the exhaustive coverage gate from one
/// id list. Adding a catalog variant forces a new entry here (the `const`
/// assertion below fails otherwise) and a new arm in `emit_widget` (`E0004`).
macro_rules! define_showcase_components {
    ($( $id:ident => $title:literal, $desc:literal, $height:literal ),+ $(,)?) => {
        /// One card per catalog component, in catalog order.
        pub const COMPONENT_EXAMPLES: &[ExampleDef] = &[
            $(
                ExampleDef::new(
                    $title,
                    $desc,
                    &["unbound-widgets"],
                    concat!(
                        "unbound_widgets::catalog::emit_widget(\n",
                        "    WidgetCatalogId::", stringify!($id), ",\n",
                        "    node, state, rect, &mut glyphs,\n",
                        ");",
                    ),
                    concat!(
                        "The genuine ", stringify!($id),
                        " emitter paints this surface; its semantic node and hit region come from the component itself.",
                    ),
                    &[],
                    DemoKind::Component { id: WidgetCatalogId::$id },
                )
                .with_height($height),
            )+
        ];

        /// Map every catalog component to the page and demo that hosts it.
        ///
        /// Exhaustive: NO `_ =>` wildcard arm. A new `WidgetCatalogId` variant
        /// fails to compile here until it is named, mirroring the probe gate
        /// in `unbound-widgets`.
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
    Tooltip => "Tooltip", "A delayed, smart-flipping hint.", 260.0,
    TreeView => "Tree view", "An expandable row with keyboard navigation.", 320.0,
    VirtualViews => "Virtual row", "A Fenwick-backed variable-height virtual row.", 240.0,
}

/// Whether `examples` mounts a card for `id` (const-comparable).
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

/// Compile-time coverage gate: every catalog component must map to a demo
/// that the target page actually mounts.
///
/// This is the static half of S2.4. Adding a `WidgetCatalogId` variant without
/// a gallery card fails here; adding one without a genuine emitter fails in
/// `unbound_widgets::catalog::emit_widget` (`E0004`). The runtime half lives in
/// `tests/shell.rs`, which asserts the emitted set covers the whole catalog.
const _: () = {
    let mut i = 0;
    while i < WidgetCatalogId::COUNT {
        let id = WidgetCatalogId::ALL[i];
        let (page, _demo) = showcase_entry_for_widget(id);
        assert!(
            hosts(page.examples(), id),
            "catalog component is not mounted by its showcase page",
        );
        i += 1;
    }
};

/// Render one component card with its genuine emitter.
pub fn component(ctx: &mut DemoCtx<'_, '_>, id: WidgetCatalogId) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    ctx.canvas.text(
        id.module_name(),
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
    let state = ctx.canvas.control_state(node, false, false);
    let mut glyphs = ctx.canvas.shape(
        "Component",
        Vec2::new(stage.origin.x + 8.0, stage.origin.y + 8.0),
        13.0,
    );
    let output = emit_widget(id, node, state, stage, &mut glyphs);
    ctx.canvas.widget_output(id, node, &output);
}

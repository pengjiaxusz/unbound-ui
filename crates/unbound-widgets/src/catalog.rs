//! The exhaustive component probe: one genuine emitter call per catalog id.
//!
//! [`probe_widget`] is the compile-time gate of the showcase: its `match`
//! names every [`WidgetCatalogId`] variant with no wildcard, so adding a
//! component to the catalog without wiring its real emitter here is a compile
//! error (`E0004`), not a silently missing showcase branch.
//!
//! Every arm builds a minimal but valid descriptor (the same constructions
//! the component's own unit tests use), runs the component's public `emit_*`
//! function and returns the emitted primitives. The only input that varies
//! between calls is [`ControlState`]; every extra state struct is a fixed
//! representative value. The returned [`WidgetShowcaseWitness`] proves the
//! emitter really ran — it can only be minted inside this crate.
//!
//! [`emit_widget`] is the full-output form of the same exhaustive match: it
//! places the component at a caller-supplied rectangle with caller-shaped
//! glyphs and returns the complete [`WidgetOutput`] (primitives, payloads,
//! hit region and semantic node). The Showcase component gallery drives it
//! directly, so every catalog component is painted by its genuine emitter
//! with a real identity, not a hand-drawn stand-in. `probe_widget` delegates
//! to it, keeping a single source of truth for the compile-time gate.

use unbound_math::{Color, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::virtualize::TreeRow;

use crate::{
    layout_form, layout_tags, parse_markdown, AccordionDesc, AccordionSection, AccordionState,
    AccordionTokens, AlertDesc, AlertDialogDesc, AvatarDesc, BreadcrumbDesc, BreadcrumbTokens,
    ButtonDesc, CalendarState, CanvasToolbarDesc, CanvasToolbarState, CardDesc, CardTokens,
    ChartDesc, ChartSeries, ChartState, CheckboxDesc, CheckboxTokens, ChipDesc, CodeBlockDesc,
    CodeBlockState, ColorPickerDesc, ColorPickerState, ColumnSpec, ComboItem, ComboboxDesc,
    ComboboxState, Command, CommandPaletteDesc, CommandPaletteState, CommandPaletteTokens,
    ContextMenuState, ControlState, Crumb, CurveEditorDesc, CurveEditorState, DatePickerDesc,
    DividerDesc, DividerTokens, DockTab, DockTokens, DockTree, DrawerDesc, DrawerSide, DrawerState,
    FileDropDesc, FileDropState, FormDesc, FormField, FormFieldDesc, GizmoDesc, GraphNodeDesc,
    GraphWire, HoverCardDesc, ImageDesc, MenuBarDesc, MenuBarItem, MenuBarState, MenuBarTokens,
    MenuDesc, MenuItem, MenuState, MinimapDesc, MinimapState, ModalDesc, NaiveDate, NodeGraphState,
    NumberInputDesc, NumberInputState, PinSpec, PointerPhase, PopoverDesc, Presence, ProgressDesc,
    ProgressState, PropertyDesc, PropertyGridDesc, PropertyGridState, PropertySection,
    PropertyValue, RadioDesc, RadioTokens, RichTextDesc, RulerDesc, RulerState, ScrollState,
    ScrollViewDesc, SegmentedDesc, SegmentedTokens, SelectDesc, SelectState, SelectionMode,
    Severity, SkeletonDesc, SkeletonState, SliderDesc, SliderTokens, SplitPaneDesc, SplitPaneState,
    SplitPaneTokens, SwitchDesc, SwitchTokens, TableDesc, TableState, TabsDesc, TabsState,
    TabsTokens, TagInputDesc, TagInputState, TextAreaDesc, TextAreaState, TextAreaVisual,
    TextEditState, TextInputDesc, TextInputVisual, ToastEntry, ToolbarDesc, ToolbarItem,
    ToolbarState, ToolbarTokens, TooltipDesc, TreeViewDesc, TreeViewRow, TreeViewTokens,
    VirtualRowTokens, WidgetCatalogId, WidgetOutput, WidgetShowcaseWitness,
};

/// The standard probe rectangle, comfortably above the minimum target size.
const RECT: Rect = Rect::new(40.0, 40.0, 220.0, 36.0);

/// A small fixed glyph run, shaped like a six-character label.
fn label() -> Vec<GlyphQuad> {
    (0..6)
        .map(|index| GlyphQuad {
            rect: Rect::new(
                RECT.origin.x + 60.0 + index as f32 * 8.0,
                RECT.origin.y + 10.0,
                7.0,
                13.0,
            ),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        })
        .collect()
}

/// The result of probing one component: proof it was really emitted, plus its
/// primitives so the stable-shape test can count them.
pub struct CatalogProbe {
    /// Proof that the component's genuine emitter ran.
    pub witness: WidgetShowcaseWitness,
    /// The primitives the emitter produced, in draw order.
    pub primitives: Vec<unbound_gpu::data::GpuPrimitiveInstance>,
}

/// Run the genuine emitter for `id` under `state`, with a minimal but valid
/// descriptor. Exhaustive: NO `_ =>` wildcard arm allowed.
#[deny(unreachable_patterns)]
pub fn probe_widget(id: WidgetCatalogId, state: ControlState) -> CatalogProbe {
    let output = emit_widget(id, 1, state, RECT, &mut label());
    CatalogProbe {
        witness: WidgetShowcaseWitness::new(id),
        primitives: output.primitives,
    }
}

/// The full-output form of the exhaustive component match.
///
/// Places the component at `rect` (used both as its bounds and, for overlay
/// components, as the placement viewport), binds it to `node` and shapes its
/// primary label from `label`. Returns the complete [`WidgetOutput`] so the
/// caller gets a real hit region and semantic node alongside the primitives.
///
/// Exhaustive: NO `_ =>` wildcard arm allowed — this is the single compile-time
/// gate that keeps every catalog component wired to its genuine emitter.
#[deny(unreachable_patterns)]
pub fn emit_widget(
    id: WidgetCatalogId,
    node: u64,
    state: ControlState,
    rect: Rect,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    let o = rect.origin;
    match id {
        WidgetCatalogId::Accordion => {
            let desc = AccordionDesc::new(
                Rect::new(o.x, o.y, rect.size.x, rect.size.y.max(200.0)),
                vec![
                    AccordionSection::new(1, "General", 100.0),
                    AccordionSection::new(2, "Advanced", 80.0),
                ],
            );
            let accordion = AccordionState::new(2);
            crate::accordion::emit_accordion_header(
                &desc,
                &accordion,
                0,
                state,
                label,
                AccordionTokens::standard(),
            )
        }
        WidgetCatalogId::Alert => {
            let desc = AlertDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 64.0),
                Severity::Warning,
                "Low disk space",
            )
            .dismissible();
            crate::alert::emit_alert(node, &desc, label, state)
        }
        WidgetCatalogId::AlertDialog => {
            let desc = AlertDialogDesc::new(rect, "Delete workspace?")
                .with_message("This cannot be undone.");
            crate::alert_dialog::emit_alert_dialog(node, rect, &desc, label, state).output
        }
        WidgetCatalogId::Avatar => {
            let side = rect.size.x.min(rect.size.y).min(48.0);
            let desc =
                AvatarDesc::new(Rect::new(o.x, o.y, side, side), "Ada").presence(Presence::Online);
            crate::avatar::emit_avatar(node, &desc, label, state)
        }
        WidgetCatalogId::Badge => {
            // The chip is the interactive half of the badge module: it answers
            // the pointer, while a plain badge is a static label.
            let desc = ChipDesc::new(Rect::new(o.x, o.y, 80.0, 24.0), "PNG").closable();
            crate::badge::emit_chip(node, &desc, state, label)
        }
        WidgetCatalogId::Breadcrumb => {
            let desc = BreadcrumbDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 24.0),
                vec![Crumb::new(1, "Root"), Crumb::new(2, "src")],
            );
            let slots = desc.layout(&[40.0, 30.0]);
            crate::breadcrumb::emit_crumb(
                &desc,
                &slots[0],
                state,
                label,
                BreadcrumbTokens::standard(),
            )
        }
        WidgetCatalogId::Button => {
            let desc = ButtonDesc::new(rect, "Save");
            crate::button::emit_button(node, &desc, state, label)
        }
        WidgetCatalogId::CanvasToolbar => {
            let desc = CanvasToolbarDesc::new(rect);
            let toolbar = CanvasToolbarState::new(1.0);
            crate::canvas_toolbar::emit_canvas_toolbar(node, &desc, &toolbar, label, state).output
        }
        WidgetCatalogId::Chart => {
            let desc = ChartDesc::new(rect).with_series(vec![ChartSeries::line(
                vec![
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 5.0),
                    Vec2::new(2.0, 2.0),
                ],
                Token::PRIMARY,
            )]);
            crate::chart::emit_chart(node, &desc, &ChartState::new(), label, state).output
        }
        WidgetCatalogId::Checkbox => {
            let desc = CheckboxDesc::new(rect, "Snap to grid");
            crate::checkbox::emit_checkbox(
                node,
                &desc,
                state,
                &[],
                label,
                CheckboxTokens::standard(),
            )
        }
        WidgetCatalogId::CodeBlock => {
            let desc = CodeBlockDesc::new(rect, "fn main() {\n    let x = 42;\n}", "rust");
            crate::code_block::emit_code_block(node, &desc, &CodeBlockState::new(), label, state)
        }
        WidgetCatalogId::ColorPicker => {
            let desc = ColorPickerDesc::new(rect);
            let picker = ColorPickerState::new(Color::from_srgb8(24, 128, 200, 255));
            crate::color_picker::emit_color_picker(node, &desc, &picker, state, label)
        }
        WidgetCatalogId::Combobox => {
            let desc = ComboboxDesc::new(
                rect,
                vec![
                    ComboItem::new("apple", "Apple"),
                    ComboItem::new("banana", "Banana"),
                ],
            );
            let combo = ComboboxState::new(&desc);
            crate::combobox::emit_combobox_trigger(node, &desc, &combo, state, label)
        }
        WidgetCatalogId::CommandPalette => {
            let desc = CommandPaletteDesc::new(rect);
            let palette = CommandPaletteState::new(vec![
                Command::new(1, "Open File", "File").with_shortcut("Ctrl+O"),
                Command::new(2, "Rebuild", "Build"),
            ]);
            crate::command_palette::emit_command_row(
                &desc,
                &palette,
                0,
                state,
                label,
                &[],
                CommandPaletteTokens::standard(),
            )
        }
        WidgetCatalogId::Containers => {
            let desc = CardDesc::new(rect).with_header(40.0).with_footer(30.0);
            crate::containers::emit_card(&desc, CardTokens::standard())
        }
        WidgetCatalogId::ContextMenu => {
            // The context menu is a placement state machine: it renders
            // through the genuine menu emitter on the descriptor it places.
            let mut context = ContextMenuState::new();
            context.open_at(o, 1, None);
            let menu = MenuDesc::new(
                Rect::new(o.x, o.y, 200.0, 0.0),
                vec![
                    MenuItem::action("Cut").with_shortcut("Ctrl+X"),
                    MenuItem::action("Copy").with_shortcut("Ctrl+C"),
                ],
            );
            let placed = context.desc_at(menu, rect);
            crate::menu::emit_menu(node, 2, &placed, &context.menu, state).0
        }
        WidgetCatalogId::CurveEditor => {
            let desc = CurveEditorDesc::new(rect);
            crate::curve_editor::emit_curve_editor(node, &desc, &CurveEditorState::new(), state)
        }
        WidgetCatalogId::DatePicker => {
            let desc = DatePickerDesc::new(rect);
            let today = NaiveDate::new(2024, 1, 15).expect("valid probe date");
            let calendar = CalendarState::new(today, SelectionMode::Single);
            crate::date_picker::emit_calendar(node, &desc, &calendar, state, label)
        }
        WidgetCatalogId::Divider => {
            let desc = DividerDesc::horizontal(Rect::new(o.x, o.y, rect.size.x, 8.0));
            crate::divider::emit_divider(&desc, label, 0.0, DividerTokens::standard())
        }
        WidgetCatalogId::Dock => {
            let pane = DockTree::new(vec![DockTab::new(1, "Explorer"), DockTab::new(2, "Search")])
                .layout(rect)
                .remove(0);
            crate::dock::emit_dock_tab(&pane, 1, 128.0, state, label, DockTokens::standard(), 10)
        }
        WidgetCatalogId::Drawer => {
            let desc = DrawerDesc::new(rect, DrawerSide::Right, (rect.size.x * 0.6).min(320.0));
            let drawer = DrawerState::new(true);
            crate::drawer::emit_drawer(node, &desc, &drawer, &[10, 11], label, state).output
        }
        WidgetCatalogId::FileDrop => {
            let desc = FileDropDesc::new(rect);
            crate::file_drop::emit_file_drop(node, &desc, &FileDropState::new(), state, label)
        }
        WidgetCatalogId::Form => {
            // The form module owns layout and validation; its painted parts
            // are the field label (with the required marker) and the message.
            let desc = FormDesc::new(rect);
            let fields = vec![FormField::new(
                1,
                FormFieldDesc::new("Name").required(true).help("Full name"),
            )];
            let layouts = layout_form(&desc, &fields);
            crate::form::emit_form_label(&fields[0], &layouts[0], label)
        }
        WidgetCatalogId::Gizmo => {
            let desc = GizmoDesc::new(rect, 2.0);
            crate::gizmo::emit_gizmo(node, &desc, None, &[], label, state).output
        }
        WidgetCatalogId::HoverCard => {
            let desc = HoverCardDesc::new(
                Rect::new(o.x, o.y, 120.0, 32.0),
                rect,
                Vec2::new(o.x + 40.0, o.y - 40.0),
            );
            crate::hover_card::emit_hover_card(node, 2, &desc, label, state).0
        }
        WidgetCatalogId::Image => {
            let desc = ImageDesc::new(rect, Vec2::new(100.0, 100.0), 7).with_alt("portrait");
            crate::image::emit_image(node, &desc, state)
        }
        WidgetCatalogId::Menu => {
            let desc = MenuDesc::new(
                rect,
                vec![
                    MenuItem::action("Open").with_shortcut("Ctrl+O"),
                    MenuItem::separator(),
                    MenuItem::action("Quit"),
                ],
            );
            crate::menu::emit_menu(node, 2, &desc, &MenuState::new(), state).0
        }
        WidgetCatalogId::Menubar => {
            let desc = MenuBarDesc::new(
                Rect::new(o.x, o.y, rect.size.x, crate::menubar::MENUBAR_HEIGHT),
                vec![MenuBarItem::new(1, "File"), MenuBarItem::new(2, "Edit")],
            );
            let item_rect = desc.layout(&[])[0].0;
            crate::menubar::emit_menubar_item(
                &desc,
                &MenuBarState::new(),
                0,
                item_rect,
                state,
                label,
                MenuBarTokens::standard(),
            )
        }
        WidgetCatalogId::Minimap => {
            let desc = MinimapDesc::new(
                rect,
                Rect::new(0.0, 0.0, 1000.0, 1000.0),
                Rect::new(200.0, 100.0, 400.0, 300.0),
            );
            crate::minimap::emit_minimap(node, &desc, &[], &MinimapState::new(), state).0
        }
        WidgetCatalogId::Modal => {
            let desc = ModalDesc::new(rect).with_label("Settings");
            crate::modal::emit_modal(node, rect, &desc, &[10, 11], state).output
        }
        WidgetCatalogId::NodeGraph => {
            let nodes = vec![
                GraphNodeDesc::new(
                    1,
                    Vec2::new(o.x, o.y + 40.0),
                    Vec2::new(160.0, 120.0),
                    "Node",
                )
                .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")]),
                GraphNodeDesc::new(
                    2,
                    Vec2::new(o.x + 300.0, o.y + 40.0),
                    Vec2::new(160.0, 120.0),
                    "Node",
                )
                .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")]),
            ];
            let wires = [GraphWire {
                from_node: 1,
                from_pin: 0,
                to_node: 2,
                to_pin: 0,
            }];
            crate::node_graph::emit_node_graph(
                node,
                &nodes,
                &wires,
                &NodeGraphState::new(),
                &[Vec::new(), Vec::new()],
            )
            .output
        }
        WidgetCatalogId::NumberInput => {
            let desc = NumberInputDesc::new(rect, 0.0, 100.0);
            let number = NumberInputState::new(10.0);
            crate::number_input::emit_number_input(node, &desc, &number, state, label)
        }
        WidgetCatalogId::Overlay => {
            // The overlay module is pure placement and stack bookkeeping
            // (`place_anchored_popup`, `OverlayStack`): it emits NO primitives
            // of its own. The visible overlay surfaces are painted by the
            // popover/menu/tooltip/modal emitters, which are probed under
            // their own catalog ids. This arm is therefore legitimately empty.
            WidgetOutput::default()
        }
        WidgetCatalogId::Popover => {
            let desc = PopoverDesc::new(
                Rect::new(o.x, o.y, 120.0, 36.0),
                rect,
                Vec2::new(o.x + 40.0, o.y),
            );
            crate::popover::emit_popover(node, 2, &desc, state).output
        }
        WidgetCatalogId::Progress => {
            let desc = ProgressDesc::new(Rect::new(o.x, o.y, rect.size.x, 8.0)).value(0.4);
            crate::progress::emit_progress(&desc, &ProgressState::new())
        }
        WidgetCatalogId::PropertyGrid => {
            let desc = PropertyGridDesc::new(
                rect,
                vec![PropertySection::new("Transform")
                    .with_row(PropertyDesc::new("X", PropertyValue::Number(10.0)))],
            );
            crate::property_grid::emit_property_grid(
                node,
                &desc,
                &PropertyGridState::new(),
                label,
                state,
            )
        }
        WidgetCatalogId::Radio => {
            let desc = RadioDesc::new(rect, "Comfortable");
            crate::radio::emit_radio(node, &desc, state, label, RadioTokens::standard())
        }
        WidgetCatalogId::RichText => {
            let desc = RichTextDesc::new(rect);
            let document = parse_markdown("# Title\n\nhello");
            let layout = desc.layout(&document, &[28.0, 20.0]);
            crate::rich_text::emit_rich_text(node, &desc, &document, &layout, label, state)
        }
        WidgetCatalogId::Ruler => {
            let desc = RulerDesc::new(rect, 1.0, Vec2::ZERO);
            crate::ruler::emit_rulers(node, &desc, &RulerState::new(), state).output
        }
        WidgetCatalogId::Scroll => {
            let desc = ScrollViewDesc::new(rect, Vec2::new(rect.size.x, 3000.0));
            crate::scroll::emit_scroll_view(node, &desc, &ScrollState::new(), state).output
        }
        WidgetCatalogId::Segmented => {
            // Probe an *unselected* slot: it is the one that answers the
            // pointer (the active slot is the selection itself).
            let desc = SegmentedDesc::new(rect, &["One", "Two", "Three"]);
            crate::segmented::emit_segmented_segment(
                node,
                &desc,
                1,
                state,
                label,
                SegmentedTokens::standard(),
            )
        }
        WidgetCatalogId::Select => {
            let desc = SelectDesc::new(rect, vec!["Apple".into(), "Banana".into()]);
            crate::select::emit_select(node, &desc, &SelectState::new(), state)
        }
        WidgetCatalogId::Skeleton => {
            let desc = SkeletonDesc::new(Rect::new(o.x, o.y, rect.size.x, 16.0)).animated();
            crate::skeleton::emit_skeleton(&desc, &SkeletonState::new())
        }
        WidgetCatalogId::Slider => {
            let desc = SliderDesc::new(rect, "Gap", 18.0, 0.0, 36.0).with_step(1.0);
            crate::slider::emit_slider(node, &desc, state, label, SliderTokens::standard())
        }
        WidgetCatalogId::SplitPane => {
            let desc = SplitPaneDesc::horizontal(rect);
            crate::split_pane::emit_split_pane(
                node,
                &desc,
                &SplitPaneState::new(0.5),
                state,
                SplitPaneTokens::standard(),
            )
        }
        WidgetCatalogId::Switch => {
            let desc = SwitchDesc::new(rect, "Animate");
            crate::switch::emit_switch(node, &desc, state, label, SwitchTokens::standard())
        }
        WidgetCatalogId::Table => {
            let desc = TableDesc::new(
                rect,
                vec![
                    ColumnSpec::new("Name", 160.0),
                    ColumnSpec::new("Size", 80.0).right_aligned(),
                ],
            )
            .with_label("Files");
            crate::table::emit_table(node, &desc, &TableState::new(), &[], label, state)
        }
        WidgetCatalogId::Tabs => {
            // Probe an inactive tab: the active one is the selection itself.
            let desc = TabsDesc::new(rect, &["One", "Two", "Three"]);
            crate::tabs::emit_tab(
                &desc,
                &TabsState::new(0),
                1,
                state,
                label,
                TabsTokens::standard(),
            )
        }
        WidgetCatalogId::TagInput => {
            let desc = TagInputDesc::new(rect);
            let tags = TagInputState::with_tags(["rust".to_owned()]);
            let layout = layout_tags(&desc, &tags, |tag| tag.len() as f32 * 8.0);
            crate::tag_input::emit_tag_input(node, &desc, &tags, state, &layout, label)
        }
        WidgetCatalogId::TextArea => {
            let desc = TextAreaDesc::new(rect);
            let area = TextAreaState::with_text("one\ntwo");
            crate::text_area::emit_text_area(
                node,
                &desc,
                &area,
                state,
                &TextAreaVisual::default(),
                label,
            )
        }
        WidgetCatalogId::TextInput => {
            let desc = TextInputDesc::new(rect).with_placeholder("type here");
            crate::text_input::emit_text_input(
                node,
                &desc,
                state,
                &TextEditState::new(),
                &TextInputVisual::default(),
                label,
            )
        }
        WidgetCatalogId::Toast => {
            let mut entry =
                ToastEntry::new(1, "Saved", Severity::Success).with_message("All changes stored.");
            entry.y_spring.value = 0.0;
            entry.opacity_spring.value = 1.0;
            crate::toast::emit_toast(node, &entry, rect, label, state).output
        }
        WidgetCatalogId::Toolbar => {
            let desc = ToolbarDesc::new(
                rect,
                vec![
                    ToolbarItem::button(1, "Bold"),
                    ToolbarItem::toggle(2, "Italic"),
                ],
            );
            let layout = desc.layout();
            crate::toolbar::emit_toolbar_item(
                &desc,
                &ToolbarState::new(),
                0,
                layout.rects[0].expect("button rect"),
                state,
                label,
                ToolbarTokens::standard(),
            )
        }
        WidgetCatalogId::Tooltip => {
            let desc = TooltipDesc::new(
                Rect::new(o.x, o.y, 80.0, 32.0),
                rect,
                Vec2::new(o.x + 20.0, o.y - 72.0),
            );
            crate::tooltip::emit_tooltip(node, 2, &desc, state).0
        }
        WidgetCatalogId::TreeView => {
            let desc = TreeViewDesc::new(rect);
            let row = TreeViewRow {
                row: TreeRow {
                    id: 1,
                    depth: 0,
                    has_children: true,
                    expanded: false,
                },
                item: 0,
                rect: Rect::new(o.x, o.y, rect.size.x, 28.0),
            };
            crate::tree_view::emit_tree_row(
                &desc,
                &row,
                false,
                state,
                label,
                TreeViewTokens::standard(),
            )
        }
        WidgetCatalogId::VirtualViews => {
            // The row emitter takes the hover bit directly; derive it from
            // the control phase so the probe still varies only ControlState.
            crate::virtual_views::emit_virtual_row(
                Rect::new(o.x, o.y, rect.size.x, 28.0),
                label,
                false,
                state.phase == PointerPhase::Hovered,
                VirtualRowTokens::standard(),
                100,
            )
        }
        WidgetCatalogId::Collapsible => {
            let desc = crate::collapsible::CollapsibleDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 76.0),
                "Details",
                true,
            );
            crate::collapsible::emit_collapsible(node, &desc, state, label)
        }
        WidgetCatalogId::Masonry => {
            let desc = crate::masonry::MasonryDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                2,
                vec![
                    crate::masonry::MasonryItem { height: 32.0 },
                    crate::masonry::MasonryItem { height: 24.0 },
                    crate::masonry::MasonryItem { height: 28.0 },
                ],
            );
            crate::masonry::emit_masonry(node, &desc, state)
        }
        WidgetCatalogId::NavigationRail => {
            let desc = crate::navigation_rail::NavigationRailDesc::new(
                Rect::new(o.x, o.y, 140.0, 110.0),
                vec![
                    crate::navigation_rail::NavRailItem::new(1, "Home").with_badge(2),
                    crate::navigation_rail::NavRailItem::new(2, "Assets"),
                ],
                1,
            );
            crate::navigation_rail::emit_navigation_rail(node, &desc, state, label)
        }
        WidgetCatalogId::Pagination => {
            let desc = crate::pagination::PaginationDesc::new(rect, 12, 5);
            crate::pagination::emit_pagination(node, &desc, state, label)
        }
        WidgetCatalogId::Stepper => {
            let desc = crate::stepper::StepperDesc::new(
                rect,
                vec![
                    crate::stepper::StepItem::new("Plan", crate::stepper::StepStatus::Completed),
                    crate::stepper::StepItem::new("Build", crate::stepper::StepStatus::Active),
                    crate::stepper::StepItem::new("Ship", crate::stepper::StepStatus::Pending),
                ],
            );
            crate::stepper::emit_stepper(node, &desc, state, label)
        }
        WidgetCatalogId::AutoComplete => {
            let desc = crate::autocomplete::AutoCompleteDesc::new(rect, "unb", 3);
            crate::autocomplete::emit_autocomplete(node, &desc, state, label)
        }
        WidgetCatalogId::Banner => {
            let desc = crate::banner::BannerDesc::new(rect, "Hot reload ready", Severity::Info);
            crate::banner::emit_banner(node, &desc, state, label)
        }
        WidgetCatalogId::Cascader => {
            let desc = crate::cascader::CascaderDesc::new(
                rect,
                vec![crate::cascader::CascaderOption::leaf("ui", "UI")],
                vec!["ui".to_owned()],
            );
            crate::cascader::emit_cascader(node, &desc, state, label)
        }
        WidgetCatalogId::DateRangePicker => {
            let desc = crate::date_range_picker::DateRangePickerDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                Some(crate::date_range_picker::DateRange::ordered(
                    NaiveDate::new(2026, 11, 1).expect("valid date"),
                    NaiveDate::new(2026, 11, 13).expect("valid date"),
                )),
            );
            crate::date_range_picker::emit_date_range_picker(node, &desc, state, label)
        }
        WidgetCatalogId::HotkeyInput => {
            let desc = crate::hotkey_input::HotkeyInputDesc::new(rect, "Ctrl+K");
            crate::hotkey_input::emit_hotkey_input(node, &desc, state, label)
        }
        WidgetCatalogId::NotificationCenter => {
            let desc = crate::notification_center::NotificationCenterDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                vec![crate::notification_center::NotificationItem {
                    id: 1,
                    title: "Build finished".to_owned(),
                    severity: Severity::Success,
                    read: false,
                }],
            );
            crate::notification_center::emit_notification_center(node, &desc, state, label)
        }
        WidgetCatalogId::PinInput => {
            let desc = crate::pin_input::PinInputDesc::new(rect, 4, 2);
            crate::pin_input::emit_pin_input(node, &desc, state, label)
        }
        WidgetCatalogId::Popconfirm => {
            let desc = crate::popconfirm::PopconfirmDesc::new(
                Rect::new(o.x, o.y, 180.0, 68.0),
                "Delete item?",
            );
            crate::popconfirm::emit_popconfirm(node, &desc, state, label)
        }
        WidgetCatalogId::Rating => {
            let desc = crate::rating::RatingDesc::new(rect, 4.5);
            crate::rating::emit_rating(node, &desc, state)
        }
        WidgetCatalogId::Spinner => {
            let desc = crate::spinner::SpinnerDesc::new(Rect::new(o.x, o.y, 36.0, 36.0), 0.25);
            crate::spinner::emit_spinner(&desc, state)
        }
        WidgetCatalogId::TimePicker => {
            let desc = crate::time_picker::TimePickerDesc::new(
                rect,
                crate::time_picker::TimeValue::new(14, 30, 0),
            );
            crate::time_picker::emit_time_picker(node, &desc, state, label)
        }
        WidgetCatalogId::Transfer => {
            let desc =
                crate::transfer::TransferDesc::new(Rect::new(o.x, o.y, rect.size.x, 96.0), 5, 2);
            crate::transfer::emit_transfer(node, &desc, state, label)
        }
        WidgetCatalogId::Carousel => {
            let desc =
                crate::carousel::CarouselDesc::new(Rect::new(o.x, o.y, rect.size.x, 84.0), 4, 1);
            crate::carousel::emit_carousel(node, &desc, state, label)
        }
        WidgetCatalogId::CodeEditor => {
            let desc = crate::code_editor::CodeEditorDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                12,
                1,
            );
            crate::code_editor::emit_code_editor(node, &desc, state, label)
        }
        WidgetCatalogId::DataGrid => {
            let desc = crate::data_grid::DataGridDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                (1, 1),
                false,
            );
            crate::data_grid::emit_data_grid(node, &desc, state, label)
        }
        WidgetCatalogId::Timeline => {
            let desc = crate::timeline::TimelineDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                vec![
                    crate::timeline::TimelineItem {
                        title: "Init".to_owned(),
                        timestamp: "10:00".to_owned(),
                        severity: Severity::Success,
                    },
                    crate::timeline::TimelineItem {
                        title: "Verify".to_owned(),
                        timestamp: "10:05".to_owned(),
                        severity: Severity::Info,
                    },
                ],
            );
            crate::timeline::emit_timeline(node, &desc, state, label)
        }
        WidgetCatalogId::TimelineTrack => {
            let desc = crate::timeline_track::TimelineTrackDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                1.5,
                4.0,
                vec![
                    crate::timeline_track::Keyframe {
                        track: 0,
                        time: 0.5,
                    },
                    crate::timeline_track::Keyframe {
                        track: 1,
                        time: 2.0,
                    },
                ],
            );
            crate::timeline_track::emit_timeline_track(node, &desc, state, label)
        }
        WidgetCatalogId::Tour => {
            let desc = crate::tour::TourDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 96.0),
                Rect::new(o.x + 12.0, o.y + 8.0, 64.0, 28.0),
                "Step 1",
            );
            crate::tour::emit_tour(node, &desc, state, label)
        }
        WidgetCatalogId::Watermark => {
            let desc = crate::watermark::WatermarkDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 84.0),
                "CONFIDENTIAL",
            );
            crate::watermark::emit_watermark(&desc, state, label)
        }
    }
}

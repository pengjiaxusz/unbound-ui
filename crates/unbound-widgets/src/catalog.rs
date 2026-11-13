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

use unbound_gpu::data::GpuPrimitiveInstance;
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
    VirtualRowTokens, WidgetCatalogId, WidgetShowcaseWitness,
};

/// The standard probe rectangle, comfortably above the minimum target size.
const RECT: Rect = Rect::new(40.0, 40.0, 220.0, 36.0);

/// The viewport overlay components place themselves into.
const VIEWPORT: Rect = Rect::new(0.0, 0.0, 1280.0, 720.0);

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
    pub primitives: Vec<GpuPrimitiveInstance>,
}

/// Run the genuine emitter for `id` under `state`, with a minimal but valid
/// descriptor. Exhaustive: NO `_ =>` wildcard arm allowed.
#[deny(unreachable_patterns)]
pub fn probe_widget(id: WidgetCatalogId, state: ControlState) -> CatalogProbe {
    let primitives = match id {
        WidgetCatalogId::Accordion => {
            let desc = AccordionDesc::new(
                Rect::new(40.0, 40.0, 300.0, 400.0),
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
                &label(),
                AccordionTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Alert => {
            let desc = AlertDesc::new(
                Rect::new(40.0, 40.0, 400.0, 64.0),
                Severity::Warning,
                "Low disk space",
            )
            .dismissible();
            crate::alert::emit_alert(1, &desc, &[], state).primitives
        }
        WidgetCatalogId::AlertDialog => {
            let desc =
                AlertDialogDesc::new(Rect::new(340.0, 300.0, 420.0, 200.0), "Delete workspace?")
                    .with_message("This cannot be undone.");
            crate::alert_dialog::emit_alert_dialog(1, VIEWPORT, &desc, &[], state)
                .output
                .primitives
        }
        WidgetCatalogId::Avatar => {
            let desc = AvatarDesc::new(Rect::new(40.0, 40.0, 40.0, 40.0), "Ada")
                .presence(Presence::Online);
            crate::avatar::emit_avatar(1, &desc, &[], state).primitives
        }
        WidgetCatalogId::Badge => {
            // The chip is the interactive half of the badge module: it answers
            // the pointer, while a plain badge is a static label.
            let desc = ChipDesc::new(Rect::new(40.0, 40.0, 80.0, 24.0), "PNG").closable();
            crate::badge::emit_chip(1, &desc, state, &label()).primitives
        }
        WidgetCatalogId::Breadcrumb => {
            let desc = BreadcrumbDesc::new(
                Rect::new(40.0, 40.0, 400.0, 24.0),
                vec![Crumb::new(1, "Root"), Crumb::new(2, "src")],
            );
            let slots = desc.layout(&[40.0, 30.0]);
            crate::breadcrumb::emit_crumb(
                &desc,
                &slots[0],
                state,
                &label(),
                BreadcrumbTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Button => {
            let desc = ButtonDesc::new(RECT, "Save");
            crate::button::emit_button(1, &desc, state, &mut label()).primitives
        }
        WidgetCatalogId::CanvasToolbar => {
            let desc = CanvasToolbarDesc::new(VIEWPORT);
            let toolbar = CanvasToolbarState::new(1.0);
            crate::canvas_toolbar::emit_canvas_toolbar(1, &desc, &toolbar, &[], state)
                .output
                .primitives
        }
        WidgetCatalogId::Chart => {
            let desc = ChartDesc::new(Rect::new(40.0, 40.0, 400.0, 240.0)).with_series(vec![
                ChartSeries::line(
                    vec![
                        Vec2::new(0.0, 0.0),
                        Vec2::new(1.0, 5.0),
                        Vec2::new(2.0, 2.0),
                    ],
                    Token::PRIMARY,
                ),
            ]);
            crate::chart::emit_chart(1, &desc, &ChartState::new(), &[], state)
                .output
                .primitives
        }
        WidgetCatalogId::Checkbox => {
            let desc = CheckboxDesc::new(RECT, "Snap to grid");
            crate::checkbox::emit_checkbox(
                1,
                &desc,
                state,
                &[],
                &label(),
                CheckboxTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::CodeBlock => {
            let desc = CodeBlockDesc::new(
                Rect::new(40.0, 40.0, 400.0, 120.0),
                "fn main() {\n    let x = 42;\n}",
                "rust",
            );
            crate::code_block::emit_code_block(1, &desc, &CodeBlockState::new(), &[], state)
                .primitives
        }
        WidgetCatalogId::ColorPicker => {
            let desc = ColorPickerDesc::new(Rect::new(40.0, 40.0, 240.0, 320.0));
            let picker = ColorPickerState::new(Color::from_srgb8(24, 128, 200, 255));
            crate::color_picker::emit_color_picker(1, &desc, &picker, state, &mut label())
                .primitives
        }
        WidgetCatalogId::Combobox => {
            let desc = ComboboxDesc::new(
                RECT,
                vec![
                    ComboItem::new("apple", "Apple"),
                    ComboItem::new("banana", "Banana"),
                ],
            );
            let combo = ComboboxState::new(&desc);
            crate::combobox::emit_combobox_trigger(1, &desc, &combo, state, &mut label()).primitives
        }
        WidgetCatalogId::CommandPalette => {
            let desc = CommandPaletteDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0));
            let palette = CommandPaletteState::new(vec![
                Command::new(1, "Open File", "File").with_shortcut("Ctrl+O"),
                Command::new(2, "Rebuild", "Build"),
            ]);
            crate::command_palette::emit_command_row(
                &desc,
                &palette,
                0,
                state,
                &label(),
                &[],
                CommandPaletteTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Containers => {
            let desc = CardDesc::new(Rect::new(40.0, 40.0, 300.0, 200.0))
                .with_header(40.0)
                .with_footer(30.0);
            crate::containers::emit_card(&desc, CardTokens::standard()).primitives
        }
        WidgetCatalogId::ContextMenu => {
            // The context menu is a placement state machine: it renders
            // through the genuine menu emitter on the descriptor it places.
            let mut context = ContextMenuState::new();
            context.open_at(Vec2::new(40.0, 40.0), 1, None);
            let menu = MenuDesc::new(
                Rect::new(0.0, 0.0, 200.0, 0.0),
                vec![
                    MenuItem::action("Cut").with_shortcut("Ctrl+X"),
                    MenuItem::action("Copy").with_shortcut("Ctrl+C"),
                ],
            );
            let placed = context.desc_at(menu, VIEWPORT);
            crate::menu::emit_menu(1, 2, &placed, &context.menu, state)
                .0
                .primitives
        }
        WidgetCatalogId::CurveEditor => {
            let desc = CurveEditorDesc::new(Rect::new(40.0, 40.0, 240.0, 200.0));
            crate::curve_editor::emit_curve_editor(1, &desc, &CurveEditorState::new(), state)
                .primitives
        }
        WidgetCatalogId::DatePicker => {
            let desc = DatePickerDesc::new(Rect::new(40.0, 40.0, 280.0, 320.0));
            let today = NaiveDate::new(2024, 1, 15).expect("valid probe date");
            let calendar = CalendarState::new(today, SelectionMode::Single);
            crate::date_picker::emit_calendar(1, &desc, &calendar, state, &mut label()).primitives
        }
        WidgetCatalogId::Divider => {
            let desc = DividerDesc::horizontal(Rect::new(40.0, 40.0, 300.0, 8.0));
            crate::divider::emit_divider(&desc, &[], 0.0, DividerTokens::standard()).primitives
        }
        WidgetCatalogId::Dock => {
            let pane = DockTree::new(vec![DockTab::new(1, "Explorer"), DockTab::new(2, "Search")])
                .layout(Rect::new(40.0, 40.0, 400.0, 300.0))
                .remove(0);
            crate::dock::emit_dock_tab(&pane, 1, 128.0, state, &label(), DockTokens::standard(), 10)
                .primitives
        }
        WidgetCatalogId::Drawer => {
            let desc = DrawerDesc::new(VIEWPORT, DrawerSide::Right, 320.0);
            let drawer = DrawerState::new(true);
            crate::drawer::emit_drawer(1, &desc, &drawer, &[10, 11], &[], state)
                .output
                .primitives
        }
        WidgetCatalogId::FileDrop => {
            let desc = FileDropDesc::new(Rect::new(40.0, 40.0, 200.0, 100.0));
            crate::file_drop::emit_file_drop(1, &desc, &FileDropState::new(), state, &mut label())
                .primitives
        }
        WidgetCatalogId::Form => {
            // The form module owns layout and validation; its painted parts
            // are the field label (with the required marker) and the message.
            let desc = FormDesc::new(Rect::new(40.0, 40.0, 320.0, 400.0));
            let fields = vec![FormField::new(
                1,
                FormFieldDesc::new("Name").required(true).help("Full name"),
            )];
            let layouts = layout_form(&desc, &fields);
            crate::form::emit_form_label(&fields[0], &layouts[0], &mut label()).primitives
        }
        WidgetCatalogId::Gizmo => {
            let desc = GizmoDesc::new(Rect::new(100.0, 100.0, 200.0, 100.0), 2.0);
            crate::gizmo::emit_gizmo(1, &desc, None, &[], &[], state)
                .output
                .primitives
        }
        WidgetCatalogId::HoverCard => {
            let desc = HoverCardDesc::new(
                Rect::new(200.0, 200.0, 120.0, 32.0),
                VIEWPORT,
                Vec2::new(240.0, 160.0),
            );
            crate::hover_card::emit_hover_card(1, 2, &desc, &[], state)
                .0
                .primitives
        }
        WidgetCatalogId::Image => {
            let desc = ImageDesc::new(
                Rect::new(40.0, 40.0, 200.0, 100.0),
                Vec2::new(100.0, 100.0),
                7,
            )
            .with_alt("portrait");
            crate::image::emit_image(1, &desc, state).primitives
        }
        WidgetCatalogId::Menu => {
            let desc = MenuDesc::new(
                Rect::new(100.0, 100.0, 220.0, 150.0),
                vec![
                    MenuItem::action("Open").with_shortcut("Ctrl+O"),
                    MenuItem::separator(),
                    MenuItem::action("Quit"),
                ],
            );
            crate::menu::emit_menu(1, 2, &desc, &MenuState::new(), state)
                .0
                .primitives
        }
        WidgetCatalogId::Menubar => {
            let desc = MenuBarDesc::new(
                Rect::new(0.0, 0.0, 400.0, crate::menubar::MENUBAR_HEIGHT),
                vec![MenuBarItem::new(1, "File"), MenuBarItem::new(2, "Edit")],
            );
            let rect = desc.layout(&[])[0].0;
            crate::menubar::emit_menubar_item(
                &desc,
                &MenuBarState::new(),
                0,
                rect,
                state,
                &label(),
                MenuBarTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Minimap => {
            let desc = MinimapDesc::new(
                Rect::new(1000.0, 500.0, 200.0, 150.0),
                Rect::new(0.0, 0.0, 1000.0, 1000.0),
                Rect::new(200.0, 100.0, 400.0, 300.0),
            );
            crate::minimap::emit_minimap(1, &desc, &[], &MinimapState::new(), state)
                .0
                .primitives
        }
        WidgetCatalogId::Modal => {
            let desc = ModalDesc::new(Rect::new(340.0, 160.0, 600.0, 400.0)).with_label("Settings");
            crate::modal::emit_modal(1, VIEWPORT, &desc, &[10, 11], state)
                .output
                .primitives
        }
        WidgetCatalogId::NodeGraph => {
            let nodes = vec![
                GraphNodeDesc::new(1, Vec2::new(0.0, 40.0), Vec2::new(160.0, 120.0), "Node")
                    .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")]),
                GraphNodeDesc::new(2, Vec2::new(300.0, 40.0), Vec2::new(160.0, 120.0), "Node")
                    .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")]),
            ];
            let wires = [GraphWire {
                from_node: 1,
                from_pin: 0,
                to_node: 2,
                to_pin: 0,
            }];
            crate::node_graph::emit_node_graph(
                1,
                &nodes,
                &wires,
                &NodeGraphState::new(),
                &[Vec::new(), Vec::new()],
            )
            .output
            .primitives
        }
        WidgetCatalogId::NumberInput => {
            let desc = NumberInputDesc::new(RECT, 0.0, 100.0);
            let number = NumberInputState::new(10.0);
            crate::number_input::emit_number_input(1, &desc, &number, state, &mut label())
                .primitives
        }
        WidgetCatalogId::Overlay => {
            // The overlay module is pure placement and stack bookkeeping
            // (`place_anchored_popup`, `OverlayStack`): it emits NO primitives
            // of its own. The visible overlay surfaces are painted by the
            // popover/menu/tooltip/modal emitters, which are probed under
            // their own catalog ids. This arm is therefore legitimately empty.
            Vec::new()
        }
        WidgetCatalogId::Popover => {
            let desc = PopoverDesc::new(
                Rect::new(200.0, 200.0, 120.0, 36.0),
                VIEWPORT,
                Vec2::new(240.0, 200.0),
            );
            crate::popover::emit_popover(1, 2, &desc, state)
                .output
                .primitives
        }
        WidgetCatalogId::Progress => {
            let desc = ProgressDesc::new(Rect::new(40.0, 40.0, 200.0, 8.0)).value(0.4);
            crate::progress::emit_progress(&desc, &ProgressState::new()).primitives
        }
        WidgetCatalogId::PropertyGrid => {
            let desc = PropertyGridDesc::new(
                Rect::new(40.0, 40.0, 300.0, 200.0),
                vec![PropertySection::new("Transform")
                    .with_row(PropertyDesc::new("X", PropertyValue::Number(10.0)))],
            );
            crate::property_grid::emit_property_grid(
                1,
                &desc,
                &PropertyGridState::new(),
                &[],
                state,
            )
            .primitives
        }
        WidgetCatalogId::Radio => {
            let desc = RadioDesc::new(RECT, "Comfortable");
            crate::radio::emit_radio(1, &desc, state, &label(), RadioTokens::standard()).primitives
        }
        WidgetCatalogId::RichText => {
            let desc = RichTextDesc::new(Rect::new(40.0, 40.0, 300.0, 200.0));
            let document = parse_markdown("# Title\n\nhello");
            let layout = desc.layout(&document, &[28.0, 20.0]);
            crate::rich_text::emit_rich_text(1, &desc, &document, &layout, &[], state).primitives
        }
        WidgetCatalogId::Ruler => {
            let desc = RulerDesc::new(Rect::new(60.0, 60.0, 800.0, 600.0), 1.0, Vec2::ZERO);
            crate::ruler::emit_rulers(1, &desc, &RulerState::new(), state)
                .output
                .primitives
        }
        WidgetCatalogId::Scroll => {
            let desc = ScrollViewDesc::new(
                Rect::new(40.0, 40.0, 400.0, 300.0),
                Vec2::new(400.0, 3000.0),
            );
            crate::scroll::emit_scroll_view(1, &desc, &ScrollState::new(), state)
                .output
                .primitives
        }
        WidgetCatalogId::Segmented => {
            // Probe an *unselected* slot: it is the one that answers the
            // pointer (the active slot is the selection itself).
            let desc =
                SegmentedDesc::new(Rect::new(40.0, 40.0, 300.0, 32.0), &["One", "Two", "Three"]);
            crate::segmented::emit_segmented_segment(
                1,
                &desc,
                1,
                state,
                &label(),
                SegmentedTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Select => {
            let desc = SelectDesc::new(RECT, vec!["Apple".into(), "Banana".into()]);
            crate::select::emit_select(1, &desc, &SelectState::new(), state).primitives
        }
        WidgetCatalogId::Skeleton => {
            let desc = SkeletonDesc::new(Rect::new(40.0, 40.0, 200.0, 16.0)).animated();
            crate::skeleton::emit_skeleton(&desc, &SkeletonState::new()).primitives
        }
        WidgetCatalogId::Slider => {
            let desc = SliderDesc::new(RECT, "Gap", 18.0, 0.0, 36.0).with_step(1.0);
            crate::slider::emit_slider(1, &desc, state, &label(), SliderTokens::standard())
                .primitives
        }
        WidgetCatalogId::SplitPane => {
            let desc = SplitPaneDesc::horizontal(Rect::new(40.0, 40.0, 500.0, 300.0));
            crate::split_pane::emit_split_pane(
                1,
                &desc,
                &SplitPaneState::new(0.5),
                state,
                SplitPaneTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Switch => {
            let desc = SwitchDesc::new(RECT, "Animate");
            crate::switch::emit_switch(1, &desc, state, &label(), SwitchTokens::standard())
                .primitives
        }
        WidgetCatalogId::Table => {
            let desc = TableDesc::new(
                Rect::new(40.0, 40.0, 320.0, 200.0),
                vec![
                    ColumnSpec::new("Name", 160.0),
                    ColumnSpec::new("Size", 80.0).right_aligned(),
                ],
            )
            .with_label("Files");
            crate::table::emit_table(1, &desc, &TableState::new(), &[], &label(), state).primitives
        }
        WidgetCatalogId::Tabs => {
            // Probe an inactive tab: the active one is the selection itself.
            let desc = TabsDesc::new(Rect::new(40.0, 40.0, 420.0, 36.0), &["One", "Two", "Three"]);
            crate::tabs::emit_tab(
                &desc,
                &TabsState::new(0),
                1,
                state,
                &label(),
                TabsTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::TagInput => {
            let desc = TagInputDesc::new(Rect::new(40.0, 40.0, 300.0, 80.0));
            let tags = TagInputState::with_tags(["rust".to_owned()]);
            let layout = layout_tags(&desc, &tags, |tag| tag.len() as f32 * 8.0);
            crate::tag_input::emit_tag_input(1, &desc, &tags, state, &layout, &mut label())
                .primitives
        }
        WidgetCatalogId::TextArea => {
            let desc = TextAreaDesc::new(Rect::new(40.0, 40.0, 260.0, 120.0));
            let area = TextAreaState::with_text("one\ntwo");
            crate::text_area::emit_text_area(
                1,
                &desc,
                &area,
                state,
                &TextAreaVisual::default(),
                &mut label(),
            )
            .primitives
        }
        WidgetCatalogId::TextInput => {
            let desc = TextInputDesc::new(RECT).with_placeholder("type here");
            crate::text_input::emit_text_input(
                1,
                &desc,
                state,
                &TextEditState::new(),
                &TextInputVisual::default(),
                &mut label(),
            )
            .primitives
        }
        WidgetCatalogId::Toast => {
            let mut entry =
                ToastEntry::new(1, "Saved", Severity::Success).with_message("All changes stored.");
            entry.y_spring.value = 0.0;
            entry.opacity_spring.value = 1.0;
            crate::toast::emit_toast(1, &entry, Rect::new(900.0, 600.0, 320.0, 64.0), &[], state)
                .output
                .primitives
        }
        WidgetCatalogId::Toolbar => {
            let desc = ToolbarDesc::new(
                Rect::new(40.0, 40.0, 300.0, 32.0),
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
                &label(),
                ToolbarTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::Tooltip => {
            let desc = TooltipDesc::new(
                Rect::new(100.0, 100.0, 80.0, 32.0),
                VIEWPORT,
                Vec2::new(120.0, 28.0),
            );
            crate::tooltip::emit_tooltip(1, 2, &desc, state)
                .0
                .primitives
        }
        WidgetCatalogId::TreeView => {
            let desc = TreeViewDesc::new(Rect::new(40.0, 40.0, 300.0, 260.0));
            let row = TreeViewRow {
                row: TreeRow {
                    id: 1,
                    depth: 0,
                    has_children: true,
                    expanded: false,
                },
                item: 0,
                rect: Rect::new(40.0, 40.0, 300.0, 28.0),
            };
            crate::tree_view::emit_tree_row(
                &desc,
                &row,
                false,
                state,
                &label(),
                TreeViewTokens::standard(),
            )
            .primitives
        }
        WidgetCatalogId::VirtualViews => {
            // The row emitter takes the hover bit directly; derive it from
            // the control phase so the probe still varies only ControlState.
            crate::virtual_views::emit_virtual_row(
                Rect::new(40.0, 40.0, 220.0, 28.0),
                &label(),
                false,
                state.phase == PointerPhase::Hovered,
                VirtualRowTokens::standard(),
                100,
            )
            .primitives
        }
    };
    CatalogProbe {
        witness: WidgetShowcaseWitness::new(id),
        primitives,
    }
}

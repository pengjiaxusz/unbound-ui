//! 穷举组件探针：为每个目录 ID 调用一次真实发射器。
//!
//! [`probe_widget`] 是展示页的编译期门禁：其 `match` 显式列出
//! [`WidgetCatalogId`] 的每个变体且不允许通配符，因此向目录添加组件而未在此处
//! 接入真实发射器将触发编译错误（`E0004`），而非静默遗漏展示分支。
//!
//! 每个分支均构造最小但有效的描述符（与组件自身单元测试一致），
//! 调用组件公开的 `emit_*` 函数并返回发射的图元。每次调用唯一变化的输入是
//! [`ControlState`]；其余附加状态结构体均为固定的代表性值。返回的
//! [`WidgetShowcaseWitness`] 证明发射器确实已运行——它只能在本 crate 内部铸造。
//!
//! [`emit_widget`] 是同一穷举匹配的完整输出形式：它将组件放置在调用方指定的
//! 矩形内，使用调用方排版的字形，并返回完整的 [`WidgetOutput`]（图元、载荷、
//! 命中区域与语义节点）。Showcase 组件画廊直接驱动它，因此每个目录组件均由其
//! 真实发射器配合真实身份绘制，而非手绘替身。`probe_widget` 委托给它，
//! 从而为编译期门禁保持单一事实来源。

use unbound_math::{Color, Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::virtualize::TreeRow;

use crate::{
    layout_form, layout_tags, parse_markdown, AccordionDesc, AccordionSection, AccordionState,
    AccordionTokens, AlertDesc, AlertDialogDesc, AspectRatioBoxDesc, AvatarDesc, AvatarGroupDesc,
    BadgeDesc, BreadcrumbDesc, BreadcrumbState, BreadcrumbTokens, ButtonDesc, CalendarState,
    CanvasToolbarDesc, CanvasToolbarState, CardDesc, CardTokens, ChartDesc, ChartSeries,
    ChartState, CheckboxDesc, CheckboxTokens, ChipDesc, CodeBlockDesc, CodeBlockState,
    ColorPickerDesc, ColorPickerState, ColumnSpec, ComboItem, ComboboxDesc, ComboboxState, Command,
    CommandPaletteDesc, CommandPaletteState, CommandPaletteTokens, ContextMenuState, ControlState,
    Crumb, CurveEditorDesc, CurveEditorState, DatePickerDesc, DividerDesc, DividerTokens, DockTab,
    DockTokens, DockTree, DrawerDesc, DrawerSide, DrawerState, EmptyStateDesc, FileDropDesc,
    FileDropState, FormDesc, FormField, FormFieldDesc, GizmoDesc, GradientEditorState,
    GraphNodeDesc, GraphWire, HoverCardDesc, IconDesc, ImageAssetResolver, ImageDesc,
    LoadingOverlayDesc, MenuBarDesc, MenuBarItem, MenuBarState, MenuBarTokens, MenuDesc, MenuItem,
    MenuState, MinimapDesc, MinimapState, ModalDesc, NaiveDate, NinePatchDesc, NodeGraphState,
    NumberInputDesc, NumberInputState, PinSpec, PointerPhase, PopoverDesc, Presence, ProgressDesc,
    ProgressState, PropertyDesc, PropertyGridDesc, PropertyGridState, PropertySection,
    PropertyValue, RadioDesc, RadioGroupDesc, RadioTokens, ResolvedImage, RichTextDesc, RulerDesc,
    RulerState, ScrollState, ScrollViewDesc, SegmentedDesc, SegmentedTokens, SelectDesc,
    SelectState, SelectionMode, Severity, SkeletonDesc, SkeletonState, SliderDesc, SliderTokens,
    SplitPaneDesc, SplitPaneState, SplitPaneTokens, StackDesc, StatusBarDesc, SwitchDesc,
    SwitchTokens, TableDesc, TableState, TabsDesc, TabsState, TabsTokens, TagInputDesc,
    TagInputState, TextAreaDesc, TextAreaState, TextAreaVisual, TextEditState, TextInputDesc,
    TextInputVisual, ToastEntry, ToolbarDesc, ToolbarItem, ToolbarState, ToolbarTokens,
    TooltipDesc, TreeViewDesc, TreeViewRow, TreeViewTokens, VirtualRowTokens, WidgetCatalogId,
    WidgetOutput, WidgetShowcaseWitness, WrapDesc,
};

struct ProbeImageResolver;
impl ImageAssetResolver for ProbeImageResolver {
    fn resolve(&self, _id: unbound_asset::AssetId) -> Option<ResolvedImage> {
        None
    }
}

/// 标准探针矩形，尺寸充裕且高于最小目标尺寸。
const RECT: Rect = Rect::new(40.0, 40.0, 220.0, 36.0);

/// 小型固定字形串，模拟六字符标签的排版结果。
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

/// 探测单个组件的结果：证明真实发射器已运行，并附带图元列表供稳定形状测试计数。
pub struct CatalogProbe {
    /// 组件真实发射器已运行的证明。
    pub witness: WidgetShowcaseWitness,
    /// 发射器按绘制顺序产出的图元。
    pub primitives: Vec<unbound_gpu::data::GpuPrimitiveInstance>,
}

/// 在 `state` 下使用最小有效描述符运行 `id` 的真实发射器。穷举匹配：禁止 `_ =>` 通配符分支。
#[deny(unreachable_patterns)]
pub fn probe_widget(id: WidgetCatalogId, state: ControlState) -> CatalogProbe {
    let output = emit_widget(id, 1, state, RECT, &mut label());
    CatalogProbe {
        witness: WidgetShowcaseWitness::new(id),
        primitives: output.primitives,
    }
}

/// 穷举组件匹配的完整输出形式。
///
/// 将组件放置在 `rect`（同时用作边界，对浮层组件也用作放置视口），
/// 绑定到 `node` 并基于 `label` 排版主标签。返回完整的 [`WidgetOutput`]，
/// 使调用方在获得图元的同时获得真实的命中区域与语义节点。
///
/// 穷举匹配：禁止 `_ =>` 通配符分支——这是确保每个目录组件都接入真实发射器的唯一编译期门禁。
#[deny(unreachable_patterns)]
pub fn emit_widget(
    id: WidgetCatalogId,
    node: u64,
    state: ControlState,
    rect: Rect,
    label: &mut [GlyphQuad],
) -> WidgetOutput {
    emit_widget_with_params(id, node, state, rect, label, 0.5, 0)
}

/// 带交互参数（归一化数值 `value_norm` 与选项索引 `variant_idx`）的穷举组件发射器。
#[deny(unreachable_patterns)]
pub fn emit_widget_with_params(
    id: WidgetCatalogId,
    node: u64,
    state: ControlState,
    rect: Rect,
    label: &mut [GlyphQuad],
    value_norm: f32,
    variant_idx: usize,
) -> WidgetOutput {
    let value_norm = value_norm.clamp(0.0, 1.0);
    let o = rect.origin;
    let mut out = match id {
        WidgetCatalogId::Accordion => {
            let desc = AccordionDesc::new(
                Rect::new(o.x, o.y, rect.size.x, rect.size.y.max(200.0)),
                vec![
                    AccordionSection::new(1, "General", 100.0),
                    AccordionSection::new(2, "Advanced", 80.0),
                ],
            );
            let accordion = AccordionState::new(2);
            let mut out = crate::accordion::emit_accordion_header(
                &desc,
                &accordion,
                0,
                state,
                label,
                AccordionTokens::standard(),
            );
            let body = crate::accordion::emit_accordion_body(
                &desc,
                &accordion,
                0,
                AccordionTokens::standard(),
            );
            out.primitives.extend(body.primitives);
            out
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
            let desc = BadgeDesc::new(Rect::new(o.x, o.y, 64.0, 22.0), "New");
            crate::badge::emit_badge(&desc, label)
        }
        WidgetCatalogId::Breadcrumb => {
            let desc = BreadcrumbDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 24.0),
                vec![Crumb::new(1, "Root"), Crumb::new(2, "src")],
            );
            let slots = desc.layout(&[40.0, 30.0]);
            let mut out = crate::breadcrumb::emit_crumb(
                &desc,
                &slots[0],
                state,
                label,
                BreadcrumbTokens::standard(),
            );
            let sep = crate::breadcrumb::emit_crumb_separator(
                &desc,
                &slots[0],
                BreadcrumbTokens::standard(),
            );
            let ell = crate::breadcrumb::emit_crumb_ellipsis(
                node,
                &desc,
                &slots[0],
                &BreadcrumbState::default(),
                state,
                BreadcrumbTokens::standard(),
            );
            out.primitives.extend(sep.primitives);
            out.primitives.extend(ell.primitives);
            out
        }
        WidgetCatalogId::Button => {
            let desc = ButtonDesc::new(rect, "Save");
            let mut out = crate::button::emit_button(node, &desc, state, label);
            let out2 =
                crate::button::emit_button_with_tokens(node, &desc, state, desc.tokens(), label);
            out.primitives.extend(out2.primitives);
            out
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
            let mut out = crate::combobox::emit_combobox_trigger(node, &desc, &combo, state, label);
            let popup = crate::combobox::emit_combobox_popup(&desc, &combo, rect, None, label);
            out.primitives.extend(popup.primitives);
            out
        }
        WidgetCatalogId::CommandPalette => {
            let desc = CommandPaletteDesc::new(rect);
            let palette = CommandPaletteState::new(vec![
                Command::new(1, "Open File", "File").with_shortcut("Ctrl+O"),
                Command::new(2, "Rebuild", "Build"),
            ]);
            let mut out = crate::command_palette::emit_command_row(
                &desc,
                &palette,
                0,
                state,
                label,
                &[],
                CommandPaletteTokens::standard(),
            );
            let pal = crate::command_palette::emit_command_palette(
                &desc,
                &palette,
                CommandPaletteTokens::standard(),
            );
            out.primitives.extend(pal.primitives);
            out
        }
        WidgetCatalogId::Containers => {
            let desc = CardDesc::new(rect).with_header(40.0).with_footer(30.0);
            crate::containers::emit_card(&desc, CardTokens::standard())
        }
        WidgetCatalogId::ContextMenu => {
            // 上下文菜单是一个放置状态机：它通过真实菜单发射器渲染所放置的描述符。
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
            let mut out = crate::dock::emit_dock_tab(
                &pane,
                1,
                128.0,
                state,
                label,
                DockTokens::standard(),
                10,
            );
            let pane_out = crate::dock::emit_dock_pane(&pane, DockTokens::standard(), 10);
            let preview_out = crate::dock::emit_dock_preview(
                crate::dock::DockHover {
                    target: 0,
                    zone: crate::dock::DockZone::Center,
                    preview: rect,
                },
                DockTokens::standard(),
                10,
            );
            out.primitives.extend(pane_out.primitives);
            out.primitives.extend(preview_out.primitives);
            out
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
            // 表单模块负责布局与校验；其绘制部分为字段标签（含必填标记）与提示消息。
            let desc = FormDesc::new(rect);
            let fields = vec![FormField::new(
                1,
                FormFieldDesc::new("Name").required(true).help("Full name"),
            )];
            let layouts = layout_form(&desc, &fields);
            let mut out = crate::form::emit_form_root(rect, "Registration");
            let label_out = crate::form::emit_form_label(&fields[0], &layouts[0], label);
            let msg_out = crate::form::emit_form_message(&fields[0], &layouts[0], None, label, 10);
            out.primitives.extend(label_out.primitives);
            out.primitives.extend(msg_out.primitives);
            out
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
            let mut out = crate::image::emit_image(node, &desc, state);
            let resolver = ProbeImageResolver;
            let asset_out = crate::image::emit_image_with_assets(node, &desc, state, &resolver);
            out.primitives.extend(asset_out.primitives);
            out
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
            // overlay 模块仅负责纯定位与栈簿记（`place_anchored_popup`、`OverlayStack`）：
            // 它自身不发射任何图元。可见的浮层表面由 popover/menu/tooltip/modal 发射器绘制，
            // 并在各自对应的目录 ID 下探测。因此该分支合理为空。
            WidgetOutput::default()
        }
        WidgetCatalogId::Popover => {
            let desc = PopoverDesc::new(
                Rect::new(o.x, o.y, 120.0, 36.0),
                rect,
                Vec2::new(o.x + 40.0, o.y),
            );
            let mut collector = crate::SceneCollector::new(0);
            crate::popover::emit_popover_scope(&mut collector, node, 2, &desc, state, []);
            crate::popover::emit_popover(node, 2, &desc, state).output
        }
        WidgetCatalogId::Progress => {
            let desc = ProgressDesc::new(Rect::new(o.x, o.y, rect.size.x, 8.0)).value(value_norm);
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
            let _group = RadioGroupDesc::new(vec![desc.clone()]);
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
            // 探测*未选中*的槽位：它是响应指针的槽位（当前激活槽位即选中项本身）。
            let desc = SegmentedDesc::new(rect, &["One", "Two", "Three"]);
            let mut out = crate::segmented::emit_segmented_segment(
                node,
                &desc,
                1,
                state,
                label,
                SegmentedTokens::standard(),
            );
            let frame = crate::segmented::emit_segmented_frame(&desc, SegmentedTokens::standard());
            out.primitives.extend(frame.primitives);
            out
        }
        WidgetCatalogId::Select => {
            let desc =
                SelectDesc::new(rect, vec!["Apple".into(), "Banana".into(), "Cherry".into()]);
            let mut select_state = SelectState::new();
            select_state.selected = Some(variant_idx % 3);
            crate::select::emit_select_with_glyphs(node, &desc, &select_state, state, label)
        }
        WidgetCatalogId::Skeleton => {
            let desc = SkeletonDesc::new(Rect::new(o.x, o.y, rect.size.x, 16.0)).animated();
            crate::skeleton::emit_skeleton(&desc, &SkeletonState::new())
        }
        WidgetCatalogId::Slider => {
            let desc = SliderDesc::new(rect, "Gap", value_norm * 36.0, 0.0, 36.0).with_step(1.0);
            crate::slider::emit_slider(node, &desc, state, label, SliderTokens::standard())
        }
        WidgetCatalogId::SplitPane => {
            let desc = SplitPaneDesc::horizontal(rect);
            let split_state = SplitPaneState::new(0.5);
            let mut out = crate::split_pane::emit_split_pane(
                node,
                &desc,
                &split_state,
                state,
                SplitPaneTokens::standard(),
            );
            let panes = crate::split_pane::emit_split_pane_panes(
                &desc,
                &split_state,
                Token::SURFACE,
                Token::SURFACE,
            );
            out.primitives.extend(panes.primitives);
            out
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
            // 探测未激活的标签页：当前激活的标签页即选中项本身。
            let desc = TabsDesc::new(rect, &["One", "Two", "Three"]);
            let tabs_state = TabsState::new(variant_idx % 3);
            let mut out =
                crate::tabs::emit_tab(&desc, &tabs_state, 1, state, label, TabsTokens::standard());
            let strip = crate::tabs::emit_tabs_strip(&desc, &tabs_state, TabsTokens::standard());
            let close = crate::tabs::emit_tab_close(
                node,
                &desc,
                &tabs_state,
                1,
                state,
                TabsTokens::standard(),
            );
            let add = crate::tabs::emit_tabs_add(node, &desc, state, TabsTokens::standard());
            out.primitives.extend(strip.primitives);
            out.primitives.extend(close.primitives);
            out.primitives.extend(add.primitives);
            out
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
            let mut out = crate::toolbar::emit_toolbar_item(
                &desc,
                &ToolbarState::new(),
                0,
                layout.rects[0].expect("button rect"),
                state,
                label,
                ToolbarTokens::standard(),
            );
            let overflow = crate::toolbar::emit_toolbar_overflow(
                node,
                Rect::new(o.x + rect.size.x - 24.0, o.y, 24.0, rect.size.y),
                state,
                ToolbarTokens::standard(),
                10,
            );
            out.primitives.extend(overflow.primitives);
            out
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
            let mut out = crate::tree_view::emit_tree_row(
                &desc,
                &row,
                false,
                state,
                label,
                TreeViewTokens::standard(),
            );
            let bg = crate::tree_view::emit_tree_background(&desc, TreeViewTokens::standard());
            out.primitives.extend(bg.primitives);
            out
        }
        WidgetCatalogId::VirtualViews => {
            // 行发射器直接接收悬停位；从控制阶段派生该位，使探针仍仅随 ControlState 变化。
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
        WidgetCatalogId::AspectRatioBox => {
            let desc = AspectRatioBoxDesc {
                bounds: rect,
                ratio: 16.0 / 9.0,
            };
            let fitted = desc.fit();
            let card = CardDesc::new(fitted);
            crate::containers::emit_card(&card, CardTokens::standard())
        }
        WidgetCatalogId::AvatarGroup => {
            let desc = AvatarGroupDesc::new(
                Rect::new(o.x, o.y, 120.0, 36.0),
                vec![
                    AvatarDesc::new(Rect::new(o.x, o.y, 36.0, 36.0), "Alice"),
                    AvatarDesc::new(Rect::new(o.x + 28.0, o.y, 36.0, 36.0), "Bob"),
                ],
            );
            crate::avatar::emit_avatar_group(&desc, &[vec![], vec![]], &[])
        }
        WidgetCatalogId::Chip => {
            let desc = ChipDesc::new(rect, "Tag");
            crate::badge::emit_chip(node, &desc, state, label)
        }
        WidgetCatalogId::EmptyState => {
            let desc = EmptyStateDesc::new(rect, "No results");
            crate::hover_card::emit_empty_state(node, &desc, label, state).output
        }
        WidgetCatalogId::GradientEditor => {
            let grad_state = GradientEditorState::new();
            crate::curve_editor::emit_gradient_editor(node, rect, &grad_state, state, 100)
        }
        WidgetCatalogId::Icon => {
            let desc = IconDesc::new(Rect::new(o.x, o.y, 24.0, 24.0), 1, Token::PRIMARY);
            crate::image::emit_icon(node, &desc, state)
        }
        WidgetCatalogId::LoadingOverlay => {
            let desc = LoadingOverlayDesc {
                rect,
                active: true,
                phase: 0.25,
                label: "Loading...".to_owned(),
                z: 100,
            };
            crate::spinner::emit_loading_overlay(&desc, state)
        }
        WidgetCatalogId::NinePatch => {
            let desc = NinePatchDesc::new(rect, Vec2::new(64.0, 64.0), 0, [8.0; 4]);
            crate::image::emit_nine_patch(node, &desc, state)
        }
        WidgetCatalogId::Stack => {
            let desc = StackDesc::new(rect);
            crate::containers::emit_stack(&desc, Token::SURFACE)
        }
        WidgetCatalogId::StatusBar => {
            let seg = crate::toolbar::StatusSegment::left(1, "Ready");
            let desc =
                StatusBarDesc::new(Rect::new(o.x, o.y, rect.size.x, 24.0), vec![seg.clone()]);
            let mut out = crate::toolbar::emit_status_bar(&desc, ToolbarTokens::standard());
            let seg_out = crate::toolbar::emit_status_segment(
                &desc,
                Rect::new(o.x, o.y, 80.0, 24.0),
                &seg,
                label,
                ToolbarTokens::standard(),
            );
            out.primitives.extend(seg_out.primitives);
            out
        }
        WidgetCatalogId::Wrap => {
            let desc = WrapDesc::new(rect, 24.0);
            let card = CardDesc::new(desc.rect);
            crate::containers::emit_card(&card, CardTokens::standard())
        }
        WidgetCatalogId::SplitButton => {
            let desc = crate::split_button::SplitButtonDesc::new(rect, "Action");
            let s = crate::split_button::SplitButtonState::new();
            crate::split_button::emit_split_button(node, &desc, &s, state)
        }
        WidgetCatalogId::ToggleButton => {
            let desc = crate::toggle_button::ToggleButtonDesc::new(rect, "Toggle", state.selected);
            let s = crate::toggle_button::ToggleButtonState::new(state.selected);
            crate::toggle_button::emit_toggle_button(node, &desc, &s, state)
        }
        WidgetCatalogId::TreeTable => {
            let desc = crate::tree_table::TreeTableDesc::new(
                rect,
                vec![
                    crate::table::ColumnSpec::new("Name", 200.0),
                    crate::table::ColumnSpec::new("Size", 100.0),
                ],
            );
            let s = crate::tree_table::TreeTableState::new();
            crate::tree_table::emit_tree_table(node, &desc, &s, state)
        }
        WidgetCatalogId::MultiSplit => {
            let desc = crate::multi_split::MultiSplitDesc::new(rect, 3);
            let s = crate::multi_split::MultiSplitState::new(3);
            crate::multi_split::emit_multi_split(node, &desc, &s, state)
        }
        WidgetCatalogId::Descriptions => {
            let desc = crate::descriptions::DescriptionsDesc::new(rect, "Details");
            let s = crate::descriptions::DescriptionsState::new();
            crate::descriptions::emit_descriptions(node, &desc, &s, state)
        }
        WidgetCatalogId::Callout => {
            let desc = crate::callout::CalloutDesc::new(rect, "Notice", Severity::Info);
            let s = crate::callout::CalloutState::new();
            crate::callout::emit_callout(node, &desc, &s, state)
        }
        WidgetCatalogId::VirtualMasonry => {
            let desc = crate::virtual_masonry::VirtualMasonryDesc::new(rect, 3);
            let s = crate::virtual_masonry::VirtualMasonryState::new();
            crate::virtual_masonry::emit_virtual_masonry(node, &desc, &s, state)
        }
        WidgetCatalogId::TerminalView => {
            let desc = crate::terminal_view::TerminalViewDesc::new(rect);
            let s = crate::terminal_view::TerminalViewState::new();
            crate::terminal_view::emit_terminal_view(node, &desc, &s, state)
        }
    };
    if out.hit.is_none() {
        out.hit = Some(unbound_input::HitRegion::new(node, 0, 0, rect));
    }
    out
}

#[cfg(test)]
mod interactive_tests {
    use super::*;

    #[test]
    fn emit_widget_with_params_propagates_state_and_fallback_hit() {
        let rect = Rect::new(0.0, 0.0, 240.0, 40.0);
        let mut glyphs = label();

        let toggle_on = emit_widget_with_params(
            WidgetCatalogId::ToggleButton,
            10,
            ControlState {
                selected: true,
                ..ControlState::default()
            },
            rect,
            &mut glyphs,
            0.5,
            0,
        );
        let toggle_off = emit_widget_with_params(
            WidgetCatalogId::ToggleButton,
            10,
            ControlState::default(),
            rect,
            &mut glyphs,
            0.5,
            0,
        );
        assert_ne!(
            toggle_on.primitives[0].fill_token_or_uv,
            toggle_off.primitives[0].fill_token_or_uv
        );

        for &id in &[WidgetCatalogId::Progress, WidgetCatalogId::Skeleton] {
            let out =
                emit_widget_with_params(id, 11, ControlState::default(), rect, &mut glyphs, 0.8, 0);
            assert!(out.hit.is_some(), "{id:?} 应具备兜底命中区域以支持展厅交互");
        }
    }
}

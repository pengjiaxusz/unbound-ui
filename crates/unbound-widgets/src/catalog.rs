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

/// 合并子组件输出，重定位载荷索引并保留子命中区域与无障碍节点。
fn merge_output(dst: &mut WidgetOutput, mut src: WidgetOutput) {
    let payload_base = u32::try_from(dst.payloads.len()).unwrap_or(u32::MAX);
    if payload_base != 0 {
        for primitive in &mut src.primitives {
            primitive.extra_payload_idx = primitive.extra_payload_idx.saturating_add(payload_base);
        }
    }
    dst.primitives.extend(src.primitives);
    dst.payloads.extend(src.payloads);
    if dst.hit.is_none() {
        dst.hit = src.hit;
    } else if let Some(hit) = src.hit {
        dst.sub_hits.push(hit);
    }
    dst.sub_hits.extend(src.sub_hits);
    if dst.a11y.is_none() {
        dst.a11y = src.a11y;
    }
    dst.sub_a11y.extend(src.sub_a11y);
    dst.focusable |= src.focusable;
    dst.sub_focus_order.extend(src.sub_focus_order);
}

/// 展厅组件发射器的交互旋钮参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatalogParams {
    /// 归一化数值（`0.0..=1.0`）。
    pub value_norm: f32,
    /// 选项 / 变体索引。
    pub variant_idx: usize,
    /// 是否显示图标。
    pub show_icon: bool,
    /// 是否显示边框。
    pub bordered: bool,
    /// 是否处于激活状态。
    pub active: bool,
    /// 是否紧凑模式。
    pub compact: bool,
    /// 是否只读模式。
    pub read_only: bool,
}

impl Default for CatalogParams {
    fn default() -> Self {
        Self {
            value_norm: 0.5,
            variant_idx: 0,
            show_icon: true,
            bordered: true,
            active: true,
            compact: false,
            read_only: false,
        }
    }
}

/// 辅助函数：将一组文本字形居中放置在目标矩形内（假设 `shape` 从 `origin` 开始水平排版）。
fn shape_centered<F>(shape: &mut F, text: &str, rect: Rect, font_size: f32) -> Vec<GlyphQuad>
where
    F: FnMut(&str, Vec2, f32) -> Vec<GlyphQuad>,
{
    let origin = Vec2::new(
        rect.origin.x + 8.0,
        rect.origin.y + (rect.size.y - font_size) * 0.5,
    );
    let mut quads = shape(text, origin, font_size);
    if let (Some(first), Some(last)) = (quads.first(), quads.last()) {
        let text_w = (last.rect.max().x - first.rect.origin.x).max(0.0);
        let target_x = rect.origin.x + ((rect.size.x - text_w) * 0.5).max(4.0);
        let dx = target_x - first.rect.origin.x;
        for q in &mut quads {
            q.rect.origin.x += dx;
        }
    }
    quads
}

/// 辅助函数：将字形切片追加到输出中。
fn push_glyphs(out: &mut WidgetOutput, quads: &[GlyphQuad], color: Token, z: u16) {
    crate::primitive::glyphs(quads, color, z, &mut out.primitives, &mut out.payloads);
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
    let params = CatalogParams {
        value_norm,
        variant_idx,
        ..CatalogParams::default()
    };
    emit_widget_with_shaper(id, node, state, rect, params, |_text, _origin, _size| {
        label.to_vec()
    })
}

/// 带完整文本塑形回调与交互旋钮参数的穷举组件发射器（供 Showcase 展厅与集成测试使用）。
#[deny(unreachable_patterns)]
pub fn emit_widget_with_shaper<F>(
    id: WidgetCatalogId,
    node: u64,
    state: ControlState,
    rect: Rect,
    params: CatalogParams,
    mut shape: F,
) -> WidgetOutput
where
    F: FnMut(&str, Vec2, f32) -> Vec<GlyphQuad>,
{
    let value_norm = params.value_norm.clamp(0.0, 1.0);
    let variant_idx = params.variant_idx;
    let o = rect.origin;
    let ctrl_rect = Rect::new(o.x, o.y, rect.size.x.min(320.0), rect.size.y.min(36.0));
    let mut out = match id {
        WidgetCatalogId::Accordion => {
            let desc = AccordionDesc::new(
                Rect::new(o.x, o.y, rect.size.x, rect.size.y.max(200.0)),
                vec![
                    AccordionSection::new(1, "General", 96.0),
                    AccordionSection::new(2, "Advanced", 80.0),
                ],
            );
            let mut accordion = AccordionState::new(2);
            if variant_idx % 2 == 0 {
                accordion.toggle(&desc, 0);
            } else {
                accordion.toggle(&desc, 1);
            }
            accordion.snap(&desc);
            let hdr0_r = desc.header_rect(&accordion, 0);
            let hdr0_glyphs = shape(
                "General",
                Vec2::new(
                    hdr0_r.origin.x + 30.0,
                    hdr0_r.origin.y + (hdr0_r.size.y - 13.0) * 0.5,
                ),
                13.0,
            );
            let mut out = crate::accordion::emit_accordion_header(
                &desc,
                &accordion,
                0,
                state,
                &hdr0_glyphs,
                AccordionTokens::standard(),
            );
            let body0_r = desc.body_rect(&accordion, 0);
            let body0 = crate::accordion::emit_accordion_body(
                &desc,
                &accordion,
                0,
                AccordionTokens::standard(),
            );
            merge_output(&mut out, body0);
            if body0_r.size.y > 24.0 {
                let body0_text = shape(
                    "Theme tokens, GPU pacing, and window chrome settings.",
                    Vec2::new(body0_r.origin.x + 16.0, body0_r.origin.y + 14.0),
                    12.5,
                );
                push_glyphs(&mut out, &body0_text, Token::TEXT_SECONDARY, desc.z + 1);
            }
            let hdr1_r = desc.header_rect(&accordion, 1);
            let hdr1_glyphs = shape(
                "Advanced",
                Vec2::new(
                    hdr1_r.origin.x + 30.0,
                    hdr1_r.origin.y + (hdr1_r.size.y - 13.0) * 0.5,
                ),
                13.0,
            );
            let hdr1 = crate::accordion::emit_accordion_header(
                &desc,
                &accordion,
                1,
                state,
                &hdr1_glyphs,
                AccordionTokens::standard(),
            );
            merge_output(&mut out, hdr1);
            let body1_r = desc.body_rect(&accordion, 1);
            let body1 = crate::accordion::emit_accordion_body(
                &desc,
                &accordion,
                1,
                AccordionTokens::standard(),
            );
            merge_output(&mut out, body1);
            if body1_r.size.y > 24.0 {
                let body1_text = shape(
                    "Vulkan swapchain and shader pipeline diagnostics.",
                    Vec2::new(body1_r.origin.x + 16.0, body1_r.origin.y + 14.0),
                    12.5,
                );
                push_glyphs(&mut out, &body1_text, Token::TEXT_SECONDARY, desc.z + 1);
            }
            out
        }
        WidgetCatalogId::Alert => {
            let (severity, title, msg) = match variant_idx % 4 {
                0 => (
                    Severity::Info,
                    "System update available",
                    "Version 2.4 includes Vulkan frame pacing fixes.",
                ),
                1 => (
                    Severity::Success,
                    "Build succeeded",
                    "All 99 components compiled with zero warnings.",
                ),
                2 => (
                    Severity::Warning,
                    "Low disk space",
                    "Less than 2 GB remaining on the asset cache volume.",
                ),
                _ => (
                    Severity::Error,
                    "Connection lost",
                    "Unable to reach the remote telemetry endpoint.",
                ),
            };
            let desc = AlertDesc::new(Rect::new(o.x, o.y, rect.size.x, 68.0), severity, title)
                .with_message(msg)
                .dismissible();
            let content = desc.content_rect();
            let text_x = if params.show_icon {
                content.origin.x
            } else {
                desc.rect.origin.x + crate::alert::ALERT_ACCENT_WIDTH + crate::alert::ALERT_PADDING
            };
            let mut alert_glyphs = shape(title, Vec2::new(text_x, content.origin.y), 13.0);
            let msg_glyphs = shape(msg, Vec2::new(text_x, content.origin.y + 20.0), 11.5);
            alert_glyphs.extend(msg_glyphs);
            let mut out = crate::alert::emit_alert(node, &desc, &alert_glyphs, state);
            if !params.show_icon && out.primitives.len() >= 5 {
                // 保持图元数量恒定，仅将图标背景与符号透明化
                for p in out.primitives.iter_mut().skip(2).take(3) {
                    p.z_and_opacity =
                        unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(p.depth_u16(), 0.0);
                }
            }
            out
        }
        WidgetCatalogId::AlertDialog => {
            let card_rect = if rect.size.x > 260.0 && rect.size.y > 140.0 {
                let w = rect.size.x.min(380.0);
                let h = rect.size.y.min(168.0);
                Rect::new(
                    o.x + (rect.size.x - w) * 0.5,
                    o.y + (rect.size.y - h) * 0.5,
                    w,
                    h,
                )
            } else {
                rect
            };
            let mut desc = AlertDialogDesc::new(card_rect, "Delete workspace?")
                .with_message("This action permanently removes local caches and cannot be undone.")
                .with_actions("Cancel", "Delete");
            if variant_idx % 4 == 3 {
                desc = desc.destructive();
            }
            let layout = desc.layout();
            let mut dlg_glyphs = shape(
                &desc.title,
                Vec2::new(layout.title.origin.x, layout.title.origin.y + 3.0),
                14.0,
            );
            dlg_glyphs.extend(shape(
                &desc.message,
                Vec2::new(layout.message.origin.x, layout.message.origin.y),
                12.0,
            ));
            dlg_glyphs.extend(shape_centered(
                &mut shape,
                &desc.cancel_label,
                layout.cancel,
                12.5,
            ));
            let confirm_glyphs =
                shape_centered(&mut shape, &desc.confirm_label, layout.confirm, 12.5);
            let mut out =
                crate::alert_dialog::emit_alert_dialog(node, rect, &desc, &dlg_glyphs, state)
                    .output;
            push_glyphs(&mut out, &confirm_glyphs, Token::ON_PRIMARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Avatar => {
            let scale = 0.5 + value_norm;
            let side = (48.0 * scale).clamp(28.0, rect.size.x.min(rect.size.y).max(28.0));
            let mut desc = AvatarDesc::new(Rect::new(o.x, o.y, side, side), "Ada Lovelace");
            if params.active {
                desc = desc.presence(Presence::Online);
            }
            let initials = desc.initials();
            let img_r = desc.image_rect();
            let mut glyphs = shape_centered(
                &mut shape,
                &initials,
                img_r,
                (side * 0.36).clamp(11.0, 20.0),
            );
            let _ = &mut glyphs;
            crate::avatar::emit_avatar(node, &desc, &glyphs, state)
        }
        WidgetCatalogId::Badge => {
            let (variant, text) = match variant_idx % 5 {
                0 => (crate::badge::BadgeVariant::Primary, "Badge: Active"),
                1 => (crate::badge::BadgeVariant::Success, "Badge: Ready"),
                2 => (crate::badge::BadgeVariant::Warning, "Badge: Pending"),
                3 => (crate::badge::BadgeVariant::Error, "Badge: Failed"),
                _ => (crate::badge::BadgeVariant::Neutral, "Badge: Draft"),
            };
            let badge_r = Rect::new(o.x, o.y, 124.0, 26.0);
            let desc = BadgeDesc::new(badge_r, text).variant(variant).with_dot();
            let glyphs = shape(text, Vec2::new(o.x + 20.0, o.y + 6.0), 12.5);
            crate::badge::emit_badge(&desc, &glyphs)
        }
        WidgetCatalogId::Breadcrumb => {
            let crumbs = vec![
                Crumb::new(1, "Root"),
                Crumb::new(2, "src"),
                Crumb::new(3, "main.rs"),
            ];
            let available = if variant_idx % 2 == 0 {
                rect.size.x
            } else {
                120.0
            };
            let desc = BreadcrumbDesc::new(Rect::new(o.x, o.y, available, 28.0), crumbs);
            let slots = desc.layout(&[44.0, 36.0, 68.0]);
            let mut out = WidgetOutput::default();
            for (index, slot) in slots.iter().enumerate() {
                let control = if index == 0 {
                    state
                } else {
                    ControlState::default()
                };
                let label_text = match slot.crumb {
                    Some(1) => "Root",
                    Some(2) => "src",
                    Some(3) => "main.rs",
                    _ => "",
                };
                let glyphs = if label_text.is_empty() {
                    Vec::new()
                } else {
                    shape(
                        label_text,
                        Vec2::new(
                            slot.rect.origin.x + 4.0,
                            slot.rect.origin.y + (slot.rect.size.y - 13.0) * 0.5,
                        ),
                        13.0,
                    )
                };
                let crumb = crate::breadcrumb::emit_crumb(
                    &desc,
                    slot,
                    control,
                    &glyphs,
                    BreadcrumbTokens::standard(),
                );
                merge_output(&mut out, crumb);
                let ellipsis = crate::breadcrumb::emit_crumb_ellipsis(
                    node,
                    &desc,
                    slot,
                    &BreadcrumbState::default(),
                    control,
                    BreadcrumbTokens::standard(),
                );
                merge_output(&mut out, ellipsis);
                let sep = crate::breadcrumb::emit_crumb_separator(
                    &desc,
                    slot,
                    BreadcrumbTokens::standard(),
                );
                merge_output(&mut out, sep);
            }
            out
        }
        WidgetCatalogId::Button => {
            let variant = match variant_idx % 3 {
                0 => crate::button::ButtonVariant::Primary,
                1 => crate::button::ButtonVariant::Secondary,
                _ => crate::button::ButtonVariant::Subtle,
            };
            let btn_r = Rect::new(o.x, o.y, rect.size.x.min(160.0), rect.size.y.min(36.0));
            let desc = ButtonDesc::new(btn_r, "Save Changes").with_variant(variant);
            let mut glyphs = shape_centered(&mut shape, "Save Changes", btn_r, 13.0);
            let mut out = crate::button::emit_button(node, &desc, state, &mut glyphs);
            // 覆盖显式配色发射器：使用独立的字形副本，避免二次排版偏移。
            let mut themed_glyphs = glyphs.clone();
            let themed = crate::button::emit_button_with_tokens(
                node,
                &desc,
                ControlState::default(),
                desc.tokens(),
                &mut themed_glyphs,
            );
            merge_output(&mut out, themed);
            out
        }
        WidgetCatalogId::CanvasToolbar => {
            let desc = CanvasToolbarDesc::new(rect);
            let mut toolbar = CanvasToolbarState::new(0.5 + value_norm);
            let tools = crate::canvas_toolbar::CanvasTool::ALL;
            toolbar.set_tool(tools[variant_idx % tools.len()]);
            let zoom_pct = format!("{:.0}%", toolbar.zoom * 100.0);
            let glyphs = shape(&zoom_pct, Vec2::new(o.x + 12.0, o.y + 10.0), 12.0);
            crate::canvas_toolbar::emit_canvas_toolbar(node, &desc, &toolbar, &glyphs, state).output
        }
        WidgetCatalogId::Chart => {
            let desc = ChartDesc::new(rect).with_series(vec![ChartSeries::line(
                vec![
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 5.0),
                    Vec2::new(2.0, 2.0),
                    Vec2::new(3.0, 4.2),
                ],
                Token::PRIMARY,
            )]);
            let glyphs = shape(
                "Frame Time (120 FPS)",
                Vec2::new(o.x + 12.0, o.y + 8.0),
                12.0,
            );
            crate::chart::emit_chart(node, &desc, &ChartState::new(), &glyphs, state).output
        }
        WidgetCatalogId::Checkbox => {
            let glyphs = shape(
                "Snap to grid",
                Vec2::new(o.x + 26.0, o.y + (ctrl_rect.size.y - 13.0) * 0.5),
                13.0,
            );
            // 勾选态属于展示变体：绑定 state.selected 会让图元数随控件状态漂移，
            // 且勾选态会覆盖悬停配色；默认变体保持未勾选。
            let desc =
                CheckboxDesc::new(ctrl_rect, "Snap to grid").with_checked(variant_idx % 2 == 1);
            crate::checkbox::emit_checkbox(
                node,
                &desc,
                state,
                &[],
                &glyphs,
                CheckboxTokens::standard(),
            )
        }
        WidgetCatalogId::CodeBlock => {
            let code = "fn main() {\n    let x = 42;\n    println!(\"{x}\");\n}";
            let desc = CodeBlockDesc::new(rect, code, "rust");
            let mut glyphs = Vec::new();
            for (i, line) in code.lines().enumerate() {
                glyphs.extend(shape(
                    line,
                    Vec2::new(o.x + 14.0, o.y + 34.0 + i as f32 * 18.0),
                    12.5,
                ));
            }
            crate::code_block::emit_code_block(node, &desc, &CodeBlockState::new(), &glyphs, state)
        }
        WidgetCatalogId::ColorPicker => {
            let desc = ColorPickerDesc::new(rect);
            let picker = ColorPickerState::new(Color::from_srgb8(24, 128, 200, 255));
            let mut glyphs = shape(
                "#1880C8",
                Vec2::new(o.x + 12.0, o.y + rect.size.y - 26.0),
                12.0,
            );
            crate::color_picker::emit_color_picker(node, &desc, &picker, state, &mut glyphs)
        }
        WidgetCatalogId::Combobox => {
            let desc = ComboboxDesc::new(
                ctrl_rect,
                vec![
                    ComboItem::new("apple", "Apple"),
                    ComboItem::new("banana", "Banana"),
                ],
            );
            let combo = ComboboxState::new(&desc);
            let mut trig_glyphs = shape("Apple", Vec2::new(o.x + 10.0, o.y + 10.0), 13.0);
            let mut out = crate::combobox::emit_combobox_trigger(
                node,
                &desc,
                &combo,
                state,
                &mut trig_glyphs,
            );
            let mut popup_glyphs = shape(
                "Apple",
                Vec2::new(o.x + 10.0, ctrl_rect.max().y + 12.0),
                13.0,
            );
            let popup =
                crate::combobox::emit_combobox_popup(&desc, &combo, rect, None, &mut popup_glyphs);
            out.primitives.extend(popup.primitives);
            out.payloads.extend(popup.payloads);
            out
        }
        WidgetCatalogId::CommandPalette => {
            let desc = CommandPaletteDesc::new(rect);
            let palette = CommandPaletteState::new(vec![
                Command::new(1, "Open File", "File").with_shortcut("Ctrl+O"),
                Command::new(2, "Rebuild Workspace", "Build").with_shortcut("Ctrl+B"),
            ]);
            let mut out = crate::command_palette::emit_command_palette(
                &desc,
                &palette,
                CommandPaletteTokens::standard(),
            );
            let search_glyphs = shape("Type a command...", Vec2::new(o.x + 16.0, o.y + 14.0), 13.0);
            push_glyphs(&mut out, &search_glyphs, Token::TEXT_SECONDARY, desc.z + 1);
            let row0_r = desc.row_rect(0);
            let row0_label = shape(
                "Open File",
                Vec2::new(row0_r.origin.x + 14.0, row0_r.origin.y + 8.0),
                13.0,
            );
            let row0_sc = shape(
                "Ctrl+O",
                Vec2::new(row0_r.max().x - 56.0, row0_r.origin.y + 9.0),
                11.5,
            );
            let row0 = crate::command_palette::emit_command_row(
                &desc,
                &palette,
                0,
                state,
                &row0_label,
                &row0_sc,
                CommandPaletteTokens::standard(),
            );
            merge_output(&mut out, row0);
            let row1_r = desc.row_rect(1);
            let row1_label = shape(
                "Rebuild Workspace",
                Vec2::new(row1_r.origin.x + 14.0, row1_r.origin.y + 8.0),
                13.0,
            );
            let row1_sc = shape(
                "Ctrl+B",
                Vec2::new(row1_r.max().x - 56.0, row1_r.origin.y + 9.0),
                11.5,
            );
            let row1 = crate::command_palette::emit_command_row(
                &desc,
                &palette,
                1,
                ControlState::default(),
                &row1_label,
                &row1_sc,
                CommandPaletteTokens::standard(),
            );
            merge_output(&mut out, row1);
            out
        }
        WidgetCatalogId::Containers => {
            let desc = CardDesc::new(rect).with_header(40.0).with_footer(32.0);
            let mut out = crate::containers::emit_card(&desc, CardTokens::standard());
            if let Some(hdr) = desc.header_rect() {
                let g = shape(
                    "Card Header",
                    Vec2::new(hdr.origin.x + 16.0, hdr.origin.y + 13.0),
                    13.0,
                );
                push_glyphs(&mut out, &g, Token::TEXT_PRIMARY, desc.z + 1);
            }
            let body = desc.body_rect();
            let bg = shape(
                "Card body container area",
                Vec2::new(body.origin.x + 16.0, body.origin.y + 16.0),
                12.5,
            );
            push_glyphs(&mut out, &bg, Token::TEXT_SECONDARY, desc.z + 1);
            if let Some(ftr) = desc.footer_rect() {
                let g = shape(
                    "Card Footer",
                    Vec2::new(ftr.origin.x + 16.0, ftr.origin.y + 9.0),
                    11.5,
                );
                push_glyphs(&mut out, &g, Token::TEXT_SECONDARY, desc.z + 1);
            }
            out
        }
        WidgetCatalogId::ContextMenu => {
            let mut context = ContextMenuState::new();
            context.open_at(o, 1, None);
            let menu = MenuDesc::new(
                Rect::new(o.x, o.y, 210.0, 0.0),
                vec![
                    MenuItem::action("Cut").with_shortcut("Ctrl+X"),
                    MenuItem::action("Copy").with_shortcut("Ctrl+C"),
                ],
            );
            context.menu.highlighted = Some(variant_idx % 2);
            let placed = context.desc_at(menu, rect);
            let mut out = crate::menu::emit_menu(node, 2, &placed, &context.menu, state).0;
            let g0 = shape(
                "Cut",
                Vec2::new(placed.rect.origin.x + 14.0, placed.rect.origin.y + 10.0),
                13.0,
            );
            let g1 = shape(
                "Copy",
                Vec2::new(placed.rect.origin.x + 14.0, placed.rect.origin.y + 40.0),
                13.0,
            );
            push_glyphs(&mut out, &g0, Token::TEXT_PRIMARY, placed.z + 2);
            push_glyphs(&mut out, &g1, Token::TEXT_PRIMARY, placed.z + 2);
            out
        }
        WidgetCatalogId::CurveEditor => {
            let desc = CurveEditorDesc::new(rect);
            crate::curve_editor::emit_curve_editor(node, &desc, &CurveEditorState::new(), state)
        }
        WidgetCatalogId::DatePicker => {
            let desc = DatePickerDesc::new(rect);
            let today = NaiveDate::new(2024, 1, 15).expect("valid probe date");
            let calendar = CalendarState::new(today, SelectionMode::Single);
            let mut glyphs = shape("January 2024", Vec2::new(o.x + 16.0, o.y + 12.0), 13.5);
            crate::date_picker::emit_calendar(node, &desc, &calendar, state, &mut glyphs)
        }
        WidgetCatalogId::Divider => {
            let div_r = Rect::new(o.x, o.y + 16.0, rect.size.x, 16.0);
            let desc = DividerDesc::horizontal(div_r);
            let glyphs = shape_centered(&mut shape, "Section", div_r, 12.0);
            crate::divider::emit_divider(&desc, &glyphs, 56.0, DividerTokens::standard())
        }
        WidgetCatalogId::Dock => {
            let pane = DockTree::new(vec![DockTab::new(1, "Explorer"), DockTab::new(2, "Search")])
                .layout(rect)
                .remove(0);
            let mut out = crate::dock::emit_dock_pane(&pane, DockTokens::standard(), 10);
            let tab1_g = shape(
                "Explorer",
                Vec2::new(pane.rect.origin.x + 14.0, pane.rect.origin.y + 8.0),
                12.5,
            );
            let tab1 = crate::dock::emit_dock_tab(
                &pane,
                1,
                128.0,
                state,
                &tab1_g,
                DockTokens::standard(),
                11,
            );
            merge_output(&mut out, tab1);
            let tab2_g = shape(
                "Search",
                Vec2::new(pane.rect.origin.x + 142.0, pane.rect.origin.y + 8.0),
                12.5,
            );
            let tab2 = crate::dock::emit_dock_tab(
                &pane,
                2,
                128.0,
                ControlState::default(),
                &tab2_g,
                DockTokens::standard(),
                11,
            );
            merge_output(&mut out, tab2);
            let preview = crate::dock::emit_dock_preview(
                crate::dock::DockHover {
                    target: 0,
                    zone: crate::dock::DockZone::Center,
                    preview: rect,
                },
                DockTokens::standard(),
                10,
            );
            merge_output(&mut out, preview);
            out
        }
        WidgetCatalogId::Drawer => {
            let w = (rect.size.x * 0.55).min(300.0);
            let desc = DrawerDesc::new(rect, DrawerSide::Right, w);
            let drawer = DrawerState::new(true);
            let panel_x = rect.max().x - w;
            let mut glyphs = shape("Inspector", Vec2::new(panel_x + 16.0, o.y + 16.0), 14.0);
            glyphs.extend(shape(
                "Side drawer content",
                Vec2::new(panel_x + 16.0, o.y + 48.0),
                12.5,
            ));
            crate::drawer::emit_drawer(node, &desc, &drawer, &[10, 11], &glyphs, state).output
        }
        WidgetCatalogId::FileDrop => {
            let desc = FileDropDesc::new(rect);
            let mut glyphs =
                shape_centered(&mut shape, "Drop files here or click to browse", rect, 13.0);
            crate::file_drop::emit_file_drop(node, &desc, &FileDropState::new(), state, &mut glyphs)
        }
        WidgetCatalogId::Form => {
            let desc = FormDesc::new(rect);
            let fields = vec![FormField::new(
                1,
                FormFieldDesc::new("Name").required(true).help("Full name"),
            )];
            let layouts = layout_form(&desc, &fields);
            let mut out = crate::form::emit_form_root(rect, "Registration");
            let mut lbl_g = shape(
                "Name *",
                Vec2::new(layouts[0].label.origin.x, layouts[0].label.origin.y + 4.0),
                13.0,
            );
            let mut msg_g = shape(
                "Full name",
                Vec2::new(
                    layouts[0].message.origin.x,
                    layouts[0].message.origin.y + 2.0,
                ),
                11.5,
            );
            let label_out = crate::form::emit_form_label(&fields[0], &layouts[0], &mut lbl_g);
            let msg_out =
                crate::form::emit_form_message(&fields[0], &layouts[0], None, &mut msg_g, 10);
            merge_output(&mut out, label_out);
            merge_output(&mut out, msg_out);
            out
        }
        WidgetCatalogId::Gizmo => {
            let desc = GizmoDesc::new(rect, 2.0);
            let glyphs = shape("120 x 80", Vec2::new(o.x + 12.0, o.y + 10.0), 12.0);
            crate::gizmo::emit_gizmo(node, &desc, None, &[], &glyphs, state).output
        }
        WidgetCatalogId::HoverCard => {
            let trig_r = Rect::new(o.x, o.y, 130.0, 32.0);
            let desc = HoverCardDesc::new(trig_r, rect, Vec2::new(o.x + 40.0, o.y - 40.0));
            let mut glyphs = shape_centered(&mut shape, "Hover trigger", trig_r, 12.5);
            glyphs.extend(shape(
                "Preview card details",
                Vec2::new(o.x + 16.0, o.y + 52.0),
                12.5,
            ));
            crate::hover_card::emit_hover_card(node, 2, &desc, &glyphs, state).0
        }
        WidgetCatalogId::Image => {
            let img_r = Rect::new(o.x, o.y, rect.size.x.min(220.0), rect.size.y.min(140.0));
            let desc = ImageDesc::new(img_r, Vec2::new(100.0, 100.0), 7)
                .with_radius(8.0)
                .with_border(1.0, Token::OUTLINE)
                .with_alt("portrait");
            let mut out = crate::image::emit_image(node, &desc, state);
            let resolver = ProbeImageResolver;
            let asset_out = crate::image::emit_image_with_assets(node, &desc, state, &resolver);
            merge_output(&mut out, asset_out);
            // 当无外部纹理图集绑定时，绘制矢量预览占位图以便展厅可见
            out.primitives.push(crate::primitive::outlined(
                img_r,
                8.0,
                Token::SURFACE_SUNKEN,
                Token::CARD_BORDER,
                1.0,
                0.0,
                desc.z,
            ));
            let sun_r = Rect::new(img_r.origin.x + 24.0, img_r.origin.y + 22.0, 22.0, 22.0);
            out.primitives.push(crate::primitive::surface(
                sun_r,
                11.0,
                Token::WARNING,
                desc.z + 1,
            ));
            let g = shape_centered(&mut shape, "Image (100x100 · Cover)", img_r, 12.0);
            push_glyphs(&mut out, &g, Token::TEXT_SECONDARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Menu => {
            let menu_r = Rect::new(o.x, o.y, rect.size.x.min(220.0), rect.size.y);
            let desc = MenuDesc::new(
                menu_r,
                vec![
                    MenuItem::action("Open").with_shortcut("Ctrl+O"),
                    MenuItem::separator(),
                    MenuItem::action("Quit"),
                ],
            );
            let mut menu_state = MenuState::new();
            menu_state.highlighted = Some(variant_idx % 3);
            let mut out = crate::menu::emit_menu(node, 2, &desc, &menu_state, state).0;
            let g0 = shape(
                "Open",
                Vec2::new(menu_r.origin.x + 14.0, menu_r.origin.y + 10.0),
                13.0,
            );
            let g0s = shape(
                "Ctrl+O",
                Vec2::new(menu_r.max().x - 56.0, menu_r.origin.y + 11.0),
                11.5,
            );
            let g2 = shape(
                "Quit",
                Vec2::new(menu_r.origin.x + 14.0, menu_r.origin.y + 46.0),
                13.0,
            );
            push_glyphs(&mut out, &g0, Token::TEXT_PRIMARY, desc.z + 2);
            push_glyphs(&mut out, &g0s, Token::TEXT_SECONDARY, desc.z + 2);
            push_glyphs(&mut out, &g2, Token::TEXT_PRIMARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Menubar => {
            let desc = MenuBarDesc::new(
                Rect::new(o.x, o.y, rect.size.x, crate::menubar::MENUBAR_HEIGHT),
                vec![
                    MenuBarItem::new(1, "File"),
                    MenuBarItem::new(2, "Edit"),
                    MenuBarItem::new(3, "View"),
                ],
            );
            let rects = desc.layout(&[44.0, 44.0, 48.0]);
            let mut out = WidgetOutput::default();
            for (idx, (item_r, _)) in rects.iter().enumerate() {
                let lbl = ["File", "Edit", "View"][idx];
                let g = shape_centered(&mut shape, lbl, *item_r, 13.0);
                let item_out = crate::menubar::emit_menubar_item(
                    &desc,
                    &MenuBarState::new(),
                    idx,
                    *item_r,
                    if idx == 0 {
                        state
                    } else {
                        ControlState::default()
                    },
                    &g,
                    MenuBarTokens::standard(),
                );
                merge_output(&mut out, item_out);
            }
            out
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
            let card_rect = if rect.size.x > 260.0 && rect.size.y > 140.0 {
                let w = rect.size.x.min(380.0);
                let h = rect.size.y.min(200.0);
                Rect::new(
                    o.x + (rect.size.x - w) * 0.5,
                    o.y + (rect.size.y - h) * 0.5,
                    w,
                    h,
                )
            } else {
                rect
            };
            let desc = ModalDesc::new(card_rect).with_label("Settings");
            let mut out = crate::modal::emit_modal(node, rect, &desc, &[10, 11], state).output;
            let title_g = shape(
                "Settings",
                Vec2::new(card_rect.origin.x + 20.0, card_rect.origin.y + 18.0),
                14.0,
            );
            let body_g = shape(
                "Modal dialog with focus trap and scrim.",
                Vec2::new(card_rect.origin.x + 20.0, card_rect.origin.y + 54.0),
                12.5,
            );
            push_glyphs(&mut out, &title_g, Token::TEXT_PRIMARY, desc.z + 2);
            push_glyphs(&mut out, &body_g, Token::TEXT_SECONDARY, desc.z + 2);
            out
        }
        WidgetCatalogId::NodeGraph => {
            let nodes = vec![
                GraphNodeDesc::new(
                    1,
                    Vec2::new(o.x, o.y + 40.0),
                    Vec2::new(160.0, 120.0),
                    "Input",
                )
                .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")]),
                GraphNodeDesc::new(
                    2,
                    Vec2::new(o.x + 280.0, o.y + 40.0),
                    Vec2::new(160.0, 120.0),
                    "Output",
                )
                .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")]),
            ];
            let wires = [GraphWire {
                from_node: 1,
                from_pin: 0,
                to_node: 2,
                to_pin: 0,
            }];
            let mut out = crate::node_graph::emit_node_graph(
                node,
                &nodes,
                &wires,
                &NodeGraphState::new(),
                &[Vec::new(), Vec::new()],
            )
            .output;
            let g1 = shape("Input", Vec2::new(o.x + 12.0, o.y + 48.0), 12.5);
            let g2 = shape("Output", Vec2::new(o.x + 292.0, o.y + 48.0), 12.5);
            push_glyphs(&mut out, &g1, Token::TEXT_PRIMARY, 650);
            push_glyphs(&mut out, &g2, Token::TEXT_PRIMARY, 650);
            out
        }
        WidgetCatalogId::NumberInput => {
            let desc = NumberInputDesc::new(ctrl_rect, 0.0, 100.0);
            let val = (value_norm * 100.0).round();
            let number = NumberInputState::new(f64::from(val));
            let text = format!("{val:.0}");
            let mut glyphs = shape(
                &text,
                Vec2::new(ctrl_rect.origin.x + 10.0, ctrl_rect.origin.y + 10.0),
                13.0,
            );
            crate::number_input::emit_number_input(node, &desc, &number, state, &mut glyphs)
        }
        WidgetCatalogId::Overlay => WidgetOutput::default(),
        WidgetCatalogId::Popover => {
            let trig_r = Rect::new(o.x, o.y, 120.0, 36.0);
            let desc = PopoverDesc::new(trig_r, rect, Vec2::new(o.x + 40.0, o.y));
            let mut collector = crate::SceneCollector::new(0);
            crate::popover::emit_popover_scope(&mut collector, node, 2, &desc, state, []);
            let mut out = crate::popover::emit_popover(node, 2, &desc, state).output;
            let g = shape("Popover content", Vec2::new(o.x + 14.0, o.y + 56.0), 12.5);
            push_glyphs(&mut out, &g, Token::TEXT_PRIMARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Progress => {
            let bar_r = Rect::new(o.x, o.y + 12.0, rect.size.x, 10.0);
            let desc = ProgressDesc::new(bar_r).value(value_norm);
            let mut progress_state = ProgressState::new();
            progress_state.jump_to(value_norm);
            let mut out = crate::progress::emit_progress(&desc, &progress_state);
            let pct = format!("{:.0}%", value_norm * 100.0);
            let g = shape(&pct, Vec2::new(o.x, o.y + 30.0), 12.0);
            push_glyphs(&mut out, &g, Token::TEXT_SECONDARY, desc.z + 1);
            out
        }
        WidgetCatalogId::PropertyGrid => {
            let desc = PropertyGridDesc::new(
                rect,
                vec![PropertySection::new("Transform")
                    .with_row(PropertyDesc::new("Position X", PropertyValue::Number(10.0)))],
            );
            let glyphs = shape("Transform", Vec2::new(o.x + 12.0, o.y + 8.0), 13.0);
            let mut out = crate::property_grid::emit_property_grid(
                node,
                &desc,
                &PropertyGridState::new(),
                &glyphs,
                state,
            );
            let row_k = shape("Position X", Vec2::new(o.x + 16.0, o.y + 42.0), 12.5);
            let row_v = shape(
                "10.0",
                Vec2::new(o.x + rect.size.x * 0.55, o.y + 42.0),
                12.5,
            );
            push_glyphs(&mut out, &row_k, Token::TEXT_SECONDARY, desc.z + 2);
            push_glyphs(&mut out, &row_v, Token::TEXT_PRIMARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Radio => {
            let desc = RadioDesc::new(ctrl_rect, "Comfortable");
            let _group = RadioGroupDesc::new(vec![desc.clone()]);
            let glyphs = shape(
                "Comfortable",
                Vec2::new(o.x + 26.0, o.y + (ctrl_rect.size.y - 13.0) * 0.5),
                13.0,
            );
            crate::radio::emit_radio(node, &desc, state, &glyphs, RadioTokens::standard())
        }
        WidgetCatalogId::RichText => {
            let desc = RichTextDesc::new(rect);
            let document = parse_markdown("# Title\n\nMarkdown rendered as styled runs.");
            let layout = desc.layout(&document, &[28.0, 20.0]);
            let mut glyphs = shape("Rich Text Heading", Vec2::new(o.x + 12.0, o.y + 10.0), 17.0);
            glyphs.extend(shape(
                "Markdown parsed into styled GPU glyph runs.",
                Vec2::new(o.x + 12.0, o.y + 40.0),
                13.0,
            ));
            crate::rich_text::emit_rich_text(node, &desc, &document, &layout, &glyphs, state)
        }
        WidgetCatalogId::Ruler => {
            let desc = RulerDesc::new(rect, 1.0, Vec2::ZERO);
            crate::ruler::emit_rulers(node, &desc, &RulerState::new(), state).output
        }
        WidgetCatalogId::Scroll => {
            let desc = ScrollViewDesc::new(rect, Vec2::new(rect.size.x, 3000.0));
            let mut out =
                crate::scroll::emit_scroll_view(node, &desc, &ScrollState::new(), state).output;
            let g = shape(
                "Scrollable viewport content (3000px virtual height)",
                Vec2::new(o.x + 14.0, o.y + 14.0),
                12.5,
            );
            push_glyphs(&mut out, &g, Token::TEXT_SECONDARY, desc.z + 1);
            out
        }
        WidgetCatalogId::Segmented => {
            let mut desc = SegmentedDesc::new(ctrl_rect, &["One", "Two", "Three"]);
            desc.selected = variant_idx % 3;
            let interactive_idx = if desc.selected == 1 { 0 } else { 1 };
            let mut out =
                crate::segmented::emit_segmented_frame(&desc, SegmentedTokens::standard());
            for idx in 0..3 {
                let seg_state = if idx == interactive_idx {
                    state
                } else {
                    ControlState::default()
                };
                let seg_r = desc.segment_rect(idx);
                let seg_label =
                    shape_centered(&mut shape, ["One", "Two", "Three"][idx], seg_r, 12.5);
                let seg = crate::segmented::emit_segmented_segment(
                    node + idx as u64,
                    &desc,
                    idx,
                    seg_state,
                    &seg_label,
                    SegmentedTokens::standard(),
                );
                merge_output(&mut out, seg);
            }
            out
        }
        WidgetCatalogId::Select => {
            let options = ["Apple", "Banana", "Cherry"];
            let desc = SelectDesc::new(
                ctrl_rect,
                vec!["Apple".into(), "Banana".into(), "Cherry".into()],
            );
            let mut select_state = SelectState::new();
            let sel = variant_idx % 3;
            select_state.selected = Some(sel);
            let glyphs = shape(
                options[sel],
                Vec2::new(
                    ctrl_rect.origin.x + 10.0,
                    ctrl_rect.origin.y + (ctrl_rect.size.y - 13.0) * 0.5,
                ),
                13.0,
            );
            crate::select::emit_select_with_glyphs(node, &desc, &select_state, state, &glyphs)
        }
        WidgetCatalogId::Skeleton => {
            let desc = SkeletonDesc::new(Rect::new(o.x, o.y, rect.size.x, 18.0)).animated();
            crate::skeleton::emit_skeleton(&desc, &SkeletonState::new())
        }
        WidgetCatalogId::Slider => {
            let val = (value_norm * 36.0).round();
            let desc = SliderDesc::new(ctrl_rect, "Gap", val, 0.0, 36.0).with_step(1.0);
            let text = format!("Gap: {val:.0}px");
            let glyphs = shape(
                &text,
                Vec2::new(ctrl_rect.origin.x, ctrl_rect.origin.y + 22.0),
                12.0,
            );
            crate::slider::emit_slider(node, &desc, state, &glyphs, SliderTokens::standard())
        }
        WidgetCatalogId::SplitPane => {
            let desc = SplitPaneDesc::horizontal(rect);
            let split_state = SplitPaneState::new(value_norm.clamp(0.15, 0.85));
            let mut out = crate::split_pane::emit_split_pane_panes(
                &desc,
                &split_state,
                Token::SURFACE,
                Token::SURFACE_SUNKEN,
            );
            let splitter = crate::split_pane::emit_split_pane(
                node,
                &desc,
                &split_state,
                state,
                SplitPaneTokens::standard(),
            );
            merge_output(&mut out, splitter);
            let g_left = shape("Left Pane", Vec2::new(o.x + 12.0, o.y + 12.0), 12.0);
            let g_right = shape(
                "Right Pane",
                Vec2::new(o.x + rect.size.x * split_state.ratio + 12.0, o.y + 12.0),
                12.0,
            );
            push_glyphs(&mut out, &g_left, Token::TEXT_SECONDARY, desc.z + 1);
            push_glyphs(&mut out, &g_right, Token::TEXT_SECONDARY, desc.z + 1);
            out
        }
        WidgetCatalogId::Switch => {
            let desc = SwitchDesc::new(ctrl_rect, "Animate");
            let glyphs = shape(
                "Animate",
                Vec2::new(o.x + 48.0, o.y + (ctrl_rect.size.y - 13.0) * 0.5),
                13.0,
            );
            crate::switch::emit_switch(node, &desc, state, &glyphs, SwitchTokens::standard())
        }
        WidgetCatalogId::Table => {
            let desc = TableDesc::new(
                rect,
                vec![
                    ColumnSpec::new("Name", 180.0),
                    ColumnSpec::new("Size", 100.0).right_aligned(),
                ],
            )
            .with_label("Files");
            let mut table_state = TableState::new();
            table_state.select_row(variant_idx % 3);
            let row0_g = shape("main.rs", Vec2::new(o.x + 12.0, o.y + 38.0), 12.5);
            let row1_g = shape("lib.rs", Vec2::new(o.x + 12.0, o.y + 68.0), 12.5);
            let row2_g = shape("Cargo.toml", Vec2::new(o.x + 12.0, o.y + 98.0), 12.5);
            let rows = [
                crate::table::TableRow {
                    index: 0,
                    glyphs: &row0_g,
                },
                crate::table::TableRow {
                    index: 1,
                    glyphs: &row1_g,
                },
                crate::table::TableRow {
                    index: 2,
                    glyphs: &row2_g,
                },
            ];
            let hdr_g = shape("Name", Vec2::new(o.x + 12.0, o.y + 10.0), 12.5);
            let mut out = crate::table::emit_table(node, &desc, &table_state, &rows, &hdr_g, state);
            let hdr2_g = shape("Size", Vec2::new(o.x + 220.0, o.y + 10.0), 12.5);
            push_glyphs(&mut out, &hdr2_g, Token::TEXT_SECONDARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Tabs => {
            let desc = TabsDesc::new(
                Rect::new(o.x, o.y, rect.size.x, rect.size.y.min(36.0)),
                &["One", "Two", "Three"],
            );
            let active = variant_idx % 3;
            let tabs_state = TabsState::new(active);
            let interactive_idx = if active == 1 { 0 } else { 1 };
            let mut out = crate::tabs::emit_tabs_strip(&desc, &tabs_state, TabsTokens::standard());
            for idx in 0..3 {
                let tab_state = if idx == interactive_idx {
                    state
                } else {
                    ControlState::default()
                };
                let tab_r = desc.tab_rect(idx, 0.0);
                let tab_label = shape(
                    ["One", "Two", "Three"][idx],
                    Vec2::new(
                        tab_r.origin.x + 14.0,
                        tab_r.origin.y + (tab_r.size.y - 13.0) * 0.5,
                    ),
                    13.0,
                );
                let tab = crate::tabs::emit_tab(
                    &desc,
                    &tabs_state,
                    idx,
                    tab_state,
                    &tab_label,
                    TabsTokens::standard(),
                );
                merge_output(&mut out, tab);
            }
            let close = crate::tabs::emit_tab_close(
                node,
                &desc,
                &tabs_state,
                interactive_idx,
                state,
                TabsTokens::standard(),
            );
            merge_output(&mut out, close);
            let add = crate::tabs::emit_tabs_add(node, &desc, state, TabsTokens::standard());
            merge_output(&mut out, add);
            out
        }
        WidgetCatalogId::TagInput => {
            let desc = TagInputDesc::new(ctrl_rect);
            let tags = TagInputState::with_tags(["rust".to_owned()]);
            let layout = layout_tags(&desc, &tags, |tag| tag.len() as f32 * 8.0);
            let chip_r = layout.chips.first().copied().unwrap_or(ctrl_rect);
            let mut glyphs = shape(
                "rust",
                Vec2::new(chip_r.origin.x + 10.0, chip_r.origin.y + 4.0),
                12.5,
            );
            crate::tag_input::emit_tag_input(node, &desc, &tags, state, &layout, &mut glyphs)
        }
        WidgetCatalogId::TextArea => {
            let desc = TextAreaDesc::new(rect);
            let area = TextAreaState::with_text("one\ntwo");
            let mut glyphs = shape("one", Vec2::new(o.x + 42.0, o.y + 10.0), 13.0);
            glyphs.extend(shape("two", Vec2::new(o.x + 42.0, o.y + 30.0), 13.0));
            crate::text_area::emit_text_area(
                node,
                &desc,
                &area,
                state,
                &TextAreaVisual::default(),
                &mut glyphs,
            )
        }
        WidgetCatalogId::TextInput => {
            let desc = TextInputDesc::new(ctrl_rect).with_placeholder("type here");
            let mut glyphs = shape(
                "type here",
                Vec2::new(
                    ctrl_rect.origin.x + 10.0,
                    ctrl_rect.origin.y + (ctrl_rect.size.y - 13.0) * 0.5,
                ),
                13.0,
            );
            crate::text_input::emit_text_input(
                node,
                &desc,
                state,
                &TextEditState::new(),
                &TextInputVisual::default(),
                &mut glyphs,
            )
        }
        WidgetCatalogId::Toast => {
            let sev = match variant_idx % 4 {
                0 => Severity::Info,
                1 => Severity::Success,
                2 => Severity::Warning,
                _ => Severity::Error,
            };
            let mut entry = ToastEntry::new(1, "Saved", sev).with_message("All changes stored.");
            entry.y_spring.value = 0.0;
            entry.opacity_spring.value = 1.0;
            let toast_r = Rect::new(o.x, o.y, rect.size.x.min(340.0), 64.0);
            let mut glyphs = shape(
                "Saved",
                Vec2::new(toast_r.origin.x + 36.0, toast_r.origin.y + 12.0),
                13.0,
            );
            glyphs.extend(shape(
                "All changes stored.",
                Vec2::new(toast_r.origin.x + 36.0, toast_r.origin.y + 32.0),
                12.0,
            ));
            crate::toast::emit_toast(node, &entry, toast_r, &glyphs, state).output
        }
        WidgetCatalogId::Toolbar => {
            let bar_r = Rect::new(o.x, o.y, rect.size.x, 36.0);
            let desc = ToolbarDesc::new(
                bar_r,
                vec![
                    ToolbarItem::button(1, "Bold"),
                    ToolbarItem::toggle(2, "Italic"),
                ],
            );
            let layout = desc.layout();
            let mut out = crate::toolbar::emit_toolbar(&desc, ToolbarTokens::standard());
            if let Some(r0) = layout.rects[0] {
                let g0 = shape_centered(&mut shape, "B", r0, 13.0);
                let item0 = crate::toolbar::emit_toolbar_item(
                    &desc,
                    &ToolbarState::new(),
                    0,
                    r0,
                    state,
                    &g0,
                    ToolbarTokens::standard(),
                );
                merge_output(&mut out, item0);
            }
            if let Some(r1) = layout.rects.get(1).and_then(|r| *r) {
                let g1 = shape_centered(&mut shape, "I", r1, 13.0);
                let item1 = crate::toolbar::emit_toolbar_item(
                    &desc,
                    &ToolbarState::new(),
                    1,
                    r1,
                    ControlState::default(),
                    &g1,
                    ToolbarTokens::standard(),
                );
                merge_output(&mut out, item1);
            }
            let overflow = crate::toolbar::emit_toolbar_overflow(
                node,
                Rect::new(o.x + bar_r.size.x - 28.0, o.y + 4.0, 24.0, 28.0),
                state,
                ToolbarTokens::standard(),
                10,
            );
            merge_output(&mut out, overflow);
            out
        }
        WidgetCatalogId::Tooltip => {
            let desc = TooltipDesc::new(
                Rect::new(o.x, o.y, 120.0, 32.0),
                rect,
                Vec2::new(o.x + 20.0, o.y + 40.0),
            );
            let mut out = crate::tooltip::emit_tooltip(node, 2, &desc, state).0;
            let g = shape(
                "Auto-flipping hint",
                Vec2::new(o.x + 28.0, o.y + 46.0),
                12.0,
            );
            push_glyphs(&mut out, &g, Token::TEXT_PRIMARY, desc.z + 2);
            out
        }
        WidgetCatalogId::TreeView => {
            let desc = TreeViewDesc::new(rect);
            let row = TreeViewRow {
                row: TreeRow {
                    id: 1,
                    depth: 0,
                    has_children: true,
                    expanded: variant_idx % 2 == 1,
                },
                item: 0,
                rect: Rect::new(o.x, o.y, rect.size.x, 28.0),
            };
            let mut out = crate::tree_view::emit_tree_background(&desc, TreeViewTokens::standard());
            let row_g = shape(
                "crates/unbound-widgets",
                Vec2::new(o.x + 26.0, o.y + 7.0),
                13.0,
            );
            let row_out = crate::tree_view::emit_tree_row(
                &desc,
                &row,
                variant_idx % 2 == 1,
                state,
                &row_g,
                TreeViewTokens::standard(),
            );
            merge_output(&mut out, row_out);
            out
        }
        WidgetCatalogId::VirtualViews => {
            let row_r = Rect::new(o.x, o.y, rect.size.x, 32.0);
            let g = shape(
                "Virtual Row #0 (variable height)",
                Vec2::new(o.x + 12.0, o.y + 9.0),
                13.0,
            );
            crate::virtual_views::emit_virtual_row(
                row_r,
                &g,
                false,
                state.phase == PointerPhase::Hovered,
                VirtualRowTokens::standard(),
                100,
            )
        }
        WidgetCatalogId::Collapsible => {
            let open = variant_idx % 2 == 0;
            let desc = crate::collapsible::CollapsibleDesc::new(
                Rect::new(o.x, o.y, rect.size.x, 88.0),
                "Details",
                open,
            );
            let mut glyphs = shape("Details", Vec2::new(o.x + 12.0, o.y + 9.0), 13.0);
            if open {
                glyphs.extend(shape(
                    "Expanded disclosure panel content.",
                    Vec2::new(o.x + 12.0, o.y + 44.0),
                    12.5,
                ));
            }
            crate::collapsible::emit_collapsible(node, &desc, state, &glyphs)
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
            let rail_r = Rect::new(o.x, o.y, 160.0, 110.0);
            let desc = crate::navigation_rail::NavigationRailDesc::new(
                rail_r,
                vec![
                    crate::navigation_rail::NavRailItem::new(1, "Home").with_badge(2),
                    crate::navigation_rail::NavRailItem::new(2, "Assets"),
                ],
                1,
            );
            let mut glyphs = shape("Home", Vec2::new(o.x + 16.0, o.y + 18.0), 13.0);
            glyphs.extend(shape("Assets", Vec2::new(o.x + 16.0, o.y + 62.0), 13.0));
            crate::navigation_rail::emit_navigation_rail(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Pagination => {
            let pg_r = Rect::new(o.x, o.y, rect.size.x.min(380.0), rect.size.y.min(36.0));
            let desc = crate::pagination::PaginationDesc::new(pg_r, 12, (variant_idx % 12) + 1);
            let slots = desc.visible_slots();
            let total_slots = 9usize;
            let gap = 4.0;
            let pad = 4.0;
            let slot_w = ((pg_r.width() - pad * 2.0 - gap * (total_slots - 1) as f32)
                / total_slots as f32)
                .max(18.0);
            let slot_h = (pg_r.height() - pad * 2.0).max(18.0);
            let mut glyphs = Vec::new();
            for idx in 0..total_slots {
                let r = Rect::new(
                    pg_r.origin.x + pad + idx as f32 * (slot_w + gap),
                    pg_r.origin.y + pad,
                    slot_w,
                    slot_h,
                );
                let txt = if idx == 0 {
                    "<".to_string()
                } else if idx == total_slots - 1 {
                    ">".to_string()
                } else if let Some(p) = slots[idx - 1] {
                    p.to_string()
                } else {
                    "...".to_string()
                };
                glyphs.extend(shape_centered(&mut shape, &txt, r, 12.0));
            }
            crate::pagination::emit_pagination(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Stepper => {
            let stp_r = Rect::new(o.x, o.y, rect.size.x, 64.0);
            let desc = crate::stepper::StepperDesc::new(
                stp_r,
                vec![
                    crate::stepper::StepItem::new("Plan", crate::stepper::StepStatus::Completed),
                    crate::stepper::StepItem::new("Build", crate::stepper::StepStatus::Active),
                    crate::stepper::StepItem::new("Ship", crate::stepper::StepStatus::Pending),
                ],
            );
            let mut glyphs = shape("1  Plan", Vec2::new(o.x + 12.0, o.y + 46.0), 11.5);
            glyphs.extend(shape_centered(&mut shape, "2  Build", stp_r, 11.5));
            crate::stepper::emit_stepper(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::AutoComplete => {
            let desc = crate::autocomplete::AutoCompleteDesc::new(ctrl_rect, "unb", 3);
            let glyphs = shape(
                "unb",
                Vec2::new(ctrl_rect.origin.x + 10.0, ctrl_rect.origin.y + 10.0),
                13.0,
            );
            crate::autocomplete::emit_autocomplete(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Banner => {
            let sev = match variant_idx % 4 {
                0 => Severity::Info,
                1 => Severity::Success,
                2 => Severity::Warning,
                _ => Severity::Error,
            };
            let bnr_r = Rect::new(o.x, o.y, rect.size.x, 44.0);
            let desc = crate::banner::BannerDesc::new(bnr_r, "Hot reload ready", sev);
            let glyphs = shape(
                "Hot reload ready",
                Vec2::new(bnr_r.origin.x + 16.0, bnr_r.origin.y + 15.0),
                13.0,
            );
            crate::banner::emit_banner(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Cascader => {
            let desc = crate::cascader::CascaderDesc::new(
                ctrl_rect,
                vec![crate::cascader::CascaderOption::leaf("ui", "UI")],
                vec!["UI".to_owned()],
            );
            let glyphs = shape(
                "UI",
                Vec2::new(ctrl_rect.origin.x + 14.0, ctrl_rect.origin.y + 10.0),
                12.5,
            );
            crate::cascader::emit_cascader(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::DateRangePicker => {
            let drp_r = Rect::new(o.x, o.y, rect.size.x, 96.0);
            let desc = crate::date_range_picker::DateRangePickerDesc::new(
                drp_r,
                Some(crate::date_range_picker::DateRange::ordered(
                    NaiveDate::new(2026, 11, 1).expect("valid date"),
                    NaiveDate::new(2026, 11, 13).expect("valid date"),
                )),
            );
            let glyphs = shape(
                "Nov 1 - Nov 13, 2026",
                Vec2::new(drp_r.origin.x + 16.0, drp_r.origin.y + 42.0),
                12.0,
            );
            crate::date_range_picker::emit_date_range_picker(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::HotkeyInput => {
            let desc = crate::hotkey_input::HotkeyInputDesc::new(ctrl_rect, "Ctrl+K");
            let glyphs = shape(
                "Ctrl+K",
                Vec2::new(ctrl_rect.origin.x + 14.0, ctrl_rect.origin.y + 10.0),
                13.0,
            );
            crate::hotkey_input::emit_hotkey_input(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::NotificationCenter => {
            let nc_r = Rect::new(o.x, o.y, rect.size.x, 110.0);
            let desc = crate::notification_center::NotificationCenterDesc::new(
                nc_r,
                vec![
                    crate::notification_center::NotificationItem {
                        id: 1,
                        title: "Build finished".to_owned(),
                        severity: Severity::Success,
                        read: false,
                    },
                    crate::notification_center::NotificationItem {
                        id: 2,
                        title: "Shader cache warmed".to_owned(),
                        severity: Severity::Info,
                        read: true,
                    },
                ],
            );
            let mut glyphs = shape(
                "Build finished",
                Vec2::new(nc_r.origin.x + 14.0, nc_r.origin.y + 10.0),
                12.0,
            );
            glyphs.extend(shape(
                "Shader cache warmed",
                Vec2::new(nc_r.origin.x + 14.0, nc_r.origin.y + 36.0),
                12.0,
            ));
            crate::notification_center::emit_notification_center(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::PinInput => {
            let desc = crate::pin_input::PinInputDesc::new(
                Rect::new(o.x, o.y, rect.size.x.min(220.0), rect.size.y.min(40.0)),
                4,
                (variant_idx % 4) + 1,
            );
            crate::pin_input::emit_pin_input(node, &desc, state, &[])
        }
        WidgetCatalogId::Popconfirm => {
            let pc_r = Rect::new(o.x, o.y, 200.0, 72.0);
            let desc = crate::popconfirm::PopconfirmDesc::new(pc_r, "Delete item?");
            let mut glyphs = shape(
                "Delete item?",
                Vec2::new(pc_r.origin.x + 12.0, pc_r.origin.y + 12.0),
                13.0,
            );
            let btn_w = 56.0_f32.min((pc_r.width() - 24.0) * 0.45);
            let btn_h = 22.0_f32.min(pc_r.height() * 0.4);
            let btn_y = pc_r.max().y - btn_h - 8.0;
            let confirm_r = Rect::new(pc_r.max().x - btn_w - 8.0, btn_y, btn_w, btn_h);
            let cancel_r = Rect::new(confirm_r.origin.x - btn_w - 6.0, btn_y, btn_w, btn_h);
            glyphs.extend(shape_centered(&mut shape, "Cancel", cancel_r, 11.5));
            let confirm_g = shape_centered(&mut shape, "Confirm", confirm_r, 11.5);
            let mut out = crate::popconfirm::emit_popconfirm(node, &desc, state, &glyphs);
            push_glyphs(&mut out, &confirm_g, Token::ON_PRIMARY, desc.z + 3);
            out
        }
        WidgetCatalogId::Rating => {
            let stars = if variant_idx > 0 {
                ((variant_idx % 5) + 1) as f32
            } else {
                4.5
            };
            let desc = crate::rating::RatingDesc::new(
                Rect::new(o.x, o.y, rect.size.x.min(200.0), rect.size.y.min(36.0)),
                stars,
            );
            crate::rating::emit_rating(node, &desc, state)
        }
        WidgetCatalogId::Spinner => {
            let desc =
                crate::spinner::SpinnerDesc::new(Rect::new(o.x, o.y, 36.0, 36.0), value_norm);
            crate::spinner::emit_spinner(&desc, state)
        }
        WidgetCatalogId::TimePicker => {
            let desc = crate::time_picker::TimePickerDesc {
                active_column: variant_idx % 3,
                ..crate::time_picker::TimePickerDesc::new(
                    ctrl_rect,
                    crate::time_picker::TimeValue::new(14, 30, 0),
                )
            };
            let gap = 6.0;
            let pad = 4.0;
            let col_w = ((ctrl_rect.width() - pad * 2.0 - gap * 2.0) / 3.0).max(16.0);
            let col_h = (ctrl_rect.height() - pad * 2.0).max(16.0);
            let mut glyphs = Vec::new();
            for (col, val) in ["14", "30", "00"].iter().enumerate() {
                let r = Rect::new(
                    ctrl_rect.origin.x + pad + col as f32 * (col_w + gap),
                    ctrl_rect.origin.y + pad,
                    col_w,
                    col_h,
                );
                glyphs.extend(shape_centered(&mut shape, val, r, 13.0));
            }
            crate::time_picker::emit_time_picker(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Transfer => {
            let tr_r = Rect::new(o.x, o.y, rect.size.x, 96.0);
            let desc = crate::transfer::TransferDesc::new(tr_r, 5, 2);
            let (right_btn, left_btn) = desc.action_button_rects();
            let mut glyphs = shape_centered(&mut shape, ">", right_btn, 12.0);
            glyphs.extend(shape_centered(&mut shape, "<", left_btn, 12.0));
            glyphs.extend(shape(
                "Source items (5)",
                Vec2::new(tr_r.origin.x + 14.0, tr_r.origin.y + 12.0),
                12.0,
            ));
            let pad = 6.0;
            let mid_w = 28.0;
            let pane_w = ((tr_r.width() - pad * 4.0 - mid_w) * 0.5).max(24.0);
            glyphs.extend(shape(
                "Target items (2)",
                Vec2::new(
                    tr_r.origin.x + pad * 3.0 + pane_w + mid_w + 8.0,
                    tr_r.origin.y + 12.0,
                ),
                12.0,
            ));
            crate::transfer::emit_transfer(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Carousel => {
            let car_r = Rect::new(o.x, o.y, rect.size.x, 96.0);
            let active = variant_idx % 4;
            let desc = crate::carousel::CarouselDesc::new(car_r, 4, active);
            let title = format!("Slide {} of 4", active + 1);
            let glyphs = shape_centered(&mut shape, &title, car_r, 14.0);
            crate::carousel::emit_carousel(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::CodeEditor => {
            let ce_r = Rect::new(o.x, o.y, rect.size.x, 96.0);
            let desc = crate::code_editor::CodeEditorDesc::new(ce_r, 12, 1);
            let mut glyphs = shape(
                "1",
                Vec2::new(ce_r.origin.x + 12.0, ce_r.origin.y + 8.0),
                12.0,
            );
            glyphs.extend(shape(
                "2",
                Vec2::new(ce_r.origin.x + 12.0, ce_r.origin.y + 26.0),
                12.0,
            ));
            glyphs.extend(shape(
                "fn render_frame() {",
                Vec2::new(ce_r.origin.x + 44.0, ce_r.origin.y + 8.0),
                12.5,
            ));
            glyphs.extend(shape(
                "    submit_commands();",
                Vec2::new(ce_r.origin.x + 44.0, ce_r.origin.y + 26.0),
                12.5,
            ));
            crate::code_editor::emit_code_editor(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::DataGrid => {
            let dg_r = Rect::new(o.x, o.y, rect.size.x, 110.0);
            let desc = crate::data_grid::DataGridDesc::new(
                dg_r,
                (variant_idx % 3, (variant_idx / 3) % 3),
                false,
            );
            let mut glyphs = Vec::new();
            for r in 0..3 {
                for c in 0..3 {
                    let cell = desc.cell_rect(r, c);
                    let txt = if r == 0 {
                        format!("Col {}", (b'A' + c as u8) as char)
                    } else {
                        format!("R{}C{}", r, c + 1)
                    };
                    glyphs.extend(shape_centered(&mut shape, &txt, cell, 12.0));
                }
            }
            crate::data_grid::emit_data_grid(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Timeline => {
            let tl_r = Rect::new(o.x, o.y, rect.size.x, 96.0);
            let desc = crate::timeline::TimelineDesc::new(
                tl_r,
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
            let mut glyphs = shape(
                "Init  ·  10:00",
                Vec2::new(tl_r.origin.x + 36.0, tl_r.origin.y + 10.0),
                12.0,
            );
            glyphs.extend(shape(
                "Verify  ·  10:05",
                Vec2::new(tl_r.origin.x + 36.0, tl_r.origin.y + 30.0),
                12.0,
            ));
            crate::timeline::emit_timeline(node, &desc, state, &glyphs)
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
            crate::timeline_track::emit_timeline_track(node, &desc, state, &[])
        }
        WidgetCatalogId::Tour => {
            let tr_r = Rect::new(o.x, o.y, rect.size.x, 110.0);
            let target_r = Rect::new(o.x + 12.0, o.y + 8.0, 80.0, 28.0);
            let desc = crate::tour::TourDesc::new(tr_r, target_r, "Step 1");
            let mut glyphs = shape_centered(&mut shape, "Target", target_r, 12.0);
            let bubble_y = (target_r.max().y + 6.0).min(tr_r.max().y - 54.0);
            glyphs.extend(shape(
                "Step 1: Welcome",
                Vec2::new(target_r.origin.x + 10.0, bubble_y + 8.0),
                12.0,
            ));
            crate::tour::emit_tour(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::Watermark => {
            let wm_r = Rect::new(o.x, o.y, rect.size.x, 84.0);
            let desc = crate::watermark::WatermarkDesc::new(wm_r, "CONFIDENTIAL");
            let mut glyphs = Vec::new();
            let cw = wm_r.width() / 3.0;
            let ch = wm_r.height() / 2.0;
            for r in 0..2 {
                for c in 0..3 {
                    let cell = Rect::new(
                        wm_r.origin.x + c as f32 * cw,
                        wm_r.origin.y + r as f32 * ch,
                        cw,
                        ch,
                    );
                    glyphs.extend(shape_centered(&mut shape, "CONFIDENTIAL", cell, 11.5));
                }
            }
            crate::watermark::emit_watermark(&desc, state, &glyphs)
        }
        WidgetCatalogId::AspectRatioBox => {
            let desc = AspectRatioBoxDesc {
                bounds: rect,
                ratio: 16.0 / 9.0,
            };
            let fitted = desc.fit();
            let card = CardDesc::new(fitted);
            let mut out = crate::containers::emit_card(&card, CardTokens::standard());
            let g = shape_centered(&mut shape, "16 : 9 AspectRatioBox", fitted, 13.0);
            push_glyphs(&mut out, &g, Token::TEXT_PRIMARY, card.z + 1);
            out
        }
        WidgetCatalogId::AvatarGroup => {
            let a0 = AvatarDesc::new(Rect::new(o.x, o.y, 40.0, 40.0), "Alice");
            let a1 = AvatarDesc::new(Rect::new(o.x + 28.0, o.y, 40.0, 40.0), "Bob");
            let g0 = shape_centered(&mut shape, "AL", a0.image_rect(), 14.0);
            let g1 = shape_centered(&mut shape, "BO", a1.image_rect(), 14.0);
            let desc = AvatarGroupDesc::new(Rect::new(o.x, o.y, 140.0, 40.0), vec![a0, a1]);
            crate::avatar::emit_avatar_group(&desc, &[g0, g1], &[])
        }
        WidgetCatalogId::Chip => {
            let chip_r = Rect::new(o.x, o.y, 100.0, 30.0);
            let desc = ChipDesc::new(chip_r, "Filter");
            let glyphs = shape(
                "Filter",
                Vec2::new(chip_r.origin.x + 14.0, chip_r.origin.y + 8.0),
                12.5,
            );
            crate::badge::emit_chip(node, &desc, state, &glyphs)
        }
        WidgetCatalogId::EmptyState => {
            let desc = EmptyStateDesc::new(rect, "No results found");
            let glyphs = shape_centered(&mut shape, "No results found", rect, 13.5);
            crate::hover_card::emit_empty_state(node, &desc, &glyphs, state).output
        }
        WidgetCatalogId::GradientEditor => {
            let grad_state = GradientEditorState::new();
            crate::curve_editor::emit_gradient_editor(node, rect, &grad_state, state, 100)
        }
        WidgetCatalogId::Icon => {
            let icon_r = Rect::new(o.x, o.y, 32.0, 32.0);
            let desc = IconDesc::new(icon_r, 1, Token::PRIMARY);
            let mut out = crate::image::emit_icon(node, &desc, state);
            // 绘制矢量对勾图标，保证无外部图集时清晰可见
            out.primitives.push(crate::primitive::outlined(
                icon_r,
                8.0,
                Token::PRIMARY_SOFT,
                Token::PRIMARY,
                1.0,
                0.0,
                desc.z,
            ));
            let cx = icon_r.center().x;
            let cy = icon_r.center().y;
            crate::primitive::curve_stroke(
                [cx - 6.0, cy],
                [cx - 1.5, cy + 5.0],
                2.0,
                Token::PRIMARY,
                desc.z + 1,
                &mut out.primitives,
                &mut out.payloads,
            );
            crate::primitive::curve_stroke(
                [cx - 1.5, cy + 5.0],
                [cx + 6.5, cy - 4.5],
                2.0,
                Token::PRIMARY,
                desc.z + 1,
                &mut out.primitives,
                &mut out.payloads,
            );
            out
        }
        WidgetCatalogId::LoadingOverlay => {
            let desc = LoadingOverlayDesc {
                rect,
                active: params.active,
                phase: value_norm,
                label: "Loading...".to_owned(),
                z: 100,
            };
            let mut out = crate::spinner::emit_loading_overlay(&desc, state);
            if params.active {
                let lbl_r = Rect::new(o.x, o.y + rect.size.y * 0.5 + 28.0, rect.size.x, 20.0);
                let g = shape_centered(&mut shape, "Loading...", lbl_r, 12.5);
                push_glyphs(&mut out, &g, Token::TEXT_SECONDARY, desc.z + 2);
            }
            out
        }
        WidgetCatalogId::NinePatch => {
            let np_r = Rect::new(o.x, o.y, rect.size.x.min(260.0), rect.size.y.min(140.0));
            let desc = NinePatchDesc::new(np_r, Vec2::new(64.0, 64.0), 0, [16.0; 4]);
            let mut out = crate::image::emit_nine_patch(node, &desc, state);
            // 绘制九宫格切片结构可视化预览（4 固定角 + 4 拉伸边 + 1 中心区）
            for (idx, cell) in desc.cells().iter().enumerate() {
                let is_corner = matches!(idx, 0 | 2 | 6 | 8);
                let is_center = idx == 4;
                let fill = if is_corner {
                    Token::PRIMARY_SOFT
                } else if is_center {
                    Token::SURFACE_RAISED
                } else {
                    Token::SURFACE_SUNKEN
                };
                out.primitives.push(crate::primitive::outlined(
                    *cell,
                    2.0,
                    fill,
                    Token::OUTLINE,
                    1.0,
                    0.0,
                    desc.z + 1,
                ));
            }
            let g = shape_centered(&mut shape, "9-Slice (fixed 16px corners)", np_r, 12.0);
            push_glyphs(&mut out, &g, Token::TEXT_PRIMARY, desc.z + 2);
            out
        }
        WidgetCatalogId::Stack => {
            let desc = StackDesc::new(rect);
            let mut out = crate::containers::emit_stack(&desc, Token::SURFACE);
            let layers = desc.layout(3);
            for (idx, _) in layers.iter().enumerate() {
                let inset = idx as f32 * 18.0;
                let layer_r = Rect::new(
                    rect.origin.x + inset,
                    rect.origin.y + inset * 0.7,
                    (rect.size.x - inset * 2.0).max(60.0),
                    (rect.size.y - inset * 1.4).max(32.0),
                );
                let fill = match idx {
                    0 => Token::SURFACE_SUNKEN,
                    1 => Token::SURFACE_RAISED,
                    _ => Token::PRIMARY_SOFT,
                };
                let border = if idx == 2 {
                    Token::PRIMARY
                } else {
                    Token::CARD_BORDER
                };
                out.primitives.push(crate::primitive::outlined(
                    layer_r,
                    8.0,
                    fill,
                    border,
                    1.0,
                    0.0,
                    desc.layer_z(idx),
                ));
                let g = shape(
                    ["Layer 1 (z=100)", "Layer 2 (z=101)", "Layer 3 (z=102)"][idx],
                    Vec2::new(layer_r.origin.x + 12.0, layer_r.origin.y + 8.0),
                    11.5,
                );
                push_glyphs(&mut out, &g, Token::TEXT_PRIMARY, desc.layer_z(idx));
            }
            out
        }
        WidgetCatalogId::StatusBar => {
            let bar_r = Rect::new(o.x, o.y + 12.0, rect.size.x, 28.0);
            let seg_left = crate::toolbar::StatusSegment::left(1, "Ready");
            let seg_mid = crate::toolbar::StatusSegment::left(2, "Ln 42, Col 8");
            let seg_right = crate::toolbar::StatusSegment::right(3, "Vulkan 1.3");
            let desc = StatusBarDesc::new(
                bar_r,
                vec![seg_left.clone(), seg_mid.clone(), seg_right.clone()],
            );
            let mut out = crate::toolbar::emit_status_bar(&desc, ToolbarTokens::standard());
            let rects = desc.layout(&desc.widths());
            let labels = ["Ready", "Ln 42, Col 8", "Vulkan 1.3"];
            for (idx, seg_r) in rects.into_iter().enumerate() {
                // 空间不足时被丢弃的分段没有矩形，跳过即可保持形状稳定。
                let Some(seg_r) = seg_r else { continue };
                let g = shape(
                    labels[idx],
                    Vec2::new(
                        seg_r.origin.x + 8.0,
                        seg_r.origin.y + (seg_r.size.y - 12.0) * 0.5,
                    ),
                    12.0,
                );
                let seg_out = crate::toolbar::emit_status_segment(
                    &desc,
                    seg_r,
                    &desc.segments[idx],
                    &g,
                    ToolbarTokens::standard(),
                );
                merge_output(&mut out, seg_out);
            }
            out
        }
        WidgetCatalogId::Wrap => {
            let desc = WrapDesc::new(rect, 28.0);
            let card = CardDesc::new(desc.rect);
            let mut out = crate::containers::emit_card(&card, CardTokens::standard());
            let items = ["Layout", "Vulkan", "SDF", "Text", "Input", "Theme", "A11y"];
            let widths = [72.0, 72.0, 54.0, 56.0, 62.0, 70.0, 56.0];
            let inner = Rect::new(
                rect.origin.x + 12.0,
                rect.origin.y + 12.0,
                (rect.size.x - 24.0).max(40.0),
                (rect.size.y - 24.0).max(28.0),
            );
            let wrap = WrapDesc::new(inner, 28.0);
            let sizes: Vec<Vec2> = widths.iter().map(|&w| Vec2::new(w, 28.0)).collect();
            for (idx, item_r) in wrap.layout(&sizes).into_iter().enumerate() {
                out.primitives.push(crate::primitive::outlined(
                    item_r,
                    14.0,
                    Token::PRIMARY_SOFT,
                    Token::PRIMARY,
                    1.0,
                    0.0,
                    card.z + 1,
                ));
                let g = shape_centered(&mut shape, items[idx], item_r, 12.0);
                push_glyphs(&mut out, &g, Token::PRIMARY, card.z + 2);
            }
            out
        }
        WidgetCatalogId::SplitButton => {
            let sb_r = Rect::new(o.x, o.y, rect.size.x.min(180.0), rect.size.y.min(36.0));
            let desc = crate::split_button::SplitButtonDesc::new(sb_r, "Run Build");
            let s = crate::split_button::SplitButtonState::new();
            let main_r = Rect::new(
                sb_r.origin.x,
                sb_r.origin.y,
                (sb_r.width() - crate::split_button::DROPDOWN_WIDTH).max(0.0),
                sb_r.height(),
            );
            let g = shape_centered(&mut shape, "Run Build", main_r, 13.0);
            crate::split_button::emit_split_button_with_glyphs(node, &desc, &s, state, &g)
        }
        WidgetCatalogId::ToggleButton => {
            let tb_r = Rect::new(o.x, o.y, rect.size.x.min(160.0), rect.size.y.min(36.0));
            let desc =
                crate::toggle_button::ToggleButtonDesc::new(tb_r, "Pin Panel", state.selected);
            let s = crate::toggle_button::ToggleButtonState::new(state.selected);
            let g = shape_centered(&mut shape, "Pin Panel", tb_r, 13.0);
            crate::toggle_button::emit_toggle_button_with_glyphs(node, &desc, &s, state, &g)
        }
        WidgetCatalogId::TreeTable => {
            let desc = crate::tree_table::TreeTableDesc::new(
                rect,
                vec![
                    crate::table::ColumnSpec::new("Name", 200.0),
                    crate::table::ColumnSpec::new("Size", 100.0),
                ],
            )
            .with_rows(vec![
                crate::tree_table::TreeTableRow {
                    id: 0,
                    depth: 0,
                    has_children: true,
                    cells: vec!["src/".into(), "42 KB".into()],
                },
                crate::tree_table::TreeTableRow {
                    id: 1,
                    depth: 1,
                    has_children: false,
                    cells: vec!["main.rs".into(), "12 KB".into()],
                },
            ]);
            let mut s = crate::tree_table::TreeTableState::new();
            s.toggle_expand(0);
            s.selected = Some((variant_idx % 4) as u64);
            let mut glyphs = shape("Name", Vec2::new(o.x + 14.0, o.y + 9.0), 12.5);
            glyphs.extend(shape("Size", Vec2::new(o.x + 214.0, o.y + 9.0), 12.5));
            glyphs.extend(shape("src/", Vec2::new(o.x + 24.0, o.y + 39.0), 12.5));
            glyphs.extend(shape("42 KB", Vec2::new(o.x + 214.0, o.y + 39.0), 12.5));
            glyphs.extend(shape("main.rs", Vec2::new(o.x + 38.0, o.y + 67.0), 12.5));
            glyphs.extend(shape("12 KB", Vec2::new(o.x + 214.0, o.y + 67.0), 12.5));
            crate::tree_table::emit_tree_table_with_glyphs(node, &desc, &s, state, &glyphs)
        }
        WidgetCatalogId::MultiSplit => {
            let desc = crate::multi_split::MultiSplitDesc::new(rect, 3);
            let s = crate::multi_split::MultiSplitState::new(3);
            let mut out = crate::multi_split::emit_multi_split(node, &desc, &s, state);
            let pane_w = rect.width() / 3.0;
            for idx in 0..3 {
                let g = shape(
                    ["Pane 1", "Pane 2", "Pane 3"][idx],
                    Vec2::new(o.x + idx as f32 * pane_w + 12.0, o.y + 12.0),
                    12.0,
                );
                push_glyphs(&mut out, &g, Token::TEXT_SECONDARY, 704);
            }
            out
        }
        WidgetCatalogId::Descriptions => {
            let desc = crate::descriptions::DescriptionsDesc::new(rect, "Details");
            let s = crate::descriptions::DescriptionsState::new();
            let row_h =
                crate::descriptions::DESCRIPTION_ROW_HEIGHT.min((rect.height() / 4.0).max(1.0));
            let mut glyphs = shape("Engine: Unbound UI", Vec2::new(o.x + 12.0, o.y + 8.0), 12.5);
            glyphs.extend(shape(
                "Backend: Vulkan 1.3",
                Vec2::new(o.x + 12.0, o.y + row_h + 8.0),
                12.5,
            ));
            glyphs.extend(shape(
                "Pacing: 120 FPS",
                Vec2::new(o.x + 12.0, o.y + row_h * 2.0 + 8.0),
                12.5,
            ));
            crate::descriptions::emit_descriptions_with_glyphs(node, &desc, &s, state, &glyphs)
        }
        WidgetCatalogId::Callout => {
            let sev = match variant_idx % 4 {
                0 => Severity::Info,
                1 => Severity::Success,
                2 => Severity::Warning,
                _ => Severity::Error,
            };
            let co_r = Rect::new(o.x, o.y, rect.size.x, 68.0);
            let desc = crate::callout::CalloutDesc::new(co_r, "Tip", sev);
            let s = crate::callout::CalloutState::new();
            let mut glyphs = shape(
                "Tip",
                Vec2::new(co_r.origin.x + 16.0, co_r.origin.y + 12.0),
                13.0,
            );
            glyphs.extend(shape(
                "GPU memory usage is within the 64 MiB frame budget.",
                Vec2::new(co_r.origin.x + 16.0, co_r.origin.y + 34.0),
                12.0,
            ));
            crate::callout::emit_callout_with_glyphs(node, &desc, &s, state, &glyphs)
        }
        WidgetCatalogId::VirtualMasonry => {
            let desc = crate::virtual_masonry::VirtualMasonryDesc::new(rect, 3);
            let s = crate::virtual_masonry::VirtualMasonryState::new();
            crate::virtual_masonry::emit_virtual_masonry(node, &desc, &s, state)
        }
        WidgetCatalogId::TerminalView => {
            let desc = crate::terminal_view::TerminalViewDesc::new(rect);
            let s = crate::terminal_view::TerminalViewState::new();
            let mut glyphs = shape("1", Vec2::new(o.x + 14.0, o.y + 8.0), 12.0);
            glyphs.extend(shape("2", Vec2::new(o.x + 14.0, o.y + 26.0), 12.0));
            glyphs.extend(shape(
                "$ cargo test --workspace",
                Vec2::new(o.x + 48.0, o.y + 8.0),
                12.5,
            ));
            glyphs.extend(shape(
                "test result: ok. 142 passed",
                Vec2::new(o.x + 48.0, o.y + 26.0),
                12.5,
            ));
            crate::terminal_view::emit_terminal_view_with_glyphs(node, &desc, &s, state, &glyphs)
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

//! 布局与传送门（Portals）：弹性布局沙盒、浮层与拖拽排序。
//! lang-allow: bilingual showcase page copy; Chinese is the zh-CN half

use accesskit::Role;
use unbound_abi::{Align, Justify, RawNodeDesc};
use unbound_layout::LayoutEngine;
use unbound_math::{Rect, Vec2};
use unbound_widgets::overlay::Placement;
use unbound_widgets::tooltip::TooltipDesc;
use unbound_world::UiWorld;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant, Icon};

pub const FLEX_INFO: PageInfo = PageInfo {
    title: "Flex & box sandbox",
    short: "Flex layout",
    description: "A real flex solver behind the stage: change direction, gap and alignment and the boxes are re-solved before the frame is built.",
    crates: &["unbound-layout", "unbound-world"],
    keywords: "flex box layout gap align direction padding margin taffy",
};

pub const OVERLAYS_INFO: PageInfo = PageInfo {
    title: "Portals & overlays",
    short: "Portals",
    description: "Menus, dialogs and tooltips paint above the page on portal layers. Tooltips automatically flip and clamp within the window bounds.",
    crates: &["unbound-widgets/src/tooltip.rs", "unbound-widgets/src/overlay.rs"],
    keywords: "portal overlay modal dialog dropdown menu tooltip popup z order auto flip clamp",
};

pub const DRAG_INFO: PageInfo = PageInfo {
    title: "Drag & drop",
    short: "Drag & drop",
    description: "Pointer capture, threshold slop and drop-zone resolution. Release commits a reorder into the application state.",
    crates: &["unbound-input/src/dnd.rs", "unbound-input/src/router.rs"],
    keywords: "drag drop dnd reorder pointer capture drop zone",
};

pub const FLEX_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Flex sandbox",
        "Four items in a live flex container. The dashed outline is the solved content box; gap and alignment are read back from the solver.",
        &["unbound-layout"],
        r#"FlexBox::new()
    .direction(Axis::Row)
    .gap(gap)
    .align(Align::Center)
    .children(items)"#,
        "A layout group whose accessible description reports direction, gap and alignment.",
        &[
            KnobSpec::segmented("Direction", &["Row", "Column"], 0),
            KnobSpec::slider("Gap", 0.0, 32.0, 1.0, 12.0, "px"),
            KnobSpec::segmented("Align", &["Start", "Center", "End"], 1),
        ],
        DemoKind::Flex { box_model: false },
    )
    .with_height(290.0),
    ExampleDef::new(
        "Box model",
        "Margin, border, padding and content are painted as four nested surfaces. The values come from the same resolved layout node the renderer consumes.",
        &["unbound-layout"],
        r#"let node = LayoutNode::new(Size::px(180.0, 96.0))
    .padding(Padding::all(padding))
    .border(Border::all(1.0));"#,
        "A labelled box-model diagram; each band is described by its role.",
        &[
            KnobSpec::slider("Padding", 0.0, 32.0, 1.0, 16.0, "px"),
            KnobSpec::slider("Margin", 0.0, 24.0, 1.0, 8.0, "px"),
            KnobSpec::check("Show labels", true),
        ],
        DemoKind::Flex { box_model: true },
    )
    .with_height(250.0),
];

pub const OVERLAY_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Menus and dialogs",
        "Both surfaces are portals: they paint on layer 2 and receive hits before the page. The scrim blocks input to the content below.",
        &["unbound-input", "unbound-graph"],
        r#"Portal::new(menu)
    .layer(2)
    .anchor(button_rect)
    .dismiss_on_outside(true);"#,
        "A menu with three items and a modal dialog with Cancel / Delete actions.",
        &[
            KnobSpec::segmented("Anchor", &["Left", "Center", "Right"], 0),
            KnobSpec::toggle("Dark scrim", true),
        ],
        DemoKind::Overlays { tooltips: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Tooltips (In-Window Auto-Positioning)",
        "Hover any button — including buttons pinned to the top, bottom, left, and right edges. TooltipDesc automatically flips across axes and clamps within the window bounds.",
        &["unbound-widgets/src/tooltip.rs", "unbound-widgets/src/overlay.rs"],
        r#"let tip = TooltipDesc::labeled(anchor, viewport, "Open command palette")
    .with_placement(Placement::Top)
    .with_viewport_margin(8.0);
let placed = tip.placed();"#,
        "Five trigger buttons (including edge/corner triggers) exposing auto-flipping in-window tooltips.",
        &[
            KnobSpec::segmented("Preferred", &["Top", "Bottom", "Left", "Right"], 0),
            KnobSpec::toggle("Show shortcut hints", true),
        ],
        DemoKind::Overlays { tooltips: true },
    )
    .with_height(260.0),
];

pub const DRAG_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Sortable cards",
    "Press and drag a card; the drop target is resolved against four slots. Reordering is committed on release, not during the drag.",
    &["unbound-input/src/dnd.rs"],
    r#"DragSource::new(index)
    .payload(Payload::Card(index))
    .drop_zone(DropZone::Slot(slot));"#,
    "Four draggable list items; position and drop target are announced during the drag.",
    &[
        KnobSpec::segmented("Density", &["Comfortable", "Compact"], 0),
        KnobSpec::toggle("Show drop zones", true),
    ],
    DemoKind::DragDrop,
)
.with_height(260.0)];

/// 弹性布局或盒模型。
pub fn flex(ctx: &mut DemoCtx<'_, '_>, box_model: bool) {
    if box_model {
        box_model_page(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let row = ctx.knob(0) < 0.5;
    let gap = ctx.knob(1);
    let alignment = ctx.knob(2) as usize;
    let inner = rect.inflate(-18.0);
    ctx.canvas
        .rect(inner, 10.0, palette.stage_bg, palette.stage_grid, 1.0, 0.0);
    let item = 62.0;
    let count = 4;

    let mut world = UiWorld::new();
    let root = world.root();
    let loc = std::panic::Location::caller();
    let justify = match alignment {
        1 => Justify::Center,
        2 => Justify::FlexEnd,
        _ => Justify::FlexStart,
    };
    let container_desc = if row {
        RawNodeDesc::container().row()
    } else {
        RawNodeDesc::container().column()
    }
    .size(inner.size.x, inner.size.y)
    .gap(gap)
    .align(Align::Center)
    .justify(justify);
    let container = world.insert(root, container_desc, loc);
    let mut children = Vec::with_capacity(count);
    for _ in 0..count {
        children.push(world.insert(container, RawNodeDesc::container().size(item, item), loc));
    }
    let mut engine = LayoutEngine::new();
    engine.solve(&mut world, container, inner.size);

    let colors = [
        palette.primary_soft,
        palette.success_soft,
        palette.warning_soft,
        palette.secondary_soft,
    ];
    let inks = [
        palette.primary,
        palette.success,
        palette.warning,
        palette.secondary,
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (index, &child) in children.iter().enumerate() {
        let solved = engine
            .rect(child)
            .unwrap_or(Rect::new(0.0, 0.0, item, item));
        let cell = Rect::new(
            inner.origin.x + solved.origin.x,
            inner.origin.y + solved.origin.y,
            solved.size.x,
            solved.size.y,
        );
        min_x = min_x.min(cell.origin.x);
        min_y = min_y.min(cell.origin.y);
        max_x = max_x.max(cell.max().x);
        max_y = max_y.max(cell.max().y);
        ctx.canvas.rect(cell, 10.0, colors[index], 0, 0.0, 0.0);
        ctx.canvas.text(
            &format!("{}", index + 1),
            cell,
            18.0,
            inks[index],
            AlignX::Center,
            AlignY::Center,
        );
    }
    let outline = if row {
        Rect::new(
            min_x,
            inner.origin.y,
            (max_x - min_x).clamp(0.0, inner.size.x),
            inner.size.y,
        )
    } else {
        Rect::new(
            inner.origin.x,
            min_y,
            inner.size.x,
            (max_y - min_y).clamp(0.0, inner.size.y),
        )
    };
    ctx.canvas.stroke(outline, 6.0, palette.grid_dot, 1.0);
    ctx.canvas.text(
        ctx.state.locale.pick(
            "content box · solved by unbound-layout",
            "内容盒 · 由 unbound-layout 实时求解",
        ),
        Rect::new(
            outline.origin.x + 4.0,
            inner.max().y - 22.0,
            (inner.size.x - 8.0).min(260.0),
            16.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}

fn box_model_page(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let padding = ctx.knob(0);
    let margin = ctx.knob(1);
    let labels = ctx.knob_on(2);
    let outer = Rect::new(
        rect.origin.x + 30.0,
        rect.origin.y + 18.0,
        (rect.size.x - 60.0).min(420.0),
        rect.size.y - 36.0,
    );
    ctx.canvas
        .rect(outer, 12.0, palette.warning_soft, palette.warning, 1.0, 0.0);
    let border = outer.inflate(-margin);
    ctx.canvas.rect(
        border,
        10.0,
        palette.secondary_soft,
        palette.secondary,
        2.0,
        0.0,
    );
    let inner = border.inflate(-padding);
    ctx.canvas
        .rect(inner, 8.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    let locale = ctx.state.locale;
    if labels {
        for (label, area, color) in [
            (
                locale.pick("margin", "外边距 (margin)"),
                outer,
                palette.warning,
            ),
            (
                locale.pick("border", "边框 (border)"),
                border,
                palette.secondary,
            ),
            (
                locale.pick("padding", "内边距 (padding)"),
                inner,
                palette.primary,
            ),
        ] {
            ctx.canvas.text(
                label,
                Rect::new(
                    area.origin.x + 10.0,
                    area.origin.y + 8.0,
                    area.size.x - 20.0,
                    16.0,
                ),
                9.5,
                color,
                AlignX::Start,
                AlignY::Top,
            );
        }
    }
    ctx.canvas.text(
        locale.pick("content", "内容区"),
        inner,
        16.0,
        palette.text,
        AlignX::Center,
        AlignY::Center,
    );
    ctx.canvas.text(
        &if locale.is_zh() {
            format!("外边距 {margin:.0} · 内边距 {padding:.0}")
        } else {
            format!("margin {margin:.0} · padding {padding:.0}")
        },
        Rect::new(outer.origin.x, outer.max().y + 8.0, outer.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
}

/// 打开菜单与模态框，或悬停提示框。
pub fn overlays(ctx: &mut DemoCtx<'_, '_>, tooltips: bool) {
    if tooltips {
        tooltip_page(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let anchor = ctx.knob(0) as usize;
    let scrim = ctx.knob_on(1);
    // 背景页面内容。
    for index in 0..3 {
        let card = Rect::new(
            rect.origin.x + 16.0 + index as f32 * 106.0,
            rect.origin.y + 20.0,
            96.0,
            76.0,
        );
        ctx.canvas
            .rect(card, 9.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        ctx.canvas.rect(
            Rect::new(card.origin.x + 10.0, card.origin.y + 10.0, 48.0, 8.0),
            4.0,
            palette.stage_grid,
            0,
            0.0,
            0.0,
        );
    }
    let menu_button = Rect::new(rect.origin.x + 22.0, rect.max().y - 54.0, 132.0, 34.0);
    let dialog_button = Rect::new(
        menu_button.max().x + 10.0,
        menu_button.origin.y,
        132.0,
        34.0,
    );
    let locale = ctx.state.locale;
    widgets::button(
        ctx.canvas,
        ctx.part(1),
        menu_button,
        locale.pick("Open menu", "打开菜单"),
        ButtonVariant::Secondary,
        true,
    );
    widgets::button(
        ctx.canvas,
        ctx.part(2),
        dialog_button,
        locale.pick("Show dialog", "打开对话框"),
        ButtonVariant::Subtle,
        true,
    );
    ctx.canvas.text(
        locale.pick(
            "portal layer 2 · z-sorted above content",
            "Portal 第 2 层 · 位于页面内容之上按 Z 序绘制",
        ),
        Rect::new(
            dialog_button.max().x + 12.0,
            menu_button.origin.y,
            rect.size.x - dialog_button.max().x + rect.origin.x - 24.0,
            34.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );

    if ctx.state.demo.dropdown_open {
        let x = match anchor {
            0 => menu_button.origin.x,
            2 => menu_button.max().x - 178.0,
            _ => menu_button.center().x - 89.0,
        };
        let menu = Rect::new(x, menu_button.origin.y - 150.0, 178.0, 142.0);
        ctx.canvas.rect(
            menu,
            10.0,
            palette.overlay_bg,
            palette.card_border,
            1.0,
            20.0,
        );
        for (index, label) in [
            locale.pick("Rename", "重命名"),
            locale.pick("Duplicate", "创建副本"),
            locale.pick("Delete", "删除"),
        ]
        .iter()
        .enumerate()
        {
            let item = Rect::new(
                menu.origin.x + 5.0,
                menu.origin.y + 5.0 + index as f32 * 42.0,
                menu.size.x - 10.0,
                38.0,
            );
            let hover = ctx.canvas.visual(ctx.part(10 + index as u16), false);
            if hover.is_hovered() {
                ctx.canvas
                    .rect(item, 7.0, palette.control_hover, 0, 0.0, 0.0);
            }
            ctx.canvas.text(
                label,
                Rect::new(
                    item.origin.x + 12.0,
                    item.origin.y,
                    item.size.x - 24.0,
                    item.size.y,
                ),
                12.0,
                if index == 2 {
                    palette.danger
                } else {
                    palette.text
                },
                AlignX::Start,
                AlignY::Center,
            );
            ctx.canvas.control_on_layer(
                ctx.part(10 + index as u16),
                item,
                Role::MenuItem,
                *label,
                unbound_input::CursorKind::Pointer,
                false,
                false,
                2,
            );
        }
    }

    if ctx.state.demo.modal_open {
        if scrim {
            ctx.canvas.begin_clip(rect);
            ctx.canvas
                .rect(rect, 10.0, palette.overlay_scrim, 0, 0.0, 0.0);
            ctx.canvas.end_clip();
        }
        let dialog = Rect::new(
            rect.center().x - 170.0,
            rect.center().y - 82.0,
            340.0,
            164.0,
        );
        ctx.canvas.rect(
            dialog,
            14.0,
            palette.overlay_bg,
            palette.card_border,
            1.0,
            28.0,
        );
        let cancel_label = locale.pick("Cancel", "取消");
        let delete_label = locale.pick("Delete", "删除");
        ctx.canvas.text(
            locale.pick("Delete this item?", "确认删除该条目？"),
            Rect::new(
                dialog.origin.x + 18.0,
                dialog.origin.y + 16.0,
                dialog.size.x - 36.0,
                26.0,
            ),
            17.0,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text_wrapped(
            locale.pick(
                "The action can be undone from the history timeline. The portal captures focus until dismissed.",
                "此操作可在历史时间线中撤销。关闭前 Portal 会捕获键盘焦点。",
            ),
            Rect::new(dialog.origin.x + 18.0, dialog.origin.y + 50.0, dialog.size.x - 36.0, 50.0),
            11.0,
            palette.text_muted,
        );
        let cancel = Rect::new(dialog.origin.x + 18.0, dialog.max().y - 48.0, 92.0, 32.0);
        let delete = Rect::new(dialog.max().x - 112.0, cancel.origin.y, 94.0, 32.0);
        widgets::button(
            ctx.canvas,
            ctx.part(20),
            cancel,
            cancel_label,
            ButtonVariant::Secondary,
            true,
        );
        widgets::button(
            ctx.canvas,
            ctx.part(21),
            delete,
            delete_label,
            ButtonVariant::Primary,
            true,
        );
        // 在传送门层上重新注册对话框动作。
        for (node, rect, role, label) in [
            (ctx.part(20), cancel, Role::Button, cancel_label),
            (ctx.part(21), delete, Role::Button, delete_label),
        ] {
            ctx.canvas.control_on_layer(
                node,
                rect,
                role,
                label,
                unbound_input::CursorKind::Pointer,
                false,
                false,
                2,
            );
        }
    }
}

/// 将 [`Placement`] 方位翻译为当前语言环境文案（替代裸 `{chosen:?}`）。
fn localized_placement(state: &crate::state::GalleryState, placement: Placement) -> &str {
    let key = match placement {
        Placement::Top => "tooltip.placement.top",
        Placement::TopStart => "tooltip.placement.top_start",
        Placement::TopEnd => "tooltip.placement.top_end",
        Placement::Bottom => "tooltip.placement.bottom",
        Placement::BottomStart => "tooltip.placement.bottom_start",
        Placement::BottomEnd => "tooltip.placement.bottom_end",
        Placement::Left => "tooltip.placement.left",
        Placement::LeftStart => "tooltip.placement.left_start",
        Placement::LeftEnd => "tooltip.placement.left_end",
        Placement::Right => "tooltip.placement.right",
        Placement::RightStart => "tooltip.placement.right_start",
        Placement::RightEnd => "tooltip.placement.right_end",
    };
    state.t(key)
}

/// Tooltip 演示页气泡模式：纯文本 Win11 样式或富内容组合样式。
#[derive(Copy, Clone)]
enum TooltipDemoKind {
    Plain {
        text_key: &'static str,
    },
    Rich {
        title_key: &'static str,
        desc_key: &'static str,
    },
}

fn tooltip_page(ctx: &mut DemoCtx<'_, '_>) {
    use unbound_theme::Token;
    use unbound_widgets::kbd::{emit_kbd_chord, KbdChordDesc, KbdSize};
    use unbound_widgets::primitive::{glyphs as emit_glyphs, surface};
    use unbound_widgets::tooltip::{TOOLTIP_FONT_SIZE, TOOLTIP_MAX_WIDTH, TOOLTIP_PADDING};
    use unbound_widgets::{ControlState, WidgetOutput};

    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let preferred = match ctx.knob(0) as usize {
        1 => Placement::Bottom,
        2 => Placement::Left,
        3 => Placement::Right,
        _ => Placement::Top,
    };
    let show_hints = ctx.knob_on(1);

    let btn_w = 104.0_f32.min((rect.size.x - 24.0) * 0.3);
    let btn_h = 34.0;
    let triggers: [(Rect, TooltipDemoKind, Icon, &[&str]); 5] = [
        (
            Rect::new(rect.origin.x + 6.0, rect.origin.y + 6.0, btn_w, btn_h),
            TooltipDemoKind::Rich {
                title_key: "tooltip.demo.tl.title",
                desc_key: "tooltip.demo.tl.desc",
            },
            Icon::Search,
            &["Ctrl", "K"],
        ),
        (
            Rect::new(
                rect.max().x - btn_w - 6.0,
                rect.origin.y + 6.0,
                btn_w,
                btn_h,
            ),
            TooltipDemoKind::Plain {
                text_key: "tooltip.demo.tr.plain",
            },
            Icon::Moon,
            &["T"],
        ),
        (
            Rect::new(
                rect.center().x - btn_w * 0.5,
                rect.center().y - btn_h * 0.5,
                btn_w,
                btn_h,
            ),
            TooltipDemoKind::Rich {
                title_key: "tooltip.demo.center.title",
                desc_key: "tooltip.demo.center.desc",
            },
            Icon::Layers,
            &["Space"],
        ),
        (
            Rect::new(
                rect.origin.x + 6.0,
                rect.max().y - btn_h - 26.0,
                btn_w,
                btn_h,
            ),
            TooltipDemoKind::Plain {
                text_key: "tooltip.demo.bl.plain",
            },
            Icon::Chart,
            &["F12"],
        ),
        (
            Rect::new(
                rect.max().x - btn_w - 6.0,
                rect.max().y - btn_h - 26.0,
                btn_w,
                btn_h,
            ),
            TooltipDemoKind::Rich {
                title_key: "tooltip.demo.br.title",
                desc_key: "tooltip.demo.br.desc",
            },
            Icon::Snap,
            &["Ctrl", "N"],
        ),
    ];

    let mut active: Option<(u64, Rect, TooltipDemoKind, &[&str])> = None;

    for (index, (button, kind, icon, shortcut_keys)) in triggers.iter().enumerate() {
        let node = ctx.part(30 + index as u16);
        let visual = widgets::button(
            ctx.canvas,
            node,
            *button,
            "",
            ButtonVariant::Secondary,
            true,
        );
        widgets::icon(ctx.canvas, *icon, *button, palette.text, palette.card_bg);
        if show_hints {
            let mode_tag = match kind {
                TooltipDemoKind::Plain { .. } => ctx.state.t("tooltip.demo.kind.plain"),
                TooltipDemoKind::Rich { .. } => ctx.state.t("tooltip.demo.kind.rich"),
            };
            ctx.canvas.text(
                mode_tag,
                Rect::new(
                    button.origin.x + 6.0,
                    button.origin.y + 2.0,
                    button.size.x - 12.0,
                    12.0,
                ),
                8.5,
                palette.text_muted,
                AlignX::Start,
                AlignY::Top,
            );
            let shortcut_label = shortcut_keys.join("+");
            ctx.canvas.text(
                &shortcut_label,
                Rect::new(
                    button.origin.x + 6.0,
                    button.origin.y + 2.0,
                    button.size.x - 12.0,
                    12.0,
                ),
                8.5,
                palette.text_muted,
                AlignX::End,
                AlignY::Top,
            );
        }
        if visual.is_hovered() {
            active = Some((node, *button, *kind, *shortcut_keys));
        }
    }

    if let Some((anchor_node, anchor, kind, shortcut_keys)) = active {
        let tip_node = ctx.part(40);
        match kind {
            TooltipDemoKind::Plain { text_key } => {
                let base = ctx.state.t(text_key);
                let probe = TooltipDesc::labeled(anchor, rect, base)
                    .with_placement(preferred)
                    .with_gap(6.0)
                    .with_viewport_margin(6.0);
                let placement_label = localized_placement(ctx.state, probe.chosen_placement());
                let full_text = format!("{base} ({placement_label})");
                ctx.canvas.emit_tooltip_plain(
                    tip_node,
                    anchor_node,
                    anchor,
                    rect,
                    &full_text,
                    preferred,
                );
            }
            TooltipDemoKind::Rich {
                title_key,
                desc_key,
            } => {
                let title = ctx.state.t(title_key);
                let desc_text = ctx.state.t(desc_key);
                let icon_size = 18.0;
                let icon_gap = 8.0;
                let kbd_gap = 8.0;
                let probe_chord =
                    KbdChordDesc::from_keys(Vec2::ZERO, shortcut_keys).with_size(KbdSize::Sm);
                let kbd_bounds = probe_chord.bounds();
                let title_w = ctx.canvas.labels.measure(title, TOOLTIP_FONT_SIZE).x;
                let row1_w = icon_size + icon_gap + title_w + kbd_gap + kbd_bounds.width();
                let row1_h = kbd_bounds.height().max(16.0);
                let row2_h = 15.0;
                let content_h = row1_h + 4.0 + row2_h;
                let probe =
                    TooltipDesc::rich(anchor, rect, Vec2::new(row1_w.max(180.0), content_h))
                        .with_placement(preferred)
                        .with_gap(6.0)
                        .with_viewport_margin(6.0);
                let placement_label = localized_placement(ctx.state, probe.chosen_placement());
                let body_line = format!("{desc_text} · {placement_label}");
                let body_w = ctx.canvas.labels.measure(&body_line, 11.0).x;
                let max_content_w = (TOOLTIP_MAX_WIDTH - TOOLTIP_PADDING.horizontal()).max(1.0);
                let content_w = row1_w
                    .max(icon_size + icon_gap + body_w)
                    .clamp(160.0, max_content_w);
                let a11y_label =
                    format!("{title} — {body_line} ({})", probe_chord.formatted_text());
                let desc = TooltipDesc::rich(anchor, rect, Vec2::new(content_w, content_h))
                    .with_label(a11y_label)
                    .with_placement(preferred)
                    .with_gap(6.0)
                    .with_viewport_margin(6.0);

                ctx.canvas.emit_tooltip_rich(
                    tip_node,
                    anchor_node,
                    &desc,
                    |canvas, content_rect, content_z| {
                        let mut child = WidgetOutput::default();
                        // 左侧图标徽章（纯展示圆角底板 + 中心强调点）。
                        let badge_r = Rect::new(
                            content_rect.origin.x,
                            content_rect.origin.y + (row1_h - icon_size) * 0.5,
                            icon_size,
                            icon_size,
                        );
                        child
                            .primitives
                            .push(surface(badge_r, 4.0, Token::ACCENT_SOFT, content_z));
                        let inner_dot =
                            Rect::new(badge_r.origin.x + 5.0, badge_r.origin.y + 5.0, 8.0, 8.0);
                        child.primitives.push(surface(
                            inner_dot,
                            4.0,
                            Token::ACCENT,
                            content_z.saturating_add(1),
                        ));

                        // 第一行：标题与右侧 Kbd 快捷键徽章。
                        let text_x = content_rect.origin.x + icon_size + icon_gap;
                        let title_w_avail =
                            (content_rect.max().x - text_x - kbd_bounds.width() - kbd_gap).max(0.0);
                        let title_r =
                            Rect::new(text_x, content_rect.origin.y, title_w_avail, row1_h);
                        let (title_quads, _) = canvas.layout_text(
                            title,
                            title_r,
                            TOOLTIP_FONT_SIZE,
                            AlignX::Start,
                            AlignY::Center,
                        );
                        emit_glyphs(
                            &title_quads,
                            Token::TEXT_PRIMARY,
                            content_z,
                            &mut child.primitives,
                            &mut child.payloads,
                        );

                        let kbd_x = (content_rect.max().x - kbd_bounds.width()).max(text_x);
                        let kbd_y = content_rect.origin.y + (row1_h - kbd_bounds.height()) * 0.5;
                        let mut chord =
                            KbdChordDesc::from_keys(Vec2::new(kbd_x, kbd_y), shortcut_keys)
                                .with_size(KbdSize::Sm);
                        chord.z = content_z;
                        let mut kbd_quads = Vec::new();
                        for (key, cap_rect) in chord.keys.iter().zip(chord.keycap_rects()) {
                            let (q, _) = canvas.layout_text(
                                key,
                                cap_rect,
                                10.0,
                                AlignX::Center,
                                AlignY::Center,
                            );
                            kbd_quads.extend(q);
                        }
                        let kbd_out =
                            emit_kbd_chord(0, &chord, ControlState::default(), &kbd_quads);
                        child.append_child_primitives(kbd_out);

                        // 第二行：正文说明与本地化放置方位标签。
                        let body_r = Rect::new(
                            text_x,
                            content_rect.origin.y + row1_h + 4.0,
                            (content_rect.max().x - text_x).max(0.0),
                            row2_h,
                        );
                        let (body_quads, _) = canvas.layout_text(
                            &body_line,
                            body_r,
                            11.0,
                            AlignX::Start,
                            AlignY::Center,
                        );
                        emit_glyphs(
                            &body_quads,
                            Token::TEXT_SECONDARY,
                            content_z,
                            &mut child.primitives,
                            &mut child.payloads,
                        );

                        child
                    },
                );
            }
        }
    }

    ctx.canvas.text(
        ctx.state.t("tooltip.demo.hint"),
        Rect::new(rect.origin.x, rect.max().y - 20.0, rect.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Center,
        AlignY::Center,
    );
}

/// 可排序卡片行。
pub fn drag_drop(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let compact = ctx.knob(0) > 0.5;
    let show_zones = ctx.knob_on(1);
    let gap = 12.0;
    let card_w = ((rect.size.x - 36.0 - gap * 3.0) / 4.0).max(84.0);
    let card_h = if compact { 64.0 } else { 84.0 };
    let top = rect.origin.y + (rect.size.y - card_h) * 0.5;
    let locale = ctx.state.locale;
    let card_titles = [
        locale.pick("Hero", "首屏横幅"),
        locale.pick("Sidebar", "侧边栏"),
        locale.pick("Grid", "内容网格"),
        locale.pick("Footer", "页脚栏"),
    ];
    let order = ctx.state.demo.drag_order.clone();
    for (slot, index) in order.iter().copied().enumerate() {
        let cell = Rect::new(
            rect.origin.x + 18.0 + slot as f32 * (card_w + gap),
            top,
            card_w,
            card_h,
        );
        if show_zones {
            ctx.canvas.stroke(cell, 9.0, palette.grid_dot, 1.0);
        }
        let drag_index = ctx.state.demo.drag_active;
        if drag_index == Some(index) {
            ctx.canvas.rect(cell, 9.0, palette.stage_bg, 0, 0.0, 0.0);
            ctx.canvas.text(
                locale.pick("drop here", "放置于此"),
                cell,
                9.5,
                palette.text_muted,
                AlignX::Center,
                AlignY::Center,
            );
            ctx.control(
                1 + index as u16,
                cell,
                Role::ListItem,
                &format!("{} {}", locale.pick("Card", "卡片"), index + 1),
                false,
            );
            continue;
        }
        let card = Rect::new(cell.origin.x, cell.origin.y, card_w, card_h);
        let hover = ctx.canvas.visual(ctx.part(1 + index as u16), false);
        ctx.canvas.rect(
            card,
            9.0,
            if hover.is_hovered() {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            palette.card_border,
            1.0,
            6.0,
        );
        ctx.canvas.rect(
            Rect::new(card.origin.x + 10.0, card.origin.y + 10.0, 24.0, 24.0),
            6.0,
            palette.primary,
            0,
            0.0,
            0.0,
        );
        ctx.canvas.text(
            &format!("{}", index + 1),
            Rect::new(card.origin.x + 10.0, card.origin.y + 10.0, 24.0, 24.0),
            11.0,
            palette.on_primary,
            AlignX::Center,
            AlignY::Center,
        );
        ctx.canvas.text(
            card_titles[index],
            Rect::new(
                card.origin.x + 42.0,
                card.origin.y + 12.0,
                card.size.x - 50.0,
                14.0,
            ),
            10.5,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        ctx.canvas.text(
            if compact {
                locale.pick("card", "卡片")
            } else {
                locale.pick("drag to reorder", "拖拽以排序")
            },
            Rect::new(
                card.origin.x + 10.0,
                card.max().y - 20.0,
                card.size.x - 20.0,
                14.0,
            ),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.control(
            1 + index as u16,
            card,
            Role::ListItem,
            &format!("{} {}", locale.pick("Card", "卡片"), index + 1),
            false,
        );
    }
    if let (Some(index), Some(position)) =
        (ctx.state.demo.drag_active, ctx.state.demo.drag_position)
    {
        let card = Rect::new(
            position.x - card_w * 0.5,
            position.y - card_h * 0.5,
            card_w,
            card_h,
        );
        ctx.canvas.rect(card, 9.0, palette.primary, 0, 0.0, 22.0);
        ctx.canvas.text(
            card_titles[index],
            Rect::new(
                card.origin.x + 12.0,
                card.origin.y,
                card.size.x - 24.0,
                card.size.y,
            ),
            13.0,
            palette.on_primary,
            AlignX::Start,
            AlignY::Center,
        );
    }
}

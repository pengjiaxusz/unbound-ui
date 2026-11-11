//! Layout & Portals: flex sandbox, overlays and drag-and-drop.

use accesskit::Role;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant, Icon};

pub const FLEX_INFO: PageInfo = PageInfo {
    title: "Flex & box sandbox",
    short: "Flex layout",
    description: "A real flex solver behind the stage: change direction, gap and alignment and the boxes are re-solved before the frame is built.",
    crates: &["unbound-layout", "unbound-world"],
    docs: "docs/03-core/layout.md",
    keywords: "flex box layout gap align direction padding margin taffy",
};

pub const OVERLAYS_INFO: PageInfo = PageInfo {
    title: "Portals & overlays",
    short: "Portals",
    description: "Menus, dialogs and tooltips paint above the page on portal layers. Hit testing follows paint order, so the topmost surface wins.",
    crates: &["unbound-input/src/hit.rs", "unbound-graph"],
    docs: "docs/03-core/input.md",
    keywords: "portal overlay modal dialog dropdown menu tooltip popup z order",
};

pub const DRAG_INFO: PageInfo = PageInfo {
    title: "Drag & drop",
    short: "Drag & drop",
    description: "Pointer capture, threshold slop and drop-zone resolution. Release commits a reorder into the application state.",
    crates: &["unbound-input/src/dnd.rs", "unbound-input/src/router.rs"],
    docs: "docs/03-core/input.md",
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
        "Tooltips",
        "Hover any button. Tooltips are portals too, positioned above or below the anchor and never clipped by the stage.",
        &["unbound-input/src/hit.rs"],
        r#"Tooltip::new("Open command palette")
    .placement(Placement::Bottom)
    .delay(Duration::from_millis(400));"#,
        "Three buttons each exposing a tooltip via `aria-describedby`.",
        &[
            KnobSpec::segmented("Placement", &["Top", "Bottom"], 1),
            KnobSpec::toggle("Show role hints", true),
        ],
        DemoKind::Overlays { tooltips: true },
    )
    .with_height(240.0),
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

fn align_offset(index: usize, count: usize, extent: f32, width: f32, alignment: usize) -> f32 {
    let start = match alignment {
        2 => extent - count as f32 * width,
        1 => (extent - count as f32 * width) * 0.5,
        _ => 0.0,
    };
    start + index as f32 * width
}

/// Flex layout or the box model.
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
    let space = item + gap;
    let count = 4;
    let content_w = count as f32 * space - gap;
    for index in 0..count {
        let (x, y) = if row {
            (
                inner.origin.x + align_offset(index, count, inner.size.x, space, alignment),
                inner.origin.y + (inner.size.y - item) * 0.5,
            )
        } else {
            (
                inner.origin.x + (inner.size.x - item) * 0.5,
                inner.origin.y + align_offset(index, count, inner.size.y, space, alignment),
            )
        };
        let cell = Rect::new(x, y, item, item);
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
            inner.origin.x + (inner.size.x - content_w) * 0.0,
            inner.origin.y,
            content_w.min(inner.size.x),
            inner.size.y,
        )
    } else {
        Rect::new(
            inner.origin.x,
            inner.origin.y,
            inner.size.x,
            content_w.min(inner.size.y),
        )
    };
    ctx.canvas.stroke(outline, 6.0, palette.grid_dot, 1.0);
    ctx.canvas.text(
        "content box · solved by unbound-layout",
        Rect::new(
            outline.origin.x + 4.0,
            inner.max().y - 22.0,
            inner.size.x - 8.0,
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
    if labels {
        for (label, area, color) in [
            ("margin", outer, palette.warning),
            ("border", border, palette.secondary),
            ("padding", inner, palette.primary),
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
        "content",
        inner,
        16.0,
        palette.text,
        AlignX::Center,
        AlignY::Center,
    );
    ctx.canvas.text(
        &format!("margin {margin:.0} · padding {padding:.0}"),
        Rect::new(outer.origin.x, outer.max().y + 8.0, outer.size.x, 16.0),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );
}

/// Open menu and modal, or hover tooltips.
pub fn overlays(ctx: &mut DemoCtx<'_, '_>, tooltips: bool) {
    if tooltips {
        tooltip_page(ctx);
        return;
    }
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let anchor = ctx.knob(0) as usize;
    let scrim = ctx.knob_on(1);
    // Background page content.
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
    widgets::button(
        ctx.canvas,
        ctx.part(1),
        menu_button,
        "Open menu",
        ButtonVariant::Secondary,
        true,
    );
    widgets::button(
        ctx.canvas,
        ctx.part(2),
        dialog_button,
        "Show dialog",
        ButtonVariant::Subtle,
        true,
    );
    ctx.canvas.text(
        "portal layer 2 · z-sorted above content",
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
        for (index, label) in ["Rename", "Duplicate", "Delete"].iter().enumerate() {
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
        ctx.canvas.text(
            "Delete this item?",
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
            "The action can be undone from the history timeline. The portal captures focus until dismissed.",
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
            "Cancel",
            ButtonVariant::Secondary,
            true,
        );
        widgets::button(
            ctx.canvas,
            ctx.part(21),
            delete,
            "Delete",
            ButtonVariant::Primary,
            true,
        );
        // Re-register the dialog actions on the portal layer.
        for (node, rect, role, label) in [
            (ctx.part(20), cancel, Role::Button, "Cancel"),
            (ctx.part(21), delete, Role::Button, "Delete"),
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

fn tooltip_page(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let placement_bottom = ctx.knob(0) > 0.5;
    let show_hints = ctx.knob_on(1);
    let buttons = [
        ("Command palette", Icon::Search, "Ctrl+K"),
        ("Toggle theme", Icon::Moon, "T"),
        ("Toggle HUD", Icon::Chart, "F12"),
    ];
    for (index, (label, icon, shortcut)) in buttons.iter().enumerate() {
        let button = Rect::new(
            rect.origin.x + 36.0 + index as f32 * 132.0,
            rect.origin.y + rect.size.y * 0.5 - 20.0,
            116.0,
            40.0,
        );
        let node = ctx.part(30 + index as u16);
        let visual = widgets::button(ctx.canvas, node, button, "", ButtonVariant::Secondary, true);
        widgets::icon(ctx.canvas, *icon, button, palette.text, palette.card_bg);
        if show_hints {
            ctx.canvas.text(
                shortcut,
                Rect::new(button.origin.x, button.max().y + 6.0, button.size.x, 14.0),
                9.0,
                palette.text_muted,
                AlignX::Center,
                AlignY::Top,
            );
        }
        if visual.is_hovered() {
            let tip = Rect::new(
                button.center().x - 82.0,
                if placement_bottom {
                    button.max().y + 22.0
                } else {
                    button.origin.y - 46.0
                },
                164.0,
                34.0,
            );
            ctx.canvas
                .rect(tip, 7.0, palette.overlay_bg, palette.card_border, 1.0, 12.0);
            ctx.canvas.text(
                label,
                Rect::new(
                    tip.origin.x + 10.0,
                    tip.origin.y,
                    tip.size.x - 20.0,
                    tip.size.y,
                ),
                10.5,
                palette.text,
                AlignX::Center,
                AlignY::Center,
            );
        }
    }
    ctx.canvas.text(
        "Hover a button · tooltip placement is a portal anchor",
        Rect::new(rect.origin.x, rect.max().y - 22.0, rect.size.x, 16.0),
        9.5,
        palette.text_muted,
        AlignX::Center,
        AlignY::Center,
    );
}

/// Sortable card row.
pub fn drag_drop(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let compact = ctx.knob(0) > 0.5;
    let show_zones = ctx.knob_on(1);
    let gap = 12.0;
    let card_w = ((rect.size.x - 36.0 - gap * 3.0) / 4.0).max(84.0);
    let card_h = if compact { 64.0 } else { 84.0 };
    let top = rect.origin.y + (rect.size.y - card_h) * 0.5;
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
                "drop here",
                cell,
                9.5,
                palette.text_muted,
                AlignX::Center,
                AlignY::Center,
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
            ["Hero", "Sidebar", "Grid", "Footer"][index],
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
            if compact { "card" } else { "drag to reorder" },
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
            &format!("Card {}", index + 1),
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
            ["Hero", "Sidebar", "Grid", "Footer"][index],
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

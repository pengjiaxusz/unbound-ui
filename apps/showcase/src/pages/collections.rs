//! 集合控件：虚拟化行与流式资源网格。

use accesskit::Role;
use unbound_app::AssetResidency;
use unbound_math::{Rect, Vec2};
use unbound_widgets::image::{emit_image_with_assets, ImageDesc, ObjectFit};
use unbound_widgets::progress::{emit_progress, ProgressDesc, ProgressState};
use unbound_widgets::skeleton::{emit_skeleton, SkeletonDesc, SkeletonState, SKELETON_PERIOD};
use unbound_widgets::{ControlState, WidgetCatalogId};

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets;

pub const LIST_INFO: PageInfo = PageInfo {
    title: "Virtualized ListView",
    short: "Virtualized list",
    description: "One hundred thousand rows, twelve live nodes. The world only materialises the visible window; scrolling swaps entities instead of allocating.",
    crates: &["unbound-world/src/virtualize.rs", "unbound-world"],
    docs: "docs/architecture/ui-and-layout.md",
    keywords: "virtual list listview scroll virtualization 100k rows windowing",
};

pub const ASSETS_INFO: PageInfo = PageInfo {
    title: "Asset streaming grid",
    short: "Asset grid",
    description: "Asynchronous decode, a ring buffer upload queue and progressive textures. The grid keeps presenting while images arrive.",
    crates: &["unbound-asset", "unbound-asset/src/ring.rs"],
    docs: "docs/architecture/hot-reload-and-assets.md",
    keywords: "asset image decode streaming ring buffer upload grid progressive",
};

pub const LIST_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "100 000 rows",
        "Scroll the stage. Row height is a knob; the active node count stays constant because the window is measured, not the data set.",
        &["unbound-world", "unbound-world/src/virtualize.rs"],
        r#"VirtualList::new(100_000)
    .row_height(32.0)
    .overscan(3)
    .scroll(state.offset)"#,
        "A list with 100 000 items; only the active window is exposed to assistive technology.",
        &[
            KnobSpec::segmented("Items", &["1 k", "10 k", "100 k"], 2),
            KnobSpec::slider("Row height", 24.0, 48.0, 2.0, 32.0, "px"),
            KnobSpec::toggle("Overscan", true),
        ],
        DemoKind::VirtualList { metrics: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Virtualization metrics",
        "The right panel is the solver's own telemetry: visible range, overscan, materialised nodes and the recycle delta per frame.",
        &["unbound-world", "unbound-kpi"],
        r#"metrics.active_nodes = visible.len() + overscan * 2;
metrics.recycled = previous - active;"#,
        "A list plus a status group describing the active range and node count.",
        &[
            KnobSpec::segmented("Items", &["1 k", "10 k", "100 k"], 1),
            KnobSpec::toggle("Show overscan", true),
        ],
        DemoKind::VirtualList { metrics: true },
    )
    .with_height(300.0),
];

pub const ASSET_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Progressive image grid",
    "Each tile runs the same pipeline: fetch, async decode, ring-buffer upload, then a progressive texture swap. Shimmer marks the in-flight region.",
    &["unbound-asset", "unbound-asset/src/ring.rs"],
    r#"AssetGrid::new(columns)
    .quality(1024)
    .stream(true)
    .decoder(Decoder::Async)""#,
    "A six-tile image grid; each tile exposes its loading percentage.",
    &[
        KnobSpec::segmented("Quality", &["512", "1024", "2048"], 1),
        KnobSpec::slider("Columns", 2.0, 5.0, 1.0, 3.0, ""),
        KnobSpec::toggle("Streaming", true),
    ],
    DemoKind::AssetGrid,
)
.with_height(300.0)];

/// 虚拟化列表。
pub fn virtual_list(ctx: &mut DemoCtx<'_, '_>, metrics: bool) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let total = match ctx.knob(0) as usize {
        0 => 1_000usize,
        1 => 10_000,
        _ => 100_000,
    };
    let row_h = ctx.knob(1).clamp(24.0, 48.0);
    let overscan = ctx.knob_on(2);
    let panel_w = if metrics {
        (rect.size.x * 0.3).clamp(150.0, 220.0)
    } else {
        0.0
    };
    let list = Rect::new(
        rect.origin.x,
        rect.origin.y,
        if metrics {
            rect.size.x - panel_w - 12.0
        } else {
            rect.size.x
        },
        rect.size.y,
    );
    ctx.canvas
        .rect(list, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    let header = Rect::new(list.origin.x, list.origin.y, list.size.x, 38.0);
    ctx.canvas.rect(header, 10.0, palette.stage_bg, 0, 0.0, 0.0);
    ctx.canvas.text(
        &format!("{} items", format_count(total)),
        Rect::new(
            header.origin.x + 14.0,
            header.origin.y,
            header.size.x * 0.5,
            header.size.y,
        ),
        12.0,
        palette.text,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.text(
        if overscan { "overscan 3" } else { "overscan 0" },
        Rect::new(
            header.max().x - 114.0,
            header.origin.y,
            100.0,
            header.size.y,
        ),
        9.5,
        palette.text_muted,
        AlignX::End,
        AlignY::Center,
    );
    let viewport = Rect::new(
        list.origin.x,
        header.max().y,
        list.size.x,
        list.max().y - header.max().y,
    );
    let scroll = ctx.state.demo.list_scroll[ctx.card].max(0.0);
    let vlist = &ctx.state.demo.virtual_lists[ctx.card];
    let update = &ctx.state.demo.virtual_updates[ctx.card];
    let first = (scroll / row_h).floor() as usize;
    let visible = (viewport.size.y / row_h).ceil() as usize + 1;
    ctx.canvas.begin_clip(viewport);
    for slot in 0..vlist.slot_count() {
        let Some(index) = vlist.item_at(slot) else {
            continue;
        };
        if index == usize::MAX || index >= total {
            continue;
        }
        let screen_y = viewport.origin.y + (vlist.item_y(index) - scroll);
        let row = Rect::new(
            viewport.origin.x,
            screen_y + 1.0,
            viewport.size.x,
            row_h - 2.0,
        );
        if row.max().y < viewport.origin.y || row.origin.y > viewport.max().y {
            continue;
        }
        let selected = ctx.state.demo.list_selected[ctx.card] == Some(index);
        ctx.canvas.rect(
            row,
            5.0,
            if selected {
                palette.primary_soft
            } else if index % 2 == 0 {
                palette.card_bg
            } else {
                palette.stage_bg
            },
            0,
            0.0,
            0.0,
        );
        let marker = Rect::new(
            row.origin.x + 12.0,
            row.origin.y + (row.size.y - 18.0) * 0.5,
            18.0,
            18.0,
        );
        ctx.canvas.rect(
            marker,
            5.0,
            if selected {
                palette.primary
            } else {
                palette.control_track
            },
            0,
            0.0,
            0.0,
        );
        ctx.canvas.text(
            &format!("{:06}", index + 1),
            Rect::new(row.origin.x + 40.0, row.origin.y, 84.0, row.size.y),
            10.0,
            if selected {
                palette.primary
            } else {
                palette.text_muted
            },
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            &format!("Entity<{}> · chunk {}", index % 64, (index / 64) % 512),
            Rect::new(
                row.origin.x + 128.0,
                row.origin.y,
                row.size.x - 148.0,
                row.size.y,
            ),
            11.0,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            "recycled",
            Rect::new(row.max().x - 84.0, row.origin.y, 72.0, row.size.y),
            9.5,
            palette.text_muted,
            AlignX::End,
            AlignY::Center,
        );
    }
    ctx.canvas.end_clip();
    let active = vlist.slot_count();
    let recycled_str = format!("{} / frame", update.recycled);
    ctx.canvas.text(
        &format!("active nodes {active} · scroll {:.0} px", scroll),
        Rect::new(
            viewport.origin.x + 12.0,
            viewport.max().y - 18.0,
            viewport.size.x - 24.0,
            14.0,
        ),
        9.0,
        palette.hud_text,
        AlignX::Start,
        AlignY::Center,
    );
    // 滚动交互控件——真实控件而非装饰。滑块可拖拽、轨道可点击；
    // 滚轮区域为*视口*（不含表头），以便钳制能触及最后一行而非差 38px。
    // 每张卡片拥有独立的区域标签和滚动偏移。
    //
    let track = Rect::new(
        list.max().x - 10.0,
        viewport.origin.y + 4.0,
        6.0,
        viewport.size.y - 8.0,
    );
    ctx.canvas
        .rect(track, 3.0, palette.control_track, 0, 0.0, 0.0);
    let max_scroll = (total as f32 * row_h - viewport.size.y).max(1.0);
    let thumb_h = (viewport.size.y * viewport.size.y / (total as f32 * row_h)).max(18.0);
    let thumb_y = track.origin.y + (track.size.y - thumb_h) * (scroll / max_scroll).clamp(0.0, 1.0);
    ctx.canvas.rect(
        Rect::new(track.origin.x, thumb_y, 6.0, thumb_h),
        3.0,
        palette.primary,
        0,
        0.0,
        0.0,
    );
    ctx.control(
        200,
        Rect::new(track.origin.x - 4.0, track.origin.y, 14.0, track.size.y),
        Role::ScrollBar,
        "Scroll list",
        false,
    );
    let zone_tag = if ctx.card == 0 { 1 } else { 5 };
    ctx.canvas.scroll_zone(viewport, zone_tag);

    if metrics {
        let panel = Rect::new(rect.max().x - panel_w, rect.origin.y, panel_w, rect.size.y);
        ctx.canvas
            .rect(panel, 10.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
        ctx.canvas.text(
            "VIRTUALIZE",
            Rect::new(
                panel.origin.x + 12.0,
                panel.origin.y + 10.0,
                panel.size.x - 24.0,
                14.0,
            ),
            9.0,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
        for (index, (label, value)) in [
            ("visible", format!("{visible}")),
            ("overscan", if overscan { "3".into() } else { "0".into() }),
            ("active", format!("{active}")),
            ("recycled", recycled_str),
            ("total", format_count(total)),
        ]
        .iter()
        .enumerate()
        {
            let y = panel.origin.y + 40.0 + index as f32 * 32.0;
            ctx.canvas.text(
                label,
                Rect::new(panel.origin.x + 12.0, y, panel.size.x - 24.0, 12.0),
                9.0,
                palette.text_muted,
                AlignX::Start,
                AlignY::Top,
            );
            ctx.canvas.text(
                value,
                Rect::new(panel.origin.x + 12.0, y + 11.0, panel.size.x - 24.0, 18.0),
                14.0,
                palette.text,
                AlignX::Start,
                AlignY::Top,
            );
        }
        widgets::badge(
            ctx.canvas,
            Rect::new(panel.origin.x + 12.0, panel.max().y - 32.0, 92.0, 20.0),
            "DOD world",
            palette.primary,
            palette.primary_soft,
        );
    }
    let mut registered = 0usize;
    for slot in 0..vlist.slot_count() {
        if registered >= 10 {
            break;
        }
        let Some(index) = vlist.item_at(slot) else {
            continue;
        };
        if index == usize::MAX || index >= total {
            continue;
        }
        let screen_y = viewport.origin.y + (vlist.item_y(index) - scroll);
        let row = Rect::new(
            viewport.origin.x,
            screen_y + 1.0,
            viewport.size.x,
            row_h - 2.0,
        );
        if row.max().y < viewport.origin.y || row.origin.y > viewport.max().y {
            continue;
        }
        let visible_slot = index.saturating_sub(first);
        ctx.control(
            100 + visible_slot as u16,
            row,
            Role::ListItem,
            &format!("Row {}", index + 1),
            ctx.state.demo.list_selected[ctx.card] == Some(index),
        );
        registered += 1;
    }
}

fn format_count(value: usize) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// 流式资源图块。
pub fn asset_grid(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let quality = ["512", "1024", "2048"][ctx.knob(0).min(2.0) as usize];
    let columns = ctx.knob(1).clamp(2.0, 5.0) as usize;
    let streaming = ctx.knob_on(2);
    let gap = 10.0;
    let rows = 2;
    let cell_w = (rect.size.x - gap * (columns - 1) as f32) / columns as f32;
    let cell_h = (rect.size.y - gap * (rows - 1) as f32) / rows as f32;
    let palette_tints = [
        palette.primary_soft,
        palette.success_soft,
        palette.warning_soft,
        palette.secondary_soft,
        palette.primary_soft,
        palette.success_soft,
        palette.warning_soft,
        palette.secondary_soft,
        palette.primary_soft,
        palette.success_soft,
    ];
    for index in 0..columns * rows {
        let col = index % columns;
        let row = index / columns;
        let cell = Rect::new(
            rect.origin.x + col as f32 * (cell_w + gap),
            rect.origin.y + row as f32 * (cell_h + gap),
            cell_w,
            cell_h,
        );
        let progress = if streaming {
            ((ctx.state.demo.elapsed * 0.18 + index as f32 * 0.13) % 1.0).max(0.08)
        } else {
            1.0
        };
        let asset_residency = ctx
            .state
            .demo
            .asset_hub
            .as_ref()
            .zip(ctx.state.demo.asset_ids.get(index).copied())
            .and_then(|(hub, id)| hub.residency(id).map(|res| (hub, id, res)));
        ctx.canvas
            .rect(cell, 9.0, palette.card_bg, palette.card_border, 1.0, 0.0);
        ctx.canvas.begin_clip(cell);
        let thumb_rect = Rect::new(
            cell.origin.x,
            cell.origin.y,
            cell.size.x,
            cell.size.y * 0.62,
        );
        ctx.canvas.rect(
            thumb_rect,
            0.0,
            palette_tints[index % palette_tints.len()],
            0,
            0.0,
            0.0,
        );
        if let Some((hub, id, res)) = asset_residency {
            if matches!(res, AssetResidency::Ready { .. }) && progress >= 1.0 {
                let mut desc = ImageDesc::new(thumb_rect, Vec2::new(16.0, 16.0), 0).with_asset(id);
                desc.fit = ObjectFit::Cover;
                let part = 100 + index as u16;
                let output =
                    emit_image_with_assets(ctx.part(part), &desc, ControlState::default(), hub);
                ctx.canvas
                    .widget_output(WidgetCatalogId::Image, ctx.part(part), &output);
            }
        }
        if progress < 1.0 || asset_residency.is_some_and(|(_, _, r)| r == AssetResidency::Loading) {
            let mut skel_state = SkeletonState::new();
            skel_state.set_phase(
                (ctx.state.demo.elapsed / SKELETON_PERIOD + index as f32 * 0.21).rem_euclid(1.0),
            );
            let skel_desc = SkeletonDesc::new(thumb_rect).with_radius(0.0).animated();
            let output = emit_skeleton(&skel_desc, &skel_state);
            ctx.canvas
                .widget_output(WidgetCatalogId::Skeleton, ctx.part(index as u16), &output);
        }
        ctx.canvas.end_clip();
        ctx.canvas.text(
            &format!("shot-{:02}.jpg", index + 1),
            Rect::new(
                cell.origin.x + 10.0,
                cell.origin.y + cell.size.y * 0.62 + 6.0,
                cell.size.x - 20.0,
                14.0,
            ),
            9.5,
            palette.text,
            AlignX::Start,
            AlignY::Top,
        );
        let bar = Rect::new(
            cell.origin.x + 10.0,
            cell.max().y - 16.0,
            cell.size.x - 20.0,
            6.0,
        );
        let bar_desc = ProgressDesc::new(bar).value(progress);
        let mut bar_state = ProgressState::new();
        bar_state.tick(1.0, Some(progress), false);
        let bar_output = emit_progress(&bar_desc, &bar_state);
        ctx.canvas.widget_output(
            WidgetCatalogId::Progress,
            ctx.part(200 + index as u16),
            &bar_output,
        );
        ctx.canvas.text(
            &format!("{quality}× · {:.0} %", progress * 100.0),
            Rect::new(
                cell.origin.x + 10.0,
                bar.origin.y - 14.0,
                cell.size.x - 20.0,
                12.0,
            ),
            9.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Top,
        );
    }
}

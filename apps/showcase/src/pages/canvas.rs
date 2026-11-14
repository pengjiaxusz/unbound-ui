//! 无限画布：视口、四叉树、框选与历史记录。

use accesskit::Role;
use unbound_canvas::{Axis, LooseQuadtree, SnapConfig, Snapper};
use unbound_math::{Rect, Vec2};

use super::{DemoCtx, DemoKind, ExampleDef, KnobSpec, PageInfo};
use crate::canvas::{AlignX, AlignY};
use crate::widgets::{self, ButtonVariant, Icon};

pub const VIEWPORT_INFO: PageInfo = PageInfo {
    title: "Viewport & quadtree",
    short: "Viewport",
    description: "An unbounded world with a quadtree spatial index. Wheel to zoom, drag to pan; only intersecting chunks are submitted to the renderer.",
    crates: &["unbound-canvas", "unbound-canvas/src/quadtree.rs"],
    docs: "docs/architecture/canvas-and-graph.md",
    keywords: "canvas viewport pan zoom quadtree cull infinite spatial index",
};

pub const MARQUEE_INFO: PageInfo = PageInfo {
    title: "Marquee & snapping",
    short: "Marquee",
    description: "Drag on the stage to box-select. Smart guides snap to node edges and centres while the marquee is live.",
    crates: &["unbound-canvas/src/select.rs", "unbound-canvas/src/snap.rs"],
    docs: "docs/architecture/canvas-and-graph.md",
    keywords: "marquee box selection snap guides align canvas",
};

pub const UNDO_INFO: PageInfo = PageInfo {
    title: "Undo / redo",
    short: "History",
    description: "Every operation lands on a bounded history stack. The timeline shows the applied cursor and the labels attached to each command.",
    crates: &["unbound-canvas/src/undo.rs"],
    docs: "docs/architecture/canvas-and-graph.md",
    keywords: "undo redo history stack timeline command",
};

const WORLD_NODES: [(&str, f32, f32, f32, f32); 12] = [
    ("Frame", -180.0, -120.0, 150.0, 92.0),
    ("Hero", 20.0, -150.0, 120.0, 70.0),
    ("Card", -210.0, 20.0, 108.0, 64.0),
    ("Grid", -60.0, 10.0, 140.0, 90.0),
    ("Badge", 120.0, -44.0, 70.0, 34.0),
    ("List", 110.0, 42.0, 120.0, 96.0),
    ("Node", -250.0, 128.0, 80.0, 54.0),
    ("Curve", -120.0, 140.0, 110.0, 58.0),
    ("Portal", 10.0, 152.0, 92.0, 52.0),
    ("Canvas", 160.0, 160.0, 130.0, 74.0),
    ("Text", -90.0, -220.0, 110.0, 40.0),
    ("Icon", 240.0, -120.0, 44.0, 44.0),
];

pub const VIEWPORT_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Pan, zoom & cull",
        "Wheel over the stage to zoom around the pointer and drag to pan. The badge reports the visible node count after quadtree culling.",
        &["unbound-canvas", "cull.comp"],
        r#"let view = Viewport::new(pan, zoom);
let chunks = quadtree.query(view.visible_world());
renderer.submit(chunks.iter().filter(|c| view.intersects(c)));"#,
        "A spatial canvas with a live zoom status; nodes expose their world coordinates.",
        &[
            KnobSpec::check("Grid", true),
            KnobSpec::toggle("Cull off-screen", true),
            KnobSpec::slider("Grid step", 16.0, 48.0, 4.0, 24.0, "px"),
        ],
        DemoKind::Viewport { quadtree: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Quadtree overlay",
        "The index is visualised directly: each level splits a chunk into four until the depth budget is exhausted.",
        &["unbound-canvas/src/quadtree.rs"],
        r#"let tree = Quadtree::new(world_bounds, max_depth);
tree.insert_all(nodes.iter().map(|n| n.bounds()));"#,
        "The quadtree overlay is described as a group of depth levels.",
        &[
            KnobSpec::slider("Depth", 1.0, 4.0, 1.0, 2.0, ""),
            KnobSpec::toggle("Show chunk labels", true),
        ],
        DemoKind::Viewport { quadtree: true },
    )
    .with_height(290.0),
];

pub const MARQUEE_EXAMPLES: &[ExampleDef] = &[
    ExampleDef::new(
        "Box selection",
        "Drag anywhere on the stage. Nodes whose centres fall inside the marquee become selected on release.",
        &["unbound-canvas/src/select.rs"],
        r#"let marquee = Rect::from_points(start, current);
selected = nodes.filter(|n| marquee.contains(n.center())).collect();"#,
        "A selection surface with twelve selectable nodes; the status line reports the count.",
        &[
            KnobSpec::slider("Snap threshold", 0.0, 24.0, 1.0, 8.0, "px"),
            KnobSpec::toggle("Smart guides", true),
        ],
        DemoKind::Marquee { snapping: false },
    )
    .with_height(300.0),
    ExampleDef::new(
        "Smart guides",
        "While dragging, edges and centres within the threshold emit alignment guides. The same solver powers node-to-node snapping.",
        &["unbound-canvas/src/snap.rs"],
        r#"for guide in snap_engine.candidates(node.bounds(), threshold) {
    overlay.line(guide.axis, guide.position, Token::WARNING);
}"#,
        "A selected node being dragged, with horizontal and vertical guide announcements.",
        &[
            KnobSpec::slider("Threshold", 0.0, 24.0, 1.0, 8.0, "px"),
            KnobSpec::check("Show guides", true),
        ],
        DemoKind::Marquee { snapping: true },
    )
    .with_height(300.0),
];

pub const UNDO_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "History timeline",
    "Move the node, then undo and redo. Redo is truncated as soon as a new command is pushed — the classic bounded history contract.",
    &["unbound-canvas/src/undo.rs"],
    r#"history.push(Command::Move { node, delta });
history.undo(); // applied cursor moves back
history.redo(); // ...until a new command truncates the redo tail"#,
    "Undo and redo buttons report their availability; the timeline exposes the applied command.",
    &[
        KnobSpec::toggle("Show labels", true),
        KnobSpec::segmented("Limit", &["32", "128", "∞"], 1),
    ],
    DemoKind::Undo,
)
.with_height(280.0)];

const fn node_color(index: usize) -> usize {
    index % 4
}

/// 平移 / 缩放 / 四叉树舞台。
pub fn viewport(ctx: &mut DemoCtx<'_, '_>, quadtree: bool) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let show_grid = if quadtree { true } else { ctx.knob_on(0) };
    let cull = ctx.knob_on(1);
    let step = if quadtree {
        24.0
    } else {
        ctx.knob(2).max(16.0)
    };
    let depth = if quadtree {
        ctx.knob(0).clamp(1.0, 4.0) as usize
    } else {
        2
    };

    ctx.canvas
        .rect(rect, 10.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.begin_clip(rect);
    if show_grid {
        let mut x = rect.origin.x + step;
        while x < rect.max().x {
            let mut y = rect.origin.y + step;
            while y < rect.max().y {
                ctx.canvas.rect(
                    Rect::new(x, y, 1.5, 1.5),
                    0.75,
                    palette.grid_dot,
                    0,
                    0.0,
                    0.0,
                );
                y += step;
            }
            x += step;
        }
    }

    let center = rect.center();
    let zoom = ctx.state.demo.canvas_zoom.clamp(0.4, 2.5);
    let pan = ctx.state.demo.canvas_pan;
    let world_to_screen = |world: Vec2| {
        Vec2::new(
            center.x + (world.x - pan.x) * zoom,
            center.y + (world.y - pan.y) * zoom,
        )
    };
    if quadtree {
        let mut cells = vec![rect];
        for _ in 0..depth {
            let mut next = Vec::with_capacity(cells.len() * 4);
            for cell in cells {
                let half = cell.size * 0.5;
                for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    next.push(Rect::new(
                        cell.origin.x + half.x * x,
                        cell.origin.y + half.y * y,
                        half.x,
                        half.y,
                    ));
                }
            }
            cells = next;
        }
        for (index, cell) in cells.iter().enumerate() {
            ctx.canvas.stroke(*cell, 0.0, palette.grid_dot, 1.0);
            if ctx.knob_on(1) && index % 5 == 0 && cell.size.x > 46.0 {
                ctx.canvas.text(
                    &format!("L{}·{}", index % (depth + 1), index),
                    Rect::new(
                        cell.origin.x + 4.0,
                        cell.origin.y + 2.0,
                        cell.size.x - 8.0,
                        12.0,
                    ),
                    8.0,
                    palette.text_muted,
                    AlignX::Start,
                    AlignY::Top,
                );
            }
        }
    }

    // 以正世界坐标（偏移 +2048）填充真实的 LooseQuadtree。
    const WORLD_HALF: f32 = 2048.0;
    let mut tree = LooseQuadtree::new(WORLD_HALF * 2.0);
    for (index, (_, wx, wy, ww, wh)) in WORLD_NODES.iter().enumerate() {
        tree.insert(
            index as u32,
            Rect::new(wx + WORLD_HALF, wy + WORLD_HALF, *ww, *wh),
            0,
        );
    }
    let world_min = Vec2::new(
        (rect.origin.x - center.x) / zoom + pan.x + WORLD_HALF,
        (rect.origin.y - center.y) / zoom + pan.y + WORLD_HALF,
    );
    let world_query = Rect::new(
        world_min.x,
        world_min.y,
        rect.size.x / zoom,
        rect.size.y / zoom,
    );
    let mut visible_ids = Vec::with_capacity(WORLD_NODES.len());
    tree.query(world_query, &mut visible_ids);
    let qt_stats = tree.stats();

    let mut visible = 0;
    for (index, (label, wx, wy, ww, wh)) in WORLD_NODES.iter().enumerate() {
        let origin = world_to_screen(Vec2::new(*wx, *wy));
        let node = Rect::new(origin.x, origin.y, ww * zoom, wh * zoom);
        let in_tree = visible_ids.contains(&(index as u32));
        let inside = in_tree
            && node.max().x > rect.origin.x
            && node.origin.x < rect.max().x
            && node.max().y > rect.origin.y
            && node.origin.y < rect.max().y;
        if !inside {
            if cull {
                continue;
            }
            ctx.canvas.stroke(node, 6.0, palette.danger_soft, 1.0);
            continue;
        }
        visible += 1;
        let tints = [
            palette.primary_soft,
            palette.success_soft,
            palette.warning_soft,
            palette.secondary_soft,
        ];
        ctx.canvas
            .rect(node, 6.0, tints[node_color(index)], 0, 0.0, 4.0);
        ctx.canvas.text(
            label,
            Rect::new(
                node.origin.x + 6.0,
                node.origin.y,
                node.size.x - 12.0,
                node.size.y,
            ),
            9.5,
            palette.text,
            AlignX::Start,
            AlignY::Center,
        );
    }
    ctx.canvas.text(
        &format!(
            "{:.0}% · pan {:.0},{:.0} · vis {visible}/{} · qtree nodes {} visited {}",
            zoom * 100.0,
            pan.x,
            pan.y,
            WORLD_NODES.len(),
            tree.occupied_nodes(),
            qt_stats.nodes_visited
        ),
        Rect::new(
            rect.origin.x + 12.0,
            rect.max().y - 24.0,
            rect.size.x - 24.0,
            18.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.end_clip();
    ctx.canvas.scroll_zone(rect, 2);
    // 拖拽空白舞台可平移视口。
    ctx.control(1, rect, Role::Group, "Canvas viewport", false);
    widgets::badge(
        ctx.canvas,
        Rect::new(rect.max().x - 96.0, rect.origin.y + 10.0, 86.0, 20.0),
        if quadtree { "quadtree" } else { "cull.comp" },
        palette.primary,
        palette.primary_soft,
    );
}

/// 框选与吸附参考线。
pub fn marquee(ctx: &mut DemoCtx<'_, '_>, snapping: bool) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let threshold = ctx.knob(0);
    let show_guides = ctx.knob_on(1);
    let inner = rect.inflate(-12.0);
    ctx.canvas
        .rect(rect, 10.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
    ctx.canvas.begin_clip(rect);
    let cols = 4;
    let rows = 3;
    let gap = 14.0;
    let cell_w = (inner.size.x - gap * (cols - 1) as f32) / cols as f32;
    let cell_h = (inner.size.y - gap * (rows - 1) as f32) / rows as f32;
    let mut selected_count = 0;
    for index in 0..12 {
        let col = index % cols;
        let row = index / cols;
        let node = Rect::new(
            inner.origin.x + col as f32 * (cell_w + gap),
            inner.origin.y + row as f32 * (cell_h + gap),
            cell_w.min(96.0),
            cell_h.min(58.0),
        );
        let center = node.center();
        let marquee_selected = ctx.state.demo.marquee.is_some_and(|(a, b)| {
            let m = Rect::from_points(a, b);
            m.contains_point(center)
        });
        let selected = ctx.state.demo.selected_nodes[index] || marquee_selected;
        if selected {
            selected_count += 1;
        }
        ctx.canvas.rect(
            node,
            8.0,
            if selected {
                palette.primary_soft
            } else {
                palette.card_bg
            },
            if selected {
                palette.primary
            } else {
                palette.card_border
            },
            if selected { 2.0 } else { 1.0 },
            if selected { 4.0 } else { 0.0 },
        );
        ctx.canvas.text(
            &format!("N{}", index + 1),
            node,
            10.0,
            if selected {
                palette.primary
            } else {
                palette.text_muted
            },
            AlignX::Center,
            AlignY::Center,
        );
        ctx.control(
            10 + index as u16,
            node,
            Role::ListItem,
            &format!("Node {}", index + 1),
            selected,
        );
    }
    if let Some((start, current)) = ctx.state.demo.marquee {
        let marquee = Rect::from_points(start, current);
        ctx.canvas.rect(
            marquee,
            0.0,
            palette.selection_fill,
            palette.primary,
            1.0,
            0.0,
        );
        if show_guides {
            let mut others = Vec::with_capacity(12);
            for node_index in 0..12 {
                let col = node_index % cols;
                let row = node_index / cols;
                others.push(Rect::new(
                    inner.origin.x + col as f32 * (cell_w + gap),
                    inner.origin.y + row as f32 * (cell_h + gap),
                    cell_w.min(96.0),
                    cell_h.min(58.0),
                ));
            }
            let mut snapper = Snapper::new(SnapConfig {
                threshold_px: threshold.max(2.0),
                ..SnapConfig::default()
            });
            let moving = Rect::new(current.x, current.y, 1.0, 1.0);
            let result = snapper.snap(moving, &others, 1.0);
            for guide in &result.guides {
                let line = match guide.axis {
                    Axis::X => Rect::new(
                        guide.coordinate,
                        guide.start.max(rect.origin.y + 8.0),
                        1.0,
                        (guide.end - guide.start).max(rect.size.y - 16.0),
                    ),
                    Axis::Y => Rect::new(
                        guide.start.max(rect.origin.x + 8.0),
                        guide.coordinate,
                        (guide.end - guide.start).max(rect.size.x - 16.0),
                        1.0,
                    ),
                };
                ctx.canvas.rect(line, 0.0, palette.warning, 0, 0.0, 0.0);
            }
        }
        ctx.canvas.text(
            &format!("{:.0} × {:.0}", marquee.size.x, marquee.size.y),
            Rect::new(marquee.origin.x, marquee.origin.y - 16.0, 96.0, 14.0),
            9.0,
            palette.primary,
            AlignX::Start,
            AlignY::Top,
        );
    }
    ctx.canvas.text(
        &format!(
            "{selected_count} selected · {} · drag to select",
            if snapping { "snap engine" } else { "marquee" }
        ),
        Rect::new(
            rect.origin.x + 12.0,
            rect.max().y - 22.0,
            rect.size.x - 24.0,
            16.0,
        ),
        9.5,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    ctx.canvas.end_clip();
    ctx.control(1, inner, Role::Group, "Selection surface", false);
    ctx.canvas.scroll_zone(rect, 3);
}

/// 撤销 / 重做历史记录。
pub fn undo(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let show_labels = ctx.knob_on(0);
    let applied = ctx.state.demo.history_index;
    let total = ctx.state.demo.history.len();

    let stage = Rect::new(
        rect.origin.x,
        rect.origin.y,
        rect.size.x * 0.56 - 8.0,
        rect.size.y,
    );
    ctx.canvas
        .rect(stage, 10.0, palette.stage_bg, palette.card_border, 1.0, 0.0);
    let progress = applied as f32 / total.max(1) as f32;
    let node = Rect::new(
        stage.origin.x + 24.0 + (stage.size.x - 172.0) * progress,
        stage.origin.y + stage.size.y * 0.5 - 30.0,
        124.0,
        60.0,
    );
    ctx.canvas.rect(node, 9.0, palette.primary, 0, 0.0, 12.0);
    ctx.canvas.text(
        "node",
        node,
        12.0,
        palette.on_primary,
        AlignX::Center,
        AlignY::Center,
    );
    let stack = &ctx.state.demo.undo_stack;
    ctx.canvas.text(
        &format!(
            "applied {applied} / {total} · stack {}/{} ({} B)",
            stack.depth(),
            stack.depth() + stack.redo_depth(),
            stack.bytes()
        ),
        Rect::new(
            stage.origin.x + 14.0,
            stage.origin.y + 10.0,
            stage.size.x - 28.0,
            16.0,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Top,
    );

    let panel = Rect::new(
        rect.max().x - rect.size.x * 0.42,
        rect.origin.y,
        rect.size.x * 0.42 - 8.0,
        rect.size.y,
    );
    ctx.canvas
        .rect(panel, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);
    let undo = Rect::new(panel.origin.x + 12.0, panel.origin.y + 12.0, 38.0, 30.0);
    let redo = Rect::new(undo.max().x + 6.0, undo.origin.y, 38.0, 30.0);
    widgets::icon_button(ctx.canvas, ctx.part(1), undo, Icon::Undo, "Undo", false);
    widgets::icon_button(ctx.canvas, ctx.part(2), redo, Icon::Redo, "Redo", false);
    ctx.canvas.text(
        &format!("{} command(s)", total),
        Rect::new(
            redo.max().x + 8.0,
            undo.origin.y,
            panel.max().x - redo.max().x - 18.0,
            30.0,
        ),
        10.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
    let timeline_top = panel.origin.y + 54.0;
    for (index, label) in ctx.state.demo.history.iter().enumerate() {
        let y = timeline_top + index as f32 * 28.0;
        if y + 24.0 > panel.max().y {
            break;
        }
        let active = index < applied;
        ctx.canvas.rect(
            Rect::new(panel.origin.x + 12.0, y + 8.0, 8.0, 8.0),
            4.0,
            if active {
                palette.primary
            } else {
                palette.control_track
            },
            0,
            0.0,
            0.0,
        );
        if index > 0 {
            ctx.canvas.rect(
                Rect::new(panel.origin.x + 15.5, y + 14.0, 1.0, 14.0),
                0.0,
                palette.divider,
                0,
                0.0,
                0.0,
            );
        }
        if show_labels {
            ctx.canvas.text(
                label,
                Rect::new(panel.origin.x + 28.0, y, panel.size.x - 40.0, 24.0),
                10.5,
                if active {
                    palette.text
                } else {
                    palette.text_muted
                },
                AlignX::Start,
                AlignY::Center,
            );
        }
        ctx.canvas.text(
            &format!("{:02}", index + 1),
            Rect::new(panel.max().x - 34.0, y, 22.0, 24.0),
            9.5,
            palette.text_muted,
            AlignX::End,
            AlignY::Center,
        );
    }
    let add = Rect::new(
        panel.origin.x + 12.0,
        panel.max().y - 40.0,
        panel.size.x - 24.0,
        28.0,
    );
    widgets::button(
        ctx.canvas,
        ctx.part(3),
        add,
        "Push “Move node”",
        ButtonVariant::Secondary,
        true,
    );
}

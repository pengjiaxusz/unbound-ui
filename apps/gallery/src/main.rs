//! 全链路演示程序（`docs` §9）。
//!
//! `gallery` runs the CPU half of the engine end to end on a real scene —
//! `UiWorld` → incremental Taffy layout → loose quadtree → hit testing →
//! smart snapping → transactional undo → AccessKit semantics → the five
//! headless sanitizers — and prints what each stage produced.
//!
//! It is a *demo and a smoke test at the same time*: every stage asserts its own
//! invariants, so `cargo run -p gallery` failing is a real signal. Pass `--kpi`
//! to also print the acceptance table from `docs` §10.2.
//!
//! ```text
//! cargo run -p gallery
//! cargo run -p gallery -- --kpi
//! ```

#![forbid(unsafe_op_in_unsafe_fn)]

use std::time::Instant;

use accesskit::Role;
use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::RawNodeDesc;
use unbound_a11y::{A11yActions, A11yNode, Semantics};
use unbound_canvas::{LooseQuadtree, Selection, SelectionMode, SnapConfig, Snapper, UndoStack};
use unbound_canvas::{Handle, HandleDrag, Marquee, Modifiers, TransactionKind};
use unbound_headless::{FrameBuilder, Harness, NodeKind};
use unbound_input::{CursorKind, HitRegion, Router};
use unbound_layout::LayoutEngine;
use unbound_math::{Rect, Vec2};
use unbound_world::{NodeId, UiWorld};

const VIEWPORT: Vec2 = Vec2 { x: 1280.0, y: 720.0 };
const ROWS: usize = 200;

fn insert(world: &mut UiWorld, parent: NodeId, desc: RawNodeDesc) -> NodeId {
    web_insert(world, parent, desc)
}

#[track_caller]
fn web_insert(world: &mut UiWorld, parent: NodeId, desc: RawNodeDesc) -> NodeId {
    world.insert(parent, desc, core::panic::Location::caller())
}

/// Absolute rectangle of a laid-out node.
///
/// `LayoutEngine::rect` is parent-relative, like Taffy's; everything outside the
/// layout engine (spatial index, hit testing, painting) works in world space.
fn world_rect(world: &UiWorld, id: NodeId) -> Rect {
    world.get(id).map_or(Rect::ZERO, |node| node.world_rect)
}

fn main() {
    let with_kpi = std::env::args().any(|arg| arg == "--kpi");
    let started = Instant::now();

    // ---- 1. World -----------------------------------------------------------
    let mut world = UiWorld::new();
    let root = world.root();
    let window = insert(&mut world, root, RawNodeDesc::text().size(VIEWPORT.x, VIEWPORT.y).column());

    let toolbar = insert(&mut world, window, RawNodeDesc::text().size(VIEWPORT.x, 48.0).row());
    let mut buttons = Vec::new();
    for (index, label) in ["New", "Open", "Save", "Export"].into_iter().enumerate() {
        let mut desc = RawNodeDesc::text().size(96.0, 32.0).row();
        desc.flex_shrink = 0.0;
        let button = insert(&mut world, toolbar, desc);
        world.set_text(button, label);
        // The action target is a stable index into the button table, not the
        // SlotMap key: the reducer must not depend on a key that a removal can
        // invalidate.
        world.bind_action(
            button,
            RawAction::new(ActionKind::Press, index as u32),
        );
        buttons.push(button);
    }

    let list = insert(&mut world, window, RawNodeDesc::text().size(VIEWPORT.x, 600.0).column());
    let mut rows = Vec::new();
    for index in 0..ROWS {
        let mut desc = RawNodeDesc::text().size(VIEWPORT.x, 32.0).row();
        // A list row keeps its height and overflows the viewport instead of
        // being squeezed to nothing by flexbox.
        desc.flex_shrink = 0.0;
        let row = insert(&mut world, list, desc);
        world.set_text(row, format!("row {index:03}"));
        world.bind_action(row, RawAction::new(ActionKind::CanvasSelect, index as u32));
        rows.push(row);
    }

    // ---- 2. Incremental layout ---------------------------------------------
    let mut engine = LayoutEngine::new();
    let layout_start = Instant::now();
    let stats = engine.solve(&mut world, root, VIEWPORT);
    let layout_us = layout_start.elapsed().as_secs_f64() * 1e6;

    // A single row edit must re-solve one boundary, not the whole tree.
    let boundary_start = Instant::now();
    let row = rows[ROWS / 2];
    engine.solve_boundaries(&mut world, &[row], VIEWPORT);
    let boundary_us = boundary_start.elapsed().as_secs_f64() * 1e6;

    // ---- 3. Spatial index ---------------------------------------------------
    let mut tree = LooseQuadtree::new(65_536.0);
    for (index, row) in rows.iter().enumerate() {
        tree.insert(index as u32, world_rect(&world, *row), 0);
    }
    let mut visible = Vec::new();
    let viewport_rect = Rect::new(0.0, 0.0, VIEWPORT.x, VIEWPORT.y);
    let cull_start = Instant::now();
    tree.query_exact(viewport_rect, &mut visible);
    let cull_us = cull_start.elapsed().as_secs_f64() * 1e6;

    // ---- 4. Picking ---------------------------------------------------------
    let mut regions: Vec<HitRegion> = visible
        .iter()
        .filter_map(|id| tree.bounds_of(*id).map(|rect| (*id, rect)))
        .map(|(id, rect)| {
            HitRegion::new(u64::from(id), 1, id, rect).with_cursor(CursorKind::Pointer)
        })
        .collect();
    for (index, button) in buttons.iter().enumerate() {
        regions.push(HitRegion::new(
            10_000 + index as u64,
            1,
            999,
            world_rect(&world, *button),
        ));
    }
    let mut router = Router::default();
    router.set_regions(regions);
    let probe = Vec2::new(600.0, 200.0);
    let hovered = router.pointer_down(probe, 0.0).and_then(|event| event.target());
    router.pointer_up(probe, 0.05);

    // ---- 5. Snapping --------------------------------------------------------
    let mut snapper = Snapper::new(SnapConfig::default());
    let dragging = world_rect(&world, rows[10]).translate(Vec2::new(3.0, 2.0));
    let snap = snapper.snap_against_tree(&mut tree, dragging, Some(10), viewport_rect, 1.0);
    let snap_offset = snap.offset;
    let guide_count = snap.guides.len();

    // ---- 6. Marquee + handles ----------------------------------------------
    let mut selection = Selection::new();
    let mut marquee = Marquee::new(Vec2::new(0.0, 100.0));
    marquee.drag_to(Vec2::new(VIEWPORT.x, 260.0));
    marquee.apply(&mut tree, SelectionMode::Replace, &mut selection);

    let selected_bounds = selection
        .bounds(|id| tree.bounds_of(id))
        .unwrap_or(Rect::ZERO);
    let mut drag = HandleDrag::new(
        Handle::SouthEast,
        selected_bounds,
        selected_bounds.max(),
        24.0,
    );
    let stretched = drag.update(selected_bounds.max() + Vec2::new(40.0, 20.0), Modifiers::NONE);

    // ---- 7. Transactions ----------------------------------------------------
    let mut undo = UndoStack::new(64);
    undo.begin(TransactionKind::Drag, "move 3 rows", 0.0);
    for index in 0..3 {
        let id = rows[index + 1];
        let before = world_rect(&world, id);
        let after = before.translate(Vec2::new(12.0, 0.0));
        undo.record_geometry(index as u32, before, after, 0.016);
    }
    undo.commit();
    let committed = undo.depth();
    let undone_label = undo.undo().map(|transaction| transaction.label());
    let redo_label = undo.redo_label();
    undo.redo();

    // ---- 8. Semantics -------------------------------------------------------
    let mut semantics = Semantics::new(1);
    semantics.upsert(
        1,
        A11yNode::new(Role::Window, "Gallery", Rect::new(0.0, 0.0, VIEWPORT.x, VIEWPORT.y)),
    );
    let mut child_ids = Vec::new();
    for (index, button) in buttons.iter().enumerate() {
        let id = 100 + index as u64;
        let rect = world_rect(&world, *button);
        let node = A11yNode::new(Role::Button, world.text(*button).unwrap_or("Button"), rect)
            .with_actions(A11yActions::single(unbound_a11y::A11yAction::Click))
            .clickable();
        semantics.upsert(id, node);
        child_ids.push(id);
    }
    semantics.set_children(1, &child_ids);
    let first_update = semantics.take_update();
    let initial_nodes = first_update.as_ref().map_or(0, |update| update.nodes.len());
    let second_update = semantics.take_update();

    // ---- 9. Headless verification ------------------------------------------
    let mut frame = FrameBuilder::new(viewport_rect);
    frame.open(NodeKind::Container, viewport_rect, "Root");
    for button in &buttons {
        let rect = world_rect(&world, *button);
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "Button",
        );
        frame.close();
    }
    for index in 0..visible.len().min(20) {
        let rect = tree.bounds_of(visible[index]).unwrap_or(Rect::ZERO);
        frame.open(NodeKind::Container, rect, "Row");
        frame.close();
    }
    frame.close();
    let frame = frame.build();
    let report = Harness::run(&frame);

    // ---- Report -------------------------------------------------------------
    println!("Unbound UI — gallery");
    println!("====================");
    println!("world      : {} nodes, {} children under the list", world.len(), rows.len());
    println!(
        "layout     : {} nodes in {layout_us:.1}µs ({} boundaries), single row re-solve {boundary_us:.1}µs",
        stats.nodes_visited, stats.boundaries_solved
    );
    println!(
        "spatial    : {} of {ROWS} rows visible, cull {cull_us:.1}µs",
        visible.len()
    );
    println!("picking    : probe {probe:?} -> {hovered:?}, cursor {:?}", router.cursor());
    println!(
        "snapping   : offset {snap_offset:?}, {guide_count} guides, {} candidates",
        snapper.retained_candidates()
    );
    println!(
        "selection  : {} rows, bounds {:?}, resize -> {:?}",
        selection.len(),
        selected_bounds,
        stretched.apply_rect(selected_bounds)
    );
    println!(
        "undo       : depth {committed}, reverted {undone_label:?}, redo {redo_label:?} re-applied"
    );
    println!(
        "semantics  : {initial_nodes} nodes in the first update, second update {}",
        if second_update.is_some() { "present" } else { "suppressed" }
    );
    println!(
        "headless   : {} — {}",
        if report.is_clean() { "clean" } else { "FAILED" },
        report.summary()
    );
    println!("total      : {:.1}µs", started.elapsed().as_secs_f64() * 1e6);

    if with_kpi {
        println!();
        print!("{}", unbound_kpi::measure_all().to_text());
    }

    assert!(report.is_clean(), "{}", report.summary());
    assert_eq!(committed, 1);
    assert_eq!(second_update, None, "an idle frame must not emit a semantic update");
}

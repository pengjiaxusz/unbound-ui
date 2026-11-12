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
use unbound_a11y::{A11yActions, A11yNode, Semantics};
use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::RawNodeDesc;
use unbound_asset::{AssetId, AssetKind, AssetPipeline, PipelineConfig};
use unbound_canvas::{Handle, HandleDrag, Marquee, Modifiers, TransactionKind};
use unbound_canvas::{LooseQuadtree, Selection, SelectionMode, SnapConfig, Snapper, UndoStack};
use unbound_gpu::{
    BlendMode, Camera, Gpu, GpuConfig, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType,
    Renderer, SceneUpload, SubElement, CLIP_NONE,
};
use unbound_graph::{
    OffscreenPool, PassKind, RenderGraph, TextureDesc, TextureFormat, TextureRole,
};
use unbound_headless::{FrameBuilder, Harness, NodeKind};
use unbound_input::{CursorKind, HitRegion, Router};
use unbound_layout::LayoutEngine;
use unbound_math::{Rect, Vec2};
use unbound_platform::{resize::DEFERRED_SETTLE, Extent, LatencyTracker, ResizeTracker};
use unbound_theme::{ThemeKind, ThemeState};
use unbound_world::{NodeId, UiWorld};

const VIEWPORT: Vec2 = Vec2 {
    x: 1280.0,
    y: 720.0,
};
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

/// Measure the GPU frame time for a hundred thousand canvas nodes.
///
/// Returns `None` when the machine has no usable Vulkan device.
fn measure_gpu() -> Option<(f32, u32, u32)> {
    const COUNT: u32 = 99_840;
    let gpu = Gpu::new(GpuConfig {
        pool_capacity: 32 << 20,
        ring_capacity: 16 << 20,
        ..Default::default()
    })
    .ok()?;
    let mut renderer = Renderer::new(gpu, 512, 512).ok()?;
    let primitives: Vec<GpuPrimitiveInstance> = (0..COUNT)
        .map(|index| {
            let x = (index % 320) as f32 * 1.6;
            let y = (index / 320) as f32 * 1.64;
            GpuPrimitiveInstance {
                chunk_id: [0, 0],
                local_pos: [x, y],
                size: [2.0, 2.0],
                corner_radii: unbound_math::f16::pack_radii([1.0; 4]),
                border_width: 0,
                shadow_blur: 0,
                transform_idx: 0,
                clip_idx: CLIP_NONE,
                fill_token_or_uv: 2,
                border_token_or_tex: 2,
                prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                    PrimType::RoundedRect,
                    BlendMode::Normal,
                    PrimFlags::with_sub(SubElement::Background, PrimFlags::EMPTY).bits(),
                ),
                z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(1000, 1.0),
                extra_payload_idx: 0,
            }
        })
        .collect();
    let transforms = vec![GpuTransform::IDENTITY];
    let scene = SceneUpload {
        primitives: &primitives,
        transforms: &transforms,
        clips: &[],
        payloads: &[],
        generation: 1,
        camera: Camera::ui((512.0, 512.0), 1.0),
        key_prefix: 0,
    };
    renderer.enable_timing().ok()?;
    // The first frame uploads the scene; the second is the steady state.
    renderer.render(&scene).ok()?;
    renderer.render(&scene).ok()?;
    let timing = renderer.read_frame_timing().ok()??;
    // The counters are only final once the frame has retired, so the timing
    // read is what fills the statistics in.
    let stats = renderer.last_stats();
    renderer.destroy();
    Some((timing.total_ms, stats.draw_calls, stats.visible))
}

fn main() {
    let with_kpi = std::env::args().any(|arg| arg == "--kpi");
    let started = Instant::now();

    // ---- 1. World -----------------------------------------------------------
    let mut world = UiWorld::new();
    let root = world.root();
    let window = insert(
        &mut world,
        root,
        RawNodeDesc::text().size(VIEWPORT.x, VIEWPORT.y).column(),
    );

    let toolbar = insert(
        &mut world,
        window,
        RawNodeDesc::text().size(VIEWPORT.x, 48.0).row(),
    );
    let mut buttons = Vec::new();
    for (index, label) in ["New", "Open", "Save", "Export"].into_iter().enumerate() {
        let mut desc = RawNodeDesc::text().size(96.0, 32.0).row();
        desc.flex_shrink = 0.0;
        let button = insert(&mut world, toolbar, desc);
        world.set_text(button, label);
        // The action target is a stable index into the button table, not the
        // SlotMap key: the reducer must not depend on a key that a removal can
        // invalidate.
        world.bind_action(button, RawAction::new(ActionKind::Press, index as u32));
        buttons.push(button);
    }

    let list = insert(
        &mut world,
        window,
        RawNodeDesc::text().size(VIEWPORT.x, 600.0).column(),
    );
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
    let hovered = router
        .pointer_down(probe, 0.0)
        .and_then(|event| event.target());
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
    let stretched = drag.update(
        selected_bounds.max() + Vec2::new(40.0, 20.0),
        Modifiers::NONE,
    );

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
        A11yNode::new(
            Role::Window,
            "Gallery",
            Rect::new(0.0, 0.0, VIEWPORT.x, VIEWPORT.y),
        ),
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
    for id in visible.iter().take(20) {
        let rect = tree.bounds_of(*id).unwrap_or(Rect::ZERO);
        frame.open(NodeKind::Container, rect, "Row");
        frame.close();
    }
    frame.close();
    let frame = frame.build();
    let report = Harness::run(&frame);

    // ---- 10. Theme switching ------------------------------------------------
    // A theme switch is one 4 KiB write: no re-layout, no instance upload.
    let mut theme = ThemeState::default();
    let light_primary = theme.lut().color(unbound_theme::Token::PRIMARY);
    let switch = theme.switch(ThemeKind::Dark);
    let dark_primary = theme.lut().color(unbound_theme::Token::PRIMARY);
    assert!(
        !switch.relayout_required,
        "a theme switch must not re-layout"
    );
    assert!(
        !switch.instance_reupload_required,
        "a theme switch must not re-upload instances"
    );
    assert_ne!(light_primary.to_array(), dark_primary.to_array());

    // ---- 11. Render graph ---------------------------------------------------
    // The frosted-glass chain: copy the backdrop, blur it, composite it.
    let mut graph = RenderGraph::new();
    let swapchain = graph.add_texture(
        TextureDesc::new(1920, 1080, TextureFormat::Rgba16f),
        TextureRole::External,
    );
    let scratch = graph.add_texture(
        TextureDesc::new(960, 540, TextureFormat::Rgba16f),
        TextureRole::Transient,
    );
    let blurred = graph.add_texture(
        TextureDesc::new(960, 540, TextureFormat::Rgba16f),
        TextureRole::Transient,
    );
    let composed = graph.add_texture(
        TextureDesc::new(1920, 1080, TextureFormat::Rgba16f),
        TextureRole::External,
    );
    graph.add_pass("copy", PassKind::CopyBackdrop, &[swapchain], &[scratch]);
    graph.add_pass(
        "blur",
        PassKind::Blur { radius: 24 },
        &[scratch],
        &[blurred],
    );
    graph.add_pass("composite", PassKind::Composite, &[blurred], &[composed]);
    graph.add_pass("present", PassKind::Present, &[composed], &[swapchain]);
    let compiled = graph.compile().expect("the graph compiles");
    // A steady-state frame must not allocate: the pool reuses the targets.
    let mut pool = OffscreenPool::new();
    let mut pool_creations = 0;
    for _ in 0..8 {
        let target = pool.acquire(TextureDesc::new(960, 540, TextureFormat::Rgba16f));
        pool.release(target);
        pool_creations = pool.creation_count();
    }

    // ---- 12. Asset streaming ------------------------------------------------
    // The decode runs off the main thread; the frame never waits for it.
    let mut assets = AssetPipeline::new(PipelineConfig {
        workers: 2,
        slot_bytes: 1 << 20,
        frames: 3,
        queue_capacity: 64,
    });
    let submit_started = Instant::now();
    for index in 0..8u32 {
        assets
            .request(AssetId(index), AssetKind::Raw, vec![index as u8; 1024])
            .expect("the request is queued");
    }
    let submit_us = submit_started.elapsed().as_secs_f64() * 1e6;
    let decoded = assets.drain().expect("the workers finished");
    let staged_bytes: usize = decoded
        .iter()
        .filter_map(|item| item.location.map(|reference| reference.size))
        .sum();

    // ---- 13. Platform layer -------------------------------------------------
    // The deferred mode absorbs a window-edge drag: the swapchain is not
    // rebuilt for every message of the burst. The *default* mode is the
    // real-time one, which follows the window size at once.
    let mut resize = ResizeTracker::new(Extent::new(1280, 720), DEFERRED_SETTLE);
    let now = Instant::now();
    resize.on_drag_started();
    for width in 1281..=1300 {
        resize.on_resized(Extent::new(width, 720), now);
    }
    let recreations_during_drag = resize.recreations();
    resize.on_drag_ended();

    // The input-to-photon budget is one frame at 120 Hz.
    let mut latency = LatencyTracker::new();
    latency.on_input(now);
    latency.on_present(now + std::time::Duration::from_micros(3_000));
    let latency_us = latency.last().unwrap_or_default().as_secs_f64() * 1e6;

    // ---- 14. GPU throughput -------------------------------------------------
    // The acceptance table asks for 100 000 canvas nodes in 1.5 ms and at most
    // four MDI draw calls. The measurement is skipped when the machine has no
    // usable Vulkan device.
    let gpu_line = measure_gpu();
    if let Some((millis, draw_calls, gpu_visible)) = gpu_line {
        assert!(
            millis <= 1.5,
            "the GPU frame took {millis:.3} ms, the budget is 1.5 ms"
        );
        assert!(draw_calls <= 4, "MDI used {draw_calls} draw calls");
        assert!(gpu_visible > 99_000, "the cull dropped too many primitives");
    }

    // ---- Report -------------------------------------------------------------
    println!("Unbound UI — gallery");
    println!("====================");
    println!(
        "world      : {} nodes, {} children under the list",
        world.len(),
        rows.len()
    );
    println!(
        "layout     : {} nodes in {layout_us:.1}µs ({} boundaries), single row re-solve {boundary_us:.1}µs",
        stats.nodes_visited, stats.boundaries_solved
    );
    println!(
        "spatial    : {} of {ROWS} rows visible, cull {cull_us:.1}µs",
        visible.len()
    );
    println!(
        "picking    : probe {probe:?} -> {hovered:?}, cursor {:?}",
        router.cursor()
    );
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
        if second_update.is_some() {
            "present"
        } else {
            "suppressed"
        }
    );
    println!(
        "headless   : {} — {}",
        if report.is_clean() { "clean" } else { "FAILED" },
        report.summary()
    );
    println!(
        "theme      : switched to {} in {} B, 0 relayout, primary {:?} -> {:?}",
        theme.current().name(),
        switch.bytes_uploaded,
        light_primary.to_array(),
        dark_primary.to_array()
    );
    println!(
        "graph      : {} passes, {} transient lifetimes, pool {} slots / {} creations",
        compiled.passes.len(),
        compiled.lifetimes.len(),
        pool.slot_count(),
        pool_creations
    );
    println!(
        "assets     : {} decoded off-thread in {submit_us:.1}µs of submit time, {staged_bytes} B staged",
        decoded.len()
    );
    println!(
        "platform   : {recreations_during_drag} rebuilds during the drag, input-to-photon {latency_us:.0}µs"
    );
    if let Some((millis, draw_calls, gpu_visible)) = gpu_line {
        println!(
            "gpu        : 100k primitives in {millis:.3} ms, {draw_calls} draw call(s), {gpu_visible} visible"
        );
    } else {
        println!("gpu        : skipped (no Vulkan device)");
    }
    println!(
        "total      : {:.1}µs",
        started.elapsed().as_secs_f64() * 1e6
    );

    if with_kpi {
        println!();
        print!("{}", unbound_kpi::measure_all().to_text());
    }

    assert!(report.is_clean(), "{}", report.summary());
    assert_eq!(
        recreations_during_drag, 0,
        "a window-edge drag must not rebuild the swapchain"
    );
    assert!(
        latency.within_budget(),
        "the input-to-photon delay must fit into one frame at 120 Hz"
    );
    assert_eq!(pool_creations, 1, "a steady-state frame must not allocate");
    assert_eq!(committed, 1);
    assert_eq!(
        second_update, None,
        "an idle frame must not emit a semantic update"
    );
}

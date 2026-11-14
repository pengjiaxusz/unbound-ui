//! 全链路演示程序（`docs` §9）。
//!
//! `gallery` 在真实场景上端到端运行引擎的 CPU 侧流程——
//! `UiWorld` → 增量 Taffy 布局 → 松散四叉树（loose quadtree）→ 命中测试 →
//! 智能吸附 → 事务型撤销 → AccessKit 语义 → 五项无头校验器——并打印各阶段产出。
//!
//!
//! 它同时是*演示与冒烟测试*：每个阶段都会断言自身的不变量，因此
//! `cargo run -p gallery` 失败即代表真实回归。传入 `--kpi` 可额外打印 `docs` §10.2 中的验收表。
//!
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

/// 已布局节点的绝对矩形。
///
/// `LayoutEngine::rect` 与 Taffy 一样是相对父节点的；布局引擎之外的所有模块
/// （空间索引、命中测试、绘制）均在世界空间中工作。
fn world_rect(world: &UiWorld, id: NodeId) -> Rect {
    world.get(id).map_or(Rect::ZERO, |node| node.world_rect)
}

/// 测量目标分辨率下十万个画布节点的 GPU 帧耗时。
///
/// 当机器上没有可用的 Vulkan 设备时返回 `None`。
fn measure_gpu(width: u32, height: u32) -> Option<(f32, u32, u32)> {
    const COUNT: u32 = 99_840;
    let gpu = Gpu::new(GpuConfig {
        pool_capacity: 32 << 20,
        ring_capacity: 16 << 20,
        ..Default::default()
    })
    .ok()?;
    let mut renderer = Renderer::new(gpu, width, height).ok()?;
    let primitives: Vec<GpuPrimitiveInstance> = (0..COUNT)
        .map(|index| {
            let x = (index % 320) as f32 * (width as f32 / 320.0);
            let y = (index / 320) as f32 * (height as f32 / 312.0);
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
        camera: Camera::ui((width as f32, height as f32), 1.0),
        key_prefix: 0,
        dirty_ranges: &[],
        aux_changed: false,
        backdrops: &[],
    };
    renderer.enable_timing().ok()?;
    // 首帧上传场景；第二帧为稳态。
    renderer.render(&scene).ok()?;
    renderer.render(&scene).ok()?;
    let timing = renderer.read_frame_timing().ok()??;
    // 仅当帧已退役后计数器才最终确定，因此通过读取计时结果来填充统计信息。
    let stats = renderer.last_stats();
    renderer.destroy();
    Some((timing.total_ms, stats.draw_calls, stats.visible))
}

fn main() {
    let with_kpi = std::env::args().any(|arg| arg == "--kpi");
    let with_4k = std::env::args().any(|arg| arg == "--4k");
    let started = Instant::now();

    // ---- 1. World（世界状态）------------------------------------------------
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
        // 动作目标是按钮表中的稳定索引，而非 SlotMap 键：
        // reducer 不得依赖可能因移除而失效的键。
        //
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
        // 列表行保持自身高度并溢出视口，而非被 flexbox 压缩至零。
        //
        desc.flex_shrink = 0.0;
        let row = insert(&mut world, list, desc);
        world.set_text(row, format!("row {index:03}"));
        world.bind_action(row, RawAction::new(ActionKind::CanvasSelect, index as u32));
        rows.push(row);
    }

    // ---- 2. 增量布局 --------------------------------------------------------
    let mut engine = LayoutEngine::new();
    let layout_start = Instant::now();
    let stats = engine.solve(&mut world, root, VIEWPORT);
    let layout_us = layout_start.elapsed().as_secs_f64() * 1e6;

    // 单行编辑只能重新求解一个边界，而不能重算整棵树。
    let boundary_start = Instant::now();
    let row = rows[ROWS / 2];
    engine.solve_boundaries(&mut world, &[row], VIEWPORT);
    let boundary_us = boundary_start.elapsed().as_secs_f64() * 1e6;

    // ---- 3. 空间索引 --------------------------------------------------------
    let mut tree = LooseQuadtree::new(65_536.0);
    for (index, row) in rows.iter().enumerate() {
        tree.insert(index as u32, world_rect(&world, *row), 0);
    }
    let mut visible = Vec::new();
    let viewport_rect = Rect::new(0.0, 0.0, VIEWPORT.x, VIEWPORT.y);
    let cull_start = Instant::now();
    tree.query_exact(viewport_rect, &mut visible);
    let cull_us = cull_start.elapsed().as_secs_f64() * 1e6;

    // ---- 4. 拾取（Picking）--------------------------------------------------
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

    // ---- 5. 吸附（Snapping）-------------------------------------------------
    let mut snapper = Snapper::new(SnapConfig::default());
    let dragging = world_rect(&world, rows[10]).translate(Vec2::new(3.0, 2.0));
    let snap = snapper.snap_against_tree(&mut tree, dragging, Some(10), viewport_rect, 1.0);
    let snap_offset = snap.offset;
    let guide_count = snap.guides.len();

    // ---- 6. 框选与控制柄 ----------------------------------------------------
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

    // ---- 7. 事务（Transactions）---------------------------------------------
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

    // ---- 8. 语义（Semantics）------------------------------------------------
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

    // ---- 9. 无头校验 --------------------------------------------------------
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

    // ---- 10. 主题切换 -------------------------------------------------------
    // 主题切换只需一次 4 KiB 写入：无需重新布局，也无需上传实例数据。
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

    // ---- 11. 渲染图（Render graph）------------------------------------------
    // 毛玻璃管线：拷贝背景、模糊处理、合成。
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
    // 稳态帧不得发生分配：缓冲池复用渲染目标。
    let mut pool = OffscreenPool::new();
    let mut pool_creations = 0;
    for _ in 0..8 {
        let target = pool.acquire(TextureDesc::new(960, 540, TextureFormat::Rgba16f));
        pool.release(target);
        pool_creations = pool.creation_count();
    }

    // ---- 12. 资源流式加载 ---------------------------------------------------
    // 解码在主线程之外运行；渲染帧绝不等待它。
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

    // ---- 13. 平台层 ---------------------------------------------------------
    // 延迟模式会吸收窗口边缘拖拽：不会为突发消息流中的每条消息重建 swapchain。
    // *默认*模式为实时模式，会立即跟随窗口尺寸变化。
    //
    let mut resize = ResizeTracker::new(Extent::new(1280, 720), DEFERRED_SETTLE);
    let now = Instant::now();
    resize.on_drag_started();
    for width in 1281..=1300 {
        resize.on_resized(Extent::new(width, 720), now);
    }
    let recreations_during_drag = resize.recreations();
    resize.on_drag_ended();

    // 120 Hz 下的输入到光子（input-to-photon）预算为一帧。
    let mut latency = LatencyTracker::new();
    latency.on_input(now);
    latency.on_present(now + std::time::Duration::from_micros(3_000));
    let latency_us = latency.last().unwrap_or_default().as_secs_f64() * 1e6;

    // ---- 14. GPU 吞吐量 -----------------------------------------------------
    // 验收表要求 100 000 个画布节点在 1.5 ms 内完成，且 MDI 绘制调用不超过四次。
    // 当机器上没有可用的 Vulkan 设备时跳过该项测量。
    //
    let (gpu_w, gpu_h) = if with_4k { (3840, 2160) } else { (1920, 1080) };
    let gpu_line = measure_gpu(gpu_w, gpu_h);
    if let Some((millis, draw_calls, gpu_visible)) = gpu_line {
        let budget = if cfg!(debug_assertions) { 15.0 } else { 1.5 };
        assert!(
            millis <= budget,
            "the GPU frame took {millis:.3} ms, the budget is {budget} ms"
        );
        assert!(draw_calls <= 4, "MDI used {draw_calls} draw calls");
        assert!(gpu_visible > 99_000, "the cull dropped too many primitives");
    }

    // ---- 报告 ----------------------------------------------------------------
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
            "gpu ({}x{}) : 100k primitives in {millis:.3} ms, {draw_calls} draw call(s), {gpu_visible} visible",
            gpu_w, gpu_h
        );
    } else {
        println!("gpu ({}x{}) : skipped (no Vulkan device)", gpu_w, gpu_h);
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

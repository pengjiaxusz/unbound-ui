//! 展厅必须真正绘制出内容，且按钮必须真正可交互。
//! CPU 部分通过 `unbound_headless` 中的无头驱动和 `showcase::testing` 中的按钮装置
//! 在所有环境（包括无 GPU 的 CI）运行；像素部分在没有 Vulkan 设备时自动跳过。
//!
//!
//!

use accesskit::Role;
use showcase::button::{ButtonVisualState, PointerPhase};
use showcase::gallery::{build, button_rect, palette, Labels};
use showcase::keys::{self, KeyAction};
use showcase::testing::ButtonHarness;
use unbound_gpu::data::PrimType;
use unbound_input::PointerEvent;
use unbound_math::Vec2;
use unbound_platform::{
    apply, ButtonState, Extent, MouseButton, PlatformEvent, PlatformKey, PlatformState,
    PresentWaitPolicy, WindowConfig,
};

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 720.0;

fn has_fill(primitives: &[unbound_gpu::data::GpuPrimitiveInstance], token: u16) -> bool {
    primitives.iter().any(|primitive| {
        primitive.prim_type() == PrimType::RoundedRect
            && primitive.fill_token_or_uv == u32::from(token)
    })
}

#[test]
fn hover_and_press_switch_the_fill_token() {
    let mut harness = ButtonHarness::new();

    harness.hover();
    assert_eq!(harness.visual().phase, PointerPhase::Hovered);
    assert!(
        has_fill(&harness.scene.primitives, palette().primary_hover),
        "hover must use the hover slot, not overwrite the shared primary slot"
    );

    harness.press();
    assert_eq!(harness.visual().phase, PointerPhase::Pressed);
    assert!(has_fill(
        &harness.scene.primitives,
        palette().primary_pressed
    ));
}

#[test]
fn a_drag_off_the_button_cancels_the_click() {
    let mut harness = ButtonHarness::new();
    harness.hover();
    harness.press();
    assert_eq!(harness.visual().phase, PointerPhase::Pressed);

    // 路由器捕获了指针，因此指针离开时按钮仍保持按下状态——
    // 这正是旧的 `bool` 方案会丢失的状态。
    let centre = harness.screen_center();
    harness.app.move_to(centre + Vec2::new(0.0, 200.0));
    harness.sync();
    assert_eq!(harness.visual().phase, PointerPhase::Pressed);

    let events = harness.release();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { .. })),
        "a drag that leaves the button must not synthesise a click"
    );
    assert_eq!(harness.count, 0);
    assert_eq!(harness.visual().phase, PointerPhase::Idle);
}

#[test]
fn a_release_inside_emits_a_click() {
    let mut harness = ButtonHarness::new();
    harness.hover();
    harness.press();
    let events = harness.release();
    assert!(
        events.iter().any(
            |event| matches!(event, PointerEvent::Click { target, .. } if *target == harness.node_key())
        ),
        "a release inside the button must produce a click for its node"
    );
    assert_eq!(harness.count, 1);
    assert_eq!(harness.visual().phase, PointerPhase::Hovered);
}

#[test]
fn a_click_changes_the_rendered_label() {
    let mut harness = ButtonHarness::new();
    let idle_uv: Vec<[f32; 4]> = harness.glyphs.iter().map(|quad| quad.uv).collect();

    harness.click();
    let clicked_uv: Vec<[f32; 4]> = harness.glyphs.iter().map(|quad| quad.uv).collect();

    assert_eq!(idle_uv.len(), clicked_uv.len());
    assert_ne!(
        idle_uv, clicked_uv,
        "the click count must reach the glyph quads, otherwise nothing repaints"
    );
    assert_eq!(harness.label(), "Clicked: 1");
}

#[test]
fn the_focus_ring_is_a_stroke_and_focus_stays_orthogonal() {
    let mut harness = ButtonHarness::new();
    harness.hover();
    harness.focus();

    let visual = harness.visual();
    assert_eq!(visual.phase, PointerPhase::Hovered);
    assert!(visual.focused);

    assert!(
        harness.scene.primitives.iter().any(|primitive| {
            primitive.prim_type() == PrimType::Stroke
                && primitive.border_token_or_tex == u32::from(palette().focus_ring)
        }),
        "a focused button must emit an outer focus ring"
    );
    assert!(has_fill(&harness.scene.primitives, palette().primary_hover));
    assert_eq!(harness.app.semantics_node(harness.node).role, Role::Button);
}

#[test]
fn the_semantics_tree_exposes_a_button() {
    let mut harness = ButtonHarness::new();
    harness.rebuild_with(ButtonVisualState::default(), 3);

    assert_eq!(
        harness.app.router.hover(),
        None,
        "no pointer has entered yet"
    );
    assert_eq!(
        harness.app.pick(harness.screen_center()),
        Some(harness.node_key())
    );
    assert!(!harness.glyphs.is_empty(), "the label must be laid out");

    let node = harness.app.semantics_node(harness.node);
    assert_eq!(node.role, Role::Button);
    assert_eq!(node.label, "Clicked: 3");
    assert_eq!(node.value, "3");

    let update = harness
        .app
        .semantics
        .take_update()
        .expect("initial tree update");
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.role() == Role::Button),
        "the update AccessKit receives must carry the button role"
    );

    harness.app.set_focus(Some(harness.node));
    let update = harness.app.semantics.take_update().expect("focus update");
    assert_eq!(update.focus, accesskit::NodeId(harness.node_key()));
}

/// 在无窗口的情况下驱动完全相同的窗口管线：平台事件必须被转发、以画布坐标到达路由器、
/// 合成一次点击，并使下一帧场景携带递增后的标签。
///
///
/// 这是针对“平台层在仅更新延迟时钟后就消费掉所有输入事件，导致整个 showcase 失去响应”
/// 这一缺陷的回归测试。
///
#[test]
fn a_platform_click_activates_the_button() {
    use std::time::Instant;

    let mut harness = ButtonHarness::new();
    let mut state = PlatformState::new(
        &WindowConfig::default(),
        Extent::new(WIDTH as u32, HEIGHT as u32),
        PresentWaitPolicy::from_extensions(&[]),
    );
    let now = Instant::now();
    let centre = harness.screen_center();

    // ---- 悬停 ----------------------------------------------------------------
    let forwarded = apply(
        &mut state,
        &PlatformEvent::CursorMoved {
            x: f64::from(centre.x),
            y: f64::from(centre.y),
        },
        now,
    );
    let [PlatformEvent::CursorMoved { x, y }] = forwarded.as_slice() else {
        panic!("the cursor must reach the application, got {forwarded:?}");
    };
    harness.app.move_to(Vec2::new(*x as f32, *y as f32));
    harness.sync();
    assert_eq!(harness.visual().phase, PointerPhase::Hovered);

    // ---- 按下 ----------------------------------------------------------------
    let forwarded = apply(
        &mut state,
        &PlatformEvent::MouseInput {
            state: ButtonState::Pressed,
            button: MouseButton::Left,
        },
        now,
    );
    assert!(
        matches!(forwarded.as_slice(), [PlatformEvent::MouseInput { .. }]),
        "the press must reach the application, got {forwarded:?}"
    );
    harness.press();
    assert_eq!(harness.visual().phase, PointerPhase::Pressed);

    // ---- 释放 ----------------------------------------------------------------
    let forwarded = apply(
        &mut state,
        &PlatformEvent::MouseInput {
            state: ButtonState::Released,
            button: MouseButton::Left,
        },
        now,
    );
    assert!(
        matches!(forwarded.as_slice(), [PlatformEvent::MouseInput { .. }]),
        "the release must reach the application, got {forwarded:?}"
    );
    let events = harness.release();
    let clicks = events
        .iter()
        .filter(|event| {
            matches!(event, PointerEvent::Click { target, .. } if *target == harness.node_key())
        })
        .count();
    assert_eq!(
        clicks, 1,
        "one press/release pair must synthesize one click"
    );
    assert_eq!(
        harness.visual().phase,
        PointerPhase::Hovered,
        "the pointer still rests on the button after the click"
    );

    // ---- 下一帧场景以新计数作为标签 -----------------------------------------
    assert_eq!(harness.label(), "Clicked: 1");
    assert_eq!(harness.value(), "1");
    assert!(!harness.glyphs.is_empty(), "the new label must be laid out");
}

#[test]
fn the_button_passes_the_five_sanitizers() {
    let report = ButtonHarness::new().assert_clean();
    assert!(report.is_clean());
    assert!(report.nodes >= 3, "root, card, button (and label)");
}

#[test]
fn a_disabled_button_leaves_the_hit_set_and_never_activates() {
    let mut harness = ButtonHarness::new();
    let centre = harness.screen_center();
    harness.set_disabled(true);

    assert_eq!(
        harness.app.pick(centre),
        None,
        "a disabled control must not be hittable"
    );
    assert_eq!(harness.visual().phase, PointerPhase::Idle);
    assert!(
        has_fill(&harness.scene.primitives, palette().disabled_fill),
        "the disabled state must use its own fill token"
    );
    assert!(!harness.glyphs.is_empty(), "the label still renders");

    let report = harness.click();
    assert_eq!(report.clicked(), None);
    assert_eq!(harness.count, 0);
    assert_eq!(harness.label(), "Clicked: 0");
    assert!(harness.app.semantics_node(harness.node).disabled);

    harness.set_disabled(false);
    assert_eq!(harness.app.pick(centre), Some(harness.node_key()));
    harness.click();
    assert_eq!(harness.count, 1);
}

#[test]
fn the_scene_is_only_rebuilt_on_discrete_transitions() {
    let mut harness = ButtonHarness::new();
    let idle = harness.generation();

    harness.hover();
    let hovered = harness.generation();
    assert!(hovered > idle, "entering the button is a rebuild");

    let centre = harness.screen_center();
    harness.hover_at(centre + Vec2::new(20.0, 0.0));
    assert_eq!(
        harness.generation(),
        hovered,
        "sliding inside the button must not upload a new scene"
    );
    harness.sync();
    harness.sync();
    assert_eq!(
        harness.generation(),
        hovered,
        "a steady frame must not rebuild"
    );

    harness.press();
    let pressed = harness.generation();
    assert!(pressed > hovered, "pressing is a rebuild");

    harness.release();
    let clicked = harness.generation();
    assert!(clicked > pressed, "the click count changed the label");
    harness.sync();
    assert_eq!(
        harness.generation(),
        clicked,
        "the click is already baked into the current scene"
    );
}

#[test]
fn the_keyboard_mapping_matches_the_window_and_can_activate_the_button() {
    let mut harness = ButtonHarness::new();
    let named = |name: &str| PlatformKey::Named(name.to_owned());

    assert_eq!(
        keys::resolve(&named("tab"), false),
        Some(KeyAction::FocusButton)
    );
    harness.focus();
    assert!(harness.visual().focused);

    assert_eq!(
        keys::resolve(&named("enter"), true),
        Some(KeyAction::Activate)
    );
    harness.activate();
    assert_eq!(harness.count, 1);
    assert_eq!(harness.label(), "Clicked: 1");

    assert_eq!(
        keys::resolve(&named("space"), true),
        Some(KeyAction::Activate)
    );
    assert_eq!(
        keys::resolve(&named("space"), false),
        Some(KeyAction::ToggleTheme),
        "Space without focus keeps its theme meaning"
    );
    assert_eq!(
        keys::resolve(&named("escape"), true),
        Some(KeyAction::ClearFocus)
    );
    harness.blur();
    assert!(!harness.visual().focused);

    assert_eq!(
        keys::resolve(&PlatformKey::Character("t".to_owned()), false),
        Some(KeyAction::ToggleTheme)
    );
    assert_eq!(
        keys::resolve(&named("enter"), false),
        None,
        "Enter without focus does nothing"
    );
    assert_eq!(keys::resolve(&named("q"), true), None);
}

#[test]
fn hover_and_press_change_the_rendered_pixels() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // 无 Vulkan 设备：测试无法运行，但不得失败。
        return;
    };
    let mut renderer =
        unbound_gpu::Renderer::new(gpu, WIDTH as u32, HEIGHT as u32).expect("renderer");
    let atlases = std::sync::Arc::new(
        unbound_gpu::atlas::AtlasSet::new(&renderer.gpu, 4).expect("atlas set"),
    );
    renderer.install_atlases(atlases.clone()).expect("install");

    let mut harness = ButtonHarness::new();
    let idle = render_button(&mut renderer, &atlases, &mut harness, PointerPhase::Idle);
    let hover = render_button(&mut renderer, &atlases, &mut harness, PointerPhase::Hovered);
    let pressed = render_button(&mut renderer, &atlases, &mut harness, PointerPhase::Pressed);

    assert!(
        differing_pixels(&idle, &hover) > 200,
        "hover must change the button's pixels, not only its bookkeeping"
    );
    assert!(
        differing_pixels(&hover, &pressed) > 200,
        "pressing must change the button's pixels"
    );
    renderer.destroy();
}

/// 用相同颜色过度绘制表面不得改变任何一个像素。
///
/// 旧的合成曾将填充色二次乘以自身覆盖率，并把阴影色混入部分覆盖像素，
/// 导致绘制在同色表面上的圆角矩形沿抗锯齿边缘留下一圈暗边。
///
///
#[test]
fn overdrawing_the_same_colour_leaves_no_seam() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // 无 Vulkan 设备：测试无法运行，但不得失败。
        return;
    };
    let mut renderer = unbound_gpu::Renderer::new(gpu, 256, 256).expect("renderer");
    let atlases = std::sync::Arc::new(
        unbound_gpu::atlas::AtlasSet::new(&renderer.gpu, 4).expect("atlas set"),
    );
    renderer.install_atlases(atlases).expect("install");
    renderer
        .write_theme_lut(&showcase::theme::lut(false))
        .expect("theme");
    let scene = showcase::testing::same_colour_overlay_scene((256.0, 256.0));
    renderer.render(&scene.upload(1, &[])).expect("render");
    let pixels = renderer.read_pixels().expect("readback");
    let first = &pixels[0..4];
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        assert_eq!(
            pixel, first,
            "pixel {index} is not the flat surface colour: the same-colour overlay left a seam"
        );
    }
    renderer.destroy();
}

fn render_button(
    renderer: &mut unbound_gpu::Renderer,
    atlases: &std::sync::Arc<unbound_gpu::atlas::AtlasSet>,
    harness: &mut ButtonHarness,
    phase: PointerPhase,
) -> Vec<u8> {
    paint_button(renderer, atlases, harness, phase, None)
}

/// 渲染单个按钮状态，全量上传整个场景。
///
/// `patch` 选择上传路径：`None` 在全新代次下全量上传，
/// `Some((generation, ranges))` 保持代次并仅修补指定切片——即悬停所走的路径。
///
fn paint_button(
    renderer: &mut unbound_gpu::Renderer,
    atlases: &std::sync::Arc<unbound_gpu::atlas::AtlasSet>,
    harness: &mut ButtonHarness,
    phase: PointerPhase,
    patch: Option<(u64, Vec<(u32, u32)>)>,
) -> Vec<u8> {
    let visual = ButtonVisualState {
        phase,
        focused: false,
        disabled: false,
        selected: false,
    };
    harness.rebuild_with(visual, 0);
    upload_atlases(renderer, atlases, harness);
    renderer
        .write_theme_lut(&showcase::theme::lut(false))
        .expect("theme");
    match &patch {
        // 修补路径保持缓冲池当前所处的代次，并列出发生变化的切片；
        // 渲染器仅重写这些切片。
        Some((generation, ranges)) => {
            renderer
                .render(&harness.scene.upload(*generation, ranges))
                .expect("render");
        }
        // 每个状态从驱动获取各自独立的代次，否则图元缓冲区会被视为未变化，
        // 从而再次绘制上一状态。
        //
        None => {
            renderer
                .render(&harness.scene.upload(harness.app.generation(), &[]))
                .expect("render");
        }
    }
    renderer.read_pixels().expect("readback")
}

/// 将上次重建光栅化的所有字形发送到 GPU 图集。
fn upload_atlases(
    renderer: &mut unbound_gpu::Renderer,
    atlases: &std::sync::Arc<unbound_gpu::atlas::AtlasSet>,
    harness: &mut ButtonHarness,
) {
    for update in harness.labels.cache.take_updates() {
        let kind = match update.target {
            unbound_text::AtlasTarget::Coverage => unbound_gpu::atlas::AtlasKind::R8,
            unbound_text::AtlasTarget::Color => unbound_gpu::atlas::AtlasKind::Rgba8,
        };
        atlases
            .upload(
                &renderer.gpu,
                &unbound_gpu::atlas::AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                },
            )
            .expect("upload");
    }
}

/// 修补上传绘制的像素与全量上传完全一致。
///
/// 修补路径只写入图元池的切片并保持其余部分不变，因此错误的偏移、长度或遗漏刷写
/// 都会在此处表现为过期像素。两种方式下的场景相同，因此两次回读必须逐字节一致。
///
///
#[test]
fn a_patched_upload_paints_the_same_pixels_as_a_full_one() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // 无 Vulkan 设备：测试无法运行，但不得失败。
        return;
    };
    let mut renderer =
        unbound_gpu::Renderer::new(gpu, WIDTH as u32, HEIGHT as u32).expect("renderer");
    let atlases = std::sync::Arc::new(
        unbound_gpu::atlas::AtlasSet::new(&renderer.gpu, 4).expect("atlas set"),
    );
    renderer.install_atlases(atlases.clone()).expect("install");
    renderer
        .write_theme_lut(&showcase::theme::lut(false))
        .expect("theme");

    let mut harness = ButtonHarness::new();
    let visual = |phase| ButtonVisualState {
        phase,
        focused: false,
        disabled: false,
        selected: false,
    };

    // 1. 空闲按钮在代次 7 下全量上传。
    let idle = paint_button(
        &mut renderer,
        &atlases,
        &mut harness,
        PointerPhase::Idle,
        None,
    );
    let generation = 7;
    let idle_primitives = harness.scene.primitives.clone();
    let idle_transforms = harness.scene.transforms.clone();
    let idle_clips = harness.scene.clips.clone();
    let idle_payloads = harness.scene.payloads.clone();

    // 2. 悬停按钮是同一场景仅少数颜色变化，因此走修补路径。
    //
    harness.rebuild_with(visual(PointerPhase::Hovered), 0);
    upload_atlases(&mut renderer, &atlases, &mut harness);
    let hovered = &harness.scene;
    let diff = unbound_widgets::diff::diff_primitives(
        &idle_primitives,
        &hovered.primitives,
        idle_transforms != hovered.transforms
            || idle_clips != hovered.clips
            || idle_payloads != hovered.payloads,
    );
    let unbound_widgets::diff::SceneDiff::Ranges(ranges) = diff else {
        panic!("a hover must be a patch, got {diff:?}");
    };

    // 3. 修补缓冲池，然后全量上传同一场景。两者都必须绘制出悬停状态的按钮。
    //
    renderer
        .render(&hovered.upload(generation, &ranges))
        .expect("patched render");
    let patched = renderer.read_pixels().expect("readback");
    renderer
        .render(&hovered.upload(generation + 1, &[]))
        .expect("full render");
    let full = renderer.read_pixels().expect("readback");

    assert_eq!(
        patched,
        full,
        "the patch left stale pixels behind: {} differ",
        differing_pixels(&patched, &full)
    );
    assert!(
        differing_pixels(&patched, &idle) > 200,
        "the hover must actually have changed the picture"
    );
    renderer.destroy();
}

fn differing_pixels(left: &[u8], right: &[u8]) -> usize {
    let button = button_rect();
    let mut differing = 0usize;
    let mut y = button.origin.y + 8.0;
    while y < button.origin.y + button.height() - 8.0 {
        let mut x = button.origin.x + 8.0;
        while x < button.origin.x + button.width() - 8.0 {
            let index = ((y as usize) * WIDTH as usize + x as usize) * 4;
            if left[index..index + 4] != right[index..index + 4] {
                differing += 1;
            }
            x += 4.0;
        }
        y += 4.0;
    }
    differing
}

#[test]
fn the_gallery_draws_something() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // 无 Vulkan 设备：测试无法运行，但不得失败。
        return;
    };
    let mut renderer = unbound_gpu::Renderer::new(gpu, 640, 400).expect("renderer");

    // 使用真实图集，避免标签从虚拟的 1x1 纹理中采样。
    //
    let atlases = std::sync::Arc::new(
        unbound_gpu::atlas::AtlasSet::new(&renderer.gpu, 4).expect("atlas set"),
    );
    renderer.install_atlases(atlases.clone()).expect("install");

    let mut labels = Labels::new();
    let (scene, _) = build(&mut labels, &palette(), (640.0, 400.0));
    assert!(
        scene.primitives.len() > 10,
        "the gallery must contain many primitives, found {}",
        scene.primitives.len()
    );

    // 上传标签生成的字形。
    for update in labels.cache.take_updates() {
        let kind = match update.target {
            unbound_text::AtlasTarget::Coverage => unbound_gpu::atlas::AtlasKind::R8,
            unbound_text::AtlasTarget::Color => unbound_gpu::atlas::AtlasKind::Rgba8,
        };
        atlases
            .upload(
                &renderer.gpu,
                &unbound_gpu::atlas::AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                },
            )
            .expect("upload");
    }

    renderer
        .write_theme_lut(&showcase::theme::lut(false))
        .expect("theme");
    renderer.render(&scene.upload(1, &[])).expect("render");
    let pixels = renderer.read_pixels().expect("readback");

    // 画面不能全都是统一背景：色板、卡片与曲线必须存在。
    //
    let mut distinct = 0usize;
    let mut previous = &pixels[0..4];
    for pixel in pixels.chunks_exact(4) {
        if pixel != previous {
            distinct += 1;
        }
        previous = pixel;
    }
    assert!(
        distinct > 500,
        "the gallery must draw more than a flat background, found {distinct} colour changes"
    );
    renderer.destroy();
}

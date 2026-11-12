//! The gallery must actually draw something, and the button must actually be
//! interactive.
//!
//! The CPU half runs everywhere (including CI without a GPU) through the
//! headless driver in `unbound_headless` and the button harness in
//! `showcase::testing`; the pixel half skips itself when no Vulkan device
//! exists.

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

    // The router captured the pointer, so the button stays pressed while the
    // pointer leaves — that is the state the old `bool` approach used to lose.
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

/// Drive the exact window pipeline without a window: a platform event must be
/// forwarded, reach the router in canvas coordinates, synthesize a click, and
/// make the next scene carry the incremented label.
///
/// This is the regression test for the bug where the platform layer consumed
/// every input event after only updating its latency clock, leaving the whole
/// showcase inert.
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

    // ---- hover ----------------------------------------------------------
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

    // ---- press ----------------------------------------------------------
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

    // ---- release --------------------------------------------------------
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

    // ---- the next scene labels itself with the new count ----------------
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
        // No Vulkan device: the test cannot run, but it must not fail.
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

fn render_button(
    renderer: &mut unbound_gpu::Renderer,
    atlases: &std::sync::Arc<unbound_gpu::atlas::AtlasSet>,
    harness: &mut ButtonHarness,
    phase: PointerPhase,
) -> Vec<u8> {
    paint_button(renderer, atlases, harness, phase, None)
}

/// Render one button state, uploading the whole scene.
///
/// `patch` chooses the upload path: `None` uploads everything under a fresh
/// generation, `Some((generation, ranges))` keeps the generation and patches
/// the given slices — the path a hover takes.
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
        // A patch keeps the generation the pool is already at and names the
        // slices that changed; the renderer rewrites only those.
        Some((generation, ranges)) => {
            renderer
                .render(&harness.scene.upload(*generation, ranges))
                .expect("render");
        }
        // Every state gets its own generation from the driver, otherwise the
        // primitive buffer is considered unchanged and the previous state is
        // drawn again.
        None => {
            renderer
                .render(&harness.scene.upload(harness.app.generation(), &[]))
                .expect("render");
        }
    }
    renderer.read_pixels().expect("readback")
}

/// Send every glyph the last rebuild rasterised to the GPU atlases.
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

/// A patched upload paints exactly the pixels a full one does.
///
/// The patch path writes slices of the primitive pool and leaves the rest
/// alone, so a wrong offset, a wrong length or a missed flush would show up
/// here as a stale pixel. The scene is the same either way, so the two
/// readbacks must agree byte for byte.
#[test]
fn a_patched_upload_paints_the_same_pixels_as_a_full_one() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // No Vulkan device: the test cannot run, but it must not fail.
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

    // 1. The idle button goes up in full, under generation 7.
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

    // 2. The hovered button is the same scene with a handful of colours
    //    changed, so it is a patch.
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

    // 3. Patch the pool, then upload the same scene in full. Both have to
    //    paint the hovered button.
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
        // No Vulkan device: the test cannot run, but it must not fail.
        return;
    };
    let mut renderer = unbound_gpu::Renderer::new(gpu, 640, 400).expect("renderer");

    // The real atlases, so the labels are not sampled from a dummy 1x1
    // texture.
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

    // Upload the glyphs the labels produced.
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

    // The picture must not be uniformly the background: the swatches, the
    // card and the curves have to be there.
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

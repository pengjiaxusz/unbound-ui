//! The gallery must actually draw something, and the button must actually be
//! interactive.
//!
//! The showcase is a windowed program, so these tests drive the same scene
//! state machine offscreen: the CPU half runs everywhere (including CI without
//! a GPU), the pixel half skips itself when no Vulkan device exists.

use accesskit::Role;
use showcase::button::{self, ButtonDesc, ButtonVisualState, PointerPhase};
use showcase::gallery::{
    build, build_interactive, button_frame, button_rect, palette, theme_table, Gallery, Labels,
};
use unbound_a11y::Semantics;
use unbound_gpu::data::PrimType;
use unbound_headless::Harness;
use unbound_input::{HitRegion, PointerConfig, PointerEvent, Router};
use unbound_math::Vec2;
use unbound_platform::{
    apply, ButtonState, Extent, MouseButton, PlatformEvent, PlatformState, PresentWaitPolicy,
    WindowConfig,
};
use unbound_text::GlyphQuad;
use unbound_world::{NodeId, UiWorld};

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 720.0;

/// A button registered with a router, built without a window.
struct Fixture {
    labels: Labels,
    desc: ButtonDesc,
    id: NodeId,
    router: Router,
}

impl Fixture {
    fn new() -> Self {
        let mut world = UiWorld::new();
        let desc = ButtonDesc::new(button_rect(), "Clicked: 0");
        let id = button::insert_button(&mut world, desc.rect);
        let mut router = Router::new(PointerConfig::default());
        let mut no_glyphs: [GlyphQuad; 0] = [];
        let artifacts = button::emit_button(
            button::node_u64(id),
            &desc,
            ButtonVisualState::default(),
            palette().button_tokens(),
            &mut no_glyphs,
        );
        router.set_regions(vec![artifacts.hit]);
        Self {
            labels: Labels::new(),
            desc,
            id,
            router,
        }
    }

    fn node(&self) -> u64 {
        button::node_u64(self.id)
    }

    fn visual(&self, focused: Option<NodeId>) -> ButtonVisualState {
        ButtonVisualState::resolve(self.id, &self.router, focused, false)
    }

    fn scene(
        &mut self,
        visual: ButtonVisualState,
        click_count: u32,
    ) -> (Gallery, Vec<GlyphQuad>, Vec<HitRegion>, Semantics) {
        let desc = self
            .desc
            .clone()
            .with_label(format!("Clicked: {click_count}"));
        let node = self.node();
        build_interactive(
            &mut self.labels,
            &palette(),
            (WIDTH, HEIGHT),
            &desc,
            node,
            visual,
            click_count,
        )
    }
}

fn has_fill(primitives: &[unbound_gpu::data::GpuPrimitiveInstance], token: u16) -> bool {
    primitives.iter().any(|primitive| {
        primitive.prim_type() == PrimType::RoundedRect
            && primitive.fill_token_or_uv == u32::from(token)
    })
}

#[test]
fn hover_and_press_switch_the_fill_token() {
    let mut fixture = Fixture::new();
    let centre = fixture.desc.rect.center();
    let tokens = palette().button_tokens();

    fixture.router.pointer_move(centre);
    let hovered = fixture.visual(None);
    assert_eq!(hovered.phase, PointerPhase::Hovered);
    let artifacts = button::emit_button(
        fixture.node(),
        &fixture.desc,
        hovered,
        tokens,
        &mut [],
    );
    assert!(
        has_fill(&artifacts.primitives, palette().primary_hover),
        "hover must use the hover slot, not overwrite the shared primary slot"
    );

    fixture.router.pointer_down(centre, 0.0);
    let pressed = fixture.visual(None);
    assert_eq!(pressed.phase, PointerPhase::Pressed);
    let artifacts = button::emit_button(
        fixture.node(),
        &fixture.desc,
        pressed,
        tokens,
        &mut [],
    );
    assert!(has_fill(&artifacts.primitives, palette().primary_pressed));
}

#[test]
fn a_drag_off_the_button_cancels_the_click() {
    let mut fixture = Fixture::new();
    let centre = fixture.desc.rect.center();
    fixture.router.pointer_move(centre);
    fixture.router.pointer_down(centre, 0.0);
    assert_eq!(fixture.visual(None).phase, PointerPhase::Pressed);

    // The router captured the pointer, so the button stays pressed while the
    // pointer leaves — that is the state the old `bool` approach used to lose.
    let outside = Vec2::new(centre.x, centre.y + 200.0);
    fixture.router.pointer_move(outside);
    assert_eq!(fixture.visual(None).phase, PointerPhase::Pressed);

    let events = fixture.router.pointer_up(outside, 0.4);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { .. })),
        "a drag that leaves the button must not synthesise a click"
    );
    assert_eq!(fixture.visual(None).phase, PointerPhase::Idle);
}

#[test]
fn a_release_inside_emits_a_click() {
    let mut fixture = Fixture::new();
    let centre = fixture.desc.rect.center();
    fixture.router.pointer_move(centre);
    fixture.router.pointer_down(centre, 0.0);
    let events = fixture.router.pointer_up(centre, 0.2);
    assert!(
        events.iter().any(
            |event| matches!(event, PointerEvent::Click { target, .. } if *target == fixture.node())
        ),
        "a release inside the button must produce a click for its node"
    );
    assert_eq!(fixture.visual(None).phase, PointerPhase::Hovered);
}

#[test]
fn a_click_changes_the_rendered_label() {
    let mut fixture = Fixture::new();
    let (_, idle, _, _) = fixture.scene(ButtonVisualState::default(), 0);
    let (_, clicked, _, _) = fixture.scene(ButtonVisualState::default(), 1);
    assert_eq!(idle.len(), clicked.len());
    let idle_uv: Vec<[f32; 4]> = idle.iter().map(|quad| quad.uv).collect();
    let clicked_uv: Vec<[f32; 4]> = clicked.iter().map(|quad| quad.uv).collect();
    assert_ne!(
        idle_uv, clicked_uv,
        "the click count must reach the glyph quads, otherwise nothing repaints"
    );
}

#[test]
fn the_focus_ring_is_a_stroke_and_focus_stays_orthogonal() {
    let mut fixture = Fixture::new();
    let centre = fixture.desc.rect.center();
    fixture.router.pointer_move(centre);
    let focused = fixture.visual(Some(fixture.id));
    assert_eq!(focused.phase, PointerPhase::Hovered);
    assert!(focused.focused);

    let artifacts = button::emit_button(
        fixture.node(),
        &fixture.desc,
        focused,
        palette().button_tokens(),
        &mut [],
    );
    assert!(
        artifacts.primitives.iter().any(|primitive| {
            primitive.prim_type() == PrimType::Stroke
                && primitive.border_token_or_tex == u32::from(palette().focus_ring)
        }),
        "a focused button must emit an outer focus ring"
    );
    assert_eq!(artifacts.a11y.role, Role::Button);
}

#[test]
fn the_semantics_tree_exposes_a_button() {
    let mut fixture = Fixture::new();
    let (_, glyphs, regions, mut semantics) = fixture.scene(ButtonVisualState::default(), 3);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].node, fixture.node());

    let node = semantics.get(fixture.node()).expect("button node");
    assert_eq!(node.role, Role::Button);
    assert_eq!(node.label, "Clicked: 3");
    assert_eq!(node.value, "3");

    let update = semantics.take_update().expect("initial tree update");
    assert!(
        update.nodes.iter().any(|(_, node)| node.role() == Role::Button),
        "the update AccessKit receives must carry the button role"
    );

    semantics.set_focus(fixture.node());
    let update = semantics.take_update().expect("focus update");
    assert_eq!(update.focus, accesskit::NodeId(fixture.node()));

    // The label quads are laid out inside the button; the headless frame uses
    // them for the text-clipping check.
    assert!(!glyphs.is_empty());
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

    let mut fixture = Fixture::new();
    let mut state = PlatformState::new(
        &WindowConfig::default(),
        Extent::new(WIDTH as u32, HEIGHT as u32),
        PresentWaitPolicy::from_extensions(&[]),
    );
    let now = Instant::now();
    let centre = fixture.desc.rect.center();

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
    fixture.router.pointer_move(Vec2::new(*x as f32, *y as f32));
    assert_eq!(fixture.visual(None).phase, PointerPhase::Hovered);

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
    fixture.router.pointer_down(centre, 0.0);
    assert_eq!(fixture.visual(None).phase, PointerPhase::Pressed);

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
    let clicks = fixture
        .router
        .pointer_up(centre, 0.1)
        .into_iter()
        .filter(|event| {
            matches!(event, PointerEvent::Click { target, .. } if *target == fixture.node())
        })
        .count();
    assert_eq!(clicks, 1, "one press/release pair must synthesize one click");
    assert_eq!(
        fixture.visual(None).phase,
        PointerPhase::Hovered,
        "the pointer still rests on the button after the click"
    );

    // ---- the next scene labels itself with the new count ----------------
    let (_, glyphs, _, semantics) = fixture.scene(ButtonVisualState::default(), 1);
    assert_eq!(semantics.get(fixture.node()).unwrap().label, "Clicked: 1");
    assert!(!glyphs.is_empty(), "the new label must be laid out");
}

#[test]
fn the_button_passes_the_five_sanitizers() {
    let mut fixture = Fixture::new();
    let (_, glyphs, _, _) = fixture.scene(ButtonVisualState::default(), 0);
    let frame = button_frame((WIDTH, HEIGHT), &fixture.desc, &glyphs);
    let report = Harness::assert_clean(&frame);
    assert!(report.is_clean());
    assert!(report.nodes >= 3, "root, card, button (and label)");
}

#[test]
fn hover_and_press_change_the_rendered_pixels() {
    let Some(gpu) = unbound_gpu::device::test_device() else {
        // No Vulkan device: the test cannot run, but it must not fail.
        return;
    };
    let mut renderer = unbound_gpu::Renderer::new(gpu, WIDTH as u32, HEIGHT as u32).expect("renderer");
    let atlases = std::sync::Arc::new(
        unbound_gpu::atlas::AtlasSet::new(&renderer.gpu, 4).expect("atlas set"),
    );
    renderer.install_atlases(atlases.clone()).expect("install");

    let mut fixture = Fixture::new();
    let idle = render_button(&mut renderer, &atlases, &mut fixture, PointerPhase::Idle);
    let hover = render_button(&mut renderer, &atlases, &mut fixture, PointerPhase::Hovered);
    let pressed = render_button(&mut renderer, &atlases, &mut fixture, PointerPhase::Pressed);

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
    fixture: &mut Fixture,
    phase: PointerPhase,
) -> Vec<u8> {
    let visual = ButtonVisualState {
        phase,
        focused: false,
        disabled: false,
    };
    let desc = fixture.desc.clone();
    let node = fixture.node();
    let (scene, _, _, _) = build_interactive(
        &mut fixture.labels,
        &palette(),
        (WIDTH, HEIGHT),
        &desc,
        node,
        visual,
        0,
    );
    for update in fixture.labels.cache.take_updates() {
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
    renderer.write_theme(&theme_table(false)).expect("theme");
    // Every state needs its own generation, otherwise the primitive buffer is
    // considered unchanged and the previous state is drawn again.
    let generation: u64 = match phase {
        PointerPhase::Idle => 1,
        PointerPhase::Hovered => 2,
        PointerPhase::Pressed => 3,
    };
    renderer.render(&scene.upload(generation)).expect("render");
    renderer.read_pixels().expect("readback")
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

    renderer.write_theme(&theme_table(false)).expect("theme");
    renderer.render(&scene.upload(1)).expect("render");
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

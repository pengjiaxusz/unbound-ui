//! Test harness for the showcase button.
//!
//! [`ButtonHarness`] is the thin widget layer on top of
//! [`unbound_headless::driver::TestApp`]: it owns the button identity, the
//! label shaper, the activation counter and the most recently built scene.
//! Every interaction helper re-derives the visual state from the router and
//! rebuilds only when the discrete `(visual, count)` pair changed — the same
//! guard the window uses, so the tests exercise the real redraw policy.
//!
//! Integration tests in `tests/gallery.rs` use this harness; the GPU pixel
//! test uses [`ButtonHarness::rebuild_with`] to bake a specific state.

use unbound_headless::driver::{ClickReport, TestApp};
use unbound_headless::Report;
use unbound_input::PointerEvent;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_world::NodeId;

use crate::button::{self, ButtonDesc, ButtonVisualState};
use crate::gallery::{self, Labels};

/// The virtual viewport the harness renders into.
pub const VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// A headless button session.
pub struct ButtonHarness {
    /// The shared interaction session.
    pub app: TestApp,
    /// The label shaper and glyph cache.
    pub labels: Labels,
    /// The geometry and base label.
    pub desc: ButtonDesc,
    /// The button identity.
    pub node: NodeId,
    /// How many times the button was activated.
    pub count: u32,
    /// Whether the button is disabled.
    pub disabled: bool,
    /// The glyphs of the most recent scene.
    pub glyphs: Vec<GlyphQuad>,
    /// The most recent scene.
    pub scene: gallery::Gallery,
    /// The visual state currently baked into `scene`.
    baked: ButtonVisualState,
    /// The count currently baked into `scene`.
    baked_count: u32,
}

/// A scene that paints a rounded rectangle over a full-surface rectangle of
/// the **same colour**.
///
/// A correct straight-alpha composite is invisible, no matter how the edge
/// pixels are antialiased. The old double-alpha blend folded the shadow
/// colour into every partially covered pixel, so the edge of the rounded
/// rectangle showed up as a dark rim on an otherwise flat surface — the
/// "same colour, visible seam" bug.
#[must_use]
pub fn same_colour_overlay_scene(viewport: (f32, f32)) -> gallery::Gallery {
    let palette = gallery::palette();
    let mut scene = gallery::Gallery::new();
    scene.camera = unbound_gpu::Camera::ui(viewport, 1.0);
    scene.rect(
        0.0,
        0.0,
        viewport.0,
        viewport.1,
        0.0,
        palette.surface,
        0,
        0.0,
        0.0,
        0,
    );
    scene.rect(
        40.0,
        40.0,
        viewport.0 - 80.0,
        viewport.1 - 80.0,
        28.0,
        palette.surface,
        0,
        0.0,
        0.0,
        1,
    );
    scene
}

impl Default for ButtonHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl ButtonHarness {
    /// Build the default card and bake the idle scene.
    #[must_use]
    pub fn new() -> Self {
        let mut app = TestApp::new(VIEWPORT);
        let desc = ButtonDesc::new(gallery::button_rect(), "Clicked: 0");
        let node = button::insert_button(&mut app.world, desc.rect);
        let mut harness = Self {
            app,
            labels: Labels::new(),
            desc,
            node,
            count: 0,
            disabled: false,
            glyphs: Vec::new(),
            scene: gallery::Gallery::default(),
            baked: ButtonVisualState::default(),
            baked_count: 0,
        };
        harness.rebuild_with(ButtonVisualState::default(), 0);
        harness
    }

    /// The button's semantic id.
    #[must_use]
    pub fn node_key(&self) -> u64 {
        button::node_u64(self.node)
    }

    /// The visual state derived from the router and the focus slot.
    #[must_use]
    pub fn visual(&self) -> ButtonVisualState {
        let focused = if self.app.focused_key() == Some(self.node_key()) {
            Some(self.node)
        } else {
            None
        };
        button::resolve_state(self.node, &self.app.router, focused, self.disabled)
    }

    /// The button centre in window pixels.
    #[must_use]
    pub fn screen_center(&self) -> Vec2 {
        self.app.canvas_to_screen(self.desc.rect.center())
    }

    /// Rebuild the scene when the discrete state changed.
    pub fn sync(&mut self) {
        let visual = self.visual();
        if visual != self.baked || self.count != self.baked_count {
            self.rebuild_with(visual, self.count);
        }
    }

    /// Bake a specific state into a fresh scene.
    ///
    /// This is what the GPU test uses to render idle / hover / pressed without
    /// driving the router; interaction tests should prefer [`sync`](Self::sync).
    pub fn rebuild_with(&mut self, visual: ButtonVisualState, count: u32) -> &gallery::Gallery {
        let mut desc = ButtonDesc::new(self.desc.rect, format!("Clicked: {count}"));
        desc.radius = self.desc.radius;
        desc.font_size = self.desc.font_size;
        desc.z = self.desc.z;
        desc.variant = self.desc.variant;
        let (scene, glyphs, regions, semantics) = gallery::build_interactive(
            &mut self.labels,
            &gallery::palette(),
            VIEWPORT,
            &desc,
            button::node_u64(self.node),
            visual,
            count,
        );
        self.scene = scene;
        self.glyphs = glyphs;
        self.app.install(regions, semantics);
        self.baked = visual;
        self.baked_count = count;
        self.disabled = visual.disabled;
        &self.scene
    }

    /// Move the pointer to the button centre.
    pub fn hover(&mut self) {
        self.hover_at(self.screen_center());
    }

    /// Move the pointer to a window position.
    pub fn hover_at(&mut self, screen: Vec2) {
        self.app.move_to(screen);
        self.sync();
    }

    /// Press the left button at the current pointer position.
    pub fn press(&mut self) {
        self.app.pointer_down();
        // The window focuses a button as soon as the router captures it.
        if self.app.router.pointer().pressed() == Some(button::node_u64(self.node)) {
            self.app.set_focus(Some(self.node));
        }
        self.sync();
    }

    /// Release the left button at the current pointer position.
    pub fn release(&mut self) -> Vec<PointerEvent> {
        let events = self.app.pointer_up();
        if events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { target, .. } if *target == button::node_u64(self.node)))
        {
            self.count = self.count.wrapping_add(1);
        }
        self.sync();
        events
    }

    /// Hover, press and release the button in one step.
    pub fn click(&mut self) -> ClickReport {
        let report = self.app.click(self.screen_center());
        if report.pressed == Some(button::node_u64(self.node)) {
            self.app.set_focus(Some(self.node));
        }
        if report.clicked() == Some(button::node_u64(self.node)) {
            self.count = self.count.wrapping_add(1);
        }
        self.sync();
        report
    }

    /// Activate the button the way `Enter` / `Space` does in the window.
    pub fn activate(&mut self) {
        self.count = self.count.wrapping_add(1);
        self.sync();
    }

    /// Toggle the disabled state and re-bake the scene.
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        self.sync();
    }

    /// How many scenes have been installed so far.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.app.generation()
    }

    /// Focus the button and re-bake the focus ring.
    pub fn focus(&mut self) {
        self.app.set_focus(Some(self.node));
        self.sync();
    }

    /// Clear focus and re-bake the scene.
    pub fn blur(&mut self) {
        self.app.set_focus(None);
        self.sync();
    }

    /// The accessible label currently baked into the semantic tree.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.app.semantics_node(self.node).label
    }

    /// The accessible value currently baked into the semantic tree.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.app.semantics_node(self.node).value
    }

    /// The frame the geometry sanitizers inspect.
    #[must_use]
    pub fn frame(&self) -> unbound_headless::Frame {
        let mut desc = ButtonDesc::new(self.desc.rect, format!("Clicked: {}", self.baked_count));
        desc.radius = self.desc.radius;
        desc.font_size = self.desc.font_size;
        desc.z = self.desc.z;
        desc.variant = self.desc.variant;
        gallery::button_frame(VIEWPORT, &desc, &self.glyphs)
    }

    /// Run the five geometry sanitizers over the button frame.
    pub fn assert_clean(&self) -> Report {
        self.app.assert_clean(&self.frame())
    }

    /// The canvas rectangle the button occupies.
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.desc.rect
    }
}

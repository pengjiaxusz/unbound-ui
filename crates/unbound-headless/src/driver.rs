//! A deterministic, GPU-free driver for UI interaction tests.
//!
//! [`TestApp`] owns the pieces every interactive component shares — the node
//! world, the pointer router, the semantic tree, the camera and the focus
//! slot — and exposes synthetic input in **window pixels**. Tests never touch
//! canvas coordinates: the driver applies the same inverse camera transform
//! the window does, so a zoomed or panned scene is tested exactly as it is
//! seen.
//!
//! The driver deliberately knows nothing about rendering or layout. A widget
//! supplies a thin harness that builds its scene from the derived state (see
//! `apps/showcase/src/testing.rs` for the button), installs the hit regions
//! and semantic tree through [`TestApp::install`], and can then ask for the
//! five geometry sanitizers via [`TestApp::check`].
//!
//! ```ignore
//! let mut app = TestApp::new((1280.0, 720.0));
//! let region = HitRegion::new(7, 1, 0, Rect::new(100.0, 100.0, 200.0, 40.0));
//! app.install(vec![region], Semantics::new(1));
//!
//! let report = app.click(Vec2::new(200.0, 120.0));
//! assert_eq!(report.clicked(), Some(7));
//! ```

use slotmap::Key;
use unbound_a11y::{A11yNode, Semantics};
use unbound_input::{CursorKind, HitRegion, PointerConfig, PointerEvent, Router};
use unbound_math::Vec2;
use unbound_world::{NodeId, UiWorld};

use crate::{Frame, Harness, Report};

/// The semantic root a fresh [`TestApp`] creates.
pub const DEFAULT_ROOT: u64 = 1;

/// What a synthetic click produced.
#[derive(Clone, Debug, Default)]
pub struct ClickReport {
    /// The node the pointer captured on press, if any.
    pub pressed: Option<u64>,
    /// Everything the router emitted, in order.
    pub events: Vec<PointerEvent>,
}

impl ClickReport {
    /// The node that received a `Click`, if the press became one.
    #[must_use]
    pub fn clicked(&self) -> Option<u64> {
        self.events.iter().find_map(|event| match event {
            PointerEvent::Click { target, .. } => Some(*target),
            _ => None,
        })
    }

    /// How many consecutive clicks the router counted (1 for a single click).
    #[must_use]
    pub fn click_count(&self) -> Option<u8> {
        self.events.iter().find_map(|event| match event {
            PointerEvent::Click { count, .. } => Some(*count),
            _ => None,
        })
    }

    /// Whether the pair formed a double click.
    #[must_use]
    pub fn is_double_click(&self) -> bool {
        self.events
            .iter()
            .any(|event| matches!(event, PointerEvent::DoubleClick { .. }))
    }
}

/// The shared state of a headless UI session.
pub struct TestApp {
    /// The node world; widgets allocate their identity here.
    pub world: UiWorld,
    /// Hit regions, hover and pointer capture.
    pub router: Router,
    /// The semantic tree.
    pub semantics: Semantics,
    /// The viewport size, in window pixels.
    pub viewport: Vec2,
    /// The camera zoom.
    pub zoom: f32,
    /// The camera pan, in window pixels.
    pub pan: Vec2,
    /// The focused semantic id, if any.
    focus: Option<u64>,
    /// The last pointer position, in window pixels.
    pointer: Option<Vec2>,
    /// The logical clock the router timestamps events with.
    time: f64,
    /// Bumped on every [`install`](Self::install).
    generation: u64,
}

impl TestApp {
    /// Create a session with the [`DEFAULT_ROOT`].
    #[must_use]
    pub fn new(viewport: (f32, f32)) -> Self {
        Self::with_root(viewport, DEFAULT_ROOT)
    }

    /// Create a session with an explicit semantic root id.
    #[must_use]
    pub fn with_root(viewport: (f32, f32), root: u64) -> Self {
        Self {
            world: UiWorld::new(),
            router: Router::new(PointerConfig::default()),
            semantics: Semantics::new(root),
            viewport: Vec2::new(viewport.0, viewport.1),
            zoom: 1.0,
            pan: Vec2::ZERO,
            focus: None,
            pointer: None,
            time: 0.0,
            generation: 0,
        }
    }

    /// Advance the logical clock, in seconds.
    ///
    /// Use this between clicks to avoid the router pairing them into a
    /// double click, or to keep two synthetic events out of one frame.
    pub fn advance(&mut self, seconds: f64) {
        self.time += seconds;
    }

    /// The logical time, in seconds.
    #[must_use]
    pub fn time(&self) -> f64 {
        self.time
    }

    /// The number of scenes installed so far.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The camera anchor implied by `zoom` and `pan`.
    #[must_use]
    pub fn anchor(&self) -> Vec2 {
        self.viewport * 0.5 - self.pan / self.zoom
    }

    /// Map a window position to canvas coordinates.
    ///
    /// The shader maps `screen = (canvas - anchor) * zoom + viewport / 2`, so
    /// the inverse is `canvas = (screen - viewport / 2) / zoom + anchor`.
    #[must_use]
    pub fn screen_to_canvas(&self, screen: Vec2) -> Vec2 {
        (screen - self.viewport * 0.5) / self.zoom + self.anchor()
    }

    /// Map a canvas position back to window coordinates.
    #[must_use]
    pub fn canvas_to_screen(&self, canvas: Vec2) -> Vec2 {
        (canvas - self.anchor()) * self.zoom + self.viewport * 0.5
    }

    /// Pan the camera by a window-space delta.
    ///
    /// Hover and cursor are re-picked at the new canvas point: the pointer did
    /// not move, but the canvas under it did.
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
        self.refresh_pointer();
    }

    /// Zoom around a window-space anchor, keeping the canvas point under it.
    pub fn zoom_at(&mut self, anchor: Vec2, factor: f32) {
        let before = self.screen_to_canvas(anchor);
        self.zoom = (self.zoom * factor).clamp(0.25, 8.0);
        let anchor_canvas = before - (anchor - self.viewport * 0.5) / self.zoom;
        self.pan = (self.viewport * 0.5 - anchor_canvas) * self.zoom;
        self.refresh_pointer();
    }

    /// Move the pointer to a window position and update hover.
    ///
    /// Returns the hovered node, if any.
    pub fn move_to(&mut self, screen: Vec2) -> Option<u64> {
        self.pointer = Some(screen);
        self.router.pointer_move(self.screen_to_canvas(screen));
        self.router.hover()
    }

    /// Press the left button at the current pointer position.
    ///
    /// Returns the node the router captured, if any.
    pub fn pointer_down(&mut self) -> Option<u64> {
        let position = self.screen_to_canvas(self.pointer_position());
        self.router.pointer_down(position, self.time);
        self.router.pointer().pressed()
    }

    /// Release the left button at the current pointer position.
    pub fn pointer_up(&mut self) -> Vec<PointerEvent> {
        let position = self.screen_to_canvas(self.pointer_position());
        self.router.pointer_up(position, self.time)
    }

    /// Move, press and release in one step.
    pub fn click(&mut self, screen: Vec2) -> ClickReport {
        self.move_to(screen);
        let pressed = self.pointer_down();
        let events = self.pointer_up();
        ClickReport { pressed, events }
    }

    /// Drag from one window position to another with the left button held.
    pub fn drag(&mut self, from: Vec2, to: Vec2) -> ClickReport {
        self.move_to(from);
        let pressed = self.pointer_down();
        self.move_to(to);
        let events = self.pointer_up();
        ClickReport { pressed, events }
    }

    /// Abort the interaction, as a window focus loss would.
    pub fn cancel(&mut self) -> Option<PointerEvent> {
        self.router.cancel()
    }

    /// The node under a window position, without moving the pointer.
    #[must_use]
    pub fn pick(&self, screen: Vec2) -> Option<u64> {
        self.router
            .pick(self.screen_to_canvas(screen))
            .map(|region| region.node)
    }

    /// The cursor the router would request at the current pointer position.
    #[must_use]
    pub fn cursor(&self) -> CursorKind {
        self.router.cursor()
    }

    /// Replace the hit regions and the semantic tree.
    ///
    /// This is the "frame committed" step: the generation is bumped and the
    /// focus is re-applied to the fresh tree.
    ///
    /// A rebuild can remove the node that holds the pointer capture or the
    /// focus. Both are validated against the new scene: a capture on a node
    /// that no longer exists is cancelled (and the `CaptureLost` event is
    /// returned), and a focus on a vanished node falls back to the root.
    /// Without this, a later release would deliver a click for a node that
    /// the scene no longer contains.
    pub fn install(
        &mut self,
        regions: Vec<HitRegion>,
        semantics: Semantics,
    ) -> Option<PointerEvent> {
        self.router.set_regions(regions);
        let capture_lost = if self
            .router
            .pointer()
            .capture()
            .is_some_and(|node| !self.router.regions().iter().any(|r| r.node == node))
        {
            self.router.cancel()
        } else {
            None
        };
        self.semantics = semantics;
        if self
            .focus
            .is_some_and(|key| key != self.semantics.root() && self.semantics.get(key).is_none())
        {
            self.focus = None;
        }
        self.apply_focus();
        self.generation = self.generation.wrapping_add(1);
        capture_lost
    }

    /// Focus a node and mirror it into the semantic tree.
    pub fn set_focus(&mut self, node: Option<NodeId>) {
        self.focus = node.map(node_key);
        self.apply_focus();
    }

    /// Focus a raw semantic id, for tests that drive snapshots.
    pub fn set_focus_key(&mut self, key: u64) {
        self.focus = Some(key);
        self.apply_focus();
    }

    /// The focused semantic id, if any.
    #[must_use]
    pub fn focused_key(&self) -> Option<u64> {
        self.focus
    }

    /// The semantic node behind an identity.
    ///
    /// # Panics
    ///
    /// Panics when the node is not in the current tree.
    #[must_use]
    pub fn semantics_node(&self, node: NodeId) -> &A11yNode {
        let key = node_key(node);
        self.semantics
            .get(key)
            .unwrap_or_else(|| panic!("node {key} is not in the semantic tree"))
    }

    /// Assert a node's accessible label.
    pub fn assert_label(&self, node: NodeId, expected: &str) {
        assert_eq!(
            self.semantics_node(node).label,
            expected,
            "label of {node:?}"
        );
    }

    /// Assert a node's accessible value.
    pub fn assert_value(&self, node: NodeId, expected: &str) {
        assert_eq!(
            self.semantics_node(node).value,
            expected,
            "value of {node:?}"
        );
    }

    /// Assert the focused semantic id.
    pub fn assert_focus(&self, expected: Option<NodeId>) {
        assert_eq!(
            self.focused_key(),
            expected.map(node_key),
            "focus must match"
        );
        assert_eq!(
            self.semantics.focus(),
            self.focused_key().unwrap_or_else(|| self.semantics.root())
        );
    }

    /// Run the five geometry sanitizers over a frame.
    #[must_use]
    pub fn check(&self, frame: &Frame) -> Report {
        Harness::run(frame)
    }

    /// Run the sanitizers and panic on any violation.
    pub fn assert_clean(&self, frame: &Frame) -> Report {
        Harness::assert_clean(frame)
    }

    /// The current pointer position, in window pixels.
    fn pointer_position(&self) -> Vec2 {
        self.pointer
            .expect("move the pointer before pressing or releasing")
    }

    /// Re-pick hover and cursor after the camera moved under the pointer.
    fn refresh_pointer(&mut self) {
        if let Some(screen) = self.pointer {
            self.router.refresh_at(self.screen_to_canvas(screen));
        }
    }

    /// Mirror `focus` into the semantic tree (the root means "no focus").
    fn apply_focus(&mut self) {
        let target = self.focus.unwrap_or_else(|| self.semantics.root());
        self.semantics.set_focus(target);
    }
}

/// The semantic id of a node.
#[must_use]
pub fn node_key(node: NodeId) -> u64 {
    node.data().as_ffi()
}

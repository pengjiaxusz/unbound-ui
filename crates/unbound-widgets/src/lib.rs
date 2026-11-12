//! Standard controls: the vocabulary every widget speaks.
//!
//! The crate sits between the engine's capability crates and the application
//! layer. It owns the *control* semantics — a pure descriptor, a visual state
//! derived from the input router, and a scene emission — but no windowing, no
//! layout solving and no application state.
//!
//! Three rules hold for every control in this crate:
//!
//! 1. **No interaction booleans.** A widget never stores "am I hovered".
//!    [`ControlState::resolve`] derives the phase from the
//!    [`unbound_input::Router`] on every frame, so a pointer that leaves the
//!    widget while captured still reads as *pressed*.
//! 2. **No closures, no type erasure.** A control is a plain data struct plus
//!    a pure function that turns `(descriptor, state)` into primitives, a hit
//!    region and a semantic node.
//! 3. **Colours are tokens, never literals.** A widget names
//!    [`unbound_theme::Token`] slots; the theme decides what they resolve to.
//!    A theme switch therefore never re-uploads a single primitive.
//!
//! [`app-guest`](../../app_guest) still depends on [`unbound_abi`] alone, so a
//! guest application may host these controls without pulling in the engine.

pub mod button;

use rustc_hash::FxHashMap;
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform};
use unbound_headless::driver::{ClickReport, TestApp};
use unbound_input::{HitRegion, PointerEvent, Router};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;

pub use button::{ButtonDesc, ButtonTokens, ButtonVariant, ButtonWidget};

/// The default viewport a headless widget session renders into.
pub const DEFAULT_VIEWPORT: (f32, f32) = (1280.0, 720.0);

/// Where the pointer is relative to a widget.
///
/// This is the *only* pointer dimension a control needs. Focus is a separate,
/// orthogonal dimension ([`ControlState::focused`]): a button reached with the
/// keyboard must show its focus ring even while the mouse hovers elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerPhase {
    /// The pointer is not over the widget.
    #[default]
    Idle,
    /// The pointer is over the widget.
    Hovered,
    /// The pointer is over the widget and the button is down.
    Pressed,
}

/// The visual state of a control in one frame.
///
/// Every field is derived, never stored: [`ControlState::resolve`] is the only
/// constructor the engine uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlState {
    /// Where the pointer is relative to the widget.
    pub phase: PointerPhase,
    /// Whether the widget holds keyboard focus.
    pub focused: bool,
    /// Whether the widget refuses interaction.
    pub disabled: bool,
    /// Whether the widget is toggled on.
    pub selected: bool,
}

impl ControlState {
    /// Derive the state from the pointer router and the focus slot.
    ///
    /// A disabled widget never shows a pointer phase: it is not a target, so
    /// the pointer cannot be over it.
    #[must_use]
    pub fn resolve(
        node_key: u64,
        router: &Router,
        focused_key: Option<u64>,
        disabled: bool,
        selected: bool,
    ) -> Self {
        let phase = if disabled {
            PointerPhase::Idle
        } else if router.pointer().pressed() == Some(node_key) {
            PointerPhase::Pressed
        } else if router.hover() == Some(node_key) {
            PointerPhase::Hovered
        } else {
            PointerPhase::Idle
        };
        Self {
            phase,
            focused: focused_key == Some(node_key),
            disabled,
            selected,
        }
    }
}

/// What one control emits in one frame.
///
/// The three parts are deliberately independent: a purely decorative widget
/// emits no hit region, a widget that is not on screen emits no primitives,
/// and a widget that is not reachable by assistive technology emits no
/// semantic node.
#[derive(Clone, Debug, Default)]
pub struct WidgetOutput {
    /// The primitives, in draw order.
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// The payloads the primitives refer to.
    pub payloads: Vec<GpuPayload>,
    /// The pickable surface, if the widget accepts input.
    pub hit: Option<HitRegion>,
    /// The semantic node, if the widget is reachable.
    pub a11y: Option<A11yNode>,
    /// Whether the widget takes part in the keyboard tab order.
    pub focusable: bool,
}

/// Collects the output of many widgets into one frame.
///
/// The collector also records, for every widget, the half-open range of
/// primitives it owns. That map is what makes a *local* repaint possible: a
/// hover that changes five primitives can rewrite
/// `primitives[start..end]` and upload just those bytes instead of the whole
/// scene.
#[derive(Default)]
pub struct SceneCollector {
    /// The primitives, in draw order.
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// The transform table.
    pub transforms: Vec<GpuTransform>,
    /// The clip nodes.
    pub clips: Vec<GpuClipNode>,
    /// The payloads the primitives refer to.
    pub payloads: Vec<GpuPayload>,
    /// The pickable surfaces.
    pub regions: Vec<HitRegion>,
    /// The semantic tree.
    pub semantics: Semantics,
    /// The keyboard tab order.
    pub focus_order: Vec<u64>,
    /// The primitive range each widget owns, keyed by widget identity.
    pub node_spans: FxHashMap<u64, (u32, u32)>,
}

impl SceneCollector {
    /// An empty collector whose semantic tree is rooted at `root`.
    #[must_use]
    pub fn new(root: u64) -> Self {
        Self {
            semantics: Semantics::new(root),
            ..Self::default()
        }
    }

    /// Append one widget's output and record the primitives it owns.
    pub fn push(&mut self, key: u64, output: WidgetOutput) {
        let start = u32::try_from(self.primitives.len()).unwrap_or(u32::MAX);
        self.primitives.extend(output.primitives);
        let end = u32::try_from(self.primitives.len()).unwrap_or(u32::MAX);
        self.node_spans.insert(key, (start, end));
        self.payloads.extend(output.payloads);
        if let Some(hit) = output.hit {
            self.regions.push(hit);
        }
        if let Some(node) = output.a11y {
            self.semantics.upsert(key, node);
        }
        if output.focusable {
            self.focus_order.push(key);
        }
    }

    /// The primitives one widget owns.
    #[must_use]
    pub fn span(&self, key: u64) -> Option<&[GpuPrimitiveInstance]> {
        let (start, end) = *self.node_spans.get(&key)?;
        let start = usize::try_from(start).ok()?;
        let end = usize::try_from(end).ok()?;
        self.primitives.get(start..end)
    }
}

/// A widget descriptor: the pure data that determines a widget's scene.
///
/// The trait exists so that one harness ([`WidgetHarness`]) can drive any
/// control: the descriptor answers "where am I" and "what do I look like in
/// this state", and nothing else.
pub trait WidgetDesc {
    /// The interactive rectangle of the widget.
    fn rect(&self) -> Rect;

    /// The stable identity of the widget instance.
    ///
    /// The identity must survive a rebuild: it is what the router captures,
    /// what the focus slot points at and what the semantic tree is keyed by.
    fn key(&self) -> u64;

    /// Whether the widget refuses interaction.
    fn disabled(&self) -> bool {
        false
    }

    /// Whether the widget is toggled on.
    fn selected(&self) -> bool {
        false
    }

    /// Emit the widget's scene output for a state.
    ///
    /// `glyphs` carries the shaped text the widget draws; the emitter may
    /// position it but never re-shapes it.
    fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput;
}

/// A headless session around one widget.
///
/// The harness owns the interaction session, re-derives the visual state from
/// the router after every input event and re-emits the widget only when that
/// state actually changed — the same redraw policy the window applies.
pub struct WidgetHarness<D: WidgetDesc> {
    /// The shared interaction session.
    pub app: TestApp,
    /// The descriptor under test.
    pub desc: D,
    /// The shaped glyphs the widget draws.
    pub glyphs: Vec<GlyphQuad>,
    /// The most recent emission.
    pub output: WidgetOutput,
    /// The state currently baked into `output`.
    baked: ControlState,
}

impl<D: WidgetDesc> WidgetHarness<D> {
    /// A session for the default viewport.
    #[must_use]
    pub fn new(desc: D, glyphs: Vec<GlyphQuad>) -> Self {
        Self::with_viewport(desc, glyphs, DEFAULT_VIEWPORT)
    }

    /// A session for an explicit viewport.
    #[must_use]
    pub fn with_viewport(desc: D, glyphs: Vec<GlyphQuad>, viewport: (f32, f32)) -> Self {
        let mut harness = Self {
            app: TestApp::new(viewport),
            desc,
            glyphs,
            output: WidgetOutput::default(),
            baked: ControlState::default(),
        };
        harness.rebuild();
        harness
    }

    /// The widget's identity.
    #[must_use]
    pub fn node_key(&self) -> u64 {
        self.desc.key()
    }

    /// The state derived from the router and the focus slot.
    #[must_use]
    pub fn state(&self) -> ControlState {
        ControlState::resolve(
            self.desc.key(),
            &self.app.router,
            self.app.focused_key(),
            self.desc.disabled(),
            self.desc.selected(),
        )
    }

    /// Emit the widget and install the result into the session.
    pub fn rebuild(&mut self) {
        let state = self.state();
        self.output = self.desc.emit(state, &mut self.glyphs);
        self.install();
        self.baked = state;
    }

    /// Re-emit the widget when the visual state changed.
    ///
    /// Returns whether the scene was rebuilt.
    pub fn sync(&mut self) -> bool {
        let state = self.state();
        if state == self.baked {
            return false;
        }
        self.rebuild();
        true
    }

    fn install(&mut self) {
        let key = self.desc.key();
        let mut semantics = Semantics::new(key);
        if let Some(node) = self.output.a11y.clone() {
            semantics.upsert(key, node);
        }
        let regions = self.output.hit.clone().into_iter().collect();
        self.app.install(regions, semantics);
    }

    /// The widget centre in window pixels.
    #[must_use]
    pub fn screen_center(&self) -> Vec2 {
        self.app.canvas_to_screen(self.desc.rect().center())
    }

    /// Move the pointer onto the widget.
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
        self.sync();
    }

    /// Release the left button at the current pointer position.
    pub fn release(&mut self) -> Vec<PointerEvent> {
        let events = self.app.pointer_up();
        self.sync();
        events
    }

    /// Hover, press and release the widget in one step.
    pub fn click(&mut self) -> ClickReport {
        let report = self.app.click(self.screen_center());
        self.sync();
        report
    }

    /// Give the widget keyboard focus.
    pub fn focus(&mut self) {
        self.app.set_focus_key(self.desc.key());
        self.sync();
    }

    /// Take keyboard focus away from the widget.
    pub fn blur(&mut self) {
        self.app.clear_focus();
        self.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_input::PointerConfig;

    /// A router with one square hit region owned by `node`.
    fn router_with(node: u64) -> Router {
        let mut router = Router::new(PointerConfig::default());
        router.set_regions(vec![HitRegion::new(
            node,
            1,
            0,
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )]);
        router
    }

    #[test]
    fn the_pointer_phase_follows_the_router() {
        let mut router = router_with(7);
        router.pointer_move(Vec2::new(10.0, 10.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);

        router.pointer_move(Vec2::new(500.0, 500.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Idle);
    }

    #[test]
    fn a_captured_pointer_stays_pressed_after_it_leaves() {
        let mut router = router_with(7);
        router.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        // The pointer is captured, so moving away must not end the press.
        router.pointer_move(Vec2::new(500.0, 500.0));
        let state = ControlState::resolve(7, &router, None, false, false);
        assert_eq!(state.phase, PointerPhase::Pressed);
    }

    #[test]
    fn a_disabled_control_never_shows_a_pointer_phase() {
        let mut router = router_with(7);
        router.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        let state = ControlState::resolve(7, &router, None, true, false);
        assert_eq!(state.phase, PointerPhase::Idle);
        assert!(state.disabled);
    }

    #[test]
    fn the_focus_slot_is_orthogonal_to_the_pointer_phase() {
        let mut router = router_with(7);
        router.pointer_move(Vec2::new(10.0, 10.0));
        let state = ControlState::resolve(7, &router, Some(7), false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);
        assert!(state.focused);

        // Focus on another widget leaves the phase alone.
        let state = ControlState::resolve(7, &router, Some(9), false, false);
        assert_eq!(state.phase, PointerPhase::Hovered);
        assert!(!state.focused);
    }

    #[test]
    fn the_collector_records_a_span_per_widget() {
        let mut collector = SceneCollector::new(1);
        let first = WidgetOutput {
            primitives: vec![GpuPrimitiveInstance::default(); 3],
            ..WidgetOutput::default()
        };
        let second = WidgetOutput {
            primitives: vec![GpuPrimitiveInstance::default(); 2],
            ..WidgetOutput::default()
        };
        collector.push(10, first);
        collector.push(20, second);

        assert_eq!(collector.node_spans.get(&10), Some(&(0, 3)));
        assert_eq!(collector.node_spans.get(&20), Some(&(3, 5)));
        assert_eq!(
            collector.span(20).map(<[GpuPrimitiveInstance]>::len),
            Some(2)
        );
        assert_eq!(collector.span(30), None);
    }

    #[test]
    fn a_focusable_widget_joins_the_tab_order() {
        let mut collector = SceneCollector::new(1);
        let output = WidgetOutput {
            focusable: true,
            ..WidgetOutput::default()
        };
        collector.push(10, output);
        assert_eq!(collector.focus_order, vec![10]);
    }
}

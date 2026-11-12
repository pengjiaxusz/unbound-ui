//! The application runtime: one event loop shape for every app.
//!
//! The runtime owns the pieces that used to be copied by hand into each
//! application shell:
//!
//! * the [`Application`] contract — `init`, `reduce`, `view`,
//! * a [`SceneCollector`] rebuilt once per frame and diffed against the
//!   previous frame, so the renderer receives an upload plan rather than a
//!   whole scene,
//! * pointer routing through the real [`Router`], including hover, press,
//!   click, double-click and drag translation,
//! * the keyboard focus policy through [`FocusManager`],
//! * dirty flags that let a frame with no changes skip the rebuild entirely.
//!
//! The runtime is intentionally headless: it produces primitives, hit regions
//! and semantics, never a window or a swapchain. A windowed shell feeds it
//! window events and uploads [`FrameReport`]s; a test feeds it synthetic
//! events and asserts on the same reports. The showcase shell and
//! `runtime_headless.rs` therefore exercise exactly the same code path.

use unbound_a11y::A11yAction;
use unbound_abi::{Dim, NodeFlags, RawNodeDesc};
use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform};
use unbound_input::{
    CursorKind, FocusCause, FocusKey, FocusManager, PointerButton, PointerEvent, Router,
};
use unbound_layout::LayoutEngine;
use unbound_math::{Rect, Vec2};
use unbound_text::{TextEngine, TextStyle};
use unbound_theme::ThemeKind;
use unbound_widgets::button::{ButtonDesc, ButtonWidget};
use unbound_widgets::diff::{diff_primitives, SceneDiff};
use unbound_widgets::{BackdropRequest, ControlState, SceneCollector, WidgetDesc, WidgetOutput};
use unbound_world::{NodeId, UiWorld};

/// The semantic root identity.
pub const ROOT: u64 = 1;

/// What the application says a reduction invalidated.
///
/// The runtime uses the flags to skip rebuilds and to tell a window whether it
/// needs a new accessibility snapshot; the renderer itself only ever sees a
/// [`SceneDiff`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DirtyFlags {
    bits: u8,
}

impl DirtyFlags {
    /// Nothing changed.
    pub const NONE: Self = Self { bits: 0 };
    /// Only pixel contents changed.
    pub const PAINT: Self = Self { bits: 1 };
    /// Geometry changed; a rebuild is mandatory.
    pub const LAYOUT: Self = Self { bits: 1 << 1 };
    /// The accessibility tree changed.
    pub const A11Y: Self = Self { bits: 1 << 2 };
    /// The text / IME layout changed.
    pub const TEXT: Self = Self { bits: 1 << 3 };
    /// Everything, including a new scene root.
    pub const ALL: Self = Self { bits: 0b1111 };

    /// The raw bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }

    /// Build from raw bits, dropping unknown bits.
    #[must_use]
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self {
            bits: bits & Self::ALL.bits,
        }
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    /// Combine two results.
    #[must_use]
    pub const fn merge(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// Whether a rebuild is needed.
    #[must_use]
    pub const fn needs_rebuild(self) -> bool {
        self.contains(Self::LAYOUT) || self.contains(Self::ALL)
    }

    /// Whether the accessibility tree must be re-published.
    #[must_use]
    pub const fn needs_a11y(self) -> bool {
        self.contains(Self::A11Y) || self.needs_rebuild()
    }
}

impl core::ops::BitOr for DirtyFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.merge(rhs)
    }
}

impl core::ops::BitOrAssign for DirtyFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.merge(rhs);
    }
}

/// A keyboard key in the application's vocabulary.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AppKey {
    Tab,
    Escape,
    Enter,
    Space,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Any other key, identified by its character.
    Character(char),
}

/// Held modifier keys.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// An IME composition update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeUpdate {
    /// The composing text, empty when composition ended.
    pub preedit: String,
    /// A committed string, when the composition finished this frame.
    pub commit: Option<String>,
}

/// A **resolved** application event.
///
/// Pointer targets have already been picked from the previous frame's hit
/// regions; the application never has to know where a widget is.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// The pointer moved; `hovered` is the node under it.
    PointerMove { pos: Vec2, hovered: Option<u64> },
    /// A button went down over `target`.
    PointerDown {
        pos: Vec2,
        target: Option<u64>,
        button: PointerButton,
        click_count: u8,
    },
    /// A button was released over `target`.
    PointerUp {
        pos: Vec2,
        target: Option<u64>,
        button: PointerButton,
    },
    /// A completed click.
    Click { node: u64, pos: Vec2, count: u8 },
    /// A completed double click.
    DoubleClick { node: u64, pos: Vec2 },
    /// A drag started on `node`.
    DragStart { node: u64, pos: Vec2 },
    /// A drag moved `delta`.
    Drag { node: u64, pos: Vec2, delta: Vec2 },
    /// The wheel moved over `target`.
    Wheel {
        pos: Vec2,
        delta: Vec2,
        target: Option<u64>,
    },
    /// A key was pressed or released.
    Key {
        key: AppKey,
        pressed: bool,
        modifiers: Modifiers,
    },
    /// Text was typed (already IME-committed).
    Text { text: String },
    /// The IME composition changed.
    Ime { update: ImeUpdate },
    /// The window changed size.
    Resize { size: Vec2 },
    /// The theme changed.
    Theme { kind: ThemeKind },
    /// An assistive technology requested an action.
    A11y {
        target: u64,
        action: A11yAction,
        value: Option<String>,
    },
    /// A frame clock tick.
    Tick { dt: f32 },
}

/// The runtime's read/write view of the world during `reduce`.
#[derive(Clone, Debug)]
pub struct AppContext {
    /// The current viewport size.
    pub viewport: Vec2,
    /// The active theme.
    pub theme: ThemeKind,
    /// The focused node, kept in sync with the focus manager.
    pub focused: Option<u64>,
    /// Whether the focus ring should be painted.
    pub focus_visible: bool,
    /// The node under the pointer.
    pub hovered: Option<u64>,
    /// The node currently pressed.
    pub pressed: Option<u64>,
    /// The requested cursor.
    pub cursor: CursorKind,
    /// The last pointer position.
    pub pointer: Vec2,
    /// Seconds since the runtime started.
    pub time: f64,
    /// Ask the runtime to focus a node after this reduction.
    pub request_focus: Option<(u64, FocusCause)>,
    /// Ask the runtime to blur after this reduction.
    pub request_blur: bool,
}

impl AppContext {
    fn new(viewport: Vec2) -> Self {
        Self {
            viewport,
            theme: ThemeKind::Dark,
            focused: None,
            focus_visible: false,
            hovered: None,
            pressed: None,
            cursor: CursorKind::Pointer,
            pointer: Vec2::ZERO,
            time: 0.0,
            request_focus: None,
            request_blur: false,
        }
    }
}

/// The immutable context available while `view` builds the scene.
#[derive(Copy, Clone, Debug)]
pub struct ViewContext {
    /// The viewport size.
    pub viewport: Vec2,
    /// The active theme.
    pub theme: ThemeKind,
    /// The node under the pointer.
    pub hovered: Option<u64>,
    /// The node currently pressed.
    pub pressed: Option<u64>,
    /// The focused node.
    pub focused: Option<u64>,
    /// Whether the focus ring should be painted.
    pub focus_visible: bool,
    /// Whether the host asked for a full rebuild.
    ///
    /// Set by [`AppRuntime::invalidate`]: an application that normally skips a
    /// rebuild when nothing changed must honour this and rebuild anyway.
    pub force: bool,
}

impl ViewContext {
    /// The control state for a key, resolved from hover / press / focus.
    #[must_use]
    pub fn control_state(&self, key: u64, disabled: bool, selected: bool) -> ControlState {
        let phase = if disabled {
            unbound_widgets::PointerPhase::Idle
        } else if self.pressed == Some(key) {
            unbound_widgets::PointerPhase::Pressed
        } else if self.hovered == Some(key) {
            unbound_widgets::PointerPhase::Hovered
        } else {
            unbound_widgets::PointerPhase::Idle
        };
        ControlState {
            phase,
            focused: self.focused == Some(key) && self.focus_visible,
            disabled,
            selected,
        }
    }
}

/// A widget the declarative view placed in the layout tree.
///
/// The widget is emitted in the second pass, once the layout engine has
/// resolved its rectangle; the widget itself never guesses its geometry.
#[derive(Clone, Debug)]
pub enum WidgetSpec {
    /// A flat button. Its descriptor rect is replaced by the laid-out rect.
    Button(ButtonDesc),
    /// A wrapping paragraph measured by the text engine.
    Text {
        /// The paragraph contents.
        text: String,
        /// The shaping style.
        style: TextStyle,
    },
}

/// One deferred widget together with the node the layout engine solved.
#[derive(Clone, Debug)]
pub struct PendingWidget {
    /// The widget identity used by input and accessibility.
    pub key: u64,
    /// The layout node that owns the widget's geometry.
    pub node: NodeId,
    /// What to emit in the second pass.
    pub spec: WidgetSpec,
}

/// The scene builder handed to [`Application::view`].
pub struct ViewBuilder<'a> {
    collector: &'a mut SceneCollector,
    /// The immutable frame context.
    pub ctx: ViewContext,
    world: &'a mut UiWorld,
    parent: NodeId,
    pending: &'a mut Vec<PendingWidget>,
}

impl<'a> ViewBuilder<'a> {
    /// Append a widget's output.
    pub fn push(&mut self, key: u64, output: WidgetOutput) {
        self.collector.push(key, output);
    }

    /// Append a widget's output with a fixed primitive reservation, so a
    /// dynamic glyph run does not shift every following widget's span.
    pub fn push_with_capacity(&mut self, key: u64, output: WidgetOutput, reserved_slots: usize) {
        self.collector
            .push_with_capacity(key, output, reserved_slots);
    }

    /// Declare a vertical stack. Children are laid out in order.
    pub fn column(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        self.container(key, RawNodeDesc::container().column().gap(8.0), build)
    }

    /// Declare a horizontal stack.
    pub fn row(&mut self, key: u64, build: impl FnOnce(&mut Self)) -> NodeId {
        self.container(key, RawNodeDesc::container().row().gap(8.0), build)
    }

    /// Declare a grid with up to eight explicit column tracks.
    pub fn grid(&mut self, key: u64, columns: &[Dim], build: impl FnOnce(&mut Self)) -> NodeId {
        let desc = RawNodeDesc::container()
            .display(unbound_abi::Display::Grid)
            .grid_columns(columns)
            .gap(8.0);
        self.container(key, desc, build)
    }

    /// Declare a layout-only container with an explicit descriptor.
    pub fn container(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        desc.key = key;
        let node = self
            .world
            .insert(self.parent, desc, core::panic::Location::caller());
        let parent = core::mem::replace(&mut self.parent, node);
        build(self);
        self.parent = parent;
        node
    }

    /// Place a widget in the current container; emit happens after layout.
    pub fn widget(&mut self, key: u64, mut desc: RawNodeDesc, spec: WidgetSpec) -> NodeId {
        desc.key = key;
        if let WidgetSpec::Button(button) = &spec {
            // Auto-sized buttons get the descriptor's rect when it is already
            // resolved, otherwise a conservative intrinsic from the label.
            let intrinsic = Vec2::new(
                (button.label.chars().count() as f32 * 8.0 + 32.0).max(button.rect.width()),
                button.rect.height().max(32.0),
            );
            desc = desc.size(intrinsic.x, intrinsic.y);
        }
        let node = self
            .world
            .insert(self.parent, desc, core::panic::Location::caller());
        if let WidgetSpec::Button(button) = &spec {
            let size = Vec2::new(
                (button.label.chars().count() as f32 * 8.0 + 32.0).max(button.rect.width()),
                button.rect.height().max(32.0),
            );
            if let Some(data) = self.world.get_mut(node) {
                data.measure = Some(size);
            }
        }
        self.pending.push(PendingWidget { key, node, spec });
        node
    }

    /// Place a wrapping paragraph. It spans the container width and its
    /// measured height comes from [`TextEngine::wrap`].
    pub fn text(&mut self, key: u64, text: impl Into<String>, style: TextStyle) -> NodeId {
        let text = text.into();
        let desc = RawNodeDesc::container()
            .key(key)
            .width(Dim::percent(100.0))
            .flags(NodeFlags::WRAP);
        let node = self
            .world
            .insert(self.parent, desc, core::panic::Location::caller());
        self.world.set_text(node, text.clone());
        self.pending.push(PendingWidget {
            key,
            node,
            spec: WidgetSpec::Text { text, style },
        });
        node
    }

    /// Direct access for widgets that need to append transforms or clips.
    pub fn collector(&mut self) -> &mut SceneCollector {
        self.collector
    }

    /// Ask the renderer for a backdrop-blur pass behind `rect`.
    ///
    /// Overlay emitters return a `BackdropRequest`; forwarding it here puts it
    /// on the frame report, where a renderer can activate the frosted-glass
    /// chain. An empty list means the graph skips the blur passes entirely.
    pub fn request_backdrop(&mut self, request: BackdropRequest) {
        self.collector.backdrops.push(request);
    }

    /// The viewport as a rectangle at the origin.
    #[must_use]
    pub fn viewport_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.ctx.viewport.x, self.ctx.viewport.y)
    }
}

/// The application contract.
///
/// The trait is deliberately tiny: state in, events in, primitives out. A
/// windowed shell, a headless test and the showcase all drive the same three
/// methods.
pub trait Application {
    /// The application state.
    type State;

    /// Build the initial state.
    fn init(&mut self, ctx: &mut AppContext) -> Self::State;

    /// Reduce one resolved event.
    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> DirtyFlags;

    /// Emit the scene for the current state.
    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>);
}

/// The result of rebuilding one frame.
#[derive(Clone, Debug)]
pub struct FrameReport {
    /// What the renderer must upload.
    pub diff: SceneDiff,
    /// The dirty flags accumulated since the previous frame.
    pub dirty: DirtyFlags,
    /// The primitive count in the new frame.
    pub primitives: usize,
    /// The bytes the diff would upload.
    pub bytes: u32,
    /// Whether one of the lightweight auxiliary tables (payloads, transforms,
    /// clips) changed and must be re-uploaded without re-sending the
    /// primitive pool.
    pub aux_changed: bool,
    /// Backdrop-blur requests for this frame.
    pub backdrops: Vec<BackdropRequest>,
}

/// The event-reducing, scene-diffing application runtime.
pub struct AppRuntime<A: Application> {
    app: A,
    state: A::State,
    ctx: AppContext,
    router: Router,
    focus: FocusManager,
    collector: SceneCollector,
    previous_primitives: Vec<GpuPrimitiveInstance>,
    previous_payloads: Vec<GpuPayload>,
    previous_transforms: Vec<GpuTransform>,
    previous_clips: Vec<GpuClipNode>,
    diff: SceneDiff,
    dirty: DirtyFlags,
    backdrops: Vec<BackdropRequest>,
    time: f64,
    force_rebuild: bool,
    world: UiWorld,
    layout: LayoutEngine,
    text_engine: TextEngine,
    pending: Vec<PendingWidget>,
}

impl<A: Application> AppRuntime<A> {
    /// Build a runtime, initialising the application state.
    pub fn new(mut app: A, viewport: Vec2) -> Self {
        let mut ctx = AppContext::new(viewport);
        let state = app.init(&mut ctx);
        Self {
            app,
            state,
            ctx,
            router: Router::default(),
            focus: FocusManager::default(),
            collector: SceneCollector::new(ROOT),
            previous_primitives: Vec::new(),
            previous_payloads: Vec::new(),
            previous_transforms: Vec::new(),
            previous_clips: Vec::new(),
            diff: SceneDiff::Whole,
            dirty: DirtyFlags::ALL,
            backdrops: Vec::new(),
            time: 0.0,
            force_rebuild: true,
            world: UiWorld::new(),
            layout: LayoutEngine::new(),
            text_engine: TextEngine::new(),
            pending: Vec::new(),
        }
    }

    /// The application state.
    #[must_use]
    pub fn state(&self) -> &A::State {
        &self.state
    }

    /// The mutable application state, for a shell that needs to poke it.
    pub fn state_mut(&mut self) -> &mut A::State {
        &mut self.state
    }

    /// The application itself.
    ///
    /// A windowed shell needs it for the parts the runtime deliberately does
    /// not own: the glyph cache, the camera, and the accessibility tree the
    /// platform adapter publishes.
    #[must_use]
    pub fn application(&self) -> &A {
        &self.app
    }

    /// The application itself, mutably.
    pub fn application_mut(&mut self) -> &mut A {
        &mut self.app
    }

    /// Advance the clock and reduce one tick.
    ///
    /// A shell with a frame clock calls this instead of
    /// [`Self::frame`]'s `dt`, so a time-driven application reports the frame
    /// it actually needs instead of being rebuilt on every vsync.
    pub fn tick(&mut self, dt: f32) -> DirtyFlags {
        self.advance(dt);
        self.dispatch(AppEvent::Tick { dt })
    }

    /// The current application context.
    #[must_use]
    pub fn context(&self) -> &AppContext {
        &self.ctx
    }

    /// The scene of the most recent frame.
    #[must_use]
    pub fn scene(&self) -> &SceneCollector {
        &self.collector
    }

    /// The scene of the most recent frame, mutably.
    ///
    /// A window host uses this to drain the semantic delta into the platform
    /// accessibility adapter.
    pub fn scene_mut(&mut self) -> &mut SceneCollector {
        &mut self.collector
    }

    /// The diff of the most recent frame.
    #[must_use]
    pub fn last_diff(&self) -> &SceneDiff {
        &self.diff
    }

    /// The dirty flags accumulated since the last frame.
    #[must_use]
    pub fn dirty(&self) -> DirtyFlags {
        self.dirty
    }

    /// The focus manager, for a shell that wires platform focus events.
    #[must_use]
    pub fn focus(&self) -> &FocusManager {
        &self.focus
    }

    /// Focus a node (or drop focus) without going through an event.
    ///
    /// A host uses this for window-activation changes; a test uses it to
    /// reproduce the focus the runtime's own pointer policy would resolve.
    pub fn focus_node(&mut self, node: Option<u64>) {
        let changed = match node {
            Some(node) => self.focus.focus(node, FocusCause::Programmatic),
            None => self.focus.blur(),
        };
        if changed {
            self.sync_focus_context();
            self.mark(DirtyFlags::LAYOUT);
        }
    }

    /// Mark the next frame as a full rebuild.
    pub fn invalidate(&mut self) {
        self.dirty = self.dirty.merge(DirtyFlags::ALL);
        self.force_rebuild = true;
    }

    /// Move the pointer, producing hover / move events.
    pub fn pointer_move(&mut self, pos: Vec2) -> DirtyFlags {
        self.ctx.pointer = pos;
        let mut flags = DirtyFlags::PAINT;
        if let Some(event) = self.router.pointer_move(pos) {
            flags = flags.merge(self.reduce_pointer(event));
        }
        self.sync_router_context();
        self.mark(flags);
        flags
    }

    /// Press the primary button.
    pub fn pointer_down(&mut self, pos: Vec2) -> DirtyFlags {
        let event = self.router.pointer_down(pos, self.time);
        self.ctx.pointer = pos;
        let flags = event.map_or(DirtyFlags::NONE, |event| self.reduce_pointer(event));
        self.sync_router_context();
        self.mark(flags.merge(DirtyFlags::PAINT));
        flags
    }

    /// Release the primary button.
    pub fn pointer_up(&mut self, pos: Vec2) -> DirtyFlags {
        let events = self.router.pointer_up(pos, self.time);
        self.ctx.pointer = pos;
        let mut flags = DirtyFlags::PAINT;
        for event in events {
            flags = flags.merge(self.reduce_pointer(event));
        }
        self.sync_router_context();
        self.mark(flags);
        flags
    }

    /// Cancel the pointer capture without synthesising a click.
    ///
    /// A host that loses focus mid-press has to drop the capture: the release
    /// will never arrive, and the next `pointer_up` must not turn the
    /// abandoned press into an activation.
    pub fn cancel_pointer(&mut self) -> DirtyFlags {
        let _ = self.router.cancel();
        self.sync_router_context();
        self.mark(DirtyFlags::LAYOUT);
        DirtyFlags::LAYOUT
    }

    /// A wheel notch at `pos`.
    pub fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags {
        let target = self
            .router
            .wheel(delta, pos)
            .first()
            .map(|region| region.node);
        let event = AppEvent::Wheel { pos, delta, target };
        self.dispatch(event)
    }

    /// A raw pointer event, for a shell that owns a different router.
    pub fn pointer_event(&mut self, event: PointerEvent) -> DirtyFlags {
        self.reduce_pointer(event)
    }

    /// A key press or release.
    pub fn key(&mut self, key: AppKey, pressed: bool, modifiers: Modifiers) -> DirtyFlags {
        let mut flags = DirtyFlags::NONE;
        if pressed {
            let focus_key = match key {
                AppKey::Tab if modifiers.shift => Some(FocusKey::ShiftTab),
                AppKey::Tab => Some(FocusKey::Tab),
                AppKey::Escape => Some(FocusKey::Escape),
                _ => None,
            };
            if let Some(focus_key) = focus_key {
                if self.focus.handle_key(focus_key).is_some() || focus_key == FocusKey::Escape {
                    flags = flags.merge(DirtyFlags::PAINT);
                }
                self.sync_focus_context();
            }
        }
        let flags = self
            .dispatch(AppEvent::Key {
                key,
                pressed,
                modifiers,
            })
            .merge(flags);
        self.mark(flags);
        flags
    }

    /// Type text.
    pub fn text(&mut self, text: impl Into<String>) -> DirtyFlags {
        self.dispatch(AppEvent::Text { text: text.into() })
    }

    /// An IME update.
    pub fn ime(&mut self, update: ImeUpdate) -> DirtyFlags {
        self.dispatch(AppEvent::Ime { update })
    }

    /// A resize.
    pub fn resize(&mut self, size: Vec2) -> DirtyFlags {
        self.ctx.viewport = size;
        self.force_rebuild = true;
        let flags = self
            .dispatch(AppEvent::Resize { size })
            .merge(DirtyFlags::LAYOUT);
        self.mark(flags);
        flags
    }

    /// A theme change.
    pub fn set_theme(&mut self, kind: ThemeKind) -> DirtyFlags {
        self.ctx.theme = kind;
        let flags = self
            .dispatch(AppEvent::Theme { kind })
            .merge(DirtyFlags::PAINT);
        self.mark(flags);
        flags
    }

    /// An accessibility action.
    pub fn a11y(&mut self, target: u64, action: A11yAction, value: Option<String>) -> DirtyFlags {
        self.dispatch(AppEvent::A11y {
            target,
            action,
            value,
        })
    }

    /// Advance the clock without producing a `Tick` event.
    pub fn advance(&mut self, dt: f32) {
        self.time += f64::from(dt.max(0.0));
        self.ctx.time = self.time;
    }

    /// Rebuild the scene, diff it against the previous frame, and update the
    /// router, the focus order and the accessibility snapshot.
    ///
    /// A frame with no dirty flags and no forced rebuild is a no-op that
    /// reports [`SceneDiff::Identical`] — the zero-work path.
    pub fn frame(&mut self, dt: f32) -> FrameReport {
        let dirty = core::mem::replace(&mut self.dirty, DirtyFlags::NONE);
        self.advance(dt);
        if dirty == DirtyFlags::NONE && !self.force_rebuild {
            return FrameReport {
                diff: SceneDiff::Identical,
                dirty,
                primitives: self.collector.primitives.len(),
                bytes: 0,
                aux_changed: false,
                backdrops: self.backdrops.clone(),
            };
        }

        // The focus order comes from the previous frame; adopting it before
        // the rebuild lets `Tab` act between frames.
        self.focus
            .set_order(self.collector.focus_order.iter().copied());
        self.sync_focus_context();

        self.collector.reset(ROOT);
        self.pending.clear();
        self.world = UiWorld::new();
        self.layout = LayoutEngine::new();
        let root = self.world.root();
        if let Some(data) = self.world.get_mut(root) {
            data.desc = RawNodeDesc::container().size(self.ctx.viewport.x, self.ctx.viewport.y);
        }

        let view_ctx = ViewContext {
            viewport: self.ctx.viewport,
            theme: self.ctx.theme,
            hovered: self.router.hover(),
            pressed: self.router.pointer().pressed(),
            focused: self.ctx.focused,
            focus_visible: self.ctx.focus_visible,
            force: self.force_rebuild,
        };
        let mut view = ViewBuilder {
            collector: &mut self.collector,
            ctx: view_ctx,
            world: &mut self.world,
            parent: root,
            pending: &mut self.pending,
        };
        self.app.view(&self.state, &mut view);
        self.layout_pending();

        // Publish the new hit regions and focus order.
        let regions = core::mem::take(&mut self.collector.regions);
        self.router.set_regions(regions);
        self.focus
            .set_order(self.collector.focus_order.iter().copied());
        self.sync_focus_context();

        // Only the tables a primitive *indexes into* decide whether a slice
        // patch can repair the pool. Payload changes (glyphs, gradients) are
        // uploaded separately through `aux_changed`, so typing in a text
        // field keeps patching the primitive pool instead of re-sending it.
        let structure_changed = self.collector.transforms != self.previous_transforms
            || self.collector.clips != self.previous_clips;
        let aux_changed = self.collector.payloads != self.previous_payloads;
        let diff = diff_primitives(
            &self.previous_primitives,
            &self.collector.primitives,
            structure_changed || self.force_rebuild,
        );
        let bytes = diff.bytes(self.collector.primitives.len());

        self.previous_primitives.clear();
        self.previous_primitives
            .extend_from_slice(&self.collector.primitives);
        self.previous_payloads.clear();
        self.previous_payloads
            .extend_from_slice(&self.collector.payloads);
        self.previous_transforms.clear();
        self.previous_transforms
            .extend_from_slice(&self.collector.transforms);
        self.previous_clips.clear();
        self.previous_clips.extend_from_slice(&self.collector.clips);

        self.diff = diff.clone();
        self.backdrops.clear();
        self.backdrops.extend_from_slice(&self.collector.backdrops);
        self.force_rebuild = false;

        FrameReport {
            diff,
            dirty,
            primitives: self.collector.primitives.len(),
            bytes,
            aux_changed,
            backdrops: self.backdrops.clone(),
        }
    }

    /// The layout world of the most recent frame.
    #[must_use]
    pub fn world(&self) -> &UiWorld {
        &self.world
    }

    /// Solve the declarative tree and emit the deferred widgets.
    ///
    /// Pass 1 builds the world and pre-measures nothing but buttons; pass 2
    /// hands Taffy a measurer for wrapping paragraphs, then emits every widget
    /// at the resolved rectangle.
    fn layout_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let viewport = self.ctx.viewport;
        let root = self.world.root();
        let texts: Vec<(NodeId, String, TextStyle)> = self
            .pending
            .iter()
            .filter_map(|entry| match &entry.spec {
                WidgetSpec::Text { text, style } => Some((entry.node, text.clone(), style.clone())),
                WidgetSpec::Button(_) => None,
            })
            .collect();
        {
            let world = &mut self.world;
            let text_engine = &mut self.text_engine;
            self.layout
                .solve_with_measurer(world, root, viewport, |id, known_width, available| {
                    let Some((_, text, style)) = texts.iter().find(|(node, _, _)| *node == id)
                    else {
                        return Vec2::ZERO;
                    };
                    let space = match available.width {
                        unbound_layout::AvailableSpace::Definite(width) => Some(width),
                        _ => None,
                    };
                    let width = known_width.or(space).unwrap_or(viewport.x).max(1.0);
                    let wrapped = text_engine.wrap(text, style, width);
                    Vec2::new(wrapped.width, wrapped.height)
                });
        }

        let pending = core::mem::take(&mut self.pending);
        for entry in &pending {
            let rect = self
                .world
                .get(entry.node)
                .map(|node| node.rect)
                .unwrap_or(Rect::ZERO);
            let WidgetSpec::Button(desc) = &entry.spec else {
                continue;
            };
            let phase = if self.ctx.pressed == Some(entry.key) {
                unbound_widgets::PointerPhase::Pressed
            } else if self.ctx.hovered == Some(entry.key) {
                unbound_widgets::PointerPhase::Hovered
            } else {
                unbound_widgets::PointerPhase::Idle
            };
            let state = ControlState {
                phase,
                focused: self.ctx.focused == Some(entry.key) && self.ctx.focus_visible,
                disabled: false,
                selected: false,
            };
            let mut desc = desc.clone();
            desc.rect = rect;
            let widget = ButtonWidget {
                node: entry.key,
                desc,
            };
            self.collector.push(entry.key, widget.emit(state, &mut []));
        }
        self.pending = pending;
    }

    fn reduce_pointer(&mut self, event: PointerEvent) -> DirtyFlags {
        match event {
            PointerEvent::Down {
                target,
                position,
                button,
                click_count,
            } => self.dispatch(AppEvent::PointerDown {
                pos: position,
                target,
                button,
                click_count,
            }),
            PointerEvent::Move {
                position,
                delta: _,
                target,
            } => self.dispatch(AppEvent::PointerMove {
                pos: position,
                hovered: target,
            }),
            PointerEvent::DragStart { target, position } => self.dispatch(AppEvent::DragStart {
                node: target,
                pos: position,
            }),
            PointerEvent::Drag {
                target,
                position,
                delta,
            } => self.dispatch(AppEvent::Drag {
                node: target,
                pos: position,
                delta,
            }),
            PointerEvent::Up {
                target,
                position,
                button,
            } => self.dispatch(AppEvent::PointerUp {
                pos: position,
                target,
                button,
            }),
            PointerEvent::Click {
                target,
                position,
                count,
            } => self.dispatch(AppEvent::Click {
                node: target,
                pos: position,
                count,
            }),
            PointerEvent::DoubleClick { target, position } => {
                self.dispatch(AppEvent::DoubleClick {
                    node: target,
                    pos: position,
                })
            }
            PointerEvent::CaptureLost { .. } => DirtyFlags::NONE,
        }
    }

    fn dispatch(&mut self, event: AppEvent) -> DirtyFlags {
        let flags = self.app.reduce(&mut self.state, event, &mut self.ctx);
        if let Some((key, cause)) = self.ctx.request_focus.take() {
            self.focus.focus(key, cause);
        }
        if core::mem::take(&mut self.ctx.request_blur) {
            self.focus.blur();
        }
        self.sync_focus_context();
        self.mark(flags);
        flags
    }

    fn mark(&mut self, flags: DirtyFlags) {
        self.dirty = self.dirty.merge(flags);
        if flags.needs_rebuild() {
            self.force_rebuild = true;
        }
    }

    fn sync_router_context(&mut self) {
        self.ctx.hovered = self.router.hover();
        self.ctx.pressed = self.router.pointer().pressed();
        self.ctx.cursor = self.router.cursor();
    }

    fn sync_focus_context(&mut self) {
        self.ctx.focused = self.focus.focused();
        self.ctx.focus_visible = self.focus.focus_visible();
    }
}

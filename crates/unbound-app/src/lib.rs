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

use std::cell::{Cell, RefCell, RefMut};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::A11yAction;
use unbound_abi::{Dim, NodeFlags, RawNodeDesc};
use unbound_gpu::atlas::AtlasUpload;
use unbound_gpu::data::{
    BlendMode, GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType,
    SubElement, CLIP_NONE,
};
use unbound_input::{
    CursorKind, FocusCause, FocusKey, FocusManager, KeyChord, KeyName, Keymap, PointerButton,
    PointerEvent, Router,
};
use unbound_layout::{LayoutEngine, LayoutStats};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine, TextStyle};
use unbound_theme::ThemeKind;
use unbound_widgets::alert::{emit_alert, AlertDesc};
use unbound_widgets::badge::{emit_badge, BadgeDesc};
use unbound_widgets::button::{ButtonDesc, ButtonWidget};
use unbound_widgets::checkbox::{emit_checkbox, CheckboxDesc, CheckboxTokens};
use unbound_widgets::diff::{diff_primitives, SceneDiff};
use unbound_widgets::divider::{emit_divider, DividerDesc, DividerTokens};
use unbound_widgets::progress::{emit_progress, ProgressDesc, ProgressState};
use unbound_widgets::slider::{emit_slider, SliderDesc, SliderTokens};
use unbound_widgets::switch::{emit_switch, SwitchDesc, SwitchTokens};
use unbound_widgets::{BackdropRequest, ControlState, SceneCollector, WidgetDesc, WidgetOutput};
use unbound_world::{NodeId, UiWorld};

pub mod assets;
pub mod guest_bridge;
pub mod inspector;
pub mod scheduler;
pub mod state_store;

pub use assets::{AssetHub, AssetResidency};
pub use guest_bridge::GuestAppAdapter;
pub use inspector::{Inspector, InspectorNode, InspectorScene};
pub use scheduler::{AppWaker, TimerQueue, TimerRequest, TimerToken};
pub use state_store::{LifecycleChanges, LifecycleEvent, WidgetStateStore};

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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AppKey {
    Tab,
    Escape,
    Enter,
    Space,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    ContextMenu,
    CapsLock,
    /// An `F1`..`F24` function key.
    F(u8),
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    /// Any other key, identified by its character.
    Character(char),
}

impl AppKey {
    /// The stable name a [`KeyChord`] binding uses.
    #[must_use]
    pub const fn chord_name(self) -> Option<&'static str> {
        Some(match self {
            Self::Tab => "Tab",
            Self::Escape => "Escape",
            Self::Enter => "Enter",
            Self::Space => "Space",
            Self::Backspace => "Backspace",
            Self::Delete => "Delete",
            Self::Insert => "Insert",
            Self::Left => "ArrowLeft",
            Self::Right => "ArrowRight",
            Self::Up => "ArrowUp",
            Self::Down => "ArrowDown",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
            Self::ContextMenu => "ContextMenu",
            Self::CapsLock => "CapsLock",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Paste => "Paste",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::F(_) | Self::Character(_) => return None,
        })
    }

    /// The chord this key forms with `modifiers`.
    ///
    /// Characters are folded to lower case: a keyboard reports `P` when Shift
    /// is held, but a binding written as `Ctrl+Shift+P` must match it.
    #[must_use]
    pub fn chord(self, modifiers: Modifiers) -> KeyChord {
        let key = match self {
            Self::Character(character) => KeyName::Char(character.to_ascii_lowercase()),
            Self::F(number) => KeyName::Function(number),
            other => KeyName::Named(other.chord_name().unwrap_or("Unidentified")),
        };
        KeyChord {
            key,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.meta,
        }
    }
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
    /// A secondary click completed over `node`.
    ContextMenu { node: u64, pos: Vec2 },
    /// A keyboard shortcut matched a [`Keymap`] binding.
    Shortcut { action: u64 },
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
    /// A timer scheduled through [`AppContext::set_timeout`] fired.
    Timer { token: TimerToken },
    /// OS files are hovering over the window.
    FileHover(Vec<std::path::PathBuf>),
    /// OS files were dropped onto the window.
    FileDrop(Vec<std::path::PathBuf>),
    /// An OS file drag left the window without dropping.
    FileHoverCancelled,
}

/// Platform-agnostic clipboard interface for `AppContext`.
pub trait ClipboardProvider {
    /// Read the current clipboard text, if any.
    fn get_text(&self) -> Option<String>;
    /// Replace the clipboard contents with `text`.
    fn set_text(&mut self, text: &str);
}

/// In-memory clipboard used in headless tests and as a fallback.
#[derive(Clone, Debug, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl MemoryClipboard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClipboardProvider for MemoryClipboard {
    fn get_text(&self) -> Option<String> {
        self.text.clone()
    }

    fn set_text(&mut self, text: &str) {
        self.text = Some(text.to_owned());
    }
}

/// The runtime's read/write view of the world during `reduce`.
#[derive(Clone)]
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
    /// The local state pool, shared with the view that renders it.
    widget_state: Rc<RefCell<WidgetStateStore>>,
    /// Timers requested during this reduction, drained by the runtime.
    timer_requests: Vec<TimerRequest>,
    /// Timers cancelled during this reduction, drained by the runtime.
    timer_cancels: Vec<TimerToken>,
    /// Shared with the runtime queue so tokens are unique across both.
    timer_tokens: Rc<Cell<u64>>,
    /// Where a background task posts its result.
    async_sender: Option<Sender<AppEvent>>,
    /// Wakes the parked event loop when a task posts a result.
    waker: Option<Arc<dyn AppWaker>>,
    /// How much of the current wheel delta the last handler consumed.
    scroll_consumed: Vec2,
    /// Shared clipboard provider (SystemClipboard in windowed mode, MemoryClipboard headless).
    clipboard: Rc<RefCell<Box<dyn ClipboardProvider>>>,
}

impl core::fmt::Debug for AppContext {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AppContext")
            .field("viewport", &self.viewport)
            .field("theme", &self.theme)
            .field("focused", &self.focused)
            .field("focus_visible", &self.focus_visible)
            .field("hovered", &self.hovered)
            .field("pressed", &self.pressed)
            .field("cursor", &self.cursor)
            .field("pointer", &self.pointer)
            .field("time", &self.time)
            .field("widget_state", &self.widget_state)
            .finish_non_exhaustive()
    }
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
            widget_state: Rc::new(RefCell::new(WidgetStateStore::new())),
            timer_requests: Vec::new(),
            timer_cancels: Vec::new(),
            timer_tokens: Rc::new(Cell::new(0)),
            async_sender: None,
            waker: None,
            scroll_consumed: Vec2::ZERO,
            clipboard: Rc::new(RefCell::new(Box::new(MemoryClipboard::new()))),
        }
    }

    /// Replace the clipboard provider (for example with `SystemClipboard`).
    pub fn set_clipboard(&mut self, provider: Box<dyn ClipboardProvider>) {
        *self.clipboard.borrow_mut() = provider;
    }

    /// Read text from the active clipboard provider.
    #[must_use]
    pub fn clipboard_get(&self) -> Option<String> {
        self.clipboard.borrow().get_text()
    }

    /// Write text to the active clipboard provider.
    pub fn clipboard_set(&self, text: &str) {
        self.clipboard.borrow_mut().set_text(text);
    }

    /// The local state for a node key.
    ///
    /// Unlike the view's [`ViewBuilder::use_state`], this does not mark the key
    /// as rendered: an event handler may poke at a widget's state without
    /// keeping a removed widget alive.
    pub fn use_state<T: Default + Send + Sync + 'static>(&self, key: u64) -> RefMut<'_, T> {
        RefMut::map(self.widget_state.borrow_mut(), |store| store.entry(key))
    }

    /// Schedule a one-shot event `after` this reduction.
    ///
    /// The token is allocated first and handed to `make`, so the event can
    /// carry the token it will be dispatched as; the runtime enqueues it once
    /// the event has been reduced.
    pub fn set_timeout(
        &mut self,
        after: Duration,
        make: impl FnOnce(TimerToken) -> AppEvent,
    ) -> TimerToken {
        let token = self.allocate_timer();
        self.timer_requests.push(TimerRequest {
            token,
            delay_ms: millis(after),
            period_ms: None,
            event: make(token),
        });
        token
    }

    /// Schedule a repeating event every `period`.
    pub fn set_interval(
        &mut self,
        period: Duration,
        make: impl FnOnce(TimerToken) -> AppEvent,
    ) -> TimerToken {
        let token = self.allocate_timer();
        self.timer_requests.push(TimerRequest {
            token,
            delay_ms: millis(period),
            period_ms: Some(millis(period)),
            event: make(token),
        });
        token
    }

    /// Cancel a timer scheduled earlier.
    pub fn cancel_timer(&mut self, token: TimerToken) {
        self.timer_cancels.push(token);
    }

    /// Report how much of a wheel delta this handler used.
    ///
    /// The runtime subtracts it from the delta and offers the remainder to the
    /// next region under the pointer, which is how a nested scroll area hands
    /// the leftover wheel to its outer scroller.
    pub fn consume_scroll(&mut self, delta: Vec2) {
        self.scroll_consumed += delta;
    }

    /// Run `task` on a worker thread and reduce the event it returns.
    ///
    /// The result is delivered on the next [`AppRuntime::poll_async`] (called
    /// automatically at the top of every frame); [`AppWaker`] wakes a parked
    /// event loop in the meantime.
    pub fn spawn(&self, task: impl FnOnce() -> AppEvent + Send + 'static) {
        let Some(sender) = self.async_sender.clone() else {
            return;
        };
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let event = task();
            if sender.send(event).is_ok() {
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        });
    }

    fn allocate_timer(&self) -> TimerToken {
        let id = self.timer_tokens.get().saturating_add(1);
        self.timer_tokens.set(id);
        TimerToken::new(id)
    }

    /// Drain the timer requests for the runtime.
    fn take_timer_requests(&mut self) -> (Vec<TimerRequest>, Vec<TimerToken>) {
        (
            core::mem::take(&mut self.timer_requests),
            core::mem::take(&mut self.timer_cancels),
        )
    }

    /// Share the runtime's timer token counter.
    fn share_timer_tokens(&mut self, tokens: Rc<Cell<u64>>) {
        self.timer_tokens = tokens;
    }

    /// Share the runtime's local state pool with the view.
    fn share_state(&mut self, state: Rc<RefCell<WidgetStateStore>>) {
        self.widget_state = state;
    }

    /// Install the channel and waker used by [`Self::spawn`].
    fn install_async(&mut self, sender: Sender<AppEvent>, waker: Option<Arc<dyn AppWaker>>) {
        self.async_sender = Some(sender);
        self.waker = waker;
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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
    /// A checkbox control.
    Checkbox(CheckboxDesc),
    /// A toggle switch.
    Switch(SwitchDesc),
    /// A continuous slider.
    Slider(SliderDesc),
    /// A progress bar or ring.
    Progress {
        /// Descriptor (rect is replaced by layout).
        desc: ProgressDesc,
        /// Animation state.
        state: ProgressState,
    },
    /// A pill badge.
    Badge(BadgeDesc),
    /// A hairline divider.
    Divider(DividerDesc),
    /// A status alert banner.
    Alert(AlertDesc),
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
    widget_state: Rc<RefCell<WidgetStateStore>>,
    node_keys: &'a mut FxHashMap<u64, NodeId>,
    seen_keys: &'a mut FxHashSet<u64>,
    ephemeral_nodes: &'a mut Vec<NodeId>,
}

impl<'a> ViewBuilder<'a> {
    /// The local state for a widget key.
    ///
    /// The key is recorded as rendered, so a key that stops calling this is
    /// unmounted at the end of the frame and its value reclaimed.
    pub fn use_state<T: Default + Send + Sync + 'static>(&mut self, key: u64) -> RefMut<'_, T> {
        RefMut::map(self.widget_state.borrow_mut(), |store| store.use_state(key))
    }

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

    fn reconcile_node(
        &mut self,
        key: u64,
        mut desc: RawNodeDesc,
        loc: &'static core::panic::Location<'static>,
    ) -> NodeId {
        desc.key = key;
        if key != 0 {
            self.seen_keys.insert(key);
            if let Some(&existing) = self.node_keys.get(&key) {
                if let Some(node_data) = self.world.get(existing) {
                    if node_data.parent == self.parent {
                        self.world.set_desc(existing, desc);
                        return existing;
                    }
                }
            }
        }
        let node = self.world.insert(self.parent, desc, loc);
        if key != 0 {
            self.node_keys.insert(key, node);
        } else {
            self.ephemeral_nodes.push(node);
        }
        node
    }

    /// Declare a layout-only container with an explicit descriptor.
    pub fn container(
        &mut self,
        key: u64,
        desc: RawNodeDesc,
        build: impl FnOnce(&mut Self),
    ) -> NodeId {
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
        let parent = core::mem::replace(&mut self.parent, node);
        build(self);
        self.parent = parent;
        node
    }

    /// Place a widget in the current container; emit happens after layout.
    pub fn widget(&mut self, key: u64, mut desc: RawNodeDesc, spec: WidgetSpec) -> NodeId {
        desc.key = key;
        let intrinsic = match &spec {
            WidgetSpec::Button(button) => Some(Vec2::new(
                (button.label.chars().count() as f32 * 8.0 + 32.0).max(button.rect.width()),
                button.rect.height().max(32.0),
            )),
            WidgetSpec::Checkbox(cb) => Some(Vec2::new(
                cb.rect.width().max(120.0),
                cb.rect.height().max(24.0),
            )),
            WidgetSpec::Switch(sw) => Some(Vec2::new(
                sw.rect.width().max(48.0),
                sw.rect.height().max(26.0),
            )),
            WidgetSpec::Slider(sl) => Some(Vec2::new(
                sl.rect.width().max(160.0),
                sl.rect.height().max(28.0),
            )),
            WidgetSpec::Progress { desc: pr, .. } => Some(Vec2::new(
                pr.rect.width().max(160.0),
                pr.rect.height().max(12.0),
            )),
            WidgetSpec::Badge(badge) => Some(Vec2::new(
                badge.rect.width().max(64.0),
                badge.rect.height().max(22.0),
            )),
            WidgetSpec::Divider(div) => Some(Vec2::new(
                div.rect.width().max(1.0),
                div.rect.height().max(1.0),
            )),
            WidgetSpec::Alert(alert) => Some(Vec2::new(
                alert.rect.width().max(240.0),
                alert.rect.height().max(48.0),
            )),
            WidgetSpec::Text { .. } => None,
        };
        if let Some(size) = intrinsic {
            desc = desc.size(size.x, size.y);
        }
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
        if let Some(size) = intrinsic {
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
        let node = self.reconcile_node(key, desc, core::panic::Location::caller());
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
    ) -> EventOutcome;

    /// React to a lifecycle transition for a local state key.
    ///
    /// Runs after the view, once the runtime knows which keys appeared or
    /// disappeared, and when the keyboard focus moves. Returning flags that
    /// need a rebuild schedules one for the next frame.
    fn lifecycle(&mut self, _state: &mut Self::State, _event: LifecycleEvent) -> DirtyFlags {
        DirtyFlags::NONE
    }

    /// Emit the scene for the current state.
    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>);
}

/// Whether an event keeps bubbling up the hit path.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Propagation {
    /// The event may be offered to the next ancestor.
    #[default]
    Bubble,
    /// The handler consumed the event; the runtime stops walking the path.
    Stop,
}

/// What reducing one event produced.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct EventOutcome {
    /// The invalidation the reduction caused.
    pub dirty: DirtyFlags,
    /// Whether the event should still be offered to an ancestor.
    pub propagation: Propagation,
}

impl EventOutcome {
    /// An outcome that changed `dirty` and keeps bubbling.
    #[must_use]
    pub const fn new(dirty: DirtyFlags) -> Self {
        Self {
            dirty,
            propagation: Propagation::Bubble,
        }
    }

    /// An outcome that stops the event.
    #[must_use]
    pub const fn stopping(dirty: DirtyFlags) -> Self {
        Self {
            dirty,
            propagation: Propagation::Stop,
        }
    }

    /// The same outcome, marked as consumed.
    #[must_use]
    pub const fn stop(self) -> Self {
        Self {
            dirty: self.dirty,
            propagation: Propagation::Stop,
        }
    }
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
    /// Atlas rectangles the asset hub decoded this frame.
    ///
    /// The shell uploads them with [`unbound_gpu::atlas::AtlasSet::upload_batch`]
    /// before it draws; the runtime itself never touches the GPU.
    pub atlas_uploads: Vec<AtlasUpload>,
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
    glyph_cache: GlyphCache,
    asset_hub: AssetHub,
    pending: Vec<PendingWidget>,
    widget_state: Rc<RefCell<WidgetStateStore>>,
    timers: TimerQueue,
    async_rx: Receiver<AppEvent>,
    keymap: Keymap,
    key_context: Option<String>,
    last_focused: Option<u64>,
    inspector: Inspector,
    debug_sanitizers: bool,
    node_keys: FxHashMap<u64, NodeId>,
    seen_keys: FxHashSet<u64>,
    ephemeral_nodes: Vec<NodeId>,
    last_layout_stats: LayoutStats,
    layout_initialized: bool,
}

impl<A: Application> AppRuntime<A> {
    /// Build a runtime, initialising the application state.
    pub fn new(mut app: A, viewport: Vec2) -> Self {
        let mut ctx = AppContext::new(viewport);
        let widget_state = Rc::new(RefCell::new(WidgetStateStore::new()));
        let timers = TimerQueue::new();
        let (async_tx, async_rx) = crossbeam_channel::unbounded();
        ctx.share_state(Rc::clone(&widget_state));
        ctx.share_timer_tokens(timers.tokens());
        ctx.install_async(async_tx.clone(), None);
        let state = app.init(&mut ctx);
        let mut runtime = Self {
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
            glyph_cache: GlyphCache::new(),
            asset_hub: AssetHub::new(),
            pending: Vec::new(),
            widget_state,
            timers,
            async_rx,
            keymap: Keymap::new(),
            key_context: None,
            last_focused: None,
            inspector: Inspector::new(),
            debug_sanitizers: false,
            node_keys: FxHashMap::default(),
            seen_keys: FxHashSet::default(),
            ephemeral_nodes: Vec::new(),
            last_layout_stats: LayoutStats::default(),
            layout_initialized: false,
        };
        // A timer scheduled from `init` is already due relative to the zero
        // clock; enqueue it before the first frame can advance past it.
        runtime.collect_timer_requests();
        runtime
    }

    /// Install the waker a background task uses to wake a parked event loop.
    pub fn set_waker(&mut self, waker: Arc<dyn AppWaker>) {
        self.ctx.waker = Some(waker);
    }

    /// The mutable shortcut table.
    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    /// Set the active shortcut scope (`"TextInput"`, `"Modal"`, ...).
    pub fn set_key_context(&mut self, context: Option<&str>) {
        self.key_context = context.map(str::to_owned);
        self.keymap.clear_pending();
    }

    /// The DevTools inspector state.
    #[must_use]
    pub const fn inspector(&self) -> &Inspector {
        &self.inspector
    }

    /// Mutable access for shell-side wiring (e.g. a menu item).
    pub fn inspector_mut(&mut self) -> &mut Inspector {
        &mut self.inspector
    }

    /// Toggle the inspector overlay; returns the new state.
    pub fn toggle_inspector(&mut self) -> bool {
        let enabled = self.inspector.toggle();
        // The overlay is part of the scene, so the next frame has to rebuild
        // even when no widget dirtied itself.
        self.force_rebuild = true;
        self.dirty |= DirtyFlags::PAINT;
        enabled
    }

    /// Turn the per-frame headless sanitizer pass on or off.
    ///
    /// While on, every frame mirrors the world into an `unbound_headless::Frame` and
    /// runs the five sanitizers; violations show up as blinking boxes in the
    /// inspector. This is a debug switch and costs a full tree walk per frame.
    pub fn enable_debug_sanitizers(&mut self, enabled: bool) {
        self.debug_sanitizers = enabled;
        self.force_rebuild = true;
        self.dirty |= DirtyFlags::PAINT;
    }

    /// Whether the sanitizer pass is currently enabled.
    #[must_use]
    pub const fn debug_sanitizers_enabled(&self) -> bool {
        self.debug_sanitizers
    }

    /// How long until the next timer, for the host's `ControlFlow`.
    #[must_use]
    pub fn next_wake_in(&self) -> Option<Duration> {
        let deadline = self.timers.next_deadline_ms()?;
        Some(Duration::from_millis(
            deadline.saturating_sub(self.now_ms()),
        ))
    }

    /// Reduce every result a background task posted.
    pub fn poll_async(&mut self) -> DirtyFlags {
        let mut flags = DirtyFlags::NONE;
        while let Ok(event) = self.async_rx.try_recv() {
            flags = flags.merge(self.dispatch(event));
        }
        flags
    }

    /// Reduce every timer due at the current clock.
    pub fn poll_timers(&mut self) -> DirtyFlags {
        self.collect_timer_requests();
        let due = self.timers.due(self.now_ms());
        let mut flags = DirtyFlags::NONE;
        for event in due {
            flags = flags.merge(self.dispatch(event));
        }
        flags
    }

    /// Access one local state value by key.
    pub fn with_state<T: Default + Send + Sync + 'static, R>(
        &mut self,
        key: u64,
        f: impl FnOnce(&mut T) -> R,
    ) -> R {
        let mut store = self.widget_state.borrow_mut();
        f(store.entry(key))
    }

    /// The local state pool, for a test or a host that needs to inspect it.
    #[must_use]
    pub fn state_store(&self) -> core::cell::Ref<'_, WidgetStateStore> {
        self.widget_state.borrow()
    }

    fn now_ms(&self) -> u64 {
        millis(Duration::from_secs_f64(self.time.max(0.0)))
    }

    fn collect_timer_requests(&mut self) {
        let (requests, cancels) = self.ctx.take_timer_requests();
        let now = self.now_ms();
        for request in requests {
            self.timers.schedule(
                request.token,
                now,
                request.delay_ms,
                request.period_ms,
                request.event,
            );
        }
        for token in cancels {
            self.timers.cancel(token);
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

    /// Press `button`.
    pub fn pointer_down(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        let event = self.router.pointer_down_button(pos, button, self.time);
        self.ctx.pointer = pos;
        let flags = event.map_or(DirtyFlags::NONE, |event| self.reduce_pointer(event));
        self.sync_router_context();
        self.mark(flags.merge(DirtyFlags::PAINT));
        flags
    }

    /// Release `button`.
    pub fn pointer_up(&mut self, pos: Vec2, button: PointerButton) -> DirtyFlags {
        let events = self.router.pointer_up_button(pos, button, self.time);
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
    ///
    /// The delta walks the hit stack topmost-first. Each handler reports what
    /// it consumed through [`AppContext::consume_scroll`]; the remainder is
    /// offered to the ancestor under it, so a scroll area at its end hands the
    /// wheel to the page behind it. A handler that returns
    /// [`Propagation::Stop`] ends the chain immediately.
    pub fn wheel(&mut self, pos: Vec2, delta: Vec2) -> DirtyFlags {
        let path: Vec<u64> = self
            .router
            .wheel(delta, pos)
            .iter()
            .map(|region| region.node)
            .collect();
        if path.is_empty() {
            return self.dispatch(AppEvent::Wheel {
                pos,
                delta,
                target: None,
            });
        }

        let mut remaining = delta;
        let mut flags = DirtyFlags::NONE;
        for node in path {
            if remaining.length() <= 0.01 {
                break;
            }
            self.ctx.scroll_consumed = Vec2::ZERO;
            let outcome = self.dispatch_outcome(AppEvent::Wheel {
                pos,
                delta: remaining,
                target: Some(node),
            });
            remaining -= self.ctx.scroll_consumed;
            flags = flags.merge(outcome.dirty);
            if outcome.propagation == Propagation::Stop {
                break;
            }
        }
        flags
    }

    /// A raw pointer event, for a shell that owns a different router.
    pub fn pointer_event(&mut self, event: PointerEvent) -> DirtyFlags {
        self.reduce_pointer(event)
    }

    /// A key press or release.
    pub fn key(&mut self, key: AppKey, pressed: bool, modifiers: Modifiers) -> DirtyFlags {
        let mut flags = DirtyFlags::NONE;
        if pressed && matches!(key, AppKey::F(12)) {
            // F12 is the DevTools toggle; it never reaches the application.
            self.toggle_inspector();
            return DirtyFlags::PAINT;
        }
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

            // A bound chord is a command, not text: reducing it instead of
            // the raw key stops a focused text field from also inserting it.
            let action =
                self.keymap
                    .feed(key.chord(modifiers), self.key_context.as_deref(), self.time);
            if let Some(action) = action {
                let flags = self.dispatch(AppEvent::Shortcut { action }).merge(flags);
                self.mark(flags);
                return flags;
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

    /// OS files hovered over the window.
    pub fn file_hover(&mut self, paths: Vec<std::path::PathBuf>) -> DirtyFlags {
        self.dispatch(AppEvent::FileHover(paths))
    }

    /// OS files dropped onto the window.
    pub fn file_drop(&mut self, paths: Vec<std::path::PathBuf>) -> DirtyFlags {
        self.dispatch(AppEvent::FileDrop(paths))
    }

    /// OS file hover cancelled.
    pub fn file_hover_cancelled(&mut self) -> DirtyFlags {
        self.dispatch(AppEvent::FileHoverCancelled)
    }

    /// Install a platform clipboard provider on the runtime's context.
    pub fn set_clipboard(&mut self, provider: Box<dyn ClipboardProvider>) {
        self.ctx.set_clipboard(provider);
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
        self.advance(dt);
        // Results and timers that arrived since the previous frame are
        // reduced first, so a frame that only woke up for them still rebuilds.
        self.poll_async();
        self.poll_timers();
        // Decoded assets invalidate the same way a reduction does.
        let asset_dirty = self.asset_hub.poll(&mut self.text_engine);
        self.dirty = self.dirty.merge(asset_dirty);
        let dirty = core::mem::replace(&mut self.dirty, DirtyFlags::NONE);
        if dirty == DirtyFlags::NONE && !self.force_rebuild {
            return FrameReport {
                diff: SceneDiff::Identical,
                dirty,
                primitives: self.collector.primitives.len(),
                bytes: 0,
                aux_changed: false,
                backdrops: self.backdrops.clone(),
                atlas_uploads: self.asset_hub.take_uploads(),
            };
        }

        // The focus order comes from the previous frame; adopting it before
        // the rebuild lets `Tab` act between frames.
        self.focus
            .set_order(self.collector.focus_order.iter().copied());
        self.sync_focus_context();

        self.collector.reset(ROOT);
        self.pending.clear();
        self.widget_state.borrow_mut().begin_frame();
        for old_ephemeral in self.ephemeral_nodes.drain(..) {
            if self.world.get(old_ephemeral).is_some() {
                self.world.remove(old_ephemeral);
            }
        }
        self.seen_keys.clear();
        let root = self.world.root();
        self.world.set_desc(
            root,
            RawNodeDesc::container().size(self.ctx.viewport.x, self.ctx.viewport.y),
        );

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
            widget_state: Rc::clone(&self.widget_state),
            node_keys: &mut self.node_keys,
            seen_keys: &mut self.seen_keys,
            ephemeral_nodes: &mut self.ephemeral_nodes,
        };
        self.app.view(&self.state, &mut view);
        let vanished: Vec<(u64, NodeId)> = self
            .node_keys
            .iter()
            .filter(|(k, _)| !self.seen_keys.contains(k))
            .map(|(k, v)| (*k, *v))
            .collect();
        for (k, node) in vanished {
            self.node_keys.remove(&k);
            if self.world.get(node).is_some() {
                self.world.remove(node);
            }
        }
        self.layout_pending();
        self.dispatch_lifecycle();

        // Publish the new hit regions and focus order.
        let regions = core::mem::take(&mut self.collector.regions);
        self.router.set_regions(regions);
        self.focus
            .set_order(self.collector.focus_order.iter().copied());
        self.sync_focus_context();

        // DevTools overlay + the debug sanitizer loop. Both read the world and
        // the hit regions the frame just produced, and both append to the end
        // of the primitive list so the debug pass paints last.
        let inspector_active = self.inspector.enabled() || self.debug_sanitizers;
        if inspector_active {
            self.inspector.tick(dt);
            self.inspector
                .update(&self.world, self.router.regions(), self.ctx.pointer);
            if self.debug_sanitizers {
                let frame = inspector::capture_frame(&self.world, self.ctx.viewport);
                let report = unbound_headless::Harness::run(&frame);
                self.inspector.set_diagnostics(&report.diagnostics);
            } else {
                self.inspector.clear_diagnostics();
            }
            if self.inspector.enabled() {
                let scene = self.inspector.emit(self.ctx.viewport);
                self.collector.primitives.extend(scene.primitives);
                self.collector.payloads.extend(scene.payloads);
                let panel_w = inspector::PANEL_WIDTH.min(self.ctx.viewport.x * 0.45);
                let panel_x = self.ctx.viewport.x - panel_w + 12.0;
                let style = TextStyle::sized(11.0);
                for (index, line) in scene.lines.iter().enumerate().take(16) {
                    let shaped = self.text_engine.shape(line, &style);
                    let origin = Vec2::new(panel_x, 16.0 + index as f32 * 18.0);
                    let quads = self.glyph_cache.quads_for(
                        &mut self.text_engine,
                        &shaped,
                        origin,
                        400,
                        false,
                    );
                    for quad in quads {
                        let payload_idx = self.collector.payloads.len() as u32;
                        self.collector.payloads.push(GpuPayload {
                            p0: [quad.uv[0], quad.uv[1]],
                            p1: [quad.uv[2], quad.uv[3]],
                            p2: [0.0, 0.0],
                            line_width: 0.0,
                            extra: 0,
                            flags: 0,
                            _reserved: [0; 3],
                        });
                        let prim_type = if quad.color {
                            PrimType::GlyphColor
                        } else {
                            PrimType::Glyph
                        };
                        let glyph = GpuPrimitiveInstance {
                            local_pos: [quad.rect.origin.x, quad.rect.origin.y],
                            size: [quad.rect.size.x, quad.rect.size.y],
                            clip_idx: CLIP_NONE,
                            border_token_or_tex: quad.layer,
                            extra_payload_idx: payload_idx,
                            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(u16::MAX, 1.0),
                            fill_token_or_uv: u32::from(unbound_theme::Token::TEXT_PRIMARY.raw()),
                            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                                prim_type,
                                BlendMode::Normal,
                                SubElement::Content as u16,
                            ) | if quad.msdf {
                                u32::from(PrimFlags::MSDF.bits()) << 16
                            } else {
                                0
                            },
                            ..Default::default()
                        };
                        self.collector.primitives.push(glyph);
                    }
                }
            }
        }

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
        // The inspector blinks and follows the pointer, so its presence forces
        // the next frame to rebuild even when nothing else is dirty.
        self.force_rebuild = inspector_active;

        FrameReport {
            diff,
            dirty,
            primitives: self.collector.primitives.len(),
            bytes,
            aux_changed,
            backdrops: self.backdrops.clone(),
            atlas_uploads: self.asset_hub.take_uploads(),
        }
    }

    /// The asset hub, for submitting images and fonts.
    #[must_use]
    pub fn asset_hub(&self) -> &AssetHub {
        &self.asset_hub
    }

    /// Mutable access to the asset hub.
    pub fn asset_hub_mut(&mut self) -> &mut AssetHub {
        &mut self.asset_hub
    }

    /// The layout world of the most recent frame.
    #[must_use]
    pub fn world(&self) -> &UiWorld {
        &self.world
    }

    /// The application itself, for inspection from tests and debug overlays.
    #[must_use]
    pub fn app(&self) -> &A {
        &self.app
    }

    /// The layout statistics from the most recent solve pass.
    #[must_use]
    pub fn last_layout_stats(&self) -> LayoutStats {
        self.last_layout_stats
    }

    /// Solve the declarative tree and emit the deferred widgets.
    ///
    /// Pass 1 reconciles the world and pre-measures widgets; pass 2 solves
    /// either the full tree (on topology/viewport change) or only dirty
    /// relayout boundaries via [`LayoutEngine::solve_boundaries`], then emits
    /// every widget at its resolved rectangle.
    fn layout_pending(&mut self) {
        if self.pending.is_empty() && !self.world.topology_dirty() {
            let boundaries = self.world.dirty_boundaries();
            if boundaries.is_empty() {
                self.last_layout_stats = LayoutStats::default();
                return;
            }
            self.last_layout_stats =
                self.layout
                    .solve_boundaries(&mut self.world, &boundaries, self.ctx.viewport);
            self.world.clear_dirty();
            return;
        }
        let viewport = self.ctx.viewport;
        let root = self.world.root();
        let texts: Vec<(NodeId, String, TextStyle)> = self
            .pending
            .iter()
            .filter_map(|entry| match &entry.spec {
                WidgetSpec::Text { text, style } => Some((entry.node, text.clone(), style.clone())),
                _ => None,
            })
            .collect();
        if !self.layout_initialized || self.world.topology_dirty() || self.force_rebuild {
            let world = &mut self.world;
            let text_engine = &mut self.text_engine;
            self.last_layout_stats = self.layout.solve_with_measurer(
                world,
                root,
                viewport,
                |id, known_width, available| {
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
                },
            );
            self.layout_initialized = true;
            self.world.clear_dirty();
        } else {
            let boundaries = self.world.dirty_boundaries();
            if !boundaries.is_empty() {
                self.last_layout_stats =
                    self.layout
                        .solve_boundaries(&mut self.world, &boundaries, viewport);
                self.world.clear_dirty();
            } else {
                self.last_layout_stats = LayoutStats::default();
            }
        }

        let pending = core::mem::take(&mut self.pending);
        for entry in &pending {
            let rect = self
                .world
                .get(entry.node)
                .map(|node| node.rect)
                .unwrap_or(Rect::ZERO);
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
            match &entry.spec {
                WidgetSpec::Button(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    let widget = ButtonWidget {
                        node: entry.key,
                        desc,
                    };
                    self.collector.push(entry.key, widget.emit(state, &mut []));
                }
                WidgetSpec::Checkbox(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector.push(
                        entry.key,
                        emit_checkbox(
                            entry.key,
                            &desc,
                            state,
                            &[],
                            &[],
                            CheckboxTokens::standard(),
                        ),
                    );
                }
                WidgetSpec::Switch(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector.push(
                        entry.key,
                        emit_switch(entry.key, &desc, state, &[], SwitchTokens::standard()),
                    );
                }
                WidgetSpec::Slider(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector.push(
                        entry.key,
                        emit_slider(entry.key, &desc, state, &[], SliderTokens::standard()),
                    );
                }
                WidgetSpec::Progress {
                    desc,
                    state: pstate,
                } => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector.push(entry.key, emit_progress(&desc, pstate));
                }
                WidgetSpec::Badge(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector.push(entry.key, emit_badge(&desc, &[]));
                }
                WidgetSpec::Divider(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector.push(
                        entry.key,
                        emit_divider(&desc, &[], 0.0, DividerTokens::standard()),
                    );
                }
                WidgetSpec::Alert(desc) => {
                    let mut desc = desc.clone();
                    desc.rect = rect;
                    self.collector
                        .push(entry.key, emit_alert(entry.key, &desc, &[], state));
                }
                WidgetSpec::Text { .. } => {}
            }
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
            } => {
                let mut flags = self.dispatch(AppEvent::PointerUp {
                    pos: position,
                    target,
                    button,
                });
                // A secondary click opens the menu only when the release is
                // still over the node that was pressed.
                if button == PointerButton::Right {
                    if let Some(node) = target {
                        if self.router.hover() == Some(node) {
                            flags = flags.merge(self.dispatch(AppEvent::ContextMenu {
                                node,
                                pos: position,
                            }));
                        }
                    }
                }
                flags
            }
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

    /// Report the mount / unmount delta and the focus change to the app.
    fn dispatch_lifecycle(&mut self) {
        let changes = {
            // Do not hold the borrow while the hooks run: a hook may ask for
            // the same key's state again.
            self.widget_state.borrow_mut().end_frame()
        };
        let mut flags = DirtyFlags::NONE;
        for key in &changes.unmounted {
            flags = flags.merge(
                self.app
                    .lifecycle(&mut self.state, LifecycleEvent::Unmounted(*key)),
            );
        }
        for key in &changes.mounted {
            flags = flags.merge(
                self.app
                    .lifecycle(&mut self.state, LifecycleEvent::Mounted(*key)),
            );
        }

        let focused = self.ctx.focused;
        if focused != self.last_focused {
            if let Some(previous) = self.last_focused {
                flags = flags.merge(
                    self.app
                        .lifecycle(&mut self.state, LifecycleEvent::Blurred(previous)),
                );
            }
            if let Some(current) = focused {
                flags = flags.merge(
                    self.app
                        .lifecycle(&mut self.state, LifecycleEvent::Focused(current)),
                );
            }
            self.last_focused = focused;
        }
        self.mark(flags);
    }

    /// Dispatch one resolved event outside the pointer/keyboard path.
    ///
    /// Embedders that synthesise events (tests, remote control, a scripting
    /// bridge) use this instead of reaching into the router. The returned flags
    /// are already marked on the runtime, so the next [`Self::frame`] rebuilds.
    pub fn dispatch(&mut self, event: AppEvent) -> DirtyFlags {
        self.dispatch_outcome(event).dirty
    }

    fn dispatch_outcome(&mut self, event: AppEvent) -> EventOutcome {
        let outcome = self.app.reduce(&mut self.state, event, &mut self.ctx);
        if let Some((key, cause)) = self.ctx.request_focus.take() {
            self.focus.focus(key, cause);
        }
        if core::mem::take(&mut self.ctx.request_blur) {
            self.focus.blur();
        }
        self.sync_focus_context();
        self.collect_timer_requests();
        self.mark(outcome.dirty);
        outcome
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

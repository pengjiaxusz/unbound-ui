//! The windowing backend: `winit` events translated into engine events.
//!
//! The translation is deliberately one-way and lossy-free: the engine layer
//! never sees a `winit` type, so the windowing library can be swapped without
//! touching the renderer.

use std::time::Instant;

use crate::latency::LatencyTracker;
use crate::pacing::{FramePacer, PresentWaitPolicy};
use crate::resize::{Extent, ResizeAction, ResizeTracker};

/// How many device pixels one "line" of mouse-wheel travel is worth.
///
/// Windows reports wheel movement in pixels while most other platforms report
/// it in lines; the conversion keeps the engine's unit platform-independent.
const PIXELS_PER_LINE: f64 = 40.0;

/// Configuration of the platform window.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Initial logical width.
    pub width: u32,
    /// Initial logical height.
    pub height: u32,
    /// Whether the user may resize the window.
    pub resizable: bool,
    /// Whether the window starts visible.
    pub visible: bool,
    /// Target refresh interval.
    pub refresh: std::time::Duration,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Unbound UI".to_string(),
            width: 1280,
            height: 800,
            resizable: true,
            visible: true,
            refresh: crate::pacing::REFRESH_120,
        }
    }
}

/// The state of a modifier key set.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Whether a button or key is down.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// A mouse button.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// A keyboard key, flattened to the few categories the engine cares about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformKey {
    /// A printable character.
    Character(String),
    /// A named key such as `Enter` or `ArrowLeft`.
    Named(String),
    /// Any other key, identified by its platform code.
    Other(u32),
}

/// Everything the engine layer needs to know about the window.
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    /// The window was resized by the OS.
    Resized(Extent),
    /// The swapchain must be recreated at this extent.
    RecreateSwapchain(Extent),
    /// The OS scale factor changed.
    ScaleFactorChanged(f64),
    /// The pointer moved, in logical pixels relative to the window.
    CursorMoved { x: f64, y: f64 },
    /// Raw pointer motion, in device pixels.
    PointerRaw { dx: f64, dy: f64 },
    /// A mouse button changed state.
    MouseInput {
        state: ButtonState,
        button: MouseButton,
    },
    /// The mouse wheel turned, in logical lines.
    ///
    /// Windows reports pixel deltas; they are converted to lines here so the
    /// engine never has to know about the platform's scroll granularity.
    MouseWheel {
        delta_x: f64,
        delta_y: f64,
    },
    /// A key changed state.
    KeyInput {
        state: ButtonState,
        key: PlatformKey,
        repeat: bool,
    },
    /// The modifier keys changed.
    ModifiersChanged(Modifiers),
    /// The window gained or lost focus.
    Focused(bool),
    /// The window became visible or hidden.
    VisibilityChanged(bool),
    /// The OS asks for a new frame.
    RedrawRequested,
    /// The user asked to close the window.
    CloseRequested,
}

/// Errors raised while creating or running the window.
#[derive(Debug)]
pub enum PlatformError {
    /// The windowing system refused the request.
    Window(String),
    /// The event loop could not be created.
    EventLoop(String),
}

impl core::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Window(message) => write!(f, "window error: {message}"),
            Self::EventLoop(message) => write!(f, "event loop error: {message}"),
        }
    }
}

impl std::error::Error for PlatformError {}

/// The live state of the platform layer.
///
/// Everything the engine needs to poll each frame lives here, and nothing in
/// it depends on `winit`.
#[derive(Debug)]
pub struct PlatformState {
    /// Resize bookkeeping.
    pub resize: ResizeTracker,
    /// Input-to-photon accounting.
    pub latency: LatencyTracker,
    /// Frame pacing.
    pub pacer: FramePacer,
    /// Presentation policy.
    pub present: PresentWaitPolicy,
    /// The current OS scale factor.
    pub scale_factor: f64,
    /// Whether the window is focused.
    pub focused: bool,
    /// Whether the window is visible.
    pub visible: bool,
}

impl PlatformState {
    /// Create the state for a freshly created window.
    #[must_use]
    pub fn new(config: &WindowConfig, extent: Extent, present: PresentWaitPolicy) -> Self {
        Self {
            resize: ResizeTracker::with_default_settle(extent),
            latency: LatencyTracker::new(),
            pacer: FramePacer::new(config.refresh),
            present,
            scale_factor: 1.0,
            focused: true,
            visible: config.visible,
        }
    }

    /// Tell the platform which device extensions the renderer actually got.
    ///
    /// The window exists before the Vulkan device does, so the presentation
    /// policy cannot be derived at construction time. The application
    /// creates the device in [`crate::backend::PlatformApp::resumed`] and
    /// reports the extensions back here; without this the policy would stay
    /// at its "no extensions" fallback and the low-latency present path
    /// would never be taken.
    pub fn set_present_extensions(&mut self, extensions: &[String]) {
        self.present = PresentWaitPolicy::from_extensions(extensions);
    }

    /// Poll the resize tracker and commit a size that has settled.
    ///
    /// The settle window needs a tick that does **not** depend on another
    /// resize event arriving. A drag burst ends with one final event, and if
    /// nothing else ever arrives, that last size would never be committed —
    /// the window would keep showing the stale extent forever.
    ///
    /// Returns the new extent when the swapchain must be rebuilt.
    pub fn poll_resize(&mut self, now: Instant) -> Option<Extent> {
        match self.resize.poll(now) {
            ResizeAction::None => None,
            ResizeAction::Recreate(extent) => Some(extent),
        }
    }
}

/// Translate a `winit` window event into a platform event.
///
/// Returns `None` for events the engine does not care about.
#[must_use]
pub fn translate_window_event(event: &winit::event::WindowEvent) -> Option<PlatformEvent> {
    use winit::event::{ElementState, MouseButton as WinitMouseButton, WindowEvent};

    Some(match event {
        WindowEvent::Resized(size) => PlatformEvent::Resized(Extent::new(size.width, size.height)),
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            PlatformEvent::ScaleFactorChanged(*scale_factor)
        }
        WindowEvent::CursorMoved { position, .. } => {
            PlatformEvent::CursorMoved { x: position.x, y: position.y }
        }
        WindowEvent::MouseInput { state, button, .. } => PlatformEvent::MouseInput {
            state: match state {
                ElementState::Pressed => ButtonState::Pressed,
                ElementState::Released => ButtonState::Released,
            },
            button: match button {
                WinitMouseButton::Left => MouseButton::Left,
                WinitMouseButton::Right => MouseButton::Right,
                WinitMouseButton::Middle => MouseButton::Middle,
                WinitMouseButton::Other(code) => MouseButton::Other(*code),
                _ => MouseButton::Other(u16::MAX),
            },
        },
        WindowEvent::MouseWheel { delta, .. } => {
            // Windows reports pixel deltas, most other platforms report lines.
            // Both are normalised to lines so the engine sees one unit.
            let (x, y) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => (f64::from(*x), f64::from(*y)),
                winit::event::MouseScrollDelta::PixelDelta(position) => {
                    (position.x / PIXELS_PER_LINE, position.y / PIXELS_PER_LINE)
                }
            };
            PlatformEvent::MouseWheel { delta_x: x, delta_y: y }
        }
        WindowEvent::KeyboardInput { event, .. } => PlatformEvent::KeyInput {
            state: match event.state {
                ElementState::Pressed => ButtonState::Pressed,
                ElementState::Released => ButtonState::Released,
            },
            key: translate_key(&event.logical_key),
            repeat: event.repeat,
        },
        WindowEvent::ModifiersChanged(mods) => {
            let state = mods.state();
            PlatformEvent::ModifiersChanged(Modifiers {
                shift: state.shift_key(),
                control: state.control_key(),
                alt: state.alt_key(),
                meta: state.super_key(),
            })
        }
        WindowEvent::Focused(focused) => PlatformEvent::Focused(*focused),
        WindowEvent::CloseRequested => PlatformEvent::CloseRequested,
        WindowEvent::RedrawRequested => PlatformEvent::RedrawRequested,
        _ => return None,
    })
}

/// Flatten a platform-native key code into a stable `u32`.
///
/// The platform codes are not comparable across operating systems, so they are
/// folded into a single integer space; the engine only needs to distinguish
/// *different* keys.
#[must_use]
pub fn native_key_code(code: &winit::keyboard::NativeKey) -> u32 {
    use winit::keyboard::NativeKey;
    match code {
        NativeKey::Unidentified => 0,
        NativeKey::Android(value) => *value,
        NativeKey::MacOS(value) => u32::from(*value),
        NativeKey::Windows(value) => u32::from(*value),
        NativeKey::Xkb(value) => *value,
        NativeKey::Web(text) => text.chars().next().map_or(0, |c| c as u32),
    }
}

/// Flatten a `winit` logical key into the engine's key taxonomy.
#[must_use]
pub fn translate_key(key: &winit::keyboard::Key) -> PlatformKey {
    use winit::keyboard::{Key as WinitKey, NamedKey};

    match key {
        WinitKey::Character(text) => PlatformKey::Character(text.to_string()),
        WinitKey::Named(named) => match named {
            NamedKey::Enter => PlatformKey::Named("Enter".into()),
            NamedKey::Escape => PlatformKey::Named("Escape".into()),
            NamedKey::Tab => PlatformKey::Named("Tab".into()),
            NamedKey::Backspace => PlatformKey::Named("Backspace".into()),
            NamedKey::Delete => PlatformKey::Named("Delete".into()),
            NamedKey::ArrowLeft => PlatformKey::Named("ArrowLeft".into()),
            NamedKey::ArrowRight => PlatformKey::Named("ArrowRight".into()),
            NamedKey::ArrowUp => PlatformKey::Named("ArrowUp".into()),
            NamedKey::ArrowDown => PlatformKey::Named("ArrowDown".into()),
            NamedKey::Home => PlatformKey::Named("Home".into()),
            NamedKey::End => PlatformKey::Named("End".into()),
            NamedKey::PageUp => PlatformKey::Named("PageUp".into()),
            NamedKey::PageDown => PlatformKey::Named("PageDown".into()),
            NamedKey::Space => PlatformKey::Named("Space".into()),
            other => PlatformKey::Named(format!("{other:?}")),
        },
        WinitKey::Unidentified(code) => PlatformKey::Other(native_key_code(code)),
        // A dead key (e.g. the French `^` accent) produces no character yet.
        WinitKey::Dead(_) => PlatformKey::Named("Dead".into()),
    }
}

/// Apply a platform event to the shared state.
///
/// Returns the events that the engine must react to.
pub fn apply(state: &mut PlatformState, event: &PlatformEvent, now: Instant) -> Vec<PlatformEvent> {
    let mut out = Vec::new();
    match event {
        PlatformEvent::Resized(extent) => {
            match state.resize.on_resized(*extent, now) {
                ResizeAction::None => {}
                ResizeAction::Recreate(new) => out.push(PlatformEvent::RecreateSwapchain(new)),
            }
        }
        PlatformEvent::ScaleFactorChanged(factor) => {
            state.scale_factor = *factor;
        }
        PlatformEvent::CursorMoved { .. }
        | PlatformEvent::PointerRaw { .. }
        | PlatformEvent::MouseInput { .. }
        | PlatformEvent::MouseWheel { .. }
        | PlatformEvent::KeyInput { .. } => {
            // Anything the user did invalidates the pending frame.
            state.latency.on_input(now);
        }
        PlatformEvent::ModifiersChanged(_) => {}
        PlatformEvent::Focused(focused) => {
            state.focused = *focused;
        }
        PlatformEvent::VisibilityChanged(visible) => {
            state.visible = *visible;
            state.pacer.set_visible(*visible);
        }
        PlatformEvent::RedrawRequested => out.push(PlatformEvent::RedrawRequested),
        PlatformEvent::CloseRequested => out.push(PlatformEvent::CloseRequested),
        PlatformEvent::RecreateSwapchain(_) => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pacing::PresentSync;

    #[test]
    fn a_resize_event_produces_a_swapchain_recreation_once_it_settles() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        let out = apply(
            &mut state,
            &PlatformEvent::Resized(Extent::new(1000, 700)),
            now,
        );
        assert!(out.is_empty(), "the first message must not rebuild");
        let out = apply(
            &mut state,
            &PlatformEvent::Resized(Extent::new(1000, 700)),
            now + std::time::Duration::from_millis(30),
        );
        assert_eq!(out, vec![PlatformEvent::RecreateSwapchain(Extent::new(1000, 700))]);
    }

    /// A drag burst ends with one final resize event. If nothing else ever
    /// arrives, that last size must still be committed — the settle window
    /// needs a tick that does not depend on another event.
    #[test]
    fn a_settled_size_is_committed_even_without_a_further_event() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        let out = apply(
            &mut state,
            &PlatformEvent::Resized(Extent::new(1000, 700)),
            now,
        );
        assert!(out.is_empty(), "the first message must not rebuild");

        // No further event arrives; the frame loop polls instead.
        assert_eq!(
            state.poll_resize(now + std::time::Duration::from_millis(5)),
            None,
            "the size has not settled yet"
        );
        assert_eq!(
            state.poll_resize(now + std::time::Duration::from_millis(30)),
            Some(Extent::new(1000, 700)),
            "the settled size must be committed by the frame loop"
        );
        assert_eq!(
            state.poll_resize(now + std::time::Duration::from_millis(60)),
            None,
            "a committed size is not committed twice"
        );
    }

    /// The presentation policy has to follow the device, not a hard-coded
    /// fallback: the window is created before the Vulkan device exists.
    #[test]
    fn the_present_policy_follows_the_device_extensions() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        assert_eq!(state.present.sync(), PresentSync::Immediate);

        state.set_present_extensions(&[
            "VK_KHR_swapchain".to_string(),
            "VK_KHR_present_wait".to_string(),
        ]);
        assert_eq!(state.present.sync(), PresentSync::WaitForPresent);
        assert!(state.present.can_wait_for_present());
    }

    #[test]
    fn input_events_start_the_latency_clock() {        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        apply(
            &mut state,
            &PlatformEvent::CursorMoved { x: 10.0, y: 20.0 },
            now,
        );
        state.latency.on_present(now + std::time::Duration::from_millis(4));
        assert!(state.latency.last().is_some());
    }

    #[test]
    fn modifiers_are_translated_faithfully() {
        use winit::event::{Modifiers, WindowEvent};
        use winit::keyboard::ModifiersState;
        let mods = ModifiersState::SHIFT | ModifiersState::CONTROL;
        let event = WindowEvent::ModifiersChanged(Modifiers::from(mods));
        let Some(PlatformEvent::ModifiersChanged(m)) = translate_window_event(&event) else {
            panic!("expected modifiers");
        };
        assert!(m.shift);
        assert!(m.control);
        assert!(!m.alt);
        assert!(!m.meta);
    }

    #[test]
    fn keys_are_flattened_into_named_and_character_forms() {
        use winit::keyboard::{Key, NamedKey};
        assert_eq!(
            translate_key(&Key::Character("a".into())),
            PlatformKey::Character("a".into())
        );
        assert_eq!(
            translate_key(&Key::Named(NamedKey::Escape)),
            PlatformKey::Named("Escape".into())
        );
    }

    #[test]
    fn unknown_events_are_dropped_rather_than_guessed() {
        use winit::event::WindowEvent;
        assert!(translate_window_event(&WindowEvent::Destroyed).is_none());
    }
}

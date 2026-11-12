//! The concrete `winit` backend.
//!
//! The backend owns the event loop; the window itself is created *inside* the
//! loop, as `winit` requires, so the Vulkan surface is always created against
//! a live window.
//!
//! The engine never sees a `winit` type: everything is translated into
//! [`PlatformEvent`] and applied to a [`PlatformState`].

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::DeviceEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::a11y_bridge::A11yRequest;
use crate::a11y_winit::WinitA11y;
use crate::pacing::PresentWaitPolicy;
use crate::resize::Extent;
use crate::window::{
    apply, translate_ime_event, translate_window_event, PlatformError, PlatformEvent,
    PlatformState, WindowConfig,
};

/// What the engine wants the platform to do next.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Keep running.
    Continue,
    /// Close the window and end the loop.
    Exit,
}

/// The engine side of the platform layer.
///
/// The trait is deliberately small: the platform layer owns the timing and
/// the resize policy, the application owns the scene.
pub trait PlatformApp {
    /// The window exists and the application may create its renderer.
    fn resumed(&mut self, state: &mut PlatformState, window: &Window);

    /// A platform event happened.
    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow;

    /// Draw one frame.
    ///
    /// Called from the event loop's idle point, after every pending event has
    /// been delivered. The application acquires a swapchain image, renders
    /// into it and presents it.
    fn frame(&mut self, state: &mut PlatformState, window: &Window) -> Flow;

    /// The window was hidden; the renderer must stop drawing.
    fn suspended(&mut self, state: &mut PlatformState) {
        let _ = state;
    }

    /// The pending accessibility delta, if the application publishes one.
    ///
    /// Called only while an assistive technology is listening; returning
    /// `None` means "nothing changed".
    fn accessibility_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    /// An assistive technology requested an action.
    ///
    /// The application handles it exactly like the equivalent pointer or
    /// keyboard event.
    fn accessibility_action(
        &mut self,
        state: &mut PlatformState,
        window: &Window,
        request: A11yRequest,
    ) -> Flow {
        let _ = (state, window, request);
        Flow::Continue
    }

    /// A secondary window exists and may create its own surface.
    ///
    /// The default treats it like the primary window, so an application that
    /// only knows one window still runs (it simply builds a second renderer).
    fn window_created(&mut self, state: &mut PlatformState, window: &Window) {
        self.resumed(state, window);
    }

    /// A secondary window is about to close; release its resources.
    fn window_closing(&mut self, window: &Window) {
        let _ = window;
    }

    /// Ask the platform to open another window.
    ///
    /// The event loop polls this after every frame; returning `Some` opens a
    /// window with that configuration and delivers [`Self::window_created`].
    fn take_window_request(&mut self) -> Option<WindowConfig> {
        None
    }
}

/// The platform window: an event loop that will own a window.
pub struct PlatformWindow {
    event_loop: EventLoop<()>,
    config: WindowConfig,
}

impl PlatformWindow {
    /// Create the event loop.
    ///
    /// The window itself is created once the loop runs, because `winit`
    /// requires the window to be created from inside the event loop.
    ///
    /// # Errors
    /// Returns [`PlatformError`] when the event loop cannot be created.
    pub fn new(config: &WindowConfig) -> Result<Self, PlatformError> {
        let event_loop =
            EventLoop::new().map_err(|error| PlatformError::EventLoop(error.to_string()))?;
        Ok(Self {
            event_loop,
            config: config.clone(),
        })
    }

    /// Run the event loop until the application exits.
    ///
    /// # Errors
    /// Returns [`PlatformError`] when the event loop itself fails.
    pub fn run<A: PlatformApp>(self, app: A) -> Result<(), PlatformError> {
        let Self { event_loop, config } = self;
        let mut runner = Runner {
            app,
            config,
            windows: Vec::new(),
            primary: None,
        };
        event_loop
            .run_app(&mut runner)
            .map_err(|error| PlatformError::EventLoop(error.to_string()))
    }
}

/// One live window and everything the platform owns for it.
struct WindowEntry {
    window: Window,
    state: PlatformState,
    a11y: WinitA11y,
}

/// The glue between `winit` and [`PlatformApp`].
///
/// Every window owns its own `PlatformState` (resize tracker, pacer, IME) and
/// its own accessibility adapter, while the application receives the `winit`
/// handle and can key its rendering resources on `window.id()`. Closing a
/// secondary window removes only that entry; closing the primary ends the
/// loop.
struct Runner<A> {
    app: A,
    config: WindowConfig,
    windows: Vec<WindowEntry>,
    primary: Option<WindowId>,
}

impl<A: PlatformApp> Runner<A> {
    /// Drain inbound accessibility actions and publish the outbound delta.
    ///
    /// Accessibility is pumped at the frame boundary, never per event: the
    /// semantic tree is a frame product, and a static frame produces no
    /// update at all.
    fn pump(app: &mut A, a11y: &mut WinitA11y, state: &mut PlatformState, window: &Window) -> Flow {
        for request in a11y.drain_requests() {
            if app.accessibility_action(state, window, request) == Flow::Exit {
                return Flow::Exit;
            }
        }
        a11y.publish_with(|| app.accessibility_update());
        Flow::Continue
    }

    /// Create one window, its accessibility adapter and its platform state.
    ///
    /// The window is created hidden because AccessKit requires the adapter to
    /// exist before the first `show`.
    fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Option<WindowEntry> {
        let mut attributes = Window::default_attributes()
            .with_title(&config.title)
            .with_inner_size(LogicalSize::new(
                f64::from(config.width),
                f64::from(config.height),
            ))
            .with_resizable(config.resizable)
            .with_decorations(config.decorations)
            .with_visible(false);
        if config.min_width > 0 && config.min_height > 0 {
            attributes = attributes.with_min_inner_size(LogicalSize::new(
                f64::from(config.min_width),
                f64::from(config.min_height),
            ));
        }
        let window = event_loop.create_window(attributes).ok()?;
        // The adapter owns the window's UI Automation provider on Windows.
        let a11y = WinitA11y::new(event_loop, &window);
        window.set_visible(config.visible);
        let size = window.inner_size();
        let mut state = PlatformState::new(
            config,
            Extent::new(size.width, size.height),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.scale_factor = window.scale_factor();
        state.visible = window.is_visible().unwrap_or(true);
        Some(WindowEntry {
            window,
            state,
            a11y,
        })
    }

    /// Translate and dispatch one window event.
    fn on_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: &winit::event::WindowEvent,
    ) -> Flow {
        let Some(index) = self
            .windows
            .iter()
            .position(|entry| entry.window.id() == window_id)
        else {
            return Flow::Continue;
        };
        let entry = &mut self.windows[index];
        // The adapter has to see every event before the application does.
        entry.a11y.process_event(&entry.window, event);
        // IME composition is state, not a delta: fold it into the platform
        // state before anything else looks at the event. Without this an East
        // Asian keyboard could only ever type ASCII.
        if let Some(ime) = translate_ime_event(event) {
            entry.state.ime.apply(&ime);
        }
        let Some(platform_event) = translate_window_event(event) else {
            return Flow::Continue;
        };
        let now = Instant::now();
        if matches!(platform_event, PlatformEvent::CloseRequested) {
            self.app
                .event(&mut entry.state, &entry.window, platform_event);
            if self.primary == Some(window_id) {
                return Flow::Exit;
            }
            let entry = &self.windows[index];
            self.app.window_closing(&entry.window);
            self.windows.remove(index);
            return Flow::Continue;
        }
        for out in apply(&mut entry.state, &platform_event, now) {
            if self.app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                return Flow::Exit;
            }
        }
        // Windows runs a *modal* loop while the user drags a window edge
        // (`WM_ENTERSIZEMOVE`): the event loop is suspended and
        // `about_to_wait` is not called until the drag ends. A frame that is
        // only driven from `about_to_wait` therefore freezes for the whole
        // drag, and the compositor stretches the last frame to the new size.
        //
        // Driving the frame right here makes every `WM_SIZE` produce a
        // freshly rendered image at the new resolution.
        //
        // Only a resize is driven this way: `RedrawRequested` carries no
        // size information and is served by `about_to_wait` outside the
        // modal loop — rendering it here as well would draw every frame
        // twice.
        if matches!(platform_event, PlatformEvent::Resized(_)) {
            entry.state.pacer.request_immediate(now);
            if self.app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                return Flow::Exit;
            }
            if Self::pump(
                &mut self.app,
                &mut entry.a11y,
                &mut entry.state,
                &entry.window,
            ) == Flow::Exit
            {
                return Flow::Exit;
            }
        }
        let _ = event_loop;
        Flow::Continue
    }
}

impl<A: PlatformApp> ApplicationHandler<()> for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            return;
        }
        let config = self.config.clone();
        let Some(mut entry) = self.create_window(event_loop, &config) else {
            event_loop.exit();
            return;
        };
        self.primary = Some(entry.window.id());
        self.app.resumed(&mut entry.state, &entry.window);
        self.windows.push(entry);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        if self.on_window_event(event_loop, window_id, &event) == Flow::Exit {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        let DeviceEvent::MouseMotion { delta } = event else {
            return;
        };
        let Some(index) = self.primary.and_then(|id| {
            self.windows
                .iter()
                .position(|entry| entry.window.id() == id)
        }) else {
            return;
        };
        let entry = &mut self.windows[index];
        entry.state.latency.on_input(Instant::now());
        let out = PlatformEvent::PointerRaw {
            dx: delta.0,
            dy: delta.1,
        };
        if self.app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
            event_loop.exit();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        for index in 0..self.windows.len() {
            let entry = &mut self.windows[index];
            let out = PlatformEvent::VisibilityChanged(false);
            for out in apply(&mut entry.state, &out, now) {
                let _ = out;
            }
            self.app.suspended(&mut entry.state);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The window may come back after a suspend/resume cycle.
        if self.windows.is_empty() {
            event_loop.exit();
            return;
        }
        // Draw one frame per window: the application keys its resources on
        // the handle, and the per-window state owns its own cadence.
        for index in 0..self.windows.len() {
            let entry = &mut self.windows[index];
            if self.app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                event_loop.exit();
                return;
            }
            if Self::pump(
                &mut self.app,
                &mut entry.a11y,
                &mut entry.state,
                &entry.window,
            ) == Flow::Exit
            {
                event_loop.exit();
                return;
            }
        }

        // A secondary window requested during the frame is created here, so
        // the application never has to reach into the event loop.
        while let Some(config) = self.app.take_window_request() {
            let Some(mut entry) = self.create_window(event_loop, &config) else {
                break;
            };
            self.app.window_created(&mut entry.state, &entry.window);
            self.windows.push(entry);
        }

        // The frame loop has to drive itself. `ControlFlow::Wait` — the
        // default — parks the loop until the *next input event*, so an
        // application that only draws on demand would show a frozen window.
        // The pacer knows when the next frame is due; the loop sleeps until
        // then and wakes up on its own. The primary window's cadence drives
        // the loop.
        let Some(index) = self.primary.and_then(|id| {
            self.windows
                .iter()
                .position(|entry| entry.window.id() == id)
        }) else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            return;
        };
        let state = &self.windows[index].state;
        let now = Instant::now();
        let flow = match state.pacer.next_wake(now) {
            Some(deadline) => winit::event_loop::ControlFlow::WaitUntil(deadline),
            // Nothing has been presented yet *and* the application still
            // wants frames: poll so the first frame gets through.
            None if state.visible && state.pacer.animating() => {
                winit::event_loop::ControlFlow::Poll
            }
            // The window is hidden, or the page is static and idle: park
            // until an event arrives instead of spinning on an empty loop.
            None => winit::event_loop::ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }
}

/// The raw window handle of a window, for `vkCreateWin32SurfaceKHR`.
///
/// Returns `None` when the platform refuses to hand out a handle, which
/// happens for windows that are not backed by a native surface.
#[must_use]
pub fn raw_window_handle(window: &Window) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    window.window_handle().ok().map(|handle| handle.as_raw())
}

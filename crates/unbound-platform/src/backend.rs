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
            state: None,
            window: None,
        };
        event_loop
            .run_app(&mut runner)
            .map_err(|error| PlatformError::EventLoop(error.to_string()))
    }
}

/// The glue between `winit` and [`PlatformApp`].
struct Runner<A> {
    app: A,
    config: WindowConfig,
    state: Option<PlatformState>,
    window: Option<Window>,
}

impl<A: PlatformApp> Runner<A> {
    /// Translate and dispatch one window event.
    fn on_window_event(&mut self, event: &winit::event::WindowEvent) -> Flow {
        // IME composition is state, not a delta: fold it into the platform
        // state before anything else looks at the event. Without this an East
        // Asian keyboard could only ever type ASCII.
        if let Some(ime) = translate_ime_event(event) {
            if let Some(state) = self.state.as_mut() {
                state.ime.apply(&ime);
            }
        }
        let Some(platform_event) = translate_window_event(event) else {
            return Flow::Continue;
        };
        let Some(state) = self.state.as_mut() else {
            return Flow::Continue;
        };
        let Some(window) = self.window.as_ref() else {
            return Flow::Continue;
        };
        let now = Instant::now();
        if matches!(platform_event, PlatformEvent::CloseRequested) {
            self.app.event(state, window, platform_event);
            return Flow::Exit;
        }
        for out in apply(state, &platform_event, now) {
            if self.app.event(state, window, out) == Flow::Exit {
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
            state.pacer.request_immediate(now);
            if self.app.frame(state, window) == Flow::Exit {
                return Flow::Exit;
            }
        }
        Flow::Continue
    }
}

impl<A: PlatformApp> ApplicationHandler<()> for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let mut attributes = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(
                f64::from(self.config.width),
                f64::from(self.config.height),
            ))
            .with_resizable(self.config.resizable)
            .with_decorations(self.config.decorations)
            .with_visible(self.config.visible);
        if self.config.min_width > 0 && self.config.min_height > 0 {
            attributes = attributes.with_min_inner_size(LogicalSize::new(
                f64::from(self.config.min_width),
                f64::from(self.config.min_height),
            ));
        }
        let Ok(window) = event_loop.create_window(attributes) else {
            event_loop.exit();
            return;
        };
        let size = window.inner_size();
        let mut state = PlatformState::new(
            &self.config,
            Extent::new(size.width, size.height),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.scale_factor = window.scale_factor();
        state.visible = window.is_visible().unwrap_or(true);
        self.app.resumed(&mut state, &window);
        self.state = Some(state);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        if self.on_window_event(&event) == Flow::Exit {
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
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };
        state.latency.on_input(Instant::now());
        let out = PlatformEvent::PointerRaw {
            dx: delta.0,
            dy: delta.1,
        };
        if self.app.event(state, window, out) == Flow::Exit {
            event_loop.exit();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if let Some(state) = self.state.as_mut() {
            let out = PlatformEvent::VisibilityChanged(false);
            for out in apply(state, &out, now) {
                let _ = out;
            }
            self.app.suspended(state);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The window may come back after a suspend/resume cycle.
        if self.state.is_none() {
            event_loop.exit();
            return;
        }
        // Draw one frame. The window is still needed because the application
        // may have to recreate the swapchain when it was resized.
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if self.app.frame(state, window) == Flow::Exit {
            event_loop.exit();
            return;
        }

        // The frame loop has to drive itself. `ControlFlow::Wait` — the
        // default — parks the loop until the *next input event*, so an
        // application that only draws on demand would show a frozen window.
        // The pacer knows when the next frame is due; the loop sleeps until
        // then and wakes up on its own.
        let now = Instant::now();
        let flow = match state.pacer.next_deadline(now) {
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

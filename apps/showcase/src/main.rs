//! The windowed Unbound UI Gallery.
//!
//! Run it with `cargo run -p showcase`. The window is a dogfooded application
//! shell: a custom title bar, a collapsible NavigationView, a searchable
//! command palette, live example cards and a performance HUD.
//!
//! This is the visible counterpart of `apps/gallery` and
//! `unbound-headless`: the gallery prints measurements, the headless verifier
//! checks layout, the showcase shows both.

use std::sync::Arc;
use std::time::Instant;

use showcase::gallery;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;

use unbound_gpu::atlas::{AtlasKind, AtlasSet, AtlasUpload};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, PresentOutcome, Surface, Swapchain};
use unbound_gpu::swapchain::PresentConfig;
use unbound_gpu::{Camera, Renderer};
use unbound_input::{resize_edge, CursorKind, ResizeEdge};
use unbound_math::Vec2;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::window::{
    ButtonState, Modifiers, MouseButton, PlatformEvent, PlatformState, WindowConfig,
};
use unbound_platform::Extent;
use winit::window::Window;

use showcase::shell::WindowCommand;

/// The window title.
const TITLE: &str = "Unbound UI Gallery";

/// How long to wait for a swapchain image, in nanoseconds.
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// The application state.
struct Showcase {
    app: GalleryApp,
    renderer: Option<Renderer>,
    surface: Option<Surface>,
    swapchain: Option<Swapchain>,
    atlases: Option<Arc<AtlasSet>>,
    /// The theme currently uploaded to the GPU.
    uploaded_theme: Option<ThemeMode>,
    /// Physical pointer position.
    pointer: Vec2,
    /// Keyboard modifiers.
    modifiers: Modifiers,
    cursor: CursorKind,
    clock: Instant,
    last_frame: Instant,
    /// The OS scale factor (physical pixels per logical unit).
    os_scale: f32,
    /// The window size in logical units at 100 % app zoom.
    base_logical: Vec2,
    /// The zoom the current window size was requested for.
    last_zoom: f32,
    /// The last physical window size observed, for base-size tracking.
    last_physical: (u32, u32),
    /// The window edge under the pointer, if any.
    resize_edge: Option<ResizeEdge>,
    /// The last title-bar press, for double-click-to-maximise.
    last_title_press: Option<(Instant, Vec2)>,
    /// True when the current press was consumed by the window chrome.
    press_consumed: bool,
    /// True when the driver reported that the swapchain is stale.
    swapchain_dirty: bool,
}

impl Showcase {
    fn new() -> Self {
        Self {
            app: GalleryApp::new(),
            renderer: None,
            surface: None,
            swapchain: None,
            atlases: None,
            uploaded_theme: None,
            pointer: Vec2::new(0.0, 0.0),
            modifiers: Modifiers::default(),
            cursor: CursorKind::Default,
            clock: Instant::now(),
            last_frame: Instant::now(),
            os_scale: 1.0,
            base_logical: Vec2::new(1440.0, 900.0),
            last_zoom: 1.0,
            last_physical: (1440, 900),
            resize_edge: None,
            last_title_press: None,
            press_consumed: false,
            swapchain_dirty: false,
        }
    }

    /// Physical pixels per logical unit.
    fn effective_scale(&self) -> f32 {
        (self.os_scale * self.app.state.zoom()).clamp(0.5, 4.0)
    }

    fn logical(&self, physical: Vec2) -> Vec2 {
        physical / self.effective_scale()
    }

    fn logical_viewport(&self, window: &Window) -> (f32, f32) {
        let size = window.inner_size();
        let scale = self.effective_scale();
        (
            size.width.max(1) as f32 / scale,
            size.height.max(1) as f32 / scale,
        )
    }

    /// Track the OS scale, the base logical size and the zoomed window size.
    fn sync_window_size(&mut self, window: &Window) {
        let os_scale = window.scale_factor() as f32;
        if os_scale.is_finite() && os_scale > 0.0 {
            self.os_scale = os_scale;
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let current = Vec2::new(size.width as f32, size.height as f32);
        let previous = Vec2::new(
            self.last_physical.0.max(1) as f32,
            self.last_physical.1.max(1) as f32,
        );
        let zoom = self.app.state.zoom();
        let old_scale = (self.os_scale * self.last_zoom).max(0.25);
        if (zoom - self.last_zoom).abs() > 1e-6 {
            // The user picked a new zoom: keep the logical size they were
            // looking at and grow/shrink the physical window to match.
            let reference = if (current - previous).length() > 0.5 {
                current
            } else {
                previous
            };
            self.base_logical = reference / old_scale;
            let target = zoomed_physical_size(
                self.base_logical,
                self.os_scale * zoom,
                window.current_monitor().map(|monitor| {
                    let size = monitor.size();
                    Vec2::new(size.width as f32, size.height as f32)
                }),
            );
            if !window.is_maximized() && window.is_minimized() != Some(true) {
                let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(
                    target.x as u32,
                    target.y as u32,
                ));
            }
            self.last_zoom = zoom;
        } else if (current - previous).length() > 0.5 {
            // A manual resize: remember it as the base size for the next zoom.
            self.base_logical = current / (self.os_scale * zoom).max(0.25);
        }
        self.last_physical = (size.width, size.height);
    }

    fn resize_edge_at(&self, window: &Window) -> Option<ResizeEdge> {
        if window.is_decorated() || window.is_maximized() {
            return None;
        }
        let size = window.inner_size();
        let scale = self.os_scale.max(0.5);
        resize_edge(
            self.pointer,
            Vec2::new(size.width as f32, size.height as f32),
            6.0 * scale,
            16.0 * scale,
        )
    }

    fn handle_window_command(&mut self, window: &Window, command: WindowCommand) -> Flow {
        match command {
            WindowCommand::Minimize => window.set_minimized(true),
            WindowCommand::ToggleMaximize => {
                window.set_maximized(!window.is_maximized());
            }
            WindowCommand::Close => return Flow::Exit,
        }
        Flow::Continue
    }

    fn upload_glyphs(&mut self) {
        if self.atlases.is_none() || self.renderer.is_none() {
            return;
        }
        let updates = self.app.labels.cache.take_updates();
        if updates.is_empty() {
            return;
        }
        let Some(atlases) = &self.atlases else {
            return;
        };
        let Some(renderer) = &self.renderer else {
            return;
        };
        let uploads: Vec<AtlasUpload> = updates
            .into_iter()
            .map(|update| {
                let kind = match update.target {
                    unbound_text::AtlasTarget::Coverage => AtlasKind::R8,
                    unbound_text::AtlasTarget::Color => AtlasKind::Rgba8,
                };
                AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                }
            })
            .collect();
        // One batch: a single staging buffer, a single command buffer and a
        // single queue wait for the whole page of text. Uploading glyph by
        // glyph stalled the pipeline once per character.
        let _ = atlases.upload_batch(&renderer.gpu, &uploads);
    }

    fn write_theme_if_needed(&mut self) {
        if self.uploaded_theme == Some(self.app.state.theme) {
            return;
        }
        if let Some(renderer) = &mut self.renderer {
            let _ = renderer.write_theme(&gallery::theme_table(self.app.state.theme.is_dark()));
            self.uploaded_theme = Some(self.app.state.theme);
        }
    }

    fn rebuild(&mut self, window: &Window) {
        self.app.set_effective_scale(self.effective_scale());
        let viewport = self.logical_viewport(window);
        self.app.rebuild(viewport);
        self.upload_glyphs();
        self.write_theme_if_needed();
    }

    /// Rebuild the scene when anything observable changed.
    ///
    /// The scene's upload generation is bumped by the rebuild itself, so
    /// the renderer always sees a fresh generation as soon as the state
    /// moved. The redraw request is what makes the change visible in the
    /// same frame: without it the window would keep showing the previous
    /// picture until something else happened to trigger a frame.
    fn sync_scene(&mut self, window: &Window) -> bool {
        let viewport = self.logical_viewport(window);
        let changed = self.app.sync(viewport);
        if changed {
            self.upload_glyphs();
            window.request_redraw();
        }
        changed
    }

    fn apply_cursor(&mut self, window: &Window) {
        let kind = self
            .resize_edge
            .map_or_else(|| self.app.cursor(), ResizeEdge::cursor);
        if kind != self.cursor {
            self.cursor = kind;
            window.set_cursor(cursor_icon(kind));
        }
    }

    /// Rebuild the swapchain for a new surface size.
    ///
    /// The old chain is handed to the driver through `oldSwapchain`, so the
    /// presentation engine can hand its resources over instead of tearing
    /// the surface down — that is what removes the flicker on a resize.
    fn recreate_swapchain(&mut self, width: u32, height: u32) -> bool {
        let Some(renderer) = &mut self.renderer else {
            return false;
        };
        let Some(surface) = &self.surface else {
            return false;
        };
        let present =
            PresentConfig::from_capabilities(&renderer.gpu.enabled_device_extensions, false);
        let Some(swapchain) = &mut self.swapchain else {
            return false;
        };
        if let Err(error) = swapchain.recreate(&renderer.gpu, surface, width, height, &present) {
            eprintln!(" could not rebuild the swapchain: {error}");
            return false;
        }
        if let Err(error) = renderer.resize(width, height) {
            eprintln!("could not resize the renderer: {error}");
            return false;
        }
        true
    }
}

/// The physical window size for a logical layout at `scale`, clamped to 95 %
/// of the monitor so a 200 % zoom never opens a window larger than the screen.
fn zoomed_physical_size(base_logical: Vec2, scale: f32, monitor: Option<Vec2>) -> Vec2 {
    let mut target = base_logical * scale.max(0.25);
    if let Some(monitor) = monitor {
        let limit = monitor * 0.95;
        if target.x > limit.x || target.y > limit.y {
            let fit = (limit.x / target.x).min(limit.y / target.y);
            target = target * fit;
        }
    }
    Vec2::new(target.x.round().max(320.0), target.y.round().max(240.0))
}

/// Map an engine resize edge to the OS drag direction.
fn resize_direction(edge: ResizeEdge) -> winit::window::ResizeDirection {
    use winit::window::ResizeDirection as Direction;
    match edge {
        ResizeEdge::North => Direction::North,
        ResizeEdge::South => Direction::South,
        ResizeEdge::West => Direction::West,
        ResizeEdge::East => Direction::East,
        ResizeEdge::NorthWest => Direction::NorthWest,
        ResizeEdge::NorthEast => Direction::NorthEast,
        ResizeEdge::SouthWest => Direction::SouthWest,
        ResizeEdge::SouthEast => Direction::SouthEast,
    }
}

/// Map an engine cursor to a winit one.
fn cursor_icon(kind: CursorKind) -> winit::window::CursorIcon {
    use winit::window::CursorIcon as Icon;
    match kind {
        CursorKind::Pointer => Icon::Pointer,
        CursorKind::Text => Icon::Text,
        CursorKind::Grab => Icon::Grab,
        CursorKind::Grabbing => Icon::Grabbing,
        CursorKind::Crosshair => Icon::Crosshair,
        CursorKind::Move => Icon::Move,
        CursorKind::ResizeNorthSouth => Icon::NsResize,
        CursorKind::ResizeEastWest => Icon::EwResize,
        CursorKind::ResizeNorthWestSouthEast => Icon::NwseResize,
        CursorKind::ResizeNorthEastSouthWest => Icon::NeswResize,
        CursorKind::NotAllowed => Icon::NotAllowed,
        CursorKind::Default | CursorKind::Custom(_) => Icon::Default,
    }
}

impl PlatformApp for Showcase {
    fn resumed(&mut self, state: &mut PlatformState, window: &Window) {
        let config = GpuConfig {
            instance_extensions: vec![
                "VK_KHR_surface".to_string(),
                "VK_KHR_win32_surface".to_string(),
            ],
            device_extensions: vec![
                "VK_KHR_swapchain".to_string(),
                "VK_EXT_swapchain_maintenance1".to_string(),
                "VK_KHR_present_wait".to_string(),
            ],
            ..Default::default()
        };
        let Ok(gpu) = Gpu::new(config) else {
            eprintln!("could not create a Vulkan device");
            return;
        };

        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match window.window_handle().ok().map(|h| h.as_raw()) {
                Some(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => {
                    eprintln!("the window has no native handle");
                    return;
                }
            }
        };
        let Ok(surface) = Surface::new_win32(&gpu, hwnd) else {
            eprintln!("could not create a Vulkan surface");
            return;
        };

        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false);
        let Ok(swapchain) = Swapchain::new(&gpu, &surface, width, height, &present) else {
            eprintln!("could not create a swapchain");
            return;
        };

        let device_extensions = gpu.enabled_device_extensions.clone();
        let mut renderer = match Renderer::new(gpu, width, height) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("could not create the renderer: {error}");
                return;
            }
        };
        let atlases = match AtlasSet::new(&renderer.gpu, 64) {
            Ok(atlases) => Arc::new(atlases),
            Err(error) => {
                eprintln!("could not create the atlases: {error}");
                return;
            }
        };
        if let Err(error) = renderer.install_atlases(atlases.clone()) {
            eprintln!("could not bind the atlases: {error}");
            return;
        }

        self.atlases = Some(atlases);
        self.swapchain = Some(swapchain);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        state.set_present_extensions(&device_extensions);

        self.os_scale = window.scale_factor() as f32;
        self.last_zoom = self.app.state.zoom();
        let size = window.inner_size();
        self.last_physical = (size.width.max(1), size.height.max(1));
        self.base_logical = Vec2::new(
            size.width.max(1) as f32 / self.effective_scale(),
            size.height.max(1) as f32 / self.effective_scale(),
        );
        self.rebuild(window);
        self.last_frame = Instant::now();
    }

    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow {
        self.sync_window_size(window);
        self.app.set_effective_scale(self.effective_scale());
        let mut flow = Flow::Continue;
        match event {
            PlatformEvent::ScaleFactorChanged(factor) => {
                self.os_scale = factor as f32;
                self.rebuild(window);
            }
            PlatformEvent::RecreateSwapchain(extent) => {
                // The platform layer settled a resize: the swapchain has to
                // be rebuilt for the new size.
                self.swapchain_dirty = true;
                let _ = extent;
            }
            PlatformEvent::KeyInput {
                state: ButtonState::Pressed,
                key,
                repeat: false,
            } => {
                self.app.key(&key, self.modifiers);
                if let Some(command) = self.app.take_window_command() {
                    flow = self.handle_window_command(window, command);
                }
            }
            PlatformEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
            }
            PlatformEvent::MouseInput {
                state,
                button: MouseButton::Left,
            } => {
                let position = self.logical(self.pointer);
                let now = self.clock.elapsed().as_secs_f64();
                if state == ButtonState::Pressed {
                    // The window chrome owns its edges and its title bar: the
                    // UI underneath must not also see the press.
                    self.resize_edge = self.resize_edge_at(window);
                    if let Some(edge) = self.resize_edge {
                        self.press_consumed = true;
                        let _ = window.drag_resize_window(resize_direction(edge));
                    } else {
                        self.app.pointer_move(position);
                        if self.app.is_titlebar_drag(position) {
                            self.press_consumed = true;
                            let double = self.last_title_press.is_some_and(|(when, at)| {
                                Instant::now().duration_since(when)
                                    < std::time::Duration::from_millis(400)
                                    && (at - position).length() < 6.0
                            });
                            self.last_title_press = Some((Instant::now(), position));
                            if double {
                                window.set_maximized(!window.is_maximized());
                            } else {
                                let _ = window.drag_window();
                            }
                        } else {
                            self.press_consumed = false;
                            self.app.pointer_down(position, now);
                            if let Some(command) = self.app.take_window_command() {
                                flow = self.handle_window_command(window, command);
                            }
                        }
                    }
                } else if self.press_consumed {
                    self.press_consumed = false;
                } else {
                    self.app.pointer_up(position, now);
                    if let Some(command) = self.app.take_window_command() {
                        flow = self.handle_window_command(window, command);
                    }
                }
            }
            PlatformEvent::CursorMoved { x, y } => {
                self.pointer = Vec2::new(x as f32, y as f32);
                self.resize_edge = self.resize_edge_at(window);
                if self.resize_edge.is_none() {
                    self.app.pointer_move(self.logical(self.pointer));
                }
            }
            PlatformEvent::MouseWheel { delta_y, .. } => {
                if self.modifiers.control {
                    let steps = showcase::state::ZOOM_STEPS.len();
                    let current = self.app.state.zoom_step.min(steps - 1);
                    self.app.state.zoom_step = if delta_y > 0.0 {
                        (current + 1).min(steps - 1)
                    } else {
                        current.saturating_sub(1)
                    };
                    self.app.state.bump();
                } else {
                    self.app.wheel(delta_y as f32);
                }
            }
            PlatformEvent::Focused(false) => {
                // The window lost the focus: whatever the pointer was doing
                // is over. The capture, the scrollbar grab and the demo
                // drags have to go, or the shell keeps a ghost drag alive
                // until the user happens to click somewhere else.
                self.app.cancel_interactions();
            }
            PlatformEvent::RedrawRequested => {
                // The OS asks for a frame: draw it right away instead of
                // waiting for the pacer's next deadline.
                state.pacer.request_immediate(Instant::now());
            }
            PlatformEvent::CloseRequested => return Flow::Exit,
            _ => {}
        }
        if flow == Flow::Exit {
            return Flow::Exit;
        }
        // Re-read the effective scale: a zoom click in this very event changed
        // it after the pre-handler snapshot.
        self.app.set_effective_scale(self.effective_scale());
        // The scene is rebuilt here, not in the frame loop: the state the
        // event changed has to reach the screen in the very same frame.
        self.sync_scene(window);
        self.write_theme_if_needed();
        self.apply_cursor(window);
        Flow::Continue
    }

    fn frame(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        self.sync_window_size(window);
        self.app.set_effective_scale(self.effective_scale());
        self.app.set_window_maximized(window.is_maximized());
        let size = window.inner_size();
        let (window_width, window_height) = (size.width.max(1), size.height.max(1));

        // A resize that settled, or a swapchain the driver already rejected,
        // both mean the same thing: rebuild the chain for the new size.
        //
        // The third condition is the important one: as long as the chain and
        // the window disagree, the frame would be drawn at one resolution and
        // stretched to the other. Comparing the two directly makes the
        // invariant "chain size == window size" hold on *every* frame, no
        // matter which path noticed the resize.
        let settled = state.poll_resize(Instant::now());
        // A minimised window reports a zero extent: there is nothing to
        // rebuild for, and a one-pixel chain would be nonsense.
        let size_mismatch = size.width > 0
            && size.height > 0
            && self
                .swapchain
                .as_ref()
                .is_some_and(|chain| chain.extent() != (window_width, window_height));
        if settled.is_some() || self.swapchain_dirty || size_mismatch {
            self.swapchain_dirty = false;
            if self.recreate_swapchain(window_width, window_height) {
                self.rebuild(window);
            }
        }

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.1);
        self.last_frame = now;
        if dt > 0.0 {
            self.app.state.demo.fps = 1.0 / dt;
            self.app.state.demo.frame_ms = dt * 1000.0;
        }
        self.app.tick(dt);
        self.sync_scene(window);
        self.write_theme_if_needed();
        self.apply_cursor(window);

        let scale = self.effective_scale();
        let Some(renderer) = &mut self.renderer else {
            return Flow::Continue;
        };
        let Some(swapchain) = &mut self.swapchain else {
            return Flow::Continue;
        };
        let (width, height) = swapchain.extent();
        if !state.pacer.should_render(Instant::now()) {
            window.request_redraw();
            return Flow::Continue;
        }
        if let Err(error) = renderer.wait_for_previous_frame() {
            eprintln!("waiting for the previous frame failed: {error}");
            return Flow::Continue;
        }
        // The driver rejects the image when the surface changed under us.
        // That is not a failure: the swapchain is rebuilt for the new size
        // on the next frame.
        let image = match swapchain.acquire_outcome(ACQUIRE_TIMEOUT_NS) {
            Ok(AcquireOutcome::Ready { image, suboptimal }) => {
                if suboptimal {
                    self.swapchain_dirty = true;
                }
                image
            }
            Ok(AcquireOutcome::OutOfDate) => {
                self.swapchain_dirty = true;
                state
                    .resize
                    .mark_out_of_date(Extent::new(window_width, window_height));
                return Flow::Continue;
            }
            Err(error) => {
                eprintln!("could not acquire a swapchain image: {error}");
                return Flow::Continue;
            }
        };

        // The app lays out in logical units (physical / (OS scale × zoom)); the
        // UI camera scales those units back up, so a 150 % layout maps exactly
        // onto the physical viewport at any DPI.
        self.app.scene.camera = Camera::ui((width as f32, height as f32), scale);
        let scene = self.app.scene.upload(self.app.generation());
        if let Err(error) = renderer.render_into_swapchain(&scene, &image, width, height) {
            eprintln!("render failed: {error}");
            return Flow::Continue;
        }
        // The render-finished semaphore belongs to *this* swapchain image, so
        // the presentation engine never waits on a semaphore another image
        // still owns.
        let finished = image.render_finished;
        // A present that came back "recreate" means the surface moved under
        // the swapchain: the frame was shown, but the chain is stale now.
        match swapchain.present_outcome(&renderer.gpu, image.index, finished) {
            Ok(PresentOutcome::Optimal) => {}
            Ok(PresentOutcome::Recreate) => {
                self.swapchain_dirty = true;
                state
                    .resize
                    .mark_out_of_date(Extent::new(window_width, window_height));
            }
            Err(error) => {
                eprintln!("present failed: {error}");
            }
        }
        state.pacer.on_present(Instant::now());
        window.request_redraw();
        state.latency.on_present(Instant::now());
        Flow::Continue
    }
}

/// Render the gallery offscreen and write it to a PNG.
fn render_to_png(
    path: &str,
    width: u32,
    height: u32,
    dark: bool,
    zoom_step: usize,
    show_hud: bool,
) -> Result<(), String> {
    let gpu = Gpu::new(GpuConfig::default()).map_err(|e| e.to_string())?;
    let mut renderer = Renderer::new(gpu, width, height).map_err(|e| e.to_string())?;
    let atlases = Arc::new(AtlasSet::new(&renderer.gpu, 64).map_err(|e| e.to_string())?);
    renderer
        .install_atlases(atlases.clone())
        .map_err(|e| e.to_string())?;

    let mut app = GalleryApp::new();
    app.state.theme = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    app.state.zoom_step = zoom_step.min(showcase::state::ZOOM_STEPS.len() - 1);
    app.state.show_hud = show_hud;
    let zoom = app.state.zoom();
    app.set_effective_scale(zoom);
    app.rebuild((width as f32 / zoom, height as f32 / zoom));
    app.scene.camera = Camera::ui((width as f32, height as f32), zoom);

    for update in app.labels.cache.take_updates() {
        let kind = match update.target {
            unbound_text::AtlasTarget::Coverage => AtlasKind::R8,
            unbound_text::AtlasTarget::Color => AtlasKind::Rgba8,
        };
        atlases
            .upload(
                &renderer.gpu,
                &AtlasUpload {
                    kind,
                    layer: update.layer,
                    x: update.x,
                    y: update.y,
                    width: update.width,
                    height: update.height,
                    data: update.data,
                },
            )
            .map_err(|e| e.to_string())?;
    }
    renderer
        .write_theme(&gallery::theme_table(dark))
        .map_err(|e| e.to_string())?;
    renderer
        .render(&app.scene.upload(1))
        .map_err(|e| e.to_string())?;
    let pixels = renderer.read_pixels().map_err(|e| e.to_string())?;
    renderer.destroy();
    write_png(path, width, height, &pixels)
}

/// Write RGBA pixels to a PNG file, creating the parent directory if needed.
fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let mut rgba = rgba.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--shot") {
        let dark = args.iter().any(|a| a == "--dark");
        let zoom_step = args
            .windows(2)
            .find(|w| w[0] == "--zoom")
            .and_then(|w| w[1].parse::<usize>().ok())
            .map_or(0, |percent| match percent {
                125 => 1,
                150 => 2,
                200 => 3,
                _ => 0,
            });
        let path = args
            .windows(2)
            .find(|w| w[0] == "--shot" && !w[1].starts_with("--"))
            .map_or_else(
                || {
                    let name = if dark {
                        "gallery-dark.png"
                    } else {
                        "gallery-light.png"
                    };
                    format!("target/showcase/{name}")
                },
                |w| w[1].clone(),
            );
        let width = args
            .windows(2)
            .find(|w| w[0] == "--width")
            .and_then(|w| w[1].parse::<u32>().ok())
            .unwrap_or(1440);
        let height = args
            .windows(2)
            .find(|w| w[0] == "--height")
            .and_then(|w| w[1].parse::<u32>().ok())
            .unwrap_or(900);
        match render_to_png(
            &path,
            width,
            height,
            dark,
            zoom_step,
            args.iter().any(|a| a == "--hud"),
        ) {
            Ok(()) => println!("wrote {path}"),
            Err(error) => eprintln!("could not render: {error}"),
        }
        return;
    }

    let config = WindowConfig {
        title: TITLE.to_string(),
        width: 1440,
        height: 900,
        resizable: true,
        decorations: false,
        min_width: 960,
        min_height: 600,
        visible: true,
        ..Default::default()
    };
    let Ok(window) = PlatformWindow::new(&config) else {
        eprintln!("could not create the window");
        return;
    };
    if let Err(error) = window.run(Showcase::new()) {
        eprintln!("the event loop failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use showcase::pages::PageId;

    #[test]
    fn zooming_keeps_the_layout_and_grows_the_physical_window() {
        let base = Vec2::new(1440.0, 900.0);
        assert_eq!(
            zoomed_physical_size(base, 1.0, Some(Vec2::new(3840.0, 2160.0))),
            Vec2::new(1440.0, 900.0)
        );
        assert_eq!(
            zoomed_physical_size(base, 1.5, Some(Vec2::new(3840.0, 2160.0))),
            Vec2::new(2160.0, 1350.0)
        );
        // A 200 % window on a 1080p panel is clamped to the screen.
        let fitted = zoomed_physical_size(base, 2.0, Some(Vec2::new(1920.0, 1080.0)));
        assert!(fitted.x <= 1920.0 * 0.95 + 0.5);
        assert!(fitted.y <= 1080.0 * 0.95 + 0.5);
        assert!(fitted.x > 900.0 && fitted.y > 500.0);
    }

    #[test]
    fn the_gallery_starts_on_home_with_a_full_catalogue() {
        let app = GalleryApp::new();
        assert_eq!(app.state.active_page, PageId::Home);
        assert_eq!(PageId::ALL.len(), 21);
        assert!(!PageId::Home.examples().is_empty());
        assert!(PageId::Telemetry.examples().len() >= 2);
    }

    #[test]
    fn every_page_has_examples_and_unique_metadata() {
        for page in PageId::ALL {
            let examples = page.examples();
            assert!(!examples.is_empty(), "{page:?} has no examples");
            for example in examples {
                assert!(!example.code.is_empty());
                assert!(!example.a11y.is_empty());
                assert!(example.stage_height >= 200.0);
            }
        }
    }
}

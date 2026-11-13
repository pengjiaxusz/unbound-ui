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

use showcase::frame_plan::{frame_size, needs_rebuild, upload_plan};
use showcase::runtime_app::GalleryRuntime;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::theme;

use unbound_app::{AppKey, AppRuntime, DirtyFlags, Modifiers as AppModifiers};
use unbound_gpu::atlas::{AtlasKind, AtlasSet, AtlasUpload};
use unbound_gpu::data::PassLayer;
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, PresentOutcome, Surface, Swapchain};
use unbound_gpu::swapchain::PresentConfig;
use unbound_gpu::{Camera, Renderer, SceneUpload};
use unbound_input::{resize_edge, CursorKind, PointerButton, ResizeEdge};
use unbound_math::Vec2;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::window::{
    ButtonState, Modifiers, MouseButton, PlatformEvent, PlatformKey, PlatformState, WindowConfig,
};
use unbound_platform::Extent;
use unbound_theme::{ThemeKind, ThemeState};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::Window;

use showcase::shell::WindowCommand;

/// The window title.
const TITLE: &str = "Unbound UI Gallery";

/// How long to wait for a swapchain image, in nanoseconds.
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// One secondary window: its own surface, swapchain and upload generation.
///
/// The device, renderer, pipelines and glyph atlases are shared with the
/// primary window; only the presentation chain is per-window.
struct ExtraWindow {
    id: winit::window::WindowId,
    surface: Surface,
    swapchain: Swapchain,
    swapchain_dirty: bool,
    /// The size the swapchain was last requested at.
    ///
    /// The chain's actual extent is the driver's answer and may differ from
    /// the request; the two are never compared with each other.
    committed_request: (u32, u32),
}

/// The application state.
struct Showcase {
    /// The generic runtime: it reduces every event, owns the router / focus
    /// policy and diffs each frame against the previous one.
    app: AppRuntime<GalleryRuntime>,
    renderer: Option<Renderer>,
    surface: Option<Surface>,
    swapchain: Option<Swapchain>,
    atlases: Option<Arc<AtlasSet>>,
    /// The theme currently uploaded to the GPU.
    uploaded_theme: Option<ThemeMode>,
    /// The upload generation of the last `SceneUpload`.
    ///
    /// The renderer skips the upload when the generation does not move; the
    /// host advances it exactly when the runtime reported a whole-scene
    /// change.
    scene_generation: u64,
    /// True when the previous frame was rebuilt but never uploaded.
    ///
    /// The runtime moves its diff base on every rebuild, whether the frame
    /// reaches the GPU or not. A dropped frame therefore leaves the pool
    /// behind the base, and the next upload has to be a full one.
    frame_dropped: bool,
    /// The size the swapchain was last requested at.
    ///
    /// The chain's *actual* extent is the driver's answer and may differ from
    /// the request; this remembers the request so the two are never compared
    /// with each other.
    committed_request: (u32, u32),
    /// The live theme: the engine owns the colour table, the app only asks
    /// for a different palette.
    theme: ThemeState,
    /// Physical pointer position.
    pointer: Vec2,
    /// Keyboard modifiers.
    modifiers: Modifiers,
    cursor: CursorKind,
    last_frame: Instant,
    /// The OS scale factor (physical pixels per logical unit).
    os_scale: f32,
    /// The window edge under the pointer, if any.
    resize_edge: Option<ResizeEdge>,
    /// Whether the IME is currently allowed, so the call is only made on
    /// a transition instead of every frame.
    ime_allowed: bool,
    /// The last title-bar press, for double-click-to-maximise.
    last_title_press: Option<(Instant, Vec2)>,
    /// True when the current press was consumed by the window chrome.
    press_consumed: bool,
    /// True when the driver reported that the swapchain is stale.
    swapchain_dirty: bool,
    /// The primary window's identity, so secondary windows can be told apart.
    primary: Option<winit::window::WindowId>,
    /// Secondary windows and their presentation chains.
    extras: Vec<ExtraWindow>,
    /// Set by `Ctrl+N`; the platform opens the window at the next frame.
    new_window_requested: bool,
}

impl Showcase {
    fn new() -> Self {
        Self {
            app: {
                let mut app = GalleryRuntime::new();
                // The runtime diffs the frame, so the shell must not scan the
                // primitive list a second time for the same answer.
                app.set_external_diff(true);
                AppRuntime::new(app, Vec2::new(1440.0, 900.0))
            },
            renderer: None,
            surface: None,
            swapchain: None,
            atlases: None,
            uploaded_theme: None,
            scene_generation: 0,
            frame_dropped: false,
            committed_request: (0, 0),
            theme: ThemeState::new(theme::theme_set(), ThemeKind::Light),
            pointer: Vec2::new(0.0, 0.0),
            modifiers: Modifiers::default(),
            cursor: CursorKind::Default,
            last_frame: Instant::now(),
            os_scale: 1.0,
            resize_edge: None,
            ime_allowed: false,
            last_title_press: None,
            press_consumed: false,
            swapchain_dirty: false,
            primary: None,
            extras: Vec::new(),
            new_window_requested: false,
        }
    }

    /// Physical pixels per logical unit.
    fn effective_scale(&self) -> f32 {
        (self.os_scale * self.shell().state.zoom()).clamp(0.5, 4.0)
    }

    /// The dogfooded shell the runtime drives.
    fn shell(&self) -> &GalleryApp {
        self.app.application().shell()
    }

    /// The shell, mutably.
    fn shell_mut(&mut self) -> &mut GalleryApp {
        self.app.application_mut().shell_mut()
    }

    fn logical(&self, physical: Vec2) -> Vec2 {
        physical / self.effective_scale()
    }

    /// Track the OS scale factor only.
    ///
    /// Changing the app zoom must never resize the physical window: the UI
    /// scales in place because the logical viewport is recomputed from the
    /// committed extent (`physical_size / (os_scale × app_zoom)`), and the page
    /// reflows for the smaller logical viewport.
    fn sync_window_size(&mut self, window: &Window) {
        let os_scale = window.scale_factor() as f32;
        if os_scale.is_finite() && os_scale > 0.0 {
            self.os_scale = os_scale;
        }
    }

    fn resize_edge_at(&self, window: &Window) -> Option<ResizeEdge> {
        if window.is_decorated() || window.is_maximized() {
            return None;
        }
        let size = window.inner_size();
        let scale = self.os_scale.max(0.5);
        // The window buttons live in the top-right 3 × 46 px. Testing the
        // resize border first made that corner start a window drag instead of
        // a click, so the exclusion has to happen before the edge test.
        let logical_w = size.width as f32 / self.effective_scale();
        if self.pointer.y < 48.0 && self.pointer.x > logical_w - 3.0 * 46.0 {
            return None;
        }
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
        let updates = self.shell_mut().labels.cache.take_updates();
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
            .iter()
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
                    data: update.data.clone(),
                }
            })
            .collect();
        // One batch: a single staging buffer, a single command buffer and a
        // single queue wait for the whole page of text. Uploading glyph by
        // glyph stalled the pipeline once per character.
        //
        // A failed batch must not lose the glyphs: the cache believes they
        // are in the atlas, so the text would sample texels that were never
        // written — blank or garbled glyphs, forever. The updates therefore
        // go back into the queue and the next frame tries again.
        if let Err(error) = atlases.upload_batch(&renderer.gpu, &uploads) {
            eprintln!("could not upload the glyphs: {error}");
            self.shell_mut().labels.cache.restore_updates(updates);
        }
    }

    /// Upload the RGBA8 rectangles the asset hub decoded this frame.
    fn upload_asset_textures(&mut self, uploads: &[AtlasUpload]) {
        if uploads.is_empty() {
            return;
        }
        let (Some(atlases), Some(renderer)) = (&self.atlases, &self.renderer) else {
            return;
        };
        if let Err(error) = atlases.upload_batch(&renderer.gpu, uploads) {
            eprintln!("could not upload the image assets: {error}");
        }
    }

    fn write_theme_if_needed(&mut self) {
        let mode = self.shell().state.theme;
        if self.uploaded_theme == Some(mode) {
            return;
        }
        let dark = mode.is_dark();
        if let Some(renderer) = &mut self.renderer {
            // Switching the theme only rewrites the 4 KiB colour table: no
            // layout, no instance re-upload.
            let _ = self.theme.switch(theme::kind(dark));
            let _ = renderer.write_theme_lut(self.theme.lut());
            self.uploaded_theme = Some(mode);
        }
    }

    /// Rebuild the scene through the runtime, diff it and upload the glyphs
    /// the rebuild produced.
    ///
    /// This replaces the hand-written `sync` / `dirty_ranges` / `generation`
    /// bookkeeping: the runtime owns the frame pipeline now, and the host only
    /// turns the returned report into an upload plan.
    fn advance_scene(&mut self, dt: f32) -> unbound_app::FrameReport {
        let scale = self.effective_scale();
        self.shell_mut().set_effective_scale(scale);
        let report = self.app.frame(dt);
        self.upload_asset_textures(&report.atlas_uploads);
        // The glyphs are uploaded whenever there are any. A frame whose scene
        // did not change can still carry fresh glyphs — a re-rasterised glyph
        // lands in a different atlas slot, so the primitive changed too, but
        // the upload must not depend on that coincidence.
        self.upload_glyphs();
        let shell = self.shell_mut();
        shell.state.demo.upload_bytes = report.bytes;
        shell.state.demo.primitive_count = report.primitives as u32;
        report
    }

    fn apply_cursor(&mut self, window: &Window) {
        let kind = self
            .resize_edge
            .map_or_else(|| self.app.context().cursor, ResizeEdge::cursor);
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
    fn recreate_swapchain(&mut self, window: &Window, width: u32, height: u32) -> bool {
        // Read the limit before borrowing the renderer: both go through
        // `self`, and the shell read would conflict with the mutable borrow.
        let limit = self.shell().state.effective_fps_limit();
        let Some(renderer) = &mut self.renderer else {
            return false;
        };
        let Some(surface) = &self.surface else {
            return false;
        };
        // A limit above the display's physical refresh rate cannot be reached
        // through FIFO — the compositor would quantise every frame down to the
        // VBlank (that is why "120" used to run at 60 FPS on a 60 Hz panel).
        // Any such limit hands the cadence to the software pacer and presents
        // without vsync; a limit at or below the refresh rate keeps the
        // tear-free path.
        let refresh_hz = window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .map_or(60.0, |millihertz| millihertz as f32 / 1000.0);
        let allow_tearing = limit == 0 || (limit as f32) > refresh_hz + 0.5;
        let present = PresentConfig::from_capabilities(
            &renderer.gpu.enabled_device_extensions,
            allow_tearing,
        );
        let Some(swapchain) = &mut self.swapchain else {
            return false;
        };
        if let Err(error) = swapchain.recreate(&renderer.gpu, surface, width, height, &present) {
            eprintln!(" could not rebuild the swapchain: {error}");
            return false;
        }
        // The chain now answers for this request. Its *actual* extent may
        // still differ from it — the driver clamps it to its own limits — and
        // that difference must never be mistaken for a new request.
        self.committed_request = (width, height);
        // The windowed path borrows the swapchain image as its colour
        // attachment, so only the depth buffer has to follow the new size.
        // `ensure_depth_size` grows in 256 px steps and never shrinks, which
        // keeps a 1 px drag step from reallocating anything.
        let (actual_w, actual_h) = swapchain.extent();
        if let Err(error) = renderer.ensure_depth_size(actual_w, actual_h) {
            eprintln!("could not resize the depth buffer: {error}");
            return false;
        }
        true
    }

    /// Bring the layout viewport in line with the committed chain extent.
    ///
    /// The layout follows the *presented* size, not the window: the driver
    /// decides how large the images are, and the image is what the compositor
    /// shows. The logical viewport is `physical / effective scale`, so a zoom
    /// changes it exactly like a physical resize does.
    fn sync_viewport(&mut self) {
        let Some(chain) = &self.swapchain else {
            return;
        };
        let (width, height) = chain.extent();
        let scale = self.effective_scale();
        let presented = frame_size((width, height), scale);
        let wanted = Vec2::new(presented.logical.0, presented.logical.1);
        if (self.app.context().viewport - wanted).length() > 0.01 {
            self.app.resize(wanted);
        }
    }

    /// Keep the OS input method in sync with the focused control.
    ///
    /// Without `set_ime_allowed` Windows never sends composition at all, so a
    /// Chinese/Japanese/Korean keyboard could only ever type ASCII. The
    /// candidate window is anchored at the click position.
    fn sync_ime(&mut self, state: &mut PlatformState, window: &Window) {
        let active = self.shell().ime_active();
        if active != self.ime_allowed {
            window.set_ime_allowed(active);
            self.ime_allowed = active;
        }
        if active {
            let scale = self.effective_scale();
            let caret = self.shell().ime_caret() * scale;
            window.set_ime_cursor_area(
                PhysicalPosition::new(caret.x.round() as i32, caret.y.round() as i32),
                PhysicalSize::new(
                    (2.0 * scale).round().max(1.0) as u32,
                    (18.0 * scale).round().max(1.0) as u32,
                ),
            );
        }
        // Committed text must be inserted exactly once, so the buffer is
        // drained here rather than read.
        let commit = state.ime.take_commit();
        let mut changed = false;
        if !commit.is_empty() {
            // Through the runtime, so the commit also marks the frame dirty.
            self.app.text(&commit);
            changed = true;
        }
        let preedit = if active {
            state.ime.preedit().to_string()
        } else {
            String::new()
        };
        if preedit != self.shell().state.ime_preedit {
            self.shell_mut().state.ime_preedit = preedit;
            self.shell_mut().state.bump();
            self.app.invalidate();
            changed = true;
        }
        // IME events are not `PlatformEvent`s, so a parked loop would never
        // learn that a composition appeared. Bypass the cadence for them too.
        if changed {
            state.pacer.request_immediate(Instant::now());
        }
    }
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

/// Translate a platform key into the runtime's key vocabulary.
fn app_key(key: &PlatformKey) -> AppKey {
    match key {
        PlatformKey::Named(name) => match name.as_str() {
            "Tab" => AppKey::Tab,
            "Escape" => AppKey::Escape,
            "Enter" => AppKey::Enter,
            "Backspace" => AppKey::Backspace,
            "Delete" => AppKey::Delete,
            "ArrowLeft" => AppKey::Left,
            "ArrowRight" => AppKey::Right,
            "ArrowUp" => AppKey::Up,
            "ArrowDown" => AppKey::Down,
            "Home" => AppKey::Home,
            "End" => AppKey::End,
            "PageUp" => AppKey::PageUp,
            "PageDown" => AppKey::PageDown,
            "Insert" => AppKey::Insert,
            "ContextMenu" => AppKey::ContextMenu,
            "CapsLock" => AppKey::CapsLock,
            "Copy" => AppKey::Copy,
            "Cut" => AppKey::Cut,
            "Paste" => AppKey::Paste,
            "Undo" => AppKey::Undo,
            "Redo" => AppKey::Redo,
            " " | "Space" => AppKey::Space,
            other => match other.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
                Some(number) if other.len() > 1 && (1..=24).contains(&number) => AppKey::F(number),
                _ => other
                    .chars()
                    .next()
                    .map_or(AppKey::Character('\0'), AppKey::Character),
            },
        },
        PlatformKey::Character(text) => text
            .chars()
            .next()
            .map_or(AppKey::Character('\0'), AppKey::Character),
        PlatformKey::Other(_) => AppKey::Character('\0'),
    }
}

/// Map a platform mouse button into the runtime's.
fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        // Back / forward are navigation, not pointer state the router owns.
        MouseButton::Other(_) => None,
    }
}

/// Adapts the backend's wake callback to the runtime's [`AppWaker`] trait.
struct SignalWaker(std::sync::Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for SignalWaker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SignalWaker")
    }
}

impl unbound_app::AppWaker for SignalWaker {
    fn wake(&self) {
        (self.0)();
    }
}

/// Translate the platform's modifier set into the runtime's.
fn app_modifiers(modifiers: Modifiers) -> AppModifiers {
    AppModifiers {
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.meta,
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
        let instance_extensions = window
            .display_handle()
            .map(|handle| {
                unbound_gpu::surface_extensions::required_instance_extensions(handle.as_raw())
            })
            .map(|extensions| extensions.into_iter().map(str::to_owned).collect())
            .unwrap_or_else(|_| vec!["VK_KHR_surface".to_owned()]);
        let config = GpuConfig {
            instance_extensions,
            device_extensions: vec![
                "VK_KHR_swapchain".to_string(),
                "VK_EXT_swapchain_maintenance1".to_string(),
                "VK_KHR_present_wait".to_string(),
            ],
            ..Default::default()
        };
        let gpu = match Gpu::new(config) {
            Ok(gpu) => gpu,
            Err(error) => {
                // The old message was "could not create a Vulkan device" and
                // threw the cause away, which left Windows 10 users with no
                // way to know whether the driver, the loader or a feature was
                // missing.
                eprintln!("could not create a Vulkan device: {error}");
                match &error {
                    unbound_gpu::device::GpuError::UnsupportedFeatures(_) => eprintln!(
                        "hint: update the GPU driver; the renderer needs descriptor indexing, \
                         dynamic rendering, synchronization2 and bufferDeviceAddress"
                    ),
                    unbound_gpu::device::GpuError::NoSuitableDevice(detail) => {
                        eprintln!("hint: {detail}");
                        eprintln!(
                            "hint: on Windows 10 install a Vulkan 1.2+ driver, or run on a \
                             machine with a GPU that exposes VK_KHR_synchronization2 and \
                             VK_KHR_dynamic_rendering"
                        );
                    }
                    unbound_gpu::device::GpuError::Loader(detail) => eprintln!(
                        "hint: the Vulkan loader is missing ({detail}); install the Vulkan \
                         Runtime from https://vulkan.lunarg.com"
                    ),
                    _ => {}
                }
                return;
            }
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
        self.primary = Some(window.id());
        // The chain was just created for the window's size: that request is
        // committed, so the frame does not rebuild it again.
        let size = window.inner_size();
        self.committed_request = (size.width.max(1), size.height.max(1));
        self.sync_viewport();
        self.last_frame = Instant::now();
    }

    fn window_created(&mut self, _state: &mut PlatformState, window: &Window) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let Some(raw) = ({
            // The GPU crate has its own handle enum so it does not have to
            // depend on winit; extract the native handle once here.
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match window.window_handle().ok().map(|handle| handle.as_raw()) {
                Some(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get()),
                _ => None,
            }
        }) else {
            eprintln!("the secondary window has no native handle");
            return;
        };
        let Ok(surface) = Surface::new_win32(&renderer.gpu, raw) else {
            eprintln!("could not create a surface for the secondary window");
            return;
        };
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let present =
            PresentConfig::from_capabilities(&renderer.gpu.enabled_device_extensions, false);
        let Ok(swapchain) = Swapchain::new(&renderer.gpu, &surface, width, height, &present) else {
            eprintln!("could not create a swapchain for the secondary window");
            return;
        };
        self.extras.push(ExtraWindow {
            id: window.id(),
            surface,
            swapchain,
            swapchain_dirty: false,
            committed_request: (width, height),
        });
    }

    fn window_closing(&mut self, window: &Window) {
        self.extras.retain(|extra| extra.id != window.id());
    }

    fn take_window_request(&mut self) -> Option<WindowConfig> {
        if !self.new_window_requested {
            return None;
        }
        self.new_window_requested = false;
        Some(WindowConfig {
            title: "Unbound UI Gallery — Second Window".to_string(),
            width: 960,
            height: 640,
            resizable: true,
            decorations: false,
            min_width: 640,
            min_height: 480,
            visible: true,
            ..Default::default()
        })
    }

    fn next_wake(&self) -> Option<Instant> {
        // A pending timer or a parked background task has to wake the loop;
        // the pacer's frame deadline is considered separately by the backend.
        self.app.next_wake_in().map(|delay| Instant::now() + delay)
    }

    fn attach_waker(&mut self, waker: std::sync::Arc<dyn Fn() + Send + Sync>) {
        self.app.set_waker(std::sync::Arc::new(SignalWaker(waker)));
    }

    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow {
        // Secondary windows present the same application; their input is not
        // routed yet, so only the presentation chain reacts.
        if Some(window.id()) != self.primary {
            if matches!(event, PlatformEvent::RecreateSwapchain(_)) {
                if let Some(extra) = self.extras.iter_mut().find(|extra| extra.id == window.id()) {
                    extra.swapchain_dirty = true;
                }
            }
            return Flow::Continue;
        }
        self.sync_window_size(window);
        let scale = self.effective_scale();
        self.shell_mut().set_effective_scale(scale);
        let mut flow = Flow::Continue;
        match event {
            PlatformEvent::ScaleFactorChanged(factor) => {
                self.os_scale = factor as f32;
                // The scale changes the logical viewport, which the frame
                // derives from the committed extent. Invalidating makes the
                // next frame rebuild the scene for the new size.
                self.app.invalidate();
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
                repeat,
            } => {
                // Ctrl+N opens a second window: a real second surface and
                // swapchain on the same device.
                if !repeat
                    && self.modifiers.control
                    && matches!(&key, PlatformKey::Character(text) if text.eq_ignore_ascii_case("n"))
                {
                    self.new_window_requested = true;
                    state.pacer.request_immediate(Instant::now());
                }
                // While the IME owns the text, the platform also reports the
                // composition's characters as key presses. Forwarding both
                // would insert every letter twice ("nihao" → "nniihhaaoo").
                let ime_owns_text = state.ime.enabled && matches!(key, PlatformKey::Character(_));
                if !ime_owns_text {
                    if repeat {
                        self.shell_mut().key_repeat(&key);
                    } else {
                        let modifiers = app_modifiers(self.modifiers);
                        self.app.key(app_key(&key), true, modifiers);
                    }
                }
                if let Some(command) = self.shell_mut().take_window_command() {
                    flow = self.handle_window_command(window, command);
                }
            }
            PlatformEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
            }
            PlatformEvent::MouseInput { state, button } => {
                if let Some(button) = pointer_button(button) {
                    let position = self.logical(self.pointer);
                    if button != PointerButton::Left {
                        // A secondary press never moves the window chrome; it
                        // is the UI's context-menu gesture. The middle button
                        // is a canvas pan, also owned by the runtime.
                        if state == ButtonState::Pressed {
                            self.app.pointer_down(position, button);
                        } else {
                            self.app.pointer_up(position, button);
                        }
                    } else if state == ButtonState::Pressed {
                        // The window chrome owns its edges and its title bar:
                        // the UI underneath must not also see the press.
                        self.resize_edge = self.resize_edge_at(window);
                        if let Some(edge) = self.resize_edge {
                            self.press_consumed = true;
                            let _ = window.drag_resize_window(resize_direction(edge));
                        } else {
                            self.app.pointer_move(position);
                            if self
                                .shell()
                                .is_titlebar_drag(position, self.app.context().hovered)
                            {
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
                                self.app.pointer_down(position, PointerButton::Left);
                                if let Some(command) = self.shell_mut().take_window_command() {
                                    flow = self.handle_window_command(window, command);
                                }
                            }
                        }
                    } else if self.press_consumed {
                        self.press_consumed = false;
                    } else {
                        self.app.pointer_up(position, PointerButton::Left);
                        if let Some(command) = self.shell_mut().take_window_command() {
                            flow = self.handle_window_command(window, command);
                        }
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
                    let current = self.shell().state.zoom_step.min(steps - 1);
                    self.shell_mut().state.zoom_step = if delta_y > 0.0 {
                        (current + 1).min(steps - 1)
                    } else {
                        current.saturating_sub(1)
                    };
                    self.shell_mut().state.bump();
                    self.app.invalidate();
                } else {
                    self.app
                        .wheel(self.logical(self.pointer), Vec2::new(0.0, delta_y as f32));
                }
            }
            PlatformEvent::Focused(false) => {
                // The window lost the focus: whatever the pointer was doing
                // is over. The capture, the scrollbar grab and the demo
                // drags have to go, or the shell keeps a ghost drag alive
                // until the user happens to click somewhere else.
                self.shell_mut().cancel_interactions();
                self.app.cancel_pointer();
                self.app
                    .pointer_up(self.logical(self.pointer), PointerButton::Left);
            }
            PlatformEvent::RedrawRequested => {
                // The OS asked to repaint (the window was uncovered, or the
                // compositor lost the surface). Only a *parked* loop has to be
                // kicked: while the pacer is animating it already owns the
                // cadence, and forcing a frame here would cancel the deadline
                // that was set one moment ago on every single frame.
                if !state.pacer.animating() {
                    state.pacer.request_immediate(Instant::now());
                }
            }
            PlatformEvent::ThemeChanged { dark } => {
                // Follow the OS preference; the in-app toggle still wins
                // until the next OS change.
                self.shell_mut().state.theme = if dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                self.shell_mut().state.bump();
                self.app.invalidate();
            }
            PlatformEvent::CloseRequested => return Flow::Exit,
            _ => {}
        }
        if flow == Flow::Exit {
            return Flow::Exit;
        }
        // Re-read the effective scale: a zoom click in this very event changed
        // it after the pre-handler snapshot.
        let scale = self.effective_scale();
        self.shell_mut().set_effective_scale(scale);
        self.sync_ime(state, window);
        // The rebuild itself happens in the frame loop, through the runtime.
        // Kick the pacer when anything in the scene moved, so a click is
        // visible now instead of at the next vsync.
        if self.app.dirty() != DirtyFlags::NONE {
            state.pacer.request_immediate(Instant::now());
        }
        self.write_theme_if_needed();
        self.apply_cursor(window);
        Flow::Continue
    }

    fn frame(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        Showcase::frame_impl(self, state, window)
    }

    fn accessibility_update(&mut self) -> Option<accesskit::TreeUpdate> {
        Showcase::accessibility_update_impl(self)
    }

    fn accessibility_action(
        &mut self,
        state: &mut PlatformState,
        window: &Window,
        request: unbound_platform::A11yRequest,
    ) -> Flow {
        Showcase::accessibility_action_impl(self, state, window, request)
    }
}

impl Showcase {
    /// Rebuild a secondary window's presentation chain.
    fn recreate_extra_swapchain(&mut self, index: usize, width: u32, height: u32) -> bool {
        let Some(renderer) = &mut self.renderer else {
            return false;
        };
        let Some(extra) = self.extras.get_mut(index) else {
            return false;
        };
        let present =
            PresentConfig::from_capabilities(&renderer.gpu.enabled_device_extensions, false);
        if let Err(error) =
            extra
                .swapchain
                .recreate(&renderer.gpu, &extra.surface, width, height, &present)
        {
            eprintln!("could not rebuild the secondary swapchain: {error}");
            return false;
        }
        // The chain now answers for this request. Its *actual* extent may
        // still differ from it — the driver clamps it to its own limits — and
        // that difference must never be mistaken for a new request.
        extra.committed_request = (width, height);
        let (actual_w, actual_h) = extra.swapchain.extent();
        if let Err(error) = renderer.ensure_depth_size(actual_w, actual_h) {
            eprintln!("could not resize the shared depth buffer: {error}");
            return false;
        }
        true
    }

    /// Render one secondary window.
    ///
    /// The window shares the device and the renderer with the primary one,
    /// but owns its surface, its swapchain and its cadence. The application is
    /// laid out for this window's logical viewport right before the draw, so
    /// the same UI fills either window at its own size.
    fn frame_secondary(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        let Some(index) = self.extras.iter().position(|extra| extra.id == window.id()) else {
            return Flow::Continue;
        };
        let size = window.inner_size();
        let requested = (size.width.max(1), size.height.max(1));
        // The rebuild criterion compares two *requested* sizes: the window's
        // current request against the one the chain was last built for. The
        // chain's actual extent is the driver's answer and may differ by a
        // pixel — comparing the two would rebuild on every frame and never
        // converge.
        let dirty = self.extras[index].swapchain_dirty;
        if needs_rebuild(requested, self.extras[index].committed_request, dirty) {
            self.extras[index].swapchain_dirty = false;
            if !self.recreate_extra_swapchain(index, requested.0, requested.1) {
                self.extras[index].swapchain_dirty = true;
                return Flow::Continue;
            }
        }
        if !state.pacer.should_render(Instant::now()) {
            return Flow::Continue;
        }

        // Lay the shared application out for this window and rebuild. The
        // viewport follows the committed extent — the same size the frame is
        // actually drawn at — so the layout and the camera can never describe
        // two different pictures.
        let scale = (window.scale_factor() as f32 * self.shell().state.zoom()).clamp(0.5, 4.0);
        self.shell_mut().set_effective_scale(scale);
        let (chain_w, chain_h) = self.extras[index].swapchain.extent();
        let presented = frame_size((chain_w, chain_h), scale);
        self.app
            .resize(Vec2::new(presented.logical.0, presented.logical.1));
        let report = self.app.frame(0.0);
        self.upload_asset_textures(&report.atlas_uploads);
        self.upload_glyphs();
        self.write_theme_if_needed();

        let Some(renderer) = &mut self.renderer else {
            return Flow::Continue;
        };
        let extra = &mut self.extras[index];
        if let Err(error) = renderer.wait_for_previous_frame() {
            eprintln!("waiting for the previous frame failed: {error}");
            return Flow::Continue;
        }
        let image = match extra.swapchain.acquire_outcome(ACQUIRE_TIMEOUT_NS) {
            Ok(AcquireOutcome::Ready { image, suboptimal }) => {
                if suboptimal {
                    extra.swapchain_dirty = true;
                }
                image
            }
            Ok(AcquireOutcome::OutOfDate) => {
                extra.swapchain_dirty = true;
                return Flow::Continue;
            }
            Err(error) => {
                eprintln!("could not acquire a secondary swapchain image: {error}");
                return Flow::Continue;
            }
        };
        let (actual_w, actual_h) = extra.swapchain.extent();
        self.scene_generation = self.scene_generation.wrapping_add(1);
        self.app.application_mut().shell_mut().scene.camera =
            Camera::ui((actual_w as f32, actual_h as f32), scale);
        // Two windows share the pool, so each draw uploads the whole scene
        // with a fresh generation instead of patching a base that belongs to
        // the other window.
        let shell = self.app.application().shell();
        let scene = SceneUpload {
            primitives: &shell.scene.primitives,
            transforms: &shell.scene.transforms,
            clips: &shell.scene.clips,
            payloads: &shell.scene.payloads,
            generation: self.scene_generation,
            camera: shell.scene.camera,
            key_prefix: (PassLayer::Ui as u64) << 56,
            dirty_ranges: &[],
            aux_changed: false,
            backdrops: &[],
        };
        if let Err(error) = renderer.render_into_swapchain(&scene, &image, actual_w, actual_h) {
            eprintln!("secondary render failed: {error}");
            return Flow::Continue;
        }
        let finished = image.render_finished;
        match extra
            .swapchain
            .present_outcome(&renderer.gpu, image.index, finished)
        {
            Ok(PresentOutcome::Optimal) => {}
            Ok(PresentOutcome::Recreate) => extra.swapchain_dirty = true,
            Err(error) => eprintln!("secondary present failed: {error}"),
        }
        state.pacer.on_present(Instant::now());
        Flow::Continue
    }

    fn frame_impl(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        if Some(window.id()) != self.primary {
            return self.frame_secondary(state, window);
        }
        self.sync_window_size(window);
        let scale = self.effective_scale();
        self.shell_mut().set_effective_scale(scale);

        // The window size is only a *request*. The driver decides how large
        // the swapchain images are, and the image is what the compositor
        // shows, so everything below derives from the committed extent: the
        // layout and the camera then always describe the same picture.
        let size = window.inner_size();
        let requested = (size.width, size.height);
        // A minimised window reports a zero extent: there is nothing to
        // rebuild for, and a one-pixel chain would be nonsense.
        let live = size.width > 0 && size.height > 0;

        // A resize that settled, or a swapchain the driver already rejected,
        // both mean the same thing: rebuild the chain for the new size.
        let settled = state.poll_resize(Instant::now());
        // The frame-rate limit lives in the app state; the pacer is the only
        // place that enforces it. A change also changes the present mode
        // (capped runs stay tear-free, an uncapped run must not be pinned to
        // the display's refresh rate), so the chain is rebuilt next.
        let limit = self.shell().state.effective_fps_limit();
        let mut dirty = self.swapchain_dirty || settled.is_some();
        if state.pacer.fps_limit() != limit {
            state.pacer.set_fps_limit(limit);
            dirty = true;
        }
        // The comparison is between two *requested* sizes. The chain's actual
        // extent is the driver's answer and may legitimately differ from the
        // request by a pixel; comparing the two would rebuild the chain on
        // every frame, never converge, and present a layout that belongs to
        // one size in an image of another.
        let rebuild = live && needs_rebuild(requested, self.committed_request, dirty);
        if rebuild {
            self.swapchain_dirty = false;
            if !self.recreate_swapchain(window, requested.0, requested.1) {
                // The old chain belongs to the old size: presenting the new
                // layout in it would put the UI in the wrong place. Skip the
                // frame and try again on the next one.
                self.swapchain_dirty = true;
                state.pacer.request_immediate(Instant::now());
                return Flow::Continue;
            }
        }

        // The layout follows the *presented* size: the committed extent of
        // the swapchain. The page can therefore never be laid out for a size
        // the frame is not drawn at.
        if self.swapchain.is_none() {
            return Flow::Continue;
        }
        self.sync_viewport();
        self.sync_ime(state, window);
        let maximized = window.is_maximized();
        if self.shell().window_maximized != maximized {
            self.shell_mut().set_window_maximized(maximized);
            self.app.invalidate();
        }

        let mut now = Instant::now();
        // Check the cadence *before* touching the frame clock: an early
        // return used to leave `last_frame` and the FPS counter updated for a
        // frame that was never drawn.
        //
        // The event loop wakes `SPIN_LEAD` before the deadline, so the last
        // sub-millisecond gap is closed here instead of being left to the
        // millisecond-granular OS timer.
        if !rebuild && !state.pacer.should_render(now) {
            if state.pacer.within_spin_lead(now) {
                now = state.pacer.spin_to_deadline();
            } else {
                return Flow::Continue;
            }
        }

        // Advance time and rebuild the scene before borrowing the renderer:
        // the app and the GPU objects are disjoint, but the helper methods
        // borrow all of `self`.
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.1);
        self.last_frame = now;
        if dt > 0.0 {
            // Exponential moving average: a single hitch must not make the
            // HUD jump between 40 and 120.
            let instant_fps = 1.0 / dt;
            let alpha = 0.15;
            let demo = &mut self.shell_mut().state.demo;
            demo.fps += (instant_fps - demo.fps) * alpha;
            demo.frame_ms += (dt * 1000.0 - demo.frame_ms) * alpha;
        }
        // With a second window on the same renderer, every window rebuilds
        // for its own viewport and uploads the whole scene: the shared
        // instance pool can only hold one patch base at a time.
        let multi = !self.extras.is_empty();
        // The runtime reduces the tick, rebuilds the scene and returns the
        // upload plan for this frame.
        let dirty = self.app.tick(dt);
        let moving = dirty != DirtyFlags::NONE;
        let report = self.advance_scene(0.0);
        self.write_theme_if_needed();
        self.apply_cursor(window);

        // The upload plan. The generation moves when the scene's shape
        // changed, and it also moves when the previous frame was dropped:
        // the runtime's diff base runs ahead of the pool after a dropped
        // frame, and only a full upload can bring the two back together.
        let plan = upload_plan(self.scene_generation, &report.diff, self.frame_dropped);

        let scale = self.effective_scale();
        let Some(renderer) = &mut self.renderer else {
            return Flow::Continue;
        };
        let Some(swapchain) = &mut self.swapchain else {
            return Flow::Continue;
        };
        let (width, height) = swapchain.extent();
        if let Err(error) = renderer.wait_for_previous_frame() {
            eprintln!("waiting for the previous frame failed: {error}");
            self.frame_dropped = true;
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
                state.resize.mark_out_of_date(Extent::new(width, height));
                self.frame_dropped = true;
                return Flow::Continue;
            }
            Err(error) => {
                eprintln!("could not acquire a swapchain image: {error}");
                self.frame_dropped = true;
                return Flow::Continue;
            }
        };

        // The app lays out in logical units (physical / (OS scale × zoom)); the
        // UI camera scales those units back up, so a 150 % layout maps exactly
        // onto the physical viewport at any DPI.
        //
        // The camera is built from the same extent the layout was solved
        // for, so the two can never describe different pictures.
        self.app.application_mut().shell_mut().scene.camera =
            Camera::ui((width as f32, height as f32), scale);
        let shell = self.app.application().shell();
        let (generation, dirty_ranges): (u64, &[(u32, u32)]) = if multi {
            // Two windows share the instance pool, so each draw uploads the
            // whole scene under a *fresh* generation instead of patching a
            // base that belongs to the other window. The generation moves on
            // every draw, so the two windows can never claim the same one.
            (self.scene_generation.wrapping_add(1), &[])
        } else {
            (plan.generation, plan.ranges)
        };
        self.scene_generation = generation;
        let scene = SceneUpload {
            primitives: &shell.scene.primitives,
            transforms: &shell.scene.transforms,
            clips: &shell.scene.clips,
            payloads: &shell.scene.payloads,
            generation,
            camera: shell.scene.camera,
            key_prefix: (PassLayer::Ui as u64) << 56,
            dirty_ranges,
            aux_changed: report.aux_changed,
            backdrops: &[],
        };
        if let Err(error) = renderer.render_into_swapchain(&scene, &image, width, height) {
            eprintln!("render failed: {error}");
            self.frame_dropped = true;
            return Flow::Continue;
        }
        // The scene reached the pool: the diff base and the GPU are in sync
        // again, so a later drop starts from a clean slate.
        self.frame_dropped = false;
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
                state.resize.mark_out_of_date(Extent::new(width, height));
            }
            Err(error) => {
                eprintln!("present failed: {error}");
            }
        }
        state.pacer.on_present(Instant::now());
        // A static page with no HUD, no animation and a capped frame rate has
        // nothing to show: let the event loop park instead of submitting a
        // full cull + radix sort every refresh.
        let keep_animating = self.shell().state.fps_limit == 0
            || self.shell().state.show_hud
            || self.shell().state.active_page == showcase::pages::PageId::Telemetry
            || moving;
        state.pacer.set_animating(keep_animating);
        // No `request_redraw` here: `about_to_wait` arms
        // `ControlFlow::WaitUntil(pacer.next_wake())` on every iteration, so
        // the loop wakes itself exactly when the next frame is due. Requesting
        // a redraw as well delivered `RedrawRequested` immediately, whose
        // handler cancelled the deadline that had just been set — the software
        // limiter never got to enforce anything.
        state.latency.on_present(Instant::now());
        Flow::Continue
    }

    fn accessibility_update_impl(&mut self) -> Option<accesskit::TreeUpdate> {
        // The runtime's collector holds the shell's tree; taking the delta
        // here is what makes a static frame cost no system call.
        self.app.scene_mut().semantics.take_update()
    }

    fn accessibility_action_impl(
        &mut self,
        _state: &mut PlatformState,
        _window: &Window,
        request: unbound_platform::A11yRequest,
    ) -> Flow {
        // Through the reducer, so an assistive-technology click is the same
        // code path as a mouse click.
        self.app.a11y(request.node, request.action, request.value);
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
    app.rebuild(
        (width as f32 / zoom, height as f32 / zoom),
        showcase::shell::Interaction::default(),
    );
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
        .write_theme_lut(&theme::lut(dark))
        .map_err(|e| e.to_string())?;
    renderer
        .render(&app.scene.upload(1, &[]))
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

    /// Changing the app zoom must not resize the window.
    ///
    /// The layout is re-derived from `physical / (os_scale × zoom)`, so the
    /// physical window is left alone; this guards against the old behaviour
    /// coming back (a resize request per zoom step broke the drag gestures and
    /// could move the window off-screen at 200 %).
    #[test]
    fn app_zoom_never_changes_the_physical_window_size() {
        let mut app = GalleryApp::new();
        let before = app.state.zoom();
        app.state.zoom_step = (app.state.zoom_step + 1).min(3);
        assert!(app.state.zoom() > before);
    }

    #[test]
    fn the_gallery_starts_on_home_with_a_full_catalogue() {
        let app = GalleryApp::new();
        assert_eq!(app.state.active_page, PageId::Home);
        assert_eq!(PageId::ALL.len(), 22);
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

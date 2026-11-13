//! Windowed application host and runner adapting [`AppRuntime`] to [`PlatformApp`].

use std::sync::Arc;
use std::time::Instant;

use unbound_gpu::atlas::AtlasSet;
use unbound_gpu::data::PassLayer;
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, Surface, Swapchain};
use unbound_gpu::swapchain::PresentConfig;
use unbound_gpu::{Camera, Renderer, SceneUpload};
use unbound_input::PointerButton;
use unbound_math::Vec2;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::window::{
    ButtonState, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey, PlatformState,
    WindowConfig,
};
use winit::window::Window;

use crate::{AppKey, AppRuntime, Application, Modifiers as AppModifiers};

/// Clipboard adapter wrapping platform clipboard.
struct WinClipboard(std::cell::RefCell<unbound_platform::SystemClipboard>);

impl crate::ClipboardProvider for WinClipboard {
    fn get_text(&self) -> Option<String> {
        use unbound_platform::ClipboardProvider as _;
        self.0.borrow_mut().read_text().ok().flatten()
    }

    fn set_text(&mut self, text: &str) {
        use unbound_platform::ClipboardProvider as _;
        let _ = self.0.borrow_mut().write_text(text);
    }
}

/// Translates a platform key into the runtime's [`AppKey`] vocabulary.
fn translate_app_key(key: &PlatformKey) -> AppKey {
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

/// Map a platform mouse button into the runtime's [`PointerButton`].
fn translate_pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Other(_) => None,
    }
}

fn translate_modifiers(m: Modifiers) -> AppModifiers {
    AppModifiers {
        shift: m.shift,
        ctrl: m.control,
        alt: m.alt,
        meta: m.meta,
    }
}

/// The generic windowed host adapter driving [`AppRuntime`].
pub struct WindowedRunner<A: Application> {
    runtime: AppRuntime<A>,
    renderer: Option<Renderer>,
    surface: Option<Surface>,
    swapchain: Option<Swapchain>,
    atlases: Option<Arc<AtlasSet>>,
    pointer: Vec2,
    modifiers: Modifiers,
    last_frame: Instant,
    swapchain_dirty: bool,
    committed_request: (u32, u32),
    scene_generation: u64,
}

impl<A: Application> WindowedRunner<A> {
    /// Create a new window runner for `app`.
    pub fn new(app: A, config: &WindowConfig) -> Self {
        let (w, h) = (config.width.max(1), config.height.max(1));
        let viewport = Vec2::new(w as f32, h as f32);
        let mut runtime = AppRuntime::new(app, viewport);
        runtime.set_clipboard(Box::new(WinClipboard(std::cell::RefCell::new(
            unbound_platform::SystemClipboard::new(),
        ))));

        Self {
            runtime,
            renderer: None,
            surface: None,
            swapchain: None,
            atlases: None,
            pointer: Vec2::ZERO,
            modifiers: Modifiers::default(),
            last_frame: Instant::now(),
            swapchain_dirty: false,
            committed_request: (w, h),
            scene_generation: 0,
        }
    }

    /// Access the underlying application runtime.
    #[must_use]
    pub fn runtime(&self) -> &AppRuntime<A> {
        &self.runtime
    }

    /// Mutably access the underlying application runtime.
    pub fn runtime_mut(&mut self) -> &mut AppRuntime<A> {
        &mut self.runtime
    }
}

impl<A: Application> PlatformApp for WindowedRunner<A> {
    fn resumed(&mut self, state: &mut PlatformState, window: &Window) {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        self.committed_request = (width, height);
        self.runtime.resize(Vec2::new(width as f32, height as f32));

        let config = GpuConfig {
            app_name: "Unbound App".to_string(),
            device_extensions: vec![
                "VK_KHR_swapchain".to_string(),
                "VK_KHR_dynamic_rendering".to_string(),
                "VK_KHR_synchronization2".to_string(),
            ],
            ..Default::default()
        };

        let gpu = match Gpu::new(config) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("[unbound-app] Vulkan device initialization failed: {error:?}");
                return;
            }
        };

        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match window.window_handle().ok().map(|h| h.as_raw()) {
                Some(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => {
                    eprintln!("[unbound-app] Window has no native Win32 handle");
                    return;
                }
            }
        };

        let Ok(surface) = Surface::new_win32(&gpu, hwnd) else {
            eprintln!("[unbound-app] Could not create Vulkan surface");
            return;
        };

        let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false);
        let Ok(swapchain) = Swapchain::new(&gpu, &surface, width, height, &present) else {
            eprintln!("[unbound-app] Could not create swapchain");
            return;
        };

        let device_extensions = gpu.enabled_device_extensions.clone();
        let mut renderer = match Renderer::new(gpu, width, height) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("[unbound-app] Could not create renderer: {error:?}");
                return;
            }
        };

        let atlases = match AtlasSet::new(&renderer.gpu, 64) {
            Ok(atlases) => Arc::new(atlases),
            Err(error) => {
                eprintln!("[unbound-app] Could not create atlas set: {error:?}");
                return;
            }
        };

        if let Err(error) = renderer.install_atlases(atlases.clone()) {
            eprintln!("[unbound-app] Could not install atlases: {error:?}");
            return;
        }

        self.atlases = Some(atlases);
        self.swapchain = Some(swapchain);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        state.set_present_extensions(&device_extensions);
        self.last_frame = Instant::now();
    }

    fn event(&mut self, _state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow {
        match event {
            PlatformEvent::Resized(extent) => {
                let size = window.inner_size();
                self.committed_request = (size.width.max(1), size.height.max(1));
                self.runtime
                    .resize(Vec2::new(extent.width as f32, extent.height as f32));
                self.swapchain_dirty = true;
            }
            PlatformEvent::RecreateSwapchain(_) => {
                self.swapchain_dirty = true;
            }
            PlatformEvent::CursorMoved { x, y } => {
                self.pointer = Vec2::new(x as f32, y as f32);
                self.runtime.pointer_move(self.pointer);
            }
            PlatformEvent::MouseInput { state, button } => {
                if let Some(btn) = translate_pointer_button(button) {
                    if state == ButtonState::Pressed {
                        self.runtime.pointer_down(self.pointer, btn);
                    } else {
                        self.runtime.pointer_up(self.pointer, btn);
                    }
                }
            }
            PlatformEvent::MouseWheel { delta_x, delta_y } => {
                self.runtime
                    .wheel(self.pointer, Vec2::new(delta_x as f32, delta_y as f32));
            }
            PlatformEvent::KeyInput { state, key, .. } => {
                let app_k = translate_app_key(&key);
                let pressed = state == ButtonState::Pressed;
                let modifiers = translate_modifiers(self.modifiers);
                self.runtime.key(app_k, pressed, modifiers);
            }
            PlatformEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
            }
            PlatformEvent::CloseRequested => {
                return Flow::Exit;
            }
            PlatformEvent::RedrawRequested => {
                window.request_redraw();
            }
            _ => {}
        }

        Flow::Continue
    }

    fn frame(&mut self, _state: &mut PlatformState, window: &Window) -> Flow {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        let report = self.runtime.frame(dt);

        if let (Some(renderer), Some(atlases)) = (&mut self.renderer, &self.atlases) {
            for upload in report.atlas_uploads {
                let _ = atlases.upload_batch(&renderer.gpu, &[upload]);
            }

            if let (Some(swapchain), Some(surface)) = (&mut self.swapchain, &self.surface) {
                let size = window.inner_size();
                let (width, height) = (size.width.max(1), size.height.max(1));
                if self.swapchain_dirty || self.committed_request != (width, height) {
                    self.committed_request = (width, height);
                    let present = PresentConfig::from_capabilities(
                        &renderer.gpu.enabled_device_extensions,
                        false,
                    );
                    let _ = swapchain.recreate(
                        &renderer.gpu,
                        surface,
                        self.committed_request.0,
                        self.committed_request.1,
                        &present,
                    );
                    self.swapchain_dirty = false;
                }

                if let Ok(AcquireOutcome::Ready { image, suboptimal }) =
                    swapchain.acquire_outcome(1_000_000_000)
                {
                    if suboptimal {
                        self.swapchain_dirty = true;
                    }
                    self.scene_generation = self.scene_generation.wrapping_add(1);
                    let camera = Camera::ui((width as f32, height as f32), 1.0);
                    let scene_data = self.runtime.scene();
                    let scene = SceneUpload {
                        primitives: &scene_data.primitives,
                        transforms: &scene_data.transforms,
                        clips: &scene_data.clips,
                        payloads: &scene_data.payloads,
                        generation: self.scene_generation,
                        camera,
                        key_prefix: (PassLayer::Ui as u64) << 56,
                        dirty_ranges: &[],
                        aux_changed: report.aux_changed,
                        backdrops: &[],
                    };

                    let _ = renderer.render_into_swapchain(&scene, &image, width, height);
                    let _ = swapchain.present_outcome(
                        &renderer.gpu,
                        image.index,
                        image.render_finished,
                    );
                }
            }
        }

        if report.dirty.needs_rebuild() {
            window.request_redraw();
        }

        Flow::Continue
    }
}

/// One-line windowed application entry point.
pub fn run_windowed<A: Application + 'static>(
    app: A,
    config: WindowConfig,
) -> Result<(), PlatformError> {
    let runner = WindowedRunner::new(app, &config);
    PlatformWindow::new(&config)?.run(runner)
}

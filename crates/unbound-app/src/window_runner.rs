//! 窗口化应用宿主与运行器，将 [`AppRuntime`] 适配至 [`PlatformApp`]。

use std::sync::Arc;
use std::time::Instant;

use unbound_gpu::atlas::AtlasSet;
use unbound_gpu::data::PassLayer;
use unbound_gpu::debug::{is_device_lost, recover_with, RecoveryPolicy};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, Surface, Swapchain};
use unbound_gpu::swapchain::PresentConfig;
use unbound_gpu::{Camera, Renderer, SceneUpload};
use unbound_input::PointerButton;
use unbound_math::Vec2;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::win32_frame::{install_custom_frame, set_window_dark_mode, FrameHitMetrics};
use unbound_platform::window::{
    ButtonState, ImeEvent, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey,
    PlatformState, WindowConfig,
};
use unbound_theme::{ThemeSet, ThemeState};
use winit::window::Window;

use crate::{AppKey, AppRuntime, Application, ImeUpdate, Modifiers as AppModifiers, ThemeKind};

/// 封装平台剪贴板的适配器。
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

/// 将平台按键翻译为运行时的 [`AppKey`] 词汇表。
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

/// 将平台鼠标按键映射为运行时的 [`PointerButton`]。
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

/// 派生窗口呈现所需的实例扩展。
///
/// 平台相关的一半（Windows 上是 `VK_KHR_win32_surface`，X11 上是
/// `VK_KHR_xcb_surface` …）只能从窗口的*显示*句柄取得：缺少它时
/// `vkCreateWin32SurfaceKHR` 会是无法解析的函数指针；缺少 `VK_KHR_surface`
/// 则使 `VK_KHR_swapchain` 成为非法设备扩展
/// （`VUID-vkCreateDevice-ppEnabledExtensionNames-01387`）。若把这一对硬编码，
/// 运行器将只能在 Windows 工作，因此改为派生，与 `apps/showcase` 保持一致。
fn window_instance_extensions(window: &Window) -> Result<Vec<String>, String> {
    use raw_window_handle::HasDisplayHandle;

    let handle = window
        .display_handle()
        .map_err(|error| format!("window has no native display handle: {error}"))?;
    Ok(
        unbound_gpu::surface_extensions::required_instance_extensions(handle.as_raw())
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
}

/// 驱动 [`AppRuntime`] 的通用窗口化宿主适配器。
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
    frame_metrics: Option<Arc<FrameHitMetrics>>,
    hwnd: Option<isize>,
    /// GPU 着色器用于解析颜色 token 的语义颜色表。
    theme: ThemeState,
    /// 当前渲染器 LUT 中已写入的主题；`None` 表示仍需上传
    /// （设备重建完成后同样如此）。
    uploaded_theme: Option<ThemeKind>,
}

impl<A: Application> WindowedRunner<A> {
    /// 为 `app` 创建新的窗口运行器。
    pub fn new(app: A, config: &WindowConfig) -> Self {
        let (w, h) = (config.width.max(1), config.height.max(1));
        let viewport = Vec2::new(w as f32, h as f32);
        let mut runtime = AppRuntime::new(app, viewport);
        runtime.set_clipboard(Box::new(WinClipboard(std::cell::RefCell::new(
            unbound_platform::SystemClipboard::new(),
        ))));
        let theme = ThemeState::new(ThemeSet::default_set(), runtime.context().theme);

        Self {
            runtime,
            theme,
            uploaded_theme: None,
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
            frame_metrics: None,
            hwnd: None,
        }
    }

    /// 访问底层应用运行时。
    #[must_use]
    pub fn runtime(&self) -> &AppRuntime<A> {
        &self.runtime
    }

    /// 可变访问底层应用运行时。
    pub fn runtime_mut(&mut self) -> &mut AppRuntime<A> {
        &mut self.runtime
    }

    /// 让渲染器的 4 KiB 颜色表与运行时主题保持同步。
    ///
    /// `Renderer::new` 只写入 5 个占位 token，其余 token 一律解析为透明黑，
    /// 而 `SURFACE` 清屏为白色。在写入真正的 `ThemeLut` 之前，整窗就是一片
    /// 纯白：图元确实绘制了，但颜色全透明。showcase 自行上传 LUT，通用运行器
    /// 必须为每个示例做同样的事，并在设备重建后重新上传。
    fn sync_theme(&mut self, window: &Window) {
        let kind = self.runtime.context().theme;
        if self.uploaded_theme == Some(kind) {
            return;
        }
        self.theme.switch(kind);
        let lut = self.theme.lut().clone();
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.write_theme_lut(&lut) {
            Ok(()) => {
                self.uploaded_theme = Some(kind);
                // 切换主题只改变颜色：既不重排也不重传图元，
                // 但需要再多呈现一帧。
                window.request_redraw();
            }
            Err(error) => {
                eprintln!("[unbound-app] Could not upload the theme LUT: {error:?}");
            }
        }
    }

    fn recover_gpu(&mut self, window: &Window) {
        // 所有 Vulkan 对象都没有 `Drop`：过去直接置 `None` 会泄漏全部句柄。
        // 尤其是旧 `VkSurfaceKHR` 仍绑定在 HWND 上，导致第一次重建就报
        // `ERROR_NATIVE_WINDOW_IN_USE_KHR`。必须先销毁子对象，再让持有设备的
        // renderer 拆除设备本身。
        let renderer = self.renderer.take();
        let swapchain = self.swapchain.take();
        let surface = self.surface.take();
        self.atlases = None;
        if let (Some(renderer), Some(swapchain), Some(surface)) = (renderer, swapchain, surface) {
            swapchain.destroy(&renderer.gpu);
            surface.destroy(&renderer.gpu);
            renderer.destroy();
        }

        let hwnd = match self.hwnd {
            Some(h) => h,
            None => return,
        };

        let instance_extensions = match window_instance_extensions(window) {
            Ok(extensions) => extensions,
            Err(error) => {
                eprintln!("[unbound-app] GPU recovery cannot continue: {error}");
                return;
            }
        };

        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let policy = RecoveryPolicy::default();
        let device_config = GpuConfig {
            app_name: "Unbound App (Recovered)".to_string(),
            instance_extensions,
            device_extensions: vec![
                "VK_KHR_swapchain".to_string(),
                "VK_KHR_dynamic_rendering".to_string(),
                "VK_KHR_synchronization2".to_string(),
            ],
            ..Default::default()
        };

        let recovered = recover_with(policy, || {
            let gpu = Gpu::new(device_config.clone())?;
            let surface = Surface::new_win32(&gpu, hwnd)?;
            let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false);
            let swapchain = Swapchain::new(&gpu, &surface, width, height, &present)?;
            let mut renderer = Renderer::new(gpu, width, height)?;
            let atlases = Arc::new(AtlasSet::new(&renderer.gpu, 64)?);
            renderer.install_atlases(atlases.clone())?;
            Ok((renderer, swapchain, surface, atlases))
        });

        match recovered {
            Ok(((renderer, swapchain, surface, atlases), attempts)) => {
                eprintln!("[unbound-app] GPU recovered successfully after {attempts} attempt(s).");
                self.renderer = Some(renderer);
                self.swapchain = Some(swapchain);
                self.surface = Some(surface);
                self.atlases = Some(atlases);
                self.swapchain_dirty = true;
                // 新建的渲染器此时只有占位 LUT。
                self.uploaded_theme = None;
                self.sync_theme(window);
                self.runtime.request_full_rebuild();
                window.request_redraw();
            }
            Err(e) => {
                eprintln!("[unbound-app] GPU recovery failed: {e:?}");
            }
        }
    }
}

impl<A: Application> PlatformApp for WindowedRunner<A> {
    fn resumed(&mut self, state: &mut PlatformState, window: &Window) {
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        self.committed_request = (width, height);
        self.runtime.resize(Vec2::new(width as f32, height as f32));

        let instance_extensions = match window_instance_extensions(window) {
            Ok(extensions) => extensions,
            Err(error) => {
                eprintln!("[unbound-app] {error}");
                return;
            }
        };
        let config = GpuConfig {
            app_name: "Unbound App".to_string(),
            instance_extensions,
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
        self.hwnd = Some(hwnd);

        let metrics = Arc::new(FrameHitMetrics::new());
        metrics.set_scale_factor(window.scale_factor() as f32);
        install_custom_frame(hwnd, metrics.clone());
        self.frame_metrics = Some(metrics);

        state.set_present_extensions(&device_extensions);
        self.sync_theme(window);
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
            PlatformEvent::Ime(ime) => match ime {
                ImeEvent::Preedit(text, selection) => {
                    self.runtime.ime(ImeUpdate {
                        preedit: text,
                        cursor_range: selection,
                        commit: None,
                    });
                }
                ImeEvent::Commit(text) => {
                    self.runtime.ime(ImeUpdate {
                        preedit: String::new(),
                        cursor_range: None,
                        commit: Some(text),
                    });
                }
                ImeEvent::Disabled => {
                    self.runtime.ime(ImeUpdate {
                        preedit: String::new(),
                        cursor_range: None,
                        commit: None,
                    });
                }
                ImeEvent::Enabled => {}
            },
            PlatformEvent::ScaleFactorChanged(scale) => {
                if let Some(metrics) = &self.frame_metrics {
                    metrics.set_scale_factor(scale as f32);
                }
            }
            PlatformEvent::ThemeChanged { dark } => {
                self.runtime.set_theme(if dark {
                    ThemeKind::Dark
                } else {
                    ThemeKind::Light
                });
                if let Some(hwnd) = self.hwnd {
                    set_window_dark_mode(hwnd, dark);
                }
            }
            PlatformEvent::FileHover(path) => {
                self.runtime.file_hover(vec![path]);
            }
            PlatformEvent::FileDrop(path) => {
                self.runtime.file_drop(vec![path]);
            }
            PlatformEvent::FileHoverCancelled => {
                self.runtime.file_hover_cancelled();
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

        // `reduce` 可能改动 `ctx.theme`（示例 01 的主题按钮正是如此），
        // 因此每帧都要检查 LUT。比较只是一次枚举读取，仅当真正切换主题时
        // 才会发生 4 KiB 上传。
        self.sync_theme(window);

        if let (Some(renderer), Some(atlases)) = (&mut self.renderer, &self.atlases) {
            // 整帧合并为一批：一个暂存缓冲、一个命令缓冲、一次队列等待
            // 即可上传全部字形。逐个上传会为每个字符提交一次命令缓冲，
            // 并同样频繁地阻塞队列。
            if !report.atlas_uploads.is_empty() {
                if let Err(error) = atlases.upload_batch(&renderer.gpu, &report.atlas_uploads) {
                    eprintln!("[unbound-app] Could not upload atlas glyphs: {error:?}");
                }
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
                    if let Err(error) = swapchain.recreate(
                        &renderer.gpu,
                        surface,
                        self.committed_request.0,
                        self.committed_request.1,
                        &present,
                    ) {
                        eprintln!("[unbound-app] Could not recreate swapchain: {error:?}");
                    } else {
                        // 窗口路径借用交换链图像作为颜色附件，因此只有
                        // 深度缓冲需要跟随驱动实际返回的尺寸。
                        let (actual_w, actual_h) = swapchain.extent();
                        if let Err(error) = renderer.ensure_depth_size(actual_w, actual_h) {
                            eprintln!("[unbound-app] Could not resize depth buffer: {error:?}");
                        }
                    }
                    self.swapchain_dirty = false;
                }

                // acquire 信号量池按在途帧数分槽。只有等待该信号量的帧退役后
                // 槽位才可复用，因此必须在下一次 acquire *之前*等待 fence。
                // `render()` 也会等待，但那时信号量已被再次置位——
                // 正是 `VUID-vkAcquireNextImageKHR-semaphore-01779`。
                if let Err(error) = renderer.wait_for_previous_frame() {
                    eprintln!("[unbound-app] Waiting for the previous frame failed: {error:?}");
                    return Flow::Continue;
                }

                match swapchain.acquire_outcome(1_000_000_000) {
                    Ok(AcquireOutcome::Ready { image, suboptimal }) => {
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

                        if let Err(ref err) =
                            renderer.render_into_swapchain(&scene, &image, width, height)
                        {
                            if is_device_lost(err) {
                                eprintln!("[unbound-app] GPU device lost during render. Triggering recovery...");
                                self.recover_gpu(window);
                                return Flow::Continue;
                            }
                            eprintln!("[unbound-app] render failed: {err:?}");
                        }

                        if let Err(ref err) = swapchain.present_outcome(
                            &renderer.gpu,
                            image.index,
                            image.render_finished,
                        ) {
                            if is_device_lost(err) {
                                eprintln!("[unbound-app] GPU device lost during present. Triggering recovery...");
                                self.recover_gpu(window);
                                return Flow::Continue;
                            }
                        }
                    }
                    Err(ref err) if is_device_lost(err) => {
                        eprintln!(
                            "[unbound-app] GPU device lost during acquire. Triggering recovery..."
                        );
                        self.recover_gpu(window);
                        return Flow::Continue;
                    }
                    _ => {}
                }
            }
        }

        if report.dirty.needs_rebuild() {
            window.request_redraw();
        }

        Flow::Continue
    }
}

/// 单行窗口化应用入口函数。
pub fn run_windowed<A: Application + 'static>(
    app: A,
    config: WindowConfig,
) -> Result<(), PlatformError> {
    let runner = WindowedRunner::new(app, &config);
    PlatformWindow::new(&config)?.run(runner)
}

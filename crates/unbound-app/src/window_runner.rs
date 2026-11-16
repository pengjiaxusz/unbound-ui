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
use unbound_math::{Rect, Vec2};
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::win32_frame::{install_custom_frame, set_window_dark_mode, FrameHitMetrics};
use unbound_platform::window::{
    ButtonState, ImeEvent, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey,
    PlatformState, WindowConfig,
};
use unbound_platform::Window;
use unbound_theme::{ThemeSet, ThemeState};

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
    /// 当前渲染器 LUT 中已写入的主题与品牌色三元组；`None` 表示仍需上传
    /// （设备重建完成后同样如此）。
    uploaded_theme: Option<(
        ThemeKind,
        unbound_theme::ProductPreset,
        unbound_theme::AccentBrand,
    )>,
    /// 原生窗口句柄创建完成后的可选宿主回调（例如供 下游渲染工具 注册主窗口 HWND）。
    on_hwnd_created: Option<Box<dyn FnMut(isize) + Send>>,
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
        let ctx = runtime.context();
        let theme = ThemeState::new(ThemeSet::for_product(ctx.preset, ctx.accent), ctx.theme);

        Self {
            runtime,
            theme,
            uploaded_theme: None,
            on_hwnd_created: None,
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

    /// 注册原生窗口句柄（Win32 `HWND`）就绪时的回调函数。
    #[must_use]
    pub fn with_hwnd_callback(mut self, callback: impl FnMut(isize) + Send + 'static) -> Self {
        self.on_hwnd_created = Some(Box::new(callback));
        self
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
        let ctx = self.runtime.context();
        let key = (ctx.theme, ctx.preset, ctx.accent);
        if self.uploaded_theme == Some(key) {
            return;
        }
        let prev_preset_accent = self.uploaded_theme.map(|(_, p, a)| (p, a));
        if prev_preset_accent != Some((key.1, key.2)) {
            self.theme = ThemeState::new(ThemeSet::for_product(key.1, key.2), key.0);
        } else {
            self.theme.switch(key.0);
        }
        let lut = self.theme.lut().clone();
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.write_theme_lut(&lut) {
            Ok(()) => {
                self.uploaded_theme = Some(key);
                // 切换主题或强调色只改变 4 KiB LUT：既不重排也不重传图元，
                // 但需要再多呈现一帧。
                window.request_redraw();
            }
            Err(error) => {
                eprintln!("[unbound-app] Could not upload the theme LUT: {error:?}");
            }
        }
    }

    fn destroy_vulkan(&mut self) {
        let renderer = self.renderer.take();
        let swapchain = self.swapchain.take();
        let surface = self.surface.take();
        self.atlases = None;
        if let Some(renderer) = renderer {
            // 必须先等待队列空闲，确保在途帧信号量与交换链图像全部退役后再销毁。
            renderer.gpu.wait_idle();
            if let Some(swapchain) = swapchain {
                swapchain.destroy(&renderer.gpu);
            }
            if let Some(surface) = surface {
                surface.destroy(&renderer.gpu);
            }
            renderer.destroy();
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
        if let Some(renderer) = renderer {
            renderer.gpu.wait_idle();
            if let Some(swapchain) = swapchain {
                swapchain.destroy(&renderer.gpu);
            }
            if let Some(surface) = surface {
                surface.destroy(&renderer.gpu);
            }
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
                "VK_EXT_swapchain_maintenance1".to_string(),
                "VK_KHR_present_wait".to_string(),
                "VK_KHR_dynamic_rendering".to_string(),
                "VK_KHR_synchronization2".to_string(),
            ],
            ..Default::default()
        };

        let recovered = recover_with(policy, || {
            let gpu = Gpu::new(device_config.clone())?;
            let surface = match Surface::new_win32(&gpu, hwnd) {
                Ok(s) => s,
                Err(err) => {
                    gpu.destroy();
                    return Err(err);
                }
            };
            let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false);
            let swapchain = match Swapchain::new(&gpu, &surface, width, height, &present) {
                Ok(sc) => sc,
                Err(err) => {
                    surface.destroy(&gpu);
                    gpu.destroy();
                    return Err(err);
                }
            };
            let mut renderer = match Renderer::new(gpu, width, height) {
                Ok(r) => r,
                Err(err) => {
                    return Err(err);
                }
            };
            let atlases = match AtlasSet::new(&renderer.gpu, 64) {
                Ok(a) => Arc::new(a),
                Err(err) => {
                    swapchain.destroy(&renderer.gpu);
                    surface.destroy(&renderer.gpu);
                    renderer.destroy();
                    return Err(err);
                }
            };
            if let Err(err) = renderer.install_atlases(atlases.clone()) {
                drop(atlases);
                swapchain.destroy(&renderer.gpu);
                surface.destroy(&renderer.gpu);
                renderer.destroy();
                return Err(err);
            }
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
                eprintln!("[unbound-app] Window display handle initialization failed: {error}");
                window.request_close();
                return;
            }
        };
        let config = GpuConfig {
            app_name: "Unbound App".to_string(),
            instance_extensions,
            device_extensions: vec![
                "VK_KHR_swapchain".to_string(),
                "VK_EXT_swapchain_maintenance1".to_string(),
                "VK_KHR_present_wait".to_string(),
                "VK_KHR_dynamic_rendering".to_string(),
                "VK_KHR_synchronization2".to_string(),
            ],
            ..Default::default()
        };

        let gpu = match Gpu::new(config) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Vulkan device initialization failed:\n{}",
                    error.diagnostic_report()
                );
                window.request_close();
                return;
            }
        };

        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match window.window_handle().ok().map(|h| h.as_raw()) {
                Some(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => {
                    eprintln!("[unbound-app] Window has no native Win32 handle");
                    gpu.destroy();
                    window.request_close();
                    return;
                }
            }
        };

        let surface = match Surface::new_win32(&gpu, hwnd) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create Vulkan surface (hwnd=0x{hwnd:X}):\n{}",
                    error.diagnostic_report()
                );
                gpu.destroy();
                window.request_close();
                return;
            }
        };

        let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false);
        let swapchain = match Swapchain::new(&gpu, &surface, width, height, &present) {
            Ok(swapchain) => swapchain,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create swapchain ({width}x{height}):\n{}",
                    error.diagnostic_report()
                );
                surface.destroy(&gpu);
                gpu.destroy();
                window.request_close();
                return;
            }
        };

        let device_extensions = gpu.enabled_device_extensions.clone();
        let mut renderer = match Renderer::new(gpu, width, height) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create renderer ({width}x{height}):\n{}",
                    error.diagnostic_report()
                );
                window.request_close();
                return;
            }
        };

        let atlases = match AtlasSet::new(&renderer.gpu, 64) {
            Ok(atlases) => Arc::new(atlases),
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create atlas set:\n{}",
                    error.diagnostic_report()
                );
                swapchain.destroy(&renderer.gpu);
                surface.destroy(&renderer.gpu);
                renderer.destroy();
                window.request_close();
                return;
            }
        };

        if let Err(error) = renderer.install_atlases(atlases.clone()) {
            eprintln!(
                "[unbound-app] Could not install atlases:\n{}",
                error.diagnostic_report()
            );
            drop(atlases);
            swapchain.destroy(&renderer.gpu);
            surface.destroy(&renderer.gpu);
            renderer.destroy();
            window.request_close();
            return;
        }

        self.atlases = Some(atlases);
        self.swapchain = Some(swapchain);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.hwnd = Some(hwnd);
        if let Some(cb) = &mut self.on_hwnd_created {
            cb(hwnd);
        }

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
                let ui_scale = self.runtime.context().ui_scale.max(0.5);
                self.runtime.resize(Vec2::new(
                    extent.width as f32 / ui_scale,
                    extent.height as f32 / ui_scale,
                ));
                self.swapchain_dirty = true;
            }
            PlatformEvent::RecreateSwapchain(_) => {
                self.swapchain_dirty = true;
            }
            PlatformEvent::CursorMoved { x, y } => {
                let ui_scale = self.runtime.context().ui_scale.max(0.5);
                self.pointer = Vec2::new(x as f32 / ui_scale, y as f32 / ui_scale);
                self.runtime.pointer_move(self.pointer);
            }
            PlatformEvent::MouseInput { state, button } => {
                if let Some(btn) = translate_pointer_button(button) {
                    if state == ButtonState::Pressed {
                        self.runtime.pointer_down(self.pointer, btn);
                    } else {
                        self.runtime.pointer_up(self.pointer, btn);
                    }
                    for cmd in self.runtime.drain_window_commands() {
                        match cmd {
                            crate::WindowCommand::StartDrag => {
                                let _ = window.drag_window();
                            }
                            crate::WindowCommand::Minimize => {
                                window.set_minimized(true);
                            }
                            crate::WindowCommand::ToggleMaximize => {
                                window.set_maximized(!window.is_maximized());
                            }
                            crate::WindowCommand::Close => {
                                window.request_close();
                                return Flow::Exit;
                            }
                        }
                    }
                }
            }
            PlatformEvent::MouseWheel { delta_x, delta_y } => {
                if self.modifiers.control {
                    let step = if delta_y > 0.0 {
                        1
                    } else if delta_y < 0.0 {
                        -1
                    } else {
                        0
                    };
                    if step != 0 {
                        let next_scale = self.runtime.context_mut().step_ui_scale(step);
                        let size = window.inner_size();
                        self.runtime.resize(Vec2::new(
                            size.width.max(1) as f32 / next_scale,
                            size.height.max(1) as f32 / next_scale,
                        ));
                        window.request_redraw();
                    }
                } else {
                    self.runtime
                        .wheel(self.pointer, Vec2::new(delta_x as f32, delta_y as f32));
                }
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
            PlatformEvent::Focused(focused) => {
                self.runtime.set_window_focused(focused);
            }
            PlatformEvent::CloseRequested => {
                self.destroy_vulkan();
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

        // 标题栏在构建视图时读取该状态，必须在 `runtime.frame` 之前同步，避免滞后一帧。
        self.runtime.context_mut().window_maximized = window.is_maximized();
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
                        if self.scene_generation == 0 || report.diff.bumps_generation() {
                            self.scene_generation = self.scene_generation.wrapping_add(1).max(1);
                        }
                        let ui_scale = self.runtime.context().ui_scale.max(0.5);
                        let expected_vp =
                            Vec2::new(width as f32 / ui_scale, height as f32 / ui_scale);
                        if (self.runtime.context().viewport - expected_vp).length() > 0.5 {
                            self.runtime.resize(expected_vp);
                        }
                        let camera = Camera::ui((expected_vp.x, expected_vp.y), ui_scale);
                        let backdrops: Vec<unbound_gpu::frame::BackdropRequest> = report
                            .backdrops
                            .iter()
                            .map(|b| unbound_gpu::frame::BackdropRequest {
                                rect: [
                                    b.rect.origin.x * ui_scale,
                                    b.rect.origin.y * ui_scale,
                                    b.rect.size.x * ui_scale,
                                    b.rect.size.y * ui_scale,
                                ],
                                radius: b.blur * ui_scale,
                                corner_radii: [0.0; 4],
                                layer_key_prefix: (PassLayer::Overlay as u64) << 56,
                            })
                            .collect();
                        let scene_data = self.runtime.scene();
                        let scene = SceneUpload {
                            primitives: &scene_data.primitives,
                            transforms: &scene_data.transforms,
                            clips: &scene_data.clips,
                            payloads: &scene_data.payloads,
                            generation: self.scene_generation,
                            camera,
                            key_prefix: (PassLayer::Ui as u64) << 56,
                            dirty_ranges: report.diff.ranges(),
                            aux_changed: report.aux_changed,
                            backdrops: &backdrops,
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

                        match swapchain.present_outcome(
                            &renderer.gpu,
                            image.index,
                            image.render_finished,
                        ) {
                            Ok(unbound_gpu::surface::PresentOutcome::Optimal) => {}
                            Ok(unbound_gpu::surface::PresentOutcome::Recreate) => {
                                self.swapchain_dirty = true;
                            }
                            Err(ref err) => {
                                if is_device_lost(err) {
                                    eprintln!("[unbound-app] GPU device lost during present. Triggering recovery...");
                                    self.recover_gpu(window);
                                    return Flow::Continue;
                                }
                                self.swapchain_dirty = true;
                            }
                        }
                    }
                    Ok(AcquireOutcome::OutOfDate) => {
                        self.swapchain_dirty = true;
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

        if let Some(metrics) = &self.frame_metrics {
            let scale = window.scale_factor().max(0.5) as f32;
            let ui_scale = self.runtime.context().ui_scale.max(0.5);
            metrics.set_scale_factor(scale);
            // 保持 FrameHitMetrics 的 title_bar_height 为 0：标题栏拖拽与双击最大化
            // 全部由 UI 层冒泡兜底产出 `WindowCommand`，Win32 不再争夺标题栏内部交互。
            metrics.set_title_bar_height(0.0);
            // 仅把「最大化」按钮热区交给 Win32：`HTMAXBUTTON` 是 Win11 Snap Layout
            // 悬停贴靠菜单的唯一钩子。最小化 / 关闭必须留在客户区，否则 Windows 会把
            // 点击转成 `WM_NCLBUTTONUP` + `SC_MINIMIZE` / `SC_CLOSE`，应用层再也拦不到关闭。
            let (_, max_id, _) = self.runtime.primary_title_bar_buttons().unwrap_or((
                unbound_widgets::TITLE_BAR_MINIMIZE_ID,
                unbound_widgets::TITLE_BAR_MAXIMIZE_ID,
                unbound_widgets::TITLE_BAR_CLOSE_ID,
            ));
            let max_rect = self
                .runtime
                .widget_rect(max_id)
                .map(|rect| FrameHitMetrics::ui_rect_to_frame_logical(rect, ui_scale, scale))
                .unwrap_or(Rect::ZERO);
            metrics.set_maximize_button_rect(max_rect);
            metrics.set_minimize_button_rect(Rect::ZERO);
            metrics.set_close_button_rect(Rect::ZERO);
        }

        if report.dirty.needs_rebuild() {
            window.request_redraw();
        }

        Flow::Continue
    }

    fn window_closing(&mut self, _window: &Window) {
        self.destroy_vulkan();
    }
}

impl<A: Application> Drop for WindowedRunner<A> {
    fn drop(&mut self) {
        self.destroy_vulkan();
    }
}

/// 单行窗口化应用入口函数。
pub fn run_windowed<A: Application + 'static>(
    app: A,
    mut config: WindowConfig,
) -> Result<(), PlatformError> {
    // `WindowedRunner` 一律自带客户端自绘标题栏并安装自定义边框子类，
    // 因此必须按无边框窗口创建。此前沿用 `WindowConfig::default()` 的
    // `decorations = true`，样式上仍带 `WS_CAPTION`，只是被子类事后把非客户区
    // 削成 0；配置与外观不一致，且子类安装前可能露出系统原生标题栏。
    config.decorations = false;
    let runner = WindowedRunner::new(app, &config);
    PlatformWindow::new(&config)?.run(runner)
}

/// 带原生窗口句柄（`HWND`）就绪回调的单行窗口化应用入口函数。
pub fn run_windowed_with_hwnd<A: Application + 'static>(
    app: A,
    mut config: WindowConfig,
    on_hwnd: impl FnMut(isize) + Send + 'static,
) -> Result<(), PlatformError> {
    config.decorations = false;
    let runner = WindowedRunner::new(app, &config).with_hwnd_callback(on_hwnd);
    PlatformWindow::new(&config)?.run(runner)
}

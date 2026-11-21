//! 窗口化应用宿主与运行器，将 [`AppRuntime`] 适配至 [`PlatformApp`]。

use std::sync::Arc;
use std::time::Instant;

use rustc_hash::FxHashMap;
use unbound_gpu::atlas::AtlasSet;
use unbound_gpu::debug::{is_device_lost, recover_with, RecoveryPolicy};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, Surface, Swapchain};
use unbound_gpu::swapchain::{CompositeAlphaMode, PresentConfig};
use unbound_gpu::{Camera, ManagedExternalTextureDesc, Renderer};
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::win32_frame::{
    install_custom_frame, FrameHitMetrics, WindowBackdropMaterial,
};
use unbound_platform::window::{
    ButtonState, ImeEvent, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey,
    PlatformState, SurfaceAlpha, WindowConfig,
};
use unbound_platform::{
    begin_outbound_file_drag, OutboundFileDragOutcome, PopupHostRegistry, PopupWindowConfig,
    Window, WindowId,
};
use unbound_theme::{ThemeSet, ThemeState};

use crate::appearance::{AppearanceCaps, AppearanceDiff, WindowAppearance};
use crate::{
    AppKey, AppRuntime, Application, ExternalTextureHandle, ExternalTextureOp, ImeUpdate,
    Modifiers as AppModifiers, ThemeKind,
};

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

/// 按表面能力解析交换链合成方式。
///
/// 请求预乘 alpha 但驱动不支持时回退为不透明：宁可窗口不透明，也不能让内容以
/// 未定义的 alpha 语义参与合成。回退只影响观感（原生材质被内容盖住），不会让
/// 窗口变成「透明却没有材质」的空壳。
fn resolve_composite_alpha(
    requested: SurfaceAlpha,
    surface: &Surface,
    gpu: &Gpu,
) -> CompositeAlphaMode {
    match requested {
        SurfaceAlpha::Opaque => CompositeAlphaMode::Opaque,
        SurfaceAlpha::PreMultiplied => {
            if surface.supports_premultiplied_alpha(gpu).unwrap_or(false) {
                CompositeAlphaMode::PreMultiplied
            } else {
                CompositeAlphaMode::Opaque
            }
        }
    }
}

/// 将平台后端唤醒闭包适配为运行时 [`crate::AppWaker`] 的桥接器。
struct SignalWaker(Arc<dyn Fn() + Send + Sync>);
impl core::fmt::Debug for SignalWaker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SignalWaker")
    }
}

impl crate::AppWaker for SignalWaker {
    fn wake(&self) {
        (self.0)();
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

/// 每帧开始时执行的宿主钩子；在 UI 线程上拿到运行时的可变引用。
type FrameHook<A> = Box<dyn FnMut(&mut AppRuntime<A>) + Send>;

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
    /// 请求的表面合成方式；每条交换链创建时按表面能力解析一次。
    ///
    /// 它必须与 `WindowConfig` 一致并贯穿整个窗口生命周期：交换链一旦按不透明
    /// 合成建出来，原生后景的内容层就再也透不出来，只能靠重建窗口纠正。
    surface_alpha: SurfaceAlpha,
    scene_generation: u64,
    /// 上一帧是否已执行场景构建但未能送达 GPU（例如交换链过期或获取图像失败）。
    ///
    /// 运行时每次构建都会推进 diff 基准；若上一帧被丢弃，下一帧必须推进代次并执行全量上传。
    frame_dropped: bool,
    /// 当前窗口的系统物理显示 DPI 缩放比（`window.scale_factor()`）。
    os_scale: f32,
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
    /// 每帧开始时的可选宿主回调，拿到运行时的可变引用。
    ///
    /// 这条钩子存在只有一个理由：**Agent 能力必须住在 UI 线程上**。
    /// 界面状态不离开 UI 线程，跨线程的只有纯值请求与回执——这样
    /// Agent 操作界面时既不抢真实鼠标，也不会把渲染栈的线程约束
    /// 传染给业务代码。
    on_frame: Option<FrameHook<A>>,
    /// 窗口外观意图账本。
    appearance: WindowAppearance,
    /// 已上传至当前渲染器的外部纹理句柄映射：`handle -> (slot, width, height, srgb)`。
    external_gpu_slots: FxHashMap<ExternalTextureHandle, (u32, u32, u32, bool)>,
    /// 主窗口平台 ID（用于与原生弹出窗区分，避免弹出窗覆写或销毁主窗口渲染资源）。
    primary_window_id: Option<WindowId>,
    /// 主窗口客户区左上角在屏幕坐标系中的物理像素原点。
    main_screen_origin: (i32, i32),
    /// 已创建的原生弹出窗 `WindowId -> popup_id` 映射。
    native_popup_windows: FxHashMap<WindowId, u64>,
    /// 越界原生弹出窗状态注册表。
    popup_host: PopupHostRegistry,
    /// 待平台后端在事件循环安全点同步的越界弹出窗请求队列。
    pending_popup_syncs: Vec<(u64, PopupWindowConfig)>,
    /// 待平台后端显式关闭的越界弹出窗 ID 队列。
    pending_popup_closes: Vec<u64>,
    /// 上一帧是否处于最小化/遮挡暂停态，用于在恢复可见后强制刷新首帧。
    was_suspended: bool,
}

impl<A: Application> WindowedRunner<A> {
    /// 为 `app` 创建新的窗口运行器。
    pub fn new(app: A, config: &WindowConfig) -> Self {
        let (w, h) = (config.width.max(1), config.height.max(1));
        let viewport = Vec2::new(w as f32, h as f32);
        let mut runtime = AppRuntime::new(app, viewport);
        runtime.set_backdrop(config.backdrop);
        runtime.set_clipboard(Box::new(WinClipboard(std::cell::RefCell::new(
            unbound_platform::SystemClipboard::new(),
        ))));
        let ctx = runtime.context();
        let theme = ThemeState::new(ThemeSet::for_product(ctx.preset, ctx.accent), ctx.theme);

        // 根据 ADR 0014 与建窗契约：建窗计算 surface_alpha 时，只有当配置中显式意图开启原生后景
        // （config.backdrop 存在且非 WindowBackdropMaterial::None）时，才强制覆盖为 PreMultiplied；
        // 当意图关闭（None 或 Some(WindowBackdropMaterial::None)）时，严格保持 config.surface_alpha（默认 Opaque），
        // 保证改造前后逐字节一致，不破坏原有不透明交换链。
        let surface_alpha = match config.backdrop {
            Some(material) if material != WindowBackdropMaterial::None => {
                SurfaceAlpha::PreMultiplied
            }
            _ => config.surface_alpha,
        };

        Self {
            runtime,
            theme,
            uploaded_theme: None,
            on_hwnd_created: None,
            on_frame: None,
            renderer: None,
            surface: None,
            swapchain: None,
            atlases: None,
            pointer: Vec2::ZERO,
            modifiers: Modifiers::default(),
            last_frame: Instant::now(),
            swapchain_dirty: false,
            committed_request: (w, h),
            surface_alpha,
            scene_generation: 0,
            frame_dropped: false,
            os_scale: 1.0,
            frame_metrics: None,
            hwnd: None,
            appearance: WindowAppearance::new(),
            external_gpu_slots: FxHashMap::default(),
            primary_window_id: None,
            main_screen_origin: (0, 0),
            native_popup_windows: FxHashMap::default(),
            popup_host: PopupHostRegistry::new(),
            pending_popup_syncs: Vec::new(),
            pending_popup_closes: Vec::new(),
            was_suspended: false,
        }
    }

    /// 注册原生窗口句柄（Win32 `HWND`）就绪时的回调函数。
    #[must_use]
    pub fn with_hwnd_callback(mut self, callback: impl FnMut(isize) + Send + 'static) -> Self {
        self.on_hwnd_created = Some(Box::new(callback));
        self
    }

    /// 注册每帧回调，回调拿到运行时的可变引用。
    ///
    /// 回调在每帧的**最前面**执行——此时上一帧的场景已经提交，
    /// 本帧尚未构建。Agent 能力在这里排空跨线程请求：动作会走完整的
    /// 事件管线（合成输入 → 归约 → 重建），语义树与像素随之更新。
    #[must_use]
    pub fn with_frame_callback(
        mut self,
        callback: impl FnMut(&mut AppRuntime<A>) + Send + 'static,
    ) -> Self {
        self.on_frame = Some(Box::new(callback));
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

    /// 同步窗口当前系统 DPI 缩放因子（`os_scale`）。
    fn sync_os_scale(&mut self, window: &Window) -> f32 {
        let raw = window.scale_factor() as f32;
        if raw.is_finite() && raw > 0.0 {
            self.os_scale = raw.clamp(0.5, 4.0);
        }
        self.os_scale
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

    /// 将外观意图账本同步至窗口与渲染器。
    fn sync_appearance(&mut self, _window: &Window) {
        let Some(hwnd) = self.hwnd else {
            return;
        };
        let caps = AppearanceCaps::new(
            unbound_platform::windows_build(),
            self.runtime.premultiplied_surface(),
            unbound_platform::accent_policy_export_available(),
        );
        let theme = self.runtime.context().theme;
        self.appearance
            .set_host_clear_alpha(self.runtime.clear_alpha());
        let diff = self
            .appearance
            .set_intent(self.runtime.backdrop_intent(), caps, theme);
        self.apply_appearance_diff(hwnd, diff);
    }

    /// 执行外观变更 Diff。
    fn apply_appearance_diff(&mut self, hwnd: isize, diff: AppearanceDiff) {
        crate::appearance::apply_appearance_diff_to_platform(
            &diff,
            hwnd,
            self.frame_metrics.as_deref(),
        );
        if let Some(clear_alpha) = diff.clear_alpha {
            if let Some(renderer) = &mut self.renderer {
                renderer.set_clear_alpha(clear_alpha);
            }
        }
    }

    /// 判断给定平台窗口是否为原生弹出窗并返回其业务浮层 ID。
    fn resolve_popup_id(&self, window: &Window) -> Option<u64> {
        if let Some(id) = window.popup_id() {
            return Some(id);
        }
        let win_id = WindowId(window.id().0 as u64);
        self.native_popup_windows
            .get(&win_id)
            .copied()
            .or_else(|| self.popup_host.get_by_window(win_id).map(|e| e.id))
    }

    /// 将原生弹出窗内的局部物理像素指针坐标按屏幕原点偏移换算为主窗口逻辑坐标。
    #[must_use]
    pub fn map_popup_pointer_to_main_logical(
        &self,
        popup_screen_rect: unbound_platform::ScreenRect,
        local_x: f64,
        local_y: f64,
    ) -> Vec2 {
        let effective_scale = self.runtime.context().effective_scale().max(0.01);
        let main_phys_x = local_x as f32 + (popup_screen_rect.x - self.main_screen_origin.0) as f32;
        let main_phys_y = local_y as f32 + (popup_screen_rect.y - self.main_screen_origin.1) as f32;
        Vec2::new(main_phys_x / effective_scale, main_phys_y / effective_scale)
    }

    /// 判断当前帧是否因窗口最小化或遮挡暂停而应跳过构建与交换链提交。
    ///
    /// 从暂停态恢复可见时会自动置位 `frame_dropped` 并失效运行时脏状态，
    /// 确保恢复后的首帧立即产出完整画面且不丢失期间积累的脏状态。
    pub fn should_skip_frame_for_throttle(&mut self, state: &PlatformState) -> bool {
        if state.pacer.is_suspended() {
            self.was_suspended = true;
            return true;
        }
        if self.was_suspended {
            self.was_suspended = false;
            self.frame_dropped = true;
            self.runtime.invalidate();
        }
        false
    }

    /// 排空应用运行时产生的窗口控制命令并执行平台动作。
    fn drain_runtime_commands(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        let mut outbound_drag = None;
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
                crate::WindowCommand::SetThrottlePolicy(policy) => {
                    state.pacer.set_throttle_policy(policy);
                }
                crate::WindowCommand::BeginExternalFileDrag(request) => {
                    outbound_drag = Some(request);
                }
                crate::WindowCommand::SyncNativePopup { id, config } => {
                    if self.popup_host.sync_popup(id, config).is_some() {
                        self.pending_popup_syncs.push((id, config));
                    }
                }
                crate::WindowCommand::CloseNativePopup { id } => {
                    if self.popup_host.close_popup(id).is_some() {
                        self.pending_popup_closes.push(id);
                    }
                }
                crate::WindowCommand::OpenWindow(_) => {
                    eprintln!(
                        "[unbound-app] WindowCommand::OpenWindow requires MultiWindowCoordinator"
                    );
                }
                crate::WindowCommand::CloseWindow(target) => {
                    if target == WindowId(window.id().0 as u64) {
                        window.request_close();
                        return Flow::Exit;
                    }
                }
            }
        }
        if let Some(request) = outbound_drag {
            let _ = self.runtime.cancel_pointer();
            let outcome = match begin_outbound_file_drag(&request) {
                Ok(outcome) => outcome,
                Err(error) => {
                    eprintln!("[unbound-app] Outbound OLE file drag failed: {error:?}");
                    OutboundFileDragOutcome::Cancelled
                }
            };
            let _ = self.runtime.notify_external_file_drag_finished(outcome);
            state.pacer.note_activity(Instant::now());
            window.request_redraw();
        }
        Flow::Continue
    }

    fn process_external_texture_ops(&mut self, ops: Vec<ExternalTextureOp>, window: &Window) {
        if ops.is_empty() {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let mut needs_redraw = false;
        for op in ops {
            match op {
                ExternalTextureOp::Upload {
                    handle,
                    width,
                    height,
                    srgb,
                    rgba,
                } => {
                    let desc = ManagedExternalTextureDesc::new(width, height, srgb);
                    match renderer.upload_external_rgba8(desc, &rgba) {
                        Ok(slot) => {
                            self.external_gpu_slots
                                .insert(handle, (slot, width, height, srgb));
                            self.runtime
                                .asset_hub_mut()
                                .commit_external_texture_slot(handle, Ok(slot));
                            needs_redraw = true;
                        }
                        Err(error) => {
                            eprintln!(
                                "[unbound-app] Could not upload external texture {handle:?} ({width}x{height}): {error:?}"
                            );
                            self.runtime
                                .asset_hub_mut()
                                .commit_external_texture_slot(handle, Err(format!("{error:?}")));
                            needs_redraw = true;
                        }
                    }
                }
                ExternalTextureOp::Update {
                    handle,
                    width,
                    height,
                    rgba,
                } => {
                    let Some((slot, cur_w, cur_h, srgb)) =
                        self.external_gpu_slots.get(&handle).copied()
                    else {
                        eprintln!(
                            "[unbound-app] Skipping update for unbound external texture {handle:?}"
                        );
                        continue;
                    };
                    if cur_w == width && cur_h == height {
                        if let Err(error) = renderer.update_external_rgba8(slot, &rgba) {
                            eprintln!(
                                "[unbound-app] Could not update external texture {handle:?} (slot {slot}): {error:?}"
                            );
                            self.runtime
                                .asset_hub_mut()
                                .commit_external_texture_slot(handle, Err(format!("{error:?}")));
                        }
                        needs_redraw = true;
                    } else {
                        if let Err(error) = renderer.destroy_external_texture(slot) {
                            eprintln!(
                                "[unbound-app] Could not destroy resized external texture {handle:?} (slot {slot}): {error:?}"
                            );
                        }
                        self.external_gpu_slots.remove(&handle);
                        let desc = ManagedExternalTextureDesc::new(width, height, srgb);
                        match renderer.upload_external_rgba8(desc, &rgba) {
                            Ok(new_slot) => {
                                self.external_gpu_slots
                                    .insert(handle, (new_slot, width, height, srgb));
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_external_texture_slot(handle, Ok(new_slot));
                            }
                            Err(error) => {
                                eprintln!(
                                    "[unbound-app] Could not re-upload resized external texture {handle:?} ({width}x{height}): {error:?}"
                                );
                                self.runtime.asset_hub_mut().commit_external_texture_slot(
                                    handle,
                                    Err(format!("{error:?}")),
                                );
                            }
                        }
                        needs_redraw = true;
                    }
                }
                ExternalTextureOp::Release { handle } => {
                    if let Some((slot, _, _, _)) = self.external_gpu_slots.remove(&handle) {
                        if let Err(error) = renderer.destroy_external_texture(slot) {
                            eprintln!(
                                "[unbound-app] Could not destroy external texture {handle:?} (slot {slot}): {error:?}"
                            );
                        }
                    }
                }
            }
        }
        if needs_redraw {
            self.runtime.invalidate();
            window.request_redraw();
        }
    }

    fn destroy_vulkan(&mut self) {
        self.external_gpu_slots.clear();
        self.runtime
            .asset_hub_mut()
            .invalidate_all_external_textures();
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
        self.external_gpu_slots.clear();
        self.runtime
            .asset_hub_mut()
            .invalidate_all_external_textures();
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
        let requested_alpha = self.surface_alpha;
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
            let composite_alpha = resolve_composite_alpha(requested_alpha, &surface, &gpu);
            let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false)
                .with_composite_alpha(composite_alpha);
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
            let premultiplied = composite_alpha == CompositeAlphaMode::PreMultiplied;
            Ok((renderer, swapchain, surface, atlases, premultiplied))
        });

        match recovered {
            Ok(((renderer, swapchain, surface, atlases, premultiplied), attempts)) => {
                eprintln!("[unbound-app] GPU recovered successfully after {attempts} attempt(s).");
                self.renderer = Some(renderer);
                self.swapchain = Some(swapchain);
                self.surface = Some(surface);
                self.atlases = Some(atlases);
                self.runtime.set_premultiplied_surface(premultiplied);
                self.swapchain_dirty = true;
                self.frame_dropped = true;
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
    fn attach_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.runtime.set_waker(Arc::new(SignalWaker(waker)));
    }

    fn next_wake(&self) -> Option<Instant> {
        self.runtime
            .next_wake_in()
            .map(|delay| Instant::now() + delay)
    }

    fn take_popup_sync_requests(&mut self) -> Vec<(u64, PopupWindowConfig)> {
        core::mem::take(&mut self.pending_popup_syncs)
    }

    fn take_popup_close_requests(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.pending_popup_closes)
    }

    fn window_created(&mut self, state: &mut PlatformState, window: &Window) {
        if let Some(popup_id) = self.resolve_popup_id(window) {
            let win_id = WindowId(window.id().0 as u64);
            self.native_popup_windows.insert(win_id, popup_id);
            let _ = self.popup_host.bind_native_window(popup_id, win_id);
            return;
        }
        if self.surface.is_some() {
            return;
        }
        self.resumed(state, window);
    }

    fn resumed(&mut self, state: &mut PlatformState, window: &Window) {
        if self.resolve_popup_id(window).is_some() || self.surface.is_some() {
            return;
        }
        self.primary_window_id = Some(WindowId(window.id().0 as u64));
        let client_screen = window.client_rect_on_screen();
        self.main_screen_origin = (client_screen.x, client_screen.y);
        let os_scale = self.sync_os_scale(window);
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        self.committed_request = (width, height);
        self.runtime
            .sync_physical_viewport(Vec2::new(width as f32, height as f32), os_scale);

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

        let composite_alpha = resolve_composite_alpha(self.surface_alpha, &surface, &gpu);
        self.runtime
            .set_premultiplied_surface(composite_alpha == CompositeAlphaMode::PreMultiplied);
        let present = PresentConfig::from_capabilities(&gpu.enabled_device_extensions, false)
            .with_composite_alpha(composite_alpha);
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

        let (chain_w, chain_h) = swapchain.extent();
        self.runtime.sync_physical_viewport(
            Vec2::new(chain_w.max(1) as f32, chain_h.max(1) as f32),
            os_scale,
        );

        self.atlases = Some(atlases);
        self.swapchain = Some(swapchain);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.hwnd = Some(hwnd);
        if let Some(cb) = &mut self.on_hwnd_created {
            cb(hwnd);
        }

        let metrics = Arc::new(FrameHitMetrics::new());
        metrics.set_scale_factor(os_scale);
        install_custom_frame(hwnd, metrics.clone());
        self.frame_metrics = Some(metrics);

        state.set_present_extensions(&device_extensions);
        self.sync_theme(window);
        self.sync_appearance(window);
        self.last_frame = Instant::now();
    }

    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow {
        if let Some(popup_id) = self.resolve_popup_id(window) {
            let popup_rect = {
                let live = window.client_rect_on_screen();
                if live.width > 0 && live.height > 0 {
                    live
                } else {
                    self.popup_host
                        .get(popup_id)
                        .map_or(live, |entry| entry.config.screen_rect)
                }
            };
            match event {
                PlatformEvent::CursorMoved { x, y } => {
                    self.pointer = self.map_popup_pointer_to_main_logical(popup_rect, x, y);
                    state.pacer.note_activity(Instant::now());
                    self.runtime.pointer_move(self.pointer);
                }
                PlatformEvent::MouseInput {
                    state: btn_state,
                    button,
                } => {
                    if let Some(btn) = translate_pointer_button(button) {
                        if btn_state == ButtonState::Pressed {
                            self.runtime.pointer_down(self.pointer, btn);
                        } else {
                            self.runtime.pointer_up(self.pointer, btn);
                        }
                        state.pacer.note_activity(Instant::now());
                    }
                }
                PlatformEvent::MouseWheel { delta_x, delta_y } => {
                    state.pacer.note_activity(Instant::now());
                    self.runtime
                        .wheel(self.pointer, Vec2::new(delta_x as f32, delta_y as f32));
                }
                PlatformEvent::CloseRequested => {
                    let _ = self.popup_host.close_popup(popup_id);
                    let _ = self.runtime.notify_native_popup_dismissed(popup_id);
                    state.pacer.note_activity(Instant::now());
                }
                _ => {}
            }
            if self.drain_runtime_commands(state, window) == Flow::Exit {
                return Flow::Exit;
            }
            return Flow::Continue;
        }
        let os_scale = self.sync_os_scale(window);
        match event {
            PlatformEvent::Resized(_) => {
                // 始终读取窗口当前实时客户区尺寸（而非事件中可能滞后的历史尺寸），
                // 并且仅标记 `swapchain_dirty = true`，不在交换链真正重建前提前覆写 `committed_request`。
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    self.runtime.sync_physical_viewport(
                        Vec2::new(size.width as f32, size.height as f32),
                        os_scale,
                    );
                    self.swapchain_dirty = true;
                }
            }
            PlatformEvent::RecreateSwapchain(_) => {
                self.swapchain_dirty = true;
            }
            PlatformEvent::CursorMoved { x, y } => {
                let effective_scale = self.runtime.context().effective_scale();
                self.pointer = Vec2::new(x as f32 / effective_scale, y as f32 / effective_scale);
                state.pacer.note_activity(Instant::now());
                self.runtime.pointer_move(self.pointer);
            }
            PlatformEvent::MouseInput {
                state: input_state,
                button,
            } => {
                if let Some(btn) = translate_pointer_button(button) {
                    if input_state == ButtonState::Pressed {
                        self.runtime.pointer_down(self.pointer, btn);
                    } else {
                        self.runtime.pointer_up(self.pointer, btn);
                    }
                    state.pacer.note_activity(Instant::now());
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
                        let _ = self.runtime.context_mut().step_ui_scale(step);
                        let size = window.inner_size();
                        self.runtime.sync_physical_viewport(
                            Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32),
                            os_scale,
                        );
                        window.request_redraw();
                    }
                } else {
                    self.runtime
                        .wheel(self.pointer, Vec2::new(delta_x as f32, delta_y as f32));
                }
            }
            PlatformEvent::KeyInput {
                state: input_state,
                key,
                ..
            } => {
                let app_k = translate_app_key(&key);
                let pressed = input_state == ButtonState::Pressed;
                let modifiers = translate_modifiers(self.modifiers);
                state.pacer.note_activity(Instant::now());
                self.runtime.key(app_k, pressed, modifiers);
            }
            PlatformEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
            }
            PlatformEvent::Ime(ime) => match ime {
                ImeEvent::Preedit(text, selection) => {
                    self.runtime.ime(ImeUpdate {
                        enabled: true,
                        preedit: text,
                        cursor_range: selection,
                        commit: None,
                        cancelled: false,
                    });
                }
                ImeEvent::Commit(text) => {
                    self.runtime.ime(ImeUpdate {
                        enabled: false,
                        preedit: String::new(),
                        cursor_range: None,
                        commit: Some(text),
                        cancelled: false,
                    });
                }
                ImeEvent::Disabled => {
                    self.runtime.ime(ImeUpdate {
                        enabled: false,
                        preedit: String::new(),
                        cursor_range: None,
                        commit: None,
                        cancelled: true,
                    });
                }
                ImeEvent::Enabled => {}
            },
            PlatformEvent::ScaleFactorChanged(scale) => {
                let new_os_scale = (scale as f32).clamp(0.5, 4.0);
                self.os_scale = new_os_scale;
                if let Some(metrics) = &self.frame_metrics {
                    metrics.set_scale_factor(new_os_scale);
                }
                let size = window.inner_size();
                self.runtime.sync_physical_viewport(
                    Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32),
                    new_os_scale,
                );
                self.swapchain_dirty = true;
                window.request_redraw();
            }
            PlatformEvent::ThemeChanged { dark } => {
                // 非客户区永远跟随系统（标题栏/边框由 DWM 绘制），
                // 内容主题只在应用声明「跟随系统」时才切换。
                if self.runtime.follows_system_theme() {
                    self.runtime.set_theme(if dark {
                        ThemeKind::Dark
                    } else {
                        ThemeKind::Light
                    });
                }
                self.sync_appearance(window);
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
                state.pacer.set_focused(focused);
                if focused {
                    state.pacer.note_activity(Instant::now());
                }
                self.runtime.set_window_focused(focused);
            }
            PlatformEvent::Minimized(minimized) => {
                state.pacer.set_minimized(minimized);
                if !minimized {
                    self.frame_dropped = true;
                    state.pacer.request_immediate(Instant::now());
                    window.request_redraw();
                }
            }
            PlatformEvent::Occluded(occluded) => {
                state.pacer.set_occluded(occluded);
                if !occluded {
                    self.frame_dropped = true;
                    state.pacer.request_immediate(Instant::now());
                    window.request_redraw();
                }
            }
            PlatformEvent::Moved { x, y } => {
                self.main_screen_origin = (x, y);
                self.runtime
                    .set_window_screen_origin((x, y), WindowId(window.id().0 as u64));
            }
            PlatformEvent::PopupDismissed { id } => {
                let _ = self.popup_host.close_popup(id);
                let _ = self.runtime.notify_native_popup_dismissed(id);
                state.pacer.note_activity(Instant::now());
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

        if self.drain_runtime_commands(state, window) == Flow::Exit {
            return Flow::Exit;
        }

        if self.runtime.dirty() != crate::DirtyFlags::NONE && !state.pacer.is_suspended() {
            state.pacer.request_immediate(Instant::now());
            window.request_redraw();
        }

        Flow::Continue
    }

    fn frame(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        if self.resolve_popup_id(window).is_some() {
            return Flow::Continue;
        }
        if self.should_skip_frame_for_throttle(state) {
            return Flow::Continue;
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Flow::Continue;
        }
        let client_screen = window.client_rect_on_screen();
        self.main_screen_origin = (client_screen.x, client_screen.y);
        self.runtime.set_window_screen_origin(
            (client_screen.x, client_screen.y),
            WindowId(window.id().0 as u64),
        );
        let (width, height) = (size.width.max(1), size.height.max(1));
        let os_scale = self.sync_os_scale(window);
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        // 1. 在构建场景与求解布局*之前*先检查并重建交换链（对齐 `Showcase::frame_impl`）：
        //    驱动决定交换链图像的真实呈现尺寸，提前重建可确保本帧直接按最终新尺寸布局与渲染，
        //    杜绝“先按旧视口排版一帧、渲染阶段才发现视口不一致”的单帧滞后与反复重排。
        let settled = state.poll_resize(now);
        if self.swapchain_dirty || settled.is_some() || self.committed_request != (width, height) {
            if let (Some(renderer), Some(swapchain), Some(surface)) =
                (&mut self.renderer, &mut self.swapchain, &self.surface)
            {
                // 合成方式跟随窗口配置：原生后景需要预乘 alpha，且必须在每次重建时
                // 保持同一取值，否则驱动会按新方式重配表面、后景在重建瞬间被盖住。
                let composite_alpha =
                    resolve_composite_alpha(self.surface_alpha, surface, &renderer.gpu);
                let present = PresentConfig::from_capabilities(
                    &renderer.gpu.enabled_device_extensions,
                    false,
                )
                .with_composite_alpha(composite_alpha);
                if let Err(error) =
                    swapchain.recreate(&renderer.gpu, surface, width, height, &present)
                {
                    eprintln!("[unbound-app] Could not recreate swapchain: {error:?}");
                    self.swapchain_dirty = true;
                    return Flow::Continue;
                }
                self.committed_request = (width, height);
                self.swapchain_dirty = false;
                let (actual_w, actual_h) = swapchain.extent();
                if let Err(error) = renderer.ensure_depth_size(actual_w, actual_h) {
                    eprintln!("[unbound-app] Could not resize depth buffer: {error:?}");
                }
            }
        }

        // 2. 将运行时物理视口对齐到交换链已提交范围，并在 `runtime.frame` 之前同步最大化状态与帧时钟。
        let (chain_w, chain_h) = self
            .swapchain
            .as_ref()
            .map_or((width, height), Swapchain::extent);
        self.runtime.sync_physical_viewport(
            Vec2::new(chain_w.max(1) as f32, chain_h.max(1) as f32),
            os_scale,
        );
        self.runtime.context_mut().window_maximized = window.is_maximized();

        // 3. 宿主帧回调先于运行时推进执行：Agent 能力在这里排空跨线程请求，
        //    动作走完整事件管线后，本帧的场景构建就会带上变化。
        if let Some(callback) = self.on_frame.as_mut() {
            callback(&mut self.runtime);
        }

        let _ = self.runtime.tick(dt);
        let report = self.runtime.frame(0.0);
        if self.drain_runtime_commands(state, window) == Flow::Exit {
            return Flow::Exit;
        }

        // `reduce` 可能改动 `ctx.theme`，每帧检查 4 KiB LUT 是否需要上传。
        self.sync_theme(window);
        self.sync_appearance(window);

        self.process_external_texture_ops(report.external_texture_ops, window);

        if let (Some(renderer), Some(atlases)) = (&mut self.renderer, &self.atlases) {
            if !report.atlas_uploads.is_empty() {
                if let Err(error) = atlases.upload_batch(&renderer.gpu, &report.atlas_uploads) {
                    eprintln!("[unbound-app] Could not upload atlas glyphs: {error:?}");
                }
            }

            if let Some(swapchain) = &mut self.swapchain {
                if let Err(error) = renderer.wait_for_previous_frame() {
                    eprintln!("[unbound-app] Waiting for the previous frame failed: {error:?}");
                    self.frame_dropped = true;
                    return Flow::Continue;
                }

                match swapchain.acquire_outcome(1_000_000_000) {
                    Ok(AcquireOutcome::Ready { image, suboptimal }) => {
                        if suboptimal {
                            self.swapchain_dirty = true;
                        }
                        if self.scene_generation == 0
                            || report.diff.bumps_generation()
                            || self.frame_dropped
                        {
                            self.scene_generation = self.scene_generation.wrapping_add(1).max(1);
                        }
                        let dirty_ranges = if self.frame_dropped {
                            &[][..]
                        } else {
                            report.diff.ranges()
                        };
                        let (actual_w, actual_h) = swapchain.extent();
                        let effective_scale = self.runtime.context().effective_scale();
                        // `Camera::ui` 接收物理像素视口并在内部除以 `dpi_scale` 得到逻辑中心点；
                        // 此处必须传入交换链物理尺寸 `(actual_w, actual_h)`，严禁传入已除以缩放比的逻辑尺寸。
                        let camera =
                            Camera::ui((actual_w as f32, actual_h as f32), effective_scale);
                        let backdrops =
                            crate::collect_backdrops(&report.backdrops, effective_scale);
                        let scene_data = self.runtime.scene();
                        let scene = crate::scene_upload(
                            scene_data,
                            &backdrops,
                            camera,
                            self.scene_generation,
                            dirty_ranges,
                            report.aux_changed,
                        );

                        // 根背景清屏 alpha 由外观账本统一派生：原生后景要 0 让 DWM 材质
                        // 透出，自绘与纯色路径要 1。它只影响清屏色，不重配交换链。
                        let clear_alpha = self.appearance.applied_clear_alpha();
                        renderer.set_clear_alpha(clear_alpha);
                        if let Err(ref err) =
                            renderer.render_into_swapchain(&scene, &image, actual_w, actual_h)
                        {
                            self.frame_dropped = true;
                            if is_device_lost(err) {
                                eprintln!("[unbound-app] GPU device lost during render. Triggering recovery...");
                                self.recover_gpu(window);
                                return Flow::Continue;
                            }
                            eprintln!("[unbound-app] render failed: {err:?}");
                        } else {
                            self.frame_dropped = false;
                        }

                        match swapchain.present_outcome(
                            &renderer.gpu,
                            image.index,
                            image.render_finished,
                        ) {
                            Ok(unbound_gpu::surface::PresentOutcome::Optimal) => {}
                            Ok(unbound_gpu::surface::PresentOutcome::Recreate) => {
                                self.swapchain_dirty = true;
                                state.resize.mark_out_of_date(unbound_platform::Extent::new(
                                    actual_w, actual_h,
                                ));
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
                        state.pacer.on_present(Instant::now());
                    }
                    Ok(AcquireOutcome::OutOfDate) => {
                        self.swapchain_dirty = true;
                        self.frame_dropped = true;
                        state
                            .resize
                            .mark_out_of_date(unbound_platform::Extent::new(width, height));
                    }
                    Err(ref err) if is_device_lost(err) => {
                        self.frame_dropped = true;
                        eprintln!(
                            "[unbound-app] GPU device lost during acquire. Triggering recovery..."
                        );
                        self.recover_gpu(window);
                        return Flow::Continue;
                    }
                    _ => {
                        self.frame_dropped = true;
                    }
                }
            }
        }

        if let Some(metrics) = &self.frame_metrics {
            let effective_scale = self.runtime.context().effective_scale();
            metrics.set_scale_factor(os_scale);
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
                .map(|rect| {
                    FrameHitMetrics::ui_rect_to_frame_logical(rect, effective_scale, os_scale)
                })
                .unwrap_or(Rect::ZERO);
            metrics.set_maximize_button_rect(max_rect);
            metrics.set_minimize_button_rect(Rect::ZERO);
            metrics.set_close_button_rect(Rect::ZERO);
        }

        let keep_animating =
            self.runtime.context().scale_osd_remaining > 0.0 || report.dirty.needs_rebuild();
        state.pacer.set_animating(keep_animating);
        if keep_animating {
            window.request_redraw();
        }

        Flow::Continue
    }

    fn window_closing(&mut self, window: &Window) {
        let win_id = WindowId(window.id().0 as u64);
        if let Some(popup_id) = self
            .native_popup_windows
            .remove(&win_id)
            .or_else(|| self.resolve_popup_id(window))
        {
            let _ = self.popup_host.close_popup(popup_id);
            return;
        }
        if self.primary_window_id.is_some_and(|pid| pid != win_id) {
            return;
        }
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

/// 带宿主帧回调的窗口化应用入口函数。
///
/// 帧回调拿到运行时的可变引用，因此宿主可以在 UI 线程上做任何事——
/// 最典型的用法是排空 Agent 的跨线程请求。
pub fn run_windowed_with_frame_callback<A: Application + 'static>(
    app: A,
    mut config: WindowConfig,
    on_frame: impl FnMut(&mut AppRuntime<A>) + Send + 'static,
) -> Result<(), PlatformError> {
    config.decorations = false;
    let runner = WindowedRunner::new(app, &config).with_frame_callback(on_frame);
    PlatformWindow::new(&config)?.run(runner)
}

/// 同时带窗口句柄回调与宿主帧回调的窗口化应用入口函数。
///
/// 两个钩子各司其职：句柄回调在窗口创建后触发一次，帧回调每帧触发。
pub fn run_windowed_with_hooks<A: Application + 'static>(
    app: A,
    mut config: WindowConfig,
    on_hwnd: impl FnMut(isize) + Send + 'static,
    on_frame: impl FnMut(&mut AppRuntime<A>) + Send + 'static,
) -> Result<(), PlatformError> {
    config.decorations = false;
    let runner = WindowedRunner::new(app, &config)
        .with_hwnd_callback(on_hwnd)
        .with_frame_callback(on_frame);
    PlatformWindow::new(&config)?.run(runner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, AppEvent, DirtyFlags, EventOutcome, ViewBuilder};
    use unbound_platform::{Extent, PresentWaitPolicy, RenderThrottlePolicy};

    struct DummyApp;
    impl Application for DummyApp {
        type State = ();
        fn init(&mut self, _: &mut AppContext) {}
        fn reduce(&mut self, _: &mut (), _: AppEvent, _: &mut AppContext) -> EventOutcome {
            EventOutcome::new(DirtyFlags::NONE)
        }
        fn view(&mut self, _: &(), _: &mut ViewBuilder<'_>) {}
    }

    #[test]
    fn throttle_suspends_on_minimized_or_occluded_and_recovers_dirty_frame() {
        let config = WindowConfig::default();
        let mut runner = WindowedRunner::new(DummyApp, &config);
        let mut state = PlatformState::new(
            &config,
            Extent::new(800, 600),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state
            .pacer
            .set_throttle_policy(RenderThrottlePolicy::default());

        assert!(!runner.should_skip_frame_for_throttle(&state));

        state.pacer.set_minimized(true);
        assert!(runner.should_skip_frame_for_throttle(&state));
        assert!(runner.was_suspended);

        state.pacer.set_minimized(false);
        assert!(!runner.should_skip_frame_for_throttle(&state));
        assert!(!runner.was_suspended);
        assert!(runner.frame_dropped);
        assert_ne!(runner.runtime().dirty(), DirtyFlags::NONE);

        runner.frame_dropped = false;
        let _ = runner.runtime_mut().frame(0.0);
        state.pacer.set_occluded(true);
        assert!(runner.should_skip_frame_for_throttle(&state));
        state.pacer.set_occluded(false);
        assert!(!runner.should_skip_frame_for_throttle(&state));
        assert!(runner.frame_dropped);
    }

    #[test]
    fn popup_pointer_maps_with_screen_origin_offset_and_effective_scale() {
        let config = WindowConfig::default();
        let mut runner = WindowedRunner::new(DummyApp, &config);
        runner.main_screen_origin = (100, 200);
        runner
            .runtime_mut()
            .sync_physical_viewport(Vec2::new(1600.0, 1200.0), 2.0);
        let popup_rect = unbound_platform::ScreenRect::new(900, 300, 200, 160);
        let logical = runner.map_popup_pointer_to_main_logical(popup_rect, 40.0, 20.0);
        // (900 - 100 + 40) / 2.0 = 420.0, (300 - 200 + 20) / 2.0 = 60.0
        assert!((logical.x - 420.0).abs() < 1e-4);
        assert!((logical.y - 60.0).abs() < 1e-4);
    }
}

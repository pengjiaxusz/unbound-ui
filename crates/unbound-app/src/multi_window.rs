//! 多窗口共享 `VkDevice` 协调器运行时。
//!
//! 使同一进程内的主窗口与多个辅助窗口共享单一底层 [`Arc<Gpu>`] 设备与共享基础图集
//! [`Arc<AtlasSet>`]，同时为每个窗口维持独立的 [`AppRuntime`]、[`Surface`]、[`Swapchain`]、
//! [`Renderer`]、[`FramePacer`] 与事件路由。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use crate::theme_driver::{make_theme_snapshot_key, ThemeSnapshotKey, ThemeTransitionDriver};
use rustc_hash::FxHashMap;
use unbound_gpu::atlas::AtlasSet;
use unbound_gpu::debug::is_device_lost;
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, PresentOutcome, Surface, Swapchain};
use unbound_gpu::swapchain::{CompositeAlphaMode, PresentConfig};
use unbound_gpu::{Camera, ManagedExternalTextureDesc, Renderer};
use unbound_input::PointerButton;
use unbound_math::Vec2;
use unbound_platform::backend::{Flow, PlatformApp, PlatformWindow};
use unbound_platform::win32_frame::{FrameHitMetrics, WindowBackdropMaterial};
use unbound_platform::window::{
    ButtonState, ImeEvent, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey,
    PlatformState, SurfaceAlpha, WindowConfig,
};
use unbound_platform::{
    begin_outbound_file_drag, FramePacer, NativeWindow, OutboundFileDragOutcome, PopupHostRegistry,
    PopupWindowConfig, WindowId, WindowRegistry,
};
use unbound_theme::ThemeSet;

use crate::appearance::{AppearanceCaps, AppearanceDiff, WindowAppearance};
use crate::{
    AppKey, AppRuntime, Application, ExternalTextureHandle, ExternalTextureOp, ImeUpdate,
    Modifiers as AppModifiers, ThemeKind, WindowCommand,
};

/// 为新打开的窗口构造对应 [`Application`] 实例的工厂契约。
pub trait MultiWindowFactory<A: Application> {
    /// 当协调器打开或注册编号为 `id` 的窗口时，根据其配置构造应用实例。
    fn create(&mut self, id: WindowId, config: &WindowConfig) -> A;
}

impl<A, F> MultiWindowFactory<A> for F
where
    A: Application,
    F: FnMut(WindowId, &WindowConfig) -> A,
{
    fn create(&mut self, id: WindowId, config: &WindowConfig) -> A {
        self(id, config)
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

fn translate_pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Back | MouseButton::Other(1) => Some(PointerButton::Back),
        MouseButton::Forward | MouseButton::Other(2) => Some(PointerButton::Forward),
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

fn window_instance_extensions(window: &NativeWindow) -> Result<Vec<String>, String> {
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

fn platform_window_id(window: &NativeWindow) -> WindowId {
    WindowId(window.id().0 as u64)
}

/// 单个窗口的独立运行时与渲染上下文。
///
/// 底层 [`Gpu`] 与 [`AtlasSet`] 由 [`MultiWindowCoordinator`] 统一共享，
/// 本结构体仅持有逐窗口独立的 [`AppRuntime`]、[`FramePacer`]、[`Surface`]、
/// [`Swapchain`] 与 [`Renderer`]。
pub struct WindowContext<A: Application> {
    /// 窗口独立的应用运行时。
    pub runtime: AppRuntime<A>,
    /// 窗口独立的帧节流器（单窗口最小化/遮挡暂停不影响其余存活窗口）。
    pub pacer: FramePacer,
    /// 共享设备上的逐窗口渲染器。
    renderer: Option<Renderer>,
    /// 逐窗口 Vulkan 表面。
    surface: Option<Surface>,
    /// 逐窗口交换链。
    swapchain: Option<Swapchain>,
    /// 窗口创建配置快照。
    pub config: WindowConfig,
    pointer: Vec2,
    modifiers: Modifiers,
    last_frame: Instant,
    swapchain_dirty: bool,
    committed_request: (u32, u32),
    surface_alpha: SurfaceAlpha,
    scene_generation: u64,
    frame_dropped: bool,
    os_scale: f32,
    frame_metrics: Option<Arc<FrameHitMetrics>>,
    /// 本窗口的主题过渡驱动器（多可见窗口共享协调器时间戳同步推进）。
    theme_driver: ThemeTransitionDriver,
    appearance: WindowAppearance,
    external_gpu_slots: FxHashMap<ExternalTextureHandle, (u32, u32, u32, bool)>,
    popup_host: PopupHostRegistry,
    screen_origin: (i32, i32),
    was_suspended: bool,
}

impl<A: Application> WindowContext<A> {
    /// 基于给定应用实例与窗口配置创建独立窗口上下文。
    #[must_use]
    pub fn new(app: A, config: &WindowConfig) -> Self {
        let (w, h) = (config.width.max(1), config.height.max(1));
        let viewport = Vec2::new(w as f32, h as f32);
        let mut runtime = AppRuntime::new(app, viewport);
        runtime.set_backdrop(config.backdrop);
        runtime.set_clipboard(Box::new(unbound_platform::SystemClipboard::new()));
        let ctx = runtime.context();
        let theme_driver = ThemeTransitionDriver::new(
            ThemeSet::for_product_with_system_accent(
                ctx.preset,
                ctx.accent,
                ctx.resolved_system_accent(),
            ),
            ctx.theme,
        );
        let surface_alpha = match config.backdrop {
            Some(material) if material != WindowBackdropMaterial::None => {
                SurfaceAlpha::PreMultiplied
            }
            _ => config.surface_alpha,
        };
        Self {
            runtime,
            pacer: FramePacer::new(config.refresh),
            renderer: None,
            surface: None,
            swapchain: None,
            config: config.clone(),
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
            theme_driver,
            appearance: WindowAppearance::new(),
            external_gpu_slots: FxHashMap::default(),
            popup_host: PopupHostRegistry::new(),
            screen_origin: (0, 0),
            was_suspended: false,
        }
    }

    /// 访问当前窗口渲染器持有的共享 [`Gpu`] 引用计数句柄（若已初始化）。
    #[must_use]
    pub fn shared_gpu(&self) -> Option<Arc<Gpu>> {
        self.renderer.as_ref().map(Renderer::shared_gpu)
    }

    /// 判断本窗口当前是否处于最小化/遮挡暂停态，恢复可见时自动置位首帧强制刷新。
    pub fn should_skip_frame_for_throttle(&mut self) -> bool {
        if self.pacer.is_suspended() {
            self.was_suspended = true;
            return true;
        }
        if self.was_suspended {
            self.was_suspended = false;
            self.frame_dropped = true;
            // ADR 0030 & ADR 0034：Suspended 窗口恢复可见时直接吸附至目标主题终态，不补播过期动画。
            self.snap_theme_to_current_context();
            self.runtime.invalidate();
        }
        false
    }

    fn sync_os_scale(&mut self, window: &NativeWindow) -> f32 {
        let raw = window.scale_factor() as f32;
        if raw.is_finite() && raw > 0.0 {
            self.os_scale = raw.clamp(0.5, 4.0);
        }
        self.os_scale
    }

    /// 将当前上下文中的主题立即吸附为终态（用于 Suspended 恢复可见或首帧初始化）。
    pub(crate) fn snap_theme_to_current_context(&mut self) {
        let (key, sys_accent) = {
            let ctx = self.runtime.context();
            let sys_accent = ctx.resolved_system_accent();
            (
                make_theme_snapshot_key(ctx.theme, ctx.preset, ctx.accent, sys_accent),
                sys_accent,
            )
        };
        self.theme_driver
            .snap_preset(key, sys_accent, self.renderer.as_mut());
        self.runtime.context_mut().theme_transition_remaining_secs = 0.0;
    }

    /// 以协调器共享时钟同步本窗口主题过渡状态（无头测试与运行器共用接缝）。
    ///
    /// 返回 `true` 表示需要继续请求重绘；`Suspended` 窗口不唤醒，直接返回 `false`。
    pub(crate) fn sync_theme_state_at(
        &mut self,
        now_secs: f64,
        shared_trigger_secs: f64,
        reduced_motion: bool,
    ) -> bool {
        if self.pacer.is_suspended() {
            return false;
        }
        let (key, sys_accent, transition_cfg) = {
            let ctx = self.runtime.context();
            let sys_accent = ctx.resolved_system_accent();
            (
                make_theme_snapshot_key(ctx.theme, ctx.preset, ctx.accent, sys_accent),
                sys_accent,
                ctx.theme_transition,
            )
        };
        let outcome = self.theme_driver.sync_preset_at(
            key,
            sys_accent,
            transition_cfg,
            now_secs,
            shared_trigger_secs,
            reduced_motion,
        );
        self.runtime.context_mut().theme_transition_remaining_secs = outcome.remaining_secs;
        outcome.lut_updated
    }

    fn sync_theme_at(&mut self, window: &NativeWindow, now_secs: f64, shared_trigger_secs: f64) {
        let reduced_motion = unbound_platform::prefers_reduced_motion();
        let needs_upload = self.sync_theme_state_at(now_secs, shared_trigger_secs, reduced_motion);
        if !needs_upload {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.write_theme_lut(self.theme_driver.lut()) {
            Ok(()) => {
                window.request_redraw();
            }
            Err(error) => {
                eprintln!("[unbound-app] Could not upload the theme LUT: {error:?}");
            }
        }
    }

    fn sync_appearance(&mut self, window: &NativeWindow) {
        let caps = AppearanceCaps::new(
            unbound_platform::windows_build(),
            self.runtime.premultiplied_surface(),
            unbound_platform::accent_policy_export_available(),
        );
        let theme = self.theme_driver.effective_backdrop_theme();
        self.appearance
            .set_host_clear_alpha(self.runtime.clear_alpha());
        let diff = self
            .appearance
            .set_intent(self.runtime.backdrop_intent(), caps, theme);
        self.apply_appearance_diff(window, diff);
    }

    fn apply_appearance_diff(&mut self, window: &NativeWindow, diff: AppearanceDiff) {
        crate::appearance::apply_appearance_diff_to_window(
            &diff,
            window,
            self.frame_metrics.as_deref(),
        );
        if let Some(clear_alpha) = diff.clear_alpha {
            if let Some(renderer) = &mut self.renderer {
                renderer.set_clear_alpha(clear_alpha);
            }
        }
    }

    fn process_external_texture_ops(&mut self, ops: Vec<ExternalTextureOp>, window: &NativeWindow) {
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
                    } else {
                        let _ = renderer.destroy_external_texture(slot);
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
                                    "[unbound-app] Could not re-upload resized external texture {handle:?}: {error:?}"
                                );
                                self.runtime.asset_hub_mut().commit_external_texture_slot(
                                    handle,
                                    Err(format!("{error:?}")),
                                );
                            }
                        }
                    }
                    needs_redraw = true;
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

    /// 严格按“先停用交换链与表面、再释放窗口级 `Renderer`”的顺序回收本窗口 GPU 资源。
    pub fn destroy_vulkan(&mut self) {
        self.external_gpu_slots.clear();
        self.runtime
            .asset_hub_mut()
            .invalidate_all_external_textures();
        let renderer = self.renderer.take();
        let swapchain = self.swapchain.take();
        let surface = self.surface.take();
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
    }
}

impl<A: Application> Drop for WindowContext<A> {
    fn drop(&mut self) {
        self.destroy_vulkan();
    }
}

/// 原生窗口创建完成时的可选宿主回调钩子（例如挂载系统托盘）。
pub type WindowCreatedHook = Box<dyn FnMut(WindowId, &NativeWindow) + 'static>;

/// 多窗口运行器配置。
#[derive(Default)]
pub struct MultiWindowConfig {
    /// 主窗口初始配置。
    pub primary_config: WindowConfig,
    /// 每个主/辅助原生窗口创建完成后的可选回调钩子。
    pub on_window_created: Option<WindowCreatedHook>,
}

impl MultiWindowConfig {
    /// 基于主窗口配置构造默认多窗口配置。
    #[must_use]
    pub fn new(primary_config: WindowConfig) -> Self {
        Self {
            primary_config,
            on_window_created: None,
        }
    }

    /// 绑定窗口创建完成回调钩子。
    #[must_use]
    pub fn with_on_window_created(
        mut self,
        hook: impl FnMut(WindowId, &NativeWindow) + 'static,
    ) -> Self {
        self.on_window_created = Some(Box::new(hook));
        self
    }
}

/// 多窗口共享 `VkDevice` 与共享基础图集的运行时协调器。
pub struct MultiWindowCoordinator<A: Application> {
    registry: WindowRegistry<WindowContext<A>>,
    shared_gpu: Option<Arc<Gpu>>,
    shared_atlases: Option<Arc<AtlasSet>>,
    pending_windows: VecDeque<WindowConfig>,
    pending_created_configs: VecDeque<WindowConfig>,
    pending_window_close_requests: Vec<WindowId>,
    pending_popup_syncs: Vec<(u64, PopupWindowConfig)>,
    pending_popup_closes: Vec<u64>,
    native_popup_owners: FxHashMap<WindowId, (u64, WindowId)>,
    primary_config: WindowConfig,
    factory: Box<dyn MultiWindowFactory<A>>,
    on_window_created: Option<WindowCreatedHook>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// 多窗口共享主题过渡单调时钟基准。
    theme_clock_epoch: Instant,
    /// 最近一次全局主题切换对应的共享触发时间戳（秒），确保并排可见窗口同拍推进。
    shared_theme_trigger: Option<(ThemeSnapshotKey, f64)>,
}

impl<A: Application> MultiWindowCoordinator<A> {
    /// 创建多窗口协调器并保存主窗口配置与窗口实例工厂。
    #[must_use]
    pub fn new<F>(primary_config: WindowConfig, factory: F) -> Self
    where
        F: MultiWindowFactory<A> + 'static,
    {
        Self {
            registry: WindowRegistry::new(),
            shared_gpu: None,
            shared_atlases: None,
            pending_windows: VecDeque::new(),
            pending_created_configs: VecDeque::new(),
            pending_window_close_requests: Vec::new(),
            pending_popup_syncs: Vec::new(),
            pending_popup_closes: Vec::new(),
            native_popup_owners: FxHashMap::default(),
            primary_config,
            factory: Box::new(factory),
            on_window_created: None,
            waker: None,
            theme_clock_epoch: Instant::now(),
            shared_theme_trigger: None,
        }
    }

    /// 绑定每个原生窗口创建完成后的回调钩子（例如挂载系统托盘）。
    #[must_use]
    pub fn with_on_window_created(
        mut self,
        hook: impl FnMut(WindowId, &NativeWindow) + 'static,
    ) -> Self {
        self.on_window_created = Some(Box::new(hook));
        self
    }

    /// 借用多窗口注册表。
    #[must_use]
    pub fn registry(&self) -> &WindowRegistry<WindowContext<A>> {
        &self.registry
    }

    /// 可变借用多窗口注册表。
    pub fn registry_mut(&mut self) -> &mut WindowRegistry<WindowContext<A>> {
        &mut self.registry
    }

    /// 返回当前已初始化的共享 [`Arc<Gpu>`] 设备句柄（若已创建）。
    #[must_use]
    pub fn shared_gpu(&self) -> Option<Arc<Gpu>> {
        self.shared_gpu.as_ref().map(Arc::clone)
    }

    /// 返回当前已初始化的共享基础图集 [`Arc<AtlasSet>`] 句柄（若已创建）。
    #[must_use]
    pub fn shared_atlases(&self) -> Option<Arc<AtlasSet>> {
        self.shared_atlases.as_ref().map(Arc::clone)
    }

    /// 将新窗口打开请求压入待建窗队列。
    pub fn queue_window(&mut self, mut config: WindowConfig) {
        config.decorations = false;
        self.pending_windows.push_back(config);
    }

    /// 无头注册一个窗口上下文（主要用于无头多窗口测试与集成验证）。
    pub fn register_headless_window(&mut self, id: WindowId, config: WindowConfig) {
        let app = self.factory.create(id, &config);
        let mut ctx = WindowContext::new(app, &config);
        if let Some(waker) = &self.waker {
            ctx.runtime
                .set_waker(Arc::new(SignalWaker(Arc::clone(waker))));
        }
        let _ = self.registry.insert(id, ctx);
    }

    /// 按严格回收顺序关闭并移除指定窗口；若全部窗口均已关闭则释放共享图集与设备引用。
    pub fn close_window(&mut self, id: WindowId) -> bool {
        let Some(mut ctx) = self.registry.remove(id) else {
            return false;
        };
        for action in ctx.popup_host.close_for_owner(id) {
            if let unbound_platform::PopupSyncAction::Close { id: popup_id, .. } = action {
                self.pending_popup_closes.push(popup_id);
            }
        }
        ctx.destroy_vulkan();
        drop(ctx);
        if self.registry.is_empty() {
            self.release_shared_gpu();
        }
        true
    }

    /// 排空指定窗口上下文产出的命令（无头安全路径：不依赖原生 `Window` 句柄）。
    ///
    /// 返回需要关闭的目标窗口 ID 列表。
    pub fn route_window_commands(&mut self, source_id: WindowId) -> Vec<WindowId> {
        let mut to_close = Vec::new();
        let Some(ctx) = self.registry.get_mut(source_id) else {
            return to_close;
        };
        let cmds = ctx.runtime.drain_window_commands();
        for cmd in cmds {
            match cmd {
                WindowCommand::OpenWindow(mut config) => {
                    config.decorations = false;
                    self.pending_windows.push_back(config);
                }
                WindowCommand::CloseWindow(target) => {
                    to_close.push(target);
                }
                WindowCommand::Close => {
                    to_close.push(source_id);
                }
                WindowCommand::SetThrottlePolicy(policy) => {
                    ctx.pacer.set_throttle_policy(policy);
                }
                WindowCommand::SyncNativePopup { id, config } => {
                    if ctx.popup_host.sync_popup(id, config).is_some() {
                        self.pending_popup_syncs.push((id, config));
                    }
                }
                WindowCommand::CloseNativePopup { id } => {
                    if ctx.popup_host.close_popup(id).is_some() {
                        self.pending_popup_closes.push(id);
                    }
                }
                WindowCommand::StartDrag
                | WindowCommand::Minimize
                | WindowCommand::ToggleMaximize
                | WindowCommand::BeginExternalFileDrag(_)
                | WindowCommand::SetImeCursorArea { .. }
                | WindowCommand::SetVisible(_)
                | WindowCommand::ActivateForeground
                | WindowCommand::SetOuterPosition { .. }
                | WindowCommand::SetInnerSize { .. }
                | WindowCommand::SetMaximized(_)
                | WindowCommand::SetTaskbarProgress { .. } => {}
            }
        }
        for target in &to_close {
            if self.close_window(*target) {
                self.pending_window_close_requests.push(*target);
            }
        }
        to_close
    }

    fn release_shared_gpu(&mut self) {
        if let Some(atlases) = self.shared_atlases.take() {
            if let Some(gpu) = &self.shared_gpu {
                gpu.wait_idle();
            }
            atlases.free();
        }
        if let Some(gpu) = self.shared_gpu.take() {
            if let Ok(owned) = Arc::try_unwrap(gpu) {
                owned.destroy();
            }
        }
    }

    fn attach_window_gpu(
        &mut self,
        state: &mut PlatformState,
        window: &NativeWindow,
        config: WindowConfig,
    ) {
        let id = platform_window_id(window);
        let app = self.factory.create(id, &config);
        let mut ctx = WindowContext::new(app, &config);
        if let Some(waker) = &self.waker {
            ctx.runtime
                .set_waker(Arc::new(SignalWaker(Arc::clone(waker))));
        }

        let os_scale = ctx.sync_os_scale(window);
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        ctx.committed_request = (width, height);
        ctx.runtime
            .sync_physical_viewport(Vec2::new(width as f32, height as f32), os_scale);

        let gpu = if let Some(existing) = &self.shared_gpu {
            Arc::clone(existing)
        } else {
            let instance_extensions = match window_instance_extensions(window) {
                Ok(extensions) => extensions,
                Err(error) => {
                    eprintln!("[unbound-app] Window display handle initialization failed: {error}");
                    window.request_close();
                    return;
                }
            };
            let gpu_config = GpuConfig {
                app_name: "Unbound Multi-Window App".to_string(),
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
            match Gpu::new(gpu_config) {
                Ok(gpu) => {
                    let shared = Arc::new(gpu);
                    self.shared_gpu = Some(Arc::clone(&shared));
                    shared
                }
                Err(error) => {
                    eprintln!(
                        "[unbound-app] Shared Vulkan device initialization failed:\n{}",
                        error.diagnostic_report()
                    );
                    window.request_close();
                    return;
                }
            }
        };

        let atlases = if let Some(existing) = &self.shared_atlases {
            Arc::clone(existing)
        } else {
            match AtlasSet::new(&gpu, 64) {
                Ok(a) => {
                    let shared = Arc::new(a);
                    self.shared_atlases = Some(Arc::clone(&shared));
                    shared
                }
                Err(error) => {
                    eprintln!(
                        "[unbound-app] Could not create shared atlas set:\n{}",
                        error.diagnostic_report()
                    );
                    window.request_close();
                    return;
                }
            }
        };

        let surface = match window.create_vulkan_surface(&gpu) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create Vulkan surface:\n{}",
                    error.diagnostic_report()
                );
                window.request_close();
                return;
            }
        };

        let composite_alpha = resolve_composite_alpha(ctx.surface_alpha, &surface, &gpu);
        ctx.runtime
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
                window.request_close();
                return;
            }
        };

        let device_extensions = gpu.enabled_device_extensions.clone();
        let mut renderer = match Renderer::new_shared(Arc::clone(&gpu), width, height) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create shared-device renderer ({width}x{height}):\n{}",
                    error.diagnostic_report()
                );
                swapchain.destroy(&gpu);
                surface.destroy(&gpu);
                window.request_close();
                return;
            }
        };

        if let Err(error) = renderer.install_atlases(atlases) {
            eprintln!(
                "[unbound-app] Could not install shared atlases:\n{}",
                error.diagnostic_report()
            );
            swapchain.destroy(&gpu);
            surface.destroy(&gpu);
            renderer.destroy();
            window.request_close();
            return;
        }

        let (chain_w, chain_h) = swapchain.extent();
        ctx.runtime.sync_physical_viewport(
            Vec2::new(chain_w.max(1) as f32, chain_h.max(1) as f32),
            os_scale,
        );
        ctx.swapchain = Some(swapchain);
        ctx.surface = Some(surface);
        ctx.renderer = Some(renderer);
        let metrics = Arc::new(FrameHitMetrics::new());
        metrics.set_scale_factor(os_scale);
        window.install_custom_frame(metrics.clone());
        ctx.frame_metrics = Some(metrics);

        state.set_present_extensions(&device_extensions);
        let client_screen = window.client_rect_on_screen();
        ctx.screen_origin = (client_screen.x, client_screen.y);
        let now_secs = self.theme_clock_epoch.elapsed().as_secs_f64();
        let trigger_secs = {
            let app_ctx = ctx.runtime.context();
            let sys_accent = app_ctx.resolved_system_accent();
            let sys_bits =
                sys_accent.map(|c| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()]);
            let key = (app_ctx.theme, app_ctx.preset, app_ctx.accent, sys_bits);
            match self.shared_theme_trigger {
                Some((prev_key, ts)) if prev_key == key => ts,
                _ => {
                    self.shared_theme_trigger = Some((key, now_secs));
                    now_secs
                }
            }
        };
        ctx.sync_theme_at(window, now_secs, trigger_secs);
        ctx.sync_appearance(window);
        ctx.last_frame = Instant::now();

        let _ = self.registry.insert(id, ctx);
        if let Some(hook) = self.on_window_created.as_mut() {
            hook(id, window);
        }
    }

    fn drain_window_commands_live(&mut self, id: WindowId, window: &NativeWindow) -> Flow {
        let mut outbound_drag = None;
        let mut close_targets = Vec::new();
        let mut close_self = false;

        if let Some(ctx) = self.registry.get_mut(id) {
            for cmd in ctx.runtime.drain_window_commands() {
                match cmd {
                    WindowCommand::StartDrag => {
                        let _ = window.drag_window();
                    }
                    WindowCommand::Minimize => {
                        window.set_minimized(true);
                    }
                    WindowCommand::ToggleMaximize => {
                        window.set_maximized(!window.is_maximized());
                    }
                    WindowCommand::Close => {
                        close_self = true;
                    }
                    WindowCommand::SetThrottlePolicy(policy) => {
                        ctx.pacer.set_throttle_policy(policy);
                    }
                    WindowCommand::BeginExternalFileDrag(request) => {
                        outbound_drag = Some(request);
                    }
                    WindowCommand::SyncNativePopup {
                        id: popup_id,
                        config,
                    } => {
                        if ctx.popup_host.sync_popup(popup_id, config).is_some() {
                            self.pending_popup_syncs.push((popup_id, config));
                        }
                    }
                    WindowCommand::CloseNativePopup { id: popup_id } => {
                        if ctx.popup_host.close_popup(popup_id).is_some() {
                            self.pending_popup_closes.push(popup_id);
                        }
                    }
                    WindowCommand::OpenWindow(mut config) => {
                        config.decorations = false;
                        self.pending_windows.push_back(config);
                    }
                    WindowCommand::CloseWindow(target) => {
                        if target == id {
                            close_self = true;
                        } else {
                            close_targets.push(target);
                        }
                    }
                    WindowCommand::SetImeCursorArea {
                        x,
                        y,
                        width,
                        height,
                    } => {
                        window.set_ime_cursor_area(
                            unbound_platform::PhysicalPosition::new(x, y),
                            unbound_platform::PhysicalSize::new(width, height),
                        );
                    }
                    WindowCommand::SetVisible(visible) => {
                        window.set_visible(visible);
                    }
                    WindowCommand::ActivateForeground => {
                        window.activate_foreground();
                    }
                    WindowCommand::SetOuterPosition { x, y } => {
                        window.set_outer_position(x, y);
                    }
                    WindowCommand::SetInnerSize { width, height } => {
                        window.set_inner_size(width, height);
                    }
                    WindowCommand::SetMaximized(maximized) => {
                        window.set_maximized(maximized);
                    }
                    WindowCommand::SetTaskbarProgress {
                        completed,
                        total,
                        state: progress_state,
                    } => {
                        if let Some(raw_window_handle::RawWindowHandle::Win32(handle)) =
                            unbound_platform::backend::raw_window_handle(window)
                        {
                            unbound_platform::set_taskbar_progress(
                                handle.hwnd.get(),
                                completed,
                                total,
                                progress_state,
                            );
                        }
                    }
                }
            }
            if let Some(request) = outbound_drag {
                let _ = ctx.runtime.cancel_pointer();
                let outcome = match begin_outbound_file_drag(&request) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        eprintln!("[unbound-app] Outbound OLE file drag failed: {error:?}");
                        OutboundFileDragOutcome::Cancelled
                    }
                };
                let _ = ctx.runtime.notify_external_file_drag_finished(outcome);
                ctx.pacer.note_activity(Instant::now());
                window.request_redraw();
            }
        }

        for target in close_targets {
            if self.close_window(target) {
                self.pending_window_close_requests.push(target);
            }
        }

        if close_self {
            window.request_close();
            if self.close_window(id) {
                self.pending_window_close_requests.push(id);
            }
            if self.registry.is_empty() {
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    /// 识别平台窗口是否为原生弹出窗，返回 `(popup_id, owner_window_id)`。
    fn resolve_popup_window(&self, window: &NativeWindow) -> Option<(u64, WindowId)> {
        let win_id = platform_window_id(window);
        if let Some(&mapped) = self.native_popup_owners.get(&win_id) {
            return Some(mapped);
        }
        if let Some(popup_id) = window.popup_id() {
            for slot in self.registry.iter() {
                if slot.context.popup_host.get(popup_id).is_some() {
                    return Some((popup_id, slot.id));
                }
            }
            if let Some(primary) = self.registry.primary() {
                return Some((popup_id, primary));
            }
        }
        for slot in self.registry.iter() {
            if let Some(entry) = slot.context.popup_host.get_by_window(win_id) {
                return Some((entry.id, slot.id));
            }
        }
        None
    }
}

impl<A: Application> Drop for MultiWindowCoordinator<A> {
    fn drop(&mut self) {
        let ids: Vec<WindowId> = self.registry.iter().map(|slot| slot.id).collect();
        for id in ids {
            let _ = self.close_window(id);
        }
        self.release_shared_gpu();
    }
}

impl<A: Application> PlatformApp for MultiWindowCoordinator<A> {
    fn attach_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.waker = Some(Arc::clone(&waker));
        for slot in self.registry.iter_mut() {
            slot.context
                .runtime
                .set_waker(Arc::new(SignalWaker(Arc::clone(&waker))));
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        let now = Instant::now();
        self.registry
            .iter()
            .filter(|slot| !slot.context.pacer.is_suspended())
            .filter_map(|slot| {
                let pacer_wake = slot.context.pacer.next_wake(now);
                let runtime_wake = slot.context.runtime.next_wake_in().map(|d| now + d);
                match (pacer_wake, runtime_wake) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            })
            .min()
    }

    fn resumed(&mut self, state: &mut PlatformState, window: &NativeWindow) {
        if window.is_popup() || self.resolve_popup_window(window).is_some() {
            return;
        }
        let config = self.primary_config.clone();
        self.attach_window_gpu(state, window, config);
    }

    fn window_created(&mut self, state: &mut PlatformState, window: &NativeWindow) {
        if window.is_popup() || self.resolve_popup_window(window).is_some() {
            if let Some((popup_id, owner_id)) = self.resolve_popup_window(window) {
                let popup_win_id = platform_window_id(window);
                self.native_popup_owners
                    .insert(popup_win_id, (popup_id, owner_id));
                if let Some(owner_ctx) = self.registry.get_mut(owner_id) {
                    let _ = owner_ctx
                        .popup_host
                        .bind_native_window(popup_id, popup_win_id);
                }
            }
            return;
        }
        let config = self
            .pending_created_configs
            .pop_front()
            .unwrap_or_else(|| self.primary_config.clone());
        self.attach_window_gpu(state, window, config);
    }

    fn window_closing(&mut self, window: &NativeWindow) {
        let id = platform_window_id(window);
        if let Some((popup_id, owner_id)) = self
            .native_popup_owners
            .remove(&id)
            .or_else(|| self.resolve_popup_window(window))
        {
            if let Some(owner_ctx) = self.registry.get_mut(owner_id) {
                let _ = owner_ctx.popup_host.close_popup(popup_id);
            }
            return;
        }
        if window.is_popup() {
            return;
        }
        let _ = self.close_window(id);
    }

    fn take_window_request(&mut self) -> Option<WindowConfig> {
        let config = self.pending_windows.pop_front()?;
        self.pending_created_configs.push_back(config.clone());
        Some(config)
    }

    fn take_window_close_requests(&mut self) -> Vec<WindowId> {
        core::mem::take(&mut self.pending_window_close_requests)
    }

    fn take_popup_sync_requests(&mut self) -> Vec<(u64, PopupWindowConfig)> {
        core::mem::take(&mut self.pending_popup_syncs)
    }

    fn take_popup_close_requests(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.pending_popup_closes)
    }

    fn event(
        &mut self,
        state: &mut PlatformState,
        window: &NativeWindow,
        event: PlatformEvent,
    ) -> Flow {
        let id = platform_window_id(window);
        if window.is_popup() || self.resolve_popup_window(window).is_some() {
            let Some((popup_id, owner_id)) = self.resolve_popup_window(window) else {
                return Flow::Continue;
            };
            let live_rect = window.client_rect_on_screen();
            if let Some(owner_ctx) = self.registry.get_mut(owner_id) {
                let popup_rect = if live_rect.width > 0 && live_rect.height > 0 {
                    live_rect
                } else {
                    owner_ctx
                        .popup_host
                        .get(popup_id)
                        .map_or(live_rect, |entry| entry.config.screen_rect)
                };
                match event {
                    PlatformEvent::CursorMoved { x, y } => {
                        let scale = owner_ctx.runtime.context().effective_scale().max(0.01);
                        let main_phys_x =
                            x as f32 + (popup_rect.x - owner_ctx.screen_origin.0) as f32;
                        let main_phys_y =
                            y as f32 + (popup_rect.y - owner_ctx.screen_origin.1) as f32;
                        owner_ctx.pointer = Vec2::new(main_phys_x / scale, main_phys_y / scale);
                        owner_ctx.pacer.note_activity(Instant::now());
                        owner_ctx.runtime.pointer_move(owner_ctx.pointer);
                    }
                    PlatformEvent::MouseInput {
                        state: btn_state,
                        button,
                    } => {
                        if let Some(btn) = translate_pointer_button(button) {
                            if btn_state == ButtonState::Pressed {
                                owner_ctx.runtime.pointer_down(owner_ctx.pointer, btn);
                            } else {
                                owner_ctx.runtime.pointer_up(owner_ctx.pointer, btn);
                            }
                            owner_ctx.pacer.note_activity(Instant::now());
                        }
                    }
                    PlatformEvent::MouseWheel { delta_x, delta_y } => {
                        owner_ctx.pacer.note_activity(Instant::now());
                        owner_ctx
                            .runtime
                            .wheel(owner_ctx.pointer, Vec2::new(delta_x as f32, delta_y as f32));
                    }
                    PlatformEvent::CloseRequested => {
                        let _ = owner_ctx.popup_host.close_popup(popup_id);
                        let _ = owner_ctx.runtime.notify_native_popup_dismissed(popup_id);
                        owner_ctx.pacer.note_activity(Instant::now());
                    }
                    _ => {}
                }
                if owner_ctx.runtime.dirty() != crate::DirtyFlags::NONE
                    && !owner_ctx.pacer.is_suspended()
                {
                    let now = Instant::now();
                    owner_ctx.pacer.request_immediate(now);
                    state.pacer.request_immediate(now);
                    window.request_redraw();
                }
            }
            return Flow::Continue;
        }
        if matches!(event, PlatformEvent::CloseRequested) {
            let _ = self.close_window(id);
            return if self.registry.is_empty() {
                Flow::Exit
            } else {
                Flow::Continue
            };
        }

        let Some(ctx) = self.registry.get_mut(id) else {
            return Flow::Continue;
        };
        let os_scale = ctx.sync_os_scale(window);
        match event {
            PlatformEvent::Resized(_) => {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    ctx.runtime.sync_physical_viewport(
                        Vec2::new(size.width as f32, size.height as f32),
                        os_scale,
                    );
                    ctx.swapchain_dirty = true;
                }
            }
            PlatformEvent::RecreateSwapchain(_) => {
                ctx.swapchain_dirty = true;
            }
            PlatformEvent::CursorMoved { x, y } => {
                let effective_scale = ctx.runtime.context().effective_scale();
                ctx.pointer = Vec2::new(x as f32 / effective_scale, y as f32 / effective_scale);
                ctx.pacer.note_activity(Instant::now());
                ctx.runtime.pointer_move(ctx.pointer);
            }
            PlatformEvent::MouseInput {
                state: btn_state,
                button,
            } => {
                if let Some(btn) = translate_pointer_button(button) {
                    if btn_state == ButtonState::Pressed {
                        ctx.runtime.pointer_down(ctx.pointer, btn);
                    } else {
                        ctx.runtime.pointer_up(ctx.pointer, btn);
                    }
                    ctx.pacer.note_activity(Instant::now());
                }
            }
            PlatformEvent::MouseWheel { delta_x, delta_y } => {
                ctx.pacer.note_activity(Instant::now());
                if ctx.modifiers.control {
                    let step = if delta_y > 0.0 {
                        1
                    } else if delta_y < 0.0 {
                        -1
                    } else {
                        0
                    };
                    if step != 0 {
                        let _ = ctx.runtime.context_mut().step_ui_scale(step);
                        let size = window.inner_size();
                        ctx.runtime.sync_physical_viewport(
                            Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32),
                            os_scale,
                        );
                        window.request_redraw();
                    }
                } else {
                    ctx.runtime
                        .wheel(ctx.pointer, Vec2::new(delta_x as f32, delta_y as f32));
                }
            }
            PlatformEvent::KeyInput {
                state: key_state,
                key,
                ..
            } => {
                let app_k = translate_app_key(&key);
                let pressed = key_state == ButtonState::Pressed;
                let modifiers = translate_modifiers(ctx.modifiers);
                ctx.pacer.note_activity(Instant::now());
                ctx.runtime.key(app_k, pressed, modifiers);
            }
            PlatformEvent::ModifiersChanged(modifiers) => {
                ctx.modifiers = modifiers;
            }
            PlatformEvent::Ime(ime) => match ime {
                ImeEvent::Preedit(text, selection) => {
                    ctx.runtime.ime(ImeUpdate {
                        enabled: true,
                        preedit: text,
                        cursor_range: selection,
                        commit: None,
                        cancelled: false,
                    });
                }
                ImeEvent::Commit(text) => {
                    ctx.runtime.ime(ImeUpdate {
                        enabled: false,
                        preedit: String::new(),
                        cursor_range: None,
                        commit: Some(text),
                        cancelled: false,
                    });
                }
                ImeEvent::Disabled => {
                    ctx.runtime.ime(ImeUpdate {
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
                ctx.os_scale = new_os_scale;
                if let Some(metrics) = &ctx.frame_metrics {
                    metrics.set_scale_factor(new_os_scale);
                }
                let size = window.inner_size();
                ctx.runtime.sync_physical_viewport(
                    Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32),
                    new_os_scale,
                );
                ctx.swapchain_dirty = true;
                window.request_redraw();
            }
            PlatformEvent::ThemeChanged { dark } => {
                if ctx.runtime.follows_system_theme() {
                    ctx.runtime.set_theme(if dark {
                        ThemeKind::Dark
                    } else {
                        ThemeKind::Light
                    });
                }
                ctx.sync_appearance(window);
            }
            PlatformEvent::FileHover(path) => {
                ctx.runtime.file_hover(vec![path]);
            }
            PlatformEvent::FileDrop(path) => {
                ctx.runtime.file_drop(vec![path]);
            }
            PlatformEvent::FileHoverCancelled => {
                ctx.runtime.file_hover_cancelled();
            }
            PlatformEvent::Focused(focused) => {
                ctx.pacer.set_focused(focused);
                if focused {
                    ctx.pacer.note_activity(Instant::now());
                }
                ctx.runtime.set_window_focused(focused);
            }
            PlatformEvent::Minimized(minimized) => {
                ctx.pacer.set_minimized(minimized);
                if !minimized {
                    ctx.frame_dropped = true;
                    ctx.pacer.request_immediate(Instant::now());
                    window.request_redraw();
                }
            }
            PlatformEvent::Occluded(occluded) => {
                ctx.pacer.set_occluded(occluded);
                if !occluded {
                    ctx.frame_dropped = true;
                    ctx.pacer.request_immediate(Instant::now());
                    window.request_redraw();
                }
            }
            PlatformEvent::Moved { x, y } => {
                ctx.screen_origin = (x, y);
                ctx.runtime.set_window_screen_origin((x, y), id);
            }
            PlatformEvent::PopupDismissed { id: popup_id } => {
                let _ = ctx.popup_host.close_popup(popup_id);
                let _ = ctx.runtime.notify_native_popup_dismissed(popup_id);
                ctx.pacer.note_activity(Instant::now());
            }
            PlatformEvent::RedrawRequested => {
                window.request_redraw();
            }
            _ => {}
        }

        let dirty = ctx.runtime.dirty() != crate::DirtyFlags::NONE;
        let suspended = ctx.pacer.is_suspended();
        if self.drain_window_commands_live(id, window) == Flow::Exit {
            return Flow::Exit;
        }
        if dirty && !suspended {
            state.pacer.request_immediate(Instant::now());
            window.request_redraw();
        }
        Flow::Continue
    }

    fn frame(&mut self, state: &mut PlatformState, window: &NativeWindow) -> Flow {
        if window.is_popup() || self.resolve_popup_window(window).is_some() {
            return Flow::Continue;
        }
        let id = platform_window_id(window);
        let shared_atlases = self.shared_atlases.clone();
        let Some(ctx) = self.registry.get_mut(id) else {
            return Flow::Continue;
        };
        if ctx.should_skip_frame_for_throttle() {
            return Flow::Continue;
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Flow::Continue;
        }
        let (width, height) = (size.width.max(1), size.height.max(1));
        let os_scale = ctx.sync_os_scale(window);
        let client_screen = window.client_rect_on_screen();
        ctx.screen_origin = (client_screen.x, client_screen.y);
        ctx.runtime
            .set_window_screen_origin((client_screen.x, client_screen.y), id);
        let now = Instant::now();
        let dt = (now - ctx.last_frame).as_secs_f32().min(0.1);
        ctx.last_frame = now;

        let settled = state.poll_resize(now);
        if ctx.swapchain_dirty || settled.is_some() || ctx.committed_request != (width, height) {
            if let (Some(renderer), Some(swapchain), Some(surface)) =
                (&mut ctx.renderer, &mut ctx.swapchain, &ctx.surface)
            {
                let composite_alpha =
                    resolve_composite_alpha(ctx.surface_alpha, surface, &renderer.gpu);
                let present = PresentConfig::from_capabilities(
                    &renderer.gpu.enabled_device_extensions,
                    false,
                )
                .with_composite_alpha(composite_alpha);
                if let Err(error) =
                    swapchain.recreate(&renderer.gpu, surface, width, height, &present)
                {
                    eprintln!("[unbound-app] Could not recreate swapchain: {error:?}");
                    ctx.swapchain_dirty = true;
                    return Flow::Continue;
                }
                ctx.committed_request = (width, height);
                ctx.swapchain_dirty = false;
                let (actual_w, actual_h) = swapchain.extent();
                let _ = renderer.ensure_depth_size(actual_w, actual_h);
            }
        }

        let (chain_w, chain_h) = ctx
            .swapchain
            .as_ref()
            .map_or((width, height), Swapchain::extent);
        ctx.runtime.sync_physical_viewport(
            Vec2::new(chain_w.max(1) as f32, chain_h.max(1) as f32),
            os_scale,
        );
        ctx.runtime.context_mut().window_maximized = window.is_maximized();

        let _ = ctx.runtime.tick(dt);
        let report = ctx.runtime.frame(0.0);
        let now_secs = self.theme_clock_epoch.elapsed().as_secs_f64();
        let trigger_secs = {
            let app_ctx = ctx.runtime.context();
            let sys_accent = app_ctx.resolved_system_accent();
            let sys_bits =
                sys_accent.map(|c| [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()]);
            let key = (app_ctx.theme, app_ctx.preset, app_ctx.accent, sys_bits);
            match self.shared_theme_trigger {
                Some((prev_key, ts)) if prev_key == key => ts,
                _ => {
                    self.shared_theme_trigger = Some((key, now_secs));
                    now_secs
                }
            }
        };
        ctx.sync_theme_at(window, now_secs, trigger_secs);
        ctx.sync_appearance(window);
        ctx.process_external_texture_ops(report.external_texture_ops, window);

        if let (Some(renderer), Some(atlases)) = (&mut ctx.renderer, &shared_atlases) {
            if !report.atlas_uploads.is_empty() {
                if let Err(error) = atlases.upload_batch(&renderer.gpu, &report.atlas_uploads) {
                    eprintln!("[unbound-app] Could not upload shared atlas glyphs: {error:?}");
                }
            }
            if let Some(swapchain) = &mut ctx.swapchain {
                if let Err(error) = renderer.wait_for_previous_frame() {
                    eprintln!("[unbound-app] Waiting for previous frame failed: {error:?}");
                    ctx.frame_dropped = true;
                    return Flow::Continue;
                }
                match swapchain.acquire_outcome(1_000_000_000) {
                    Ok(AcquireOutcome::Ready { image, suboptimal }) => {
                        if suboptimal {
                            ctx.swapchain_dirty = true;
                        }
                        if ctx.scene_generation == 0
                            || report.diff.bumps_generation()
                            || ctx.frame_dropped
                        {
                            ctx.scene_generation = ctx.scene_generation.wrapping_add(1).max(1);
                        }
                        let dirty_ranges = if ctx.frame_dropped {
                            &[][..]
                        } else {
                            report.diff.ranges()
                        };
                        let (actual_w, actual_h) = swapchain.extent();
                        let effective_scale = ctx.runtime.context().effective_scale();
                        let camera =
                            Camera::ui((actual_w as f32, actual_h as f32), effective_scale);
                        let backdrops =
                            crate::collect_backdrops(&report.backdrops, effective_scale);
                        let scene_data = ctx.runtime.scene();
                        let scene = crate::scene_upload(
                            scene_data,
                            &backdrops,
                            camera,
                            ctx.scene_generation,
                            dirty_ranges,
                            report.aux_changed,
                        );
                        renderer.set_clear_alpha(ctx.appearance.applied_clear_alpha());
                        if let Err(ref err) =
                            renderer.render_into_swapchain(&scene, &image, actual_w, actual_h)
                        {
                            ctx.frame_dropped = true;
                            if is_device_lost(err) {
                                eprintln!(
                                    "[unbound-app] Shared GPU device lost during render on {id:?}"
                                );
                                return Flow::Continue;
                            }
                            eprintln!("[unbound-app] multi-window render failed: {err:?}");
                        } else {
                            ctx.frame_dropped = false;
                        }
                        match swapchain.present_outcome(
                            &renderer.gpu,
                            image.index,
                            image.render_finished,
                        ) {
                            Ok(PresentOutcome::Optimal) => {}
                            Ok(PresentOutcome::Recreate) => {
                                ctx.swapchain_dirty = true;
                                state.resize.mark_out_of_date(unbound_platform::Extent::new(
                                    actual_w, actual_h,
                                ));
                            }
                            Err(_) => {
                                ctx.swapchain_dirty = true;
                            }
                        }
                        ctx.pacer.on_present(Instant::now());
                    }
                    Ok(AcquireOutcome::OutOfDate) => {
                        ctx.swapchain_dirty = true;
                        ctx.frame_dropped = true;
                        state
                            .resize
                            .mark_out_of_date(unbound_platform::Extent::new(width, height));
                    }
                    _ => {
                        ctx.frame_dropped = true;
                    }
                }
            }
        }

        let keep_animating =
            ctx.runtime.context().scale_osd_remaining > 0.0 || report.dirty.needs_rebuild();
        ctx.pacer.set_animating(keep_animating);
        if keep_animating {
            window.request_redraw();
        }

        if self.drain_window_commands_live(id, window) == Flow::Exit {
            return Flow::Exit;
        }
        Flow::Continue
    }
}

/// 启动多窗口共享设备应用的顶层入口函数。
///
/// # Errors
/// 平台窗口创建或事件循环初始化失败时返回 [`PlatformError`]。
pub fn run_multi_window<A, F>(primary_config: WindowConfig, factory: F) -> Result<(), PlatformError>
where
    A: Application + 'static,
    F: MultiWindowFactory<A> + 'static,
{
    run_multi_window_with_config(MultiWindowConfig::new(primary_config), factory)
}

/// 使用 [`MultiWindowConfig`] 启动多窗口共享设备应用（支持 `on_window_created` 钩子）。
///
/// # Errors
/// 平台窗口创建或事件循环初始化失败时返回 [`PlatformError`]。
pub fn run_multi_window_with_config<A, F>(
    mut config: MultiWindowConfig,
    factory: F,
) -> Result<(), PlatformError>
where
    A: Application + 'static,
    F: MultiWindowFactory<A> + 'static,
{
    config.primary_config.decorations = false;
    let primary_config = config.primary_config.clone();
    let mut coordinator = MultiWindowCoordinator::new(primary_config.clone(), factory);
    coordinator.on_window_created = config.on_window_created;
    PlatformWindow::new(&primary_config)?.run(coordinator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, AppEvent, DirtyFlags, EventOutcome, ViewBuilder};
    use unbound_platform::RenderThrottlePolicy;

    struct TaggedApp {
        window_id: WindowId,
        events: Vec<String>,
    }

    impl Application for TaggedApp {
        type State = ();

        fn init(&mut self, _: &mut AppContext) {}

        fn reduce(&mut self, _: &mut (), event: AppEvent, _: &mut AppContext) -> EventOutcome {
            if let AppEvent::Custom { payload, .. } = event {
                self.events.push(payload.clone());
                return EventOutcome::new(DirtyFlags::PAINT);
            }
            EventOutcome::new(DirtyFlags::NONE)
        }

        fn view(&mut self, _: &(), v: &mut ViewBuilder<'_>) {
            v.text_primary(
                self.window_id.0,
                format!("Window {}", self.window_id.0),
                14.0,
                400,
            );
        }
    }

    #[test]
    fn multi_window_isolates_events_commands_and_closing_preserves_survivors() {
        let primary_cfg = WindowConfig::default().with_title("Primary");
        let mut coord =
            MultiWindowCoordinator::new(primary_cfg.clone(), |id: WindowId, _: &WindowConfig| {
                TaggedApp {
                    window_id: id,
                    events: Vec::new(),
                }
            });

        coord.register_headless_window(WindowId(1), primary_cfg);
        // 通过 OpenWindow 命令请求打开辅助窗口。
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .runtime
            .open_window(WindowConfig::default().with_title("Inspector"));
        assert!(coord.route_window_commands(WindowId(1)).is_empty());

        let next_cfg = coord
            .take_window_request()
            .expect("OpenWindow command should yield a pending WindowConfig");
        assert_eq!(next_cfg.title, "Inspector");
        coord.register_headless_window(WindowId(2), next_cfg);
        assert_eq!(coord.registry().len(), 2);
        assert_eq!(coord.registry().primary(), Some(WindowId(1)));

        // 向窗口 1 与窗口 2 分别投递独立事件，互不串扰。
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .runtime
            .dispatch(AppEvent::Custom {
                id: 1,
                payload: "only-win-1".into(),
            });
        coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .runtime
            .dispatch(AppEvent::Custom {
                id: 2,
                payload: "only-win-2".into(),
            });
        assert_eq!(
            coord
                .registry()
                .get(WindowId(1))
                .unwrap()
                .runtime
                .app()
                .events,
            vec!["only-win-1"]
        );
        assert_eq!(
            coord
                .registry()
                .get(WindowId(2))
                .unwrap()
                .runtime
                .app()
                .events,
            vec!["only-win-2"]
        );

        // 通过 CloseWindow 关闭辅助窗口 2，窗口 1 保持完整无损。
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .runtime
            .close_window(WindowId(2));
        let closed = coord.route_window_commands(WindowId(1));
        assert_eq!(closed, vec![WindowId(2)]);
        assert_eq!(coord.take_window_close_requests(), vec![WindowId(2)]);
        assert!(coord.take_window_close_requests().is_empty());
        assert_eq!(coord.registry().len(), 1);
        assert!(coord.registry().get(WindowId(2)).is_none());
        assert_eq!(
            coord
                .registry()
                .get(WindowId(1))
                .unwrap()
                .runtime
                .app()
                .events,
            vec!["only-win-1"]
        );
    }

    #[test]
    fn mixed_throttling_suspends_only_occluded_window_without_blocking_active_window() {
        let cfg = WindowConfig::default();
        let mut coord =
            MultiWindowCoordinator::new(cfg.clone(), |id: WindowId, _: &WindowConfig| TaggedApp {
                window_id: id,
                events: Vec::new(),
            });
        coord.register_headless_window(WindowId(1), cfg.clone());
        coord.register_headless_window(WindowId(2), cfg);

        let policy = RenderThrottlePolicy::default();
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .pacer
            .set_throttle_policy(policy);
        coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .pacer
            .set_throttle_policy(policy);

        // 窗口 1 被完全遮挡进入暂停态，窗口 2 失焦但仍可见，按 ADR-0030/ADR-0023 独立保持活跃不降帧。
        let t0 = Instant::now();
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .pacer
            .set_occluded(true);
        coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .pacer
            .note_activity(t0);
        coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .pacer
            .set_focused(false);
        assert_eq!(
            coord
                .registry()
                .get(WindowId(2))
                .unwrap()
                .pacer
                .throttle_state(t0 + std::time::Duration::from_secs(5)),
            unbound_platform::RenderThrottleState::Active
        );

        assert!(coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .should_skip_frame_for_throttle());
        assert!(!coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .should_skip_frame_for_throttle());

        // 窗口 2 活跃动画时，即使窗口 1 遮挡暂停，协调器的 next_wake 仍返回窗口 2 的唤醒时间。
        coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .pacer
            .set_animating(true);
        assert!(coord.next_wake().is_some());

        // 窗口 2 可独立正常出帧。
        let report_b = coord
            .registry_mut()
            .get_mut(WindowId(2))
            .unwrap()
            .runtime
            .frame(0.0);
        assert!(report_b.primitives > 0);
    }

    #[test]
    fn shared_gpu_and_atlas_handles_use_arc_ptr_identity() {
        // 验证共享句柄封装语义与主窗口关闭提升规则。
        let cfg = WindowConfig::default();
        let mut coord =
            MultiWindowCoordinator::new(cfg.clone(), |id: WindowId, _: &WindowConfig| TaggedApp {
                window_id: id,
                events: Vec::new(),
            });
        coord.register_headless_window(WindowId(10), cfg.clone());
        coord.register_headless_window(WindowId(20), cfg);
        assert_eq!(coord.registry().primary(), Some(WindowId(10)));
        assert!(coord.close_window(WindowId(10)));
        assert_eq!(coord.registry().primary(), Some(WindowId(20)));
        assert!(coord.shared_gpu().is_none());
        assert!(coord.shared_atlases().is_none());
    }

    #[test]
    fn multi_window_theme_transition_shares_timestamp_and_skips_suspended_windows() {
        struct DummyApp;
        impl Application for DummyApp {
            type State = ();
            fn init(&mut self, _ctx: &mut crate::AppContext) {}
            fn reduce(
                &mut self,
                _state: &mut Self::State,
                _event: crate::AppEvent,
                _ctx: &mut crate::AppContext,
            ) -> crate::EventOutcome {
                crate::EventOutcome::default()
            }
            fn view(&mut self, _state: &Self::State, _view: &mut crate::ViewBuilder<'_>) {}
        }

        let cfg = WindowConfig::default();
        let mut win_a = WindowContext::new(DummyApp, &cfg);
        let mut win_b = WindowContext::new(DummyApp, &cfg);
        let mut win_suspended = WindowContext::new(DummyApp, &cfg);

        // 首帧全部强制瞬切至 Dark。
        assert!(win_a.sync_theme_state_at(0.0, 0.0, false));
        assert!(win_b.sync_theme_state_at(0.0, 0.0, false));
        assert!(win_suspended.sync_theme_state_at(0.0, 0.0, false));
        assert!(!win_a.theme_driver.is_active());

        // 第三个窗口进入 Suspended 停绘态。
        win_suspended.pacer.set_minimized(true);
        assert!(win_suspended.should_skip_frame_for_throttle());

        // 全局切换至 Light，所有可见窗口共享 trigger_secs = 1.0。
        win_a.runtime.set_theme(ThemeKind::Light);
        win_b.runtime.set_theme(ThemeKind::Light);
        win_suspended.runtime.set_theme(ThemeKind::Light);

        // 校验零布局重排不变量：清空脏标记后推进过渡帧，脏标记保持 NONE。
        let _ = win_a.runtime.frame(0.0);
        assert_eq!(win_a.runtime.dirty(), crate::DirtyFlags::NONE);

        assert!(win_a.sync_theme_state_at(1.10, 1.0, false));
        assert!(win_b.sync_theme_state_at(1.10, 1.0, false));
        assert_eq!(win_a.runtime.dirty(), crate::DirtyFlags::NONE);
        // 两可见窗口进度与当前 LUT 表面色严格一致。
        assert!((win_a.theme_driver.progress() - win_b.theme_driver.progress()).abs() < 1e-6);
        assert_eq!(
            win_a
                .theme_driver
                .lut()
                .color(unbound_theme::Token::SURFACE),
            win_b
                .theme_driver
                .lut()
                .color(unbound_theme::Token::SURFACE)
        );

        // Suspended 窗口不唤醒重绘。
        assert!(!win_suspended.sync_theme_state_at(1.10, 1.0, false));
        assert!(!win_suspended.theme_driver.is_active());

        // 恢复可见后直接吸附终态 Light（t = 1.0），不补播过期动画。
        win_suspended.pacer.set_minimized(false);
        assert!(!win_suspended.should_skip_frame_for_throttle());
        assert!(!win_suspended.theme_driver.is_active());
        assert_eq!(win_suspended.theme_driver.target_kind(), ThemeKind::Light);
        assert_eq!(win_suspended.theme_driver.progress(), 1.0);
    }

    #[test]
    fn reduced_motion_forces_instant_theme_snap_in_runner() {
        struct DummyApp;
        impl Application for DummyApp {
            type State = ();
            fn init(&mut self, _ctx: &mut crate::AppContext) {}
            fn reduce(
                &mut self,
                _state: &mut Self::State,
                _event: crate::AppEvent,
                _ctx: &mut crate::AppContext,
            ) -> crate::EventOutcome {
                crate::EventOutcome::default()
            }
            fn view(&mut self, _state: &Self::State, _view: &mut crate::ViewBuilder<'_>) {}
        }

        let cfg = WindowConfig::default();
        let mut win = WindowContext::new(DummyApp, &cfg);
        assert!(win.sync_theme_state_at(0.0, 0.0, false));

        win.runtime.set_theme(ThemeKind::Light);
        // 系统开启减弱动画（reduced_motion = true）：立即到达终态，不进入过渡态。
        assert!(win.sync_theme_state_at(1.0, 1.0, true));
        assert!(!win.theme_driver.is_active());
        assert_eq!(win.theme_driver.target_kind(), ThemeKind::Light);
        assert_eq!(win.theme_driver.progress(), 1.0);
    }
}

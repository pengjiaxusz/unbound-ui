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
    PlatformState, ScreenRect, SurfaceAlpha, WindowConfig, WindowPosition,
};
use unbound_platform::{
    begin_outbound_file_drag, clamp_rect_to_work_area, resolve_floating_placement, solve_edge_snap,
    FloatingPlacementStrategy, FloatingWindowCapabilities, FloatingWindowPolicy, FramePacer,
    MonitorWorkArea, NativeWindow, OutboundFileDragOutcome, PopupHostRegistry, PopupWindowConfig,
    RelativeMonitorAnchor, WindowId, WindowRegistry, WindowRole,
};
use unbound_theme::ThemeSet;

use crate::appearance::{AppearanceCaps, AppearanceDiff, WindowAppearance};
use crate::floating_host::FloatingHost;
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

/// 待同步到跨窗口目标 `NativeWindow` 的运行期热更新状态。
#[derive(Clone, Copy, Debug, Default)]
struct FloatingLiveSync {
    visible: Option<bool>,
    topmost: Option<bool>,
    click_through: Option<bool>,
    outer_rect: Option<ScreenRect>,
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
    floating_host: FloatingHost,
    focused_window_id: Option<WindowId>,
    headless_work_areas: Vec<MonitorWorkArea>,
    pending_floating_live_syncs: FxHashMap<WindowId, FloatingLiveSync>,
    primary_config: WindowConfig,
    factory: Box<dyn MultiWindowFactory<A>>,
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
            floating_host: FloatingHost::with_current_platform(),
            focused_window_id: None,
            headless_work_areas: Vec::new(),
            pending_floating_live_syncs: FxHashMap::default(),
            primary_config,
            factory: Box::new(factory),
            waker: None,
            theme_clock_epoch: Instant::now(),
            shared_theme_trigger: None,
        }
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

    fn default_fallback_work_area() -> MonitorWorkArea {
        MonitorWorkArea {
            monitor_bounds: ScreenRect::new(0, 0, 1920, 1080),
            work_area: ScreenRect::new(0, 0, 1920, 1040),
            scale_factor: 1.0,
            is_primary: true,
        }
    }

    fn select_work_area_for_rect(&self, desired: ScreenRect) -> MonitorWorkArea {
        if self.headless_work_areas.is_empty() {
            return Self::default_fallback_work_area();
        }
        let cx = i64::from(desired.x) + i64::from(desired.width) / 2;
        let cy = i64::from(desired.y) + i64::from(desired.height) / 2;
        if let Some(hit) = self.headless_work_areas.iter().find(|m| {
            let b = m.monitor_bounds;
            cx >= i64::from(b.x)
                && cx < i64::from(b.x) + i64::from(b.width)
                && cy >= i64::from(b.y)
                && cy < i64::from(b.y) + i64::from(b.height)
        }) {
            return hit.clone();
        }
        self.headless_work_areas
            .iter()
            .find(|m| m.is_primary)
            .cloned()
            .unwrap_or_else(|| self.headless_work_areas[0].clone())
    }

    fn resolve_open_floating_config(
        &self,
        mut config: WindowConfig,
        placement: Option<FloatingPlacementStrategy>,
    ) -> WindowConfig {
        config.decorations = false;
        config.surface_alpha = SurfaceAlpha::PreMultiplied;
        if let Some(strategy) = placement {
            let target_rect = match strategy {
                FloatingPlacementStrategy::CursorMonitorAvoid { cursor, size, .. } => {
                    ScreenRect::new(cursor.0, cursor.1, size.0, size.1)
                }
                FloatingPlacementStrategy::AnchorRectAvoid { anchor, .. } => anchor,
                FloatingPlacementStrategy::EdgeSnap { desired, .. } => desired,
            };
            let work_area = self.select_work_area_for_rect(target_rect).work_area;
            let rect = resolve_floating_placement(&strategy, work_area);
            config.position = WindowPosition::Physical {
                x: rect.x,
                y: rect.y,
            };
            config.width = rect.width.max(1);
            config.height = rect.height.max(1);
        }
        config
    }

    fn initial_rect_from_config(config: &WindowConfig) -> ScreenRect {
        match config.position {
            WindowPosition::Physical { x, y } | WindowPosition::Logical { x, y } => {
                ScreenRect::new(x, y, config.width.max(1), config.height.max(1))
            }
            _ => ScreenRect::new(0, 0, config.width.max(1), config.height.max(1)),
        }
    }

    fn is_window_no_activate(&self, id: WindowId) -> bool {
        if let Some(state) = self.floating_host.get(id) {
            return state.effective_policy.no_activate;
        }
        self.registry
            .get(id)
            .is_some_and(|ctx| ctx.config.role != WindowRole::Standard && ctx.config.floating_policy.no_activate)
    }

    /// 无头注册一个窗口上下文（主要用于无头多窗口测试与集成验证）。
    pub fn register_headless_window(&mut self, id: WindowId, config: WindowConfig) {
        if config.role != WindowRole::Standard {
            self.register_headless_floating_window(id, config);
            return;
        }
        let app = self.factory.create(id, &config);
        let mut ctx = WindowContext::new(app, &config);
        if let Some(waker) = &self.waker {
            ctx.runtime
                .set_waker(Arc::new(SignalWaker(Arc::clone(waker))));
        }
        let visible = config.visible;
        let _ = self.registry.insert(id, ctx);
        self.registry.set_visible(id, visible);
        if self.focused_window_id.is_none() && visible {
            self.focused_window_id = Some(id);
        }
    }

    /// 无头注册一个幽灵窗或悬浮浮岛上下文（含策略、初始外框与焦点隔离初始化）。
    pub fn register_headless_floating_window(&mut self, id: WindowId, mut config: WindowConfig) {
        config.decorations = false;
        config.surface_alpha = SurfaceAlpha::PreMultiplied;
        let mut outer_rect = Self::initial_rect_from_config(&config);
        if self.floating_host.capabilities().supports_global_positioning
            && !self.headless_work_areas.is_empty()
        {
            let area = self.select_work_area_for_rect(outer_rect);
            outer_rect = clamp_rect_to_work_area(outer_rect, area.work_area);
        }
        let role = config.role;
        let policy = config.floating_policy;
        let visible = config.visible;

        let app = self.factory.create(id, &config);
        let mut ctx = WindowContext::new(app, &config);
        if let Some(waker) = &self.waker {
            ctx.runtime
                .set_waker(Arc::new(SignalWaker(Arc::clone(waker))));
        }
        let _ = self.registry.insert(id, ctx);
        self.registry.set_visible(id, visible);
        let state = self
            .floating_host
            .register(id, role, policy, outer_rect, visible);
        // 绝不将 Ghost 提升为主窗口；若当前主窗口是非 Standard，则尝试保持首个 Standard 为主窗口。
        if let Some(primary) = self.registry.primary() {
            if primary == id && role == WindowRole::Ghost {
                if let Some(std_slot) = self
                    .registry
                    .iter()
                    .find(|s| s.context.config.role == WindowRole::Standard)
                {
                    let std_id = std_slot.id;
                    let _ = self.registry.set_primary(std_id);
                }
            }
        }
        if !state.effective_policy.no_activate && visible && self.focused_window_id.is_none() {
            self.focused_window_id = Some(id);
        }
    }

    /// 无头模拟窗口焦点事件，验证 `no_activate` 浮窗绝不抢占宿主焦点。
    pub fn simulate_window_focus_event(&mut self, id: WindowId, focused: bool) {
        if self.is_window_no_activate(id) {
            if let Some(ctx) = self.registry.get_mut(id) {
                ctx.runtime.set_window_focused(false);
            }
            return;
        }
        if focused {
            self.focused_window_id = Some(id);
        } else if self.focused_window_id == Some(id) {
            self.focused_window_id = None;
        }
        if let Some(ctx) = self.registry.get_mut(id) {
            ctx.pacer.set_focused(focused);
            ctx.runtime.set_window_focused(focused);
        }
    }

    /// 查询指定窗口的角色。
    #[must_use]
    pub fn window_role(&self, id: WindowId) -> Option<WindowRole> {
        self.floating_host
            .get(id)
            .map(|s| s.role)
            .or_else(|| self.registry.get(id).map(|c| c.config.role))
    }

    /// 查询指定浮窗当前生效的策略。
    #[must_use]
    pub fn floating_policy(&self, id: WindowId) -> Option<FloatingWindowPolicy> {
        self.floating_host
            .get(id)
            .map(|s| s.effective_policy)
            .or_else(|| self.registry.get(id).map(|c| c.config.floating_policy))
    }

    /// 查询指定浮窗当前记录的屏幕物理外框矩形。
    #[must_use]
    pub fn window_outer_rect(&self, id: WindowId) -> Option<ScreenRect> {
        self.floating_host.get(id).map(|s| s.outer_rect)
    }

    /// 查询指定窗口当前是否可见。
    #[must_use]
    pub fn window_visible(&self, id: WindowId) -> Option<bool> {
        self.floating_host
            .get(id)
            .map(|s| s.visible)
            .or_else(|| {
                self.registry
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.visible)
            })
    }

    /// 查询指定浮窗的实例代次（仅销毁重建时递增，热切换保持不变）。
    #[must_use]
    pub fn window_generation(&self, id: WindowId) -> Option<u64> {
        self.floating_host.get(id).map(|s| s.generation)
    }

    /// 查询指定浮窗最近一次边缘磁吸记录的相对显示器锚点。
    #[must_use]
    pub fn last_snap_anchor(&self, id: WindowId) -> Option<RelativeMonitorAnchor> {
        self.floating_host
            .get(id)
            .and_then(|s| s.last_snap_anchor.clone())
    }

    /// 查询当前拥有键盘焦点的窗口 ID。
    #[must_use]
    pub const fn focused_window_id(&self) -> Option<WindowId> {
        self.focused_window_id
    }

    /// 获取当前协调器持有的浮窗平台能力矩阵。
    #[must_use]
    pub const fn floating_capabilities(&self) -> FloatingWindowCapabilities {
        self.floating_host.capabilities()
    }

    /// 在无头模式下注入平台能力矩阵（用于测试 Win32 / X11 / Wayland 降级路径）。
    pub fn set_headless_capabilities(&mut self, caps: FloatingWindowCapabilities) {
        self.floating_host.set_capabilities(caps);
    }

    /// 在无头模式下注入多显示器工作区列表（用于测试负坐标辅屏、避让与边缘磁吸）。
    pub fn set_headless_work_areas(&mut self, areas: Vec<MonitorWorkArea>) {
        self.headless_work_areas = areas;
    }

    /// 按严格回收顺序关闭并移除指定窗口；若无任何 `Standard` 窗口存活则级联关闭全部浮窗并释放共享 GPU。
    pub fn close_window(&mut self, id: WindowId) -> bool {
        let _ = self.floating_host.unregister(id);
        self.pending_floating_live_syncs.remove(&id);
        if self.focused_window_id == Some(id) {
            self.focused_window_id = None;
        }
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

        // 绝不将 Ghost 提升为主窗口；若不再有 Standard 窗口存活，级联关闭所有剩余浮窗。
        let next_standard = self
            .registry
            .iter()
            .find(|slot| slot.context.config.role == WindowRole::Standard)
            .map(|slot| slot.id);
        if let Some(std_id) = next_standard {
            if let Some(primary_id) = self.registry.primary() {
                if self
                    .registry
                    .get(primary_id)
                    .is_some_and(|c| c.config.role == WindowRole::Ghost)
                {
                    let _ = self.registry.set_primary(std_id);
                }
            }
        } else if !self.registry.is_empty() {
            let remaining_ids: Vec<WindowId> = self.registry.iter().map(|slot| slot.id).collect();
            for rem_id in remaining_ids {
                let _ = self.floating_host.unregister(rem_id);
                self.pending_floating_live_syncs.remove(&rem_id);
                if self.focused_window_id == Some(rem_id) {
                    self.focused_window_id = None;
                }
                if let Some(mut rem_ctx) = self.registry.remove(rem_id) {
                    for action in rem_ctx.popup_host.close_for_owner(rem_id) {
                        if let unbound_platform::PopupSyncAction::Close { id: popup_id, .. } = action {
                            self.pending_popup_closes.push(popup_id);
                        }
                    }
                    rem_ctx.destroy_vulkan();
                    drop(rem_ctx);
                    self.pending_window_close_requests.push(rem_id);
                }
            }
        }

        if self.registry.is_empty() {
            self.release_shared_gpu();
        }
        true
    }

    fn apply_floating_command_state(&mut self, cmd: &WindowCommand) {
        match cmd {
            WindowCommand::UpdateFloatingPolicy { id, policy } => {
                if let Some(eff) = self.floating_host.update_policy(*id, *policy) {
                    if let Some(target_ctx) = self.registry.get_mut(*id) {
                        target_ctx.config.floating_policy = *policy;
                    }
                    let sync = self.pending_floating_live_syncs.entry(*id).or_default();
                    sync.topmost = Some(eff.always_on_top);
                    sync.click_through = Some(eff.click_through);
                }
            }
            WindowCommand::SetWindowVisibleNoActivate { id, visible } => {
                if self.floating_host.set_visible(*id, *visible) {
                    self.registry.set_visible(*id, *visible);
                    let sync = self.pending_floating_live_syncs.entry(*id).or_default();
                    sync.visible = Some(*visible);
                }
            }
            WindowCommand::SetWindowAlwaysOnTop { id, topmost } => {
                if let Some(eff_topmost) = self.floating_host.set_always_on_top(*id, *topmost) {
                    if let Some(target_ctx) = self.registry.get_mut(*id) {
                        target_ctx.config.floating_policy.always_on_top = *topmost;
                    }
                    let sync = self.pending_floating_live_syncs.entry(*id).or_default();
                    sync.topmost = Some(eff_topmost);
                }
            }
            WindowCommand::SetWindowClickThrough { id, click_through } => {
                if let Some(eff_ct) = self.floating_host.set_click_through(*id, *click_through) {
                    if let Some(target_ctx) = self.registry.get_mut(*id) {
                        target_ctx.config.floating_policy.click_through = *click_through;
                    }
                    let sync = self.pending_floating_live_syncs.entry(*id).or_default();
                    sync.click_through = Some(eff_ct);
                }
            }
            WindowCommand::SetWindowOuterRect { id, rect } => {
                let target_rect = if self.floating_host.capabilities().supports_global_positioning
                    && !self.headless_work_areas.is_empty()
                {
                    let area = self.select_work_area_for_rect(*rect);
                    clamp_rect_to_work_area(*rect, area.work_area)
                } else {
                    *rect
                };
                if let Some(applied) = self.floating_host.set_outer_rect(*id, target_rect) {
                    if self.floating_host.capabilities().supports_global_positioning {
                        let sync = self.pending_floating_live_syncs.entry(*id).or_default();
                        sync.outer_rect = Some(applied);
                    }
                }
            }
            WindowCommand::SnapFloatingWindow {
                id,
                desired,
                snap_threshold,
            } => {
                let area = self.select_work_area_for_rect(*desired);
                let snap = solve_edge_snap(*desired, area.work_area, *snap_threshold);
                if self
                    .floating_host
                    .set_snap_anchor(*id, snap.rect, Some(snap.anchor))
                {
                    if self.floating_host.capabilities().supports_global_positioning {
                        let sync = self.pending_floating_live_syncs.entry(*id).or_default();
                        sync.outer_rect = Some(snap.rect);
                    }
                }
            }
            _ => {}
        }
    }

    fn flush_floating_live_sync(&mut self, id: WindowId, window: &NativeWindow) {
        let Some(sync) = self.pending_floating_live_syncs.remove(&id) else {
            return;
        };
        if let Some(visible) = sync.visible {
            window.set_visible_no_activate(visible);
        }
        if let Some(topmost) = sync.topmost {
            window.set_always_on_top(topmost);
        }
        if let Some(click_through) = sync.click_through {
            window.set_click_through(click_through);
        }
        if let Some(rect) = sync.outer_rect {
            window.set_outer_rect(rect);
        }
    }

    /// 排空指定窗口上下文产出的命令（无头安全路径：不依赖原生 `Window` 句柄）。
    ///
    /// 返回需要关闭的目标窗口 ID 列表。
    pub fn route_window_commands(&mut self, source_id: WindowId) -> Vec<WindowId> {
        let mut to_close = Vec::new();
        let Some(cmds) = self
            .registry
            .get_mut(source_id)
            .map(|ctx| ctx.runtime.drain_window_commands())
        else {
            return to_close;
        };
        for cmd in cmds {
            match cmd {
                WindowCommand::OpenWindow(mut config) => {
                    config.decorations = false;
                    self.pending_windows.push_back(config);
                }
                WindowCommand::OpenFloatingWindow { config, placement } => {
                    let resolved = self.resolve_open_floating_config(config, placement);
                    self.pending_windows.push_back(resolved);
                }
                WindowCommand::CloseWindow(target) | WindowCommand::CloseFloatingWindow(target) => {
                    to_close.push(target);
                }
                WindowCommand::Close => {
                    to_close.push(source_id);
                }
                WindowCommand::SetThrottlePolicy(policy) => {
                    if let Some(ctx) = self.registry.get_mut(source_id) {
                        ctx.pacer.set_throttle_policy(policy);
                    }
                }
                WindowCommand::SyncNativePopup { id, config } => {
                    if let Some(ctx) = self.registry.get_mut(source_id) {
                        if ctx.popup_host.sync_popup(id, config).is_some() {
                            self.pending_popup_syncs.push((id, config));
                        }
                    }
                }
                WindowCommand::CloseNativePopup { id } => {
                    if let Some(ctx) = self.registry.get_mut(source_id) {
                        if ctx.popup_host.close_popup(id).is_some() {
                            self.pending_popup_closes.push(id);
                        }
                    }
                }
                cmd @ (WindowCommand::UpdateFloatingPolicy { .. }
                | WindowCommand::SetWindowVisibleNoActivate { .. }
                | WindowCommand::SetWindowAlwaysOnTop { .. }
                | WindowCommand::SetWindowClickThrough { .. }
                | WindowCommand::SetWindowOuterRect { .. }
                | WindowCommand::SnapFloatingWindow { .. }) => {
                    self.apply_floating_command_state(&cmd);
                }
                WindowCommand::StartDrag
                | WindowCommand::Minimize
                | WindowCommand::ToggleMaximize
                | WindowCommand::BeginExternalFileDrag(_) => {}
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

        let caps = window.capabilities();
        self.floating_host.set_capabilities(caps);
        let role = config.role;
        let policy = config.floating_policy;
        let visible = config.visible;
        let outer_rect = ScreenRect::new(
            client_screen.x,
            client_screen.y,
            width,
            height,
        );
        let _ = self.registry.insert(id, ctx);
        self.registry.set_visible(id, visible);
        if role != WindowRole::Standard {
            let state = self
                .floating_host
                .register(id, role, policy, outer_rect, visible);
            let eff = state.effective_policy;
            window.set_always_on_top(eff.always_on_top);
            window.set_click_through(eff.click_through);
            window.set_visible_no_activate(visible);
            if !eff.no_activate && visible && self.focused_window_id.is_none() {
                self.focused_window_id = Some(id);
            }
        } else if visible && self.focused_window_id.is_none() {
            self.focused_window_id = Some(id);
        }
    }

    fn drain_window_commands_live(&mut self, id: WindowId, window: &NativeWindow) -> Flow {
        let mut outbound_drag = None;
        let mut close_targets = Vec::new();
        let mut close_self = false;
        let mut deferred_floating = Vec::new();

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
                    WindowCommand::OpenFloatingWindow { config, placement } => {
                        deferred_floating.push(WindowCommand::OpenFloatingWindow { config, placement });
                    }
                    WindowCommand::CloseWindow(target)
                    | WindowCommand::CloseFloatingWindow(target) => {
                        if target == id {
                            close_self = true;
                        } else {
                            close_targets.push(target);
                        }
                    }
                    other @ (WindowCommand::UpdateFloatingPolicy { .. }
                    | WindowCommand::SetWindowVisibleNoActivate { .. }
                    | WindowCommand::SetWindowAlwaysOnTop { .. }
                    | WindowCommand::SetWindowClickThrough { .. }
                    | WindowCommand::SetWindowOuterRect { .. }
                    | WindowCommand::SnapFloatingWindow { .. }) => {
                        deferred_floating.push(other);
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

        if !deferred_floating.is_empty() {
            self.floating_host.set_capabilities(window.capabilities());
            let live_work_area = window.work_area();
            for cmd in deferred_floating {
                match cmd {
                    WindowCommand::OpenFloatingWindow { config, placement } => {
                        let resolved = if let Some(area) = &live_work_area {
                            let mut c = config;
                            c.decorations = false;
                            c.surface_alpha = SurfaceAlpha::PreMultiplied;
                            if let Some(strategy) = placement {
                                let rect = resolve_floating_placement(&strategy, area.work_area);
                                c.position = WindowPosition::Physical {
                                    x: rect.x,
                                    y: rect.y,
                                };
                                c.width = rect.width.max(1);
                                c.height = rect.height.max(1);
                            }
                            c
                        } else {
                            self.resolve_open_floating_config(config, placement)
                        };
                        self.pending_windows.push_back(resolved);
                    }
                    WindowCommand::SnapFloatingWindow {
                        id: target,
                        desired,
                        snap_threshold,
                    } => {
                        let work_area = live_work_area
                            .as_ref()
                            .map(|m| m.work_area)
                            .unwrap_or_else(|| self.select_work_area_for_rect(desired).work_area);
                        let snap = solve_edge_snap(desired, work_area, snap_threshold);
                        if self
                            .floating_host
                            .set_snap_anchor(target, snap.rect, Some(snap.anchor))
                            && self.floating_host.capabilities().supports_global_positioning
                        {
                            let sync = self.pending_floating_live_syncs.entry(target).or_default();
                            sync.outer_rect = Some(snap.rect);
                        }
                    }
                    other => self.apply_floating_command_state(&other),
                }
            }
        }

        self.flush_floating_live_sync(id, window);

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
                let no_activate = ctx.config.role != WindowRole::Standard
                    && ctx.config.floating_policy.no_activate;
                if no_activate {
                    ctx.runtime.set_window_focused(false);
                } else {
                    ctx.pacer.set_focused(focused);
                    if focused {
                        ctx.pacer.note_activity(Instant::now());
                    }
                    ctx.runtime.set_window_focused(focused);
                }
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
        if let PlatformEvent::Focused(focused) = event {
            if !self.is_window_no_activate(id) {
                if focused {
                    self.focused_window_id = Some(id);
                } else if self.focused_window_id == Some(id) {
                    self.focused_window_id = None;
                }
            }
        }
        self.flush_floating_live_sync(id, window);
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
pub fn run_multi_window<A, F>(
    mut primary_config: WindowConfig,
    factory: F,
) -> Result<(), PlatformError>
where
    A: Application + 'static,
    F: MultiWindowFactory<A> + 'static,
{
    primary_config.decorations = false;
    let coordinator = MultiWindowCoordinator::new(primary_config.clone(), factory);
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

    #[test]
    fn ghost_and_floating_island_lifecycle_and_focus_isolation() {
        let primary_cfg = WindowConfig::default().with_title("Primary");
        let mut coord =
            MultiWindowCoordinator::new(primary_cfg.clone(), |id: WindowId, _: &WindowConfig| {
                TaggedApp {
                    window_id: id,
                    events: Vec::new(),
                }
            });
        coord.set_headless_capabilities(FloatingWindowCapabilities::win32_full());
        coord.register_headless_window(WindowId(1), primary_cfg);
        assert_eq!(coord.focused_window_id(), Some(WindowId(1)));

        let (ghost_cfg, _) = crate::FloatingWindowSpec::ghost("Hover Ghost", 280, 160).build();
        coord.register_headless_floating_window(WindowId(2), ghost_cfg);
        assert_eq!(coord.window_role(WindowId(2)), Some(WindowRole::Ghost));
        assert_eq!(coord.window_visible(WindowId(2)), Some(false));
        assert_eq!(coord.window_generation(WindowId(2)), Some(1));
        assert_eq!(coord.focused_window_id(), Some(WindowId(1)));

        // 无激活显示幽灵窗与模拟焦点事件均不得抢占主窗口焦点。
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .runtime
            .set_window_visible_no_activate(WindowId(2), true);
        assert!(coord.route_window_commands(WindowId(1)).is_empty());
        assert_eq!(coord.window_visible(WindowId(2)), Some(true));
        coord.simulate_window_focus_event(WindowId(2), true);
        assert_eq!(coord.focused_window_id(), Some(WindowId(1)));

        // 注册悬浮浮岛同样不抢占主窗口焦点。
        let (island_cfg, _) =
            crate::FloatingWindowSpec::floating_island("Media Island", 320, 88).build();
        coord.register_headless_floating_window(WindowId(3), island_cfg);
        assert_eq!(
            coord.window_role(WindowId(3)),
            Some(WindowRole::FloatingIsland)
        );
        assert_eq!(coord.window_visible(WindowId(3)), Some(true));
        coord.simulate_window_focus_event(WindowId(3), true);
        assert_eq!(coord.focused_window_id(), Some(WindowId(1)));
    }

    #[test]
    fn floating_hot_toggles_preserve_generation_and_never_recreate() {
        let primary_cfg = WindowConfig::default().with_title("Primary");
        let mut coord =
            MultiWindowCoordinator::new(primary_cfg.clone(), |id: WindowId, _: &WindowConfig| {
                TaggedApp {
                    window_id: id,
                    events: Vec::new(),
                }
            });
        coord.set_headless_capabilities(FloatingWindowCapabilities::win32_full());
        coord.register_headless_window(WindowId(1), primary_cfg);

        let (island_cfg, _) =
            crate::FloatingWindowSpec::floating_island("Island", 300, 100).build();
        coord.register_headless_floating_window(WindowId(2), island_cfg);
        assert_eq!(coord.window_generation(WindowId(2)), Some(1));

        let rt = &mut coord.registry_mut().get_mut(WindowId(1)).unwrap().runtime;
        rt.set_window_click_through(WindowId(2), true);
        rt.set_window_always_on_top(WindowId(2), false);
        rt.set_window_visible_no_activate(WindowId(2), false);
        let target_rect = ScreenRect::new(200, 150, 340, 120);
        rt.set_window_outer_rect(WindowId(2), target_rect);
        assert!(coord.route_window_commands(WindowId(1)).is_empty());

        // 全部运行期热切换不得销毁重建窗口（generation 保持为 1）。
        assert_eq!(coord.window_generation(WindowId(2)), Some(1));
        let policy = coord.floating_policy(WindowId(2)).unwrap();
        assert!(policy.click_through);
        assert!(!policy.always_on_top);
        assert_eq!(coord.window_visible(WindowId(2)), Some(false));
        assert_eq!(coord.window_outer_rect(WindowId(2)), Some(target_rect));
        assert_eq!(coord.focused_window_id(), Some(WindowId(1)));
    }

    #[test]
    fn multi_monitor_negative_coordinates_edge_snap_and_clamping() {
        let primary_cfg = WindowConfig::default().with_title("Primary");
        let mut coord =
            MultiWindowCoordinator::new(primary_cfg.clone(), |id: WindowId, _: &WindowConfig| {
                TaggedApp {
                    window_id: id,
                    events: Vec::new(),
                }
            });
        coord.set_headless_capabilities(FloatingWindowCapabilities::win32_full());
        coord.set_headless_work_areas(vec![
            MonitorWorkArea {
                monitor_bounds: ScreenRect::new(0, 0, 1920, 1080),
                work_area: ScreenRect::new(0, 0, 1920, 1040),
                scale_factor: 1.0,
                is_primary: true,
            },
            MonitorWorkArea {
                monitor_bounds: ScreenRect::new(-1920, 0, 1920, 1080),
                work_area: ScreenRect::new(-1920, 0, 1920, 1040),
                scale_factor: 1.25,
                is_primary: false,
            },
        ]);
        coord.register_headless_window(WindowId(1), primary_cfg);

        let (island_cfg, _) =
            crate::FloatingWindowSpec::floating_island("Left Monitor Island", 300, 100).build();
        coord.register_headless_floating_window(WindowId(2), island_cfg);

        // 越界矩形应钳制到负坐标辅屏工作区内。
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .runtime
            .set_window_outer_rect(WindowId(2), ScreenRect::new(-2100, -50, 300, 100));
        assert!(coord.route_window_commands(WindowId(1)).is_empty());
        assert_eq!(
            coord.window_outer_rect(WindowId(2)),
            Some(ScreenRect::new(-1920, 0, 300, 100))
        );

        // 靠近负坐标辅屏左边缘（阈值 24px 内）应自动磁吸至 x = -1920 并记录相对锚点。
        coord
            .registry_mut()
            .get_mut(WindowId(1))
            .unwrap()
            .runtime
            .snap_floating_window(WindowId(2), ScreenRect::new(-1908, 220, 300, 100), 24);
        assert!(coord.route_window_commands(WindowId(1)).is_empty());
        let snapped = coord.window_outer_rect(WindowId(2)).unwrap();
        assert_eq!(snapped.x, -1920);
        assert!(coord.last_snap_anchor(WindowId(2)).is_some());
    }

    #[test]
    fn closing_last_standard_window_cascade_closes_floating_windows_and_never_promotes_ghost() {
        let primary_cfg = WindowConfig::default().with_title("Primary");
        let mut coord =
            MultiWindowCoordinator::new(primary_cfg.clone(), |id: WindowId, _: &WindowConfig| {
                TaggedApp {
                    window_id: id,
                    events: Vec::new(),
                }
            });
        coord.set_headless_capabilities(FloatingWindowCapabilities::win32_full());
        coord.register_headless_window(WindowId(1), primary_cfg.clone());
        let (ghost_cfg, _) = crate::FloatingWindowSpec::ghost("Ghost", 240, 120).build();
        coord.register_headless_floating_window(WindowId(2), ghost_cfg);
        coord.register_headless_window(WindowId(3), primary_cfg);
        let (island_cfg, _) = crate::FloatingWindowSpec::floating_island("Island", 260, 80).build();
        coord.register_headless_floating_window(WindowId(4), island_cfg);

        // 关闭主 Standard 窗口 1 时，绝不将 Ghost(2) 提升为主窗口，而应提升 Standard(3)。
        assert!(coord.close_window(WindowId(1)));
        assert_eq!(coord.registry().primary(), Some(WindowId(3)));

        // 关闭最后一个 Standard 窗口 3 时，应级联关闭剩余全部 Ghost(2) 与 FloatingIsland(4)。
        assert!(coord.close_window(WindowId(3)));
        assert!(coord.registry().is_empty());
        assert_eq!(coord.registry().primary(), None);
        let closed_reqs = coord.take_window_close_requests();
        assert!(closed_reqs.contains(&WindowId(2)));
        assert!(closed_reqs.contains(&WindowId(4)));
    }

    #[test]
    fn floating_capabilities_degrade_deterministically_across_backends() {
        let primary_cfg = WindowConfig::default().with_title("Primary");
        let mut coord =
            MultiWindowCoordinator::new(primary_cfg.clone(), |id: WindowId, _: &WindowConfig| {
                TaggedApp {
                    window_id: id,
                    events: Vec::new(),
                }
            });
        coord.register_headless_window(WindowId(1), primary_cfg);
        let (ghost_cfg, _) = crate::FloatingWindowSpec::ghost("Ghost", 260, 140).build();
        coord.register_headless_floating_window(WindowId(2), ghost_cfg);

        // Win32 全量能力。
        coord.set_headless_capabilities(FloatingWindowCapabilities::win32_full());
        let win32 = coord.floating_policy(WindowId(2)).unwrap();
        assert!(win32.no_activate && win32.always_on_top && win32.skip_taskbar && win32.click_through);

        // X11 默认能力。
        coord.set_headless_capabilities(FloatingWindowCapabilities::x11_default());
        let x11 = coord.floating_policy(WindowId(2)).unwrap();
        assert!(x11.always_on_top && x11.skip_taskbar && x11.click_through);

        // Wayland xdg_toplevel 回退能力：保留 click_through，降级 always_on_top 与 no_activate。
        coord.set_headless_capabilities(FloatingWindowCapabilities::wayland_xdg_fallback());
        let wayland = coord.floating_policy(WindowId(2)).unwrap();
        assert!(!wayland.always_on_top);
        assert!(!wayland.no_activate);
        assert!(wayland.click_through);
    }
}

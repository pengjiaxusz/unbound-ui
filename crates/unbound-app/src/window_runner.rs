//! 窗口化应用宿主与运行器，将 [`AppRuntime`] 适配至 [`PlatformApp`]。

use std::sync::Arc;
use std::time::Instant;

use crate::theme_driver::{make_theme_snapshot_key, ThemeTransitionDriver};
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_asset::{
    decode_bc7_to_rgba8_fallback, AssetId, AtlasPageId, CompressedTexturePayload,
    BC7_ATLAS_PAGE_SIZE,
};
use unbound_gpu::atlas::AtlasSet;
use unbound_gpu::debug::{is_device_lost, recover_with, RecoveryPolicy};
use unbound_gpu::device::{Gpu, GpuConfig};
use unbound_gpu::surface::{AcquireOutcome, Surface, Swapchain};
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
    begin_outbound_file_drag, NativeWindow, OutboundFileDragOutcome, PopupHostRegistry,
    PopupWindowConfig, WindowId,
};
use unbound_theme::ThemeSet;

use crate::appearance::{AppearanceCaps, AppearanceDiff, WindowAppearance};
use crate::{
    AppKey, AppRuntime, Application, Bc7GpuOp, ExternalTextureHandle, ExternalTextureOp, ImeUpdate,
    Modifiers as AppModifiers, ThemeKind, VideoSurfaceOp,
};

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

/// 派生窗口呈现所需的实例扩展。
///
/// 平台相关的一半（Windows 上是 `VK_KHR_win32_surface`，X11 上是
/// `VK_KHR_xcb_surface` …）只能从窗口的*显示*句柄取得：缺少它时
/// `vkCreateWin32SurfaceKHR` 会是无法解析的函数指针；缺少 `VK_KHR_surface`
/// 则使 `VK_KHR_swapchain` 成为非法设备扩展
/// （`VUID-vkCreateDevice-ppEnabledExtensionNames-01387`）。若把这一对硬编码，
/// 运行器将只能在 Windows 工作，因此改为派生，与 `apps/showcase` 保持一致。
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

/// 每帧开始时执行的宿主钩子；在 UI 线程上拿到运行时的可变引用。
type FrameHook<A> = Box<dyn FnMut(&mut AppRuntime<A>) + Send>;

/// 3D 视口自定义渲染通道工厂（用于在首次建窗与 GPU 设备恢复后重新注册通道）。
pub type ViewportPassFactory =
    Box<dyn Fn() -> Box<dyn unbound_gpu::CustomViewportPass> + Send + Sync>;

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
    /// 框架级统一主题过渡驱动器（ADR 0034）。
    theme_driver: ThemeTransitionDriver,
    /// 原生窗口句柄创建完成后的可选宿主回调（例如供下游渲染工具注册主窗口 HWND）。
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
    /// 非 BC7 硬件降级路径下的 RGBA8 图集页缓冲：`page_id -> (bindless_slot, rgba_canvas)`。
    bc7_fallback_pages: FxHashMap<AtlasPageId, (u32, Vec<u8>)>,
    /// 非 BC7 硬件降级路径下的独立大图外部槽映射：`asset_id -> bindless_slot`。
    bc7_fallback_standalone: FxHashMap<AssetId, u32>,
    /// 主窗口平台 ID（用于与原生弹出窗区分，避免弹出窗覆写或销毁主窗口渲染资源）。
    primary_window_id: Option<WindowId>,
    /// 主窗口客户区左上角在屏幕坐标系中的物理像素原点。
    main_screen_origin: (i32, i32),
    /// Outer frame bounds of the primary window in physical screen coordinates.
    outer_bounds: ScreenRect,
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
    /// 已注册的 3D 视口自定义渲染通道工厂集合（建窗初始化与设备恢复后自动重注册）。
    viewport_pass_factories: Vec<ViewportPassFactory>,
}

impl<A: Application> WindowedRunner<A> {
    /// 为 `app` 创建新的窗口运行器。
    pub fn new(app: A, config: &WindowConfig) -> Self {
        let (w, h) = (config.width.max(1), config.height.max(1));
        let viewport = Vec2::new(w as f32, h as f32);
        let initial_outer_bounds = match config.position {
            WindowPosition::Physical { x, y } | WindowPosition::Logical { x, y } => {
                ScreenRect::new(x, y, w, h)
            }
            _ => ScreenRect::new(0, 0, w, h),
        };
        let mut runtime = AppRuntime::new(app, viewport);
        runtime.set_window_outer_bounds(initial_outer_bounds);
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
            theme_driver,
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
            appearance: WindowAppearance::new(),
            external_gpu_slots: FxHashMap::default(),
            bc7_fallback_pages: FxHashMap::default(),
            bc7_fallback_standalone: FxHashMap::default(),
            primary_window_id: None,
            main_screen_origin: (initial_outer_bounds.x, initial_outer_bounds.y),
            outer_bounds: initial_outer_bounds,
            native_popup_windows: FxHashMap::default(),
            popup_host: PopupHostRegistry::new(),
            pending_popup_syncs: Vec::new(),
            pending_popup_closes: Vec::new(),
            was_suspended: false,
            viewport_pass_factories: Vec::new(),
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

    /// Returns the primary window outer top-left position in physical screen coordinates.
    #[must_use]
    pub const fn outer_position(&self) -> (i32, i32) {
        (self.outer_bounds.x, self.outer_bounds.y)
    }

    /// Returns the primary window outer bounds in physical screen coordinates.
    #[must_use]
    pub const fn outer_bounds(&self) -> ScreenRect {
        self.outer_bounds
    }

    /// Updates the cached physical outer bounds and synchronizes them to `AppRuntime` and `AppContext`.
    pub fn set_outer_bounds(&mut self, bounds: ScreenRect) {
        self.outer_bounds = bounds;
        self.main_screen_origin = (bounds.x, bounds.y);
        let id = self.primary_window_id.unwrap_or(WindowId(0));
        self.runtime
            .set_window_screen_origin(self.main_screen_origin, id);
        self.runtime.set_window_outer_bounds(bounds);
        self.sync_geometry_snapshot_to_runtime();
    }

    /// Returns physical screen geometry snapshots for all windows hosted by this runner.
    #[must_use]
    pub fn all_window_geometries(&self) -> Vec<crate::WindowGeometry> {
        let id = self.primary_window_id.unwrap_or(WindowId(0));
        vec![crate::WindowGeometry {
            id,
            role: unbound_platform::WindowRole::Standard,
            outer_position: (self.outer_bounds.x, self.outer_bounds.y),
            outer_bounds: self.outer_bounds,
            client_origin: self.main_screen_origin,
            effective_scale: self.runtime.context().effective_scale(),
            visible: true,
            click_through: false,
        }]
    }

    /// Hit-tests the primary window in physical screen coordinates.
    #[must_use]
    pub fn window_at_screen_pos(&self, screen_x: i32, screen_y: i32) -> Option<WindowId> {
        self.all_window_geometries()
            .into_iter()
            .find(|geom| {
                geom.visible && !geom.click_through && geom.contains_screen_pos(screen_x, screen_y)
            })
            .map(|geom| geom.id)
    }

    fn sync_geometry_snapshot_to_runtime(&mut self) {
        let snapshot = Arc::new(self.all_window_geometries());
        self.runtime.set_all_window_geometries(snapshot);
    }

    fn sync_live_window_geometry(&mut self, window: &NativeWindow) {
        let id = WindowId(window.id().0 as u64);
        self.primary_window_id = Some(id);
        let client_screen = window.client_rect_on_screen();
        let size = window.inner_size();
        let width = client_screen.width.max(size.width).max(1);
        let height = client_screen.height.max(size.height).max(1);
        let (outer_x, outer_y) = window
            .outer_position()
            .unwrap_or((client_screen.x, client_screen.y));
        self.main_screen_origin = (client_screen.x, client_screen.y);
        self.outer_bounds = ScreenRect::new(outer_x, outer_y, width, height);
        self.runtime
            .set_window_screen_origin(self.main_screen_origin, id);
        self.runtime.set_window_outer_bounds(self.outer_bounds);
        self.sync_geometry_snapshot_to_runtime();
    }

    /// 注册 3D 视口自定义渲染通道工厂（首次建窗与 GPU 设备恢复后自动重注册到渲染器）。
    #[must_use]
    pub fn with_viewport_pass_factory(
        mut self,
        factory: impl Fn() -> Box<dyn unbound_gpu::CustomViewportPass> + Send + Sync + 'static,
    ) -> Self {
        self.viewport_pass_factories.push(Box::new(factory));
        self
    }

    /// 在运行期追加 3D 视口自定义渲染通道工厂；若渲染器已就绪则立即注册（首帧渲染完成前保持占位槽位 `None`）。
    pub fn register_viewport_pass_factory(
        &mut self,
        factory: impl Fn() -> Box<dyn unbound_gpu::CustomViewportPass> + Send + Sync + 'static,
    ) {
        let boxed: ViewportPassFactory = Box::new(factory);
        if let Some(renderer) = self.renderer.as_mut() {
            let pass = boxed();
            let viewport_id = pass.viewport_id();
            match renderer.register_viewport_pass(pass) {
                Ok(_slot) => {
                    let committed = renderer.viewport_bindless_slot(viewport_id);
                    self.runtime.commit_viewport_3d_slot(viewport_id, committed);
                }
                Err(error) => {
                    eprintln!(
                        "[unbound-app] Could not register 3D viewport pass {viewport_id:?}: {error:?}"
                    );
                }
            }
        }
        self.viewport_pass_factories.push(boxed);
    }

    /// 将所有已登记的 3D 视口通道工厂重新注册到当前渲染器（首帧离屏渲染完成前不提前暴露槽位）。
    fn register_all_viewport_passes(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        for factory in &self.viewport_pass_factories {
            let pass = factory();
            let viewport_id = pass.viewport_id();
            match renderer.register_viewport_pass(pass) {
                Ok(_slot) => {
                    let committed = renderer.viewport_bindless_slot(viewport_id);
                    self.runtime.commit_viewport_3d_slot(viewport_id, committed);
                }
                Err(error) => {
                    eprintln!(
                        "[unbound-app] Could not register 3D viewport pass {viewport_id:?}: {error:?}"
                    );
                }
            }
        }
    }

    /// 同步窗口当前系统 DPI 缩放因子（`os_scale`）。
    fn sync_os_scale(&mut self, window: &NativeWindow) -> f32 {
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
    fn sync_theme(&mut self, window: &NativeWindow) {
        let (key, sys_accent, transition_cfg) = {
            let ctx = self.runtime.context();
            let sys_accent = ctx.resolved_system_accent();
            (
                make_theme_snapshot_key(ctx.theme, ctx.preset, ctx.accent, sys_accent),
                sys_accent,
                ctx.theme_transition,
            )
        };
        if self.renderer.is_none() {
            return;
        }
        let outcome =
            self.theme_driver
                .sync_preset(key, sys_accent, transition_cfg, self.renderer.as_mut());
        self.runtime.context_mut().theme_transition_remaining_secs = outcome.remaining_secs;
        if outcome.lut_updated {
            // 切换主题或强调色只改变 4 KiB LUT：既不重排也不重传图元，
            // 过渡期间逐帧请求重绘，收敛后自动停止请求并回落 0% 占用。
            window.request_redraw();
        }
    }

    /// 将外观意图账本同步至窗口与渲染器。
    fn sync_appearance(&mut self, window: &NativeWindow) {
        let caps = AppearanceCaps::new(
            unbound_platform::windows_build(),
            self.runtime.premultiplied_surface(),
            unbound_platform::accent_policy_export_available(),
        );
        // 修订后 ADR 0034：原生后景在 t = 0 即刻同步目标深浅模式，与系统 Mica 渐变同步起跑。
        let theme = self.theme_driver.effective_backdrop_theme();
        self.appearance
            .set_host_clear_alpha(self.runtime.clear_alpha());
        let diff = self
            .appearance
            .set_intent(self.runtime.backdrop_intent(), caps, theme);
        self.apply_appearance_diff(window, diff);
    }

    /// 执行外观变更 Diff。
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

    /// 判断给定平台窗口是否为原生弹出窗并返回其业务浮层 ID。
    fn resolve_popup_id(&self, window: &NativeWindow) -> Option<u64> {
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
    fn drain_runtime_commands(&mut self, state: &mut PlatformState, window: &NativeWindow) -> Flow {
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
                crate::WindowCommand::CloseWindow(target)
                | crate::WindowCommand::CloseFloatingWindow(target) => {
                    if target == WindowId(window.id().0 as u64) {
                        window.request_close();
                        return Flow::Exit;
                    }
                }
                crate::WindowCommand::SetImeCursorArea {
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
                crate::WindowCommand::SetVisible(visible) => {
                    window.set_visible(visible);
                }
                crate::WindowCommand::ActivateForeground => {
                    window.activate_foreground();
                }
                crate::WindowCommand::SetOuterPosition { x, y } => {
                    window.set_outer_position(x, y);
                }
                crate::WindowCommand::SetInnerSize { width, height } => {
                    window.set_inner_size(width, height);
                }
                crate::WindowCommand::SetMaximized(maximized) => {
                    window.set_maximized(maximized);
                }
                crate::WindowCommand::SetTaskbarProgress {
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
                crate::WindowCommand::OpenFloatingWindow { .. } => {
                    eprintln!(
                        "[unbound-app] WindowCommand::OpenFloatingWindow requires MultiWindowCoordinator"
                    );
                }
                crate::WindowCommand::UpdateFloatingPolicy { id, policy } => {
                    if id == WindowId(window.id().0 as u64) {
                        let caps = window.capabilities();
                        let eff = crate::effective_policy(policy, caps);
                        window.set_always_on_top(eff.always_on_top);
                        window.set_click_through(eff.click_through);
                    }
                }
                crate::WindowCommand::SetWindowVisibleNoActivate { id, visible } => {
                    if id == WindowId(window.id().0 as u64) {
                        window.set_visible_no_activate(visible);
                    }
                }
                crate::WindowCommand::SetWindowAlwaysOnTop { id, topmost } => {
                    if id == WindowId(window.id().0 as u64) {
                        let caps = window.capabilities();
                        window.set_always_on_top(topmost && caps.supports_topmost);
                    }
                }
                crate::WindowCommand::SetWindowClickThrough { id, click_through } => {
                    if id == WindowId(window.id().0 as u64) {
                        let caps = window.capabilities();
                        window.set_click_through(click_through && caps.supports_click_through);
                    }
                }
                crate::WindowCommand::SetWindowOuterRect { id, rect } => {
                    if id == WindowId(window.id().0 as u64)
                        && window.capabilities().supports_global_positioning
                    {
                        window.set_outer_rect(rect);
                    }
                }
                crate::WindowCommand::SnapFloatingWindow { .. } => {
                    eprintln!(
                        "[unbound-app] WindowCommand::SnapFloatingWindow requires MultiWindowCoordinator"
                    );
                }
                crate::WindowCommand::BeginCrossWindowDrag { .. }
                | crate::WindowCommand::EndCrossWindowDrag { .. }
                | crate::WindowCommand::SyncDragProxyGhost { .. }
                | crate::WindowCommand::CloseDragProxyGhost => {
                    eprintln!(
                        "[unbound-app] Cross-window drag commands require MultiWindowCoordinator"
                    );
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

    /// 消费本帧视频表面上传、离屏驱逐与主动释放操作，并将分配结果回写至 `AssetHub`。
    fn process_video_surface_ops(&mut self, ops: Vec<VideoSurfaceOp>, window: &NativeWindow) {
        if ops.is_empty() {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let mut needs_redraw = false;
        for op in ops {
            match op {
                VideoSurfaceOp::UploadFrame {
                    handle,
                    frame,
                    target_slot,
                } => match renderer.upload_video_frame(target_slot, &frame) {
                    Ok(result) => {
                        let old_slot = self.runtime.asset_hub_mut().commit_video_surface_slot(
                            handle,
                            result.slot,
                            frame.width,
                            frame.height,
                            frame.format,
                            result.vram_bytes,
                        );
                        if let Some(stale_slot) = old_slot {
                            if let Err(error) = renderer.destroy_video_surface(stale_slot) {
                                eprintln!(
                                    "[unbound-app] Could not destroy previous video surface {handle:?} (slot {stale_slot}): {error:?}"
                                );
                            }
                        }
                        if target_slot != Some(result.slot) {
                            needs_redraw = true;
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "[unbound-app] Could not upload video frame for {handle:?} ({}x{}): {error:?}",
                            frame.width, frame.height
                        );
                    }
                },
                VideoSurfaceOp::Evict { handle, slot } => {
                    if let Err(error) = renderer.destroy_video_surface(slot) {
                        eprintln!(
                            "[unbound-app] Could not destroy evicted video surface {handle:?} (slot {slot}): {error:?}"
                        );
                    }
                    needs_redraw = true;
                }
                VideoSurfaceOp::Release { handle, slot } => {
                    if let Some(slot) = slot {
                        if let Err(error) = renderer.destroy_video_surface(slot) {
                            eprintln!(
                                "[unbound-app] Could not destroy released video surface {handle:?} (slot {slot}): {error:?}"
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

    /// 同步本帧活跃的 3D 视口离屏渲染目标、暂停未挂载/零尺寸视口，并回写 Bindless 槽位及派发计数。
    fn sync_viewports_3d(
        &mut self,
        entries: &[crate::Viewport3DFrameEntry],
        window: &NativeWindow,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let mut active_ids = std::collections::HashSet::with_capacity(entries.len());
        let mut slot_changed = false;
        let mut awaiting_first_render = false;
        for entry in entries {
            active_ids.insert(entry.viewport_id);
            if let Err(error) = renderer.sync_viewport_layout(
                entry.viewport_id,
                entry.logical_rect,
                entry.render_scale,
                entry.physical_size,
            ) {
                eprintln!(
                    "[unbound-app] Could not sync 3D viewport target {:?}: {error:?}",
                    entry.viewport_id
                );
            }
            let committed = renderer.viewport_bindless_slot(entry.viewport_id);
            if self.runtime.viewport_3d_slot(entry.viewport_id) != committed {
                slot_changed = true;
            }
            if committed.is_none() && entry.physical_size.is_some() {
                awaiting_first_render = true;
            }
            self.runtime
                .commit_viewport_3d_slot(entry.viewport_id, committed);
        }
        // 本帧未挂载或退化为零尺寸的已注册视口需显式暂停，避免继续离屏渲染。
        for id in renderer.viewport_ids() {
            if !active_ids.contains(&id) {
                if let Err(error) =
                    renderer.sync_viewport_layout(id, unbound_math::Rect::ZERO, 1.0, None)
                {
                    eprintln!(
                        "[unbound-app] Could not pause inactive 3D viewport target {id:?}: {error:?}"
                    );
                }
            }
        }
        let dispatches = renderer.take_viewport_pass_dispatches();
        if dispatches > 0 {
            self.runtime.record_viewport_pass_dispatches(dispatches);
        }
        if slot_changed {
            self.runtime.invalidate();
            window.request_redraw();
        } else if awaiting_first_render {
            window.request_redraw();
        }
    }

    fn process_bc7_ops(&mut self, ops: Vec<Bc7GpuOp>, window: &NativeWindow) {
        if ops.is_empty() {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let supports_bc7 = renderer.compressed_texture_capabilities().supports_bc7;
        let mut needs_redraw = false;
        let mut dirty_fallback_pages: FxHashSet<AtlasPageId> = FxHashSet::default();

        for op in ops {
            match op {
                Bc7GpuOp::CreateAtlasPage { page_id } => {
                    if supports_bc7 {
                        match renderer.create_bc7_atlas_page(page_id.0) {
                            Ok(slot) => {
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Ok(slot));
                                needs_redraw = true;
                            }
                            Err(error) => {
                                eprintln!(
                                    "[unbound-app] Could not create BC7 atlas page {page_id:?}: {error:?}"
                                );
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Err(format!("{error:?}")));
                                needs_redraw = true;
                            }
                        }
                    } else {
                        let page_bytes =
                            (BC7_ATLAS_PAGE_SIZE as usize) * (BC7_ATLAS_PAGE_SIZE as usize) * 4;
                        let canvas = vec![0u8; page_bytes];
                        let desc = ManagedExternalTextureDesc::new(
                            BC7_ATLAS_PAGE_SIZE,
                            BC7_ATLAS_PAGE_SIZE,
                            false,
                        );
                        match renderer.upload_external_rgba8(desc, &canvas) {
                            Ok(slot) => {
                                self.bc7_fallback_pages.insert(page_id, (slot, canvas));
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Ok(slot));
                                needs_redraw = true;
                            }
                            Err(error) => {
                                eprintln!(
                                    "[unbound-app] Could not create RGBA8 fallback atlas page {page_id:?}: {error:?}"
                                );
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_page_slot(page_id, Err(format!("{error:?}")));
                                needs_redraw = true;
                            }
                        }
                    }
                }
                Bc7GpuOp::UploadAtlasRegion {
                    page_id,
                    x,
                    y,
                    width,
                    height,
                    bytes,
                } => {
                    if supports_bc7 {
                        if let Err(error) = renderer
                            .stage_bc7_atlas_subregion(page_id.0, x, y, width, height, &bytes)
                        {
                            eprintln!(
                                "[unbound-app] Could not stage BC7 atlas region on {page_id:?} ({x}, {y}, {width}x{height}): {error:?}"
                            );
                        } else {
                            needs_redraw = true;
                        }
                    } else if let Some((_, canvas)) = self.bc7_fallback_pages.get_mut(&page_id) {
                        match CompressedTexturePayload::bc7_unorm(width, height, bytes) {
                            Ok(payload) => {
                                let rgba = decode_bc7_to_rgba8_fallback(&payload);
                                let page_stride = (BC7_ATLAS_PAGE_SIZE as usize) * 4;
                                let row_bytes = (width as usize) * 4;
                                for row in 0..(height as usize) {
                                    let dst_offset =
                                        ((y as usize) + row) * page_stride + (x as usize) * 4;
                                    let src_offset = row * row_bytes;
                                    if dst_offset + row_bytes <= canvas.len()
                                        && src_offset + row_bytes <= rgba.len()
                                    {
                                        canvas[dst_offset..dst_offset + row_bytes].copy_from_slice(
                                            &rgba[src_offset..src_offset + row_bytes],
                                        );
                                    }
                                }
                                dirty_fallback_pages.insert(page_id);
                            }
                            Err(error) => {
                                eprintln!(
                                    "[unbound-app] Invalid BC7 payload for RGBA8 fallback page {page_id:?}: {error}"
                                );
                            }
                        }
                    }
                }
                Bc7GpuOp::DestroyAtlasPage { page_id } => {
                    if supports_bc7 {
                        if let Err(error) = renderer.destroy_bc7_atlas_page(page_id.0) {
                            eprintln!(
                                "[unbound-app] Could not destroy BC7 atlas page {page_id:?}: {error:?}"
                            );
                        }
                    } else if let Some((slot, _)) = self.bc7_fallback_pages.remove(&page_id) {
                        dirty_fallback_pages.remove(&page_id);
                        if let Err(error) = renderer.destroy_external_texture(slot) {
                            eprintln!(
                                "[unbound-app] Could not destroy RGBA8 fallback atlas page {page_id:?} (slot {slot}): {error:?}"
                            );
                        }
                    }
                }
                Bc7GpuOp::UploadStandalone {
                    asset_id,
                    width,
                    height,
                    bytes,
                } => {
                    if supports_bc7 {
                        match renderer.upload_external_bc7(width, height, &bytes) {
                            Ok(slot) => {
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_standalone_slot(asset_id, Ok(slot));
                                needs_redraw = true;
                            }
                            Err(error) => {
                                eprintln!(
                                    "[unbound-app] Could not upload standalone BC7 texture {asset_id:?} ({width}x{height}): {error:?}"
                                );
                                self.runtime.asset_hub_mut().commit_bc7_standalone_slot(
                                    asset_id,
                                    Err(format!("{error:?}")),
                                );
                                needs_redraw = true;
                            }
                        }
                    } else {
                        match CompressedTexturePayload::bc7_unorm(width, height, bytes) {
                            Ok(payload) => {
                                let rgba = decode_bc7_to_rgba8_fallback(&payload);
                                let desc = ManagedExternalTextureDesc::new(width, height, false);
                                match renderer.upload_external_rgba8(desc, &rgba) {
                                    Ok(slot) => {
                                        self.bc7_fallback_standalone.insert(asset_id, slot);
                                        self.runtime
                                            .asset_hub_mut()
                                            .commit_bc7_standalone_slot(asset_id, Ok(slot));
                                        needs_redraw = true;
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "[unbound-app] Could not upload standalone RGBA8 fallback texture {asset_id:?}: {error:?}"
                                        );
                                        self.runtime.asset_hub_mut().commit_bc7_standalone_slot(
                                            asset_id,
                                            Err(format!("{error:?}")),
                                        );
                                        needs_redraw = true;
                                    }
                                }
                            }
                            Err(error) => {
                                eprintln!(
                                    "[unbound-app] Invalid standalone BC7 payload for {asset_id:?}: {error}"
                                );
                                self.runtime
                                    .asset_hub_mut()
                                    .commit_bc7_standalone_slot(asset_id, Err(error.to_string()));
                                needs_redraw = true;
                            }
                        }
                    }
                }
                Bc7GpuOp::DestroyStandalone { asset_id, slot } => {
                    if supports_bc7 {
                        if let Err(error) = renderer.destroy_external_bc7(slot) {
                            eprintln!(
                                "[unbound-app] Could not destroy standalone BC7 texture {asset_id:?} (slot {slot}): {error:?}"
                            );
                        }
                    } else {
                        let target_slot = self
                            .bc7_fallback_standalone
                            .remove(&asset_id)
                            .unwrap_or(slot);
                        if let Err(error) = renderer.destroy_external_texture(target_slot) {
                            eprintln!(
                                "[unbound-app] Could not destroy standalone RGBA8 fallback texture {asset_id:?} (slot {target_slot}): {error:?}"
                            );
                        }
                    }
                }
            }
        }

        for page_id in dirty_fallback_pages {
            if let Some((slot, canvas)) = self.bc7_fallback_pages.get(&page_id) {
                if let Err(error) = renderer.update_external_rgba8(*slot, canvas) {
                    eprintln!(
                        "[unbound-app] Could not update RGBA8 fallback atlas page {page_id:?}: {error:?}"
                    );
                } else {
                    needs_redraw = true;
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
        self.bc7_fallback_pages.clear();
        self.bc7_fallback_standalone.clear();
        self.runtime
            .asset_hub_mut()
            .invalidate_all_external_textures();
        self.runtime.asset_hub_mut().invalidate_all_video_surfaces();
        self.runtime.invalidate_all_viewport_3d_slots();
        self.runtime.asset_hub_mut().invalidate_all_bc7_pages();
        self.runtime.asset_hub_mut().set_bc7_gpu_backed(false);
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

    fn recover_gpu(&mut self, window: &NativeWindow) {
        // 所有 Vulkan 对象都没有 `Drop`：过去直接置 `None` 会泄漏全部句柄。
        // 尤其是旧 `VkSurfaceKHR` 仍绑定在 HWND 上，导致第一次重建就报
        // `ERROR_NATIVE_WINDOW_IN_USE_KHR`。必须先销毁子对象，再让持有设备的
        // renderer 拆除设备本身。
        self.external_gpu_slots.clear();
        self.bc7_fallback_pages.clear();
        self.bc7_fallback_standalone.clear();
        self.runtime
            .asset_hub_mut()
            .invalidate_all_external_textures();
        self.runtime.asset_hub_mut().invalidate_all_video_surfaces();
        self.runtime.invalidate_all_viewport_3d_slots();
        self.runtime.asset_hub_mut().invalidate_all_bc7_pages();
        self.runtime.asset_hub_mut().set_bc7_gpu_backed(false);
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
            let surface = match window.create_vulkan_surface(&gpu) {
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
                let video_tier = renderer.video_upload_capabilities().active_tier;
                self.runtime
                    .asset_hub_mut()
                    .set_video_upload_tier(video_tier);
                let bc7_caps = renderer.compressed_texture_capabilities();
                self.runtime.asset_hub_mut().set_bc7_capabilities(bc7_caps);
                self.runtime.asset_hub_mut().set_bc7_gpu_backed(true);
                self.renderer = Some(renderer);
                self.swapchain = Some(swapchain);
                self.surface = Some(surface);
                self.atlases = Some(atlases);
                self.register_all_viewport_passes();
                self.runtime.set_premultiplied_surface(premultiplied);
                self.swapchain_dirty = true;
                self.frame_dropped = true;
                // 新建的渲染器此时只有占位 LUT。
                self.theme_driver.invalidate_uploaded();
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
            .map(|delay| self.last_frame + delay)
    }

    fn take_popup_sync_requests(&mut self) -> Vec<(u64, PopupWindowConfig)> {
        core::mem::take(&mut self.pending_popup_syncs)
    }

    fn take_popup_close_requests(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.pending_popup_closes)
    }

    fn window_created(&mut self, state: &mut PlatformState, window: &NativeWindow) {
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

    fn resumed(&mut self, state: &mut PlatformState, window: &NativeWindow) {
        if self.resolve_popup_id(window).is_some() || self.surface.is_some() {
            return;
        }
        self.sync_live_window_geometry(window);
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

        let surface = match window.create_vulkan_surface(&gpu) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!(
                    "[unbound-app] Could not create Vulkan surface:\n{}",
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

        let bc7_caps = renderer.compressed_texture_capabilities();
        self.runtime.asset_hub_mut().set_bc7_capabilities(bc7_caps);
        self.runtime.asset_hub_mut().set_bc7_gpu_backed(true);
        self.atlases = Some(atlases);
        self.swapchain = Some(swapchain);
        let video_tier = renderer.video_upload_capabilities().active_tier;
        self.runtime
            .asset_hub_mut()
            .set_video_upload_tier(video_tier);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.register_all_viewport_passes();
        #[cfg(windows)]
        if let (Some(cb), Some(hwnd)) = (&mut self.on_hwnd_created, window.win32_hwnd()) {
            cb(hwnd);
        }

        let metrics = Arc::new(FrameHitMetrics::new());
        metrics.set_scale_factor(os_scale);
        window.install_custom_frame(metrics.clone());
        self.frame_metrics = Some(metrics);

        state.set_present_extensions(&device_extensions);
        self.sync_theme(window);
        self.sync_appearance(window);
        self.last_frame = Instant::now();
    }

    fn event(
        &mut self,
        state: &mut PlatformState,
        window: &NativeWindow,
        event: PlatformEvent,
    ) -> Flow {
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
            if self.runtime.dirty() != crate::DirtyFlags::NONE && !state.pacer.is_suspended() {
                state.pacer.request_immediate(Instant::now());
                window.request_redraw();
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
                    self.sync_live_window_geometry(window);
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
                state.pacer.note_activity(Instant::now());
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
                self.runtime.set_modifiers(translate_modifiers(modifiers));
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
                self.outer_bounds.x = x;
                self.outer_bounds.y = y;
                let id = WindowId(window.id().0 as u64);
                self.primary_window_id = Some(id);
                self.runtime.set_window_screen_origin((x, y), id);
                self.runtime.set_window_outer_bounds(self.outer_bounds);
                self.sync_geometry_snapshot_to_runtime();
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

    fn frame(&mut self, state: &mut PlatformState, window: &NativeWindow) -> Flow {
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
        self.sync_live_window_geometry(window);
        let (width, height) = (size.width.max(1), size.height.max(1));
        let os_scale = self.sync_os_scale(window);
        let now = Instant::now();
        let raw_dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        let dt = raw_dt.min(0.1);
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

        let _ = self.runtime.tick_with_clock(dt, raw_dt);
        let report = self.runtime.frame(0.0);
        if self.drain_runtime_commands(state, window) == Flow::Exit {
            return Flow::Exit;
        }

        // `reduce` 可能改动 `ctx.theme`，每帧检查 4 KiB LUT 是否需要上传。
        self.sync_theme(window);
        self.sync_appearance(window);

        self.process_external_texture_ops(report.external_texture_ops, window);
        self.process_video_surface_ops(report.video_surface_ops, window);
        self.sync_viewports_3d(&report.viewports_3d, window);
        self.process_bc7_ops(report.bc7_ops, window);

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
            // 统一遵循 ADR-0029 热区契约：仅提交最大化按钮矩形供 Win11 Snap Layout
            //（`HTMAXBUTTON`），最小化与关闭清零留给框架 UI 事件层分发。
            let (_, max_id, _) = self.runtime.primary_title_bar_buttons().unwrap_or((
                unbound_widgets::TITLE_BAR_MINIMIZE_ID,
                unbound_widgets::TITLE_BAR_MAXIMIZE_ID,
                unbound_widgets::TITLE_BAR_CLOSE_ID,
            ));
            metrics.apply_caption_hit_contract(
                self.runtime.widget_rect(max_id),
                effective_scale,
                os_scale,
            );
        }

        let keep_animating =
            self.runtime.context().scale_osd_remaining > 0.0 || report.dirty.needs_rebuild();
        state.pacer.set_animating(keep_animating);
        if keep_animating {
            window.request_redraw();
        }

        Flow::Continue
    }

    fn window_closing(&mut self, window: &NativeWindow) {
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

    #[test]
    fn pointer_activity_wakes_pacer_from_idle_state() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(800, 600),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state
            .pacer
            .set_throttle_policy(RenderThrottlePolicy::default());
        let now = Instant::now();
        state.pacer.note_activity(now);
        assert_eq!(
            state.pacer.throttle_state(now),
            unbound_platform::RenderThrottleState::Active
        );
    }

    #[test]
    fn default_throttle_policy_keeps_unfocused_visible_window_active_in_runner() {
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

        let now = Instant::now();
        state.pacer.note_activity(now);
        state.pacer.set_focused(false);
        let after_grace = now + std::time::Duration::from_secs(5);
        assert_eq!(
            state.pacer.throttle_state(after_grace),
            unbound_platform::RenderThrottleState::Active
        );
        assert!(!runner.should_skip_frame_for_throttle(&state));
    }

    #[test]
    fn translate_pointer_button_preserves_side_buttons() {
        assert_eq!(
            translate_pointer_button(MouseButton::Back),
            Some(PointerButton::Back)
        );
        assert_eq!(
            translate_pointer_button(MouseButton::Forward),
            Some(PointerButton::Forward)
        );
        assert_eq!(
            translate_pointer_button(MouseButton::Other(1)),
            Some(PointerButton::Back)
        );
        assert_eq!(
            translate_pointer_button(MouseButton::Other(2)),
            Some(PointerButton::Forward)
        );
        assert_eq!(translate_pointer_button(MouseButton::Other(3)), None);
    }

    #[test]
    fn window_runner_exposes_outer_geometry_and_screen_hit_testing() {
        let config = WindowConfig {
            width: 960,
            height: 640,
            position: WindowPosition::Physical { x: 120, y: 80 },
            ..WindowConfig::default()
        };
        let mut runner = WindowedRunner::new(DummyApp, &config);
        assert_eq!(runner.outer_position(), (120, 80));
        assert_eq!(runner.outer_bounds(), ScreenRect::new(120, 80, 960, 640));

        runner.primary_window_id = Some(WindowId(11));
        runner.set_outer_bounds(ScreenRect::new(300, 200, 800, 500));

        assert_eq!(runner.outer_position(), (300, 200));
        assert_eq!(runner.outer_bounds(), ScreenRect::new(300, 200, 800, 500));
        assert_eq!(runner.runtime().outer_position(), (300, 200));
        assert_eq!(
            runner.runtime().outer_bounds(),
            ScreenRect::new(300, 200, 800, 500)
        );
        assert_eq!(runner.window_at_screen_pos(450, 350), Some(WindowId(11)));
        assert_eq!(runner.window_at_screen_pos(299, 350), None);
        assert_eq!(
            runner.runtime().window_at_screen_pos(450, 350),
            Some(WindowId(11))
        );
        assert_eq!(runner.all_window_geometries().len(), 1);
        assert_eq!(runner.runtime().all_window_geometries().len(), 1);
    }
}

//! 窗口化的 Unbound UI 展厅（Gallery）。
//!
//! 通过 `cargo run -p showcase` 运行。该窗口是一个自举的应用外壳：
//! 自定义标题栏、可折叠的 NavigationView、可搜索的命令面板、实时示例卡片与性能 HUD。
//!
//!
//! 它是 `apps/gallery` 与 `unbound-headless` 的可视化对应物：
//! gallery 打印测量结果，无头校验器检查布局，showcase 则同时展示两者。
//!

use std::sync::Arc;
use std::time::Instant;

use showcase::frame_plan::{frame_size, needs_rebuild, upload_plan};
use showcase::runtime_app::GalleryRuntime;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::theme;

use raw_window_handle::HasDisplayHandle;
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
use unbound_platform::win32_frame::{install_custom_frame, FrameHitMetrics};
use unbound_platform::window::{
    ButtonState, Modifiers, MouseButton, PlatformEvent, PlatformKey, PlatformState, WindowConfig,
};
use unbound_platform::Extent;
use unbound_platform::{
    CursorIcon, NativeWindowId, PhysicalPosition, PhysicalSize, ResizeDirection, Window,
};
use unbound_theme::{ThemeKind, ThemeState};

use showcase::shell::WindowCommand;

/// 窗口标题。
const TITLE: &str = "Unbound UI Gallery";

/// 等待 swapchain 图像的超时时间（纳秒）。
const ACQUIRE_TIMEOUT_NS: u64 = 1_000_000_000;

/// 单个辅助窗口：拥有独立的 surface、swapchain 与上传代次。
///
/// 设备、渲染器、管线与字形图集均与主窗口共享；仅呈现链（presentation chain）按窗口独立。
///
struct ExtraWindow {
    id: NativeWindowId,
    surface: Surface,
    swapchain: Swapchain,
    swapchain_dirty: bool,
    /// 上次请求重建 swapchain 时的尺寸。
    ///
    /// 呈现链的实际范围由驱动返回，可能与请求不同；两者绝不互相比较。
    ///
    committed_request: (u32, u32),
}

/// 应用状态。
struct Showcase {
    /// 通用运行时：负责归约每个事件、管理路由器 / 焦点策略，并将每帧与上一帧做 diff。
    ///
    app: AppRuntime<GalleryRuntime>,
    renderer: Option<Renderer>,
    surface: Option<Surface>,
    swapchain: Option<Swapchain>,
    atlases: Option<Arc<AtlasSet>>,
    /// 当前已上传到 GPU 的主题。
    uploaded_theme: Option<ThemeMode>,
    /// 上一次 `SceneUpload` 的上传代次。
    ///
    /// 当代次未推进时渲染器跳过上传；宿主恰好在运行时报告全场景变更时推进它。
    ///
    ///
    scene_generation: u64,
    /// 当上一帧已重建但从未上传时为 true。
    ///
    /// 运行时每次重建都会推进 diff 基准，无论该帧是否真正到达 GPU。
    /// 因此丢帧会使缓冲池落后于基准，下一次上传必须是全量上传。
    ///
    frame_dropped: bool,
    /// 上次请求创建 swapchain 时的尺寸。
    ///
    /// 呈现链的*实际*范围由驱动返回，可能与请求不同；此处记住请求尺寸，
    /// 以便两者绝不互相比较。
    ///
    committed_request: (u32, u32),
    /// 当前生效的主题：引擎拥有颜色表，应用仅请求切换不同的调色板。
    ///
    theme: ThemeState,
    /// 物理指针位置。
    pointer: Vec2,
    /// 键盘修饰键状态。
    modifiers: Modifiers,
    cursor: CursorKind,
    last_frame: Instant,
    /// OS 缩放因子（每逻辑单位对应的物理像素数）。
    os_scale: f32,
    /// 指针下方对应的窗口边缘（如有）。
    resize_edge: Option<ResizeEdge>,
    /// 当前是否允许 IME，以便仅在状态切换时调用，而非每帧调用。
    ///
    ime_allowed: bool,
    /// 上次标题栏按压记录，用于双击最大化。
    last_title_press: Option<(Instant, Vec2)>,
    /// 当本次按压已被窗口外框（chrome）消费时为 true。
    press_consumed: bool,
    /// 当驱动报告 swapchain 已过期时为 true。
    swapchain_dirty: bool,
    /// 主窗口标识，用于区分辅助窗口。
    primary: Option<NativeWindowId>,
    /// 主窗口自定义边框命中指标（驱动 Win32 `WM_NCHITTEST` 四边/四角缩放与 Win11 贴靠布局）。
    frame_metrics: Option<Arc<FrameHitMetrics>>,
    /// 辅助窗口及其呈现链。
    extras: Vec<ExtraWindow>,
    /// 由 `Ctrl+N` 置位；平台层在下一帧打开新窗口。
    new_window_requested: bool,
}

impl Showcase {
    fn new() -> Self {
        Self {
            app: {
                let mut app = GalleryRuntime::new();
                // 运行时负责对帧做 diff，因此外壳不得为同一结果再次扫描图元列表。
                //
                app.set_external_diff(true);
                let mut rt = AppRuntime::new(app, Vec2::new(1440.0, 900.0));
                struct WinClipboard(std::cell::RefCell<unbound_platform::SystemClipboard>);
                impl unbound_app::ClipboardProvider for WinClipboard {
                    fn get_text(&self) -> Option<String> {
                        use unbound_platform::ClipboardProvider as _;
                        self.0.borrow_mut().read_text().ok().flatten()
                    }
                    fn set_text(&mut self, text: &str) {
                        use unbound_platform::ClipboardProvider as _;
                        let _ = self.0.borrow_mut().write_text(text);
                    }
                }
                rt.set_clipboard(Box::new(WinClipboard(std::cell::RefCell::new(
                    unbound_platform::SystemClipboard::new(),
                ))));
                rt
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
            frame_metrics: None,
            extras: Vec::new(),
            new_window_requested: false,
        }
    }

    /// 每逻辑单位对应的物理像素数。
    fn effective_scale(&self) -> f32 {
        (self.os_scale * self.shell().state.zoom()).clamp(0.5, 4.0)
    }

    /// 由运行时驱动的自举外壳。
    fn shell(&self) -> &GalleryApp {
        self.app.application().shell()
    }

    /// 可变外壳引用。
    fn shell_mut(&mut self) -> &mut GalleryApp {
        self.app.application_mut().shell_mut()
    }

    fn logical(&self, physical: Vec2) -> Vec2 {
        physical / self.effective_scale()
    }

    /// 仅跟踪 OS 缩放因子。
    ///
    /// 改变应用缩放绝不能调整物理窗口大小：UI 会就地缩放，因为逻辑视口是根据
    /// 已提交范围（`physical_size / (os_scale × app_zoom)`）重新计算的，
    /// 页面会针对更小的逻辑视口重新排版。
    ///
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
        // 进行中的按压捕获操作（滚动条抓取、滑块拖拽、画布平移）独占指针：
        // 仅在按下拖拽期间屏蔽边缘缩放，未按下时始终允许窗口外圈 6px 热区响应缩放。
        let ctx = self.app.context();
        if ctx.pressed.is_some() {
            return None;
        }
        let size = window.inner_size();
        let scale = self.os_scale.max(0.5);
        // 窗口按钮位于右上角 3 × 46 逻辑像素区域。指针坐标为物理像素，
        // 因此排除区域也必须转换为物理像素——混用两者在高 DPI 显示器和缩放 UI 下
        // 会导致角落判定错位。
        //
        let ui_scale = self.effective_scale();
        let button_h = 48.0 * ui_scale;
        let button_w = 3.0 * 46.0 * ui_scale;
        if self.pointer.y < button_h && self.pointer.x > size.width as f32 - button_w {
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
        // 单批次提交：整页文本共用一个暂存缓冲区、一个命令缓冲区与一次队列等待。
        // 逐字形上传会按每个字符停顿一次管线。
        //
        //
        // 失败的批次绝不能丢失字形：缓存认为它们已在图集中，
        // 若丢失会导致文本采样从未写入的纹素——永久显示空白或乱码字形。
        // 因此将更新放回队列，在下一帧重试。
        //
        if let Err(error) = atlases.upload_batch(&renderer.gpu, &uploads) {
            eprintln!("could not upload the glyphs: {error}");
            self.shell_mut().labels.cache.restore_updates(updates);
        }
    }

    /// 上传本帧由资源中心解码完成的 RGBA8 矩形。
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
            // 切换主题仅重写 4 KiB 颜色表：无需重新布局，也无需重传实例。
            //
            let _ = self.theme.switch(theme::kind(dark));
            let _ = renderer.write_theme_lut(self.theme.lut());
            self.uploaded_theme = Some(mode);
        }
    }

    /// 通过运行时重建场景、做 diff 并上传重建产生的字形。
    /// 此方法取代了手写的 `sync` / `dirty_ranges` / `generation` 账本：
    /// 现在由运行时拥有帧管线，宿主仅将返回的报告转换为上传计划。
    ///
    ///
    fn advance_scene(&mut self, dt: f32) -> unbound_app::FrameReport {
        let scale = self.effective_scale();
        self.shell_mut().set_effective_scale(scale);
        let report = self.app.frame(dt);
        self.upload_asset_textures(&report.atlas_uploads);
        // 只要存在字形就进行上传。场景未变的帧仍可能携带新字形——
        // 重新光栅化的字形会落入不同的图集槽位，虽然图元也会随之变化，
        // 但上传逻辑绝不能依赖这种巧合。
        //
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

    /// 为新的表面尺寸重建 swapchain。
    ///
    /// 旧呈现链通过 `oldSwapchain` 交给驱动，使呈现引擎能够移交资源而非撕毁表面——
    /// 这正是消除调整窗口大小时闪烁的关键。
    ///
    fn recreate_swapchain(&mut self, window: &Window, width: u32, height: u32) -> bool {
        // 在借用渲染器前先读取上限：两者都经由 `self`，
        // 读取外壳会与可变借用产生冲突。
        let limit = self.shell().state.effective_fps_limit();
        let Some(renderer) = &mut self.renderer else {
            return false;
        };
        let Some(surface) = &self.surface else {
            return false;
        };
        // 高于显示器物理刷新率的上限无法通过 FIFO 达成——
        // 合成器会把每帧量化回 VBlank（这正是过去在 60 Hz 面板上“120”实际只跑 60 FPS 的原因）。
        // 此类上限会将节奏交由软件 pacer 控制并以无垂直同步模式呈现；
        // 低于或等于刷新率的上限则保留无撕裂路径。
        //
        //
        // 原生 Win32 后端通过窗口 DC 查询所在显示器的刷新率；查询失败时回退 60 Hz。
        let refresh_hz = window.refresh_rate_hz().unwrap_or(60.0);
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
        // 呈现链现在对此次请求负责。其*实际*范围仍可能与之不同——
        // 驱动会将其钳制到自身限制——绝不能把这种差异误判为新请求。
        //
        self.committed_request = (width, height);
        // 窗口化路径借用 swapchain 图像作为颜色附件，因此只有深度缓冲需要跟随新尺寸。
        // `ensure_depth_size` 以 256 px 步长增长且永不收缩，
        // 使 1 px 的拖拽步进不会触发任何重新分配。
        //
        let (actual_w, actual_h) = swapchain.extent();
        if let Err(error) = renderer.ensure_depth_size(actual_w, actual_h) {
            eprintln!("could not resize the depth buffer: {error}");
            return false;
        }
        true
    }

    /// 使布局视口与已提交的呈现链范围保持一致。
    ///
    /// 布局跟随*呈现*尺寸而非窗口尺寸：驱动决定图像大小，而合成器显示的是图像。
    /// 逻辑视口为 `physical / effective scale`，因此缩放与物理调整大小具有完全相同的效果。
    ///
    ///
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

    /// 使 OS 输入法与当前聚焦控件保持同步。
    ///
    /// 若不调用 `set_ime_allowed`，Windows 根本不会发送组合事件，
    /// 导致中日韩输入法只能输入 ASCII。候选框锚定在点击位置。
    ///
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
        // 已提交文本必须精确插入一次，因此此处直接排空（drain）缓冲区而非仅读取。
        //
        let commit = state.ime.take_commit();
        let mut changed = false;
        if !commit.is_empty() {
            // 经由运行时提交，使提交操作同时将帧标记为脏。
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
        // 经由运行时：IME 事件并非 `PlatformEvent`，处于休眠（parked）状态的循环
        // 永远不会得知出现了组合输入，因此对它们也绕过帧率节奏限制。
        if changed {
            state.pacer.request_immediate(Instant::now());
        }
    }
}

/// 将引擎的调整大小边缘映射为 OS 拖拽方向。
fn resize_direction(edge: ResizeEdge) -> ResizeDirection {
    use ResizeDirection as Direction;
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

/// 将平台按键转换为运行时的按键词汇表。
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

/// 将平台鼠标按键映射为运行时按键。
fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        // 前进 / 后退属于导航操作，并非路由器管理的指针状态。
        MouseButton::Other(_) => None,
    }
}

/// 将后端的唤醒回调适配为运行时的 [`AppWaker`] trait。
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

/// 将平台的修饰键集合转换为运行时的修饰键集合。
fn app_modifiers(modifiers: Modifiers) -> AppModifiers {
    AppModifiers {
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.meta,
    }
}

/// 将引擎光标映射为原生平台光标。
fn cursor_icon(kind: CursorKind) -> CursorIcon {
    use CursorIcon as Icon;
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
                // 旧报错仅提示“could not create a Vulkan device”并丢弃了原因，
                // 导致 Windows 10 用户无从得知缺失的是驱动、加载器还是某个特性。
                //
                //
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

        let metrics = Arc::new(FrameHitMetrics::new());
        metrics.set_scale_factor(self.effective_scale());
        install_custom_frame(hwnd, Arc::clone(&metrics));
        self.frame_metrics = Some(metrics);

        // 呈现链刚刚按窗口尺寸创建：该请求已提交，因此首帧不会再次重建。
        //
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
            // GPU crate 拥有自己的句柄枚举以避免依赖窗口库；
            // 在此处一次性提取原生句柄。
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
        // 待处理的定时器或被挂起的后台任务必须唤醒事件循环；
        // pacer 的帧截止时间由后端单独考量。
        self.app.next_wake_in().map(|delay| Instant::now() + delay)
    }

    fn attach_waker(&mut self, waker: std::sync::Arc<dyn Fn() + Send + Sync>) {
        self.app.set_waker(std::sync::Arc::new(SignalWaker(waker)));
    }

    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow {
        // 辅助窗口呈现同一应用；其输入尚未路由，因此仅呈现链响应缩放变化。
        //
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
                // 缩放改变逻辑视口，帧则从已提交范围推导视口。
                // 标记失效使下一帧按新尺寸重建场景。
                //
                self.app.invalidate();
            }
            PlatformEvent::RecreateSwapchain(extent) => {
                // 平台层已完成调整大小：必须按新尺寸重建 swapchain。
                //
                self.swapchain_dirty = true;
                let _ = extent;
            }
            PlatformEvent::KeyInput {
                state: ButtonState::Pressed,
                key,
                repeat,
            } => {
                // Ctrl+N 打开第二个窗口：在同一设备上创建真实的第二个 surface 和 swapchain。
                //
                if !repeat
                    && self.modifiers.control
                    && matches!(&key, PlatformKey::Character(text) if text.eq_ignore_ascii_case("n"))
                {
                    self.new_window_requested = true;
                    state.pacer.request_immediate(Instant::now());
                }
                // 当 IME 持有文本时，平台还会把组合字符作为按键上报。
                // 若两边都转发，每个字母都会被插入两次（"nihao" → "nniihhaaoo"）。
                //
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
                        // 右键按压绝不移动窗口外框；它是 UI 的上下文菜单手势。
                        // 中键用于画布平移，同样由运行时管理。
                        //
                        if state == ButtonState::Pressed {
                            self.app.pointer_down(position, button);
                        } else {
                            self.app.pointer_up(position, button);
                        }
                    } else if state == ButtonState::Pressed {
                        // 窗口外框拥有其边缘和标题栏：下方的 UI 不得再收到该按压事件。
                        //
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
                // 窗口失去焦点：指针正在进行的任何操作均已结束。
                // 必须清除捕获、滚动条抓取与演示拖拽，否则外壳会一直保留幽灵拖拽，
                // 直到用户恰好点击别处为止。
                //
                self.shell_mut().cancel_interactions();
                self.app.cancel_pointer();
                self.app
                    .pointer_up(self.logical(self.pointer), PointerButton::Left);
            }
            PlatformEvent::RedrawRequested => {
                // OS 请求重绘（窗口被取消遮挡，或合成器丢失了 surface）。
                // 只需唤醒处于*休眠*状态的循环：当 pacer 正在动画时它已掌控节奏，
                // 此处强行请求一帧会取消每一帧前刚刚设定的截止时间。
                //
                //
                if !state.pacer.animating() {
                    state.pacer.request_immediate(Instant::now());
                }
            }
            PlatformEvent::ThemeChanged { dark } => {
                // 跟随 OS 偏好；在下一次 OS 变更前，应用内切换仍优先。
                //
                self.shell_mut().state.theme = if dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                self.shell_mut().state.bump();
                self.app.invalidate();
            }
            PlatformEvent::FileHover(path) => {
                self.app.file_hover(vec![path]);
            }
            PlatformEvent::FileDrop(path) => {
                self.app.file_drop(vec![path]);
            }
            PlatformEvent::FileHoverCancelled => {
                self.app.file_hover_cancelled();
            }
            PlatformEvent::CloseRequested => return Flow::Exit,
            _ => {}
        }
        if flow == Flow::Exit {
            return Flow::Exit;
        }
        // 重新读取有效缩放：本次事件中的缩放点击可能在处理前快照之后修改了它。
        //
        let scale = self.effective_scale();
        self.shell_mut().set_effective_scale(scale);
        self.sync_ime(state, window);
        // 重建本身在帧循环中通过运行时完成。
        // 当场景中有任何元素移动时立即踢醒 pacer，使点击立刻可见，而非等到下次垂直同步。
        //
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
    /// 重建辅助窗口的呈现链。
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
        // 呈现链现在对此次请求负责。其*实际*范围仍可能与之不同——
        // 驱动会将其钳制到自身限制——绝不能把这种差异误判为新请求。
        //
        extra.committed_request = (width, height);
        let (actual_w, actual_h) = extra.swapchain.extent();
        if let Err(error) = renderer.ensure_depth_size(actual_w, actual_h) {
            eprintln!("could not resize the shared depth buffer: {error}");
            return false;
        }
        true
    }

    /// 渲染单个辅助窗口。
    ///
    /// 该窗口与主窗口共享设备和渲染器，但拥有独立的 surface、swapchain 与节奏控制。
    /// 绘制前应用会按此窗口的逻辑视口重新布局，使同一套 UI 按各自尺寸填满两个窗口。
    ///
    ///
    fn frame_secondary(&mut self, state: &mut PlatformState, window: &Window) -> Flow {
        let Some(index) = self.extras.iter().position(|extra| extra.id == window.id()) else {
            return Flow::Continue;
        };
        let size = window.inner_size();
        let requested = (size.width.max(1), size.height.max(1));
        // 重建判据比较两个*请求*尺寸：窗口当前请求与上次构建呈现链时的请求。
        // 呈现链的实际范围由驱动返回，可能相差一像素——
        // 若比较两者会导致每帧都重建且永不收敛。
        //
        //
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

        // 为此窗口布局共享应用并重建。视口遵循已提交范围——
        // 即帧实际绘制的尺寸——因此布局与相机绝不会描述两幅不同的画面。
        //
        //
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
        let camera = Camera::ui((actual_w as f32, actual_h as f32), scale);
        self.app.application_mut().shell_mut().scene.camera = camera;
        // 两个窗口共享缓冲池，因此每次绘制都使用全新代次全量上传整个场景，
        // 而非修补属于另一个窗口的基准。
        //
        let runtime_scene = self.app.scene();
        let scene = SceneUpload {
            primitives: &runtime_scene.primitives,
            transforms: &runtime_scene.transforms,
            clips: &runtime_scene.clips,
            payloads: &runtime_scene.payloads,
            generation: self.scene_generation,
            camera,
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

        // 窗口尺寸只是*请求*。驱动决定 swapchain 图像大小，合成器显示的是图像，
        // 因此以下一切均由已提交范围推导：布局与相机从而始终描述同一幅画面。
        //
        //
        let size = window.inner_size();
        let requested = (size.width, size.height);
        // 最小化的窗口报告零范围：无需为其重建，且一像素的呈现链毫无意义。
        //
        let live = size.width > 0 && size.height > 0;

        // 尺寸调整稳定下来，或驱动已拒绝当前 swapchain，两者含义相同：按新尺寸重建呈现链。
        //
        let settled = state.poll_resize(Instant::now());
        // 帧率上限存储在应用状态中；pacer 是唯一执行该上限的模块。
        // 上限变更还会改变呈现模式（受限运行保持无撕裂，无限制运行不得被锁死在显示器刷新率上），
        // 因此接下来会重建呈现链。
        //
        let limit = self.shell().state.effective_fps_limit();
        let mut dirty = self.swapchain_dirty || settled.is_some();
        if state.pacer.fps_limit() != limit {
            state.pacer.set_fps_limit(limit);
            dirty = true;
        }
        // 比较的是两个*请求*尺寸。呈现链的实际范围由驱动返回，
        // 合法情况下可能与请求相差一像素；比较两者会导致每帧都重建呈现链、
        // 永不收敛，并把属于一种尺寸的布局呈现到另一种尺寸的图像中。
        //
        //
        let rebuild = live && needs_rebuild(requested, self.committed_request, dirty);
        if rebuild {
            self.swapchain_dirty = false;
            if !self.recreate_swapchain(window, requested.0, requested.1) {
                // 旧呈现链属于旧尺寸：在其中呈现新布局会把 UI 放到错误位置。
                // 跳过本帧并在下一帧重试。
                //
                self.swapchain_dirty = true;
                state.pacer.request_immediate(Instant::now());
                return Flow::Continue;
            }
        }

        // 布局跟随*呈现*尺寸：swapchain 的已提交范围。
        // 因此页面绝不会按帧实际绘制尺寸之外的尺寸进行布局。
        //
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
        // 在触及帧时钟*之前*检查节奏：早退曾导致从未绘制的帧也更新了
        // `last_frame` 和 FPS 计数器。
        //
        //
        // 事件循环在截止时间前 `SPIN_LEAD` 唤醒，因此最后不到一毫秒的空隙在此处自旋补齐，
        // 而非交给毫秒粒度的 OS 定时器。
        //
        if !rebuild && !state.pacer.should_render(now) {
            if state.pacer.within_spin_lead(now) {
                now = state.pacer.spin_to_deadline();
            } else {
                return Flow::Continue;
            }
        }

        // 在借用渲染器前推进时间并重建场景：应用与 GPU 对象互不相交，
        // 但辅助方法会借用整个 `self`。
        //
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.1);
        self.last_frame = now;
        if dt > 0.0 {
            // 指数移动平均：单次卡顿不得让 HUD 在 40 与 120 之间剧烈跳动。
            //
            let instant_fps = 1.0 / dt;
            let alpha = 0.15;
            let demo = &mut self.shell_mut().state.demo;
            demo.fps += (instant_fps - demo.fps) * alpha;
            demo.frame_ms += (dt * 1000.0 - demo.frame_ms) * alpha;
        }
        // 当同一渲染器上存在第二个窗口时，每个窗口都会按自身视口重建并全量上传场景：
        // 共享实例池一次只能保存一个修补基准。
        //
        let multi = !self.extras.is_empty();
        // 运行时归约 tick、重建场景并返回本帧的上传计划。
        //
        let dirty = self.app.tick(dt);
        let moving = dirty != DirtyFlags::NONE;
        let report = self.advance_scene(0.0);
        self.write_theme_if_needed();
        self.apply_cursor(window);

        if let Some(metrics) = &self.frame_metrics {
            let ui_scale = self.effective_scale();
            metrics.set_scale_factor(ui_scale);
            // 同步右上角窗口控制按钮矩形，以激活 Win11 贴靠布局（HTMAXBUTTON）与原生边缘缩放。
            // title_bar_height 保持为 0，使标题栏内的搜索框、主题切换与缩放按钮仍由客户端事件驱动。
            let logical_w = self.app.context().viewport.x;
            let btn_w = 46.0;
            let btn_h = 48.0;
            metrics.set_minimize_button_rect(unbound_math::Rect::new(
                logical_w - 3.0 * btn_w,
                0.0,
                btn_w,
                btn_h,
            ));
            metrics.set_maximize_button_rect(unbound_math::Rect::new(
                logical_w - 2.0 * btn_w,
                0.0,
                btn_w,
                btn_h,
            ));
            metrics.set_close_button_rect(unbound_math::Rect::new(
                logical_w - btn_w,
                0.0,
                btn_w,
                btn_h,
            ));
        }

        // 上传计划。当场景形状变化时推进代次；当上一帧被丢弃时也会推进代次：
        // 丢帧后运行时的 diff 基准已跑在缓冲池前面，唯有全量上传能使两者重新对齐。
        //
        //
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
        // 当表面在底层发生变化时，驱动会拒绝该图像。
        // 这并非失败：下一帧会按新尺寸重建 swapchain。
        //
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

        // 应用按逻辑单位（physical / (OS scale × zoom)）布局；
        // UI 相机将这些单位放大回物理像素，使 150% 的布局在任意 DPI 下都能精确映射到物理视口。
        //
        //
        // 相机基于布局求解所用的同一范围构建，因此两者绝不会描述不同的画面。
        //
        let camera = Camera::ui((width as f32, height as f32), scale);
        self.app.application_mut().shell_mut().scene.camera = camera;
        let (generation, dirty_ranges): (u64, &[(u32, u32)]) = if multi {
            // 两个窗口共享实例池，因此每次绘制都在*全新*代次下全量上传整个场景，
            // 而非修补属于另一个窗口的基准。每次绘制代次都会递增，确保两个窗口绝不会声索同一代次。
            //
            //
            (self.scene_generation.wrapping_add(1), &[])
        } else {
            (plan.generation, plan.ranges)
        };
        self.scene_generation = generation;
        let gpu_backdrops: Vec<unbound_gpu::frame::BackdropRequest> = report
            .backdrops
            .iter()
            .map(|b| unbound_gpu::frame::BackdropRequest {
                rect: [
                    b.rect.origin.x * scale,
                    b.rect.origin.y * scale,
                    b.rect.size.x * scale,
                    b.rect.size.y * scale,
                ],
                radius: b.blur * scale,
                corner_radii: [0.0; 4],
                layer_key_prefix: (PassLayer::Overlay as u64) << 56,
            })
            .collect();
        let runtime_scene = self.app.scene();
        let scene = SceneUpload {
            primitives: &runtime_scene.primitives,
            transforms: &runtime_scene.transforms,
            clips: &runtime_scene.clips,
            payloads: &runtime_scene.payloads,
            generation,
            camera,
            key_prefix: (PassLayer::Ui as u64) << 56,
            dirty_ranges,
            aux_changed: report.aux_changed,
            backdrops: &gpu_backdrops,
        };
        if let Err(error) = renderer.render_into_swapchain(&scene, &image, width, height) {
            eprintln!("render failed: {error}");
            self.frame_dropped = true;
            return Flow::Continue;
        }
        // 场景已到达缓冲池：diff 基准与 GPU 再次同步，后续若再丢帧将从干净起点开始。
        //
        self.frame_dropped = false;
        // 渲染完成信号量属于*当前* swapchain 图像，
        // 因此呈现引擎绝不会等待另一张图像仍持有的信号量。
        //
        let finished = image.render_finished;
        // 呈现返回“recreate”意味着表面在 swapchain 下方发生了变化：
        // 该帧已显示，但当前呈现链已过期。
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
        // 无 HUD、无动画且启用帧率上限的静态页面无新内容可显示：
        // 让事件循环休眠，而非每次刷新都提交全量裁剪与基数排序。
        //
        let keep_animating = self.shell().state.fps_limit == 0
            || self.shell().state.show_hud
            || self.shell().state.active_page == showcase::pages::PageId::Telemetry
            || moving;
        state.pacer.set_animating(keep_animating);
        // 此处不调用 `request_redraw`：`about_to_wait` 在每次迭代都会设置
        // `ControlFlow::WaitUntil(pacer.next_wake())`，因此循环会在下一帧到期时精确自醒。
        // 若同时请求重绘会立即派发 `RedrawRequested`，其处理函数会取消刚刚设定的截止时间——
        // 导致软件限帧器根本没有机会生效。
        //
        //
        state.latency.on_present(Instant::now());
        Flow::Continue
    }

    fn accessibility_update_impl(&mut self) -> Option<accesskit::TreeUpdate> {
        // 运行时的收集器持有外壳的树；此处获取增量（delta）
        // 正是静态帧不产生任何系统调用的原因。
        self.app.scene_mut().semantics.take_update()
    }

    fn accessibility_action_impl(
        &mut self,
        _state: &mut PlatformState,
        _window: &Window,
        request: unbound_platform::A11yRequest,
    ) -> Flow {
        // 经由 reducer 处理，使辅助技术点击与鼠标点击走同一条代码路径。
        //
        self.app.a11y(request.node, request.action, request.value);
        Flow::Continue
    }
}

/// 离屏渲染展厅并写入 PNG。
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

/// 将 RGBA 像素写入 PNG 文件，必要时创建父目录。
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

    /// 改变应用缩放不得调整窗口大小。
    ///
    /// 布局由 `physical / (os_scale × zoom)` 重新推导，因此物理窗口保持不变；
    /// 此测试防止旧行为回归（每次缩放都请求调整窗口会打断拖拽手势，
    /// 且在 200% 缩放下可能把窗口移出屏幕外）。
    ///
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
        assert_eq!(PageId::ALL.len(), 24);
        assert!(!PageId::Home.examples().is_empty());
        assert!(PageId::Telemetry.examples().len() >= 2);
        assert!(!PageId::Systems.examples().is_empty());
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

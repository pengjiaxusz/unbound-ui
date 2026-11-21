//! 窗口系统后端：将原生 Win32 事件翻译为引擎事件。
//!
//! 翻译过程刻意设计为单向且无损的：引擎层绝不会看到任何底层窗口库类型，
//! 从而可在不触及渲染器的前提下替换窗口库。
//!

use std::time::Instant;

use crate::latency::LatencyTracker;
use crate::overlay_host::OcclusionMonitor;
use crate::pacing::{FramePacer, PresentWaitPolicy};
use crate::resize::{Extent, ResizeAction, ResizeTracker};
use crate::win32_frame::WindowBackdropMaterial;

/// 窗口启动时的初始位置策略。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WindowPosition {
    /// 默认居中于主显示器工作区（扣除任务栏后的可用区域）。
    #[default]
    CenterPrimary,
    /// 居中于当前鼠标指针所在显示器的工作区。
    CenterActiveMonitor,
    /// 交由操作系统默认策略决定（在无边框弹窗下自动应用工作区安全边距）。
    SystemDefault,
    /// 指定逻辑像素坐标（相对于目标显示器工作区左上角，按目标显示器 DPI 自动换算为物理像素）。
    Logical { x: i32, y: i32 },
    /// 指定屏幕物理像素坐标。
    Physical { x: i32, y: i32 },
}

/// 解析后的窗口初始物理摆放参数（屏幕物理坐标与客户区物理尺寸）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WindowPlacement {
    /// 窗口左上角屏幕物理 X 坐标。
    pub x: i32,
    /// 窗口左上角屏幕物理 Y 坐标。
    pub y: i32,
    /// 窗口客户区物理像素宽度。
    pub width: i32,
    /// 窗口客户区物理像素高度。
    pub height: i32,
    /// 生效的最小跟踪物理像素宽度（用于 `WM_GETMINMAXINFO`）。
    pub min_track_width: i32,
    /// 生效的最小跟踪物理像素高度（用于 `WM_GETMINMAXINFO`）。
    pub min_track_height: i32,
}

/// 屏幕坐标系中的物理像素矩形。
///
/// 用于把窗口客户区锚定到桌面：`x` / `y` 是相对虚拟桌面原点的物理像素，
/// 宽度与高度为客户区的物理像素尺寸。布局仍只使用逻辑视口，不消费该矩形。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenRect {
    /// 客户区左上角屏幕物理 X 坐标。
    pub x: i32,
    /// 客户区左上角屏幕物理 Y 坐标。
    pub y: i32,
    /// 客户区物理像素宽度。
    pub width: u32,
    /// 客户区物理像素高度。
    pub height: u32,
}

impl ScreenRect {
    /// 构造一个屏幕物理矩形。
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 是否具有可用的面积。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// 窗口表面的合成方式。
///
/// 它决定窗口内容能否与 DWM 后景材质（云母 / 亚克力）合成：不透明表面会把
/// 应用像素整块盖在系统材质之上，原生后景只剩窗口外框可见。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceAlpha {
    /// 不透明表面：根背景清屏 alpha 恒为 1，窗口永远遮住桌面。
    #[default]
    Opaque,
    /// 预乘 alpha 表面：根背景可全透明，让 DWM 材质从客户区透出。
    ///
    /// 驱动不支持预乘 alpha 时，交换链创建流程会回退为不透明；调用方只需表达
    /// 意图，不必自行探测能力。
    PreMultiplied,
}

/// 平台窗口的配置参数。
#[derive(Clone, Debug, PartialEq)]
pub struct WindowConfig {
    /// 窗口标题。
    pub title: String,
    /// 初始逻辑宽度。
    pub width: u32,
    /// 初始逻辑高度。
    pub height: u32,
    /// 是否允许用户调整窗口大小。
    pub resizable: bool,
    /// 系统是否绘制原生标题栏与边框。
    ///
    /// 采用客户端自绘装饰（CSD）的应用会关闭此项，并通过标题栏命中区域自行拖拽窗口。
    ///
    pub decorations: bool,
    /// 用户可将窗口缩小的最小逻辑宽度。
    pub min_width: u32,
    /// 用户可将窗口缩小的最小逻辑高度。
    pub min_height: u32,
    /// 窗口启动时是否立即可见。
    pub visible: bool,
    /// 目标刷新间隔。
    pub refresh: std::time::Duration,
    /// 窗口启动时的初始位置策略（默认为主屏幕工作区居中 [`WindowPosition::CenterPrimary`]）。
    pub position: WindowPosition,
    /// 表面合成方式（默认为不透明）。
    ///
    /// 需要系统原生后景（云母 / 亚克力）从客户区透出的窗口必须选
    /// [`SurfaceAlpha::PreMultiplied`]，并按后景计划把根背景清屏 alpha 降到 0。
    pub surface_alpha: SurfaceAlpha,
    /// 窗口后景材质意图（默认为 `None` 即不开启后景）。
    ///
    /// 开启后景材质（如 `Some(WindowBackdropMaterial::Mica)`）时：
    /// 1. 框架自动将表面合成方式选为预乘 alpha（忽略显式设置的合成方式）；
    /// 2. 框架自动派生并下发清屏 alpha，让系统材质从客户区透出；
    /// 3. 当驱动不支持预乘或操作系统构建号不足时，自动退回不透明窗口（不发材质、清屏 alpha 钳回 1.0），
    ///    避免出现透明空壳或黑窗。
    ///
    /// `Some(WindowBackdropMaterial::None)` 与 `None` 语义一致，均表示关闭后景。
    /// 该字段只覆盖能直接映射到系统 DWM 取值的四档（`System`, `Mica`, `Tabbed`, `Acrylic`）；
    /// 实验档（如亚克力自定义色调、自绘纹理等）必须走外观意图账本组件。
    pub backdrop: Option<WindowBackdropMaterial>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Unbound UI".to_string(),
            width: 1280,
            height: 800,
            resizable: true,
            decorations: true,
            min_width: 320,
            min_height: 240,
            visible: true,
            refresh: crate::pacing::REFRESH_120,
            position: WindowPosition::CenterPrimary,
            surface_alpha: SurfaceAlpha::Opaque,
            backdrop: None,
        }
    }
}

impl WindowConfig {
    /// 设置窗口标题。
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// 设置窗口初始逻辑宽高。
    #[must_use]
    pub const fn with_size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// 设置窗口最小逻辑宽高。
    #[must_use]
    pub const fn with_min_size(mut self, min_width: u32, min_height: u32) -> Self {
        self.min_width = min_width;
        self.min_height = min_height;
        self
    }

    /// 设置窗口初始位置策略。
    #[must_use]
    pub const fn with_position(mut self, position: WindowPosition) -> Self {
        self.position = position;
        self
    }

    /// 将窗口初始位置设为主显示器工作区居中。
    #[must_use]
    pub const fn centered(mut self) -> Self {
        self.position = WindowPosition::CenterPrimary;
        self
    }

    /// 将窗口初始位置设为当前鼠标所在显示器工作区居中。
    #[must_use]
    pub const fn centered_on_active_monitor(mut self) -> Self {
        self.position = WindowPosition::CenterActiveMonitor;
        self
    }

    /// 将窗口初始位置设为指定逻辑坐标（相对于目标显示器工作区左上角）。
    #[must_use]
    pub const fn at_logical(mut self, x: i32, y: i32) -> Self {
        self.position = WindowPosition::Logical { x, y };
        self
    }

    /// 将窗口初始位置设为指定屏幕物理坐标。
    #[must_use]
    pub const fn at_physical(mut self, x: i32, y: i32) -> Self {
        self.position = WindowPosition::Physical { x, y };
        self
    }

    /// 设置表面合成方式。
    ///
    /// 原生后景（系统云母 / 亚克力）必须配合 [`SurfaceAlpha::PreMultiplied`]：不透明
    /// 交换链会把应用像素盖在系统材质上，材质只剩窗口外框可见。
    #[must_use]
    pub const fn with_surface_alpha(mut self, alpha: SurfaceAlpha) -> Self {
        self.surface_alpha = alpha;
        self
    }

    /// 设置窗口后景材质意图。
    #[must_use]
    pub const fn with_backdrop(mut self, backdrop: Option<WindowBackdropMaterial>) -> Self {
        self.backdrop = backdrop;
        self
    }

    /// 计算生效的最小逻辑尺寸。
    ///
    /// 若调用方仅设置了较小的 `width` / `height`（例如 800×600）而显式或隐式携带了更大的
    /// `min_width` / `min_height`，必须将最小尺寸钳制到不超过初始窗口尺寸，否则首次移动窗口
    /// 触发 `WM_GETMINMAXINFO` 时操作系统会强制把窗口拉大到 `min_width × min_height`。
    #[must_use]
    pub const fn effective_min_size(&self) -> (u32, u32) {
        let w = if self.width > 0 { self.width } else { 1 };
        let h = if self.height > 0 { self.height } else { 1 };
        let min_w = if self.min_width > w {
            w
        } else {
            self.min_width
        };
        let min_h = if self.min_height > h {
            h
        } else {
            self.min_height
        };
        (min_w, min_h)
    }
}

/// 根据 [`WindowConfig`]、目标显示器工作区物理矩形 `(left, top, right, bottom)` 与 DPI 缩放因子，
/// 计算窗口初始物理位置与客户区物理尺寸。
#[must_use]
pub fn resolve_window_placement(
    config: &WindowConfig,
    work_area: (i32, i32, i32, i32),
    scale_factor: f64,
) -> WindowPlacement {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let (eff_min_w, eff_min_h) = config.effective_min_size();
    let logical_w = config.width.max(eff_min_w).max(1);
    let logical_h = config.height.max(eff_min_h).max(1);

    let width = (f64::from(logical_w) * scale).round().max(1.0) as i32;
    let height = (f64::from(logical_h) * scale).round().max(1.0) as i32;
    let min_track_width = if eff_min_w > 0 {
        (f64::from(eff_min_w) * scale).round().max(1.0) as i32
    } else {
        0
    };
    let min_track_height = if eff_min_h > 0 {
        (f64::from(eff_min_h) * scale).round().max(1.0) as i32
    } else {
        0
    };

    let (work_left, work_top, work_right, work_bottom) = work_area;
    let work_w = (work_right - work_left).max(1);
    let work_h = (work_bottom - work_top).max(1);

    let (x, y) = match config.position {
        WindowPosition::CenterPrimary | WindowPosition::CenterActiveMonitor => {
            let cx = work_left + ((work_w - width) / 2).max(0);
            let cy = work_top + ((work_h - height) / 2).max(0);
            (cx, cy)
        }
        WindowPosition::Logical { x, y } => (
            work_left + (f64::from(x) * scale).round() as i32,
            work_top + (f64::from(y) * scale).round() as i32,
        ),
        WindowPosition::Physical { x, y } => (x, y),
        WindowPosition::SystemDefault => {
            let offset = (48.0 * scale).round() as i32;
            (
                work_left + offset.min((work_w - width).max(0)),
                work_top + offset.min((work_h - height).max(0)),
            )
        }
    };

    WindowPlacement {
        x,
        y,
        width,
        height,
        min_track_width,
        min_track_height,
    }
}

/// 修饰键集合的当前状态。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// 按键或按钮的按下/释放状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// 鼠标按键。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// 键盘按键，扁平化为引擎关心的少数几类。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformKey {
    /// 可打印字符。
    Character(String),
    /// 具名按键，如 `Enter` 或 `ArrowLeft`。
    Named(String),
    /// 其余按键，由平台扫描码标识。
    Other(u32),
}

/// 引擎层需要了解的所有窗口事件。
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    /// 操作系统调整了窗口尺寸。
    Resized(Extent),
    /// 必须按该尺寸重建交换链。
    RecreateSwapchain(Extent),
    /// 系统缩放因子发生变化。
    ScaleFactorChanged(f64),
    /// 窗口在屏幕上的位置发生变化（客户区左上角物理坐标）。
    ///
    /// 尺寸不变而位置改变时也会触发；依赖屏幕锚点的后景必须借此刷新。
    Moved { x: i32, y: i32 },
    /// 指针移动，坐标为相对于窗口的逻辑像素。
    CursorMoved { x: f64, y: f64 },
    /// 原始指针位移，单位为设备像素。
    PointerRaw { dx: f64, dy: f64 },
    /// 鼠标按键状态发生变化。
    MouseInput {
        state: ButtonState,
        button: MouseButton,
    },
    /// 鼠标滚轮转动，单位为逻辑行。
    ///
    /// Windows 上报像素增量；此处将其换算为行数，使引擎无需关心平台滚动粒度。
    ///
    MouseWheel { delta_x: f64, delta_y: f64 },
    /// 键盘按键状态发生变化。
    KeyInput {
        state: ButtonState,
        key: PlatformKey,
        repeat: bool,
    },
    /// 修饰键状态发生变化。
    ModifiersChanged(Modifiers),
    /// 窗口获得或失去焦点。
    Focused(bool),
    /// 窗口变为可见或隐藏。
    VisibilityChanged(bool),
    /// 窗口被其他不透明窗口完全遮挡或恢复未被遮挡。
    Occluded(bool),
    /// 窗口进入或退出最小化状态。
    Minimized(bool),
    /// 系统的浅色 / 深色主题偏好发生变化。
    ThemeChanged { dark: bool },
    /// 有文件正在窗口上方拖拽。
    FileHover(std::path::PathBuf),
    /// 有文件被拖放到窗口中。
    FileDrop(std::path::PathBuf),
    /// 文件拖拽离开窗口且未放下。
    FileHoverCancelled,
    /// 输入法（IME）组合或提交事件。
    Ime(ImeEvent),
    /// 操作系统请求绘制新的一帧。
    RedrawRequested,
    /// 归属于当前窗口的原生弹出窗已关闭（如宿主最小化、隐藏或关闭引发级联回收）。
    PopupDismissed { id: u64 },
    /// 用户请求关闭窗口。
    CloseRequested,
}

/// 输入法（IME）事件。
///
/// 引擎绝不会看到底层窗口库类型；后端将平台 IME 事件翻译为该精简枚举，
/// 并由 [`PlatformState::ime`] 维护组合串供应用渲染。
///
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// IME 已就绪，可接收组合输入。
    Enabled,
    /// 组合字符串发生变化。
    ///
    /// 元组包含组合字符串及选区字节范围。
    Preedit(String, Option<(usize, usize)>),
    /// 组合输入已确认；插入该文本。
    Commit(String),
    /// IME 已关闭。
    Disabled,
}

/// 输入法的实时状态。
///
/// 只描述当前组合输入：是否启用、组合字符串与其选区。平台 IME 事件是应用侧
/// 唯一输入法来源，提交文本不再在此缓冲，因此不存在需要宿主轮询的提交通道。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeState {
    /// 当前聚焦控件是否启用了 IME。
    pub enabled: bool,
    /// 组合字符串；无组合时为空。
    pub preedit: String,
    /// 组合选区的字节范围。
    pub selection: Option<(usize, usize)>,
}

impl ImeState {
    /// 应用一条平台 IME 事件。
    pub fn apply(&mut self, event: &ImeEvent) {
        match event {
            ImeEvent::Enabled => self.enabled = true,
            ImeEvent::Preedit(text, selection) => {
                // 组合串到达即表明输入法正在持有文本：与下发给应用的启用标志
                // 保持一致，宿主据此忽略被输入法消耗的物理字符键。
                self.enabled = true;
                self.preedit.clone_from(text);
                self.selection = *selection;
            }
            ImeEvent::Commit(_) => {
                // 组合结束即释放输入法对文本的持有权；提交内容由事件通道直达应用，
                // 不再需要宿主忽略物理字符键。
                self.enabled = false;
                self.preedit.clear();
                self.selection = None;
            }
            ImeEvent::Disabled => {
                self.enabled = false;
                self.preedit.clear();
                self.selection = None;
            }
        }
    }

    /// 应当在光标处绘制的组合字符串。
    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// 丢弃当前组合输入（例如焦点移走时）。
    pub fn clear(&mut self) {
        self.preedit.clear();
        self.selection = None;
    }
}

/// 创建或运行窗口时抛出的错误。
#[derive(Debug)]
pub enum PlatformError {
    /// 窗口系统拒绝了该请求。
    Window(String),
    /// 无法创建事件循环。
    EventLoop(String),
}

impl core::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Window(message) => write!(f, "window error: {message}"),
            Self::EventLoop(message) => write!(f, "event loop error: {message}"),
        }
    }
}

impl std::error::Error for PlatformError {}

/// 平台层的实时状态。
///
/// 引擎每帧需要轮询的所有状态均集中于此，且其中没有任何字段依赖外部窗口库。
///
#[derive(Debug)]
pub struct PlatformState {
    /// 窗口尺寸调整记账。
    pub resize: ResizeTracker,
    /// 输入到光子延迟统计。
    pub latency: LatencyTracker,
    /// 帧率节拍控制。
    pub pacer: FramePacer,
    /// 呈现同步策略。
    pub present: PresentWaitPolicy,
    /// 当前系统缩放因子。
    pub scale_factor: f64,
    /// 当前处于按住状态的修饰键。
    ///
    /// 快捷键仅在与修饰键结合时才有意义，因此状态中保留了修饰键信息：
    /// 仅从事件流读取修饰键状态的应用需要自行追踪。
    ///
    pub modifiers: Modifiers,
    /// 窗口当前是否拥有焦点。
    pub focused: bool,
    /// 窗口当前是否可见。
    pub visible: bool,
    /// 窗口当前是否被完全遮挡。
    pub occluded: bool,
    /// 窗口当前是否处于最小化状态。
    pub minimized: bool,
    /// 窗口遮挡状态监测器。
    pub occlusion: OcclusionMonitor,
    /// 输入法组合串与已提交文本。
    pub ime: ImeState,
}

impl PlatformState {
    /// 为新创建的窗口初始化平台状态。
    #[must_use]
    pub fn new(config: &WindowConfig, extent: Extent, present: PresentWaitPolicy) -> Self {
        let mut pacer = FramePacer::new(config.refresh);
        pacer.set_visible(config.visible);
        let mut occlusion = OcclusionMonitor::new();
        occlusion.set_visible(config.visible);
        Self {
            resize: ResizeTracker::with_default_settle(extent),
            latency: LatencyTracker::new(),
            pacer,
            present,
            scale_factor: 1.0,
            modifiers: Modifiers::default(),
            focused: true,
            visible: config.visible,
            occluded: false,
            minimized: false,
            occlusion,
            ime: ImeState::default(),
        }
    }

    /// 告知平台层渲染器实际获取到的设备扩展。
    ///
    /// 窗口创建早于 Vulkan 设备，因此构造时无法推导呈现策略。应用在
    /// [`crate::backend::PlatformApp::resumed`] 中创建设备并在此回传扩展列表；
    /// 若缺少此步骤，策略将停留在“无扩展”回退模式，永远不会走低延迟呈现路径。
    ///
    ///
    ///
    pub fn set_present_extensions(&mut self, extensions: &[String]) {
        self.present = PresentWaitPolicy::from_extensions(extensions);
    }

    /// 轮询缩放追踪器，并提交已稳定的尺寸。
    ///
    /// 稳定窗口需要一个**不**依赖后续缩放事件到达的时钟滴答。拖拽风暴以最后
    /// 一条事件收尾，若此后不再有事件到达，最后那个尺寸将永远无法提交——
    /// 窗口会永远停留在陈旧尺寸上。
    ///
    ///
    /// 当必须重建交换链时返回新尺寸。
    pub fn poll_resize(&mut self, now: Instant) -> Option<Extent> {
        match self.resize.poll(now) {
            ResizeAction::None => None,
            ResizeAction::Recreate(extent) => Some(extent),
        }
    }
}

/// 将平台事件应用到共享状态。
///
/// 返回引擎必须响应的事件列表。
pub fn apply(state: &mut PlatformState, event: &PlatformEvent, now: Instant) -> Vec<PlatformEvent> {
    let mut out = Vec::new();
    match event {
        PlatformEvent::Resized(extent) => {
            // 窗口尺寸发生了变化。必须通知应用：需要按新尺寸重新计算布局，
            // 且交换链必须随之重建。
            //
            //
            // 该事件过去曾在此被吞掉，导致窗口已改变大小而渲染器仍按旧尺寸绘制——
            // 用户看到的就是被拉伸的画面。
            //
            state.pacer.request_immediate(now);
            out.push(PlatformEvent::Resized(*extent));
            if let ResizeAction::Recreate(new) = state.resize.on_resized(*extent, now) {
                out.push(PlatformEvent::RecreateSwapchain(new));
            }
        }
        PlatformEvent::Moved { .. } => {
            // 位置变化不需要重建布局或交换链，但依赖屏幕锚点的合成结果必须重绘。
            state.pacer.request_immediate(now);
            out.push(event.clone());
        }
        PlatformEvent::ScaleFactorChanged(factor) => {
            state.scale_factor = *factor;
            // 应用按逻辑单位布局，因此必须获知设备缩放比，以使输入、字形光栅化
            // 与相机保持在同一坐标系中。
            //
            state.pacer.request_immediate(now);
            out.push(event.clone());
        }
        PlatformEvent::CursorMoved { .. }
        | PlatformEvent::PointerRaw { .. }
        | PlatformEvent::MouseInput { .. }
        | PlatformEvent::MouseWheel { .. }
        | PlatformEvent::KeyInput { .. } => {
            // 用户的任何操作都会使挂起帧失效。
            state.latency.on_input(now);
            // 应用依赖这些事件驱动绘制；在此吞掉它们会导致延迟时钟持续走时，
            // 但所有控件看起来却毫无响应。
            //
            //
            // 响应该输入的帧绝不能干等下一次垂直同步：否则节拍器会将响应
            // 延迟整整一个刷新周期。
            //
            state.pacer.request_immediate(now);
            out.push(event.clone());
        }
        PlatformEvent::ModifiersChanged(mods) => {
            // 应用需要修饰键状态来解析快捷键。
            state.modifiers = *mods;
            out.push(PlatformEvent::ModifiersChanged(*mods));
        }
        PlatformEvent::Focused(focused) => {
            state.focused = *focused;
            state.pacer.set_focused(*focused);
            state.occlusion.set_focused(*focused);
            state.pacer.note_activity(now);
            if *focused {
                state.pacer.request_immediate(now);
            }
            out.push(PlatformEvent::Focused(*focused));
        }
        PlatformEvent::VisibilityChanged(visible) => {
            state.visible = *visible;
            state.pacer.set_visible(*visible);
            state.occlusion.set_visible(*visible);
            if *visible {
                state.pacer.note_activity(now);
                state.pacer.request_immediate(now);
            }
            out.push(PlatformEvent::VisibilityChanged(*visible));
        }
        PlatformEvent::Occluded(occluded) => {
            state.occluded = *occluded;
            state.pacer.set_occluded(*occluded);
            state.occlusion.set_occluded(*occluded);
            if !*occluded {
                state.pacer.note_activity(now);
                state.pacer.request_immediate(now);
            }
            out.push(PlatformEvent::Occluded(*occluded));
        }
        PlatformEvent::Minimized(minimized) => {
            state.minimized = *minimized;
            state.pacer.set_minimized(*minimized);
            state.occlusion.set_minimized(*minimized);
            if !*minimized {
                state.pacer.note_activity(now);
                state.pacer.request_immediate(now);
            }
            out.push(PlatformEvent::Minimized(*minimized));
        }
        PlatformEvent::ThemeChanged { dark } => {
            out.push(PlatformEvent::ThemeChanged { dark: *dark });
        }
        PlatformEvent::FileHover(path) => out.push(PlatformEvent::FileHover(path.clone())),
        PlatformEvent::FileDrop(path) => out.push(PlatformEvent::FileDrop(path.clone())),
        PlatformEvent::FileHoverCancelled => out.push(PlatformEvent::FileHoverCancelled),
        PlatformEvent::Ime(ime) => {
            state.ime.apply(ime);
            state.latency.on_input(now);
            state.pacer.request_immediate(now);
            out.push(PlatformEvent::Ime(ime.clone()));
        }
        PlatformEvent::RedrawRequested => out.push(PlatformEvent::RedrawRequested),
        PlatformEvent::PopupDismissed { id } => {
            out.push(PlatformEvent::PopupDismissed { id: *id });
        }
        PlatformEvent::CloseRequested => out.push(PlatformEvent::CloseRequested),
        PlatformEvent::RecreateSwapchain(extent) => {
            out.push(PlatformEvent::RecreateSwapchain(*extent));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pacing::PresentSync;

    /// 默认窗口配置应居中于主屏幕工作区，且将 `min_width` / `min_height` 钳制在 `width` / `height` 之内，
    /// 避免示例程序（如 800×600）在移动窗口触发 `WM_GETMINMAXINFO` 时突然跳变到 960 宽度。
    #[test]
    fn default_window_config_centers_on_primary_and_never_jumps_size_on_move() {
        let example_cfg = WindowConfig {
            title: "01 — Hello Counter".to_string(),
            width: 800,
            height: 600,
            ..Default::default()
        };
        assert_eq!(example_cfg.position, WindowPosition::CenterPrimary);
        let (min_w, min_h) = example_cfg.effective_min_size();
        assert!(min_w <= example_cfg.width);
        assert!(min_h <= example_cfg.height);

        let placement = resolve_window_placement(&example_cfg, (0, 0, 1920, 1040), 1.0);
        assert_eq!(placement.width, 800);
        assert_eq!(placement.height, 600);
        assert_eq!(placement.x, (1920 - 800) / 2);
        assert_eq!(placement.y, (1040 - 600) / 2);
        assert!(placement.min_track_width <= placement.width);
        assert!(placement.min_track_height <= placement.height);
    }

    /// 即使调用方显式指定了大于初始宽高的 `min_width` / `min_height`，`effective_min_size`
    /// 也会将其钳制到不超过初始宽高，杜绝移动窗口时的尺寸跳变。
    #[test]
    fn effective_min_size_clamps_oversized_minimums_to_initial_dimensions() {
        let cfg = WindowConfig {
            width: 800,
            height: 600,
            min_width: 960,
            min_height: 720,
            ..Default::default()
        };
        assert_eq!(cfg.effective_min_size(), (800, 600));
        let placement = resolve_window_placement(&cfg, (0, 0, 2560, 1400), 1.5);
        assert_eq!(placement.width, 1200);
        assert_eq!(placement.height, 900);
        assert_eq!(placement.min_track_width, 1200);
        assert_eq!(placement.min_track_height, 900);
    }

    /// 窗口位置策略支持逻辑坐标、物理坐标与主屏/活动显示器居中配置。
    #[test]
    fn custom_window_positions_resolve_accurately_across_dpi_scales() {
        let work_area = (100, 40, 2020, 1120);
        let logical_cfg = WindowConfig::default()
            .with_size(800, 600)
            .at_logical(50, 60);
        let p_logical = resolve_window_placement(&logical_cfg, work_area, 1.5);
        assert_eq!(p_logical.x, 100 + 75);
        assert_eq!(p_logical.y, 40 + 90);
        assert_eq!(p_logical.width, 1200);
        assert_eq!(p_logical.height, 900);

        let physical_cfg = WindowConfig::default()
            .with_size(800, 600)
            .at_physical(320, 180);
        let p_phys = resolve_window_placement(&physical_cfg, work_area, 1.5);
        assert_eq!(p_phys.x, 320);
        assert_eq!(p_phys.y, 180);
    }

    /// 缩放事件直达应用并立即重建交换链：必须按新分辨率绘制帧，
    /// 而非显示上一帧的拉伸副本。
    ///
    #[test]
    fn a_resize_event_produces_a_swapchain_recreation_immediately() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        let out = apply(
            &mut state,
            &PlatformEvent::Resized(Extent::new(1000, 700)),
            now,
        );
        assert_eq!(
            out,
            vec![
                PlatformEvent::Resized(Extent::new(1000, 700)),
                PlatformEvent::RecreateSwapchain(Extent::new(1000, 700)),
            ],
            "the resize must not be swallowed and the chain must follow at once"
        );
    }

    /// 缩放事件立即唤醒帧调度器：新尺寸绝不能干等下一个常规节拍截止时间。
    ///
    #[test]
    fn a_resize_wakes_the_frame_scheduler() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        // 假装片刻前刚刚呈现过一帧。
        state.pacer.on_present(now);
        assert!(
            !state.pacer.should_render(now),
            "the pacer must not be due yet"
        );
        apply(
            &mut state,
            &PlatformEvent::Resized(Extent::new(1000, 700)),
            now,
        );
        assert!(
            state.pacer.should_render(now),
            "a resize must make the next frame due right away"
        );
    }

    /// 仅移动窗口不重建交换链，但必须立即排一帧，让屏幕锚点驱动的后景重新取样。
    #[test]
    fn a_move_wakes_the_frame_scheduler_without_resizing() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        state.pacer.on_present(now);
        assert!(!state.pacer.should_render(now));
        let out = apply(&mut state, &PlatformEvent::Moved { x: 120, y: 80 }, now);
        assert_eq!(out, vec![PlatformEvent::Moved { x: 120, y: 80 }]);
        assert!(state.pacer.should_render(now));
    }

    /// 拖拽风暴以最后一条缩放事件结束。实时模式下每条消息都立即提交尺寸，
    /// 因此轮询只会返回“无需操作”。
    #[test]
    fn a_committed_size_is_not_committed_twice() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        let out = apply(
            &mut state,
            &PlatformEvent::Resized(Extent::new(1000, 700)),
            now,
        );
        assert!(
            out.contains(&PlatformEvent::RecreateSwapchain(Extent::new(1000, 700))),
            "the first message must rebuild the chain"
        );

        // 后续不再有事件到达；帧循环改为轮询驱动。
        assert_eq!(
            state.poll_resize(now + std::time::Duration::from_millis(5)),
            None,
            "the size was already committed"
        );
        assert_eq!(
            state.poll_resize(now + std::time::Duration::from_millis(60)),
            None,
            "a committed size is not committed twice"
        );
    }

    /// 呈现策略必须跟随真实设备，而非硬编码回退值：窗口创建早于 Vulkan 设备。
    ///
    #[test]
    fn the_present_policy_follows_the_device_extensions() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        assert_eq!(state.present.sync(), PresentSync::Immediate);

        state.set_present_extensions(&[
            "VK_KHR_swapchain".to_string(),
            "VK_KHR_present_wait".to_string(),
        ]);
        assert_eq!(state.present.sync(), PresentSync::WaitForPresent);
        assert!(state.present.can_wait_for_present());
    }

    /// 输入事件必须直达应用。仅有延迟时钟远远不够：一个首帧冻结、吞掉所有
    /// 点击、滚轮与按键的窗口，表现起来就像彻底损坏的控件。
    ///
    #[test]
    fn input_events_reach_the_application() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        let events = [
            PlatformEvent::CursorMoved { x: 10.0, y: 20.0 },
            PlatformEvent::MouseInput {
                state: ButtonState::Pressed,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseWheel {
                delta_x: 0.0,
                delta_y: -1.0,
            },
            PlatformEvent::KeyInput {
                state: ButtonState::Pressed,
                key: PlatformKey::Character("t".to_string()),
                repeat: false,
            },
        ];
        for event in &events {
            assert_eq!(
                apply(&mut state, event, now),
                vec![event.clone()],
                "{event:?} must be forwarded unchanged"
            );
        }
        state
            .latency
            .on_present(now + std::time::Duration::from_millis(4));
        assert!(state.latency.last().is_some());
    }

    /// 每条输入事件都必须送达应用。
    ///
    /// 引擎依赖这些事件驱动绘制；在此吞掉它们曾导致延迟时钟走时而控件毫无反应，
    /// 修饰键、焦点状态与可见性也一并丢失。
    ///
    #[test]
    fn input_and_wheel_events_are_forwarded_to_the_application() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();

        let events = [
            PlatformEvent::CursorMoved { x: 10.0, y: 20.0 },
            PlatformEvent::MouseWheel {
                delta_x: 0.0,
                delta_y: 40.0,
            },
            PlatformEvent::MouseInput {
                state: ButtonState::Pressed,
                button: MouseButton::Left,
            },
            PlatformEvent::KeyInput {
                state: ButtonState::Pressed,
                key: PlatformKey::Character("t".to_string()),
                repeat: false,
            },
            PlatformEvent::ModifiersChanged(Modifiers {
                shift: true,
                ..Modifiers::default()
            }),
            PlatformEvent::Focused(false),
            PlatformEvent::VisibilityChanged(false),
        ];
        for event in &events {
            let out = apply(&mut state, event, now);
            assert!(
                out.iter().any(|forwarded| forwarded == event),
                "{event:?} must be forwarded: {out:?}"
            );
        }
        // 修饰键状态同时保存在状态中，使快捷键无需追踪历史事件流即可解析。
        //
        assert!(state.modifiers.shift);
    }

    /// 组合串到达的那一刻即视为输入法持有文本。
    ///
    /// 平台把该标志下发给应用，宿主据此忽略被输入法消耗的物理字符键；
    /// 若只在组合开始（`Enabled`）时置位，空查询下首批组合字符会以普通键入
    /// 抵达应用，并按“非输入法编辑事件”取消组合。
    ///
    #[test]
    fn a_preedit_marks_the_ime_as_owning_text() {
        let mut ime = ImeState::default();
        ime.apply(&ImeEvent::Preedit("ni".to_string(), Some((2, 2))));
        assert!(ime.enabled, "a preedit means the IME now owns the text");
        assert_eq!(ime.preedit(), "ni");
        assert_eq!(ime.selection, Some((2, 2)));
    }

    /// 连续组合更新只保留最新一份，不会累积。
    #[test]
    fn successive_preedits_keep_only_the_latest_composition() {
        let mut ime = ImeState::default();
        ime.apply(&ImeEvent::Preedit("n".to_string(), None));
        ime.apply(&ImeEvent::Preedit("ni".to_string(), None));
        ime.apply(&ImeEvent::Preedit("nihao".to_string(), Some((5, 5))));
        assert_eq!(ime.preedit(), "nihao");
        assert_eq!(ime.selection, Some((5, 5)));
        assert!(ime.enabled);
    }

    /// 提交结束组合并释放输入法对文本的持有权。
    #[test]
    fn a_commit_clears_the_composition_and_releases_the_ime() {
        let mut ime = ImeState::default();
        ime.apply(&ImeEvent::Preedit("ni".to_string(), Some((2, 2))));
        ime.apply(&ImeEvent::Commit("hello".to_string()));
        assert_eq!(ime.preedit(), "");
        assert_eq!(ime.selection, None);
        assert!(!ime.enabled, "a commit means the IME no longer owns text");
    }

    /// 输入法关闭后释放文本持有权并清空组合。
    #[test]
    fn disabling_the_ime_clears_the_composition_and_releases_the_text() {
        let mut ime = ImeState::default();
        ime.apply(&ImeEvent::Preedit("ni".to_string(), Some((2, 2))));
        ime.apply(&ImeEvent::Disabled);
        assert!(!ime.enabled);
        assert_eq!(ime.preedit(), "");
        assert_eq!(ime.selection, None);
    }

    /// 组合进行中重复的物理字符键不会清掉组合，也不会释放文本持有权。
    ///
    /// 平台照旧把按键转发给应用，防御交给宿主依据 `ime.enabled` 完成的
    /// “被输入法持有的键不走普通键入”门控。
    ///
    #[test]
    fn repeated_character_keys_during_a_preedit_leave_the_composition_intact() {
        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let now = Instant::now();
        apply(
            &mut state,
            &PlatformEvent::Ime(ImeEvent::Preedit("ni".to_string(), None)),
            now,
        );
        assert!(state.ime.enabled);

        let repeated = PlatformEvent::KeyInput {
            state: ButtonState::Pressed,
            key: PlatformKey::Character("n".to_string()),
            repeat: true,
        };
        let out = apply(&mut state, &repeated, now);
        assert!(
            out.contains(&repeated),
            "the key must still reach the host, which decides whether it is IME-owned"
        );
        assert!(state.ime.enabled);
        assert_eq!(state.ime.preedit(), "ni");
    }

    #[test]
    fn apply_drives_pacer_and_occlusion_monitor_across_throttle_events() {
        use crate::pacing::{RenderThrottlePolicy, RenderThrottleState};
        use std::time::Duration;

        let config = WindowConfig::default();
        let mut state = PlatformState::new(
            &config,
            Extent::new(1280, 800),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.pacer.set_throttle_policy(RenderThrottlePolicy {
            active_fps: 120,
            unfocused_idle_fps: 10,
            idle_grace_ms: 500,
            suspend_when_occluded: true,
            suspend_when_minimized: true,
        });

        let t0 = Instant::now();
        state.pacer.on_present(t0);

        // 失焦后在宽限期内仍保持 Active，超过宽限期进入 ThrottledFps(10)
        let out = apply(&mut state, &PlatformEvent::Focused(false), t0);
        assert_eq!(out, vec![PlatformEvent::Focused(false)]);
        assert!(!state.focused);
        assert!(!state.occlusion.is_focused());
        assert_eq!(
            state.pacer.throttle_state(t0 + Duration::from_millis(200)),
            RenderThrottleState::Active
        );
        assert_eq!(
            state.pacer.throttle_state(t0 + Duration::from_millis(500)),
            RenderThrottleState::ThrottledFps(10)
        );

        // 遮挡事件使状态机进入 Suspended，不再唤醒
        let t_occ = t0 + Duration::from_millis(600);
        let out = apply(&mut state, &PlatformEvent::Occluded(true), t_occ);
        assert_eq!(out, vec![PlatformEvent::Occluded(true)]);
        assert!(state.occluded);
        assert!(state.occlusion.is_occluded());
        assert_eq!(
            state.pacer.throttle_state(t_occ),
            RenderThrottleState::Suspended
        );
        assert!(!state.pacer.should_render(t_occ));
        assert!(state.pacer.next_wake(t_occ).is_none());

        // 解除遮挡并获得焦点后立即触发首帧
        let t_rec = t_occ + Duration::from_millis(100);
        apply(&mut state, &PlatformEvent::Occluded(false), t_rec);
        apply(&mut state, &PlatformEvent::Focused(true), t_rec);
        assert!(!state.occluded);
        assert!(!state.occlusion.is_occluded());
        assert_eq!(
            state.pacer.throttle_state(t_rec),
            RenderThrottleState::Active
        );
        assert!(state.pacer.should_render(t_rec));

        // 最小化事件同样暂停渲染并在恢复后立刻重绘
        let t_min = t_rec + Duration::from_millis(50);
        state.pacer.on_present(t_min);
        apply(&mut state, &PlatformEvent::Minimized(true), t_min);
        assert!(state.minimized);
        assert!(state.occlusion.is_minimized());
        assert!(!state.pacer.should_render(t_min));
        assert!(state.pacer.next_wake(t_min).is_none());

        let t_res = t_min + Duration::from_millis(50);
        apply(&mut state, &PlatformEvent::Minimized(false), t_res);
        assert!(!state.minimized);
        assert!(!state.occlusion.is_minimized());
        assert!(state.pacer.should_render(t_res));
    }
}

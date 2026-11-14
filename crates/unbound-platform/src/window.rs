//! 窗口系统后端：将原生 Win32 事件翻译为引擎事件。
//!
//! 翻译过程刻意设计为单向且无损的：引擎层绝不会看到任何底层窗口库类型，
//! 从而可在不触及渲染器的前提下替换窗口库。
//!

use std::time::Instant;

use crate::latency::LatencyTracker;
use crate::pacing::{FramePacer, PresentWaitPolicy};
use crate::resize::{Extent, ResizeAction, ResizeTracker};

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
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Unbound UI".to_string(),
            width: 1280,
            height: 800,
            resizable: true,
            decorations: true,
            min_width: 960,
            min_height: 600,
            visible: true,
            refresh: crate::pacing::REFRESH_120,
        }
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
/// 文本输入框需要平台提供两项数据：当前正在显示的组合串（连同用于高亮绘制的
/// 选区范围）以及必须精确插入一次的已提交文本。提交文本在此缓冲，因为输入事件
/// 可能在两帧之间到达；[`Self::take_commit`] 负责将其取出。
///
///
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeState {
    /// 当前聚焦控件是否启用了 IME。
    pub enabled: bool,
    /// 组合字符串；无组合时为空。
    pub preedit: String,
    /// 组合选区的字节范围。
    pub selection: Option<(usize, usize)>,
    /// 等待插入的已提交文本。
    pub commit: String,
}

impl ImeState {
    /// 应用一条平台 IME 事件。
    pub fn apply(&mut self, event: &ImeEvent) {
        match event {
            ImeEvent::Enabled => self.enabled = true,
            ImeEvent::Preedit(text, selection) => {
                self.preedit.clone_from(text);
                self.selection = *selection;
            }
            ImeEvent::Commit(text) => {
                // 提交前通常会先清空预编辑串；此处防御性清空，以防部分平台未发送空预编辑事件。
                //
                self.preedit.clear();
                self.selection = None;
                self.commit.push_str(text);
            }
            ImeEvent::Disabled => {
                self.enabled = false;
                self.preedit.clear();
                self.selection = None;
            }
        }
    }

    /// 取出已提交文本，并将缓冲区置空。
    pub fn take_commit(&mut self) -> String {
        core::mem::take(&mut self.commit)
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
        self.commit.clear();
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
    /// 输入法组合串与已提交文本。
    pub ime: ImeState,
}

impl PlatformState {
    /// 为新创建的窗口初始化平台状态。
    #[must_use]
    pub fn new(config: &WindowConfig, extent: Extent, present: PresentWaitPolicy) -> Self {
        Self {
            resize: ResizeTracker::with_default_settle(extent),
            latency: LatencyTracker::new(),
            pacer: FramePacer::new(config.refresh),
            present,
            scale_factor: 1.0,
            modifiers: Modifiers::default(),
            focused: true,
            visible: config.visible,
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
            out.push(PlatformEvent::Focused(*focused));
        }
        PlatformEvent::VisibilityChanged(visible) => {
            state.visible = *visible;
            state.pacer.set_visible(*visible);
            out.push(PlatformEvent::VisibilityChanged(*visible));
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
}

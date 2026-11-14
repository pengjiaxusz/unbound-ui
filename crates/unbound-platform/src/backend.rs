//! 具体的 `winit` 平台后端。
//!
//! 后端持有事件循环；窗口本身按 `winit` 要求在事件循环**内部**创建，
//! 从而确保 Vulkan 表面始终针对存活的窗口创建。
//!
//!
//! 引擎层永远不会看到任何 `winit` 类型：所有事件均翻译为 [`PlatformEvent`]
//! 并应用到 [`PlatformState`]。

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::DeviceEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::a11y_bridge::A11yRequest;
use crate::a11y_winit::WinitA11y;
use crate::pacing::PresentWaitPolicy;
use crate::resize::Extent;
use crate::window::{
    apply, translate_ime_event, translate_window_event, PlatformError, PlatformEvent,
    PlatformState, WindowConfig,
};

/// 引擎期望平台下一步执行的操作。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    /// 继续运行事件循环。
    Continue,
    /// 关闭窗口并结束事件循环。
    Exit,
}

/// 平台层的引擎侧回调接口。
///
/// 该 trait 刻意保持精简：平台层负责帧时序与缩放策略，应用层负责场景逻辑。
///
pub trait PlatformApp {
    /// 窗口已就绪，应用可创建渲染器。
    fn resumed(&mut self, state: &mut PlatformState, window: &Window);

    /// 发生了平台事件。
    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow;

    /// 绘制一帧。
    ///
    /// 在事件循环空闲点调用，此时所有待处理事件已派发完毕。
    /// 应用在此获取交换链图像、执行渲染并提交呈现。
    ///
    fn frame(&mut self, state: &mut PlatformState, window: &Window) -> Flow;

    /// 窗口已隐藏；渲染器必须停止绘制。
    fn suspended(&mut self, state: &mut PlatformState) {
        let _ = state;
    }

    /// 待发布的无障碍语义增量（若应用发布了更新）。
    ///
    /// 仅当辅助技术正在监听时调用；返回 `None` 表示“无变更”。
    ///
    fn accessibility_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    /// 辅助技术请求执行某个动作。
    ///
    /// 应用应像处理等价的指针或键盘事件一样处理它。
    ///
    fn accessibility_action(
        &mut self,
        state: &mut PlatformState,
        window: &Window,
        request: A11yRequest,
    ) -> Flow {
        let _ = (state, window, request);
        Flow::Continue
    }

    /// 辅助窗口已创建，可创建其独立渲染表面。
    ///
    /// 默认实现将其视同主窗口，使仅支持单窗口的应用也能正常运行
    /// （仅多创建一个渲染器实例）。
    fn window_created(&mut self, state: &mut PlatformState, window: &Window) {
        self.resumed(state, window);
    }

    /// 辅助窗口即将关闭；释放其相关资源。
    fn window_closing(&mut self, window: &Window) {
        let _ = window;
    }

    /// 请求平台打开另一个窗口。
    ///
    /// 事件循环在每帧结束后轮询此方法；返回 `Some` 时按指定配置打开新窗口
    /// 并回调 [`Self::window_created`]。
    fn take_window_request(&mut self) -> Option<WindowConfig> {
        None
    }

    /// 当应用即使在无输入时也需要唤醒事件循环的时间点。
    ///
    /// 挂起的定时器（光标闪烁、提示框延迟）在此返回截止时间，
    /// 使循环可休眠在 `WaitUntil` 上而非每帧轮询。
    fn next_wake(&self) -> Option<Instant> {
        None
    }

    /// 向应用交付一个可唤醒休眠事件循环的回调。
    ///
    /// 事件循环创建后调用一次；后台任务就绪时调用该回调，触发 `about_to_wait`
    /// 以归约任务结果。
    fn attach_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        let _ = waker;
    }
}

/// 平台窗口：持有未来将拥有窗口的事件循环。
pub struct PlatformWindow {
    event_loop: EventLoop<()>,
    config: WindowConfig,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl PlatformWindow {
    /// 创建事件循环。
    ///
    /// 窗口本身在循环启动后创建，因为 `winit` 要求必须从事件循环内部创建窗口。
    ///
    ///
    /// # Errors
    /// 当无法创建事件循环时返回 [`PlatformError`]。
    pub fn new(config: &WindowConfig) -> Result<Self, PlatformError> {
        let event_loop =
            EventLoop::new().map_err(|error| PlatformError::EventLoop(error.to_string()))?;
        let proxy = event_loop.create_proxy();
        let waker = Arc::new(move || {
            let _ = proxy.send_event(());
        });
        Ok(Self {
            event_loop,
            config: config.clone(),
            waker,
        })
    }

    /// 运行事件循环直至应用退出。
    ///
    /// # Errors
    /// 当事件循环自身失败时返回 [`PlatformError`]。
    pub fn run<A: PlatformApp>(self, app: A) -> Result<(), PlatformError> {
        let Self {
            event_loop,
            config,
            waker,
        } = self;
        let mut runner = Runner {
            app,
            config,
            waker,
            windows: Vec::new(),
            primary: None,
        };
        event_loop
            .run_app(&mut runner)
            .map_err(|error| PlatformError::EventLoop(error.to_string()))
    }
}

/// 单个存活窗口及其归属的所有平台状态。
struct WindowEntry {
    window: Window,
    state: PlatformState,
    a11y: WinitA11y,
}

/// `winit` 与 [`PlatformApp`] 之间的粘合层。
///
/// 每个窗口拥有各自的 `PlatformState`（缩放追踪器、节拍器、IME）及独立的
/// 无障碍适配器；应用接收 `winit` 句柄并按 `window.id()` 索引渲染资源。
/// 关闭辅助窗口仅移除该条目；关闭主窗口则结束整个事件循环。
///
///
struct Runner<A> {
    app: A,
    config: WindowConfig,
    waker: Arc<dyn Fn() + Send + Sync>,
    windows: Vec<WindowEntry>,
    primary: Option<WindowId>,
}

impl<A: PlatformApp> Runner<A> {
    /// 排空入站无障碍动作并发布出站增量。
    ///
    /// 无障碍更新在帧边界泵送，绝不逐事件触发：语义树是帧产物，
    /// 静止帧不产生任何更新。
    ///
    fn pump(app: &mut A, a11y: &mut WinitA11y, state: &mut PlatformState, window: &Window) -> Flow {
        for request in a11y.drain_requests() {
            if app.accessibility_action(state, window, request) == Flow::Exit {
                return Flow::Exit;
            }
        }
        a11y.publish_with(|| app.accessibility_update());
        Flow::Continue
    }

    /// 创建单个窗口、其无障碍适配器与平台状态。
    ///
    /// 窗口以隐藏状态创建，因为 AccessKit 要求适配器必须在首次 `show` 前存在。
    ///
    fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        config: &WindowConfig,
    ) -> Option<WindowEntry> {
        let mut attributes = Window::default_attributes()
            .with_title(&config.title)
            .with_inner_size(LogicalSize::new(
                f64::from(config.width),
                f64::from(config.height),
            ))
            .with_resizable(config.resizable)
            .with_decorations(config.decorations)
            .with_visible(false);
        if config.min_width > 0 && config.min_height > 0 {
            attributes = attributes.with_min_inner_size(LogicalSize::new(
                f64::from(config.min_width),
                f64::from(config.min_height),
            ));
        }
        let window = event_loop.create_window(attributes).ok()?;
        // 适配器在 Windows 上持有窗口的 UI Automation 提供程序。
        let a11y = WinitA11y::new(event_loop, &window);
        window.set_visible(config.visible);
        let size = window.inner_size();
        let mut state = PlatformState::new(
            config,
            Extent::new(size.width, size.height),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.scale_factor = window.scale_factor();
        state.visible = window.is_visible().unwrap_or(true);
        Some(WindowEntry {
            window,
            state,
            a11y,
        })
    }

    /// 翻译并派发单个窗口事件。
    fn on_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: &winit::event::WindowEvent,
    ) -> Flow {
        let Some(index) = self
            .windows
            .iter()
            .position(|entry| entry.window.id() == window_id)
        else {
            return Flow::Continue;
        };
        let entry = &mut self.windows[index];
        // 适配器必须在应用之前看到每一条事件。
        entry.a11y.process_event(&entry.window, event);
        // IME 组合输入是状态而非增量：在任何其他逻辑查看该事件前先将其折叠进
        // 平台状态。缺失此步骤会导致东亚输入法只能输入 ASCII。
        //
        if let Some(ime) = translate_ime_event(event) {
            entry.state.ime.apply(&ime);
        }
        let Some(platform_event) = translate_window_event(event) else {
            return Flow::Continue;
        };
        let now = Instant::now();
        if matches!(platform_event, PlatformEvent::CloseRequested) {
            self.app
                .event(&mut entry.state, &entry.window, platform_event);
            if self.primary == Some(window_id) {
                return Flow::Exit;
            }
            let entry = &self.windows[index];
            self.app.window_closing(&entry.window);
            self.windows.remove(index);
            return Flow::Continue;
        }
        for out in apply(&mut entry.state, &platform_event, now) {
            if self.app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                return Flow::Exit;
            }
        }
        // 当用户拖拽窗口边缘时（`WM_ENTERSIZEMOVE`），Windows 会运行**模态**循环：
        // 事件循环被挂起，直到拖拽结束才会调用 `about_to_wait`。因此仅由
        // `about_to_wait` 驱动的帧渲染会在整个拖拽期间冻结，合成器只能将最后一帧
        // 拉伸到新尺寸。
        //
        //
        // 直接在此处驱动帧绘制，使每次 `WM_SIZE` 都能按新分辨率生成全新图像。
        //
        //
        // 仅尺寸调整采用此方式驱动：`RedrawRequested` 不携带尺寸信息，由模态循环外的
        // `about_to_wait` 服务——若此处也渲染，每帧会被绘制两次。
        //
        //
        if matches!(platform_event, PlatformEvent::Resized(_)) {
            entry.state.pacer.request_immediate(now);
            if self.app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                return Flow::Exit;
            }
            if Self::pump(
                &mut self.app,
                &mut entry.a11y,
                &mut entry.state,
                &entry.window,
            ) == Flow::Exit
            {
                return Flow::Exit;
            }
        }
        let _ = event_loop;
        Flow::Continue
    }
}

impl<A: PlatformApp> ApplicationHandler<()> for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            return;
        }
        let config = self.config.clone();
        let Some(mut entry) = self.create_window(event_loop, &config) else {
            event_loop.exit();
            return;
        };
        self.primary = Some(entry.window.id());
        self.app.attach_waker(Arc::clone(&self.waker));
        self.app.resumed(&mut entry.state, &entry.window);
        self.windows.push(entry);
    }

    /// 后台任务请求重绘一帧；随后 `about_to_wait` 将运行并归约其结果。
    ///
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, (): ()) {}

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        if self.on_window_event(event_loop, window_id, &event) == Flow::Exit {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        let DeviceEvent::MouseMotion { delta } = event else {
            return;
        };
        let Some(index) = self.primary.and_then(|id| {
            self.windows
                .iter()
                .position(|entry| entry.window.id() == id)
        }) else {
            return;
        };
        let entry = &mut self.windows[index];
        entry.state.latency.on_input(Instant::now());
        let out = PlatformEvent::PointerRaw {
            dx: delta.0,
            dy: delta.1,
        };
        if self.app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
            event_loop.exit();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        for index in 0..self.windows.len() {
            let entry = &mut self.windows[index];
            let out = PlatformEvent::VisibilityChanged(false);
            for out in apply(&mut entry.state, &out, now) {
                let _ = out;
            }
            self.app.suspended(&mut entry.state);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 窗口可能在挂起/恢复周期后重新出现。
        if self.windows.is_empty() {
            event_loop.exit();
            return;
        }
        // 为每个窗口绘制一帧：应用按句柄索引其资源，各窗口状态独立掌控各自节拍。
        //
        for index in 0..self.windows.len() {
            let entry = &mut self.windows[index];
            if self.app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                event_loop.exit();
                return;
            }
            if Self::pump(
                &mut self.app,
                &mut entry.a11y,
                &mut entry.state,
                &entry.window,
            ) == Flow::Exit
            {
                event_loop.exit();
                return;
            }
        }

        // 帧期间请求的辅助窗口在此处创建，使应用无需深入事件循环内部。
        //
        while let Some(config) = self.app.take_window_request() {
            let Some(mut entry) = self.create_window(event_loop, &config) else {
                break;
            };
            self.app.window_created(&mut entry.state, &entry.window);
            self.windows.push(entry);
        }

        // 帧循环必须自行驱动。默认的 `ControlFlow::Wait` 会将循环休眠至**下一次输入事件**，
        // 因此仅按需绘制的应用会显示冻结窗口。节拍器知道下一帧何时到期；循环休眠至
        // 该时刻并自行唤醒。主窗口的节拍驱动整个事件循环。
        //
        //
        //
        let Some(index) = self.primary.and_then(|id| {
            self.windows
                .iter()
                .position(|entry| entry.window.id() == id)
        }) else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            return;
        };
        let state = &self.windows[index].state;
        let now = Instant::now();
        // 节拍器知道下一**帧**何时到期；应用知道下一**定时器**何时到期。
        // 循环必须按两者中较早到达的时刻唤醒。
        //
        let deadline = match (state.pacer.next_wake(now), self.app.next_wake()) {
            (Some(frame), Some(timer)) => Some(frame.min(timer)),
            (frame, timer) => frame.or(timer),
        };
        let flow = match deadline {
            Some(deadline) => winit::event_loop::ControlFlow::WaitUntil(deadline),
            // 尚未呈现任何帧**且**应用仍请求绘制：轮询以确保首帧顺利通过。
            //
            None if state.visible && state.pacer.animating() => {
                winit::event_loop::ControlFlow::Poll
            }
            // 窗口已隐藏，或页面处于静止空闲状态：休眠等待事件到达而非空转。
            //
            None => winit::event_loop::ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }
}

/// 获取窗口的原始句柄，供 `vkCreateWin32SurfaceKHR` 使用。
///
/// 当平台拒绝提供句柄时返回 `None`（例如无原生表面支撑的窗口）。
///
#[must_use]
pub fn raw_window_handle(window: &Window) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    window.window_handle().ok().map(|handle| handle.as_raw())
}

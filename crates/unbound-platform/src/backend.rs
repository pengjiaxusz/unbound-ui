//! 原生 Win32 平台后端。
//!
//! 后端直接基于官方 `windows` crate（0.62.2）的 Win32 API（`RegisterClassExW`、
//! `CreateWindowExW`、`PeekMessageW`、`MsgWaitForMultipleObjectsEx`）驱动事件循环与
//! 多窗口生命周期，不再依赖手写 FFI 块或 `winit`。
//! 引擎层通过 [`PlatformEvent`] 与 [`PlatformState`] 消费所有平台输入。

use std::sync::Arc;
use std::time::Instant;

use crate::a11y_platform::PlatformA11y;
use crate::native_window::NativeWindow;
#[cfg(windows)]
use crate::win32_window::Window;
use crate::window::{PlatformError, PlatformEvent, PlatformState, WindowConfig};

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
pub trait PlatformApp {
    /// 窗口已就绪，应用可创建渲染器。
    fn resumed(&mut self, state: &mut PlatformState, window: &NativeWindow);

    /// 发生了平台事件。
    fn event(
        &mut self,
        state: &mut PlatformState,
        window: &NativeWindow,
        event: PlatformEvent,
    ) -> Flow;

    /// 绘制一帧。
    ///
    /// 在事件循环空闲点调用，此时所有待处理事件已派发完毕。
    /// 应用在此获取交换链图像、执行渲染并提交呈现。
    fn frame(&mut self, state: &mut PlatformState, window: &NativeWindow) -> Flow;

    /// 窗口已隐藏；渲染器必须停止绘制。
    fn suspended(&mut self, state: &mut PlatformState) {
        let _ = state;
    }

    /// 待发布的无障碍语义增量（若应用发布了更新）。
    fn accessibility_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    /// 按窗口维度的待发布无障碍语义增量（多窗场景默认回退至 [`Self::accessibility_update`]）。
    fn accessibility_update_for_window(
        &mut self,
        window: &NativeWindow,
    ) -> Option<accesskit::TreeUpdate> {
        let _ = window;
        self.accessibility_update()
    }

    /// 辅助技术请求执行某个动作。
    fn accessibility_action(
        &mut self,
        state: &mut PlatformState,
        window: &NativeWindow,
        request: crate::a11y_bridge::A11yRequest,
    ) -> Flow {
        let _ = (state, window, request);
        Flow::Continue
    }

    /// 辅助窗口已创建，可创建其独立渲染表面。
    fn window_created(&mut self, state: &mut PlatformState, window: &NativeWindow) {
        self.resumed(state, window);
    }

    /// 辅助窗口即将关闭；释放其相关资源。
    fn window_closing(&mut self, window: &NativeWindow) {
        let _ = window;
    }

    /// 请求平台打开另一个窗口。
    fn take_window_request(&mut self) -> Option<WindowConfig> {
        None
    }

    /// 排空本帧需要由平台显式关闭并销毁原生句柄的应用窗口 ID 列表。
    fn take_window_close_requests(&mut self) -> Vec<crate::windows::WindowId> {
        Vec::new()
    }

    /// 排空本帧需要同步（创建或重定位）的越界原生弹出窗请求。
    fn take_popup_sync_requests(&mut self) -> Vec<(u64, crate::popup_host::PopupWindowConfig)> {
        Vec::new()
    }

    /// 排空本帧需要显式关闭的原生弹出窗 ID 列表。
    fn take_popup_close_requests(&mut self) -> Vec<u64> {
        Vec::new()
    }

    /// 当应用即使在无输入时也需要唤醒事件循环的时间点。
    fn next_wake(&self) -> Option<Instant> {
        None
    }

    /// 向应用交付一个可唤醒休眠事件循环的回调。
    fn attach_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        let _ = waker;
    }
}

/// 平台窗口：持有初始窗口配置与原生 Win32 事件循环。
pub struct PlatformWindow {
    #[cfg_attr(not(windows), allow(dead_code))]
    config: WindowConfig,
}

impl PlatformWindow {
    /// 创建原生平台窗口运行器（Windows 走 Win32，Linux 走原生 Wayland）。
    ///
    /// # Errors
    /// 在不支持的平台或系统初始化失败时返回 [`PlatformError`]。
    pub fn new(config: &WindowConfig) -> Result<Self, PlatformError> {
        #[cfg(windows)]
        {
            win32::enable_per_monitor_v2_dpi();
            Ok(Self {
                config: config.clone(),
            })
        }
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
        {
            Ok(Self {
                config: config.clone(),
            })
        }
        #[cfg(not(any(
            windows,
            all(unix, not(target_os = "macos"), not(target_os = "android"))
        )))]
        {
            let _ = config;
            Err(PlatformError::EventLoop(
                "native window backend is not implemented on this platform".to_string(),
            ))
        }
    }

    /// 运行原生平台事件循环直至主窗口关闭或应用退出。
    ///
    /// # Errors
    /// 当窗口创建或事件循环失败时返回 [`PlatformError`]。
    pub fn run<A: PlatformApp>(self, app: A) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win32::run_event_loop(self.config, app)
        }
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
        {
            let env_val = std::env::var("UNBOUND_LINUX_BACKEND").ok();
            let pref = crate::linux_backend::parse_linux_backend_env(env_val.as_deref())?;
            let display_env = crate::linux_backend::LinuxDisplayEnv::from_process();
            let candidates = crate::linux_backend::select_linux_backend_order(pref, &display_env)?;
            if candidates.len() == 1 {
                return match candidates[0] {
                    crate::linux_backend::LinuxBackendKind::Wayland => {
                        crate::wayland_backend::run_event_loop(self.config, app)
                    }
                    crate::linux_backend::LinuxBackendKind::X11 => {
                        crate::x11_backend::run_event_loop(self.config, app)
                    }
                };
            }
            // Auto 模式下先轻量探测 Wayland 连接可用性，失败再回退 X11，避免消耗 `app` 所有权。
            if crate::wayland_backend::can_connect_wayland() {
                crate::wayland_backend::run_event_loop(self.config, app)
            } else {
                crate::x11_backend::run_event_loop(self.config, app)
            }
        }
        #[cfg(not(any(
            windows,
            all(unix, not(target_os = "macos"), not(target_os = "android"))
        )))]
        {
            let _ = app;
            Err(PlatformError::EventLoop(
                "native window backend is not implemented on this platform".to_string(),
            ))
        }
    }
}

/// 获取窗口的原始句柄，供 `vkCreateWin32SurfaceKHR` 使用。
#[must_use]
pub fn raw_window_handle(window: &NativeWindow) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    window.window_handle().ok().map(|handle| handle.as_raw())
}

/// 排空入站无障碍动作并发布出站增量。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn pump_a11y<A: PlatformApp>(
    app: &mut A,
    a11y: &mut PlatformA11y,
    state: &mut PlatformState,
    window: &NativeWindow,
) -> Flow {
    for request in a11y.drain_requests() {
        if app.accessibility_action(state, window, request) == Flow::Exit {
            return Flow::Exit;
        }
    }
    a11y.publish_with(|| app.accessibility_update_for_window(window));
    Flow::Continue
}

/// Win32 窗口样式位常量（独立于目标平台暴露，便于跨平台单元测试验证样式选择策略）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod style_bits {
    pub const WS_OVERLAPPED: u32 = 0x0000_0000;
    /// `WS_POPUP`：无边框方案曾使用的样式位。当前实现改用保留 `WS_CAPTION` 的
    /// `WS_OVERLAPPEDWINDOW`，本常量仅由样式单元测试断言"绝不出现 POPUP"时引用。
    #[allow(dead_code)]
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WS_CAPTION: u32 = 0x00C0_0000;
    pub const WS_SYSMENU: u32 = 0x0008_0000;
    pub const WS_THICKFRAME: u32 = 0x0004_0000;
    pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    pub const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    pub const WS_OVERLAPPEDWINDOW: u32 =
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
}

/// 向窗口事件队列推入新事件，并对同类的窗口几何变化（`Resized`、`Moved`）与鼠标移动（`CursorMoved`）
/// 执行原地合并。
///
/// 在 Win32 模态移动/缩放（`WM_ENTERSIZEMOVE`..`WM_EXITSIZEMOVE`）期间，操作系统会在单次
/// `DispatchMessageW` 调用内高频派发数百条 `WM_SIZE` 与 `WM_MOVE`。若逐条排队而不合并，消息循环在
/// 退出模态拖拽或单轮排空时会逐帧回放整条历史几何轨迹，在视觉上表现为极其缓慢的“滞后过渡动画”。
/// 原地覆盖未消费的旧几何事件可确保任意时刻队列中最多只保留最后一次真实窗口尺寸与位置。
///
/// 合并是对整条队列查找同类事件后原地覆盖，因此同类事件在批内最多保留最后一条，且其队列位置
/// 可能早于其间插入的其他事件；消费方不得依赖被合并事件的相对顺序。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn push_coalesced_event(events: &mut Vec<PlatformEvent>, event: PlatformEvent) {
    match event {
        PlatformEvent::Resized(extent) => {
            if let Some(existing) = events
                .iter_mut()
                .rev()
                .find(|ev| matches!(ev, PlatformEvent::Resized(_)))
            {
                *existing = PlatformEvent::Resized(extent);
            } else {
                events.push(PlatformEvent::Resized(extent));
            }
        }
        PlatformEvent::Moved { x, y } => {
            if let Some(PlatformEvent::Moved {
                x: prev_x,
                y: prev_y,
            }) = events
                .iter_mut()
                .rev()
                .find(|ev| matches!(ev, PlatformEvent::Moved { .. }))
            {
                *prev_x = x;
                *prev_y = y;
            } else {
                events.push(PlatformEvent::Moved { x, y });
            }
        }
        PlatformEvent::CursorMoved { x, y } => {
            if let Some(PlatformEvent::CursorMoved {
                x: prev_x,
                y: prev_y,
            }) = events.last_mut()
            {
                *prev_x = x;
                *prev_y = y;
            } else {
                events.push(PlatformEvent::CursorMoved { x, y });
            }
        }
        other => events.push(other),
    }
}

/// 依据窗口配置选择原生窗口样式。
///
/// - `decorations = true`：标准系统装饰窗口（`WS_OVERLAPPEDWINDOW` 或不含 `WS_THICKFRAME`）。
/// - `decorations = false`：保留 `WS_OVERLAPPED | WS_CAPTION`（而非 `WS_POPUP`），确保 DWM
///   完整保留最小化/最大化/关闭系统过渡动画、阴影、Win11 圆角与 `HTTOP` 双击纵向全高显示；
///   同时在 `resizable = true` 时保留 `WS_THICKFRAME` 以支持四边/四角缩放与 Snap Layout，
///   非客户区标题栏与边框统一由 `WM_NCCALCSIZE` 返回 `0` 裁切，避免出现系统白边。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn compute_window_style(config: &WindowConfig) -> u32 {
    use style_bits::*;
    if config.decorations {
        if config.resizable {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
        }
    } else if config.resizable {
        WS_OVERLAPPEDWINDOW
    } else {
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
    }
}

/// 将相对等待时长转换为 Win32 `SetWaitableTimer` 所需的负值 100 纳秒间隔。
///
/// 零时长返回 `0`；非零时长至少为 `-1`（100 ns），极大值钳制在 `-i64::MAX` 以防溢出。
#[must_use]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn duration_to_win32_relative_due_time(wait: std::time::Duration) -> i64 {
    if wait.is_zero() {
        return 0;
    }
    let ticks_100ns = (wait.as_nanos() / 100).clamp(1, i64::MAX as u128);
    -(ticks_100ns as i64)
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use core::ffi::c_void;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
    use std::time::Duration;

    use ::windows::core::{s, w, BOOL, PCWSTR};
    use ::windows::Win32::Foundation::{
        CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use ::windows::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, ScreenToClient,
        MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use ::windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use ::windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
    use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use ::windows::Win32::UI::Controls::WM_MOUSELEAVE;
    use ::windows::Win32::UI::HiDpi::{
        GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use ::windows::Win32::UI::Input::Ime::{
        ImmGetCompositionStringW, ImmGetContext, ImmGetDescriptionW, ImmReleaseContext,
        GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTSTR, HIMC, IMECHARPOSITION, IME_COMPOSITION_STRING,
        IMR_QUERYCHARPOSITION,
    };
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        GetFocus, GetKeyState, GetKeyboardLayout, GetKeyboardState, ReleaseCapture, SetCapture,
        ToUnicode, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, VK_APPS, VK_BACK,
        VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME,
        VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_PROCESSKEY, VK_RETURN,
        VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
    };
    use ::windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetCursorPos, GetWindow, GetWindowLongPtrW, GetWindowLongW, GetWindowRect, IsIconic,
        IsWindowVisible, IsZoomed, LoadCursorW, MsgWaitForMultipleObjectsEx, PeekMessageW,
        PostMessageW, RegisterClassExW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, CW_USEDEFAULT,
        GWLP_USERDATA, GWL_EXSTYLE, GW_HWNDPREV, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT,
        HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
        HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE, MINMAXINFO, MSG, MWMO_INPUTAVAILABLE,
        NCCALCSIZE_PARAMS, PM_REMOVE, QS_ALLINPUT, SIZE_MAXIMIZED, SIZE_MINIMIZED, SIZE_RESTORED,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
        SW_SHOW, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CHAR, WM_CLOSE, WM_DESTROY,
        WM_DPICHANGED, WM_DROPFILES, WM_GETMINMAXINFO, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION,
        WM_IME_NOTIFY, WM_IME_REQUEST, WM_IME_SETCONTEXT, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
        WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
        WM_MOUSEACTIVATE, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_MOVE, WM_NCCALCSIZE,
        WM_NCCREATE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE,
        WM_NCMOUSEMOVE, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS,
        WM_SETTINGCHANGE, WM_SHOWWINDOW, WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
        WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_EX_APPWINDOW, WS_EX_LAYERED,
        WS_EX_TRANSPARENT,
    };

    use crate::pacing::PresentWaitPolicy;
    use crate::resize::Extent;
    use crate::win32_window::{CursorIcon, WM_APP_WAKE};
    use crate::window::{
        apply, resolve_window_placement, ButtonState, ImeEvent, Modifiers, MouseButton,
        PlatformEvent, PlatformKey, WindowPosition,
    };

    const INFINITE_TIMEOUT: u32 = u32::MAX;

    /// `CreateWaitableTimerExW` 高分辨率定时器标志（Windows 10 1803+）。
    const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: u32 = 0x0000_0002;
    /// `TIMER_ALL_ACCESS`（`STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3`）。
    const TIMER_ALL_ACCESS: u32 = 0x001F_0003;
    /// 高分辨率定时器免系统调用自旋阈值：短于该时长直接返回循环由 pacer 精确自旋。
    const HIGH_RES_MIN_WAIT: Duration = Duration::from_micros(250);

    type CreateWaitableTimerExWFn =
        unsafe extern "system" fn(*const c_void, PCWSTR, u32, u32) -> HANDLE;
    type SetWaitableTimerFn = unsafe extern "system" fn(
        HANDLE,
        *const i64,
        i32,
        *const c_void,
        *const c_void,
        BOOL,
    ) -> BOOL;

    /// Win32 高分辨率可等待定时器封装（ADR-0030）。
    ///
    /// 通过从 `kernel32.dll` 动态解析 `CreateWaitableTimerExW` 与 `SetWaitableTimer`，
    /// 创建带 `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` 的自动复位定时器，仅作用于本进程等待句柄，
    /// 避免调用 `timeBeginPeriod(1)` 影响全局系统时钟与功耗；在不支持的系统上返回 `None` 并优雅回退。
    struct HighResWaitableTimer {
        handle: HANDLE,
        set_waitable_timer: SetWaitableTimerFn,
    }

    impl HighResWaitableTimer {
        fn try_create() -> Option<Self> {
            // SAFETY: `kernel32.dll` 常驻所有 Win32 进程；若符号不存在或系统不支持
            // `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`，返回空句柄并回退原有等待逻辑。
            unsafe {
                let module = GetModuleHandleW(w!("kernel32.dll")).ok()?;
                let create_proc = GetProcAddress(module, s!("CreateWaitableTimerExW"))?;
                let set_proc = GetProcAddress(module, s!("SetWaitableTimer"))?;
                let create_waitable_timer_ex: CreateWaitableTimerExWFn =
                    core::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        CreateWaitableTimerExWFn,
                    >(create_proc);
                let set_waitable_timer: SetWaitableTimerFn = core::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    SetWaitableTimerFn,
                >(set_proc);
                let handle = create_waitable_timer_ex(
                    core::ptr::null(),
                    PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                );
                if handle.0.is_null() || handle.0 == (-1isize as *mut c_void) {
                    return None;
                }
                Some(Self {
                    handle,
                    set_waitable_timer,
                })
            }
        }

        /// 以相对等待时长装填一次性高分辨率定时器；装填失败时返回 `false` 以便调用方回退。
        fn arm(&self, wait: Duration) -> bool {
            let due_time = duration_to_win32_relative_due_time(wait);
            if due_time == 0 {
                return false;
            }
            // SAFETY: `self.handle` 为本结构体持有的有效定时器句柄，`due_time` 位于当前栈帧。
            unsafe {
                (self.set_waitable_timer)(
                    self.handle,
                    &raw const due_time,
                    0,
                    core::ptr::null(),
                    core::ptr::null(),
                    BOOL(0),
                )
                .as_bool()
            }
        }

        fn handle(&self) -> HANDLE {
            self.handle
        }
    }

    impl Drop for HighResWaitableTimer {
        fn drop(&mut self) {
            if !self.handle.0.is_null() && self.handle.0 != (-1isize as *mut c_void) {
                // SAFETY: 释放由 `CreateWaitableTimerExW` 创建的定时器句柄，仅在 Drop 时调用一次。
                unsafe {
                    let _ = CloseHandle(self.handle);
                }
            }
        }
    }

    /// 等待下一批 Win32 消息或帧截止时间到期。
    ///
    /// - `wait = None`：无限期阻塞直至新消息到达（不挂载定时器句柄，防止旧定时器误唤醒空闲态）；
    /// - `wait = Some(dur)`：优先通过高分辨率可等待定时器唤醒，创建或装填失败时优雅回退至
    ///   `MsgWaitForMultipleObjectsEx` 毫秒超时等待。
    fn wait_for_events_or_deadline(
        high_res_timer: Option<&HighResWaitableTimer>,
        wait: Option<Duration>,
    ) {
        let Some(wait) = wait else {
            unsafe {
                let _ = MsgWaitForMultipleObjectsEx(
                    None,
                    INFINITE_TIMEOUT,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                );
            }
            return;
        };

        if let Some(timer) = high_res_timer {
            if wait <= HIGH_RES_MIN_WAIT {
                return;
            }
            if timer.arm(wait) {
                let handles = [timer.handle()];
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(
                        Some(handles.as_slice()),
                        INFINITE_TIMEOUT,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    );
                }
                return;
            }
        }

        if wait <= Duration::from_millis(1) {
            return;
        }
        let timeout_ms = wait.as_millis().min(u128::from(INFINITE_TIMEOUT - 1)) as u32;
        unsafe {
            let _ = MsgWaitForMultipleObjectsEx(None, timeout_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }

    std::thread_local! {
        /// 在 `DispatchMessageW` 进入系统模态移动/缩放循环期间挂载的单步即时渲染回调。
        static MODAL_FRAME_CALLBACK: std::cell::Cell<Option<*mut dyn FnMut(HWND)>> = const { std::cell::Cell::new(None) };
        /// 防止模态移动/缩放即时渲染期间产生递归重入的守卫标志。
        static MODAL_FRAME_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// 在单次 `DispatchMessageW` 派发期间临时挂载模态即时帧回调，离开作用域即卸载。
    ///
    /// 该守卫同时服务系统模态移动与模态缩放：两者都在派发内高频产生几何变化，必须各自即时出帧。
    struct ModalDispatchGuard;

    impl ModalDispatchGuard {
        fn install<'a>(cb: &'a mut dyn FnMut(HWND)) -> Self {
            // SAFETY: 裸指针仅在当前栈帧 `ModalDispatchGuard` 存活期内有效，`Drop` 时无条件清空。
            let raw: *mut dyn FnMut(HWND) = unsafe {
                core::mem::transmute::<&'a mut dyn FnMut(HWND), *mut (dyn FnMut(HWND) + 'static)>(
                    cb,
                )
            };
            MODAL_FRAME_CALLBACK.with(|slot| slot.set(Some(raw)));
            Self
        }
    }

    impl Drop for ModalDispatchGuard {
        fn drop(&mut self) {
            MODAL_FRAME_CALLBACK.with(|slot| slot.set(None));
        }
    }

    /// 模态移动与缩放共用的即时帧入口：请求并同步渲染一帧，避免几何变化积压到拖拽结束才消费。
    fn trigger_modal_immediate_frame(hwnd: HWND) {
        let busy = MODAL_FRAME_BUSY.with(|flag| {
            if flag.get() {
                true
            } else {
                flag.set(true);
                false
            }
        });
        if busy {
            return;
        }
        let cb_ptr = MODAL_FRAME_CALLBACK.with(std::cell::Cell::get);
        if let Some(ptr) = cb_ptr {
            // SAFETY: `ptr` 由当前线程上层的 `ModalDispatchGuard` 挂载，且 `MODAL_FRAME_BUSY` 已阻止重入。
            unsafe {
                (*ptr)(hwnd);
            }
        }
        MODAL_FRAME_BUSY.with(|flag| flag.set(false));
    }

    pub(super) fn enable_per_monitor_v2_dpi() {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    fn is_system_dark_mode() -> bool {
        let mut data: u32 = 1;
        let mut size = core::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut data).cast::<c_void>()),
                Some(&raw mut size),
            )
        };
        status.is_ok() && data == 0
    }

    fn current_modifiers() -> Modifiers {
        unsafe {
            Modifiers {
                shift: (GetKeyState(i32::from(VK_SHIFT.0)) as u16 & 0x8000) != 0,
                control: (GetKeyState(i32::from(VK_CONTROL.0)) as u16 & 0x8000) != 0,
                alt: (GetKeyState(i32::from(VK_MENU.0)) as u16 & 0x8000) != 0,
                meta: ((GetKeyState(i32::from(VK_LWIN.0)) as u16 & 0x8000) != 0)
                    || ((GetKeyState(i32::from(VK_RWIN.0)) as u16 & 0x8000) != 0),
            }
        }
    }

    fn translate_vk(vk: u16, scan_code: u32) -> PlatformKey {
        match vk {
            x if x == VK_RETURN.0 => PlatformKey::Named("Enter".into()),
            x if x == VK_ESCAPE.0 => PlatformKey::Named("Escape".into()),
            x if x == VK_TAB.0 => PlatformKey::Named("Tab".into()),
            x if x == VK_BACK.0 => PlatformKey::Named("Backspace".into()),
            x if x == VK_DELETE.0 => PlatformKey::Named("Delete".into()),
            x if x == VK_LEFT.0 => PlatformKey::Named("ArrowLeft".into()),
            x if x == VK_RIGHT.0 => PlatformKey::Named("ArrowRight".into()),
            x if x == VK_UP.0 => PlatformKey::Named("ArrowUp".into()),
            x if x == VK_DOWN.0 => PlatformKey::Named("ArrowDown".into()),
            x if x == VK_HOME.0 => PlatformKey::Named("Home".into()),
            x if x == VK_END.0 => PlatformKey::Named("End".into()),
            x if x == VK_PRIOR.0 => PlatformKey::Named("PageUp".into()),
            x if x == VK_NEXT.0 => PlatformKey::Named("PageDown".into()),
            x if x == VK_INSERT.0 => PlatformKey::Named("Insert".into()),
            x if x == VK_APPS.0 => PlatformKey::Named("ContextMenu".into()),
            x if x == VK_CAPITAL.0 => PlatformKey::Named("CapsLock".into()),
            x if x == VK_SPACE.0 => PlatformKey::Named("Space".into()),
            x if (VK_F1.0..=VK_F24.0).contains(&x) => {
                PlatformKey::Named(format!("F{}", x - VK_F1.0 + 1))
            }
            _ => {
                let mut key_state = [0u8; 256];
                let mut buf = [0u16; 8];
                if unsafe { GetKeyboardState(&mut key_state) }.is_ok() {
                    // 清除 Ctrl/Alt 状态以便快捷键也能得到对应字符键名
                    key_state[usize::from(VK_CONTROL.0)] = 0;
                    key_state[usize::from(VK_MENU.0)] = 0;
                    let len = unsafe {
                        ToUnicode(u32::from(vk), scan_code, Some(&key_state), &mut buf, 0)
                    };
                    if len > 0 {
                        let text = String::from_utf16_lossy(&buf[..len as usize]);
                        if !text.is_empty() && !text.chars().all(|c| c.is_control()) {
                            return PlatformKey::Character(text);
                        }
                    }
                }
                PlatformKey::Other(u32::from(vk))
            }
        }
    }

    fn imm_string(himc: HIMC, index: IME_COMPOSITION_STRING) -> Option<String> {
        let byte_len = unsafe { ImmGetCompositionStringW(himc, index, None, 0) };
        if byte_len <= 0 {
            return None;
        }
        let u16_len = (byte_len as usize) / 2;
        let mut buf = vec![0u16; u16_len];
        let copied = unsafe {
            ImmGetCompositionStringW(
                himc,
                index,
                Some(buf.as_mut_ptr().cast::<c_void>()),
                byte_len as u32,
            )
        };
        if copied <= 0 {
            return None;
        }
        let actual_u16 = (copied as usize) / 2;
        Some(String::from_utf16_lossy(&buf[..actual_u16]))
    }

    /// 当前键盘布局对应的输入法描述，仅用于诊断日志。
    ///
    /// 第三方输入法（搜狗、QQ 拼音等）与系统输入法在 IMM32 兼容层上的行为差异很大：
    /// 有的会持续投递组合串，有的只在最后给一次上屏结果。日志里带上输入法名字，
    /// 才能一眼分辨“代码回归”和“输入法实现差异”。
    fn ime_description() -> String {
        unsafe {
            let mut buf = [0u16; 128];
            let len = ImmGetDescriptionW(GetKeyboardLayout(0), Some(&mut buf));
            String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())])
        }
    }

    thread_local! {
        /// 上一次已投递给宿主的组合串，用于去重。
        ///
        /// 只缓存**预编辑**，不缓存也绝不轮询提交串：ADR 0011 规定平台层只有一条
        /// 提交通道（`WM_IME_COMPOSITION`）。
        static LAST_PREEDIT: std::cell::RefCell<String> =
            const { std::cell::RefCell::new(String::new()) };
    }

    /// 记录一次已经投递过的预编辑，让轮询与消息两条通道看到同一份“已发送”状态。
    fn mark_ime_preedit_delivered(preedit: &str) {
        LAST_PREEDIT.with(|cell| {
            let mut last = cell.borrow_mut();
            last.clear();
            last.push_str(preedit);
        });
    }

    /// 组合串是否发生了变化；变化时同步记录，避免同一份预编辑被重复推送。
    fn preedit_is_new(last: &mut String, comp: &str) -> bool {
        if last == comp {
            return false;
        }
        last.clear();
        last.push_str(comp);
        true
    }

    /// 从输入法上下文补读**组合串**，作为 `WM_IME_COMPOSITION(GCS_COMPSTR)` 的兜底。
    ///
    /// 只读 `GCS_COMPSTR`，绝不读 `GCS_RESULTSTR`。
    ///
    /// 提交串不能轮询：输入法在处理按键时就把结果写进了上下文，而
    /// `WM_IME_COMPOSITION` 是随后才派发的排队消息。轮询永远先读到它，
    /// 于是同一次上屏会被轮询和消息各投递一次（首字变两个词）；输入法在提交后
    /// 频繁发出的通知又会让轮询反复读到同一份结果（疯狂上屏）。
    /// 提交只有一个来源：`WM_IME_COMPOSITION`。
    ///
    /// 组合串则相反，它只活在上下文里，缺了消息就没人补，所以这里保留兜底读取。
    fn sync_ime_preedit(data: &WindowUserData, hwnd: HWND) {
        let (comp, cursor, himc_valid) = unsafe {
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                (String::new(), -1, false)
            } else {
                let comp = imm_string(himc, GCS_COMPSTR).unwrap_or_default();
                let cursor = ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0);
                let _ = ImmReleaseContext(hwnd, himc);
                (comp, cursor, true)
            }
        };
        ime_trace(format!(
            "ime sync himc={himc_valid} comp={comp:?} cursor={cursor}"
        ));

        let changed = LAST_PREEDIT.with(|cell| preedit_is_new(&mut cell.borrow_mut(), &comp));
        if !changed {
            return;
        }
        let selection = if cursor >= 0 {
            let byte_pos = comp
                .char_indices()
                .nth(cursor as usize)
                .map_or(comp.len(), |(idx, _)| idx);
            Some((byte_pos, byte_pos))
        } else {
            None
        };
        data.events
            .borrow_mut()
            .push(PlatformEvent::Ime(ImeEvent::Preedit(comp, selection)));
    }

    /// 是否开启输入法诊断日志。
    ///
    /// 除了环境变量，同时接受 `%TEMP%\unbound-ime.trace` 标记文件：
    /// 用资源管理器启动（非提升权限）时拿不到本进程的环境变量。
    fn ime_trace_enabled() -> bool {
        std::env::var_os("UNBOUND_IME_TRACE").is_some()
            || std::env::temp_dir().join("unbound-ime.trace").exists()
    }

    /// 输入法诊断日志。
    ///
    /// 仅在环境变量 `UNBOUND_IME_TRACE` 存在时追加写入
    /// `%TEMP%\unbound-ime.log`；正常运行时完全不产生输出。
    fn ime_trace(message: impl core::fmt::Display) {
        if !ime_trace_enabled() {
            return;
        }
        use std::io::Write;
        let path = std::env::temp_dir().join("unbound-ime.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{message}");
        }
    }

    struct WindowUserData {
        decorations: bool,
        resizable: bool,
        /// 是否为无激活越界弹出窗（用于拦截 `WM_MOUSEACTIVATE` 返回 `MA_NOACTIVATE`）。
        is_popup: bool,
        min_width: u32,
        min_height: u32,
        /// 原生装饰窗口的非客户区边框增量（用于 `WM_GETMINMAXINFO` 保证客户区最小尺寸准确）。
        frame_extra_w: std::cell::Cell<i32>,
        frame_extra_h: std::cell::Cell<i32>,
        cursor: Arc<AtomicU8>,
        tracking_leave: std::cell::Cell<bool>,
        minimized: std::cell::Cell<bool>,
        last_modifiers: std::cell::Cell<Modifiers>,
        events: RefCell<Vec<PlatformEvent>>,
        redraw_requested: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
    }

    /// 探测指定 Win32 窗口是否被 Z 序上方的不透明顶层窗口完全遮挡。
    fn probe_window_occluded(hwnd: HWND) -> bool {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return true;
            }
            let mut target_rc = RECT::default();
            if GetWindowRect(hwnd, &mut target_rc).is_err() {
                return false;
            }
            let target = unbound_math::Rect::new(
                target_rc.left as f32,
                target_rc.top as f32,
                (target_rc.right - target_rc.left).max(0) as f32,
                (target_rc.bottom - target_rc.top).max(0) as f32,
            );
            if target.is_empty() {
                return true;
            }
            let mut opaque_covers = Vec::new();
            let mut cursor = hwnd;
            for _ in 0..256 {
                let Ok(prev) = GetWindow(cursor, GW_HWNDPREV) else {
                    break;
                };
                if prev.0.is_null() {
                    break;
                }
                cursor = prev;
                if !IsWindowVisible(prev).as_bool() || IsIconic(prev).as_bool() {
                    continue;
                }
                let ex_style = GetWindowLongW(prev, GWL_EXSTYLE) as u32;
                if (ex_style & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0)) != 0 {
                    continue;
                }
                let mut cloaked: u32 = 0;
                let cloaked_ok = DwmGetWindowAttribute(
                    prev,
                    DWMWA_CLOAKED,
                    (&raw mut cloaked).cast::<c_void>(),
                    core::mem::size_of::<u32>() as u32,
                );
                if cloaked_ok.is_ok() && cloaked != 0 {
                    continue;
                }
                let mut rc = RECT::default();
                if GetWindowRect(prev, &mut rc).is_ok() {
                    let cover = unbound_math::Rect::new(
                        rc.left as f32,
                        rc.top as f32,
                        (rc.right - rc.left).max(0) as f32,
                        (rc.bottom - rc.top).max(0) as f32,
                    );
                    if !cover.is_empty() {
                        opaque_covers.push(cover);
                    }
                }
            }
            crate::overlay_host::is_rect_fully_covered(target, &opaque_covers)
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_NCCREATE {
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            if !cs.lpCreateParams.is_null() {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                }
            }
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }

        let user_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowUserData;
        if user_ptr.is_null() {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        let data = unsafe { &*user_ptr };

        if (0x0281..=0x0291).contains(&msg) || (0x0100..=0x0110).contains(&msg) {
            ime_trace(format!("ime msg=0x{msg:04X} wparam=0x{:X}", wparam.0));
        }

        match msg {
            WM_NCCALCSIZE => {
                if !data.decorations {
                    if wparam.0 != 0 && unsafe { IsZoomed(hwnd) }.as_bool() {
                        let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
                        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
                        let mut mi = MONITORINFO {
                            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
                            ..Default::default()
                        };
                        if unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
                            params.rgrc[0] = mi.rcWork;
                        }
                    }
                    // 无边框窗口将非客户区边框裁切为 0，同时保留 WS_THICKFRAME 缩放能力
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_NCHITTEST => {
                if !data.decorations && data.resizable && !unsafe { IsZoomed(hwnd) }.as_bool() {
                    let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                    let mut pt = POINT {
                        x: screen_x,
                        y: screen_y,
                    };
                    if unsafe { ScreenToClient(hwnd, &mut pt) }.as_bool() {
                        let mut client_rect = RECT::default();
                        if unsafe { GetClientRect(hwnd, &mut client_rect) }.is_ok() {
                            let scale =
                                crate::win32_window::win32::scale_factor(hwnd.0 as isize).max(0.5);
                            let border = (6.0 * scale).round().max(4.0) as i32;
                            let corner = (16.0 * scale).round().max(8.0) as i32;
                            let caption_btn_h = (48.0 * scale).round() as i32;
                            let caption_btn_w = (3.0 * 46.0 * scale).round() as i32;
                            let in_caption_buttons =
                                pt.y < caption_btn_h && pt.x > client_rect.right - caption_btn_w;

                            if !in_caption_buttons {
                                let on_left = pt.x < border;
                                let on_right = pt.x >= client_rect.right - border;
                                let on_top = pt.y < border;
                                let on_bottom = pt.y >= client_rect.bottom - border;

                                let in_left_corner = pt.x < corner;
                                let in_right_corner = pt.x >= client_rect.right - corner;
                                let in_top_corner = pt.y < corner;
                                let in_bottom_corner = pt.y >= client_rect.bottom - corner;

                                if (on_top && in_left_corner) || (on_left && in_top_corner) {
                                    return LRESULT(HTTOPLEFT as isize);
                                }
                                if (on_top && in_right_corner) || (on_right && in_top_corner) {
                                    return LRESULT(HTTOPRIGHT as isize);
                                }
                                if (on_bottom && in_left_corner) || (on_left && in_bottom_corner) {
                                    return LRESULT(HTBOTTOMLEFT as isize);
                                }
                                if (on_bottom && in_right_corner) || (on_right && in_bottom_corner)
                                {
                                    return LRESULT(HTBOTTOMRIGHT as isize);
                                }
                                if on_top {
                                    return LRESULT(HTTOP as isize);
                                }
                                if on_bottom {
                                    return LRESULT(HTBOTTOM as isize);
                                }
                                if on_left {
                                    return LRESULT(HTLEFT as isize);
                                }
                                if on_right {
                                    return LRESULT(HTRIGHT as isize);
                                }
                            }
                        }
                    }
                    return LRESULT(HTCLIENT as isize);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_MOUSEACTIVATE => {
                if data.is_popup {
                    // 越界原生弹出窗绝不抢占主窗口键盘焦点与输入法组合上下文。
                    return LRESULT(MA_NOACTIVATE as isize);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_CLOSE => {
                data.events.borrow_mut().push(PlatformEvent::CloseRequested);
                LRESULT(0)
            }
            WM_NCLBUTTONDOWN => {
                let ht = wparam.0 as u32;
                if !data.decorations && matches!(ht, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) {
                    // 阻止无 WS_CAPTION 窗口在点击自定义按钮时绘制旧式 GDI 按钮，
                    // 在 WM_NCLBUTTONUP 中完成最小化/最大化/关闭。
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_NCLBUTTONUP => {
                let ht = wparam.0 as u32;
                if ht == HTCLOSE {
                    let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
                    return LRESULT(0);
                }
                if !data.decorations && ht == HTMINBUTTON {
                    let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
                    return LRESULT(0);
                }
                if !data.decorations && ht == HTMAXBUTTON {
                    let cmd = if unsafe { IsZoomed(hwnd) }.as_bool() {
                        SW_RESTORE
                    } else {
                        SW_MAXIMIZE
                    };
                    let _ = unsafe { ShowWindow(hwnd, cmd) };
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_DESTROY => {
                data.destroyed.store(true, Ordering::Release);
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_GETMINMAXINFO => {
                if data.min_width > 0 || data.min_height > 0 {
                    let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                    let scale = crate::win32_window::win32::scale_factor(hwnd.0 as isize);
                    let extra_w = if data.decorations {
                        data.frame_extra_w.get().max(0)
                    } else {
                        0
                    };
                    let extra_h = if data.decorations {
                        data.frame_extra_h.get().max(0)
                    } else {
                        0
                    };
                    if data.min_width > 0 {
                        info.ptMinTrackSize.x =
                            (f64::from(data.min_width) * scale).round() as i32 + extra_w;
                    }
                    if data.min_height > 0 {
                        info.ptMinTrackSize.y =
                            (f64::from(data.min_height) * scale).round() as i32 + extra_h;
                    }
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SIZE => {
                let size_type = wparam.0 as u32;
                if size_type == SIZE_MINIMIZED {
                    if !data.minimized.replace(true) {
                        data.events
                            .borrow_mut()
                            .push(PlatformEvent::Minimized(true));
                    }
                    return LRESULT(0);
                }
                if (size_type == SIZE_RESTORED || size_type == SIZE_MAXIMIZED)
                    && data.minimized.replace(false)
                {
                    data.events
                        .borrow_mut()
                        .push(PlatformEvent::Minimized(false));
                }
                let width = (lparam.0 & 0xFFFF) as u32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                if width > 0 && height > 0 {
                    push_coalesced_event(
                        &mut data.events.borrow_mut(),
                        PlatformEvent::Resized(Extent::new(width, height)),
                    );
                    data.redraw_requested.store(true, Ordering::Release);
                    trigger_modal_immediate_frame(hwnd);
                }
                LRESULT(0)
            }
            WM_MOVE => {
                // `WM_MOVE` 的 lParam 是客户区左上角的屏幕物理坐标；尺寸变化由 `WM_SIZE` 处理。
                let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                push_coalesced_event(&mut data.events.borrow_mut(), PlatformEvent::Moved { x, y });
                data.redraw_requested.store(true, Ordering::Release);
                trigger_modal_immediate_frame(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let new_dpi = (wparam.0 & 0xFFFF) as u32;
                let scale = if new_dpi == 0 {
                    1.0
                } else {
                    f64::from(new_dpi) / 96.0
                };
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::ScaleFactorChanged(scale));
                let suggested = unsafe { &*(lparam.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                LRESULT(0)
            }
            WM_SETFOCUS => {
                data.events.borrow_mut().push(PlatformEvent::Focused(true));
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                data.events.borrow_mut().push(PlatformEvent::Focused(false));
                LRESULT(0)
            }
            WM_SHOWWINDOW => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::VisibilityChanged(wparam.0 != 0));
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SETTINGCHANGE => {
                data.events.borrow_mut().push(PlatformEvent::ThemeChanged {
                    dark: is_system_dark_mode(),
                });
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SETCURSOR => {
                if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                    let icon = CursorIcon::from_u8(data.cursor.load(Ordering::Acquire));
                    crate::win32_window::win32::apply_cursor(icon);
                    return LRESULT(1);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_MOUSEMOVE => {
                if !data.tracking_leave.get() {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = unsafe { TrackMouseEvent(&mut tme) };
                    data.tracking_leave.set(true);
                }
                let x = (lparam.0 & 0xFFFF) as i16 as f64;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f64;
                push_coalesced_event(
                    &mut data.events.borrow_mut(),
                    PlatformEvent::CursorMoved { x, y },
                );
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                data.tracking_leave.set(false);
                let mut pt = POINT::default();
                let mut rect = RECT::default();
                let inside_client = unsafe {
                    GetCursorPos(&mut pt).is_ok()
                        && ScreenToClient(hwnd, &mut pt).as_bool()
                        && GetClientRect(hwnd, &mut rect).is_ok()
                        && pt.x >= rect.left
                        && pt.x < rect.right
                        && pt.y >= rect.top
                        && pt.y < rect.bottom
                };
                if !inside_client {
                    data.events
                        .borrow_mut()
                        .push(PlatformEvent::CursorMoved { x: -1.0, y: -1.0 });
                }
                LRESULT(0)
            }
            WM_NCMOUSEMOVE => {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut tme) };
                let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
                let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let mut pt = POINT {
                    x: screen_x,
                    y: screen_y,
                };
                if unsafe { ScreenToClient(hwnd, &mut pt).as_bool() } {
                    data.events.borrow_mut().push(PlatformEvent::CursorMoved {
                        x: pt.x as f64,
                        y: pt.y as f64,
                    });
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_NCMOUSELEAVE => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::CursorMoved { x: -1.0, y: -1.0 });
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_LBUTTONDOWN => {
                let _ = unsafe { SetCapture(hwnd) };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Pressed,
                    button: MouseButton::Left,
                });
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = unsafe { ReleaseCapture() };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Released,
                    button: MouseButton::Left,
                });
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                let _ = unsafe { SetCapture(hwnd) };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Pressed,
                    button: MouseButton::Right,
                });
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let _ = unsafe { ReleaseCapture() };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Released,
                    button: MouseButton::Right,
                });
                LRESULT(0)
            }
            WM_MBUTTONDOWN => {
                let _ = unsafe { SetCapture(hwnd) };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Pressed,
                    button: MouseButton::Middle,
                });
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                let _ = unsafe { ReleaseCapture() };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Released,
                    button: MouseButton::Middle,
                });
                LRESULT(0)
            }
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let state = if msg == WM_XBUTTONDOWN {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                let xbutton = ((wparam.0 >> 16) & 0xFFFF) as u16;
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state,
                    button: MouseButton::from_win32_xbutton(xbutton),
                });
                LRESULT(1)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as f64 / 120.0;
                data.events.borrow_mut().push(PlatformEvent::MouseWheel {
                    delta_x: 0.0,
                    delta_y: delta,
                });
                LRESULT(0)
            }
            WM_MOUSEHWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as f64 / 120.0;
                data.events.borrow_mut().push(PlatformEvent::MouseWheel {
                    delta_x: delta,
                    delta_y: 0.0,
                });
                LRESULT(0)
            }
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
                let mods = current_modifiers();
                if mods != data.last_modifiers.get() {
                    data.last_modifiers.set(mods);
                    data.events
                        .borrow_mut()
                        .push(PlatformEvent::ModifiersChanged(mods));
                }
                let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let repeat = pressed && ((lparam.0 >> 30) & 1) != 0;
                let scan_code = ((lparam.0 >> 16) & 0xFF) as u32;
                let vk = (wparam.0 & 0xFFFF) as u16;
                let key = translate_vk(vk, scan_code);
                if pressed && ime_trace_enabled() {
                    // 输入法只在“焦点窗口 + 已关联输入上下文”时才投递 WM_IME_*，
                    // 这两个状态是排查组合事件缺失时最先要看的东西。
                    let focused = unsafe { GetFocus() } == hwnd;
                    let himc = unsafe { ImmGetContext(hwnd) };
                    let himc_valid = !himc.is_invalid();
                    if himc_valid {
                        let _ = unsafe { ImmReleaseContext(hwnd, himc) };
                    }
                    ime_trace(format!(
                        "key down vk=0x{vk:02X} key={key:?} focus={focused} himc={himc_valid}"
                    ));
                }
                if pressed && vk == VK_PROCESSKEY.0 {
                    // VK_PROCESSKEY 说明这个按键已被输入法吞掉并用于组合；
                    // 若输入法只更新了自己的组合窗、没发 WM_IME_COMPOSITION，
                    // 这里主动拉一次，预编辑就不会丢。
                    sync_ime_preedit(data, hwnd);
                }
                data.events.borrow_mut().push(PlatformEvent::KeyInput {
                    state: if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                    key,
                    repeat,
                });
                // 必须交由 DefWindowProcW 处理 WM_SYSKEYDOWN / WM_SYSKEYUP，
                // 才能保障 Alt+F4 -> SC_CLOSE -> WM_CLOSE 与 Alt+Space 系统菜单正常触发。
                if msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP {
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                LRESULT(0)
            }
            WM_CHAR | WM_SYSCHAR => {
                // Alt+F4 等系统字符交由 DefWindowProcW
                if msg == WM_SYSCHAR {
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                LRESULT(0)
            }
            WM_IME_SETCONTEXT => {
                // 组合串最终由应用自己画进输入框，但“谁来画”这一步要等确认
                // 组合串真的能拿到之后再切换：先保持输入法自带组合窗，
                // 避免自绘还没生效时连输入法自己的提示也一起消失。
                ime_trace(format!(
                    "ime setcontext active={} lparam=0x{:X}",
                    wparam.0, lparam.0
                ));
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_IME_REQUEST => {
                // 自绘组合串的应用必须回答“第 N 个字符在哪”，否则候选窗无法锚定到插入符，
                // 输入法会反复重问（表现为 WM_IME_REQUEST 高频循环）。
                // pt / rcDocument 都是屏幕坐标，而宿主上报的是客户区坐标。
                if wparam.0 == IMR_QUERYCHARPOSITION as usize {
                    if let Some((caret_x, caret_y, line_height)) =
                        crate::win32_window::win32::ime_caret_client()
                    {
                        let mut origin = POINT {
                            x: caret_x,
                            y: caret_y,
                        };
                        let mut top_left = POINT { x: 0, y: 0 };
                        let mut client = RECT::default();
                        unsafe {
                            let _ = ClientToScreen(hwnd, &mut origin);
                            let _ = ClientToScreen(hwnd, &mut top_left);
                            let _ = GetClientRect(hwnd, &mut client);
                        }
                        let info = unsafe { &mut *(lparam.0 as *mut IMECHARPOSITION) };
                        info.dwSize = core::mem::size_of::<IMECHARPOSITION>() as u32;
                        info.pt = origin;
                        info.cLineHeight = line_height;
                        info.rcDocument = RECT {
                            left: top_left.x,
                            top: top_left.y,
                            right: top_left.x + client.right,
                            bottom: top_left.y + client.bottom,
                        };
                        return LRESULT(1);
                    }
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_IME_STARTCOMPOSITION => {
                if ime_trace_enabled() {
                    ime_trace(format!(
                        "ime start ime={:?} caret={:?}",
                        ime_description(),
                        crate::win32_window::win32::ime_caret_client()
                    ));
                }
                // 组合开始时输入法会重建自己的 UI，必须再报一次锚点。
                crate::win32_window::win32::reapply_ime_area(hwnd.0 as isize);
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::Ime(ImeEvent::Enabled));
                sync_ime_preedit(data, hwnd);
                // 即便组合串由我们自绘，也必须走默认处理：默认 IME 窗口负责建立
                // 组合状态，之后 CUAS 才有“组合中”的镜像可镜像。
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_IME_NOTIFY => {
                // 通知里最要紧的是“候选窗/组合窗变了”，说明组合串也刚更新过。
                // 有些输入法（Win11 微软拼音）只发通知不发 WM_IME_COMPOSITION，
                // 这里顺手拉一次组合状态，把漏掉的预编辑补回来。
                sync_ime_preedit(data, hwnd);
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_IME_COMPOSITION => {
                ime_trace(format!("ime composition flags=0x{:08X}", lparam.0));
                let himc = unsafe { ImmGetContext(hwnd) };
                if !himc.is_invalid() {
                    let flags = lparam.0 as u32;
                    if flags == 0 {
                        // 没有任何 GCS_ 标志表示本次组合被取消，必须清掉残留的预编辑。
                        mark_ime_preedit_delivered("");
                        data.events
                            .borrow_mut()
                            .push(PlatformEvent::Ime(ImeEvent::Preedit(String::new(), None)));
                    }
                    if (flags & GCS_RESULTSTR.0) != 0 {
                        if let Some(commit) = imm_string(himc, GCS_RESULTSTR) {
                            ime_trace(format!("ime result {commit:?}"));
                            // 这是全应用唯一的提交通道（ADR 0011）：每条
                            // WM_IME_COMPOSITION 携带的结果就是一次真实上屏，
                            // 不做任何去重——去重反而会吞掉连续输入的同词。
                            mark_ime_preedit_delivered("");
                            data.events
                                .borrow_mut()
                                .push(PlatformEvent::Ime(ImeEvent::Commit(commit)));
                        }
                    }
                    if (flags & GCS_COMPSTR.0) != 0 {
                        let preedit = imm_string(himc, GCS_COMPSTR).unwrap_or_default();
                        ime_trace(format!("ime preedit {preedit:?} flags=0x{flags:08X}"));
                        let cursor =
                            unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0) };
                        let sel = if cursor >= 0 {
                            let byte_pos = preedit
                                .char_indices()
                                .nth(cursor as usize)
                                .map_or(preedit.len(), |(idx, _)| idx);
                            Some((byte_pos, byte_pos))
                        } else {
                            None
                        };
                        mark_ime_preedit_delivered(&preedit);
                        data.events
                            .borrow_mut()
                            .push(PlatformEvent::Ime(ImeEvent::Preedit(preedit, sel)));
                    }
                    let _ = unsafe { ImmReleaseContext(hwnd, himc) };
                }
                // 关键：把消息继续交给默认 IME 窗口。它负责按 lParam 的标志更新
                // IMM32 组合状态；不交还的话，CUAS 的镜像永远停在空串，
                // 后续按键也就只剩“上屏结果”而没有“组合中”的中间态。
                let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                // 组合串有可能是在默认 IME 窗口处理过程中才写进输入上下文的，
                // 所以处理完再补读一次；内容没变时会被预编辑去重挡掉。
                sync_ime_preedit(data, hwnd);
                result
            }
            WM_IME_ENDCOMPOSITION => {
                ime_trace("ime end");
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::Ime(ImeEvent::Disabled));
                // 组合结束：清理残留预编辑（若已提交，提交串在上面已经补发过）。
                sync_ime_preedit(data, hwnd);
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_DROPFILES => {
                let hdrop = HDROP(wparam.0 as *mut c_void);
                let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
                for i in 0..count {
                    let len = unsafe { DragQueryFileW(hdrop, i, None) };
                    if len > 0 {
                        let mut buf = vec![0u16; (len + 1) as usize];
                        let copied = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
                        if copied > 0 {
                            let path = String::from_utf16_lossy(&buf[..copied as usize]);
                            data.events
                                .borrow_mut()
                                .push(PlatformEvent::FileDrop(PathBuf::from(path)));
                        }
                    }
                }
                unsafe { DragFinish(hdrop) };
                LRESULT(0)
            }
            WM_APP_WAKE => {
                data.redraw_requested.store(true, Ordering::Release);
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    struct WindowEntry {
        window: NativeWindow,
        state: PlatformState,
        a11y: PlatformA11y,
        user_data: Rc<WindowUserData>,
        hwnd: HWND,
    }

    fn create_native_window(
        class_name: PCWSTR,
        hinstance: HINSTANCE,
        config: &WindowConfig,
    ) -> Result<WindowEntry, PlatformError> {
        let style = WINDOW_STYLE(compute_window_style(config));
        let (eff_min_w, eff_min_h) = config.effective_min_size();

        let sys_dpi = unsafe { GetDpiForSystem() };
        let init_scale = if sys_dpi == 0 {
            1.0
        } else {
            f64::from(sys_dpi) / 96.0
        };
        let init_logical_w = config.width.max(eff_min_w).max(1);
        let init_logical_h = config.height.max(eff_min_h).max(1);
        let init_phys_w = (f64::from(init_logical_w) * init_scale).round().max(1.0) as i32;
        let init_phys_h = (f64::from(init_logical_h) * init_scale).round().max(1.0) as i32;

        // 依据位置策略确定初始探测坐标点，使隐藏态窗口落在目标显示器上：
        // - 主屏幕左上角恒为 (0, 0)；
        // - CenterActiveMonitor 先采样当前鼠标指针屏幕坐标；
        // - 自定义坐标先落到对应坐标点所在显示器。
        let (seed_x, seed_y) = match config.position {
            WindowPosition::CenterPrimary => (0, 0),
            WindowPosition::CenterActiveMonitor => {
                let mut cursor_pt = POINT::default();
                if unsafe { GetCursorPos(&mut cursor_pt) }.is_ok() {
                    (cursor_pt.x, cursor_pt.y)
                } else {
                    (0, 0)
                }
            }
            WindowPosition::Physical { x, y } => (x, y),
            WindowPosition::Logical { x, y } => (
                (f64::from(x) * init_scale).round() as i32,
                (f64::from(y) * init_scale).round() as i32,
            ),
            WindowPosition::SystemDefault => {
                if config.decorations {
                    (CW_USEDEFAULT, CW_USEDEFAULT)
                } else {
                    (0, 0)
                }
            }
        };

        let title_wide: Vec<u16> = config
            .title
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();

        let redraw_requested = Arc::new(AtomicBool::new(true));
        let destroyed = Arc::new(AtomicBool::new(false));
        let cursor = Arc::new(AtomicU8::new(CursorIcon::Default as u8));
        let user_data = Rc::new(WindowUserData {
            decorations: config.decorations,
            resizable: config.resizable,
            is_popup: false,
            min_width: eff_min_w,
            min_height: eff_min_h,
            frame_extra_w: std::cell::Cell::new(0),
            frame_extra_h: std::cell::Cell::new(0),
            cursor: Arc::clone(&cursor),
            tracking_leave: std::cell::Cell::new(false),
            minimized: std::cell::Cell::new(false),
            last_modifiers: std::cell::Cell::new(Modifiers::default()),
            events: RefCell::new(Vec::new()),
            redraw_requested: Arc::clone(&redraw_requested),
            destroyed: Arc::clone(&destroyed),
        });

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_APPWINDOW,
                class_name,
                PCWSTR(title_wide.as_ptr()),
                style,
                seed_x,
                seed_y,
                init_phys_w,
                init_phys_h,
                None,
                None,
                Some(hinstance),
                Some(Rc::as_ptr(&user_data).cast::<c_void>()),
            )
        }
        .map_err(|err| PlatformError::Window(format!("CreateWindowExW failed: {err}")))?;

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&user_data) as isize);
            DragAcceptFiles(hwnd, true);
        }

        if !config.decorations {
            // 无边框窗口经 WM_NCCALCSIZE 去除非客户区后，通过扩展 1px 顶边帧恢复 DWM 阴影与圆角。
            crate::win32_frame::apply_borderless_dwm(hwnd.0 as isize);
        }

        // 在窗口显示前，查询目标显示器的真实工作区（扣除任务栏）与真实 DPI 缩放因子，
        // 计算精确的居中或自定义坐标以及物理宽高并一次性应用。
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut mi = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor_scale = crate::win32_window::win32::scale_factor(hwnd.0 as isize);
        let work_area = if unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
            (
                mi.rcWork.left,
                mi.rcWork.top,
                mi.rcWork.right,
                mi.rcWork.bottom,
            )
        } else {
            (
                0,
                0,
                (1920.0 * monitor_scale).round() as i32,
                (1080.0 * monitor_scale).round() as i32,
            )
        };

        // 对带原生装饰的窗口，测算外框相对于客户区的非客户区边框差值，保证客户区物理尺寸严格等于请求值。
        let (extra_w, extra_h) = if config.decorations {
            let mut client_rc = RECT::default();
            if unsafe { GetClientRect(hwnd, &mut client_rc) }.is_ok() {
                let cw = (client_rc.right - client_rc.left).max(1);
                let ch = (client_rc.bottom - client_rc.top).max(1);
                ((init_phys_w - cw).max(0), (init_phys_h - ch).max(0))
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };
        user_data.frame_extra_w.set(extra_w);
        user_data.frame_extra_h.set(extra_h);

        let placement = resolve_window_placement(config, work_area, monitor_scale);
        let outer_w = placement.width + extra_w;
        let outer_h = placement.height + extra_h;
        let (final_x, final_y) = match config.position {
            WindowPosition::CenterPrimary | WindowPosition::CenterActiveMonitor => {
                let work_w = (work_area.2 - work_area.0).max(1);
                let work_h = (work_area.3 - work_area.1).max(1);
                (
                    work_area.0 + ((work_w - outer_w) / 2).max(0),
                    work_area.1 + ((work_h - outer_h) / 2).max(0),
                )
            }
            _ => (placement.x, placement.y),
        };

        if !(config.decorations && matches!(config.position, WindowPosition::SystemDefault)) {
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    final_x,
                    final_y,
                    outer_w,
                    outer_h,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        } else if outer_w != init_phys_w || outer_h != init_phys_h {
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    outer_w,
                    outer_h,
                    ::windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | SWP_NOZORDER
                        | SWP_NOACTIVATE
                        | SWP_FRAMECHANGED,
                );
            }
        }

        // 清空隐藏定位阶段产生的中间尺寸消息，使应用在 resumed 时直接读取最终稳定尺寸。
        user_data.events.borrow_mut().clear();

        let window = NativeWindow::from_win32(Window::new(
            hwnd.0 as isize,
            hinstance.0 as isize,
            config.decorations,
            cursor,
            redraw_requested,
            destroyed,
        ));
        let a11y = PlatformA11y::for_win32(hwnd.0 as isize);
        if config.visible {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
        let size = window.inner_size();
        let mut state = PlatformState::new(
            config,
            Extent::new(size.width, size.height),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.scale_factor = window.scale_factor();
        state.visible = window.is_visible().unwrap_or(true);

        Ok(WindowEntry {
            window,
            state,
            a11y,
            user_data,
            hwnd,
        })
    }

    /// 查询指定屏幕矩形所在显示器的工作区并按配置执行越界裁剪。
    fn resolve_popup_rect(
        config: &crate::popup_host::PopupWindowConfig,
    ) -> crate::window::ScreenRect {
        if !config.clamp_to_work_area {
            return config.screen_rect;
        }
        let anchor = POINT {
            x: config.screen_rect.x,
            y: config.screen_rect.y,
        };
        let monitor = unsafe { MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST) };
        let mut mi = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
            let work_area = crate::window::ScreenRect::new(
                mi.rcWork.left,
                mi.rcWork.top,
                (mi.rcWork.right - mi.rcWork.left).max(0) as u32,
                (mi.rcWork.bottom - mi.rcWork.top).max(0) as u32,
            );
            crate::popup_host::clamp_popup_to_work_area(config.screen_rect, work_area)
        } else {
            config.screen_rect
        }
    }

    /// 创建无边框、无激活（`WS_POPUP | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`）的原生弹出窗。
    fn create_native_popup_window(
        class_name: PCWSTR,
        hinstance: HINSTANCE,
        owner_hwnd: Option<HWND>,
        popup_id: u64,
        config: &crate::popup_host::PopupWindowConfig,
    ) -> Result<WindowEntry, PlatformError> {
        let rect = resolve_popup_rect(config);
        let width = rect.width.max(1) as i32;
        let height = rect.height.max(1) as i32;

        let redraw_requested = Arc::new(AtomicBool::new(true));
        let destroyed = Arc::new(AtomicBool::new(false));
        let cursor = Arc::new(AtomicU8::new(CursorIcon::Default as u8));
        let user_data = Rc::new(WindowUserData {
            decorations: false,
            resizable: false,
            is_popup: true,
            min_width: 0,
            min_height: 0,
            frame_extra_w: std::cell::Cell::new(0),
            frame_extra_h: std::cell::Cell::new(0),
            cursor: Arc::clone(&cursor),
            tracking_leave: std::cell::Cell::new(false),
            minimized: std::cell::Cell::new(false),
            last_modifiers: std::cell::Cell::new(Modifiers::default()),
            events: RefCell::new(Vec::new()),
            redraw_requested: Arc::clone(&redraw_requested),
            destroyed: Arc::clone(&destroyed),
        });

        let style = WINDOW_STYLE(config.win32_style_bits());
        let ex_style = WINDOW_EX_STYLE(config.win32_ex_style_bits());
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class_name,
                w!(""),
                style,
                rect.x,
                rect.y,
                width,
                height,
                owner_hwnd,
                None,
                Some(hinstance),
                Some(Rc::as_ptr(&user_data).cast::<c_void>()),
            )
        }
        .map_err(|err| PlatformError::Window(format!("CreateWindowExW popup failed: {err}")))?;

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&user_data) as isize);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }

        user_data.events.borrow_mut().clear();

        let window = NativeWindow::from_win32(
            Window::new(
                hwnd.0 as isize,
                hinstance.0 as isize,
                false,
                cursor,
                redraw_requested,
                destroyed,
            )
            .with_popup_id(popup_id),
        );
        let a11y = PlatformA11y::for_win32(hwnd.0 as isize);
        let popup_win_cfg = WindowConfig {
            title: String::new(),
            width: rect.width.max(1),
            height: rect.height.max(1),
            resizable: false,
            decorations: false,
            min_width: 1,
            min_height: 1,
            visible: true,
            ..WindowConfig::default()
        };
        let mut state = PlatformState::new(
            &popup_win_cfg,
            Extent::new(rect.width.max(1), rect.height.max(1)),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.scale_factor = window.scale_factor();
        state.visible = true;
        state.focused = false;

        Ok(WindowEntry {
            window,
            state,
            a11y,
            user_data,
            hwnd,
        })
    }

    /// 重定位已有的原生弹出窗并保持无激活语义。
    fn reposition_native_popup_window(hwnd: HWND, config: &crate::popup_host::PopupWindowConfig) {
        let rect = resolve_popup_rect(config);
        let insert_after = if config.topmost {
            Some(HWND_TOPMOST)
        } else {
            None
        };
        let flags = if config.topmost {
            SWP_NOACTIVATE
        } else {
            SWP_NOACTIVATE | SWP_NOZORDER
        };
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                insert_after,
                rect.x,
                rect.y,
                rect.width.max(1) as i32,
                rect.height.max(1) as i32,
                flags,
            );
        }
    }

    /// UI 线程的 COM/OLE 作用域。
    ///
    /// 可选地在本线程初始化 OLE/COM。
    ///
    /// TSF（文本服务框架）以单线程套间（STA）为前提。
    ///
    /// 这是 Windows 输入法链路里最容易被忽略的一条硬性要求：TSF 的线程管理器、
    /// CUAS 兼容层（负责把 TSF 输入法的组合串翻译成 IMM32 的 `WM_IME_*` 消息）
    /// 都必须在 STA 线程上激活。线程未经 `OleInitialize` / `CoInitializeEx(APT)`
    /// 初始化 COM 时，`msctf.dll` 虽然已被 `user32` 加载进进程，却无法真正接管输入：
    ///
    /// * 按键仍会被输入法吞掉，应用只看到 `WM_KEYDOWN` + `VK_PROCESSKEY`；
    /// * 组合串被输入法画在它自己的窗口里（所以能看到候选框和拼音）；
    /// * `TranslateMessage` 无法生成 `WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION`；
    /// * `ImmGetCompositionString(GCS_COMPSTR)` 永远返回空；
    /// * 输入法还会反复用 `IMR_QUERYCHARPOSITION` 询问插入符位置（因为它想内联绘制）。
    ///
    /// 本窗口是 TSF-unaware 的 IMM32 应用，正确做法是：只做 COM STA 初始化，
    /// **不要**自己创建 `ITfThreadMgr`（那会把进程标记为 TSF-aware，反而关闭 CUAS
    /// 兼容层，此后必须自己实现 `ITextStoreACP` 才有输入）。初始化后 CUAS 会把
    /// TSF 输入法的组合过程翻译成标准的 `WM_IME_*` 消息，正好对应 ADR 0011
    /// “组合串即缓冲区文本”的设计。
    ///
    /// 成功初始化后必须配对反初始化，否则退出时可能残留 COM 引用。
    struct OleScope {
        initialized: bool,
    }

    impl OleScope {
        fn enter() -> Self {
            // SAFETY: 必须在创建窗口、泵消息的同一线程、且在输入法首次激活之前调用。
            let result = unsafe { OleInitialize(None) };
            let initialized = result.is_ok();
            let code = result.as_ref().err().map_or(0, |err| err.code().0);
            ime_trace(format!(
                "OleInitialize hr=0x{code:08X} ok={initialized} (0x80010106=thread already STA)"
            ));
            Self { initialized }
        }
    }

    impl Drop for OleScope {
        fn drop(&mut self) {
            if self.initialized {
                unsafe { OleUninitialize() };
            }
        }
    }

    pub(super) fn run_event_loop<A: PlatformApp>(
        config: WindowConfig,
        mut app: A,
    ) -> Result<(), PlatformError> {
        // 必须在注册窗口类与创建窗口之前完成 STA 初始化：
        // 输入法会在窗口首次激活时就接管该线程的输入。
        let _ole = OleScope::enter();
        let hmodule = unsafe { GetModuleHandleW(None) }
            .map_err(|err| PlatformError::EventLoop(format!("GetModuleHandleW failed: {err}")))?;
        let hinstance = HINSTANCE(hmodule.0);
        let class_name = w!("UnboundUiWindowClass");
        let default_cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();

        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: Default::default(),
            hCursor: default_cursor,
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: Default::default(),
        };
        unsafe { RegisterClassExW(&wc) };

        let primary_entry = create_native_window(class_name, hinstance, &config)?;
        let mut primary_id = primary_entry.window.id();
        let primary_hwnd_atomic = Arc::new(AtomicIsize::new(primary_entry.window.raw_win32_hwnd()));

        let wake_target = Arc::clone(&primary_hwnd_atomic);
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let hwnd = wake_target.load(Ordering::Acquire);
            if hwnd != 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut c_void)),
                        WM_APP_WAKE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });

        let mut windows = vec![primary_entry];
        let mut popup_registry = crate::popup_host::PopupHostRegistry::new();
        let mut popup_windows: Vec<(u64, WindowEntry)> = Vec::new();
        app.attach_waker(waker);
        let first = &mut windows[0];
        app.resumed(&mut first.state, &first.window);

        let mut msg = MSG::default();
        let high_res_timer = HighResWaitableTimer::try_create();
        let mut last_occlusion_probe = Instant::now();
        const OCCLUSION_PROBE_INTERVAL: Duration = Duration::from_millis(250);

        'main_loop: while !windows.is_empty() {
            let mut modal_exit = false;
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_QUIT {
                    break 'main_loop;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                }
                {
                    let mut modal_step = |target_hwnd: HWND| {
                        if modal_exit {
                            return;
                        }
                        let Some(entry) = windows.iter_mut().find(|e| e.hwnd == target_hwnd) else {
                            return;
                        };
                        let pending: Vec<PlatformEvent> =
                            entry.user_data.events.borrow_mut().drain(..).collect();
                        if pending.is_empty() {
                            return;
                        }
                        let now = Instant::now();
                        for platform_event in pending {
                            if matches!(platform_event, PlatformEvent::CloseRequested) {
                                entry.user_data.events.borrow_mut().push(platform_event);
                                return;
                            }
                            for out in apply(&mut entry.state, &platform_event, now) {
                                if app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                                    modal_exit = true;
                                    return;
                                }
                            }
                        }
                        entry.state.pacer.request_immediate(now);
                        if app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                            modal_exit = true;
                            return;
                        }
                        if pump_a11y(&mut app, &mut entry.a11y, &mut entry.state, &entry.window)
                            == Flow::Exit
                        {
                            modal_exit = true;
                        }
                    };
                    let _guard = ModalDispatchGuard::install(&mut modal_step);
                    unsafe {
                        DispatchMessageW(&msg);
                    }
                }
                if modal_exit {
                    break 'main_loop;
                }
            }

            let mut closing_indices = Vec::new();
            for (index, entry) in windows.iter_mut().enumerate() {
                let events: Vec<PlatformEvent> =
                    entry.user_data.events.borrow_mut().drain(..).collect();

                for platform_event in events {
                    let now = Instant::now();
                    if matches!(platform_event, PlatformEvent::CloseRequested) {
                        let flow = app.event(&mut entry.state, &entry.window, platform_event);
                        if flow == Flow::Exit {
                            break 'main_loop;
                        }
                        closing_indices.push(index);
                        break;
                    }

                    for out in apply(&mut entry.state, &platform_event, now) {
                        if app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                            break 'main_loop;
                        }
                    }
                }
            }

            // 当宿主窗口最小化或隐藏时，级联关闭其名下所有原生弹出窗并发出 PopupDismissed。
            for entry in &mut windows {
                if entry.state.minimized || !entry.state.visible {
                    let owner_id = crate::windows::WindowId(entry.window.raw_win32_hwnd() as u64);
                    let dismissed = popup_registry.close_for_owner(owner_id);
                    for action in dismissed {
                        if let crate::popup_host::PopupSyncAction::Close { id, .. } = action {
                            if let Some(pos) = popup_windows.iter().position(|(pid, _)| *pid == id)
                            {
                                let (_, popup_entry) = popup_windows.remove(pos);
                                app.window_closing(&popup_entry.window);
                                if !popup_entry.window.is_destroyed() {
                                    let _ = unsafe { DestroyWindow(popup_entry.hwnd) };
                                }
                            }
                            let now = Instant::now();
                            for out in
                                apply(&mut entry.state, &PlatformEvent::PopupDismissed { id }, now)
                            {
                                if app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                                    break 'main_loop;
                                }
                            }
                        }
                    }
                }
            }

            // 派发所有原生弹出窗收到的指针/滚轮等平台事件。
            for (_, popup_entry) in &mut popup_windows {
                let events: Vec<PlatformEvent> = popup_entry
                    .user_data
                    .events
                    .borrow_mut()
                    .drain(..)
                    .collect();
                for platform_event in events {
                    let now = Instant::now();
                    for out in apply(&mut popup_entry.state, &platform_event, now) {
                        if app.event(&mut popup_entry.state, &popup_entry.window, out) == Flow::Exit
                        {
                            break 'main_loop;
                        }
                    }
                }
            }

            for index in closing_indices.into_iter().rev() {
                let entry = windows.remove(index);
                let closed_was_primary = entry.window.id() == primary_id;
                let owner_id = crate::windows::WindowId(entry.window.raw_win32_hwnd() as u64);
                for action in popup_registry.close_for_owner(owner_id) {
                    if let crate::popup_host::PopupSyncAction::Close { id, .. } = action {
                        if let Some(pos) = popup_windows.iter().position(|(pid, _)| *pid == id) {
                            let (_, popup_entry) = popup_windows.remove(pos);
                            app.window_closing(&popup_entry.window);
                            if !popup_entry.window.is_destroyed() {
                                let _ = unsafe { DestroyWindow(popup_entry.hwnd) };
                            }
                        }
                    }
                }
                app.window_closing(&entry.window);
                if !entry.window.is_destroyed() {
                    let _ = unsafe { DestroyWindow(entry.hwnd) };
                }
                if closed_was_primary {
                    if let Some(next_primary) = windows.first() {
                        primary_id = next_primary.window.id();
                        primary_hwnd_atomic
                            .store(next_primary.window.raw_win32_hwnd(), Ordering::Release);
                    } else {
                        primary_hwnd_atomic.store(0, Ordering::Release);
                    }
                }
            }

            if windows.is_empty() {
                break;
            }

            let probe_now = Instant::now();
            if probe_now.saturating_duration_since(last_occlusion_probe) >= OCCLUSION_PROBE_INTERVAL
            {
                last_occlusion_probe = probe_now;
                for entry in &mut windows {
                    let occluded = probe_window_occluded(entry.hwnd);
                    if occluded != entry.state.occluded {
                        for out in apply(
                            &mut entry.state,
                            &PlatformEvent::Occluded(occluded),
                            probe_now,
                        ) {
                            if app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                                break 'main_loop;
                            }
                        }
                    }
                }
            }

            for entry in &mut windows {
                if app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                    break 'main_loop;
                }
                if pump_a11y(&mut app, &mut entry.a11y, &mut entry.state, &entry.window)
                    == Flow::Exit
                {
                    break 'main_loop;
                }
            }

            for (_, popup_entry) in &mut popup_windows {
                if app.frame(&mut popup_entry.state, &popup_entry.window) == Flow::Exit {
                    break 'main_loop;
                }
            }

            // 响应应用侧的原生弹出窗关闭与同步（创建/重定位）请求。
            for close_id in app.take_popup_close_requests() {
                if let Some(crate::popup_host::PopupSyncAction::Close { id, .. }) =
                    popup_registry.close_popup(close_id)
                {
                    if let Some(pos) = popup_windows.iter().position(|(pid, _)| *pid == id) {
                        let (_, popup_entry) = popup_windows.remove(pos);
                        app.window_closing(&popup_entry.window);
                        if !popup_entry.window.is_destroyed() {
                            let _ = unsafe { DestroyWindow(popup_entry.hwnd) };
                        }
                    }
                }
            }

            for (popup_id, popup_cfg) in app.take_popup_sync_requests() {
                let Some(action) = popup_registry.sync_popup(popup_id, popup_cfg) else {
                    continue;
                };
                match action {
                    crate::popup_host::PopupSyncAction::Create { id, config, .. } => {
                        let owner_hwnd = windows
                            .iter()
                            .find(|w| {
                                crate::windows::WindowId(w.window.raw_win32_hwnd() as u64)
                                    == config.owner
                            })
                            .or_else(|| windows.first())
                            .map(|w| w.hwnd);
                        if let Ok(mut popup_entry) = create_native_popup_window(
                            class_name, hinstance, owner_hwnd, id, &config,
                        ) {
                            let popup_win_id = crate::windows::WindowId(
                                popup_entry.window.raw_win32_hwnd() as u64,
                            );
                            let _ = popup_registry.bind_native_window(id, popup_win_id);
                            app.window_created(&mut popup_entry.state, &popup_entry.window);
                            popup_windows.push((id, popup_entry));
                        }
                    }
                    crate::popup_host::PopupSyncAction::Reposition { id, config, .. } => {
                        if let Some((_, popup_entry)) =
                            popup_windows.iter().find(|(pid, _)| *pid == id)
                        {
                            reposition_native_popup_window(popup_entry.hwnd, &config);
                        }
                    }
                    crate::popup_host::PopupSyncAction::Close { id, .. } => {
                        if let Some(pos) = popup_windows.iter().position(|(pid, _)| *pid == id) {
                            let (_, popup_entry) = popup_windows.remove(pos);
                            app.window_closing(&popup_entry.window);
                            if !popup_entry.window.is_destroyed() {
                                let _ = unsafe { DestroyWindow(popup_entry.hwnd) };
                            }
                        }
                    }
                }
            }
            popup_windows.retain(|(_, entry)| !entry.window.is_destroyed());

            while let Some(extra_config) = app.take_window_request() {
                let Ok(mut entry) = create_native_window(class_name, hinstance, &extra_config)
                else {
                    break;
                };
                app.window_created(&mut entry.state, &entry.window);
                windows.push(entry);
            }

            // 响应应用侧显式关闭窗口请求，销毁对应原生 HWND 并级联回收其名下原生弹出窗。
            for close_id in app.take_window_close_requests() {
                if let Some(pos) = windows.iter().position(|e| {
                    crate::windows::WindowId(e.window.raw_win32_hwnd() as u64) == close_id
                }) {
                    let entry = windows.remove(pos);
                    let closed_was_primary = entry.window.id() == primary_id;
                    for action in popup_registry.close_for_owner(close_id) {
                        if let crate::popup_host::PopupSyncAction::Close { id, .. } = action {
                            if let Some(ppos) = popup_windows.iter().position(|(pid, _)| *pid == id)
                            {
                                let (_, popup_entry) = popup_windows.remove(ppos);
                                app.window_closing(&popup_entry.window);
                                if !popup_entry.window.is_destroyed() {
                                    let _ = unsafe { DestroyWindow(popup_entry.hwnd) };
                                }
                            }
                        }
                    }
                    app.window_closing(&entry.window);
                    if !entry.window.is_destroyed() {
                        let _ = unsafe { DestroyWindow(entry.hwnd) };
                    }
                    if closed_was_primary {
                        if let Some(next_primary) = windows.first() {
                            primary_id = next_primary.window.id();
                            primary_hwnd_atomic
                                .store(next_primary.window.raw_win32_hwnd(), Ordering::Release);
                        } else {
                            primary_hwnd_atomic.store(0, Ordering::Release);
                        }
                    }
                }
            }

            windows.retain(|entry| !entry.window.is_destroyed());
            if windows.is_empty() {
                break;
            }
            if !windows.iter().any(|e| e.window.id() == primary_id) {
                if let Some(next_primary) = windows.first() {
                    primary_id = next_primary.window.id();
                    primary_hwnd_atomic
                        .store(next_primary.window.raw_win32_hwnd(), Ordering::Release);
                }
            }

            let any_redraw = windows
                .iter()
                .any(|e| e.user_data.redraw_requested.swap(false, Ordering::AcqRel));
            if any_redraw {
                continue;
            }

            let now = Instant::now();
            let any_active_animating = windows
                .iter()
                .any(|e| !e.state.pacer.is_suspended() && e.state.pacer.animating());
            let platform_deadline = windows
                .iter()
                .filter(|e| !e.state.pacer.is_suspended())
                .filter_map(|e| e.state.pacer.next_wake(now))
                .min();
            let deadline = match (platform_deadline, app.next_wake()) {
                (Some(frame), Some(timer)) => Some(frame.min(timer)),
                (frame, timer) => frame.or(timer),
            };
            let wait = match deadline {
                Some(t) => Some(t.saturating_duration_since(now)),
                None if any_active_animating => continue,
                None => None,
            };
            wait_for_events_or_deadline(high_res_timer.as_ref(), wait);
        }

        primary_hwnd_atomic.store(0, Ordering::Release);
        for (_, popup_entry) in popup_windows.drain(..) {
            app.window_closing(&popup_entry.window);
            if !popup_entry.window.is_destroyed() {
                let _ = unsafe { DestroyWindow(popup_entry.hwnd) };
            }
        }
        for entry in windows.drain(..) {
            app.window_closing(&entry.window);
            if !entry.window.is_destroyed() {
                let _ = unsafe { DestroyWindow(entry.hwnd) };
            }
        }

        Ok(())
    }

    #[cfg(test)]
    mod ime_preedit_tests {
        use super::preedit_is_new;

        #[test]
        fn repeated_reads_of_the_same_composition_are_not_republished() {
            // 轮询会在每个通知/按键后重复读到同一份组合串。
            let mut last = String::new();
            assert!(preedit_is_new(&mut last, "ni"));
            assert!(!preedit_is_new(&mut last, "ni"));
            assert!(preedit_is_new(&mut last, "nih"));
            assert!(
                preedit_is_new(&mut last, ""),
                "a commit must clear the preedit held by the host"
            );
        }

        #[test]
        fn high_res_waitable_timer_arms_or_falls_back_cleanly() {
            use super::{wait_for_events_or_deadline, HighResWaitableTimer};
            use std::time::Duration;

            // 在支持的 Windows 10 1803+ 系统上应可创建并成功装填，零时长拒绝装填以触发回退；
            // 在不支持的系统上 `try_create` 返回 `None`，`wait_for_events_or_deadline` 优雅回退。
            if let Some(timer) = HighResWaitableTimer::try_create() {
                assert!(!timer.arm(Duration::ZERO));
                assert!(timer.arm(Duration::from_millis(1)));
                wait_for_events_or_deadline(Some(&timer), Some(Duration::from_micros(100)));
            } else {
                wait_for_events_or_deadline(None, Some(Duration::from_micros(100)));
            }
        }
    }
}

#[cfg(test)]
mod style_tests {
    use super::style_bits::*;
    use super::*;

    fn config(decorations: bool, resizable: bool) -> WindowConfig {
        WindowConfig {
            decorations,
            resizable,
            ..Default::default()
        }
    }

    #[test]
    fn decorated_windows_keep_the_system_caption() {
        let style = compute_window_style(&config(true, true));
        assert_ne!(style & WS_CAPTION, 0);
        assert_ne!(style & WS_THICKFRAME, 0);
    }

    #[test]
    fn borderless_windows_keep_overlapped_caption_for_dwm_animations() {
        // 无边框可缩放窗口必须保留 WS_CAPTION 与 WS_THICKFRAME（不使用 WS_POPUP），
        // 确保 DWM 保留最小化/最大化系统动画、阴影与 HTTOP 双击全高，非客户区由 WM_NCCALCSIZE 归零。
        let resizable_style = compute_window_style(&config(false, true));
        assert_ne!(resizable_style & WS_CAPTION, 0);
        assert_ne!(resizable_style & WS_THICKFRAME, 0);
        assert_eq!(resizable_style & WS_POPUP, 0);
        assert_ne!(resizable_style & WS_MINIMIZEBOX, 0);
        assert_ne!(resizable_style & WS_MAXIMIZEBOX, 0);

        // 不可缩放的无边框窗口保留 WS_CAPTION 以维持系统动画，去掉 WS_THICKFRAME 与 WS_MAXIMIZEBOX。
        let fixed_style = compute_window_style(&config(false, false));
        assert_ne!(fixed_style & WS_CAPTION, 0);
        assert_eq!(fixed_style & WS_THICKFRAME, 0);
        assert_eq!(fixed_style & WS_MAXIMIZEBOX, 0);
        assert_eq!(fixed_style & WS_POPUP, 0);
    }

    #[test]
    fn coalescing_move_events_retains_only_the_latest_position() {
        let mut queue = Vec::new();
        for x in 100..=150 {
            push_coalesced_event(&mut queue, PlatformEvent::Moved { x, y: x * 2 });
        }
        assert_eq!(
            queue,
            vec![PlatformEvent::Moved { x: 150, y: 300 }],
            "a burst of WM_MOVE messages must coalesce into a single final Moved event"
        );
    }

    #[test]
    fn coalescing_keeps_resize_and_move_events_independent() {
        use crate::resize::Extent;
        let mut queue = Vec::new();
        push_coalesced_event(&mut queue, PlatformEvent::Moved { x: 10, y: 20 });
        push_coalesced_event(&mut queue, PlatformEvent::Resized(Extent::new(800, 600)));
        push_coalesced_event(&mut queue, PlatformEvent::Moved { x: 30, y: 40 });
        push_coalesced_event(&mut queue, PlatformEvent::Resized(Extent::new(1024, 768)));
        assert_eq!(
            queue,
            vec![
                PlatformEvent::Moved { x: 30, y: 40 },
                PlatformEvent::Resized(Extent::new(1024, 768)),
            ],
            "move and resize events must each coalesce without swallowing the other"
        );
    }

    #[test]
    fn coalescing_resize_events_retains_only_the_latest_extent() {
        use crate::resize::Extent;
        let mut queue = Vec::new();
        for w in 1000..=1050 {
            push_coalesced_event(&mut queue, PlatformEvent::Resized(Extent::new(w, 700)));
        }
        assert_eq!(
            queue,
            vec![PlatformEvent::Resized(Extent::new(1050, 700))],
            "a burst of WM_SIZE messages must coalesce into a single final Resized event"
        );
    }

    #[test]
    fn relative_due_time_converts_duration_to_negative_100ns_units() {
        use std::time::Duration;
        assert_eq!(duration_to_win32_relative_due_time(Duration::ZERO), 0);
        assert_eq!(
            duration_to_win32_relative_due_time(Duration::from_nanos(50)),
            -1
        );
        assert_eq!(
            duration_to_win32_relative_due_time(Duration::from_micros(500)),
            -5_000
        );
        assert_eq!(
            duration_to_win32_relative_due_time(Duration::from_millis(8)),
            -80_000
        );
        assert_eq!(
            duration_to_win32_relative_due_time(Duration::MAX),
            -i64::MAX
        );
    }
}

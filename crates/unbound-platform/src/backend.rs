//! 原生 Win32 平台后端。
//!
//! 后端直接基于官方 `windows` crate（0.62.2）的 Win32 API（`RegisterClassExW`、
//! `CreateWindowExW`、`PeekMessageW`、`MsgWaitForMultipleObjectsEx`）驱动事件循环与
//! 多窗口生命周期，不再依赖手写 FFI 块或 `winit`。
//! 引擎层通过 [`PlatformEvent`] 与 [`PlatformState`] 消费所有平台输入。

use std::sync::Arc;
use std::time::Instant;

use crate::a11y_win32::Win32A11y;
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
    fn resumed(&mut self, state: &mut PlatformState, window: &Window);

    /// 发生了平台事件。
    fn event(&mut self, state: &mut PlatformState, window: &Window, event: PlatformEvent) -> Flow;

    /// 绘制一帧。
    ///
    /// 在事件循环空闲点调用，此时所有待处理事件已派发完毕。
    /// 应用在此获取交换链图像、执行渲染并提交呈现。
    fn frame(&mut self, state: &mut PlatformState, window: &Window) -> Flow;

    /// 窗口已隐藏；渲染器必须停止绘制。
    fn suspended(&mut self, state: &mut PlatformState) {
        let _ = state;
    }

    /// 待发布的无障碍语义增量（若应用发布了更新）。
    fn accessibility_update(&mut self) -> Option<accesskit::TreeUpdate> {
        None
    }

    /// 辅助技术请求执行某个动作。
    fn accessibility_action(
        &mut self,
        state: &mut PlatformState,
        window: &Window,
        request: crate::a11y_bridge::A11yRequest,
    ) -> Flow {
        let _ = (state, window, request);
        Flow::Continue
    }

    /// 辅助窗口已创建，可创建其独立渲染表面。
    fn window_created(&mut self, state: &mut PlatformState, window: &Window) {
        self.resumed(state, window);
    }

    /// 辅助窗口即将关闭；释放其相关资源。
    fn window_closing(&mut self, window: &Window) {
        let _ = window;
    }

    /// 请求平台打开另一个窗口。
    fn take_window_request(&mut self) -> Option<WindowConfig> {
        None
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
    config: WindowConfig,
}

impl PlatformWindow {
    /// 创建原生 Win32 平台窗口运行器。
    ///
    /// # Errors
    /// 在非 Windows 平台或系统初始化失败时返回 [`PlatformError`]。
    pub fn new(config: &WindowConfig) -> Result<Self, PlatformError> {
        #[cfg(windows)]
        {
            win32::enable_per_monitor_v2_dpi();
            Ok(Self {
                config: config.clone(),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = config;
            Err(PlatformError::EventLoop(
                "native Win32 backend requires Windows".to_string(),
            ))
        }
    }

    /// 运行 Win32 消息循环直至主窗口关闭或应用退出。
    ///
    /// # Errors
    /// 当窗口创建或事件循环失败时返回 [`PlatformError`]。
    pub fn run<A: PlatformApp>(self, app: A) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win32::run_event_loop(self.config, app)
        }
        #[cfg(not(windows))]
        {
            let _ = app;
            Err(PlatformError::EventLoop(
                "native Win32 backend requires Windows".to_string(),
            ))
        }
    }
}

/// 获取窗口的原始句柄，供 `vkCreateWin32SurfaceKHR` 使用。
#[must_use]
pub fn raw_window_handle(window: &Window) -> Option<raw_window_handle::RawWindowHandle> {
    use raw_window_handle::HasWindowHandle;
    window.window_handle().ok().map(|handle| handle.as_raw())
}

/// 排空入站无障碍动作并发布出站增量。
pub(crate) fn pump_a11y<A: PlatformApp>(
    app: &mut A,
    a11y: &mut Win32A11y,
    state: &mut PlatformState,
    window: &Window,
) -> Flow {
    for request in a11y.drain_requests() {
        if app.accessibility_action(state, window, request) == Flow::Exit {
            return Flow::Exit;
        }
    }
    a11y.publish_with(|| app.accessibility_update());
    Flow::Continue
}

/// Win32 窗口样式位常量（独立于目标平台暴露，便于跨平台单元测试验证样式选择策略）。
pub(crate) mod style_bits {
    pub const WS_OVERLAPPED: u32 = 0x0000_0000;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WS_CAPTION: u32 = 0x00C0_0000;
    pub const WS_SYSMENU: u32 = 0x0008_0000;
    pub const WS_THICKFRAME: u32 = 0x0004_0000;
    pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    pub const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    pub const WS_OVERLAPPEDWINDOW: u32 =
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
}

/// 依据窗口配置选择原生窗口样式。
///
/// - `decorations = true`：标准系统装饰窗口（`WS_OVERLAPPEDWINDOW` 或不含 `WS_THICKFRAME`）。
/// - `decorations = false`：去掉 `WS_CAPTION`（消除系统标题栏），而在 `resizable = true` 时
///   **保留 `WS_THICKFRAME`**：Win32 唯有在窗口具备 `WS_THICKFRAME` 时才响应 `WM_NCHITTEST`
///   的 `HTLEFT..HTBOTTOMRIGHT` 与 `WM_SYSCOMMAND(SC_SIZE + WMSZ_*)` 边缘拖拽缩放，以及
///   Windows 11 贴靠布局。配合 `WM_NCCALCSIZE` 返回 `0` 可将非客户区边框裁切为 0，
///   既无双层标题栏或系统白边，又完整保留原生四边/四角缩放能力。
#[must_use]
pub(crate) fn compute_window_style(config: &WindowConfig) -> u32 {
    use style_bits::*;
    if config.decorations {
        if config.resizable {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
        }
    } else if config.resizable {
        WS_POPUP | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
    } else {
        WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
    }
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

    use ::windows::core::{w, PCWSTR};
    use ::windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use ::windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use ::windows::Win32::UI::Controls::WM_MOUSELEAVE;
    use ::windows::Win32::UI::HiDpi::{
        GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use ::windows::Win32::UI::Input::Ime::{
        ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, GCS_COMPSTR, GCS_CURSORPOS,
        GCS_RESULTSTR, HIMC, IME_COMPOSITION_STRING,
    };
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, GetKeyboardState, ReleaseCapture, SetCapture, ToUnicode, TrackMouseEvent,
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, VK_APPS, VK_BACK, VK_CAPITAL, VK_CONTROL,
        VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN,
        VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
        VK_UP,
    };
    use ::windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetCursorPos, GetWindowLongPtrW, IsZoomed, LoadCursorW, MsgWaitForMultipleObjectsEx,
        PeekMessageW, PostMessageW, RegisterClassExW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_OWNDC, CS_VREDRAW, CW_USEDEFAULT,
        GWLP_USERDATA, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT, HTCLOSE, HTLEFT,
        HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IDC_ARROW, MINMAXINFO,
        MSG, MWMO_INPUTAVAILABLE, NCCALCSIZE_PARAMS, PM_REMOVE, QS_ALLINPUT, SWP_NOACTIVATE,
        SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, WINDOW_STYLE, WM_CHAR,
        WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_GETMINMAXINFO, WM_IME_COMPOSITION,
        WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCCREATE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
        WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
        WM_SETFOCUS, WM_SETTINGCHANGE, WM_SHOWWINDOW, WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN,
        WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_EX_APPWINDOW,
    };

    use crate::pacing::PresentWaitPolicy;
    use crate::resize::Extent;
    use crate::win32_window::{CursorIcon, WM_APP_WAKE};
    use crate::window::{
        apply, ButtonState, ImeEvent, Modifiers, MouseButton, PlatformEvent, PlatformKey,
    };

    const INFINITE_TIMEOUT: u32 = u32::MAX;

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

    struct WindowUserData {
        decorations: bool,
        resizable: bool,
        min_width: u32,
        min_height: u32,
        cursor: Arc<AtomicU8>,
        tracking_leave: std::cell::Cell<bool>,
        last_modifiers: std::cell::Cell<Modifiers>,
        events: RefCell<Vec<PlatformEvent>>,
        redraw_requested: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
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
                if data.min_width > 0 && data.min_height > 0 {
                    let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                    let scale = crate::win32_window::win32::scale_factor(hwnd.0 as isize);
                    info.ptMinTrackSize.x = (f64::from(data.min_width) * scale).round() as i32;
                    info.ptMinTrackSize.y = (f64::from(data.min_height) * scale).round() as i32;
                    return LRESULT(0);
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SIZE => {
                let width = (lparam.0 & 0xFFFF) as u32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                if width > 0 && height > 0 {
                    data.events
                        .borrow_mut()
                        .push(PlatformEvent::Resized(Extent::new(width, height)));
                    data.redraw_requested.store(true, Ordering::Release);
                }
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
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::CursorMoved { x, y });
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
                    button: MouseButton::Other(xbutton),
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
            WM_IME_STARTCOMPOSITION => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::Ime(ImeEvent::Enabled));
                LRESULT(0)
            }
            WM_IME_COMPOSITION => {
                let himc = unsafe { ImmGetContext(hwnd) };
                if !himc.is_invalid() {
                    let flags = lparam.0 as u32;
                    if (flags & GCS_RESULTSTR.0) != 0 {
                        if let Some(commit) = imm_string(himc, GCS_RESULTSTR) {
                            data.events
                                .borrow_mut()
                                .push(PlatformEvent::Ime(ImeEvent::Commit(commit)));
                        }
                    }
                    if (flags & GCS_COMPSTR.0) != 0 {
                        let preedit = imm_string(himc, GCS_COMPSTR).unwrap_or_default();
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
                        data.events
                            .borrow_mut()
                            .push(PlatformEvent::Ime(ImeEvent::Preedit(preedit, sel)));
                    }
                    let _ = unsafe { ImmReleaseContext(hwnd, himc) };
                }
                LRESULT(0)
            }
            WM_IME_ENDCOMPOSITION => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::Ime(ImeEvent::Disabled));
                LRESULT(0)
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
        window: Window,
        state: PlatformState,
        a11y: Win32A11y,
        user_data: Rc<WindowUserData>,
        hwnd: HWND,
    }

    fn create_native_window(
        class_name: PCWSTR,
        hinstance: HINSTANCE,
        config: &WindowConfig,
    ) -> Result<WindowEntry, PlatformError> {
        let style = WINDOW_STYLE(compute_window_style(config));

        let sys_dpi = unsafe { GetDpiForSystem() };
        let scale = if sys_dpi == 0 {
            1.0
        } else {
            f64::from(sys_dpi) / 96.0
        };
        let phys_w = (f64::from(config.width.max(1)) * scale).round() as i32;
        let phys_h = (f64::from(config.height.max(1)) * scale).round() as i32;
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
            min_width: config.min_width,
            min_height: config.min_height,
            cursor: Arc::clone(&cursor),
            tracking_leave: std::cell::Cell::new(false),
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
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                phys_w,
                phys_h,
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

        let window = Window::new(
            hwnd.0 as isize,
            hinstance.0 as isize,
            config.decorations,
            cursor,
            redraw_requested,
            destroyed,
        );
        let a11y = Win32A11y::new(hwnd.0 as isize);
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

    pub(super) fn run_event_loop<A: PlatformApp>(
        config: WindowConfig,
        mut app: A,
    ) -> Result<(), PlatformError> {
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
        let primary_id = primary_entry.window.id();
        let primary_hwnd_atomic = Arc::new(AtomicIsize::new(primary_entry.window.hwnd()));

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
        app.attach_waker(waker);
        let first = &mut windows[0];
        app.resumed(&mut first.state, &first.window);

        let mut msg = MSG::default();

        'main_loop: while !windows.is_empty() {
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                if msg.message == WM_QUIT {
                    break 'main_loop;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            let mut closing_indices = Vec::new();
            for (index, entry) in windows.iter_mut().enumerate() {
                let events: Vec<PlatformEvent> =
                    entry.user_data.events.borrow_mut().drain(..).collect();

                for platform_event in events {
                    let now = Instant::now();
                    if matches!(platform_event, PlatformEvent::CloseRequested) {
                        let is_primary = entry.window.id() == primary_id;
                        let _ = app.event(&mut entry.state, &entry.window, platform_event);
                        if is_primary {
                            break 'main_loop;
                        }
                        closing_indices.push(index);
                        break;
                    }

                    let is_resize = matches!(platform_event, PlatformEvent::Resized(_));
                    for out in apply(&mut entry.state, &platform_event, now) {
                        if app.event(&mut entry.state, &entry.window, out) == Flow::Exit {
                            break 'main_loop;
                        }
                    }

                    // 拖拽调整窗口大小期间 Windows 会进入模态循环，立即渲染新帧避免黑边拉伸
                    if is_resize {
                        entry.state.pacer.request_immediate(now);
                        if app.frame(&mut entry.state, &entry.window) == Flow::Exit {
                            break 'main_loop;
                        }
                        if pump_a11y(&mut app, &mut entry.a11y, &mut entry.state, &entry.window)
                            == Flow::Exit
                        {
                            break 'main_loop;
                        }
                    }
                }
            }

            for index in closing_indices.into_iter().rev() {
                let entry = windows.remove(index);
                app.window_closing(&entry.window);
                if !entry.window.is_destroyed() {
                    let _ = unsafe { DestroyWindow(entry.hwnd) };
                }
            }

            if windows.is_empty() {
                break;
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

            while let Some(extra_config) = app.take_window_request() {
                let Ok(mut entry) = create_native_window(class_name, hinstance, &extra_config)
                else {
                    break;
                };
                app.window_created(&mut entry.state, &entry.window);
                windows.push(entry);
            }

            windows.retain(|entry| !entry.window.is_destroyed());
            if windows.is_empty() {
                break;
            }

            let any_redraw = windows
                .iter()
                .any(|e| e.user_data.redraw_requested.swap(false, Ordering::AcqRel));
            if any_redraw {
                continue;
            }

            let Some(primary) = windows.iter().find(|e| e.window.id() == primary_id) else {
                break;
            };
            let now = Instant::now();
            let deadline = match (primary.state.pacer.next_wake(now), app.next_wake()) {
                (Some(frame), Some(timer)) => Some(frame.min(timer)),
                (frame, timer) => frame.or(timer),
            };
            let timeout_ms = match deadline {
                Some(t) => {
                    let wait = t.saturating_duration_since(now);
                    if wait <= Duration::from_millis(1) {
                        continue;
                    }
                    wait.as_millis().min(u128::from(INFINITE_TIMEOUT - 1)) as u32
                }
                None if primary.state.visible && primary.state.pacer.animating() => continue,
                None => INFINITE_TIMEOUT,
            };

            unsafe {
                let _ =
                    MsgWaitForMultipleObjectsEx(None, timeout_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }

        primary_hwnd_atomic.store(0, Ordering::Release);
        for entry in windows.drain(..) {
            app.window_closing(&entry.window);
            if !entry.window.is_destroyed() {
                let _ = unsafe { DestroyWindow(entry.hwnd) };
            }
        }

        Ok(())
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
    fn borderless_windows_drop_caption_and_preserve_thickframe_when_resizable() {
        // 无边框可缩放窗口必须去掉 WS_CAPTION（消除系统标题栏），但保留 WS_THICKFRAME
        // 以支持 Win32 原生四边/四角拖拽调整大小与 Win11 贴靠布局（非客户区由 WM_NCCALCSIZE 归零）。
        let resizable_style = compute_window_style(&config(false, true));
        assert_eq!(resizable_style & WS_CAPTION, 0);
        assert_ne!(resizable_style & WS_THICKFRAME, 0);
        assert_ne!(resizable_style & WS_POPUP, 0);
        assert_ne!(resizable_style & WS_MINIMIZEBOX, 0);
        assert_ne!(resizable_style & WS_MAXIMIZEBOX, 0);

        // 不可缩放的无边框窗口同时去掉 WS_CAPTION 与 WS_THICKFRAME。
        let fixed_style = compute_window_style(&config(false, false));
        assert_eq!(fixed_style & WS_CAPTION, 0);
        assert_eq!(fixed_style & WS_THICKFRAME, 0);
        assert_ne!(fixed_style & WS_POPUP, 0);
    }
}

//! 原生 Win32 平台后端。
//!
//! 后端直接基于 Win32 API（`RegisterClassExW`、`CreateWindowExW`、`PeekMessageW`、
//! `MsgWaitForMultipleObjectsEx`）驱动事件循环与多窗口生命周期，不再依赖 `winit`。
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

#[cfg(windows)]
#[allow(
    non_snake_case,
    non_camel_case_types,
    clippy::upper_case_acronyms,
    dead_code
)]
mod win32 {
    use super::*;
    use core::ffi::c_void;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::time::Duration;

    use crate::pacing::PresentWaitPolicy;
    use crate::resize::Extent;
    use crate::win32_window::{CursorIcon, WM_APP_WAKE};
    use crate::window::{
        apply, ButtonState, ImeEvent, Modifiers, MouseButton, PlatformEvent, PlatformKey,
    };

    type HWND = *mut c_void;
    type HINSTANCE = *mut c_void;
    type HICON = *mut c_void;
    type HCURSOR = *mut c_void;
    type HBRUSH = *mut c_void;
    type HMENU = *mut c_void;
    type HIMC = *mut c_void;
    type HDROP = *mut c_void;
    type HMODULE = *mut c_void;
    type ATOM = u16;
    type BOOL = i32;
    type UINT = u32;
    type DWORD = u32;
    type LONG = i32;
    type WPARAM = usize;
    type LPARAM = isize;
    type LRESULT = isize;

    const CS_VREDRAW: UINT = 0x0001;
    const CS_HREDRAW: UINT = 0x0002;
    const CS_OWNDC: UINT = 0x0020;

    const WS_OVERLAPPED: DWORD = 0x0000_0000;
    const WS_CAPTION: DWORD = 0x00C0_0000;
    const WS_SYSMENU: DWORD = 0x0008_0000;
    const WS_THICKFRAME: DWORD = 0x0004_0000;
    const WS_MINIMIZEBOX: DWORD = 0x0002_0000;
    const WS_MAXIMIZEBOX: DWORD = 0x0001_0000;
    const WS_OVERLAPPEDWINDOW: DWORD =
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

    const WS_EX_APPWINDOW: DWORD = 0x0004_0000;
    const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;

    const GWLP_USERDATA: i32 = -21;

    const WM_DESTROY: UINT = 0x0002;
    const WM_SIZE: UINT = 0x0005;
    const WM_SETFOCUS: UINT = 0x0007;
    const WM_KILLFOCUS: UINT = 0x0008;
    const WM_CLOSE: UINT = 0x0010;
    const WM_QUIT: UINT = 0x0012;
    const WM_SHOWWINDOW: UINT = 0x0018;
    const WM_SETTINGCHANGE: UINT = 0x001A;
    const WM_SETCURSOR: UINT = 0x0020;
    const WM_GETMINMAXINFO: UINT = 0x0024;
    const WM_NCLBUTTONUP: UINT = 0x00A2;
    const WM_KEYDOWN: UINT = 0x0100;
    const WM_KEYUP: UINT = 0x0101;
    const WM_CHAR: UINT = 0x0102;
    const WM_SYSKEYDOWN: UINT = 0x0104;
    const WM_SYSKEYUP: UINT = 0x0105;
    const WM_SYSCHAR: UINT = 0x0106;
    const WM_IME_STARTCOMPOSITION: UINT = 0x010D;
    const WM_IME_ENDCOMPOSITION: UINT = 0x010E;
    const WM_IME_COMPOSITION: UINT = 0x010F;
    const WM_MOUSEMOVE: UINT = 0x0200;
    const WM_LBUTTONDOWN: UINT = 0x0201;
    const WM_LBUTTONUP: UINT = 0x0202;
    const WM_RBUTTONDOWN: UINT = 0x0204;
    const WM_RBUTTONUP: UINT = 0x0205;
    const WM_MBUTTONDOWN: UINT = 0x0207;
    const WM_MBUTTONUP: UINT = 0x0208;
    const WM_MOUSEWHEEL: UINT = 0x020A;
    const WM_XBUTTONDOWN: UINT = 0x020B;
    const WM_XBUTTONUP: UINT = 0x020C;
    const WM_MOUSEHWHEEL: UINT = 0x020E;
    const WM_DROPFILES: UINT = 0x0233;
    const WM_DPICHANGED: UINT = 0x02E0;
    const WM_MOUSELEAVE: UINT = 0x02A3;

    const HTCLIENT: LRESULT = 1;
    const HTCLOSE: WPARAM = 20;

    const PM_REMOVE: UINT = 0x0001;
    const QS_ALLINPUT: DWORD = 0x04FF;
    const MWMO_INPUTAVAILABLE: DWORD = 0x0004;
    const INFINITE: DWORD = 0xFFFF_FFFF;

    const SW_SHOW: i32 = 5;
    const SWP_NOZORDER: UINT = 0x0004;
    const SWP_NOACTIVATE: UINT = 0x0010;

    const IDC_ARROW: usize = 32512;
    const TME_LEAVE: DWORD = 0x0000_0002;

    const GCS_COMPSTR: DWORD = 0x0008;
    const GCS_CURSORPOS: DWORD = 0x0080;
    const GCS_RESULTSTR: DWORD = 0x0800;

    const VK_BACK: u16 = 0x08;
    const VK_TAB: u16 = 0x09;
    const VK_RETURN: u16 = 0x0D;
    const VK_SHIFT: i32 = 0x10;
    const VK_CONTROL: i32 = 0x11;
    const VK_MENU: i32 = 0x12;
    const VK_CAPITAL: u16 = 0x14;
    const VK_ESCAPE: u16 = 0x1B;
    const VK_SPACE: u16 = 0x20;
    const VK_PRIOR: u16 = 0x21;
    const VK_NEXT: u16 = 0x22;
    const VK_END: u16 = 0x23;
    const VK_HOME: u16 = 0x24;
    const VK_LEFT: u16 = 0x25;
    const VK_UP: u16 = 0x26;
    const VK_RIGHT: u16 = 0x27;
    const VK_DOWN: u16 = 0x28;
    const VK_INSERT: u16 = 0x2D;
    const VK_DELETE: u16 = 0x2E;
    const VK_LWIN: i32 = 0x5B;
    const VK_RWIN: i32 = 0x5C;
    const VK_APPS: u16 = 0x5D;
    const VK_F1: u16 = 0x70;
    const VK_F24: u16 = 0x87;

    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as isize;
    const RRF_RT_REG_DWORD: DWORD = 0x0000_0010;

    type WNDPROC =
        unsafe extern "system" fn(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

    #[repr(C)]
    struct POINT {
        x: LONG,
        y: LONG,
    }

    #[repr(C)]
    struct RECT {
        left: LONG,
        top: LONG,
        right: LONG,
        bottom: LONG,
    }

    #[repr(C)]
    struct MINMAXINFO {
        ptReserved: POINT,
        ptMaxSize: POINT,
        ptMaxPosition: POINT,
        ptMinTrackSize: POINT,
        ptMaxTrackSize: POINT,
    }

    #[repr(C)]
    struct MSG {
        hwnd: HWND,
        message: UINT,
        wParam: WPARAM,
        lParam: LPARAM,
        time: DWORD,
        pt: POINT,
    }

    #[repr(C)]
    struct WNDCLASSEXW {
        cbSize: UINT,
        style: UINT,
        lpfnWndProc: WNDPROC,
        cbClsExtra: i32,
        cbWndExtra: i32,
        hInstance: HINSTANCE,
        hIcon: HICON,
        hCursor: HCURSOR,
        hbrBackground: HBRUSH,
        lpszMenuName: *const u16,
        lpszClassName: *const u16,
        hIconSm: HICON,
    }

    #[repr(C)]
    struct TRACKMOUSEEVENT {
        cbSize: DWORD,
        dwFlags: DWORD,
        hwndTrack: HWND,
        dwHoverTime: DWORD,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lpModuleName: *const u16) -> HMODULE;
        fn GetProcAddress(hModule: HMODULE, lpProcName: *const u8) -> *const c_void;
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> ATOM;
        fn CreateWindowExW(
            dwExStyle: DWORD,
            lpClassName: *const u16,
            lpWindowName: *const u16,
            dwStyle: DWORD,
            X: i32,
            Y: i32,
            nWidth: i32,
            nHeight: i32,
            hWndParent: HWND,
            hMenu: HMENU,
            hInstance: HINSTANCE,
            lpParam: *const c_void,
        ) -> HWND;
        fn DestroyWindow(hWnd: HWND) -> BOOL;
        fn DefWindowProcW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
        fn PeekMessageW(
            lpMsg: *mut MSG,
            hWnd: HWND,
            wMsgFilterMin: UINT,
            wMsgFilterMax: UINT,
            wRemoveMsg: UINT,
        ) -> BOOL;
        fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
        fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
        fn PostMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> BOOL;
        fn PostQuitMessage(nExitCode: i32);
        fn MsgWaitForMultipleObjectsEx(
            nCount: DWORD,
            pHandles: *const *mut c_void,
            dwMilliseconds: DWORD,
            dwWakeMask: DWORD,
            dwFlags: DWORD,
        ) -> DWORD;
        fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
        fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> isize;
        fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
        fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
        fn SetWindowPos(
            hWnd: HWND,
            hWndInsertAfter: HWND,
            X: i32,
            Y: i32,
            cx: i32,
            cy: i32,
            uFlags: UINT,
        ) -> BOOL;
        fn SetCapture(hWnd: HWND) -> HWND;
        fn ReleaseCapture() -> BOOL;
        fn TrackMouseEvent(lpEventTrack: *mut TRACKMOUSEEVENT) -> BOOL;
        fn GetKeyState(nVirtKey: i32) -> i16;
        fn GetKeyboardState(lpKeyState: *mut u8) -> BOOL;
        fn ToUnicode(
            wVirtKey: UINT,
            wScanCode: UINT,
            lpKeyState: *const u8,
            pwszBuff: *mut u16,
            cchBuff: i32,
            wFlags: UINT,
        ) -> i32;
        fn GetDpiForSystem() -> UINT;
    }

    #[link(name = "imm32")]
    extern "system" {
        fn ImmGetContext(hWnd: HWND) -> HIMC;
        fn ImmReleaseContext(hWnd: HWND, hIMC: HIMC) -> BOOL;
        fn ImmGetCompositionStringW(
            hIMC: HIMC,
            dwIndex: DWORD,
            lpBuf: *mut c_void,
            dwBufLen: DWORD,
        ) -> LONG;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn DragAcceptFiles(hWnd: HWND, fAccept: BOOL);
        fn DragQueryFileW(hDrop: HDROP, iFile: UINT, lpszFile: *mut u16, cch: UINT) -> UINT;
        fn DragFinish(hDrop: HDROP);
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(
            hkey: isize,
            lpSubKey: *const u16,
            lpValue: *const u16,
            dwFlags: DWORD,
            pdwType: *mut DWORD,
            pvData: *mut c_void,
            pcbData: *mut DWORD,
        ) -> LONG;
    }

    pub(super) fn enable_per_monitor_v2_dpi() {
        unsafe {
            let user32_name: Vec<u16> = "user32.dll\0".encode_utf16().collect();
            let hmod = GetModuleHandleW(user32_name.as_ptr());
            if hmod.is_null() {
                return;
            }
            let proc = GetProcAddress(hmod, c"SetProcessDpiAwarenessContext".as_ptr().cast::<u8>());
            if !proc.is_null() {
                let func: unsafe extern "system" fn(isize) -> BOOL = core::mem::transmute(proc);
                // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
                let _ = func(-4isize);
            }
        }
    }

    fn is_system_dark_mode() -> bool {
        let subkey: Vec<u16> =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
                .encode_utf16()
                .collect();
        let value: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
        let mut data: DWORD = 1;
        let mut size: DWORD = core::mem::size_of::<DWORD>() as DWORD;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                core::ptr::null_mut(),
                (&mut data as *mut DWORD).cast::<c_void>(),
                &mut size,
            )
        };
        status == 0 && data == 0
    }

    fn current_modifiers() -> Modifiers {
        unsafe {
            Modifiers {
                shift: (GetKeyState(VK_SHIFT) as u16 & 0x8000) != 0,
                control: (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0,
                alt: (GetKeyState(VK_MENU) as u16 & 0x8000) != 0,
                meta: ((GetKeyState(VK_LWIN) as u16 & 0x8000) != 0)
                    || ((GetKeyState(VK_RWIN) as u16 & 0x8000) != 0),
            }
        }
    }

    fn translate_vk(vk: u16, scan_code: u32) -> PlatformKey {
        match vk {
            VK_RETURN => PlatformKey::Named("Enter".into()),
            VK_ESCAPE => PlatformKey::Named("Escape".into()),
            VK_TAB => PlatformKey::Named("Tab".into()),
            VK_BACK => PlatformKey::Named("Backspace".into()),
            VK_DELETE => PlatformKey::Named("Delete".into()),
            VK_LEFT => PlatformKey::Named("ArrowLeft".into()),
            VK_RIGHT => PlatformKey::Named("ArrowRight".into()),
            VK_UP => PlatformKey::Named("ArrowUp".into()),
            VK_DOWN => PlatformKey::Named("ArrowDown".into()),
            VK_HOME => PlatformKey::Named("Home".into()),
            VK_END => PlatformKey::Named("End".into()),
            VK_PRIOR => PlatformKey::Named("PageUp".into()),
            VK_NEXT => PlatformKey::Named("PageDown".into()),
            VK_INSERT => PlatformKey::Named("Insert".into()),
            VK_APPS => PlatformKey::Named("ContextMenu".into()),
            VK_CAPITAL => PlatformKey::Named("CapsLock".into()),
            VK_SPACE => PlatformKey::Named("Space".into()),
            VK_F1..=VK_F24 => PlatformKey::Named(format!("F{}", vk - VK_F1 + 1)),
            _ => {
                let mut key_state = [0u8; 256];
                let mut buf = [0u16; 8];
                let ok = unsafe { GetKeyboardState(key_state.as_mut_ptr()) };
                if ok != 0 {
                    // 清除 Ctrl/Alt 状态以便快捷键也能得到对应字符键名
                    key_state[VK_CONTROL as usize] = 0;
                    key_state[VK_MENU as usize] = 0;
                    let len = unsafe {
                        ToUnicode(
                            u32::from(vk),
                            scan_code,
                            key_state.as_ptr(),
                            buf.as_mut_ptr(),
                            buf.len() as i32,
                            0,
                        )
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

    fn imm_string(himc: HIMC, index: DWORD) -> Option<String> {
        let byte_len = unsafe { ImmGetCompositionStringW(himc, index, core::ptr::null_mut(), 0) };
        if byte_len <= 0 {
            return None;
        }
        let u16_len = (byte_len as usize) / 2;
        let mut buf = vec![0u16; u16_len];
        let copied = unsafe {
            ImmGetCompositionStringW(
                himc,
                index,
                buf.as_mut_ptr().cast::<c_void>(),
                byte_len as DWORD,
            )
        };
        if copied <= 0 {
            return None;
        }
        let actual_u16 = (copied as usize) / 2;
        Some(String::from_utf16_lossy(&buf[..actual_u16]))
    }

    struct WindowUserData {
        min_width: u32,
        min_height: u32,
        cursor: std::cell::Cell<CursorIcon>,
        tracking_leave: std::cell::Cell<bool>,
        last_modifiers: std::cell::Cell<Modifiers>,
        events: RefCell<Vec<PlatformEvent>>,
        redraw_requested: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: UINT,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let user_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowUserData;
        if user_ptr.is_null() {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        let data = unsafe { &*user_ptr };

        match msg {
            WM_CLOSE => {
                data.events.borrow_mut().push(PlatformEvent::CloseRequested);
                0
            }
            WM_NCLBUTTONUP => {
                if wparam == HTCLOSE {
                    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
                    return 0;
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
                    let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                    let scale = crate::win32_window::win32::scale_factor(hwnd as isize);
                    info.ptMinTrackSize.x = (f64::from(data.min_width) * scale).round() as i32;
                    info.ptMinTrackSize.y = (f64::from(data.min_height) * scale).round() as i32;
                    return 0;
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SIZE => {
                let width = (lparam & 0xFFFF) as u32;
                let height = ((lparam >> 16) & 0xFFFF) as u32;
                if width > 0 && height > 0 {
                    data.events
                        .borrow_mut()
                        .push(PlatformEvent::Resized(Extent::new(width, height)));
                    data.redraw_requested.store(true, Ordering::Release);
                }
                0
            }
            WM_DPICHANGED => {
                let new_dpi = (wparam & 0xFFFF) as u32;
                let scale = if new_dpi == 0 {
                    1.0
                } else {
                    f64::from(new_dpi) / 96.0
                };
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::ScaleFactorChanged(scale));
                let suggested = unsafe { &*(lparam as *const RECT) };
                unsafe {
                    SetWindowPos(
                        hwnd,
                        core::ptr::null_mut(),
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                0
            }
            WM_SETFOCUS => {
                data.events.borrow_mut().push(PlatformEvent::Focused(true));
                0
            }
            WM_KILLFOCUS => {
                data.events.borrow_mut().push(PlatformEvent::Focused(false));
                0
            }
            WM_SHOWWINDOW => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::VisibilityChanged(wparam != 0));
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SETTINGCHANGE => {
                data.events.borrow_mut().push(PlatformEvent::ThemeChanged {
                    dark: is_system_dark_mode(),
                });
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_SETCURSOR => {
                if (lparam & 0xFFFF) == HTCLIENT {
                    crate::win32_window::win32::apply_cursor(data.cursor.get());
                    return 1;
                }
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_MOUSEMOVE => {
                if !data.tracking_leave.get() {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as DWORD,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe { TrackMouseEvent(&mut tme) };
                    data.tracking_leave.set(true);
                }
                let x = (lparam & 0xFFFF) as i16 as f64;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as f64;
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::CursorMoved { x, y });
                0
            }
            WM_MOUSELEAVE => {
                data.tracking_leave.set(false);
                0
            }
            WM_LBUTTONDOWN => {
                unsafe { SetCapture(hwnd) };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Pressed,
                    button: MouseButton::Left,
                });
                0
            }
            WM_LBUTTONUP => {
                unsafe { ReleaseCapture() };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Released,
                    button: MouseButton::Left,
                });
                0
            }
            WM_RBUTTONDOWN => {
                unsafe { SetCapture(hwnd) };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Pressed,
                    button: MouseButton::Right,
                });
                0
            }
            WM_RBUTTONUP => {
                unsafe { ReleaseCapture() };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Released,
                    button: MouseButton::Right,
                });
                0
            }
            WM_MBUTTONDOWN => {
                unsafe { SetCapture(hwnd) };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Pressed,
                    button: MouseButton::Middle,
                });
                0
            }
            WM_MBUTTONUP => {
                unsafe { ReleaseCapture() };
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state: ButtonState::Released,
                    button: MouseButton::Middle,
                });
                0
            }
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let state = if msg == WM_XBUTTONDOWN {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                let xbutton = ((wparam >> 16) & 0xFFFF) as u16;
                data.events.borrow_mut().push(PlatformEvent::MouseInput {
                    state,
                    button: MouseButton::Other(xbutton),
                });
                1
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam >> 16) & 0xFFFF) as i16 as f64 / 120.0;
                data.events.borrow_mut().push(PlatformEvent::MouseWheel {
                    delta_x: 0.0,
                    delta_y: delta,
                });
                0
            }
            WM_MOUSEHWHEEL => {
                let delta = ((wparam >> 16) & 0xFFFF) as i16 as f64 / 120.0;
                data.events.borrow_mut().push(PlatformEvent::MouseWheel {
                    delta_x: delta,
                    delta_y: 0.0,
                });
                0
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
                let repeat = pressed && ((lparam >> 30) & 1) != 0;
                let scan_code = ((lparam >> 16) & 0xFF) as u32;
                let vk = (wparam & 0xFFFF) as u16;
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
                0
            }
            WM_CHAR | WM_SYSCHAR => {
                // Alt+F4 等系统字符交由 DefWindowProcW
                if msg == WM_SYSCHAR {
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                0
            }
            WM_IME_STARTCOMPOSITION => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::Ime(ImeEvent::Enabled));
                0
            }
            WM_IME_COMPOSITION => {
                let himc = unsafe { ImmGetContext(hwnd) };
                if !himc.is_null() {
                    let flags = lparam as DWORD;
                    if (flags & GCS_RESULTSTR) != 0 {
                        if let Some(commit) = imm_string(himc, GCS_RESULTSTR) {
                            data.events
                                .borrow_mut()
                                .push(PlatformEvent::Ime(ImeEvent::Commit(commit)));
                        }
                    }
                    if (flags & GCS_COMPSTR) != 0 {
                        let preedit = imm_string(himc, GCS_COMPSTR).unwrap_or_default();
                        let cursor = unsafe {
                            ImmGetCompositionStringW(himc, GCS_CURSORPOS, core::ptr::null_mut(), 0)
                        };
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
                    unsafe { ImmReleaseContext(hwnd, himc) };
                }
                0
            }
            WM_IME_ENDCOMPOSITION => {
                data.events
                    .borrow_mut()
                    .push(PlatformEvent::Ime(ImeEvent::Disabled));
                0
            }
            WM_DROPFILES => {
                let hdrop = wparam as HDROP;
                let count = unsafe { DragQueryFileW(hdrop, u32::MAX, core::ptr::null_mut(), 0) };
                for i in 0..count {
                    let len = unsafe { DragQueryFileW(hdrop, i, core::ptr::null_mut(), 0) };
                    if len > 0 {
                        let mut buf = vec![0u16; (len + 1) as usize];
                        let copied = unsafe { DragQueryFileW(hdrop, i, buf.as_mut_ptr(), len + 1) };
                        if copied > 0 {
                            let path = String::from_utf16_lossy(&buf[..copied as usize]);
                            data.events
                                .borrow_mut()
                                .push(PlatformEvent::FileDrop(PathBuf::from(path)));
                        }
                    }
                }
                unsafe { DragFinish(hdrop) };
                0
            }
            WM_APP_WAKE => {
                data.redraw_requested.store(true, Ordering::Release);
                0
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
        class_name: &[u16],
        hinstance: HINSTANCE,
        config: &WindowConfig,
    ) -> Result<WindowEntry, PlatformError> {
        let style = if config.resizable {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
        };

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

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_APPWINDOW,
                class_name.as_ptr(),
                title_wide.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                phys_w,
                phys_h,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                hinstance,
                core::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(PlatformError::Window(
                "CreateWindowExW returned null HWND".to_string(),
            ));
        }

        let redraw_requested = Arc::new(AtomicBool::new(true));
        let destroyed = Arc::new(AtomicBool::new(false));
        let user_data = Rc::new(WindowUserData {
            min_width: config.min_width,
            min_height: config.min_height,
            cursor: std::cell::Cell::new(CursorIcon::Default),
            tracking_leave: std::cell::Cell::new(false),
            last_modifiers: std::cell::Cell::new(Modifiers::default()),
            events: RefCell::new(Vec::new()),
            redraw_requested: Arc::clone(&redraw_requested),
            destroyed: Arc::clone(&destroyed),
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&user_data) as isize);
            DragAcceptFiles(hwnd, 1);
        }

        let window = Window::new(
            hwnd as isize,
            hinstance as isize,
            config.decorations,
            redraw_requested,
            destroyed,
        );
        let a11y = Win32A11y::new(hwnd as isize);
        if config.visible {
            unsafe { ShowWindow(hwnd, SW_SHOW) };
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
        let hinstance = unsafe { GetModuleHandleW(core::ptr::null()) };
        let class_name: Vec<u16> = "UnboundUiWindowClass\0".encode_utf16().collect();
        let default_cursor = unsafe { LoadCursorW(core::ptr::null_mut(), IDC_ARROW as *const u16) };

        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as UINT,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: core::ptr::null_mut(),
            hCursor: default_cursor,
            hbrBackground: core::ptr::null_mut(),
            lpszMenuName: core::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: core::ptr::null_mut(),
        };
        unsafe { RegisterClassExW(&wc) };

        let primary_entry = create_native_window(&class_name, hinstance, &config)?;
        let primary_id = primary_entry.window.id();
        let primary_hwnd_atomic = Arc::new(AtomicIsize::new(primary_entry.window.hwnd()));

        let wake_target = Arc::clone(&primary_hwnd_atomic);
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let hwnd = wake_target.load(Ordering::Acquire);
            if hwnd != 0 {
                unsafe {
                    PostMessageW(hwnd as HWND, WM_APP_WAKE, 0, 0);
                }
            }
        });

        let mut windows = vec![primary_entry];
        app.attach_waker(waker);
        // 取得唯一的可变条目借用后再分别借用其字段，避免同时索引同一容器。
        let first = &mut windows[0];
        app.resumed(&mut first.state, &first.window);

        let mut msg = MSG {
            hwnd: core::ptr::null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };

        'main_loop: while !windows.is_empty() {
            while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                if msg.message == WM_QUIT {
                    break 'main_loop;
                }
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // 同步各窗口的当前光标设置并排空事件
            let mut closing_indices = Vec::new();
            // 以迭代器持有单个条目的可变借用，避免对 `windows` 重复索引。
            for (index, entry) in windows.iter_mut().enumerate() {
                entry.user_data.cursor.set(entry.window.current_cursor());
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

            // 移除已请求关闭的辅助窗口
            for index in closing_indices.into_iter().rev() {
                let entry = windows.remove(index);
                app.window_closing(&entry.window);
                if !entry.window.is_destroyed() {
                    unsafe { DestroyWindow(entry.hwnd) };
                }
            }

            if windows.is_empty() {
                break;
            }

            // 为每个存活窗口渲染一帧
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

            // 创建帧期间请求的辅助窗口
            while let Some(extra_config) = app.take_window_request() {
                let Ok(mut entry) = create_native_window(&class_name, hinstance, &extra_config)
                else {
                    break;
                };
                app.window_created(&mut entry.state, &entry.window);
                windows.push(entry);
            }

            // 清除已销毁的窗口条目
            windows.retain(|entry| !entry.window.is_destroyed());
            if windows.is_empty() {
                break;
            }

            // 计算下一次唤醒截止时间
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
                    wait.as_millis().min(u128::from(INFINITE - 1)) as DWORD
                }
                None if primary.state.visible && primary.state.pacer.animating() => continue,
                None => INFINITE,
            };

            unsafe {
                MsgWaitForMultipleObjectsEx(
                    0,
                    core::ptr::null(),
                    timeout_ms,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                );
            }
        }

        // 有序销毁所有窗口并通知应用释放图形资源
        primary_hwnd_atomic.store(0, Ordering::Release);
        for entry in windows.drain(..) {
            app.window_closing(&entry.window);
            if !entry.window.is_destroyed() {
                unsafe { DestroyWindow(entry.hwnd) };
            }
        }

        Ok(())
    }
}

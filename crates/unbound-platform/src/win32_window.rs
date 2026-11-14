//! 原生 Win32 窗口句柄与操作封装。
//!
//! 替代 `winit::window::Window`，直接通过 Win32 API 管理窗口状态、光标、
//! IME 候选框位置、原生拖拽/调整大小以及平滑关闭请求。

use std::cell::Cell;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};

use crate::resize::Extent;
use crate::window::PlatformError;

/// 用于从后台线程唤醒 Win32 消息循环的自定义窗口消息。
pub(crate) const WM_APP_WAKE: u32 = 0x8001; // WM_APP + 1

/// 原生窗口唯一标识（对应 Win32 `HWND` 整数值）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub isize);

/// 窗口物理像素尺寸。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

impl PhysicalSize {
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl From<Extent> for PhysicalSize {
    fn from(extent: Extent) -> Self {
        Self {
            width: extent.width,
            height: extent.height,
        }
    }
}

/// 窗口物理像素位置。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PhysicalPosition {
    pub x: i32,
    pub y: i32,
}

impl PhysicalPosition {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// 窗口光标图标。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorIcon {
    #[default]
    Default,
    Pointer,
    Text,
    Grab,
    Grabbing,
    Crosshair,
    Move,
    NsResize,
    EwResize,
    NwseResize,
    NeswResize,
    NotAllowed,
}

/// 窗口边缘调整方向。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeDirection {
    North,
    South,
    West,
    East,
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

/// 原生 Win32 窗口句柄包装。
pub struct Window {
    hwnd: isize,
    hinstance: isize,
    decorated: bool,
    cursor: Cell<CursorIcon>,
    ime_allowed: Cell<bool>,
    redraw_requested: Arc<AtomicBool>,
    destroyed: Arc<AtomicBool>,
}

impl Window {
    pub(crate) fn new(
        hwnd: isize,
        hinstance: isize,
        decorated: bool,
        redraw_requested: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            hwnd,
            hinstance,
            decorated,
            cursor: Cell::new(CursorIcon::Default),
            ime_allowed: Cell::new(false),
            redraw_requested,
            destroyed,
        }
    }

    /// 返回窗口唯一标识。
    #[must_use]
    pub fn id(&self) -> WindowId {
        WindowId(self.hwnd)
    }

    /// 返回原生 Win32 `HWND`（以 `isize` 表示）。
    #[must_use]
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 获取当前选中的光标样式。
    #[must_use]
    pub fn current_cursor(&self) -> CursorIcon {
        self.cursor.get()
    }

    /// 获取客户区物理像素尺寸。
    #[must_use]
    pub fn inner_size(&self) -> PhysicalSize {
        #[cfg(windows)]
        {
            win32::client_size(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            PhysicalSize::new(1, 1)
        }
    }

    /// 获取窗口当前 DPI 缩放因子（`1.0` 对应 96 DPI）。
    #[must_use]
    pub fn scale_factor(&self) -> f64 {
        #[cfg(windows)]
        {
            win32::scale_factor(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            1.0
        }
    }

    /// 获取窗口所在显示器的垂直刷新率（Hz）；非 Windows 平台或查询失败时返回 `None`。
    #[must_use]
    pub fn refresh_rate_hz(&self) -> Option<f32> {
        #[cfg(windows)]
        {
            win32::refresh_rate_hz(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// 窗口当前是否可见。
    #[must_use]
    pub fn is_visible(&self) -> Option<bool> {
        #[cfg(windows)]
        {
            Some(win32::is_visible(self.hwnd))
        }
        #[cfg(not(windows))]
        {
            Some(true)
        }
    }

    /// 窗口当前是否处于最大化状态。
    #[must_use]
    pub fn is_maximized(&self) -> bool {
        #[cfg(windows)]
        {
            win32::is_maximized(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// 窗口是否使用系统原生装饰（标题栏与边框）。
    #[must_use]
    pub fn is_decorated(&self) -> bool {
        self.decorated
    }

    /// 设置窗口可见性。
    pub fn set_visible(&self, visible: bool) {
        #[cfg(windows)]
        win32::set_visible(self.hwnd, visible);
        #[cfg(not(windows))]
        let _ = visible;
    }

    /// 最小化或恢复窗口。
    pub fn set_minimized(&self, minimized: bool) {
        #[cfg(windows)]
        win32::set_minimized(self.hwnd, minimized);
        #[cfg(not(windows))]
        let _ = minimized;
    }

    /// 最大化或恢复窗口。
    pub fn set_maximized(&self, maximized: bool) {
        #[cfg(windows)]
        win32::set_maximized(self.hwnd, maximized);
        #[cfg(not(windows))]
        let _ = maximized;
    }

    /// 更新窗口标题（同步显示在任务栏与 Alt+Tab 切换器中）。
    pub fn set_title(&self, title: &str) {
        #[cfg(windows)]
        win32::set_title(self.hwnd, title);
        #[cfg(not(windows))]
        let _ = title;
    }

    /// 更新窗口光标样式。
    pub fn set_cursor(&self, cursor: CursorIcon) {
        self.cursor.set(cursor);
        #[cfg(windows)]
        win32::apply_cursor(cursor);
    }

    /// 请求在下一轮循环重绘一帧。
    pub fn request_redraw(&self) {
        self.redraw_requested.store(true, Ordering::Release);
        #[cfg(windows)]
        win32::wake_redraw(self.hwnd);
    }

    /// 向窗口投递原生 `WM_CLOSE`，走统一关闭流程。
    pub fn request_close(&self) {
        #[cfg(windows)]
        win32::post_close(self.hwnd);
    }

    /// 窗口是否已被销毁。
    #[must_use]
    pub fn is_destroyed(&self) -> bool {
        self.destroyed.load(Ordering::Acquire)
    }

    /// 启动系统原生标题栏拖拽移动（支持 Aero Snap）。
    pub fn drag_window(&self) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win32::drag_window(self.hwnd);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err(PlatformError::Window(
                "drag_window is only supported on Windows".to_string(),
            ))
        }
    }

    /// 启动系统原生窗口边缘拖拽缩放。
    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            win32::drag_resize_window(self.hwnd, direction);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = direction;
            Err(PlatformError::Window(
                "drag_resize_window is only supported on Windows".to_string(),
            ))
        }
    }

    /// 启用或禁用输入法（IME）组合输入。
    pub fn set_ime_allowed(&self, allowed: bool) {
        if self.ime_allowed.get() == allowed {
            return;
        }
        self.ime_allowed.set(allowed);
        #[cfg(windows)]
        win32::set_ime_allowed(self.hwnd, allowed);
    }

    /// 设置输入法候选框位置（物理像素坐标）。
    pub fn set_ime_cursor_area(&self, position: PhysicalPosition, size: PhysicalSize) {
        #[cfg(windows)]
        win32::set_ime_cursor_area(self.hwnd, position, size);
        #[cfg(not(windows))]
        let _ = (position, size);
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let Some(hwnd) = NonZeroIsize::new(self.hwnd) else {
            return Err(HandleError::Unavailable);
        };
        let mut handle = Win32WindowHandle::new(hwnd);
        handle.hinstance = NonZeroIsize::new(self.hinstance);
        let raw = RawWindowHandle::Win32(handle);
        // SAFETY: `self.hwnd` 在 `Window` 生命周期内为有效的 Win32 窗口句柄。
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Windows(WindowsDisplayHandle::new());
        // SAFETY: Windows 显示句柄为空标记结构体，无生命周期约束。
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

#[cfg(windows)]
#[allow(
    non_snake_case,
    non_camel_case_types,
    clippy::upper_case_acronyms,
    dead_code
)]
pub(crate) mod win32 {
    use super::*;
    use core::ffi::c_void;

    type HWND = *mut c_void;
    type HINSTANCE = *mut c_void;
    type HCURSOR = *mut c_void;
    type HIMC = *mut c_void;
    type HDC = *mut c_void;
    type BOOL = i32;
    type UINT = u32;
    type DWORD = u32;
    type WPARAM = usize;
    type LPARAM = isize;
    type LRESULT = isize;

    const SW_HIDE: i32 = 0;
    const SW_MAXIMIZE: i32 = 3;
    const SW_SHOW: i32 = 5;
    const SW_MINIMIZE: i32 = 6;
    const SW_RESTORE: i32 = 9;

    const WM_CLOSE: UINT = 0x0010;
    const WM_NCLBUTTONDOWN: UINT = 0x00A1;

    const HTCAPTION: WPARAM = 2;
    const HTLEFT: WPARAM = 10;
    const HTRIGHT: WPARAM = 11;
    const HTTOP: WPARAM = 12;
    const HTTOPLEFT: WPARAM = 13;
    const HTTOPRIGHT: WPARAM = 14;
    const HTBOTTOM: WPARAM = 15;
    const HTBOTTOMLEFT: WPARAM = 16;
    const HTBOTTOMRIGHT: WPARAM = 17;

    const IDC_ARROW: usize = 32512;
    const IDC_IBEAM: usize = 32513;
    const IDC_CROSS: usize = 32515;
    const IDC_SIZENWSE: usize = 32642;
    const IDC_SIZENESW: usize = 32643;
    const IDC_SIZEWE: usize = 32644;
    const IDC_SIZENS: usize = 32645;
    const IDC_SIZEALL: usize = 32646;
    const IDC_NO: usize = 32648;
    const IDC_HAND: usize = 32649;

    const CFS_POINT: DWORD = 0x0002;
    const CFS_EXCLUDE: DWORD = 0x0080;

    /// `GetDeviceCaps` 中查询垂直刷新率的索引。
    const VREFRESH: i32 = 116;

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct COMPOSITIONFORM {
        dwStyle: DWORD,
        ptCurrentPos: POINT,
        rcArea: RECT,
    }

    #[repr(C)]
    struct CANDIDATEFORM {
        dwIndex: DWORD,
        dwStyle: DWORD,
        ptCurrentPos: POINT,
        rcArea: RECT,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
        fn GetDpiForWindow(hwnd: HWND) -> UINT;
        fn IsWindowVisible(hWnd: HWND) -> BOOL;
        fn IsZoomed(hWnd: HWND) -> BOOL;
        fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
        fn SetWindowTextW(hWnd: HWND, lpString: *const u16) -> BOOL;
        fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: *const u16) -> HCURSOR;
        fn SetCursor(hCursor: HCURSOR) -> HCURSOR;
        fn PostMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> BOOL;
        fn SendMessageW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
        fn ReleaseCapture() -> BOOL;
        fn GetDC(hWnd: HWND) -> HDC;
        fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
    }

    #[link(name = "imm32")]
    extern "system" {
        fn ImmGetContext(hWnd: HWND) -> HIMC;
        fn ImmReleaseContext(hWnd: HWND, hIMC: HIMC) -> BOOL;
        fn ImmSetOpenStatus(hIMC: HIMC, fOpen: BOOL) -> BOOL;
        fn ImmSetCompositionWindow(hIMC: HIMC, lpCompForm: *const COMPOSITIONFORM) -> BOOL;
        fn ImmSetCandidateWindow(hIMC: HIMC, lpCandidate: *const CANDIDATEFORM) -> BOOL;
    }

    /// 查询窗口所在显示器的垂直刷新率（Hz）；驱动上报无效值时返回 `None`。
    pub(crate) fn refresh_rate_hz(hwnd: isize) -> Option<f32> {
        unsafe {
            let hwnd = hwnd as HWND;
            let hdc = GetDC(hwnd);
            if hdc.is_null() {
                return None;
            }
            let hz = GetDeviceCaps(hdc, VREFRESH);
            ReleaseDC(hwnd, hdc);
            // 部分驱动返回 0 或 1 表示刷新率未知，此时交由调用方使用默认值。
            (hz > 1).then_some(hz as f32)
        }
    }

    pub(crate) fn client_size(hwnd: isize) -> PhysicalSize {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetClientRect(hwnd as HWND, &mut rect) };
        if ok == 0 {
            return PhysicalSize::new(1, 1);
        }
        PhysicalSize::new(
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
        )
    }

    pub(crate) fn scale_factor(hwnd: isize) -> f64 {
        let dpi = unsafe { GetDpiForWindow(hwnd as HWND) };
        if dpi == 0 {
            1.0
        } else {
            f64::from(dpi) / 96.0
        }
    }

    pub(crate) fn is_visible(hwnd: isize) -> bool {
        unsafe { IsWindowVisible(hwnd as HWND) != 0 }
    }

    pub(crate) fn is_maximized(hwnd: isize) -> bool {
        unsafe { IsZoomed(hwnd as HWND) != 0 }
    }

    pub(crate) fn set_visible(hwnd: isize, visible: bool) {
        unsafe {
            ShowWindow(hwnd as HWND, if visible { SW_SHOW } else { SW_HIDE });
        }
    }

    pub(crate) fn set_minimized(hwnd: isize, minimized: bool) {
        unsafe {
            ShowWindow(
                hwnd as HWND,
                if minimized { SW_MINIMIZE } else { SW_RESTORE },
            );
        }
    }

    pub(crate) fn set_maximized(hwnd: isize, maximized: bool) {
        unsafe {
            ShowWindow(
                hwnd as HWND,
                if maximized { SW_MAXIMIZE } else { SW_RESTORE },
            );
        }
    }

    pub(crate) fn set_title(hwnd: isize, title: &str) {
        let wide: Vec<u16> = title.encode_utf16().chain(core::iter::once(0)).collect();
        unsafe {
            SetWindowTextW(hwnd as HWND, wide.as_ptr());
        }
    }

    pub(crate) fn apply_cursor(cursor: CursorIcon) {
        let id = match cursor {
            CursorIcon::Default => IDC_ARROW,
            CursorIcon::Pointer | CursorIcon::Grab | CursorIcon::Grabbing => IDC_HAND,
            CursorIcon::Text => IDC_IBEAM,
            CursorIcon::Crosshair => IDC_CROSS,
            CursorIcon::Move => IDC_SIZEALL,
            CursorIcon::NsResize => IDC_SIZENS,
            CursorIcon::EwResize => IDC_SIZEWE,
            CursorIcon::NwseResize => IDC_SIZENWSE,
            CursorIcon::NeswResize => IDC_SIZENESW,
            CursorIcon::NotAllowed => IDC_NO,
        };
        unsafe {
            let hcursor = LoadCursorW(core::ptr::null_mut(), id as *const u16);
            if !hcursor.is_null() {
                SetCursor(hcursor);
            }
        }
    }

    pub(crate) fn wake_redraw(hwnd: isize) {
        unsafe {
            PostMessageW(hwnd as HWND, WM_APP_WAKE, 0, 0);
        }
    }

    pub(crate) fn post_close(hwnd: isize) {
        unsafe {
            PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
        }
    }

    pub(crate) fn drag_window(hwnd: isize) {
        unsafe {
            ReleaseCapture();
            SendMessageW(hwnd as HWND, WM_NCLBUTTONDOWN, HTCAPTION, 0);
        }
    }

    pub(crate) fn drag_resize_window(hwnd: isize, direction: ResizeDirection) {
        let ht = match direction {
            ResizeDirection::North => HTTOP,
            ResizeDirection::South => HTBOTTOM,
            ResizeDirection::West => HTLEFT,
            ResizeDirection::East => HTRIGHT,
            ResizeDirection::NorthWest => HTTOPLEFT,
            ResizeDirection::NorthEast => HTTOPRIGHT,
            ResizeDirection::SouthWest => HTBOTTOMLEFT,
            ResizeDirection::SouthEast => HTBOTTOMRIGHT,
        };
        unsafe {
            ReleaseCapture();
            SendMessageW(hwnd as HWND, WM_NCLBUTTONDOWN, ht, 0);
        }
    }

    pub(crate) fn set_ime_allowed(hwnd: isize, allowed: bool) {
        unsafe {
            let himc = ImmGetContext(hwnd as HWND);
            if !himc.is_null() {
                ImmSetOpenStatus(himc, if allowed { 1 } else { 0 });
                ImmReleaseContext(hwnd as HWND, himc);
            }
        }
    }

    pub(crate) fn set_ime_cursor_area(hwnd: isize, position: PhysicalPosition, size: PhysicalSize) {
        unsafe {
            let himc = ImmGetContext(hwnd as HWND);
            if !himc.is_null() {
                let comp = COMPOSITIONFORM {
                    dwStyle: CFS_POINT,
                    ptCurrentPos: POINT {
                        x: position.x,
                        y: position.y + size.height as i32,
                    },
                    rcArea: RECT {
                        left: position.x,
                        top: position.y,
                        right: position.x + size.width as i32,
                        bottom: position.y + size.height as i32,
                    },
                };
                ImmSetCompositionWindow(himc, &comp);

                let cand = CANDIDATEFORM {
                    dwIndex: 0,
                    dwStyle: CFS_EXCLUDE,
                    ptCurrentPos: POINT {
                        x: position.x,
                        y: position.y + size.height as i32,
                    },
                    rcArea: RECT {
                        left: position.x,
                        top: position.y,
                        right: position.x + size.width as i32,
                        bottom: position.y + size.height as i32,
                    },
                };
                ImmSetCandidateWindow(himc, &cand);
                ImmReleaseContext(hwnd as HWND, himc);
            }
        }
    }
}

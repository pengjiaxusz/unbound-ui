//! 原生 Win32 窗口句柄与操作封装。
//!
//! 替代 `winit::window::Window`，基于官方 `windows` crate（0.62.2）直接管理窗口状态、
//! 光标、IME 候选框位置、原生标题栏拖拽/边缘调整大小以及平滑关闭请求。

use std::cell::Cell;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};

use crate::floating::{FloatingWindowCapabilities, MonitorWorkArea};
use crate::resize::Extent;
use crate::window::{PlatformError, ScreenRect};

/// 用于从后台线程唤醒 Win32 消息循环的自定义窗口消息。
#[cfg_attr(not(windows), allow(dead_code))]
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
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorIcon {
    #[default]
    Default = 0,
    Pointer = 1,
    Text = 2,
    Grab = 3,
    Grabbing = 4,
    Crosshair = 5,
    Move = 6,
    NsResize = 7,
    EwResize = 8,
    NwseResize = 9,
    NeswResize = 10,
    NotAllowed = 11,
}

impl CursorIcon {
    /// 从原子 `u8` 编码还原 [`CursorIcon`]。
    #[must_use]
    pub const fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Pointer,
            2 => Self::Text,
            3 => Self::Grab,
            4 => Self::Grabbing,
            5 => Self::Crosshair,
            6 => Self::Move,
            7 => Self::NsResize,
            8 => Self::EwResize,
            9 => Self::NwseResize,
            10 => Self::NeswResize,
            11 => Self::NotAllowed,
            _ => Self::Default,
        }
    }
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
    popup_id: Option<u64>,
    cursor: Arc<AtomicU8>,
    ime_allowed: Cell<bool>,
    redraw_requested: Arc<AtomicBool>,
    destroyed: Arc<AtomicBool>,
}

impl Window {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn new(
        hwnd: isize,
        hinstance: isize,
        decorated: bool,
        cursor: Arc<AtomicU8>,
        redraw_requested: Arc<AtomicBool>,
        destroyed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            hwnd,
            hinstance,
            decorated,
            popup_id: None,
            cursor,
            ime_allowed: Cell::new(false),
            redraw_requested,
            destroyed,
        }
    }

    /// 将当前窗口标记为指定业务浮层 ID 对应的原生弹出窗宿主。
    #[must_use]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn with_popup_id(mut self, popup_id: u64) -> Self {
        self.popup_id = Some(popup_id);
        self
    }

    /// 当前窗口是否为无激活越界原生弹出窗。
    #[must_use]
    pub fn is_popup(&self) -> bool {
        self.popup_id.is_some()
    }

    /// 若当前窗口为原生弹出窗，返回其绑定的业务浮层 ID。
    #[must_use]
    pub fn popup_id(&self) -> Option<u64> {
        self.popup_id
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
        CursorIcon::from_u8(self.cursor.load(Ordering::Acquire))
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

    /// 获取客户区在屏幕坐标系中的物理矩形。
    ///
    /// 供依赖屏幕锚点的后景（例如按主屏壁纸取样的自绘纹理）使用；布局仍只消费逻辑视口。
    #[must_use]
    pub fn client_rect_on_screen(&self) -> ScreenRect {
        #[cfg(windows)]
        {
            win32::client_rect_on_screen(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            let size = self.inner_size();
            ScreenRect::new(0, 0, size.width, size.height)
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

    /// 窗口当前是否处于最小化状态。
    #[must_use]
    pub fn is_minimized(&self) -> bool {
        #[cfg(windows)]
        {
            win32::is_minimized(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// 获取窗口外框左上角在屏幕物理坐标系中的位置。
    #[must_use]
    pub fn outer_position(&self) -> Option<(i32, i32)> {
        #[cfg(windows)]
        {
            win32::outer_position(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// 设置窗口外框左上角在屏幕物理坐标系中的位置。
    pub fn set_outer_position(&self, x: i32, y: i32) {
        #[cfg(windows)]
        win32::set_outer_position(self.hwnd, x, y);
        #[cfg(not(windows))]
        let _ = (x, y);
    }

    /// 设置窗口客户区物理像素尺寸。
    pub fn set_inner_size(&self, width: u32, height: u32) {
        #[cfg(windows)]
        win32::set_inner_size(self.hwnd, width, height);
        #[cfg(not(windows))]
        let _ = (width, height);
    }

    /// 恢复隐藏或最小化的窗口并将其激活置顶到前台。
    pub fn activate_foreground(&self) {
        #[cfg(windows)]
        win32::activate_foreground(self.hwnd);
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

    /// 设置窗口可见性且不抢占前台焦点。
    pub fn set_visible_no_activate(&self, visible: bool) {
        #[cfg(windows)]
        win32::set_visible_no_activate(self.hwnd, visible);
        #[cfg(not(windows))]
        let _ = visible;
    }

    /// 运行时无闪烁切换窗口是否全局置顶。
    pub fn set_always_on_top(&self, topmost: bool) {
        #[cfg(windows)]
        win32::set_always_on_top(self.hwnd, topmost);
        #[cfg(not(windows))]
        let _ = topmost;
    }

    /// 运行时无闪烁切换窗口鼠标穿透状态。
    pub fn set_click_through(&self, click_through: bool) {
        #[cfg(windows)]
        win32::set_click_through(self.hwnd, click_through);
        #[cfg(not(windows))]
        let _ = click_through;
    }

    /// 运行时设置窗口外框在屏幕坐标系中的物理矩形（不激活窗口）。
    pub fn set_outer_rect(&self, rect: ScreenRect) {
        #[cfg(windows)]
        win32::set_outer_rect(self.hwnd, rect);
        #[cfg(not(windows))]
        let _ = rect;
    }

    /// 查询当前窗口后端的浮窗能力描述。
    #[must_use]
    pub fn capabilities(&self) -> FloatingWindowCapabilities {
        FloatingWindowCapabilities::win32_full()
    }

    /// 查询当前窗口所在显示器的工作区与 DPI 信息。
    #[must_use]
    pub fn work_area(&self) -> Option<MonitorWorkArea> {
        #[cfg(windows)]
        {
            win32::work_area_for_window(self.hwnd)
        }
        #[cfg(not(windows))]
        {
            None
        }
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
        self.cursor.store(cursor as u8, Ordering::Release);
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
pub(crate) mod win32 {
    use super::*;
    use core::ffi::c_void;

    use ::windows::core::{BOOL, PCWSTR};
    use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
    use ::windows::Win32::Graphics::Gdi::{
        ClientToScreen, EnumDisplayMonitors, GetDC, GetDeviceCaps, GetMonitorInfoW,
        MonitorFromWindow, ReleaseDC, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        VREFRESH,
    };
    use ::windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
    use ::windows::Win32::UI::Input::Ime::{
        ImmAssociateContextEx, ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow,
        ImmSetCompositionWindow, CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, HIMC,
        IACE_CHILDREN, IACE_DEFAULT,
    };
    use ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetClientRect, GetCursorPos, GetWindowLongPtrW, GetWindowRect, IsIconic,
        IsWindowVisible, IsZoomed, LoadCursorW, PostMessageW, SetCursor, SetForegroundWindow,
        SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, GWL_EXSTYLE, HTCAPTION,
        HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO,
        IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, MONITORINFOF_PRIMARY,
        SC_SIZE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
        SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, WMSZ_BOTTOM,
        WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT, WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT,
        WMSZ_TOPRIGHT, WM_CLOSE, WM_NCLBUTTONDOWN, WM_SYSCOMMAND, WS_EX_TRANSPARENT,
    };

    #[inline]
    pub(crate) fn hwnd_from_isize(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    /// 查询窗口所在显示器的垂直刷新率（Hz）；驱动上报无效值时返回 `None`。
    pub(crate) fn refresh_rate_hz(hwnd: isize) -> Option<f32> {
        unsafe {
            let hwnd = hwnd_from_isize(hwnd);
            let hdc = GetDC(Some(hwnd));
            if hdc.is_invalid() {
                return None;
            }
            let hz = GetDeviceCaps(Some(hdc), VREFRESH);
            let _ = ReleaseDC(Some(hwnd), hdc);
            // 部分驱动返回 0 或 1 表示刷新率未知，此时交由调用方使用默认值。
            (hz > 1).then_some(hz as f32)
        }
    }

    pub(crate) fn client_size(hwnd: isize) -> PhysicalSize {
        let mut rect = RECT::default();
        let ok = unsafe { GetClientRect(hwnd_from_isize(hwnd), &mut rect) };
        if ok.is_err() {
            return PhysicalSize::new(1, 1);
        }
        PhysicalSize::new(
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
        )
    }

    pub(crate) fn client_rect_on_screen(hwnd: isize) -> ScreenRect {
        let hwnd = hwnd_from_isize(hwnd);
        let mut rect = RECT::default();
        let ok = unsafe { GetClientRect(hwnd, &mut rect) };
        if ok.is_err() {
            return ScreenRect::new(0, 0, 1, 1);
        }
        // `GetClientRect` 的左上角始终是 (0, 0)，需要借助客户区原点的屏幕坐标补上位置。
        let mut origin = POINT { x: 0, y: 0 };
        let mapped = unsafe { ClientToScreen(hwnd, &mut origin) };
        if !mapped.as_bool() {
            return ScreenRect::new(0, 0, 1, 1);
        }
        ScreenRect::new(
            origin.x,
            origin.y,
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
        )
    }

    pub(crate) fn scale_factor(hwnd: isize) -> f64 {
        let dpi = unsafe { GetDpiForWindow(hwnd_from_isize(hwnd)) };
        if dpi == 0 {
            1.0
        } else {
            f64::from(dpi) / 96.0
        }
    }

    pub(crate) fn is_visible(hwnd: isize) -> bool {
        unsafe { IsWindowVisible(hwnd_from_isize(hwnd)).as_bool() }
    }

    pub(crate) fn is_maximized(hwnd: isize) -> bool {
        unsafe { IsZoomed(hwnd_from_isize(hwnd)).as_bool() }
    }

    pub(crate) fn is_minimized(hwnd: isize) -> bool {
        unsafe { IsIconic(hwnd_from_isize(hwnd)).as_bool() }
    }

    pub(crate) fn outer_position(hwnd: isize) -> Option<(i32, i32)> {
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd_from_isize(hwnd), &mut rect) }.is_ok() {
            Some((rect.left, rect.top))
        } else {
            None
        }
    }

    pub(crate) fn set_outer_position(hwnd: isize, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(
                hwnd_from_isize(hwnd),
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub(crate) fn set_inner_size(hwnd: isize, width: u32, height: u32) {
        let hwnd = hwnd_from_isize(hwnd);
        let mut win_rect = RECT::default();
        let mut client_rect = RECT::default();
        let (extra_w, extra_h) = if unsafe { GetWindowRect(hwnd, &mut win_rect) }.is_ok()
            && unsafe { GetClientRect(hwnd, &mut client_rect) }.is_ok()
        {
            let ww = win_rect.right - win_rect.left;
            let wh = win_rect.bottom - win_rect.top;
            let cw = client_rect.right - client_rect.left;
            let ch = client_rect.bottom - client_rect.top;
            ((ww - cw).max(0), (wh - ch).max(0))
        } else {
            (0, 0)
        };
        let outer_w = (width.max(1) as i32).saturating_add(extra_w);
        let outer_h = (height.max(1) as i32).saturating_add(extra_h);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                outer_w,
                outer_h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub(crate) fn activate_foreground(hwnd: isize) {
        let hwnd = hwnd_from_isize(hwnd);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    /// 枚举当前系统所有显示器的工作区物理矩形、DPI 缩放与主屏标记。
    pub(crate) fn enumerate_monitor_work_areas() -> Vec<crate::window::DisplayWorkArea> {
        unsafe extern "system" fn enum_proc(
            monitor: HMONITOR,
            _hdc: HDC,
            _rect: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let out_ptr = lparam.0 as *mut Vec<crate::window::DisplayWorkArea>;
            if out_ptr.is_null() {
                return BOOL(0);
            }
            let mut mi = MONITORINFO {
                cbSize: core::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe { GetMonitorInfoW(monitor, &mut mi) }.as_bool() {
                let mut dpi_x: u32 = 96;
                let mut dpi_y: u32 = 96;
                let scale_factor = if unsafe {
                    GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y)
                }
                .is_ok()
                    && dpi_x > 0
                {
                    f64::from(dpi_x) / 96.0
                } else {
                    1.0
                };
                let area = crate::window::DisplayWorkArea {
                    x: mi.rcWork.left,
                    y: mi.rcWork.top,
                    width: (mi.rcWork.right - mi.rcWork.left).max(0),
                    height: (mi.rcWork.bottom - mi.rcWork.top).max(0),
                    scale_factor,
                    is_primary: (mi.dwFlags & MONITORINFOF_PRIMARY) != 0,
                };
                // SAFETY: `out_ptr` 指向调用栈上独占的 `Vec<MonitorWorkArea>`，仅在本回调内同步写入。
                unsafe {
                    (*out_ptr).push(area);
                }
            }
            BOOL(1)
        }

        let mut areas = Vec::new();
        let lparam = LPARAM((&raw mut areas) as isize);
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(enum_proc), lparam);
        }
        areas
    }

    pub(crate) fn set_visible(hwnd: isize, visible: bool) {
        unsafe {
            let _ = ShowWindow(
                hwnd_from_isize(hwnd),
                if visible { SW_SHOW } else { SW_HIDE },
            );
        }
    }

    pub(crate) fn set_visible_no_activate(hwnd: isize, visible: bool) {
        unsafe {
            let _ = ShowWindow(
                hwnd_from_isize(hwnd),
                if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
        }
    }

    pub(crate) fn set_always_on_top(hwnd: isize, topmost: bool) {
        let insert_after = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        unsafe {
            let _ = SetWindowPos(
                hwnd_from_isize(hwnd),
                Some(insert_after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    pub(crate) fn set_click_through(hwnd: isize, click_through: bool) {
        let hwnd = hwnd_from_isize(hwnd);
        unsafe {
            let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            if click_through {
                ex_style |= WS_EX_TRANSPARENT.0;
            } else {
                ex_style &= !WS_EX_TRANSPARENT.0;
            }
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    pub(crate) fn set_outer_rect(hwnd: isize, rect: ScreenRect) {
        unsafe {
            let _ = SetWindowPos(
                hwnd_from_isize(hwnd),
                None,
                rect.x,
                rect.y,
                rect.width.max(1) as i32,
                rect.height.max(1) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub(crate) fn work_area_for_window(hwnd: isize) -> Option<MonitorWorkArea> {
        let hmonitor =
            unsafe { MonitorFromWindow(hwnd_from_isize(hwnd), MONITOR_DEFAULTTONEAREST) };
        if hmonitor.is_invalid() {
            return None;
        }
        crate::floating::win32_monitors::query_hmonitor(hmonitor)
    }

    pub(crate) fn set_minimized(hwnd: isize, minimized: bool) {
        unsafe {
            let _ = ShowWindow(
                hwnd_from_isize(hwnd),
                if minimized { SW_MINIMIZE } else { SW_RESTORE },
            );
        }
    }

    pub(crate) fn set_maximized(hwnd: isize, maximized: bool) {
        unsafe {
            let _ = ShowWindow(
                hwnd_from_isize(hwnd),
                if maximized { SW_MAXIMIZE } else { SW_RESTORE },
            );
        }
    }

    pub(crate) fn set_title(hwnd: isize, title: &str) {
        let wide: Vec<u16> = title.encode_utf16().chain(core::iter::once(0)).collect();
        unsafe {
            let _ = SetWindowTextW(hwnd_from_isize(hwnd), PCWSTR(wide.as_ptr()));
        }
    }

    pub(crate) fn apply_cursor(cursor: CursorIcon) {
        let id: PCWSTR = match cursor {
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
            if let Ok(hcursor) = LoadCursorW(None, id) {
                let _ = SetCursor(Some(hcursor));
            }
        }
    }

    pub(crate) fn wake_redraw(hwnd: isize) {
        unsafe {
            let _ = PostMessageW(
                Some(hwnd_from_isize(hwnd)),
                WM_APP_WAKE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    pub(crate) fn post_close(hwnd: isize) {
        unsafe {
            let _ = PostMessageW(Some(hwnd_from_isize(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    fn packed_cursor_lparam() -> LPARAM {
        let mut pt = POINT::default();
        let _ = unsafe { GetCursorPos(&mut pt) };
        let x = (pt.x as i16 as u16) as isize;
        let y = ((pt.y as i16 as u16) as isize) << 16;
        LPARAM(x | y)
    }

    pub(crate) fn drag_window(hwnd: isize) {
        unsafe {
            let lparam = packed_cursor_lparam();
            let _ = ReleaseCapture();
            let _ = PostMessageW(
                Some(hwnd_from_isize(hwnd)),
                WM_NCLBUTTONDOWN,
                WPARAM(HTCAPTION as usize),
                lparam,
            );
        }
    }

    pub(crate) fn drag_resize_window(hwnd: isize, direction: ResizeDirection) {
        let wmsz = match direction {
            ResizeDirection::North => WMSZ_TOP,
            ResizeDirection::South => WMSZ_BOTTOM,
            ResizeDirection::West => WMSZ_LEFT,
            ResizeDirection::East => WMSZ_RIGHT,
            ResizeDirection::NorthWest => WMSZ_TOPLEFT,
            ResizeDirection::NorthEast => WMSZ_TOPRIGHT,
            ResizeDirection::SouthWest => WMSZ_BOTTOMLEFT,
            ResizeDirection::SouthEast => WMSZ_BOTTOMRIGHT,
        };
        unsafe {
            let lparam = packed_cursor_lparam();
            let _ = ReleaseCapture();
            let _ = PostMessageW(
                Some(hwnd_from_isize(hwnd)),
                WM_SYSCOMMAND,
                WPARAM((SC_SIZE as usize) + (wmsz as usize)),
                lparam,
            );
        }
    }

    pub(crate) fn set_ime_allowed(hwnd: isize, allowed: bool) {
        unsafe {
            let hwnd = hwnd_from_isize(hwnd);
            // 与 winit 的 Windows 后端保持一致：启用时关联线程默认输入上下文，
            // 禁用时用 IACE_CHILDREN 把上下文从本窗口摘掉（但保留线程级上下文）。
            //
            // 注意不能用 dwFlags = 0：那不是“解除关联”，而是“什么都不做”，
            // 会让禁用形同虚设；也不能顺手 ImmSetOpenStatus(true)，
            // 那会盖掉用户用 Shift / Ctrl+Space 切换的中英文状态。
            let flags = if allowed { IACE_DEFAULT } else { IACE_CHILDREN };
            let associated = ImmAssociateContextEx(hwnd, HIMC(core::ptr::null_mut()), flags);
            ime_trace(format!(
                "associate ime allowed={allowed} flags=0x{flags:X} result={associated:?} err={:?}",
                ::windows::Win32::Foundation::GetLastError()
            ));
            if allowed {
                // 组合窗位置属于“上一次会话”的状态，重新启用后要重新上报。
                IME_APPLIED.with(|cell| cell.set(None));
            }
        }
    }

    /// 输入法诊断日志（临时）。
    fn ime_trace(message: impl core::fmt::Display) {
        if std::env::var_os("UNBOUND_IME_TRACE").is_none()
            && !std::env::temp_dir().join("unbound-ime.trace").exists()
        {
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

    thread_local! {
        /// 宿主最近一次上报的插入符：客户区物理像素 (x, y, 宽, 高)。
        static IME_CARET: core::cell::Cell<Option<(i32, i32, u32, u32)>> =
            const { core::cell::Cell::new(None) };
        /// 已经写入 IMM 的插入符区域；与 `IME_CARET` 相同则跳过重复调用。
        static IME_APPLIED: core::cell::Cell<Option<(i32, i32, u32, u32)>> =
            const { core::cell::Cell::new(None) };
    }

    /// 宿主最近一次上报的插入符（客户区坐标，物理像素）与行高。
    ///
    /// 自绘组合串的应用要用它回答 `IMR_QUERYCHARPOSITION`，否则候选窗无法锚定。
    pub(crate) fn ime_caret_client() -> Option<(i32, i32, u32)> {
        IME_CARET
            .with(|cell| cell.get())
            .map(|(x, y, _, h)| (x, y, h))
    }

    /// 组合开始时重新应用组合窗 / 候选窗位置。
    ///
    /// 输入法在每次组合开始时重新建立自己的 UI，因此即使区域没变也要再写一次。
    pub(crate) fn reapply_ime_area(hwnd: isize) {
        let Some(area) = IME_CARET.with(|cell| cell.get()) else {
            return;
        };
        apply_ime_area(hwnd_from_isize(hwnd), area);
        IME_APPLIED.with(|cell| cell.set(Some(area)));
    }

    fn apply_ime_area(hwnd: HWND, area: (i32, i32, u32, u32)) {
        let (x, y, width, height) = area;
        unsafe {
            let himc = ImmGetContext(hwnd);
            if himc.is_invalid() {
                return;
            }
            let bottom = y + height as i32;
            let rc_area = RECT {
                left: x,
                top: y,
                right: x + width as i32,
                bottom,
            };
            let comp = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: POINT { x, y: bottom },
                rcArea: rc_area,
            };
            let _ = ImmSetCompositionWindow(himc, &comp);

            let cand = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_EXCLUDE,
                ptCurrentPos: POINT { x, y: bottom },
                rcArea: rc_area,
            };
            let _ = ImmSetCandidateWindow(himc, &cand);
            let _ = ImmReleaseContext(hwnd, himc);
        }
    }

    pub(crate) fn set_ime_cursor_area(hwnd: isize, position: PhysicalPosition, size: PhysicalSize) {
        let area = (position.x, position.y, size.width, size.height);
        IME_CARET.with(|cell| cell.set(Some(area)));
        // 只在区域真正变化时才写 IMM。宿主每个事件后都会重报插入符，
        // 若每帧无条件调 ImmSetCompositionWindow，输入法会持续重排、重绘，
        // 表现为 WM_IME_NOTIFY / IMR_QUERYCHARPOSITION 无休止循环。
        if IME_APPLIED.with(|cell| cell.get()) == Some(area) {
            return;
        }
        apply_ime_area(hwnd_from_isize(hwnd), area);
        IME_APPLIED.with(|cell| cell.set(Some(area)));
    }
}

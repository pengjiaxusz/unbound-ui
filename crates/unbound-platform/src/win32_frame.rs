//! Win32 Frameless Window, Snap Layouts, and DWM Integration for Windows 10 & 11.
//!
//! Provides:
//! - Seamless borderless window with `WM_NCCALCSIZE` work area clamping (zero maximized overflow).
//! - Native `WM_NCHITTEST` subclassing:
//!   - `HTMAXBUTTON` mapping on the custom maximize button, activating Windows 11 Snap Layouts hover menu.
//!   - `HTCAPTION` mapping on title bar for OS-driven window dragging and Aero Snap (`Win + Arrows`).
//!   - `HTLEFT..HTBOTTOMRIGHT` edge resizing.
//! - DWM window attributes:
//!   - Windows 11 rounded corners (`DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`).
//!   - Immersive Dark Mode synchronization (`DWMWA_USE_IMMERSIVE_DARK_MODE`).
//!   - Windows 10 drop shadow restoration (`DwmExtendFrameIntoClientArea`).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;

pub use unbound_math::Rect;

/// Atomic metrics for hit-testing the custom title bar and window buttons.
#[derive(Debug, Default)]
pub struct FrameHitMetrics {
    /// Height of the title bar in logical pixels.
    pub title_bar_height: AtomicI32,
    /// Maximize button bounding box in logical pixels.
    pub max_btn_left: AtomicI32,
    pub max_btn_top: AtomicI32,
    pub max_btn_right: AtomicI32,
    pub max_btn_bottom: AtomicI32,
    /// Minimize button bounding box in logical pixels.
    pub min_btn_left: AtomicI32,
    pub min_btn_top: AtomicI32,
    pub min_btn_right: AtomicI32,
    pub min_btn_bottom: AtomicI32,
    /// Close button bounding box in logical pixels.
    pub close_btn_left: AtomicI32,
    pub close_btn_top: AtomicI32,
    pub close_btn_right: AtomicI32,
    pub close_btn_bottom: AtomicI32,
    /// DPI scale factor multiplied by 1000 (e.g. 1500 for 1.5x scale).
    pub scale_permille: AtomicU32,
    /// Whether the custom frame hook is active.
    pub active: AtomicBool,
}

impl FrameHitMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            scale_permille: AtomicU32::new(1000),
            active: AtomicBool::new(true),
            ..Default::default()
        }
    }

    /// Update scale factor.
    pub fn set_scale_factor(&self, factor: f32) {
        let permille = (factor.clamp(0.25, 5.0) * 1000.0).round() as u32;
        self.scale_permille.store(permille, Ordering::Release);
    }

    /// Read scale factor.
    #[must_use]
    pub fn scale_factor(&self) -> f32 {
        self.scale_permille.load(Ordering::Acquire) as f32 / 1000.0
    }

    /// Update title bar height (logical pixels).
    pub fn set_title_bar_height(&self, height: f32) {
        self.title_bar_height
            .store(height.round() as i32, Ordering::Release);
    }

    /// Update maximize button rectangle (logical pixels).
    pub fn set_maximize_button_rect(&self, rect: Rect) {
        self.max_btn_left
            .store(rect.origin.x.round() as i32, Ordering::Release);
        self.max_btn_top
            .store(rect.origin.y.round() as i32, Ordering::Release);
        self.max_btn_right.store(
            (rect.origin.x + rect.width()).round() as i32,
            Ordering::Release,
        );
        self.max_btn_bottom.store(
            (rect.origin.y + rect.height()).round() as i32,
            Ordering::Release,
        );
    }

    /// Update minimize button rectangle (logical pixels).
    pub fn set_minimize_button_rect(&self, rect: Rect) {
        self.min_btn_left
            .store(rect.origin.x.round() as i32, Ordering::Release);
        self.min_btn_top
            .store(rect.origin.y.round() as i32, Ordering::Release);
        self.min_btn_right.store(
            (rect.origin.x + rect.width()).round() as i32,
            Ordering::Release,
        );
        self.min_btn_bottom.store(
            (rect.origin.y + rect.height()).round() as i32,
            Ordering::Release,
        );
    }

    /// Update close button rectangle (logical pixels).
    pub fn set_close_button_rect(&self, rect: Rect) {
        self.close_btn_left
            .store(rect.origin.x.round() as i32, Ordering::Release);
        self.close_btn_top
            .store(rect.origin.y.round() as i32, Ordering::Release);
        self.close_btn_right.store(
            (rect.origin.x + rect.width()).round() as i32,
            Ordering::Release,
        );
        self.close_btn_bottom.store(
            (rect.origin.y + rect.height()).round() as i32,
            Ordering::Release,
        );
    }

    /// Test whether a logical point falls inside a rect stored as atomic i32.
    fn contains_logical(
        &self,
        x: i32,
        y: i32,
        left: &AtomicI32,
        top: &AtomicI32,
        right: &AtomicI32,
        bottom: &AtomicI32,
    ) -> bool {
        let l = left.load(Ordering::Acquire);
        let t = top.load(Ordering::Acquire);
        let r = right.load(Ordering::Acquire);
        let b = bottom.load(Ordering::Acquire);
        x >= l && x < r && y >= t && y < b
    }
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

    type HWND = *mut c_void;
    type HMONITOR = *mut c_void;
    type LRESULT = isize;
    type UINT = u32;
    type WPARAM = usize;
    type LPARAM = isize;
    type BOOL = i32;
    type DWORD = u32;

    const WM_NCCALCSIZE: UINT = 0x0083;
    const WM_NCHITTEST: UINT = 0x0084;
    const WM_NCDESTROY: UINT = 0x0082;

    const HTCLIENT: LRESULT = 1;
    const HTCAPTION: LRESULT = 2;
    const HTMINBUTTON: LRESULT = 8;
    const HTMAXBUTTON: LRESULT = 9;
    const HTLEFT: LRESULT = 10;
    const HTRIGHT: LRESULT = 11;
    const HTTOP: LRESULT = 12;
    const HTTOPLEFT: LRESULT = 13;
    const HTTOPRIGHT: LRESULT = 14;
    const HTBOTTOM: LRESULT = 15;
    const HTBOTTOMLEFT: LRESULT = 16;
    const HTBOTTOMRIGHT: LRESULT = 17;
    const HTCLOSE: LRESULT = 20;

    const MONITOR_DEFAULTTONEAREST: DWORD = 2;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWORD = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: DWORD = 33;
    const DWMWCP_ROUND: DWORD = 2;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MARGINS {
        cxLeftWidth: i32,
        cxRightWidth: i32,
        cyTopHeight: i32,
        cyBottomHeight: i32,
    }

    #[repr(C)]
    struct MONITORINFO {
        cbSize: DWORD,
        rcMonitor: RECT,
        rcWork: RECT,
        dwFlags: DWORD,
    }

    #[repr(C)]
    struct NCCALCSIZE_PARAMS {
        rgrc: [RECT; 3],
        lppos: *mut c_void,
    }

    type SUBCLASSPROC = unsafe extern "system" fn(
        hWnd: HWND,
        uMsg: UINT,
        wParam: WPARAM,
        lParam: LPARAM,
        uIdSubclass: usize,
        dwRefData: usize,
    ) -> LRESULT;

    #[link(name = "user32")]
    extern "system" {
        fn ScreenToClient(hWnd: HWND, lpPoint: *mut POINT) -> BOOL;
        fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
        fn IsZoomed(hWnd: HWND) -> BOOL;
        fn MonitorFromWindow(hWnd: HWND, dwFlags: DWORD) -> HMONITOR;
        fn GetMonitorInfoW(hMonitor: HMONITOR, lpmi: *mut MONITORINFO) -> BOOL;
        fn SetWindowPos(
            hWnd: HWND,
            hWndInsertAfter: HWND,
            X: i32,
            Y: i32,
            cx: i32,
            cy: i32,
            uFlags: UINT,
        ) -> BOOL;
    }

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(
            hWnd: HWND,
            pfnSubclass: SUBCLASSPROC,
            uIdSubclass: usize,
            dwRefData: usize,
        ) -> BOOL;
        fn DefSubclassProc(hWnd: HWND, uMsg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
        fn RemoveWindowSubclass(hWnd: HWND, pfnSubclass: SUBCLASSPROC, uIdSubclass: usize) -> BOOL;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: HWND,
            dwAttribute: DWORD,
            pvAttribute: *const c_void,
            cbAttribute: DWORD,
        ) -> i32;
        fn DwmExtendFrameIntoClientArea(hwnd: HWND, pMarInset: *const MARGINS) -> i32;
    }

    const SUBCLASS_ID: usize = 0x554E42; // "UNB" in hex

    unsafe extern "system" fn custom_frame_subclass_proc(
        hwnd: HWND,
        msg: UINT,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid: usize,
        ref_data: usize,
    ) -> LRESULT {
        unsafe {
            let metrics_ptr = ref_data as *const FrameHitMetrics;
            if metrics_ptr.is_null() {
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
            let metrics = &*metrics_ptr;

            if !metrics.active.load(Ordering::Acquire) {
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }

            match msg {
                WM_NCCALCSIZE => {
                    if wparam != 0 {
                        let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                        if IsZoomed(hwnd) != 0 {
                            // Maximize mode: clamp to monitor work area to eliminate 8px overflow
                            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                            let mut mi = MONITORINFO {
                                cbSize: core::mem::size_of::<MONITORINFO>() as DWORD,
                                rcMonitor: RECT {
                                    left: 0,
                                    top: 0,
                                    right: 0,
                                    bottom: 0,
                                },
                                rcWork: RECT {
                                    left: 0,
                                    top: 0,
                                    right: 0,
                                    bottom: 0,
                                },
                                dwFlags: 0,
                            };
                            if GetMonitorInfoW(monitor, &mut mi) != 0 {
                                params.rgrc[0] = mi.rcWork;
                            }
                        }
                        // Return 0 indicates client area fills the window rectangle completely
                        return 0;
                    }
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                WM_NCHITTEST => {
                    // Low 16 bits = X, high 16 bits = Y in screen coordinates
                    let screen_x = (lparam & 0xFFFF) as i16 as i32;
                    let screen_y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                    let mut pt = POINT {
                        x: screen_x,
                        y: screen_y,
                    };
                    if ScreenToClient(hwnd, &mut pt) == 0 {
                        return DefSubclassProc(hwnd, msg, wparam, lparam);
                    }

                    let mut client_rect = RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    GetClientRect(hwnd, &mut client_rect);

                    let is_max = IsZoomed(hwnd) != 0;
                    let border_thickness = if is_max { 0 } else { 8 };

                    // 1. Check window borders for resizing
                    if !is_max {
                        let on_left = pt.x < border_thickness;
                        let on_right = pt.x >= client_rect.right - border_thickness;
                        let on_top = pt.y < border_thickness;
                        let on_bottom = pt.y >= client_rect.bottom - border_thickness;

                        if on_top && on_left {
                            return HTTOPLEFT;
                        }
                        if on_top && on_right {
                            return HTTOPRIGHT;
                        }
                        if on_bottom && on_left {
                            return HTBOTTOMLEFT;
                        }
                        if on_bottom && on_right {
                            return HTBOTTOMRIGHT;
                        }
                        if on_top {
                            return HTTOP;
                        }
                        if on_bottom {
                            return HTBOTTOM;
                        }
                        if on_left {
                            return HTLEFT;
                        }
                        if on_right {
                            return HTRIGHT;
                        }
                    }

                    // Convert client physical points to logical coordinates for button hit tests
                    let scale = metrics.scale_factor();
                    let logical_x = (pt.x as f32 / scale).round() as i32;
                    let logical_y = (pt.y as f32 / scale).round() as i32;

                    // 2. Maximize Button -> HTMAXBUTTON (Triggers Win11 Snap Layouts!)
                    if metrics.contains_logical(
                        logical_x,
                        logical_y,
                        &metrics.max_btn_left,
                        &metrics.max_btn_top,
                        &metrics.max_btn_right,
                        &metrics.max_btn_bottom,
                    ) {
                        return HTMAXBUTTON;
                    }

                    // 3. Minimize Button
                    if metrics.contains_logical(
                        logical_x,
                        logical_y,
                        &metrics.min_btn_left,
                        &metrics.min_btn_top,
                        &metrics.min_btn_right,
                        &metrics.min_btn_bottom,
                    ) {
                        return HTMINBUTTON;
                    }

                    // 4. Close Button
                    if metrics.contains_logical(
                        logical_x,
                        logical_y,
                        &metrics.close_btn_left,
                        &metrics.close_btn_top,
                        &metrics.close_btn_right,
                        &metrics.close_btn_bottom,
                    ) {
                        return HTCLOSE;
                    }

                    // 5. Title Bar Blank Area -> HTCAPTION (Aero Drag & Snap)
                    let title_height = metrics.title_bar_height.load(Ordering::Acquire);
                    if title_height > 0 && logical_y < title_height {
                        return HTCAPTION;
                    }

                    // 6. Otherwise regular client interaction
                    HTCLIENT
                }
                WM_NCDESTROY => {
                    RemoveWindowSubclass(hwnd, custom_frame_subclass_proc, SUBCLASS_ID);
                    let _ = Arc::from_raw(metrics_ptr);
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }

    /// Install the Win32 custom frame subclass and enable DWM attributes.
    pub fn install_frame_subclass(hwnd: isize, metrics: Arc<FrameHitMetrics>) -> bool {
        let hwnd_ptr = hwnd as HWND;
        let metrics_raw = Arc::into_raw(metrics);

        let ok = unsafe {
            SetWindowSubclass(
                hwnd_ptr,
                custom_frame_subclass_proc,
                SUBCLASS_ID,
                metrics_raw as usize,
            )
        } != 0;

        if !ok {
            let _ = unsafe { Arc::from_raw(metrics_raw) };
            return false;
        }

        // Apply DWM attributes
        unsafe {
            // Enable Win11 Rounded Corners
            let corner_pref = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd_ptr,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner_pref as *const _ as *const c_void,
                core::mem::size_of::<DWORD>() as DWORD,
            );

            // Enable Win10 / Win11 Drop Shadow
            let margins = MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 1,
                cyBottomHeight: 0,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd_ptr, &margins);

            // Trigger non-client recalculation
            const SWP_NOSIZE: UINT = 0x0001;
            const SWP_NOMOVE: UINT = 0x0002;
            const SWP_NOZORDER: UINT = 0x0004;
            const SWP_FRAMECHANGED: UINT = 0x0020;
            SetWindowPos(
                hwnd_ptr,
                core::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }

        true
    }

    /// Update Immersive Dark Mode on the window title/frame.
    pub fn set_dark_mode(hwnd: isize, dark: bool) {
        let hwnd_ptr = hwnd as HWND;
        let use_dark: BOOL = if dark { 1 } else { 0 };
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd_ptr,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &use_dark as *const _ as *const c_void,
                core::mem::size_of::<BOOL>() as DWORD,
            );
        }
    }
}

/// Install custom frame handling on the given HWND.
pub fn install_custom_frame(hwnd: isize, metrics: Arc<FrameHitMetrics>) -> bool {
    #[cfg(windows)]
    {
        win32::install_frame_subclass(hwnd, metrics)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        let _ = metrics;
        false
    }
}

/// Set DWM dark mode attribute.
pub fn set_window_dark_mode(hwnd: isize, dark: bool) {
    #[cfg(windows)]
    {
        win32::set_dark_mode(hwnd, dark);
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, dark);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_hit_metrics_tracks_and_scales_coordinates() {
        let metrics = FrameHitMetrics::new();
        metrics.set_scale_factor(2.0);
        assert!((metrics.scale_factor() - 2.0).abs() < 1e-4);

        metrics.set_title_bar_height(40.0);
        assert_eq!(metrics.title_bar_height.load(Ordering::Acquire), 40);

        let max_r = Rect::new(700.0, 0.0, 46.0, 40.0);
        metrics.set_maximize_button_rect(max_r);

        assert!(metrics.contains_logical(
            720,
            20,
            &metrics.max_btn_left,
            &metrics.max_btn_top,
            &metrics.max_btn_right,
            &metrics.max_btn_bottom,
        ));

        assert!(!metrics.contains_logical(
            600,
            20,
            &metrics.max_btn_left,
            &metrics.max_btn_top,
            &metrics.max_btn_right,
            &metrics.max_btn_bottom,
        ));
    }
}

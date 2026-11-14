//! 适用于 Windows 10 与 11 的 Win32 无边框窗口、贴靠布局（Snap Layouts）与 DWM 集成。
//!
//! 基于官方 `windows = "0.62.2"` crate 提供以下能力：
//! - 无缝无边框窗口，通过 `WM_NCCALCSIZE` 钳制到显示器工作区（最大化零溢出）。
//! - 原生 `WM_NCHITTEST` 子类化：
//!   - 在自定义最大化按钮上映射 `HTMAXBUTTON`，激活 Windows 11 贴靠布局悬停菜单。
//!   - 在标题栏区域映射 `HTCAPTION`，支持系统级窗口拖拽与 Aero 贴靠（`Win + 方向键`）。
//!   - 按 DPI 缩放比例映射 `HTLEFT..HTBOTTOMRIGHT` 实现窗口四边与四角拖拽调整大小。
//! - DWM 窗口属性：
//!   - Windows 11 圆角窗口（`DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`）。
//!   - 沉浸式深色模式同步（`DWMWA_USE_IMMERSIVE_DARK_MODE`）。
//!   - Windows 10 / 11 窗口阴影恢复（`DwmExtendFrameIntoClientArea`）。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;

pub use unbound_math::Rect;

/// 用于自定义标题栏与窗口按钮命中测试的原子指标。
#[derive(Debug, Default)]
pub struct FrameHitMetrics {
    /// 标题栏高度（单位：逻辑像素）。为 `0` 时标题栏拖拽交由客户端逻辑判断。
    pub title_bar_height: AtomicI32,
    /// 最大化按钮包围盒（单位：逻辑像素）。
    pub max_btn_left: AtomicI32,
    pub max_btn_top: AtomicI32,
    pub max_btn_right: AtomicI32,
    pub max_btn_bottom: AtomicI32,
    /// 最小化按钮包围盒（单位：逻辑像素）。
    pub min_btn_left: AtomicI32,
    pub min_btn_top: AtomicI32,
    pub min_btn_right: AtomicI32,
    pub min_btn_bottom: AtomicI32,
    /// 关闭按钮包围盒（单位：逻辑像素）。
    pub close_btn_left: AtomicI32,
    pub close_btn_top: AtomicI32,
    pub close_btn_right: AtomicI32,
    pub close_btn_bottom: AtomicI32,
    /// DPI 缩放因子乘以 1000（例如 1.5x 缩放对应 1500）。
    pub scale_permille: AtomicU32,
    /// 自定义边框钩子是否处于激活状态。
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

    /// 更新缩放因子。
    pub fn set_scale_factor(&self, factor: f32) {
        let permille = (factor.clamp(0.25, 5.0) * 1000.0).round() as u32;
        self.scale_permille.store(permille, Ordering::Release);
    }

    /// 读取缩放因子。
    #[must_use]
    pub fn scale_factor(&self) -> f32 {
        self.scale_permille.load(Ordering::Acquire) as f32 / 1000.0
    }

    /// 更新标题栏高度（逻辑像素）。
    pub fn set_title_bar_height(&self, height: f32) {
        self.title_bar_height
            .store(height.round() as i32, Ordering::Release);
    }

    /// 更新最大化按钮矩形（逻辑像素）。
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

    /// 更新最小化按钮矩形（逻辑像素）。
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

    /// 更新关闭按钮矩形（逻辑像素）。
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

    /// 测试逻辑坐标点是否落入以原子 i32 存储的矩形内。
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
mod win32 {
    use super::*;
    use core::ffi::c_void;

    use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use ::windows::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use ::windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use ::windows::Win32::UI::Controls::MARGINS;
    use ::windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, IsZoomed, SetWindowPos, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
        HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
        NCCALCSIZE_PARAMS, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE,
        WM_NCDESTROY, WM_NCHITTEST,
    };

    const SUBCLASS_ID: usize = 0x554E42; // "UNB" in hex

    #[inline]
    fn hwnd_from_isize(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    unsafe extern "system" fn custom_frame_subclass_proc(
        hwnd: HWND,
        msg: u32,
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
                    if wparam.0 != 0 {
                        let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                        if IsZoomed(hwnd).as_bool() {
                            // 最大化模式：钳制到显示器工作区以消除 8px 溢出
                            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                            let mut mi = MONITORINFO {
                                cbSize: core::mem::size_of::<MONITORINFO>() as u32,
                                ..Default::default()
                            };
                            if GetMonitorInfoW(monitor, &mut mi).as_bool() {
                                params.rgrc[0] = mi.rcWork;
                            }
                        }
                        // 返回 0 表示客户区完全填满整个窗口矩形
                        return LRESULT(0);
                    }
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                WM_NCHITTEST => {
                    // 低 16 位 = X，高 16 位 = Y（屏幕坐标）
                    let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
                    let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                    let mut pt = POINT {
                        x: screen_x,
                        y: screen_y,
                    };
                    if !ScreenToClient(hwnd, &mut pt).as_bool() {
                        return DefSubclassProc(hwnd, msg, wparam, lparam);
                    }

                    let mut client_rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client_rect);

                    let is_max = IsZoomed(hwnd).as_bool();
                    let scale = metrics.scale_factor().max(0.5);
                    let border_thickness = if is_max {
                        0
                    } else {
                        (6.0 * scale).round().max(4.0) as i32
                    };
                    let corner_extent = if is_max {
                        0
                    } else {
                        (16.0 * scale).round().max(8.0) as i32
                    };

                    // 将客户区物理坐标转换为逻辑坐标，用于窗口按钮命中测试。
                    let logical_x = (pt.x as f32 / scale).round() as i32;
                    let logical_y = (pt.y as f32 / scale).round() as i32;

                    // 1. 窗口按钮优先于边框缩放：按钮锚定在右上角，若先判定边框，
                    // 其上沿会被误判为 HTTOP / HTTOPRIGHT 而无法点击。
                    // 最大化按钮返回 HTMAXBUTTON 可触发 Win11 贴靠布局。
                    if metrics.contains_logical(
                        logical_x,
                        logical_y,
                        &metrics.max_btn_left,
                        &metrics.max_btn_top,
                        &metrics.max_btn_right,
                        &metrics.max_btn_bottom,
                    ) {
                        return LRESULT(HTMAXBUTTON as isize);
                    }

                    // 2. 最小化按钮。
                    if metrics.contains_logical(
                        logical_x,
                        logical_y,
                        &metrics.min_btn_left,
                        &metrics.min_btn_top,
                        &metrics.min_btn_right,
                        &metrics.min_btn_bottom,
                    ) {
                        return LRESULT(HTMINBUTTON as isize);
                    }

                    // 3. 关闭按钮。
                    if metrics.contains_logical(
                        logical_x,
                        logical_y,
                        &metrics.close_btn_left,
                        &metrics.close_btn_top,
                        &metrics.close_btn_right,
                        &metrics.close_btn_bottom,
                    ) {
                        return LRESULT(HTCLOSE as isize);
                    }

                    // 4. 检查用于调整大小的窗口边框（四角采用 corner_extent 扩大热区）。
                    if !is_max {
                        let on_left = pt.x < border_thickness;
                        let on_right = pt.x >= client_rect.right - border_thickness;
                        let on_top = pt.y < border_thickness;
                        let on_bottom = pt.y >= client_rect.bottom - border_thickness;

                        let in_left_corner = pt.x < corner_extent;
                        let in_right_corner = pt.x >= client_rect.right - corner_extent;
                        let in_top_corner = pt.y < corner_extent;
                        let in_bottom_corner = pt.y >= client_rect.bottom - corner_extent;

                        if (on_top && in_left_corner) || (on_left && in_top_corner) {
                            return LRESULT(HTTOPLEFT as isize);
                        }
                        if (on_top && in_right_corner) || (on_right && in_top_corner) {
                            return LRESULT(HTTOPRIGHT as isize);
                        }
                        if (on_bottom && in_left_corner) || (on_left && in_bottom_corner) {
                            return LRESULT(HTBOTTOMLEFT as isize);
                        }
                        if (on_bottom && in_right_corner) || (on_right && in_bottom_corner) {
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

                    // 5. 标题栏空白区域 -> HTCAPTION（Aero 拖拽与贴靠）。
                    let title_height = metrics.title_bar_height.load(Ordering::Acquire);
                    if title_height > 0 && logical_y < title_height {
                        return LRESULT(HTCAPTION as isize);
                    }

                    // 6. 其余情况交由常规客户区交互处理。
                    LRESULT(HTCLIENT as isize)
                }
                WM_NCDESTROY => {
                    let _ =
                        RemoveWindowSubclass(hwnd, Some(custom_frame_subclass_proc), SUBCLASS_ID);
                    let _ = Arc::from_raw(metrics_ptr);
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }

    /// 为无边框窗口应用 DWM 圆角与阴影（不涉及消息子类化）。
    ///
    /// `WS_POPUP | WS_THICKFRAME` 无边框窗口通过 `WM_NCCALCSIZE` 将非客户区裁切为 0，
    /// 把顶边 1px 扩展进客户区并请求圆角，可在不引入系统标题栏的前提下恢复阴影与圆角。
    /// 该函数是幂等的，窗口创建时与自定义边框子类安装时各调用一次均无副作用。
    pub fn apply_borderless_dwm(hwnd: isize) {
        let hwnd_ptr = hwnd_from_isize(hwnd);
        unsafe {
            // 启用 Win11 圆角。
            let corner_pref = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd_ptr,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner_pref).cast::<c_void>(),
                core::mem::size_of_val(&corner_pref) as u32,
            );

            // 启用 Win10 / Win11 窗口阴影。
            let margins = MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 1,
                cyBottomHeight: 0,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd_ptr, &margins);

            // 触发非客户区重新计算。
            let _ = SetWindowPos(
                hwnd_ptr,
                None,
                0,
                0,
                0,
                0,
                SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
    }

    /// 安装 Win32 自定义边框子类化并启用 DWM 属性。
    pub fn install_frame_subclass(hwnd: isize, metrics: Arc<FrameHitMetrics>) -> bool {
        let hwnd_ptr = hwnd_from_isize(hwnd);
        let metrics_raw = Arc::into_raw(metrics);

        let ok = unsafe {
            SetWindowSubclass(
                hwnd_ptr,
                Some(custom_frame_subclass_proc),
                SUBCLASS_ID,
                metrics_raw as usize,
            )
        }
        .as_bool();

        if !ok {
            let _ = unsafe { Arc::from_raw(metrics_raw) };
            return false;
        }

        apply_borderless_dwm(hwnd);
        true
    }

    /// 更新窗口标题栏/边框的沉浸式深色模式。
    pub fn set_dark_mode(hwnd: isize, dark: bool) {
        let hwnd_ptr = hwnd_from_isize(hwnd);
        // windows 0.62.2 未公开 BOOL 类型；DWM 深色模式属性需要的是 32 位布尔值，直接用 i32。
        let use_dark = i32::from(dark);
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd_ptr,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const use_dark).cast::<c_void>(),
                core::mem::size_of_val(&use_dark) as u32,
            );
        }
    }
}

/// 在给定 HWND 上安装自定义边框处理。
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

/// 为无边框窗口应用 DWM 圆角与阴影。
///
/// 由窗口创建路径在 `decorations = false` 时调用，使未安装自定义边框子类
/// 的窗口同样保留系统阴影与圆角。
pub fn apply_borderless_dwm(hwnd: isize) {
    #[cfg(windows)]
    {
        win32::apply_borderless_dwm(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
    }
}

/// 设置 DWM 深色模式属性。
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

//! 跨平台原生窗口句柄抽象。
//!
//! 对外隐藏 Win32 `HWND` 或 Wayland `wl_surface` 等具体平台句柄类型，
//! 为 `unbound-app` 与上层应用提供统一的窗口控制、外观下发、`raw-window-handle`
//! 导出与 Vulkan 表面创建能力；Win32 专属底层句柄仅保留在 `#[cfg(windows)]`
//! 逃生舱方法上。

use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::win32_frame::{
    clear_window_accent_tint, install_custom_frame, set_window_accent_tint, set_window_backdrop,
    set_window_dark_mode, FrameHitMetrics, WindowBackdropMaterial,
};
use crate::win32_window::{
    CursorIcon, PhysicalPosition, PhysicalSize, ResizeDirection, Window as Win32Window,
    WindowId as NativeWindowId,
};
use crate::floating::{FloatingWindowCapabilities, MonitorWorkArea};
use crate::window::{PlatformError, ScreenRect};

/// 跨平台原生窗口内部后端分派枚举。
pub(crate) enum NativeWindowInner {
    /// Win32 后端窗口句柄。
    Win32(Win32Window),
    /// Linux 原生 Wayland 后端窗口句柄。
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    Wayland(crate::wayland_backend::WaylandWindowHandle),
    /// Linux 原生 X11 (XCB) 后端窗口句柄。
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    X11(crate::x11_backend::X11WindowHandle),
}

/// 跨平台原生窗口句柄。
///
/// 统一承载窗口尺寸/缩放查询、光标、标题、最小化/最大化/关闭、CSD 拖拽缩放、
/// 外观属性下发、自定义无边框命中安装以及 Vulkan 表面创建。
pub struct NativeWindow {
    inner: NativeWindowInner,
}

impl NativeWindow {
    /// 从 Win32 窗口对象构造跨平台句柄。
    #[must_use]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn from_win32(window: Win32Window) -> Self {
        Self {
            inner: NativeWindowInner::Win32(window),
        }
    }

    /// 从 Wayland 窗口句柄构造跨平台句柄。
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    #[must_use]
    pub(crate) fn from_wayland(window: crate::wayland_backend::WaylandWindowHandle) -> Self {
        Self {
            inner: NativeWindowInner::Wayland(window),
        }
    }

    /// 从 X11 窗口句柄构造跨平台句柄。
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    #[must_use]
    pub(crate) fn from_x11(window: crate::x11_backend::X11WindowHandle) -> Self {
        Self {
            inner: NativeWindowInner::X11(window),
        }
    }

    /// Windows 专属逃生舱：获取底层 Win32 `HWND` 原始整数值。
    #[cfg(windows)]
    #[must_use]
    pub fn win32_hwnd(&self) -> Option<isize> {
        match &self.inner {
            NativeWindowInner::Win32(w) => Some(w.hwnd()),
        }
    }

    /// 供 `unbound-platform` 内部使用的 Win32 句柄访问。
    #[cfg(windows)]
    #[must_use]
    pub(crate) fn raw_win32_hwnd(&self) -> isize {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.hwnd(),
        }
    }

    /// 当前窗口是否为无激活越界原生弹出窗。
    #[must_use]
    pub fn is_popup(&self) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.is_popup(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.is_popup(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.is_popup(),
        }
    }

    /// 若当前窗口为原生弹出窗，返回其绑定的业务浮层 ID。
    #[must_use]
    pub fn popup_id(&self) -> Option<u64> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.popup_id(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.popup_id(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.popup_id(),
        }
    }

    /// 返回平台窗口唯一标识。
    #[must_use]
    pub fn id(&self) -> NativeWindowId {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.id(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.id(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.id(),
        }
    }

    /// 获取当前选中的光标样式。
    #[must_use]
    pub fn current_cursor(&self) -> CursorIcon {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.current_cursor(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.current_cursor(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.current_cursor(),
        }
    }

    /// 获取客户区物理像素尺寸。
    #[must_use]
    pub fn inner_size(&self) -> PhysicalSize {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.inner_size(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.inner_size(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.inner_size(),
        }
    }

    /// 获取客户区在屏幕坐标系中的物理矩形。
    #[must_use]
    pub fn client_rect_on_screen(&self) -> ScreenRect {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.client_rect_on_screen(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.client_rect_on_screen(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.client_rect_on_screen(),
        }
    }

    /// 获取窗口当前 DPI 缩放因子（`1.0` 对应 96 DPI）。
    #[must_use]
    pub fn scale_factor(&self) -> f64 {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.scale_factor(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.scale_factor(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.scale_factor(),
        }
    }

    /// 获取窗口所在显示器的垂直刷新率（Hz）；不可用时返回 `None`。
    #[must_use]
    pub fn refresh_rate_hz(&self) -> Option<f32> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.refresh_rate_hz(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.refresh_rate_hz(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.refresh_rate_hz(),
        }
    }

    /// 窗口当前是否可见。
    #[must_use]
    pub fn is_visible(&self) -> Option<bool> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.is_visible(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.is_visible(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.is_visible(),
        }
    }

    /// 窗口当前是否处于最大化状态。
    #[must_use]
    pub fn is_maximized(&self) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.is_maximized(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.is_maximized(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.is_maximized(),
        }
    }

    /// 窗口是否使用系统原生装饰。
    #[must_use]
    pub fn is_decorated(&self) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.is_decorated(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.is_decorated(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.is_decorated(),
        }
    }

    /// 设置窗口可见性。
    pub fn set_visible(&self, visible: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_visible(visible),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_visible(visible),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_visible(visible),
        }
    }

    /// 设置窗口可见性且不抢占前台焦点。
    pub fn set_visible_no_activate(&self, visible: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_visible_no_activate(visible),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_visible_no_activate(visible),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_visible_no_activate(visible),
        }
    }

    /// 运行时无闪烁切换窗口是否全局置顶。
    pub fn set_always_on_top(&self, topmost: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_always_on_top(topmost),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_always_on_top(topmost),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_always_on_top(topmost),
        }
    }

    /// 运行时无闪烁切换窗口鼠标穿透状态。
    pub fn set_click_through(&self, click_through: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_click_through(click_through),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_click_through(click_through),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_click_through(click_through),
        }
    }

    /// 运行时设置窗口外框在屏幕坐标系中的物理矩形（不激活窗口）。
    pub fn set_outer_rect(&self, rect: ScreenRect) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_outer_rect(rect),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_outer_rect(rect),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_outer_rect(rect),
        }
    }

    /// 查询当前窗口后端的浮窗能力描述。
    #[must_use]
    pub fn capabilities(&self) -> FloatingWindowCapabilities {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.capabilities(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.capabilities(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.capabilities(),
        }
    }

    /// 查询当前窗口所在显示器的工作区与 DPI 信息。
    #[must_use]
    pub fn work_area(&self) -> Option<MonitorWorkArea> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.work_area(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.work_area(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.work_area(),
        }
    }

    /// 最小化或还原窗口。
    pub fn set_minimized(&self, minimized: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_minimized(minimized),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_minimized(minimized),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_minimized(minimized),
        }
    }

    /// 最大化或还原窗口。
    pub fn set_maximized(&self, maximized: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_maximized(maximized),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_maximized(maximized),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_maximized(maximized),
        }
    }

    /// 更新窗口标题。
    pub fn set_title(&self, title: &str) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_title(title),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_title(title),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_title(title),
        }
    }

    /// 设置窗口光标图标。
    pub fn set_cursor(&self, cursor: CursorIcon) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_cursor(cursor),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_cursor(cursor),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_cursor(cursor),
        }
    }

    /// 请求重绘一帧。
    pub fn request_redraw(&self) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.request_redraw(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.request_redraw(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.request_redraw(),
        }
    }

    /// 请求关闭窗口。
    pub fn request_close(&self) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.request_close(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.request_close(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.request_close(),
        }
    }

    /// 窗口是否已被销毁。
    #[must_use]
    pub fn is_destroyed(&self) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.is_destroyed(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.is_destroyed(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.is_destroyed(),
        }
    }

    /// 发起自绘标题栏拖拽移动。
    ///
    /// # Errors
    /// 平台拒绝拖拽请求时返回 [`PlatformError`]。
    pub fn drag_window(&self) -> Result<(), PlatformError> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.drag_window(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.drag_window(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.drag_window(),
        }
    }

    /// 发起窗口边缘/四角拖拽缩放。
    ///
    /// # Errors
    /// 平台拒绝缩放请求时返回 [`PlatformError`]。
    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), PlatformError> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.drag_resize_window(direction),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.drag_resize_window(direction),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.drag_resize_window(direction),
        }
    }

    /// 启用或禁用输入法组合。
    pub fn set_ime_allowed(&self, allowed: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_ime_allowed(allowed),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_ime_allowed(allowed),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_ime_allowed(allowed),
        }
    }

    /// 更新输入法候选框锚点区域。
    pub fn set_ime_cursor_area(&self, position: PhysicalPosition, size: PhysicalSize) {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.set_ime_cursor_area(position, size),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.set_ime_cursor_area(position, size),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.set_ime_cursor_area(position, size),
        }
    }

    /// 安装无边框自定义帧命中指标（Windows 下接管 `WM_NCHITTEST` 与 Snap Layout，Wayland 下用于 CSD 命中）。
    pub fn install_custom_frame(&self, metrics: Arc<FrameHitMetrics>) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => install_custom_frame(w.hwnd(), metrics),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.install_custom_frame(metrics),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.install_custom_frame(metrics),
        }
    }

    /// 下发沉浸式深色模式外观标记（不支持的平台安全无操作）。
    pub fn set_dark_mode(&self, dark: bool) {
        match &self.inner {
            NativeWindowInner::Win32(w) => set_window_dark_mode(w.hwnd(), dark),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(_) | NativeWindowInner::X11(_) => {
                let _ = dark;
            }
        }
    }

    /// 下发系统后景材质（不支持的平台安全返回 `false`）。
    pub fn set_backdrop(&self, material: WindowBackdropMaterial) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => set_window_backdrop(w.hwnd(), material),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(_) | NativeWindowInner::X11(_) => {
                let _ = material;
                false
            }
        }
    }

    /// 下发系统亚克力自定义色调（不支持的平台安全返回 `false`）。
    pub fn set_accent_tint(&self, argb: u32) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => set_window_accent_tint(w.hwnd(), argb),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(_) | NativeWindowInner::X11(_) => {
                let _ = argb;
                false
            }
        }
    }

    /// 清除系统亚克力自定义色调（不支持的平台安全返回 `false`）。
    pub fn clear_accent_tint(&self) -> bool {
        match &self.inner {
            NativeWindowInner::Win32(w) => clear_window_accent_tint(w.hwnd()),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(_) | NativeWindowInner::X11(_) => false,
        }
    }

    /// 基于当前窗口的跨平台 `raw-window-handle` 句柄创建 Vulkan 呈现表面。
    ///
    /// # Errors
    /// 获取原生句柄失败、缺少实例扩展或 Vulkan 建链失败时返回错误。
    pub fn create_vulkan_surface(
        &self,
        gpu: &unbound_gpu::device::Gpu,
    ) -> Result<unbound_gpu::surface::Surface, unbound_gpu::device::GpuError> {
        let display = self
            .display_handle()
            .map_err(|e| {
                unbound_gpu::device::GpuError::Allocation(format!(
                    "window display handle unavailable: {e}"
                ))
            })?
            .as_raw();
        let window = self
            .window_handle()
            .map_err(|e| {
                unbound_gpu::device::GpuError::Allocation(format!("window handle unavailable: {e}"))
            })?
            .as_raw();
        // SAFETY: `self` 在整个调用期间保持存活，底层显示与窗口句柄有效。
        unsafe { unbound_gpu::surface::Surface::from_raw_handles(gpu, display, window) }
    }
}

impl HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.window_handle(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.window_handle(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.window_handle(),
        }
    }
}

impl HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        match &self.inner {
            NativeWindowInner::Win32(w) => w.display_handle(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::Wayland(w) => w.display_handle(),
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
            NativeWindowInner::X11(w) => w.display_handle(),
        }
    }
}

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

/// Windows 11 DWM 窗口背景材质（对齐下游渲染工具的 5 档材质枚举）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowBackdropMaterial {
    /// 跟随系统默认策略（`DWMSBT_AUTO`）。
    #[default]
    System,
    /// 标准云母材质（`DWMSBT_MAINWINDOW`）。
    Mica,
    /// 高浓度云母标签页材质（`DWMSBT_TABBEDWINDOW`）。
    Tabbed,
    /// 亚克力毛玻璃材质（`DWMSBT_TRANSIENTWINDOW`）。
    Acrylic,
    /// 禁用背景材质，回退至纯色不透明背景（`DWMSBT_NONE`）。
    None,
}

impl WindowBackdropMaterial {
    /// 转换为 Win32 `DWM_SYSTEMBACKDROP_TYPE` 整数常量。
    #[must_use]
    pub const fn dwm_backdrop_type_raw(self) -> i32 {
        match self {
            Self::System => 0,  // DWMSBT_AUTO
            Self::None => 1,    // DWMSBT_NONE
            Self::Mica => 2,    // DWMSBT_MAINWINDOW
            Self::Acrylic => 3, // DWMSBT_TRANSIENTWINDOW
            Self::Tabbed => 4,  // DWMSBT_TABBEDWINDOW
        }
    }
}

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
    /// 是否锁定 `WM_NCACTIVATE` 的 `wParam = TRUE`，防止窗口失焦时 Win11 云母/亚克力材质退化为纯色。
    pub lock_mica_active: AtomicBool,
    /// 透明覆盖层模式下的动态鼠标穿透策略（为 `Some` 时非交互区返回 `HTTRANSPARENT`）。
    pub pass_through_policy:
        std::sync::RwLock<Option<crate::overlay_host::HitTestPassThroughPolicy>>,
}

impl FrameHitMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            scale_permille: AtomicU32::new(1000),
            active: AtomicBool::new(true),
            lock_mica_active: AtomicBool::new(false),
            pass_through_policy: std::sync::RwLock::new(None),
            ..Default::default()
        }
    }

    /// 设置是否在窗口失焦时锁定云母/亚克力激活态（拦截 `WM_NCACTIVATE` 强制 `wParam = 1`）。
    pub fn set_lock_mica_active(&self, lock: bool) {
        self.lock_mica_active.store(lock, Ordering::Release);
    }

    /// 更新覆盖层窗口的动态鼠标穿透策略。
    pub fn set_pass_through_policy(
        &self,
        policy: Option<crate::overlay_host::HitTestPassThroughPolicy>,
    ) {
        if let Ok(mut guard) = self.pass_through_policy.write() {
            *guard = policy;
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

    /// 把 UI 逻辑像素包围盒换算为 `WM_NCHITTEST` 使用的 Win32 逻辑像素。
    ///
    /// UI 坐标系已被 [`crate::backend`] 按 `ui_scale` 压缩过（`widget_rect` 处于该空间），
    /// 因此需要先乘 `ui_scale` 还原物理像素，再除以窗口 DPI 缩放得到 Win32 逻辑像素。
    /// 两者混用会让 `HTMAXBUTTON` 等热区在高 DPI 且 `ui_scale != 1.0` 时二次偏移，
    /// 直接破坏 Win11 贴靠布局的悬停对齐。
    #[must_use]
    pub fn ui_rect_to_frame_logical(rect: Rect, ui_scale: f32, scale_factor: f32) -> Rect {
        let ui_scale = ui_scale.max(0.5);
        let scale_factor = scale_factor.max(0.5);
        let factor = ui_scale / scale_factor;
        Rect::new(
            rect.origin.x * factor,
            rect.origin.y * factor,
            rect.size.x * factor,
            rect.size.y * factor,
        )
    }
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use core::ffi::c_void;

    use ::windows::core::BOOL;
    use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use ::windows::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use ::windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use ::windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use ::windows::Win32::UI::Controls::MARGINS;
    use ::windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, IsZoomed, SetWindowPos, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
        HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
        NCCALCSIZE_PARAMS, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE,
        WM_NCDESTROY, WM_NCHITTEST,
    };

    const SUBCLASS_ID: usize = 0x554E42; // "UNB" in hex

    /// `DWMWA_SYSTEMBACKDROP_TYPE` 的属性编号。
    const DWMWA_SYSTEMBACKDROP_TYPE_ID: i32 = 38;

    /// 沉浸式后景的未公开系统取值候选。
    ///
    /// 公开头文件（Windows SDK 10.0.26100）里的 `DWM_SYSTEMBACKDROP_TYPE` 只定义了
    /// 0..=4；沉浸式后景若真有系统取值，只能落在公开区间之外的这几个数上。这里只放
    /// 候选，不代表它们被系统接受——是否接受由 [`probe_immersive_backdrop`] 实测。
    const IMMERSIVE_BACKDROP_CANDIDATES: [i32; 3] = [5, 6, 7];

    /// 回读探测的负对照哨兵：几个必然非法的取值。
    ///
    /// 本机实测（Windows 11 build 26100）证明 `DwmSetWindowAttribute` 对 0..=4 之外的
    /// 任意整数都返回 `S_OK`，而 `DwmGetWindowAttribute` 会原样回读，包括 `999` 与
    /// `65536`。所以「读回等于写入」本身没有判别力；只有先确认哨兵会被系统拒绝，
    /// 候选值的回读相等才可能有意义。这里避开了 `0x7FFFFFFF` 这类极端值：实测写入它
    /// 会让 `dwmapi` 在进程内触发访问冲突，探测代码绝不能拿它当探针。
    const BACKDROP_PROBE_SENTINELS: [i32; 2] = [999, 0x0001_0000];

    /// `WCA_ACCENT_POLICY`：`SetWindowCompositionAttribute` 的 accent policy 属性编号。
    ///
    /// 该枚举从未出现在公开头文件里，取值来自社区逆向资料（与 `windows` crate 里
    /// 未收录的 `WINDOWCOMPOSITIONATTRIB` 一致）。
    const WCA_ACCENT_POLICY: u32 = 19;

    /// `ACCENT_ENABLE_ACRYLICBLURBEHIND`：亚克力模糊后景。
    ///
    /// 公开资料普遍记载为 Windows 10 1803（build 17134）引入。未公开意味着不能
    /// 把它当稳定契约：系统可能静默忽略，宿主必须按返回值判断是否真正生效。
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

    /// `ACCENT_ENABLE_NONE`：关闭 accent policy，恢复默认合成。
    const ACCENT_ENABLE_NONE: u32 = 0;

    /// 社区逆向资料里的 `ACCENT_POLICY` 布局（4 个 32 位字段）。
    #[repr(C)]
    #[derive(Copy, Clone, Debug, Default)]
    struct AccentPolicy {
        /// `ACCENT_STATE` 取值。
        accent_state: u32,
        /// 额外的绘制标志位；保持 0 使用默认行为。
        accent_flags: u32,
        /// 亚克力色调，ABGR 顺序，alpha 分量参与半透明。
        gradient_color: u32,
        /// 动画编号；保持 0 表示不使用动画。
        animation_id: u32,
    }

    /// `WINDOWCOMPOSITIONATTRIBDATA` 布局：属性编号、数据指针与字节长度。
    #[repr(C)]
    struct WindowCompositionAttributeData {
        /// `WINDOWCOMPOSITIONATTRIB` 取值。
        attrib: u32,
        /// 指向具体属性结构体的指针。
        data: *mut c_void,
        /// 属性结构体的字节长度。
        size: usize,
    }

    /// 未公开导出 `SetWindowCompositionAttribute` 的函数签名。
    type SetWindowCompositionAttributeFn =
        unsafe extern "system" fn(HWND, *mut WindowCompositionAttributeData) -> BOOL;

    /// 动态解析 `SetWindowCompositionAttribute`。
    ///
    /// 该导出不在任何公开头文件与导入库里，因此只能用 `GetProcAddress` 运行时解析，
    /// 绝不能写成硬依赖导入。解析失败返回 `None`，调用方据此判定该通道不可用。
    fn resolve_set_window_composition_attribute() -> Option<SetWindowCompositionAttributeFn> {
        use ::windows::core::{s, w};

        unsafe {
            let module = GetModuleHandleW(w!("user32.dll")).ok()?;
            let proc = GetProcAddress(module, s!("SetWindowCompositionAttribute"))?;
            Some(core::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                SetWindowCompositionAttributeFn,
            >(proc))
        }
    }

    /// 下发一次 accent policy。语义见外层 [`super::set_window_accent_tint`]。
    fn set_accent_policy(hwnd: isize, state: u32, gradient_color: u32) -> bool {
        if hwnd == 0 {
            return false;
        }
        let Some(set_window_composition_attribute) = resolve_set_window_composition_attribute()
        else {
            return false;
        };
        let mut policy = AccentPolicy {
            accent_state: state,
            accent_flags: 0,
            gradient_color,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttributeData {
            attrib: WCA_ACCENT_POLICY,
            data: (&raw mut policy).cast::<c_void>(),
            size: core::mem::size_of_val(&policy),
        };
        unsafe { set_window_composition_attribute(hwnd_from_isize(hwnd), &mut data).as_bool() }
    }

    /// 亚克力自定义色调：开启 `ACCENT_ENABLE_ACRYLICBLURBEHIND` 并写入色调。
    pub fn set_accent_tint(hwnd: isize, argb: u32) -> bool {
        set_accent_policy(hwnd, ACCENT_ENABLE_ACRYLICBLURBEHIND, argb)
    }

    /// 关闭 accent policy，把窗口交还给默认合成（含 DWM 材质通道）。
    pub fn clear_accent_tint(hwnd: isize) -> bool {
        set_accent_policy(hwnd, ACCENT_ENABLE_NONE, 0)
    }

    /// 运行时能否解析出该未公开导出。语义见外层
    /// [`super::accent_policy_export_available`]。
    pub fn accent_policy_export_available() -> bool {
        resolve_set_window_composition_attribute().is_some()
    }

    #[inline]
    fn hwnd_from_isize(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    /// 探测沉浸式后景的系统取值（Win32 实现）。语义见外层
    /// [`super::probe_immersive_backdrop`]。
    pub fn probe_immersive_backdrop(hwnd: isize) -> Option<i32> {
        use ::windows::Win32::Graphics::Dwm::{
            DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
        };

        if hwnd == 0 {
            return None;
        }
        let hwnd_ptr = hwnd_from_isize(hwnd);

        let write = |value: i32| unsafe {
            DwmSetWindowAttribute(
                hwnd_ptr,
                DWMWINDOWATTRIBUTE(DWMWA_SYSTEMBACKDROP_TYPE_ID),
                (&raw const value).cast::<c_void>(),
                core::mem::size_of_val(&value) as u32,
            )
            .is_ok()
        };
        let read = || -> Option<i32> {
            let mut value: i32 = 0;
            let cb = core::mem::size_of_val(&value) as u32;
            unsafe {
                DwmGetWindowAttribute(
                    hwnd_ptr,
                    DWMWINDOWATTRIBUTE(DWMWA_SYSTEMBACKDROP_TYPE_ID),
                    (&raw mut value).cast::<c_void>(),
                    cb,
                )
            }
            .ok()?;
            Some(value)
        };

        let original = read();

        // 负对照：只要任一非法哨兵被原样回读，就说明该属性只做整数存储、不做取值
        // 校验，回读相等毫无证据价值——直接判定为「无法确认」，绝不猜数字。
        let sentinels_are_rejected = BACKDROP_PROBE_SENTINELS
            .iter()
            .all(|sentinel| write(*sentinel) && read().is_some_and(|value| value != *sentinel));

        let mut confirmed = None;
        if sentinels_are_rejected {
            for candidate in IMMERSIVE_BACKDROP_CANDIDATES {
                if write(candidate) && read() == Some(candidate) {
                    confirmed = Some(candidate);
                    break;
                }
            }
        }

        // 恢复原值，避免探测把窗口留在非法材质上。
        if let Some(original) = original {
            let _ = write(original);
        }
        confirmed
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
                    }
                    // 无论 wparam 为 0 还是非 0，均直接返回 0，使客户区完全填满整个窗口矩形，
                    // 避免 WS_THICKFRAME 在首次移动或样式刷新时被默认过程扣减非客户区边框。
                    LRESULT(0)
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

                    // 6. 若配置了透明覆盖层穿透策略，则对非交互空白区返回 HTTRANSPARENT (-1)。
                    if let Ok(guard) = metrics.pass_through_policy.read() {
                        if let Some(policy) = guard.as_ref() {
                            let pt_logical = unbound_math::Vec2::new(logical_x as f32, logical_y as f32);
                            if policy.evaluate(pt_logical)
                                == crate::overlay_host::OverlayHitDisposition::PassThrough
                            {
                                return LRESULT(-1); // HTTRANSPARENT
                            }
                        }
                    }

                    // 7. 其余情况交由常规客户区交互处理。
                    LRESULT(HTCLIENT as isize)
                }
                0x0086 /* WM_NCACTIVATE */ => {
                    // 传入 LPARAM(-1) 阻止 DefWindowProc 重绘非客户区 GDI 边框/旧标题栏，
                    // 同时保留窗口激活状态切换与 DWM 动画语义。
                    if metrics.lock_mica_active.load(Ordering::Acquire) {
                        return DefSubclassProc(hwnd, msg, WPARAM(1), LPARAM(-1));
                    }
                    DefSubclassProc(hwnd, msg, wparam, LPARAM(-1))
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

/// 设置 Windows 11 DWM 系统背景材质（`DWMWA_SYSTEMBACKDROP_TYPE = 38`）。
pub fn set_window_backdrop(hwnd: isize, material: WindowBackdropMaterial) -> bool {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::HWND;
        use ::windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
        use core::ffi::c_void;
        const DWMWA_SYSTEMBACKDROP_TYPE_ID: i32 = 38;
        let hwnd_ptr = HWND(hwnd as *mut c_void);
        let val: i32 = material.dwm_backdrop_type_raw();
        unsafe {
            DwmSetWindowAttribute(
                hwnd_ptr,
                DWMWINDOWATTRIBUTE(DWMWA_SYSTEMBACKDROP_TYPE_ID),
                (&raw const val).cast::<c_void>(),
                core::mem::size_of_val(&val) as u32,
            )
            .is_ok()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, material);
        false
    }
}

/// 探测沉浸式后景的系统取值，返回被系统真正接受的取值。
///
/// 做法是「写入后回读」：向真实窗口写入候选 `DWMWA_SYSTEMBACKDROP_TYPE`，再读回
/// 同一属性；只有读回值等于写入值才认为该取值被接受。
///
/// 但回读相等并不充分。本机实测（Windows 11 build 26100）：该属性其实只是一个整数
/// 存储，`DwmSetWindowAttribute` 对 0..=4 之外的任意值（含 `999`、`65536`）都返回
/// `S_OK`，`DwmGetWindowAttribute` 原样回读。因此探测先写入必然非法的哨兵值作负对照：
/// 一旦哨兵也被原样回读，就说明回读通道没有判别力，本函数返回 `None`，绝不猜测。
///
/// 返回 `Some(value)` 表示某候选值通过了「可判别」的写入回读校验；返回 `None` 表示
/// 无法确认（非 Windows、窗口句柄为 0、系统不改写属性、哨兵被回显）。
///
/// 本函数从不 panic，可在无头环境安全调用（`hwnd == 0` 直接返回 `None`）。探测会
/// 临时改写窗口材质并在结束时恢复原值。
#[must_use]
pub fn probe_immersive_backdrop(hwnd: isize) -> Option<i32> {
    #[cfg(windows)]
    {
        win32::probe_immersive_backdrop(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        None
    }
}

/// 运行时能否解析出未公开导出 `user32.dll!SetWindowCompositionAttribute`。
///
/// 这是亚克力自定义色调子档的专属能力探测：该导出不在导入库里，只能运行时
/// `GetProcAddress`；解析失败就说明这条通道在这台机器上不存在，界面必须禁用该
/// 子档。函数本身不做窗口级写入，因此可在创建窗口之前调用，且从不 panic。
///
/// 「冒烟探测」由宿主在真正下发时通过 [`set_window_accent_tint`] 的返回值完成：
/// 这里只保证导出存在，返回值才代表系统接受了这次写入。
#[must_use]
pub fn accent_policy_export_available() -> bool {
    #[cfg(windows)]
    {
        win32::accent_policy_export_available()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 通过未公开的 accent policy 通道设置亚克力自定义色调。
///
/// 该函数与 [`set_window_backdrop`] 是两条平行的写入路径，宿主必须按实际生效的
/// 材质二选一，绝不能同时下发。`argb` 是 ABGR 顺序的色调（alpha 分量参与半透明）。
///
/// 返回 `true` 表示调用成功且系统接受了本次写入；导出解析失败、窗口句柄为 0、
/// 或系统拒绝时返回 `false`。本函数从不 panic。
#[must_use]
pub fn set_window_accent_tint(hwnd: isize, argb: u32) -> bool {
    #[cfg(windows)]
    {
        win32::set_accent_tint(hwnd, argb)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, argb);
        false
    }
}

/// 关闭 accent policy 通道，把窗口交还给默认合成。
///
/// 从亚克力自定义色调切到其它材质时必须调用一次，避免上一档的亚克力继续叠加在
/// 新的系统材质之上。返回 `false` 表示没有可用的通道或系统拒绝；从不 panic。
#[must_use]
pub fn clear_window_accent_tint(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        win32::clear_accent_tint(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immersive_probe_reports_nothing_without_a_window() {
        // 无头 / 尚未创建窗口时探测必须安静地返回「无法确认」，不能 panic。
        assert_eq!(
            probe_immersive_backdrop(0),
            None,
            "a headless probe must never confirm a system value"
        );
    }

    #[test]
    fn accent_policy_resolution_is_headless_safe() {
        // 导出解析可能成功也可能失败，取决于运行环境；两种结果都合法，但绝不能 panic。
        let _resolved = accent_policy_export_available();
    }

    #[test]
    fn accent_policy_writes_reject_a_null_window() {
        assert!(
            !set_window_accent_tint(0, 0xCC1F_1F1F),
            "a null window must never receive an accent policy"
        );
        assert!(
            !clear_window_accent_tint(0),
            "a null window must never receive an accent policy clear"
        );
    }

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

    #[test]
    fn ui_rect_conversion_aligns_ui_scale_with_dpi_scale() {
        let dpi_scale = 1.5_f32;
        let ui_scale = 1.25_f32;
        // UI 逻辑矩形 (900, 0, 46, 38) → 物理像素 (1125..1182.5, 0..47.5)
        // → Win32 逻辑像素 (750..788.33, 0..31.67)。
        let frame_rect = FrameHitMetrics::ui_rect_to_frame_logical(
            Rect::new(900.0, 0.0, 46.0, 38.0),
            ui_scale,
            dpi_scale,
        );
        assert!((frame_rect.origin.x - 750.0).abs() < 0.01, "{frame_rect:?}");
        assert!((frame_rect.origin.y - 0.0).abs() < 0.01);
        assert!((frame_rect.size.x - 46.0 * ui_scale / dpi_scale).abs() < 0.01);
        assert!((frame_rect.size.y - 38.0 * ui_scale / dpi_scale).abs() < 0.01);

        // UI 逻辑 (1.0, 1.0) 缩放下必须与旧口径（rect / dpi_scale）完全一致。
        let identity = FrameHitMetrics::ui_rect_to_frame_logical(
            Rect::new(900.0, 0.0, 46.0, 38.0),
            1.0,
            dpi_scale,
        );
        assert!((identity.origin.x - 600.0).abs() < 0.01);
        assert!((identity.size.x - 46.0 / dpi_scale).abs() < 0.01);
    }

    #[test]
    fn frame_hit_metrics_hit_tests_high_dpi_with_ui_scaling() {
        let metrics = FrameHitMetrics::new();
        let dpi_scale = 1.5_f32;
        let ui_scale = 1.25_f32;
        metrics.set_scale_factor(dpi_scale);
        metrics.set_maximize_button_rect(FrameHitMetrics::ui_rect_to_frame_logical(
            Rect::new(900.0, 0.0, 46.0, 38.0),
            ui_scale,
            dpi_scale,
        ));

        // `WM_NCHITTEST` 收到的物理客户区坐标先除以 DPI 缩放，再与热区比较。
        // 物理像素 (920 * 1.25 = 1150, 15 * 1.25 = 18.75) → Win32 逻辑 (767, 13)。
        let inside_x = ((920.0 * ui_scale) / dpi_scale).round() as i32;
        let inside_y = ((15.0 * ui_scale) / dpi_scale).round() as i32;
        let hit = |x: i32, y: i32| {
            metrics.contains_logical(
                x,
                y,
                &metrics.max_btn_left,
                &metrics.max_btn_top,
                &metrics.max_btn_right,
                &metrics.max_btn_bottom,
            )
        };
        assert!(
            hit(inside_x, inside_y),
            "the button hot zone must hit precisely at ui_scale=1.25 and dpi=1.5"
        );

        // 旧实现漏乘 `ui_scale`，会把热区压到 (600..630)，同一点将落在区外；
        // 这里显式复算旧口径，确保该回归一旦复活能被本测试捕获。
        let legacy_right = ((900.0 + 46.0) / dpi_scale).round() as i32;
        assert!(
            inside_x > legacy_right,
            "the probe point must fall outside the legacy hot zone so the two conversions really differ"
        );
        // 左侧边界同样必须精确：热区左沿减一像素点在区外。
        assert!(!hit(749, inside_y));
        // 上边界为 0：负坐标必然在外。
        assert!(!hit(inside_x, -1));
    }
}

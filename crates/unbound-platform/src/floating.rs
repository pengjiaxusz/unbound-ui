//! 幽灵窗口与灵动岛浮窗策略、跨平台能力探测、多显示器工作区查询与纯逻辑防越界/贴边吸附。
//!
//! 本模块为 ADR 0039 提供平台无关的浮窗策略与纯 64 位安全定位算法：
//! * [`WindowRole`] 与 [`FloatingWindowPolicy`] 定义窗口角色及 Win32 扩展样式位映射；
//! * [`FloatingWindowCapabilities`] 描述当前后端对无激活、置顶、跳过任务栏、穿透与全局定位的支持度；
//! * [`all_monitors`] 与 [`work_area_for_point`] 查询真实显示器边界、工作区与 DPI 缩放；
//! * [`clamp_rect_to_work_area`]、[`place_cursor_monitor_avoid`]、[`place_anchor_rect_avoid`]、
//!   [`solve_edge_snap`] 与 [`restore_from_relative_anchor`] 实现无头可单测的防越界与跨 DPI 锚点恢复。

use crate::window::ScreenRect;

/// 窗口在平台层的语义角色。
/// 待同步到跨窗口目标 `NativeWindow` 的运行期热更新状态。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FloatingLiveSync {
    pub visible: Option<bool>,
    pub topmost: Option<bool>,
    pub click_through: Option<bool>,
    pub outer_rect: Option<ScreenRect>,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowRole {
    /// 标准应用窗口。
    #[default]
    Standard,
    /// 幽灵透明覆盖层（默认无激活、置顶、跳过任务栏、鼠标穿透、不可拖拽）。
    Ghost,
    /// 灵动岛浮窗宿主（默认无激活、置顶、跳过任务栏、接收点击、支持拖拽与贴边吸附）。
    FloatingIsland,
}

/// 浮窗行为策略。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FloatingWindowPolicy {
    /// 显示或点击时不抢占前台激活态与键盘焦点（Win32 `WS_EX_NOACTIVATE`）。
    pub no_activate: bool,
    /// 始终置于普通窗口之上（Win32 `WS_EX_TOPMOST`）。
    pub always_on_top: bool,
    /// 不出现在任务栏与 Alt+Tab 切换器中（Win32 `WS_EX_TOOLWINDOW`）。
    pub skip_taskbar: bool,
    /// 启用全窗口鼠标穿透（Win32 `WS_EX_TRANSPARENT`）。
    pub click_through: bool,
    /// 是否允许用户拖拽移动浮窗。
    pub draggable: bool,
}

impl FloatingWindowPolicy {
    /// 幽灵覆盖层默认策略：无激活、置顶、跳过任务栏、鼠标穿透、不可拖拽。
    #[must_use]
    pub const fn ghost_default() -> Self {
        Self {
            no_activate: true,
            always_on_top: true,
            skip_taskbar: true,
            click_through: true,
            draggable: false,
        }
    }

    /// 灵动岛浮窗默认策略：无激活、置顶、跳过任务栏、不穿透、可拖拽。
    #[must_use]
    pub const fn floating_island_default() -> Self {
        Self {
            no_activate: true,
            always_on_top: true,
            skip_taskbar: true,
            click_through: false,
            draggable: true,
        }
    }

    /// 根据窗口角色生成对应的默认浮窗策略。
    #[must_use]
    pub const fn for_role(role: WindowRole) -> Self {
        match role {
            WindowRole::Standard => Self {
                no_activate: false,
                always_on_top: false,
                skip_taskbar: false,
                click_through: false,
                draggable: false,
            },
            WindowRole::Ghost => Self::ghost_default(),
            WindowRole::FloatingIsland => Self::floating_island_default(),
        }
    }

    /// 将策略映射为 Win32 扩展窗口样式位（`WS_EX_*`）。
    #[must_use]
    pub const fn win32_ex_style_bits(&self) -> u32 {
        const WS_EX_TOPMOST: u32 = 0x0000_0008;
        const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
        const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
        const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

        let mut bits = 0u32;
        if self.no_activate {
            bits |= WS_EX_NOACTIVATE;
        }
        if self.always_on_top {
            bits |= WS_EX_TOPMOST;
        }
        if self.skip_taskbar {
            bits |= WS_EX_TOOLWINDOW;
        }
        if self.click_through {
            bits |= WS_EX_TRANSPARENT;
        }
        bits
    }
}

/// 当前平台后端对浮窗原语的能力描述。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FloatingWindowCapabilities {
    /// 是否支持显示/点击不抢占焦点。
    pub supports_no_activate: bool,
    /// 是否支持全局置顶。
    pub supports_topmost: bool,
    /// 是否支持从任务栏与切换器隐藏。
    pub supports_skip_taskbar: bool,
    /// 是否支持鼠标穿透切换。
    pub supports_click_through: bool,
    /// 是否支持按屏幕全局坐标定位。
    pub supports_global_positioning: bool,
}

impl FloatingWindowCapabilities {
    /// Win32 后端完整浮窗能力。
    #[must_use]
    pub const fn win32_full() -> Self {
        Self {
            supports_no_activate: true,
            supports_topmost: true,
            supports_skip_taskbar: true,
            supports_click_through: true,
            supports_global_positioning: true,
        }
    }

    /// Linux X11 后端默认浮窗能力。
    #[must_use]
    pub const fn x11_default() -> Self {
        Self {
            supports_no_activate: true,
            supports_topmost: true,
            supports_skip_taskbar: true,
            supports_click_through: true,
            supports_global_positioning: true,
        }
    }

    /// Linux Wayland `xdg_toplevel` 回退能力（支持输入穿透，不支持全局定位/置顶/跳过任务栏）。
    #[must_use]
    pub const fn wayland_xdg_fallback() -> Self {
        Self {
            supports_no_activate: false,
            supports_topmost: false,
            supports_skip_taskbar: false,
            supports_click_through: true,
            supports_global_positioning: false,
        }
    }

    /// 探测当前运行平台对应的浮窗能力集合。
    #[must_use]
    pub fn current_platform() -> Self {
        #[cfg(windows)]
        {
            Self::win32_full()
        }
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
        {
            match crate::linux_backend::active_linux_backend() {
                Some(crate::linux_backend::LinuxBackendKind::X11) => Self::x11_default(),
                Some(crate::linux_backend::LinuxBackendKind::Wayland) | None => {
                    Self::wayland_xdg_fallback()
                }
            }
        }
        #[cfg(not(any(
            windows,
            all(unix, not(target_os = "macos"), not(target_os = "android"))
        )))]
        {
            Self::default()
        }
    }
}

/// 单个显示器的物理边界、工作区（扣除任务栏/Dock）与 DPI 缩放信息。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MonitorWorkArea {
    /// 显示器完整物理像素矩形。
    pub monitor_bounds: ScreenRect,
    /// 扣除系统栏后的可用工作区物理像素矩形。
    pub work_area: ScreenRect,
    /// 显示器 DPI 缩放因子（`1.0` 对应 96 DPI）。
    pub scale_factor: f64,
    /// 是否为主显示器。
    pub is_primary: bool,
}

/// 屏幕贴边吸附的目标边缘。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SnapEdge {
    Left,
    Right,
    Top,
    Bottom,
}

/// 相对显示器工作区边缘的归一化锚点，用于跨 DPI 与多屏变化后恢复位置。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RelativeMonitorAnchor {
    /// 贴靠或最接近的工作区边缘。
    pub edge: SnapEdge,
    /// 沿边缘可移动范围内的归一化偏移（`0.0..=1.0`）。
    pub normalized_offset: f64,
}

/// 边缘吸附求解结果。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EdgeSnapResult {
    /// 裁剪并吸附后的最终屏幕矩形。
    pub rect: ScreenRect,
    /// 若触发了阈值内吸附，则为命中的边缘；否则为 `None`。
    pub snapped_edge: Option<SnapEdge>,
    /// 对应的归一化相对锚点。
    pub anchor: RelativeMonitorAnchor,
}

/// 浮窗初始或动态摆放策略。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FloatingPlacementStrategy {
    /// 跟随鼠标光标并在越出显示器工作区时自动反向避让。
    CursorMonitorAvoid {
        cursor: (i32, i32),
        size: (u32, u32),
        offset: (i32, i32),
    },
    /// 锚定到目标矩形上/下方并在越出工作区时自动翻转。
    AnchorRectAvoid {
        anchor: ScreenRect,
        size: (u32, u32),
        gap: i32,
        prefer_below: bool,
    },
    /// 在指定吸附阈值内向工作区四边自动吸附。
    EdgeSnap {
        desired: ScreenRect,
        snap_threshold: i32,
    },
}

/// 将期望矩形安全裁剪到目标显示器工作区之内。
///
/// 全程使用 64 位有符号整数运算，保证最小 1×1 尺寸、负坐标辅屏兼容以及超大矩形安全收缩。
#[must_use]
pub fn clamp_rect_to_work_area(desired: ScreenRect, work_area: ScreenRect) -> ScreenRect {
    let req_w = desired.width.max(1);
    let req_h = desired.height.max(1);
    if work_area.is_empty() {
        return ScreenRect::new(desired.x, desired.y, req_w, req_h);
    }

    let wa_left = i64::from(work_area.x);
    let wa_top = i64::from(work_area.y);
    let wa_w = i64::from(work_area.width);
    let wa_h = i64::from(work_area.height);
    let wa_right = wa_left + wa_w;
    let wa_bottom = wa_top + wa_h;

    let (x, width) = if i64::from(req_w) >= wa_w {
        (wa_left, work_area.width)
    } else {
        let max_x = wa_right - i64::from(req_w);
        (i64::from(desired.x).clamp(wa_left, max_x), req_w)
    };

    let (y, height) = if i64::from(req_h) >= wa_h {
        (wa_top, work_area.height)
    } else {
        let max_y = wa_bottom - i64::from(req_h);
        (i64::from(desired.y).clamp(wa_top, max_y), req_h)
    };

    ScreenRect::new(
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width,
        height,
    )
}

/// 依据鼠标位置与偏移放置浮窗；当右侧或底部越出工作区时自动翻转到光标另一侧。
#[must_use]
pub fn place_cursor_monitor_avoid(
    cursor: (i32, i32),
    size: (u32, u32),
    offset: (i32, i32),
    work_area: ScreenRect,
) -> ScreenRect {
    let w = size.0.max(1);
    let h = size.1.max(1);
    let mut x = i64::from(cursor.0) + i64::from(offset.0);
    let mut y = i64::from(cursor.1) + i64::from(offset.1);

    if !work_area.is_empty() {
        let wa_left = i64::from(work_area.x);
        let wa_top = i64::from(work_area.y);
        let wa_right = wa_left + i64::from(work_area.width);
        let wa_bottom = wa_top + i64::from(work_area.height);

        if x + i64::from(w) > wa_right {
            let flipped_x = i64::from(cursor.0) - i64::from(offset.0.abs()) - i64::from(w);
            if flipped_x >= wa_left {
                x = flipped_x;
            }
        }
        if y + i64::from(h) > wa_bottom {
            let flipped_y = i64::from(cursor.1) - i64::from(offset.1.abs()) - i64::from(h);
            if flipped_y >= wa_top {
                y = flipped_y;
            }
        }
    }

    let candidate = ScreenRect::new(
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        w,
        h,
    );
    clamp_rect_to_work_area(candidate, work_area)
}

/// 依据锚点矩形与间隙放置浮窗；优先按 `prefer_below` 放在下方或上方，越界时自动翻转并裁剪。
#[must_use]
pub fn place_anchor_rect_avoid(
    anchor: ScreenRect,
    size: (u32, u32),
    gap: i32,
    prefer_below: bool,
    work_area: ScreenRect,
) -> ScreenRect {
    let w = size.0.max(1);
    let h = size.1.max(1);
    let gap64 = i64::from(gap);

    let y_below = i64::from(anchor.y) + i64::from(anchor.height) + gap64;
    let y_above = i64::from(anchor.y) - gap64 - i64::from(h);

    let y = if work_area.is_empty() {
        if prefer_below {
            y_below
        } else {
            y_above
        }
    } else {
        let wa_top = i64::from(work_area.y);
        let wa_bottom = wa_top + i64::from(work_area.height);
        let below_fits = y_below >= wa_top && y_below + i64::from(h) <= wa_bottom;
        let above_fits = y_above >= wa_top && y_above + i64::from(h) <= wa_bottom;

        if prefer_below {
            if below_fits {
                y_below
            } else if above_fits {
                y_above
            } else {
                let space_below = wa_bottom - (i64::from(anchor.y) + i64::from(anchor.height));
                let space_above = i64::from(anchor.y) - wa_top;
                if space_above > space_below {
                    y_above
                } else {
                    y_below
                }
            }
        } else if above_fits {
            y_above
        } else if below_fits {
            y_below
        } else {
            let space_below = wa_bottom - (i64::from(anchor.y) + i64::from(anchor.height));
            let space_above = i64::from(anchor.y) - wa_top;
            if space_below > space_above {
                y_below
            } else {
                y_above
            }
        }
    };

    let candidate = ScreenRect::new(
        anchor.x,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        w,
        h,
    );
    clamp_rect_to_work_area(candidate, work_area)
}

/// 求解浮窗相对工作区四边的吸附结果，并生成跨 DPI 可恢复的相对锚点。
#[must_use]
pub fn solve_edge_snap(
    desired: ScreenRect,
    work_area: ScreenRect,
    snap_threshold: i32,
) -> EdgeSnapResult {
    let mut rect = clamp_rect_to_work_area(desired, work_area);
    if work_area.is_empty() {
        return EdgeSnapResult {
            rect,
            snapped_edge: None,
            anchor: RelativeMonitorAnchor {
                edge: SnapEdge::Left,
                normalized_offset: 0.0,
            },
        };
    }

    let wa_left = i64::from(work_area.x);
    let wa_top = i64::from(work_area.y);
    let wa_w = i64::from(work_area.width);
    let wa_h = i64::from(work_area.height);
    let wa_right = wa_left + wa_w;
    let wa_bottom = wa_top + wa_h;

    let rx = i64::from(rect.x);
    let ry = i64::from(rect.y);
    let rw = i64::from(rect.width);
    let rh = i64::from(rect.height);

    let d_left = (rx - wa_left).abs();
    let d_right = (wa_right - (rx + rw)).abs();
    let d_top = (ry - wa_top).abs();
    let d_bottom = (wa_bottom - (ry + rh)).abs();

    let candidates = [
        (SnapEdge::Left, d_left),
        (SnapEdge::Right, d_right),
        (SnapEdge::Top, d_top),
        (SnapEdge::Bottom, d_bottom),
    ];

    let nearest = candidates
        .iter()
        .copied()
        .min_by_key(|(_, dist)| *dist)
        .unwrap_or((SnapEdge::Left, 0));

    let thresh = i64::from(snap_threshold.max(0));
    let snapped_edge = if nearest.1 <= thresh {
        let edge = nearest.0;
        match edge {
            SnapEdge::Left => rect.x = work_area.x,
            SnapEdge::Right => {
                let snapped_x = (wa_right - rw).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
                rect.x = snapped_x as i32;
            }
            SnapEdge::Top => rect.y = work_area.y,
            SnapEdge::Bottom => {
                let snapped_y = (wa_bottom - rh).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
                rect.y = snapped_y as i32;
            }
        }
        Some(edge)
    } else {
        None
    };

    let anchor_edge = snapped_edge.unwrap_or(nearest.0);
    let normalized_offset = match anchor_edge {
        SnapEdge::Left | SnapEdge::Right => {
            let avail = wa_h - rh;
            if avail <= 0 {
                0.0
            } else {
                ((i64::from(rect.y) - wa_top) as f64 / avail as f64).clamp(0.0, 1.0)
            }
        }
        SnapEdge::Top | SnapEdge::Bottom => {
            let avail = wa_w - rw;
            if avail <= 0 {
                0.0
            } else {
                ((i64::from(rect.x) - wa_left) as f64 / avail as f64).clamp(0.0, 1.0)
            }
        }
    };

    EdgeSnapResult {
        rect,
        snapped_edge,
        anchor: RelativeMonitorAnchor {
            edge: anchor_edge,
            normalized_offset,
        },
    }
}

/// 根据归一化锚点与目标工作区恢复浮窗屏幕矩形（支持不同分辨率与混合 DPI 屏幕）。
#[must_use]
pub fn restore_from_relative_anchor(
    size: (u32, u32),
    anchor: RelativeMonitorAnchor,
    work_area: ScreenRect,
) -> ScreenRect {
    let w = size.0.max(1);
    let h = size.1.max(1);
    if work_area.is_empty() {
        return ScreenRect::new(0, 0, w, h);
    }

    let wa_left = i64::from(work_area.x);
    let wa_top = i64::from(work_area.y);
    let wa_w = i64::from(work_area.width);
    let wa_h = i64::from(work_area.height);
    let clamped_w = i64::from(w).min(wa_w);
    let clamped_h = i64::from(h).min(wa_h);
    let t = if anchor.normalized_offset.is_finite() {
        anchor.normalized_offset.clamp(0.0, 1.0)
    } else {
        0.0
    };

    let (x, y) = match anchor.edge {
        SnapEdge::Left => {
            let avail_h = (wa_h - clamped_h).max(0);
            let off_y = (avail_h as f64 * t).round() as i64;
            (wa_left, wa_top + off_y)
        }
        SnapEdge::Right => {
            let avail_h = (wa_h - clamped_h).max(0);
            let off_y = (avail_h as f64 * t).round() as i64;
            (wa_left + wa_w - clamped_w, wa_top + off_y)
        }
        SnapEdge::Top => {
            let avail_w = (wa_w - clamped_w).max(0);
            let off_x = (avail_w as f64 * t).round() as i64;
            (wa_left + off_x, wa_top)
        }
        SnapEdge::Bottom => {
            let avail_w = (wa_w - clamped_w).max(0);
            let off_x = (avail_w as f64 * t).round() as i64;
            (wa_left + off_x, wa_top + wa_h - clamped_h)
        }
    };

    let candidate = ScreenRect::new(
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        clamped_w as u32,
        clamped_h as u32,
    );
    clamp_rect_to_work_area(candidate, work_area)
}

/// 统一解析任意 [`FloatingPlacementStrategy`] 为工作区内的屏幕矩形。
#[must_use]
pub fn resolve_floating_placement(
    strategy: &FloatingPlacementStrategy,
    work_area: ScreenRect,
) -> ScreenRect {
    match *strategy {
        FloatingPlacementStrategy::CursorMonitorAvoid {
            cursor,
            size,
            offset,
        } => place_cursor_monitor_avoid(cursor, size, offset, work_area),
        FloatingPlacementStrategy::AnchorRectAvoid {
            anchor,
            size,
            gap,
            prefer_below,
        } => place_anchor_rect_avoid(anchor, size, gap, prefer_below, work_area),
        FloatingPlacementStrategy::EdgeSnap {
            desired,
            snap_threshold,
        } => solve_edge_snap(desired, work_area, snap_threshold).rect,
    }
}

/// 查询当前系统中所有可用显示器的物理边界、工作区与 DPI 缩放信息。
#[must_use]
pub fn all_monitors() -> Vec<MonitorWorkArea> {
    #[cfg(windows)]
    {
        win32_monitors::all_monitors()
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        linux_monitors::all_monitors()
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        Vec::new()
    }
}

/// 查询包含指定全局屏幕坐标点 `(x, y)` 的显示器工作区；无精确命中时返回最近或主显示器。
#[must_use]
pub fn work_area_for_point(x: i32, y: i32) -> Option<MonitorWorkArea> {
    #[cfg(windows)]
    {
        win32_monitors::work_area_for_point(x, y)
    }
    #[cfg(not(windows))]
    {
        let monitors = all_monitors();
        let px = i64::from(x);
        let py = i64::from(y);
        monitors
            .iter()
            .copied()
            .find(|m| {
                let left = i64::from(m.monitor_bounds.x);
                let top = i64::from(m.monitor_bounds.y);
                let right = left + i64::from(m.monitor_bounds.width);
                let bottom = top + i64::from(m.monitor_bounds.height);
                px >= left && px < right && py >= top && py < bottom
            })
            .or_else(|| monitors.iter().copied().find(|m| m.is_primary))
            .or_else(|| monitors.into_iter().next())
    }
}

#[cfg(windows)]
pub(crate) mod win32_monitors {
    use super::*;
    use ::windows::core::BOOL;
    use ::windows::Win32::Foundation::{LPARAM, POINT, RECT};
    use ::windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO,
        MONITOR_DEFAULTTONEAREST,
    };
    use ::windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    /// Win32 主显示器标志位（对应 `MONITORINFOF_PRIMARY`）。
    const MONITORINFOF_PRIMARY_FLAG: u32 = 1;

    fn rect_to_screen_rect(rc: RECT) -> ScreenRect {
        ScreenRect::new(
            rc.left,
            rc.top,
            (rc.right - rc.left).max(0) as u32,
            (rc.bottom - rc.top).max(0) as u32,
        )
    }

    pub(crate) fn query_hmonitor(hmonitor: HMONITOR) -> Option<MonitorWorkArea> {
        let mut mi = MONITORINFO {
            cbSize: core::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(hmonitor, &mut mi) }.as_bool() {
            return None;
        }
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let scale_factor =
            if unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
                .is_ok()
                && dpi_x > 0
            {
                f64::from(dpi_x) / 96.0
            } else {
                1.0
            };
        Some(MonitorWorkArea {
            monitor_bounds: rect_to_screen_rect(mi.rcMonitor),
            work_area: rect_to_screen_rect(mi.rcWork),
            scale_factor,
            is_primary: (mi.dwFlags & MONITORINFOF_PRIMARY_FLAG) != 0,
        })
    }

    unsafe extern "system" fn enum_monitor_proc(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _clip_rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        if let Some(area) = query_hmonitor(hmonitor) {
            // SAFETY: `lparam` 由 `all_monitors` 传入当前栈帧有效 `Vec<MonitorWorkArea>` 指针。
            let list = unsafe { &mut *(lparam.0 as *mut Vec<MonitorWorkArea>) };
            list.push(area);
        }
        BOOL(1)
    }

    pub(crate) fn all_monitors() -> Vec<MonitorWorkArea> {
        let mut list = Vec::new();
        let list_ptr = (&raw mut list) as isize;
        // SAFETY: `enum_monitor_proc` 仅在 `EnumDisplayMonitors` 同步枚举期间解引用 `list_ptr`。
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(enum_monitor_proc), LPARAM(list_ptr));
        }
        list
    }

    pub(crate) fn work_area_for_point(x: i32, y: i32) -> Option<MonitorWorkArea> {
        let hmonitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
        if hmonitor.is_invalid() {
            return None;
        }
        query_hmonitor(hmonitor)
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod linux_monitors {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    use x11rb::xcb_ffi::XCBConnection;

    fn query_x11_monitors() -> Option<Vec<MonitorWorkArea>> {
        let (conn, screen_num) = XCBConnection::connect(None).ok()?;
        let screen = conn.setup().roots.get(screen_num)?;
        let root = screen.root;

        let net_workarea_atom = conn
            .intern_atom(false, b"_NET_WORKAREA")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let global_work_area = conn
            .get_property(false, root, net_workarea_atom, AtomEnum::CARDINAL, 0, 4)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|reply| {
                let mut vals = reply.value32()?;
                let x = vals.next()? as i32;
                let y = vals.next()? as i32;
                let w = vals.next()?;
                let h = vals.next()?;
                (w > 0 && h > 0).then_some(ScreenRect::new(x, y, w, h))
            });

        if let Ok(cookie) = conn.randr_get_monitors(root, true) {
            if let Ok(reply) = cookie.reply() {
                if !reply.monitors.is_empty() {
                    let monitors = reply
                        .monitors
                        .iter()
                        .map(|mon| {
                            let bounds = ScreenRect::new(
                                i32::from(mon.x),
                                i32::from(mon.y),
                                u32::from(mon.width.max(1)),
                                u32::from(mon.height.max(1)),
                            );
                            let work_area = if mon.primary {
                                global_work_area.unwrap_or(bounds)
                            } else {
                                bounds
                            };
                            MonitorWorkArea {
                                monitor_bounds: bounds,
                                work_area,
                                scale_factor: 1.0,
                                is_primary: mon.primary,
                            }
                        })
                        .collect();
                    return Some(monitors);
                }
            }
        }

        let fallback = global_work_area.unwrap_or_else(|| {
            ScreenRect::new(
                0,
                0,
                u32::from(screen.width_in_pixels.max(1)),
                u32::from(screen.height_in_pixels.max(1)),
            )
        });
        Some(vec![MonitorWorkArea {
            monitor_bounds: fallback,
            work_area: fallback,
            scale_factor: 1.0,
            is_primary: true,
        }])
    }

    pub(crate) fn all_monitors() -> Vec<MonitorWorkArea> {
        let prefer_x11 = matches!(
            crate::linux_backend::active_linux_backend(),
            Some(crate::linux_backend::LinuxBackendKind::X11)
        ) || (std::env::var_os("WAYLAND_DISPLAY").is_none()
            && std::env::var_os("DISPLAY").is_some());

        if prefer_x11 {
            if let Some(monitors) = query_x11_monitors() {
                return monitors;
            }
        }

        if let Some(metrics) = crate::wayland_logic::primary_output_metrics() {
            let bounds = ScreenRect::new(
                0,
                0,
                metrics.physical_width.max(1),
                metrics.physical_height.max(1),
            );
            return vec![MonitorWorkArea {
                monitor_bounds: bounds,
                work_area: bounds,
                scale_factor: f64::from(metrics.scale_factor.max(1.0)),
                is_primary: true,
            }];
        }

        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_defaults_and_win32_ex_style_bits() {
        let std_policy = FloatingWindowPolicy::for_role(WindowRole::Standard);
        assert_eq!(std_policy.win32_ex_style_bits(), 0);
        assert!(!std_policy.draggable);

        let ghost = FloatingWindowPolicy::for_role(WindowRole::Ghost);
        assert_eq!(ghost, FloatingWindowPolicy::ghost_default());
        assert!(ghost.no_activate);
        assert!(ghost.always_on_top);
        assert!(ghost.skip_taskbar);
        assert!(ghost.click_through);
        assert!(!ghost.draggable);
        assert_eq!(
            ghost.win32_ex_style_bits(),
            0x0800_0000 | 0x0000_0008 | 0x0000_0080 | 0x0000_0020
        );

        let island = FloatingWindowPolicy::for_role(WindowRole::FloatingIsland);
        assert_eq!(island, FloatingWindowPolicy::floating_island_default());
        assert!(island.no_activate);
        assert!(island.always_on_top);
        assert!(island.skip_taskbar);
        assert!(!island.click_through);
        assert!(island.draggable);
        assert_eq!(
            island.win32_ex_style_bits(),
            0x0800_0000 | 0x0000_0008 | 0x0000_0080
        );
    }

    #[test]
    fn clamp_rect_protects_zero_size_and_oversized_windows() {
        let work = ScreenRect::new(0, 0, 1920, 1040);

        // 0×0 保护
        let zero = clamp_rect_to_work_area(ScreenRect::new(100, 100, 0, 0), work);
        assert_eq!(zero, ScreenRect::new(100, 100, 1, 1));

        // 超大窗口被压缩至工作区尺寸并对齐左上角
        let oversized = clamp_rect_to_work_area(ScreenRect::new(-50, -30, 3000, 2000), work);
        assert_eq!(oversized, work);
    }

    #[test]
    fn clamp_and_avoid_handle_negative_coordinate_monitors_and_four_side_taskbars() {
        // 负坐标辅屏 + 四周任务栏/Dock 留边（左 60、上 32、右 40、下 48）
        let neg_work = ScreenRect::new(-1920 + 60, 32, 1920 - 100, 1080 - 80);

        // 右下角光标避让应自动向左、向上翻转
        let placed = place_cursor_monitor_avoid((-100, 1020), (200, 120), (16, 16), neg_work);
        assert_eq!(placed.width, 200);
        assert_eq!(placed.height, 120);
        assert!(placed.x >= neg_work.x);
        assert!(placed.x + placed.width as i32 <= neg_work.x + neg_work.width as i32);
        assert!(placed.y >= neg_work.y);
        assert!(placed.y + placed.height as i32 <= neg_work.y + neg_work.height as i32);
        assert!(placed.x < -100);
        assert!(placed.y < 1020);
    }

    #[test]
    fn anchor_rect_avoid_flips_when_below_overflows() {
        let work = ScreenRect::new(0, 40, 1920, 1000);
        let anchor_near_bottom = ScreenRect::new(500, 960, 120, 32);

        let placed = place_anchor_rect_avoid(anchor_near_bottom, (220, 160), 8, true, work);
        assert_eq!(placed.x, 500);
        // 下方空间不足，翻转到锚点上方：960 - 8 - 160 = 792
        assert_eq!(placed.y, 792);

        let anchor_near_top = ScreenRect::new(500, 60, 120, 32);
        let placed_below = place_anchor_rect_avoid(anchor_near_top, (220, 160), 8, false, work);
        // 上方空间不足，翻转到锚点下方：60 + 32 + 8 = 100
        assert_eq!(placed_below.y, 100);
    }

    #[test]
    fn edge_snap_and_mixed_dpi_anchor_restoration() {
        // 1080p 100% 缩放屏幕（含底部 40px 任务栏）
        let work_1080p = ScreenRect::new(0, 0, 1920, 1040);
        let desired = ScreenRect::new(1690, 500, 220, 48);
        let snap = solve_edge_snap(desired, work_1080p, 24);
        assert_eq!(snap.snapped_edge, Some(SnapEdge::Right));
        assert_eq!(snap.rect.x, 1920 - 220);
        assert_eq!(snap.rect.y, 500);
        assert_eq!(snap.anchor.edge, SnapEdge::Right);

        // 切换到 4K 200% 缩放负坐标屏幕（左 0、顶 0、宽 3840、高 2080）并放大尺寸恢复
        let work_4k = ScreenRect::new(-3840, 0, 3840, 2080);
        let restored = restore_from_relative_anchor((440, 96), snap.anchor, work_4k);
        assert_eq!(restored.x, -440);
        assert_eq!(restored.width, 440);
        assert_eq!(restored.height, 96);
        // 垂直比例应大致保持在工作区中段
        assert!((restored.y - 1000).abs() <= 2);

        // 通过统一策略分发接口验证 EdgeSnap 与 AnchorRectAvoid
        let via_dispatch = resolve_floating_placement(
            &FloatingPlacementStrategy::EdgeSnap {
                desired: ScreenRect::new(10, 400, 200, 40),
                snap_threshold: 20,
            },
            work_1080p,
        );
        assert_eq!(via_dispatch.x, 0);
    }
}

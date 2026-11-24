//! Linux 显示协议后端探测、环境变量覆盖与 X11 越界浮层工作区策略（纯逻辑，跨平台可单测）。

use std::sync::{Mutex, OnceLock};

use crate::popup_host::clamp_popup_to_work_area;
use crate::window::{PlatformError, ScreenRect};

/// `UNBOUND_LINUX_BACKEND` 环境变量解析出的后端偏好。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LinuxBackendPreference {
    /// 自动探测：优先 Wayland，连接失败或未配置时回退 X11。
    #[default]
    Auto,
    /// 强制使用 Wayland 后端。
    Wayland,
    /// 强制使用 X11 后端。
    X11,
}

/// 具体的 Linux 显示协议后端类型。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinuxBackendKind {
    /// 原生 Wayland 后端。
    Wayland,
    /// 原生 X11（XCB / `x11rb`）后端。
    X11,
}

impl LinuxBackendKind {
    /// 人类可读的后端标识。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "wayland",
            Self::X11 => "x11",
        }
    }
}

/// 进程环境变量快照（用于无头单测，无需篡改全局进程环境）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinuxDisplayEnv {
    /// `WAYLAND_DISPLAY` 环境变量值。
    pub wayland_display: Option<String>,
    /// `WAYLAND_SOCKET` 环境变量值。
    pub wayland_socket: Option<String>,
    /// `DISPLAY` 环境变量值。
    pub x11_display: Option<String>,
}

impl LinuxDisplayEnv {
    /// 从当前进程环境变量采集显示会话快照。
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            wayland_display: std::env::var("WAYLAND_DISPLAY").ok(),
            wayland_socket: std::env::var("WAYLAND_SOCKET").ok(),
            x11_display: std::env::var("DISPLAY").ok(),
        }
    }

    fn has_wayland(&self) -> bool {
        self.wayland_display
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
            || self
                .wayland_socket
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
    }

    fn has_x11(&self) -> bool {
        self.x11_display
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
    }
}

/// 解析 `UNBOUND_LINUX_BACKEND` 环境变量。
///
/// 未设置、空串或 `auto`（大小写不敏感）返回 [`LinuxBackendPreference::Auto`]；
/// `wayland` / `x11` 对应强制选择；其余非法值返回包含合法取值说明的 [`PlatformError::EventLoop`]。
///
/// # Errors
/// 当传入值不在 `auto`、`wayland`、`x11` 范围内时返回 [`PlatformError::EventLoop`]。
pub fn parse_linux_backend_env(raw: Option<&str>) -> Result<LinuxBackendPreference, PlatformError> {
    let Some(value) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(LinuxBackendPreference::Auto);
    };
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(LinuxBackendPreference::Auto),
        "wayland" => Ok(LinuxBackendPreference::Wayland),
        "x11" => Ok(LinuxBackendPreference::X11),
        other => Err(PlatformError::EventLoop(format!(
            "invalid UNBOUND_LINUX_BACKEND='{other}'; expected one of: auto, wayland, x11"
        ))),
    }
}

/// 根据后端偏好与显示会话环境变量推导按优先级排序的候选后端列表。
///
/// # Errors
/// 当处于 `Auto` 模式且 Wayland 与 X11 显示环境变量均为空时返回结构化错误。
pub fn select_linux_backend_order(
    pref: LinuxBackendPreference,
    env: &LinuxDisplayEnv,
) -> Result<Vec<LinuxBackendKind>, PlatformError> {
    match pref {
        LinuxBackendPreference::Wayland => Ok(vec![LinuxBackendKind::Wayland]),
        LinuxBackendPreference::X11 => Ok(vec![LinuxBackendKind::X11]),
        LinuxBackendPreference::Auto => {
            let mut order = Vec::with_capacity(2);
            if env.has_wayland() {
                order.push(LinuxBackendKind::Wayland);
            }
            if env.has_x11() {
                order.push(LinuxBackendKind::X11);
            }
            if order.is_empty() {
                return Err(PlatformError::EventLoop(
                    "no Linux display server detected (neither WAYLAND_DISPLAY/WAYLAND_SOCKET nor DISPLAY is set)".to_string(),
                ));
            }
            Ok(order)
        }
    }
}

static ACTIVE_LINUX_BACKEND: OnceLock<Mutex<Option<LinuxBackendKind>>> = OnceLock::new();

/// 记录当前进程正在运行的 Linux 显示后端（退出事件循环时清空）。
pub fn set_active_linux_backend(kind: Option<LinuxBackendKind>) {
    let lock = ACTIVE_LINUX_BACKEND.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = kind;
    }
}

/// 查询当前进程正在运行的 Linux 显示后端。
#[must_use]
pub fn active_linux_backend() -> Option<LinuxBackendKind> {
    let lock = ACTIVE_LINUX_BACKEND.get_or_init(|| Mutex::new(None));
    lock.lock().ok().and_then(|guard| *guard)
}

/// 结合锚点矩形与显示器工作区（兼容多屏负坐标），为 X11 越界弹出窗执行贴边翻转与工作区夹紧。
///
/// 策略与 Wayland `xdg_positioner` 及 Win32 浮层一致：
/// 1. 若弹出窗底边越出工作区且锚点上方可用空间更大，则沿垂直方向翻转至锚点上方；
/// 2. 若弹出窗右边越出工作区且锚点左侧可用空间更大，则沿水平方向翻转至锚点左侧；
/// 3. 最后调用 [`clamp_popup_to_work_area`] 保证弹出窗宽高不超过工作区且完整落在可见区域内。
#[must_use]
pub fn place_x11_popup_in_work_area(
    anchor_rect: Option<ScreenRect>,
    desired: ScreenRect,
    work_area: ScreenRect,
) -> ScreenRect {
    let mut rect = desired;
    if let Some(anchor) = anchor_rect {
        let work_left = i64::from(work_area.x);
        let work_top = i64::from(work_area.y);
        let work_right = work_left + i64::from(work_area.width);
        let work_bottom = work_top + i64::from(work_area.height);

        let desired_bottom = i64::from(rect.y) + i64::from(rect.height);
        if desired_bottom > work_bottom {
            let space_above = i64::from(anchor.y) - work_top;
            let space_below = work_bottom - (i64::from(anchor.y) + i64::from(anchor.height));
            if space_above > space_below {
                let flipped_y = i64::from(anchor.y) - i64::from(rect.height);
                rect.y = flipped_y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            }
        }

        let desired_right = i64::from(rect.x) + i64::from(rect.width);
        if desired_right > work_right {
            let space_left = i64::from(anchor.x) - work_left;
            let space_right = work_right - (i64::from(anchor.x) + i64::from(anchor.width));
            if space_left > space_right {
                let flipped_x = i64::from(anchor.x) - i64::from(rect.width);
                rect.x = flipped_x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            }
        }
    }
    clamp_popup_to_work_area(rect, work_area)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_linux_backend_env_accepts_valid_values_and_rejects_invalid() {
        assert_eq!(
            parse_linux_backend_env(None).expect("default"),
            LinuxBackendPreference::Auto
        );
        assert_eq!(
            parse_linux_backend_env(Some("  ")).expect("empty"),
            LinuxBackendPreference::Auto
        );
        assert_eq!(
            parse_linux_backend_env(Some("AUTO")).expect("auto"),
            LinuxBackendPreference::Auto
        );
        assert_eq!(
            parse_linux_backend_env(Some("Wayland")).expect("wayland"),
            LinuxBackendPreference::Wayland
        );
        assert_eq!(
            parse_linux_backend_env(Some("x11")).expect("x11"),
            LinuxBackendPreference::X11
        );
        let err = parse_linux_backend_env(Some("mir")).expect_err("invalid backend must error");
        assert!(err.to_string().contains("UNBOUND_LINUX_BACKEND"));
    }

    #[test]
    fn select_linux_backend_order_prioritizes_wayland_then_x11_in_auto_mode() {
        let both = LinuxDisplayEnv {
            wayland_display: Some("wayland-0".into()),
            wayland_socket: None,
            x11_display: Some(":0".into()),
        };
        assert_eq!(
            select_linux_backend_order(LinuxBackendPreference::Auto, &both).unwrap(),
            vec![LinuxBackendKind::Wayland, LinuxBackendKind::X11]
        );

        let x11_only = LinuxDisplayEnv {
            wayland_display: None,
            wayland_socket: None,
            x11_display: Some(":1".into()),
        };
        assert_eq!(
            select_linux_backend_order(LinuxBackendPreference::Auto, &x11_only).unwrap(),
            vec![LinuxBackendKind::X11]
        );

        let wayland_socket_only = LinuxDisplayEnv {
            wayland_display: None,
            wayland_socket: Some("3".into()),
            x11_display: None,
        };
        assert_eq!(
            select_linux_backend_order(LinuxBackendPreference::Auto, &wayland_socket_only).unwrap(),
            vec![LinuxBackendKind::Wayland]
        );

        // 显式覆盖不受环境变量约束
        let empty = LinuxDisplayEnv::default();
        assert_eq!(
            select_linux_backend_order(LinuxBackendPreference::X11, &empty).unwrap(),
            vec![LinuxBackendKind::X11]
        );
        assert!(select_linux_backend_order(LinuxBackendPreference::Auto, &empty).is_err());
    }

    #[test]
    fn x11_popup_placement_flips_and_clamps_on_negative_coordinate_secondary_monitor() {
        // 左侧辅屏工作区：[-1920, 0) × [0, 1040)
        let work_area = ScreenRect::new(-1920, 0, 1920, 1040);
        // 锚点靠近辅屏底边与右边
        let anchor = ScreenRect::new(-160, 960, 120, 32);
        let desired = ScreenRect::new(-160, 992, 240, 200);
        let placed = place_x11_popup_in_work_area(Some(anchor), desired, work_area);

        // 应当向上翻转至 960 - 200 = 760，且完整留在 [-1920, 0) × [0, 1040) 内
        assert_eq!(placed.y, 760);
        assert!(placed.x >= work_area.x);
        assert!(placed.x + placed.width as i32 <= work_area.x + work_area.width as i32);
        assert!(placed.y >= work_area.y);
        assert!(placed.y + placed.height as i32 <= work_area.y + work_area.height as i32);
    }
}

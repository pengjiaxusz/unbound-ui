//! Windows 任务栏进度条与系统通知托盘集成。
//!
//! 基于官方 `windows = "0.62.2"` crate 提供以下功能：
//! - 任务栏按钮进度指示器（`ITaskbarList3`：`TBPF_NORMAL`、`TBPF_ERROR`、`TBPF_PAUSED`、`TBPF_INDETERMINATE`）。
//! - 系统通知托盘图标（`Shell_NotifyIconW`），支持气泡通知与悬停提示。

/// 任务栏进度指示器的状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TaskbarProgressState {
    /// 不显示进度。
    NoProgress,
    /// 不确定跑马灯进度（旋转/滚动条）。
    Indeterminate,
    /// 正常绿色进度条。
    Normal,
    /// 错误红色进度条。
    Error,
    /// 暂停黄色进度条。
    Paused,
}

/// 系统托盘气泡通知的严重级别。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrayNotificationSeverity {
    Info,
    Warning,
    Error,
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use core::ffi::c_void;

    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use ::windows::Win32::UI::Shell::{
        ITaskbarList3, Shell_NotifyIconW, TaskbarList, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR,
        NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, TBPF_ERROR,
        TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
    };

    #[inline]
    fn hwnd_from_isize(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    pub fn update_taskbar_progress(
        hwnd: isize,
        completed: u64,
        total: u64,
        state: TaskbarProgressState,
    ) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let Ok(taskbar): Result<ITaskbarList3, _> =
                CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            else {
                return;
            };
            let _ = taskbar.HrInit();

            let hwnd_ptr = hwnd_from_isize(hwnd);
            let flag = match state {
                TaskbarProgressState::NoProgress => TBPF_NOPROGRESS,
                TaskbarProgressState::Indeterminate => TBPF_INDETERMINATE,
                TaskbarProgressState::Normal => TBPF_NORMAL,
                TaskbarProgressState::Error => TBPF_ERROR,
                TaskbarProgressState::Paused => TBPF_PAUSED,
            };

            let _ = taskbar.SetProgressState(hwnd_ptr, flag);
            if state != TaskbarProgressState::NoProgress
                && state != TaskbarProgressState::Indeterminate
            {
                let _ = taskbar.SetProgressValue(hwnd_ptr, completed, total.max(1));
            }
        }
    }

    pub fn set_tray_icon(hwnd: isize, tip: &str, visible: bool) -> bool {
        let mut data = NOTIFYICONDATAW {
            cbSize: core::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd_from_isize(hwnd),
            uID: 1001,
            ..Default::default()
        };

        if !visible {
            return unsafe { Shell_NotifyIconW(NIM_DELETE, &data).as_bool() };
        }

        data.uFlags = NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = 0x8000 + 1; // WM_APP + 1

        for (idx, ch) in tip.encode_utf16().enumerate().take(127) {
            data.szTip[idx] = ch;
        }

        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
        if !added {
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
        } else {
            true
        }
    }

    pub fn show_tray_balloon(
        hwnd: isize,
        title: &str,
        message: &str,
        severity: TrayNotificationSeverity,
    ) -> bool {
        let mut data = NOTIFYICONDATAW {
            cbSize: core::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd_from_isize(hwnd),
            uID: 1001,
            uFlags: NIF_INFO,
            dwInfoFlags: match severity {
                TrayNotificationSeverity::Info => NIIF_INFO,
                TrayNotificationSeverity::Warning => NIIF_WARNING,
                TrayNotificationSeverity::Error => NIIF_ERROR,
            },
            ..Default::default()
        };

        for (idx, ch) in title.encode_utf16().enumerate().take(63) {
            data.szInfoTitle[idx] = ch;
        }
        for (idx, ch) in message.encode_utf16().enumerate().take(255) {
            data.szInfo[idx] = ch;
        }

        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
    }
}

/// 为指定窗口设置任务栏进度指示器状态与进度值。
pub fn set_taskbar_progress(hwnd: isize, completed: u64, total: u64, state: TaskbarProgressState) {
    #[cfg(windows)]
    {
        win32::update_taskbar_progress(hwnd, completed, total, state);
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, completed, total, state);
    }
}

/// 将 [`TrayNotificationSeverity`] 映射为 `org.freedesktop.Notifications` 紧急度字节（0=Low, 1=Normal, 2=Critical）。
#[must_use]
pub const fn notification_urgency(severity: TrayNotificationSeverity) -> u8 {
    match severity {
        TrayNotificationSeverity::Info => 0,
        TrayNotificationSeverity::Warning => 1,
        TrayNotificationSeverity::Error => 2,
    }
}

/// 跨平台系统托盘可见性与提示文案状态机（纯逻辑可单测）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayIconState {
    visible: bool,
    tooltip: String,
}

impl TrayIconState {
    /// 更新托盘可见性与悬停提示文本。
    pub fn update(&mut self, tooltip: &str, visible: bool) {
        self.visible = visible;
        if visible {
            self.tooltip = tooltip.to_owned();
        } else {
            self.tooltip.clear();
        }
    }

    /// 当前托盘图标是否处于显示态。
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// 当前悬停提示文案。
    #[must_use]
    pub fn tooltip(&self) -> &str {
        &self.tooltip
    }
}

/// 为指定窗口显示或隐藏系统托盘图标。
pub fn set_tray_icon(hwnd: isize, tooltip: &str, visible: bool) -> bool {
    #[cfg(windows)]
    {
        win32::set_tray_icon(hwnd, tooltip, visible)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        let _ = hwnd;
        crate::linux_dbus::linux_set_tray_icon(tooltip, visible)
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        let _ = (hwnd, tooltip, visible);
        false
    }
}

/// 显示系统托盘气泡通知。
pub fn show_tray_balloon(
    hwnd: isize,
    title: &str,
    message: &str,
    severity: TrayNotificationSeverity,
) -> bool {
    #[cfg(windows)]
    {
        win32::show_tray_balloon(hwnd, title, message, severity)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        let _ = hwnd;
        crate::linux_dbus::linux_show_notification(title, message, notification_urgency(severity))
    }
    #[cfg(not(any(
        windows,
        all(unix, not(target_os = "macos"), not(target_os = "android"))
    )))]
    {
        let _ = (hwnd, title, message, severity);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_urgency_maps_all_severities() {
        assert_eq!(notification_urgency(TrayNotificationSeverity::Info), 0);
        assert_eq!(notification_urgency(TrayNotificationSeverity::Warning), 1);
        assert_eq!(notification_urgency(TrayNotificationSeverity::Error), 2);
    }

    #[test]
    fn tray_icon_state_tracks_visibility_and_clears_tooltip_on_hide() {
        let mut state = TrayIconState::default();
        assert!(!state.is_visible());
        state.update("Unbound UI", true);
        assert!(state.is_visible());
        assert_eq!(state.tooltip(), "Unbound UI");
        state.update("ignored", false);
        assert!(!state.is_visible());
        assert!(state.tooltip().is_empty());
    }

    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    #[test]
    fn linux_tray_and_balloon_degrade_safely_without_bus() {
        // 无 D-Bus 会话或无托盘宿主时应安全返回布尔值而不 panic
        let _ = set_tray_icon(0, "Unbound UI", true);
        let _ = show_tray_balloon(0, "Notice", "Body", TrayNotificationSeverity::Info);
        let _ = set_tray_icon(0, "", false);
    }
}

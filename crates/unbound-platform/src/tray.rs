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

/// 为指定窗口显示或隐藏系统托盘图标。
pub fn set_tray_icon(hwnd: isize, tooltip: &str, visible: bool) -> bool {
    #[cfg(windows)]
    {
        win32::set_tray_icon(hwnd, tooltip, visible)
    }
    #[cfg(not(windows))]
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
    #[cfg(not(windows))]
    {
        let _ = (hwnd, title, message, severity);
        false
    }
}

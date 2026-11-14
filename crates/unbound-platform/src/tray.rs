//! Windows 任务栏进度条与系统通知托盘集成。
//!
//! 提供以下功能：
//! - 任务栏按钮进度指示器（`TBPF_NORMAL`、`TBPF_ERROR`、`TBPF_PAUSED`、`TBPF_INDETERMINATE`）。
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
    type HICON = *mut c_void;
    type DWORD = u32;
    type UINT = u32;
    type BOOL = i32;
    type ULONGLONG = u64;

    const NIM_ADD: DWORD = 0x00000000;
    const NIM_MODIFY: DWORD = 0x00000001;
    const NIM_DELETE: DWORD = 0x00000002;

    const NIF_MESSAGE: UINT = 0x00000001;
    const NIF_ICON: UINT = 0x00000002;
    const NIF_TIP: UINT = 0x00000004;
    const NIF_INFO: UINT = 0x00000010;

    const NIIF_INFO: DWORD = 0x00000001;
    const NIIF_WARNING: DWORD = 0x00000002;
    const NIIF_ERROR: DWORD = 0x00000003;

    const TBPF_NOPROGRESS: DWORD = 0;
    const TBPF_INDETERMINATE: DWORD = 1;
    const TBPF_NORMAL: DWORD = 2;
    const TBPF_ERROR: DWORD = 4;
    const TBPF_PAUSED: DWORD = 8;

    #[repr(C)]
    struct GUID {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const CLSID_TASKBAR_LIST: GUID = GUID {
        data1: 0x56FDF344,
        data2: 0xFD6D,
        data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA3, 0x47],
    };

    const IID_ITASKBAR_LIST3: GUID = GUID {
        data1: 0xEA1AFB91,
        data2: 0x9E28,
        data3: 0x4B86,
        data4: [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF],
    };

    #[repr(C)]
    struct NOTIFYICONDATAW {
        cbSize: DWORD,
        hWnd: HWND,
        uID: UINT,
        uFlags: UINT,
        uCallbackMessage: UINT,
        hIcon: HICON,
        szTip: [u16; 128],
        dwState: DWORD,
        dwStateMask: DWORD,
        szInfo: [u16; 256],
        uTimeoutOrVersion: UINT,
        szInfoTitle: [u16; 64],
        dwInfoFlags: DWORD,
        guidItem: GUID,
        hBalloonIcon: HICON,
    }

    #[repr(C)]
    struct ITaskbarList3Vtbl {
        // IUnknown
        query_interface: unsafe extern "system" fn(
            this: *mut c_void,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> i32,
        add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
        release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        // ITaskbarList
        hr_init: unsafe extern "system" fn(this: *mut c_void) -> i32,
        add_tab: unsafe extern "system" fn(this: *mut c_void, hwnd: HWND) -> i32,
        delete_tab: unsafe extern "system" fn(this: *mut c_void, hwnd: HWND) -> i32,
        activate_tab: unsafe extern "system" fn(this: *mut c_void, hwnd: HWND) -> i32,
        set_active_alt: unsafe extern "system" fn(this: *mut c_void, hwnd: HWND) -> i32,
        // ITaskbarList2
        mark_fullscreen_window:
            unsafe extern "system" fn(this: *mut c_void, hwnd: HWND, fFullscreen: BOOL) -> i32,
        // ITaskbarList3
        set_progress_value: unsafe extern "system" fn(
            this: *mut c_void,
            hwnd: HWND,
            ullCompleted: ULONGLONG,
            ullTotal: ULONGLONG,
        ) -> i32,
        set_progress_state:
            unsafe extern "system" fn(this: *mut c_void, hwnd: HWND, tbpFlags: DWORD) -> i32,
        register_tab:
            unsafe extern "system" fn(this: *mut c_void, hwndTab: HWND, hwndMDI: HWND) -> i32,
        unregister_tab: unsafe extern "system" fn(this: *mut c_void, hwndTab: HWND) -> i32,
        set_tab_order: unsafe extern "system" fn(
            this: *mut c_void,
            hwndTab: HWND,
            hwndInsertBefore: HWND,
        ) -> i32,
        set_tab_active: unsafe extern "system" fn(
            this: *mut c_void,
            hwndTab: HWND,
            hwndMDI: HWND,
            dwReserved: DWORD,
        ) -> i32,
        thumb_bar_add_buttons: unsafe extern "system" fn(
            this: *mut c_void,
            hwnd: HWND,
            cButtons: UINT,
            pButton: *const c_void,
        ) -> i32,
        thumb_bar_update_buttons: unsafe extern "system" fn(
            this: *mut c_void,
            hwnd: HWND,
            cButtons: UINT,
            pButton: *const c_void,
        ) -> i32,
        thumb_bar_set_image_list:
            unsafe extern "system" fn(this: *mut c_void, hwnd: HWND, himl: *mut c_void) -> i32,
        set_overlay_icon: unsafe extern "system" fn(
            this: *mut c_void,
            hwnd: HWND,
            hIcon: HICON,
            pszDescription: *const u16,
        ) -> i32,
        set_thumbnail_tooltip:
            unsafe extern "system" fn(this: *mut c_void, hwnd: HWND, pszTip: *const u16) -> i32,
        set_thumbnail_clip:
            unsafe extern "system" fn(this: *mut c_void, hwnd: HWND, prcClip: *const c_void) -> i32,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitialize(pvReserved: *mut c_void) -> i32;
        fn CoCreateInstance(
            rclsid: *const GUID,
            pUnkOuter: *mut c_void,
            dwClsContext: DWORD,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(dwMessage: DWORD, lpData: *mut NOTIFYICONDATAW) -> BOOL;
    }

    pub fn update_taskbar_progress(
        hwnd: isize,
        completed: u64,
        total: u64,
        state: TaskbarProgressState,
    ) {
        unsafe {
            let _ = CoInitialize(core::ptr::null_mut());
            let mut tbl_ptr: *mut c_void = core::ptr::null_mut();
            const CLSCTX_INPROC_SERVER: DWORD = 1;
            let hr = CoCreateInstance(
                &CLSID_TASKBAR_LIST,
                core::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ITASKBAR_LIST3,
                &mut tbl_ptr,
            );
            if hr < 0 || tbl_ptr.is_null() {
                return;
            }

            let vtbl = *(tbl_ptr as *mut *mut ITaskbarList3Vtbl);
            let _ = ((&*vtbl).hr_init)(tbl_ptr);

            let hwnd_ptr = hwnd as HWND;
            let flag = match state {
                TaskbarProgressState::NoProgress => TBPF_NOPROGRESS,
                TaskbarProgressState::Indeterminate => TBPF_INDETERMINATE,
                TaskbarProgressState::Normal => TBPF_NORMAL,
                TaskbarProgressState::Error => TBPF_ERROR,
                TaskbarProgressState::Paused => TBPF_PAUSED,
            };

            let _ = ((&*vtbl).set_progress_state)(tbl_ptr, hwnd_ptr, flag);
            if state != TaskbarProgressState::NoProgress
                && state != TaskbarProgressState::Indeterminate
            {
                let _ = ((&*vtbl).set_progress_value)(tbl_ptr, hwnd_ptr, completed, total.max(1));
            }

            let _ = ((&*vtbl).release)(tbl_ptr);
        }
    }

    pub fn set_tray_icon(hwnd: isize, tip: &str, visible: bool) -> bool {
        let mut data = unsafe { core::mem::zeroed::<NOTIFYICONDATAW>() };
        data.cbSize = core::mem::size_of::<NOTIFYICONDATAW>() as DWORD;
        data.hWnd = hwnd as HWND;
        data.uID = 1001;

        if !visible {
            return unsafe { Shell_NotifyIconW(NIM_DELETE, &mut data) != 0 };
        }

        data.uFlags = NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = 0x8000 + 1; // WM_APP + 1

        for (idx, ch) in tip.encode_utf16().enumerate().take(127) {
            data.szTip[idx] = ch;
        }

        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &mut data) != 0 };
        if !added {
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut data) != 0 }
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
        let mut data = unsafe { core::mem::zeroed::<NOTIFYICONDATAW>() };
        data.cbSize = core::mem::size_of::<NOTIFYICONDATAW>() as DWORD;
        data.hWnd = hwnd as HWND;
        data.uID = 1001;
        data.uFlags = NIF_INFO;

        data.dwInfoFlags = match severity {
            TrayNotificationSeverity::Info => NIIF_INFO,
            TrayNotificationSeverity::Warning => NIIF_WARNING,
            TrayNotificationSeverity::Error => NIIF_ERROR,
        };

        for (idx, ch) in title.encode_utf16().enumerate().take(63) {
            data.szInfoTitle[idx] = ch;
        }
        for (idx, ch) in message.encode_utf16().enumerate().take(255) {
            data.szInfo[idx] = ch;
        }

        unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut data) != 0 }
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

//! Windows 任务栏进度条与系统通知托盘集成。
//!
//! 基于官方 `windows = "0.62.2"` crate 提供以下功能：
//! - 任务栏按钮进度指示器（`ITaskbarList3`：`TBPF_NORMAL`、`TBPF_ERROR`、`TBPF_PAUSED`、`TBPF_INDETERMINATE`）。
//! - 系统通知托盘图标（`Shell_NotifyIconW`），支持应用图标加载、气泡通知、悬停提示、
//!   右键弹出菜单、急救菜单心跳切换、Explorer 重启后 `TaskbarCreated` 自动重挂以及点击事件队列。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// 托盘图标回调窗口消息号（避开 `win32_window::WM_APP_WAKE = 0x8001`）。
pub const WM_APP_TRAY: u32 = 0x8002;

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

/// 托盘右键弹出菜单条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuItem {
    /// 菜单命令 ID（选中后随 [`TrayEvent::MenuCommand`] 回传）。
    pub id: u32,
    /// 菜单展示文案。
    pub label: String,
    /// 是否可点击启用。
    pub enabled: bool,
    /// 是否为分隔线条目。
    pub separator: bool,
}

/// 托盘交互与生命周期事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// 左键单击托盘图标。
    LeftClick,
    /// 左键双击托盘图标。
    DoubleClick,
    /// 右键请求上下文菜单。
    ContextMenu,
    /// 用户选中了指定 ID 的托盘菜单项。
    MenuCommand(u32),
    /// Explorer 重启或显式重挂后托盘图标已重新附加。
    Reattached,
}

/// 进程级线程安全托盘状态（缓存图标参数、常规/急救菜单、心跳与事件队列）。
#[derive(Default)]
struct TrayRuntimeState {
    last_icon: Option<(isize, String, bool)>,
    menu_items: Vec<TrayMenuItem>,
    rescue_menu_items: Vec<TrayMenuItem>,
    last_heartbeat: Option<Instant>,
    heartbeat_stalled_override: Option<bool>,
    events: VecDeque<TrayEvent>,
}

fn tray_state() -> &'static Mutex<TrayRuntimeState> {
    static STATE: OnceLock<Mutex<TrayRuntimeState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(TrayRuntimeState::default()))
}

/// 向托盘事件队列推入一条事件。
pub(crate) fn push_tray_event(ev: TrayEvent) {
    let mut state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.events.push_back(ev);
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use core::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    use ::windows::core::{w, PCWSTR};
    use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use ::windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use ::windows::Win32::UI::Shell::{
        ExtractIconExW, ITaskbarList3, Shell_NotifyIconW, TaskbarList, NIF_ICON, NIF_INFO,
        NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, LoadIconW,
        PostMessageW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, HICON,
        IDI_APPLICATION, MF_GRAYED, MF_SEPARATOR, MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
    };

    #[inline]
    fn hwnd_from_isize(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    /// 注册并缓存 `TaskbarCreated` 广播消息号，用于 Explorer 重启后自动重挂托盘图标。
    pub(crate) fn taskbar_created_message() -> u32 {
        static MSG: OnceLock<u32> = OnceLock::new();
        *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
    }

    /// 优先从当前可执行文件提取小图标，失败时回退系统默认应用图标。
    ///
    /// 返回 `(icon, owned)`；仅当 `owned == true` 时调用方须在提交后调用 `DestroyIcon` 释放。
    fn load_default_tray_icon() -> (HICON, bool) {
        if let Ok(exe_path) = std::env::current_exe() {
            let wide: Vec<u16> = exe_path
                .as_os_str()
                .encode_wide()
                .chain(core::iter::once(0))
                .collect();
            let mut small_icon = HICON::default();
            let extracted =
                unsafe { ExtractIconExW(PCWSTR(wide.as_ptr()), 0, None, Some(&mut small_icon), 1) };
            if extracted > 0 && !small_icon.is_invalid() {
                return (small_icon, true);
            }
        }
        let fallback = unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default();
        (fallback, false)
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

        let (icon, owned_icon) = load_default_tray_icon();
        data.uFlags = NIF_MESSAGE | NIF_TIP;
        if !icon.is_invalid() {
            data.uFlags |= NIF_ICON;
            data.hIcon = icon;
        }
        data.uCallbackMessage = WM_APP_TRAY;

        for (idx, ch) in tip.encode_utf16().enumerate().take(127) {
            data.szTip[idx] = ch;
        }

        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
        let ok = if !added {
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
        } else {
            true
        };

        if owned_icon && !icon.is_invalid() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
        ok
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

    /// 处理 `WM_APP_TRAY` 鼠标回调：推送单击/双击/菜单事件并在右键时弹出原生菜单。
    pub(crate) fn handle_tray_callback(hwnd: isize, lparam: isize) {
        let event_msg = (lparam as u32) & 0xFFFF;
        match event_msg {
            WM_LBUTTONUP => push_tray_event(TrayEvent::LeftClick),
            WM_LBUTTONDBLCLK => push_tray_event(TrayEvent::DoubleClick),
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                push_tray_event(TrayEvent::ContextMenu);
                let items = active_tray_menu_items(3000);
                if items.is_empty() {
                    return;
                }
                let hwnd = hwnd_from_isize(hwnd);
                let Ok(hmenu) = (unsafe { CreatePopupMenu() }) else {
                    return;
                };
                for item in &items {
                    if item.separator {
                        let _ = unsafe { AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null()) };
                    } else {
                        let mut flags = MF_STRING;
                        if !item.enabled {
                            flags |= MF_GRAYED;
                        }
                        let wide: Vec<u16> = item
                            .label
                            .encode_utf16()
                            .chain(core::iter::once(0))
                            .collect();
                        let _ = unsafe {
                            AppendMenuW(hmenu, flags, item.id as usize, PCWSTR(wide.as_ptr()))
                        };
                    }
                }
                let mut pt = POINT::default();
                let _ = unsafe { GetCursorPos(&mut pt) };
                let _ = unsafe { SetForegroundWindow(hwnd) };
                let cmd = unsafe {
                    TrackPopupMenu(
                        hmenu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                        pt.x,
                        pt.y,
                        Some(0),
                        hwnd,
                        None,
                    )
                };
                let _ = unsafe { DestroyMenu(hmenu) };
                let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
                if cmd.0 > 0 {
                    push_tray_event(TrayEvent::MenuCommand(cmd.0 as u32));
                }
            }
            _ => {}
        }
    }
}

/// 获取 Win32 `TaskbarCreated` 广播消息号（仅供 Win32 WndProc 使用）。
#[cfg(windows)]
pub(crate) fn taskbar_created_message() -> u32 {
    win32::taskbar_created_message()
}

/// 处理 Win32 `WM_APP_TRAY` 回调消息（仅供 Win32 WndProc 使用）。
#[cfg(windows)]
pub(crate) fn handle_tray_callback(hwnd: isize, lparam: isize) {
    win32::handle_tray_callback(hwnd, lparam);
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

/// 为指定窗口显示或隐藏系统托盘图标，并缓存最近一次配置以支持重挂。
pub fn set_tray_icon(hwnd: isize, tooltip: &str, visible: bool) -> bool {
    {
        let mut state = tray_state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.last_icon = Some((hwnd, tooltip.to_owned(), visible));
    }

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

/// 配置托盘右键常规菜单项列表。
pub fn set_tray_menu(hwnd: isize, items: &[TrayMenuItem]) {
    let _ = hwnd;
    let mut state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.menu_items = items.to_vec();
}

/// 配置心跳停滞时切换展示的托盘急救菜单项列表。
pub fn set_tray_rescue_menu(hwnd: isize, items: &[TrayMenuItem]) {
    let _ = hwnd;
    let mut state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.rescue_menu_items = items.to_vec();
}

/// 上报主线程心跳时间戳，用于判断 UI 主循环是否停滞。
pub fn update_tray_heartbeat(hwnd: isize) {
    let _ = hwnd;
    let mut state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.last_heartbeat = Some(Instant::now());
}

/// 检查主线程心跳是否已超过给定毫秒阈值（未记录过心跳时返回 `false`）。
#[must_use]
pub fn is_tray_heartbeat_stalled(threshold_ms: u64) -> bool {
    let state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(forced) = state.heartbeat_stalled_override {
        return forced;
    }
    state
        .last_heartbeat
        .is_some_and(|ts| ts.elapsed().as_millis() >= u128::from(threshold_ms))
}

/// 返回当前生效的托盘菜单项：当心跳停滞且配置了急救菜单时返回急救菜单，否则返回常规菜单。
#[must_use]
pub fn active_tray_menu_items(threshold_ms: u64) -> Vec<TrayMenuItem> {
    let state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stalled = if let Some(forced) = state.heartbeat_stalled_override {
        forced
    } else {
        state
            .last_heartbeat
            .is_some_and(|ts| ts.elapsed().as_millis() >= u128::from(threshold_ms))
    };
    if stalled && !state.rescue_menu_items.is_empty() {
        state.rescue_menu_items.clone()
    } else {
        state.menu_items.clone()
    }
}

/// 测试辅助：覆盖心跳停滞判定结果（传 `None` 恢复真实时钟判定）。
pub fn set_tray_heartbeat_stalled_for_test(stalled: Option<bool>) {
    let mut state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.heartbeat_stalled_override = stalled;
}

/// 测试辅助：向托盘事件队列手动推入一条事件。
pub fn push_tray_event_for_test(ev: TrayEvent) {
    push_tray_event(ev);
}

/// 在 Explorer 重启（`TaskbarCreated`）或显式调用时重新挂载当前可见的托盘图标。
pub fn reattach_tray_icon(hwnd: isize) -> bool {
    let cached = {
        let state = tray_state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.last_icon.clone()
    };
    let Some((cached_hwnd, tooltip, visible)) = cached else {
        return false;
    };
    if !visible {
        return false;
    }
    let target_hwnd = if hwnd != 0 { hwnd } else { cached_hwnd };
    #[cfg(windows)]
    let ok = win32::set_tray_icon(target_hwnd, &tooltip, true);
    #[cfg(not(windows))]
    let ok = {
        let _ = (target_hwnd, tooltip);
        true
    };
    if ok {
        push_tray_event(TrayEvent::Reattached);
    }
    ok
}

/// 排空并返回自上次轮询以来积累的所有托盘事件。
#[must_use]
pub fn poll_tray_events() -> Vec<TrayEvent> {
    let mut state = tray_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.events.drain(..).collect()
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

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

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

    #[test]
    fn tray_heartbeat_stall_switches_to_rescue_menu() {
        let _lock = test_guard();
        let normal = vec![TrayMenuItem {
            id: 1,
            label: "Show Window".to_owned(),
            enabled: true,
            separator: false,
        }];
        let rescue = vec![TrayMenuItem {
            id: 99,
            label: "Force Restart".to_owned(),
            enabled: true,
            separator: false,
        }];
        set_tray_menu(0, &normal);
        set_tray_rescue_menu(0, &rescue);
        update_tray_heartbeat(0);

        set_tray_heartbeat_stalled_for_test(Some(false));
        assert!(!is_tray_heartbeat_stalled(3000));
        assert_eq!(active_tray_menu_items(3000), normal);

        set_tray_heartbeat_stalled_for_test(Some(true));
        assert!(is_tray_heartbeat_stalled(3000));
        assert_eq!(active_tray_menu_items(3000), rescue);

        // 清空急救菜单时即使停滞也回退常规菜单。
        set_tray_rescue_menu(0, &[]);
        assert_eq!(active_tray_menu_items(3000), normal);

        set_tray_heartbeat_stalled_for_test(None);
    }

    #[test]
    fn tray_reattach_and_event_queue_polling() {
        let _lock = test_guard();
        let _ = poll_tray_events();

        // 隐藏态不触发重挂。
        let _ = set_tray_icon(42, "", false);
        assert!(!reattach_tray_icon(42));
        assert!(poll_tray_events().is_empty());

        // 可见态重挂后应推入 Reattached 事件，并与手动推入事件按 FIFO 顺序消费。
        let _ = set_tray_icon(42, "Unbound UI", true);
        push_tray_event_for_test(TrayEvent::LeftClick);
        push_tray_event_for_test(TrayEvent::MenuCommand(7));
        #[cfg(not(windows))]
        {
            assert!(reattach_tray_icon(42));
            assert_eq!(
                poll_tray_events(),
                vec![
                    TrayEvent::LeftClick,
                    TrayEvent::MenuCommand(7),
                    TrayEvent::Reattached,
                ]
            );
        }
        #[cfg(windows)]
        {
            let _ = reattach_tray_icon(42);
            let drained = poll_tray_events();
            assert!(drained.starts_with(&[TrayEvent::LeftClick, TrayEvent::MenuCommand(7)]));
        }
        assert!(poll_tray_events().is_empty());

        let _ = set_tray_icon(42, "", false);
    }

    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    #[test]
    fn linux_tray_and_balloon_degrade_safely_without_bus() {
        let _lock = test_guard();
        // 无 D-Bus 会话或无托盘宿主时应安全返回布尔值而不 panic
        let _ = set_tray_icon(0, "Unbound UI", true);
        let _ = show_tray_balloon(0, "Notice", "Body", TrayNotificationSeverity::Info);
        let _ = set_tray_icon(0, "", false);
    }
}

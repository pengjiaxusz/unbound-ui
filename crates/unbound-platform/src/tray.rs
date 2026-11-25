//! Windows 任务栏进度条与系统通知托盘集成。
//!
//! 基于官方 `windows = "0.62.2"` crate 提供以下功能：
//! - 任务栏按钮进度指示器（`ITaskbarList3`：`TBPF_NORMAL`、`TBPF_ERROR`、`TBPF_PAUSED`、`TBPF_INDETERMINATE`）。
//! - 系统通知托盘图标（`Shell_NotifyIconW`），支持应用图标加载、气泡通知、悬停提示、
//!   右键弹出菜单、急救菜单心跳切换、Explorer 重启后 `TaskbarCreated` 自动重挂以及点击事件队列。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// 托盘图标回调窗口消息号（避开 `win32_window::WM_APP_WAKE = 0x8001`）。
pub const WM_APP_TRAY: u32 = 0x8002;
/// 独立托盘消息线程的内部命令投递消息号。
#[cfg(windows)]
const WM_APP_TRAY_CMD: u32 = 0x8003;

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

type RescueCallback = Arc<dyn Fn() + Send + Sync>;

/// 独立的托盘急救回调注册表（与 GUI 主线程状态锁完全隔离，确保主线程卡死时零阻塞响应）。
fn rescue_callbacks() -> &'static Mutex<HashMap<u32, RescueCallback>> {
    static CALLBACKS: OnceLock<Mutex<HashMap<u32, RescueCallback>>> = OnceLock::new();
    CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册无需经过 GUI 主线程即可立即执行的同步急救回调（如强制退出或外部诊断转储）。
pub fn register_tray_rescue_callback(command_id: u32, callback: impl Fn() + Send + Sync + 'static) {
    let mut map = rescue_callbacks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.insert(command_id, Arc::new(callback));
}

/// 注销指定命令 ID 对应的同步急救回调。
pub fn unregister_tray_rescue_callback(command_id: u32) -> bool {
    let mut map = rescue_callbacks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.remove(&command_id).is_some()
}

/// 清空所有已注册的托盘急救回调（供进程退出或单元测试复位使用）。
pub fn clear_tray_rescue_callbacks() {
    let mut map = rescue_callbacks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.clear();
}

/// 分发一条托盘菜单命令：若命中急救回调则在当前线程无锁立即执行并返回 `true`，
/// 否则投递常规 [`TrayEvent::MenuCommand`] 至主线程异步队列并返回 `false`。
pub(crate) fn dispatch_tray_menu_command(command_id: u32) -> bool {
    let callback = {
        let map = rescue_callbacks()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(&command_id).cloned()
    };
    if let Some(cb) = callback {
        cb();
        return true;
    }
    push_tray_event(TrayEvent::MenuCommand(command_id));
    false
}

/// 测试辅助：模拟用户在托盘右键菜单选中指定命令 ID，返回是否触发了同步急救回调。
pub fn dispatch_tray_menu_command_for_test(command_id: u32) -> bool {
    dispatch_tray_menu_command(command_id)
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

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    use ::windows::core::{w, PCWSTR};
    use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use ::windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use ::windows::Win32::UI::Shell::{
        ExtractIconExW, ITaskbarList3, Shell_NotifyIconW, TaskbarList, NIF_ICON, NIF_INFO,
        NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW, TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW, RegisterClassW,
        RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON,
        HWND_MESSAGE, IDI_APPLICATION, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, TPM_NONOTIFY,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CONTEXTMENU,
        WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
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
                    let _ = dispatch_tray_menu_command(cmd.0 as u32);
                }
            }
            _ => {}
        }
    }

    enum TrayThreadCommand {
        SetIcon {
            tooltip: String,
            visible: bool,
        },
        ShowBalloon {
            title: String,
            message: String,
            severity: TrayNotificationSeverity,
        },
        Reattach {
            tooltip: String,
        },
    }

    struct TrayThreadHandle {
        hwnd: isize,
        tx: mpsc::Sender<TrayThreadCommand>,
    }

    static TRAY_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
    static TRAY_THREAD_RX: OnceLock<Mutex<Option<mpsc::Receiver<TrayThreadCommand>>>> =
        OnceLock::new();

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let taskbar_created = taskbar_created_message();
        if taskbar_created != 0 && msg == taskbar_created {
            let _ = super::reattach_tray_icon(hwnd.0 as isize);
            return LRESULT(0);
        }
        match msg {
            WM_APP_TRAY => {
                handle_tray_callback(hwnd.0 as isize, lparam.0);
                LRESULT(0)
            }
            WM_APP_TRAY_CMD => {
                if let Some(lock) = TRAY_THREAD_RX.get() {
                    let mut cmds = Vec::new();
                    if let Ok(guard) = lock.lock() {
                        if let Some(rx) = guard.as_ref() {
                            while let Ok(cmd) = rx.try_recv() {
                                cmds.push(cmd);
                            }
                        }
                    }
                    for cmd in cmds {
                        match cmd {
                            TrayThreadCommand::SetIcon { tooltip, visible } => {
                                let _ = set_tray_icon(hwnd.0 as isize, &tooltip, visible);
                            }
                            TrayThreadCommand::ShowBalloon {
                                title,
                                message,
                                severity,
                            } => {
                                let _ =
                                    show_tray_balloon(hwnd.0 as isize, &title, &message, severity);
                            }
                            TrayThreadCommand::Reattach { tooltip } => {
                                if set_tray_icon(hwnd.0 as isize, &tooltip, true) {
                                    push_tray_event(TrayEvent::Reattached);
                                }
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// 惰性启动专用后台 OS 线程与 `HWND_MESSAGE` 隐藏消息窗口，运行独立 Win32 托盘消息循环。
    fn ensure_tray_thread() -> Option<&'static TrayThreadHandle> {
        static HANDLE: OnceLock<Option<TrayThreadHandle>> = OnceLock::new();
        HANDLE
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel::<TrayThreadCommand>();
                let _ = TRAY_THREAD_RX.set(Mutex::new(Some(rx)));
                let (ready_tx, ready_rx) = mpsc::sync_channel::<isize>(1);
                let spawned = std::thread::Builder::new()
                    .name("unbound-tray".to_owned())
                    .spawn(move || {
                        let class_name = w!("UnboundTrayMessageWindow");
                        let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
                        let wc = WNDCLASSW {
                            lpfnWndProc: Some(tray_wnd_proc),
                            hInstance: hinstance.into(),
                            lpszClassName: class_name,
                            ..Default::default()
                        };
                        unsafe {
                            let _ = RegisterClassW(&wc);
                        }
                        let hwnd = unsafe {
                            CreateWindowExW(
                                WINDOW_EX_STYLE::default(),
                                class_name,
                                w!("UnboundTray"),
                                WINDOW_STYLE::default(),
                                0,
                                0,
                                0,
                                0,
                                Some(HWND_MESSAGE),
                                None,
                                Some(hinstance.into()),
                                None,
                            )
                        };
                        let Ok(hwnd) = hwnd else {
                            let _ = ready_tx.send(0);
                            return;
                        };
                        TRAY_THREAD_RUNNING.store(true, Ordering::Release);
                        let _ = ready_tx.send(hwnd.0 as isize);
                        let mut msg = MSG::default();
                        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                            unsafe {
                                let _ = TranslateMessage(&msg);
                                let _ = DispatchMessageW(&msg);
                            }
                        }
                        TRAY_THREAD_RUNNING.store(false, Ordering::Release);
                    });
                if spawned.is_err() {
                    return None;
                }
                let raw_hwnd = ready_rx.recv().unwrap_or(0);
                if raw_hwnd == 0 {
                    return None;
                }
                Some(TrayThreadHandle { hwnd: raw_hwnd, tx })
            })
            .as_ref()
    }

    pub fn is_tray_thread_running() -> bool {
        TRAY_THREAD_RUNNING.load(Ordering::Acquire)
    }

    pub fn set_tray_icon_threaded(fallback_hwnd: isize, tooltip: &str, visible: bool) -> bool {
        if let Some(handle) = ensure_tray_thread() {
            if handle
                .tx
                .send(TrayThreadCommand::SetIcon {
                    tooltip: tooltip.to_owned(),
                    visible,
                })
                .is_ok()
            {
                let _ = unsafe {
                    PostMessageW(
                        Some(hwnd_from_isize(handle.hwnd)),
                        WM_APP_TRAY_CMD,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
                return true;
            }
        }
        set_tray_icon(fallback_hwnd, tooltip, visible)
    }

    pub fn show_tray_balloon_threaded(
        fallback_hwnd: isize,
        title: &str,
        message: &str,
        severity: TrayNotificationSeverity,
    ) -> bool {
        if let Some(handle) = ensure_tray_thread() {
            if handle
                .tx
                .send(TrayThreadCommand::ShowBalloon {
                    title: title.to_owned(),
                    message: message.to_owned(),
                    severity,
                })
                .is_ok()
            {
                let _ = unsafe {
                    PostMessageW(
                        Some(hwnd_from_isize(handle.hwnd)),
                        WM_APP_TRAY_CMD,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
                return true;
            }
        }
        show_tray_balloon(fallback_hwnd, title, message, severity)
    }

    pub fn reattach_tray_icon_threaded(fallback_hwnd: isize, tooltip: &str) -> bool {
        if let Some(handle) = ensure_tray_thread() {
            if handle
                .tx
                .send(TrayThreadCommand::Reattach {
                    tooltip: tooltip.to_owned(),
                })
                .is_ok()
            {
                let _ = unsafe {
                    PostMessageW(
                        Some(hwnd_from_isize(handle.hwnd)),
                        WM_APP_TRAY_CMD,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
                return true;
            }
        }
        if set_tray_icon(fallback_hwnd, tooltip, true) {
            push_tray_event(TrayEvent::Reattached);
            true
        } else {
            false
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
        win32::set_tray_icon_threaded(hwnd, tooltip, visible)
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
    {
        win32::reattach_tray_icon_threaded(target_hwnd, &tooltip)
    }
    #[cfg(not(windows))]
    {
        let _ = (target_hwnd, tooltip);
        push_tray_event(TrayEvent::Reattached);
        true
    }
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
        win32::show_tray_balloon_threaded(hwnd, title, message, severity)
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

/// 独立 OS 线程原生系统托盘句柄：封装托盘图标、常规/急救菜单、零锁急救回调与事件轮询。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemTray {
    owner_hwnd: isize,
}

impl SystemTray {
    /// 创建绑定到指定宿主窗口句柄（无头或非 Windows 下可传 `0`）的系统托盘句柄。
    #[must_use]
    pub const fn new(owner_hwnd: isize) -> Self {
        Self { owner_hwnd }
    }

    /// 显示或隐藏系统托盘图标（Windows 下自动由独立 `HWND_MESSAGE` 后台线程承载）。
    pub fn set_icon(&self, tooltip: &str, visible: bool) -> bool {
        set_tray_icon(self.owner_hwnd, tooltip, visible)
    }

    /// 配置托盘右键常规菜单项列表。
    pub fn set_menu(&self, items: &[TrayMenuItem]) {
        set_tray_menu(self.owner_hwnd, items);
    }

    /// 配置主线程卡死时自动切换展示的急救菜单项列表。
    pub fn set_rescue_menu(&self, items: &[TrayMenuItem]) {
        set_tray_rescue_menu(self.owner_hwnd, items);
    }

    /// 注册无需经过 GUI 主线程即可立即执行的原生急救回调。
    pub fn register_rescue_callback(
        &self,
        command_id: u32,
        callback: impl Fn() + Send + Sync + 'static,
    ) {
        register_tray_rescue_callback(command_id, callback);
    }

    /// 注销指定命令 ID 的原生急救回调。
    pub fn unregister_rescue_callback(&self, command_id: u32) -> bool {
        unregister_tray_rescue_callback(command_id)
    }

    /// 上报 GUI 主线程心跳时间戳。
    pub fn heartbeat(&self) {
        update_tray_heartbeat(self.owner_hwnd);
    }

    /// 显示系统托盘气泡通知。
    pub fn show_balloon(
        &self,
        title: &str,
        message: &str,
        severity: TrayNotificationSeverity,
    ) -> bool {
        show_tray_balloon(self.owner_hwnd, title, message, severity)
    }

    /// 在 Explorer 重启或显式请求时重新挂载托盘图标。
    pub fn reattach(&self) -> bool {
        reattach_tray_icon(self.owner_hwnd)
    }

    /// 排空自上次轮询以来积累的常规托盘事件。
    #[must_use]
    pub fn poll_events(&self) -> Vec<TrayEvent> {
        poll_tray_events()
    }

    /// 返回 Windows 独立托盘消息线程是否处于运行态（非 Windows 平台恒为 `false`）。
    #[must_use]
    pub fn is_background_thread_running() -> bool {
        #[cfg(windows)]
        {
            win32::is_tray_thread_running()
        }
        #[cfg(not(windows))]
        {
            false
        }
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

    #[test]
    fn rescue_callback_executes_synchronously_without_queuing_regular_event() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let _lock = test_guard();
        clear_tray_rescue_callbacks();
        let _ = poll_tray_events();

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = Arc::clone(&hits);
        let tray = SystemTray::new(0);
        tray.register_rescue_callback(9901, move || {
            hits_clone.fetch_add(1, Ordering::SeqCst);
        });

        // 命中急救命令 ID：同步执行回调且不阻塞/不进入主线程常规队列。
        assert!(dispatch_tray_menu_command_for_test(9901));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(tray.poll_events().is_empty());

        // 未注册急救回调的常规菜单项仍投递至异步 TrayEvent 队列。
        assert!(!dispatch_tray_menu_command_for_test(1002));
        assert_eq!(tray.poll_events(), vec![TrayEvent::MenuCommand(1002)]);

        // 注销急救回调后退回常规事件队列。
        assert!(tray.unregister_rescue_callback(9901));
        assert!(!dispatch_tray_menu_command_for_test(9901));
        assert_eq!(tray.poll_events(), vec![TrayEvent::MenuCommand(9901)]);
        clear_tray_rescue_callbacks();
    }
}

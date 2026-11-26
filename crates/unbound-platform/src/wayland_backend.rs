//! 原生 Wayland 平台后端（基于 `smithay-client-toolkit` 0.19）。
//!
//! 覆盖一期 Spec（#204/#205/#206）：
//! - `xdg_toplevel` 主窗与多窗（`Second Window`）生命周期；
//! - `wl_pointer`、`wl_keyboard`（含软件按键重复）、`wl_output` 主屏几何/缩放采集；
//! - 高分辨率单调时钟帧等待（`ppoll` 纳秒级超时）与 `wl_surface::frame` 驱动的可见性节流；
//! - CSD 自绘标题栏拖拽移动、四边/四角拖拽缩放、窗口控制按钮与 `ThemedPointer` 光标映射；
//! - `wl_data_device` 系统剪贴板文本读写桥接；
//! - `xdg_popup` + `xdg_positioner` 越界浮层（自动翻转/滑动/压缩，ToolTip 鼠标穿透不抢焦点）。

use core::ffi::c_void;
use core::ptr::NonNull;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle as RwhWaylandWindowHandle,
    WindowHandle,
};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::{
    CopyPasteSource, DataSourceHandler, DragSource,
};
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{
    Window as SctkWindow, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::{XdgShell, XdgSurface};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::{Buffer as ShmBuffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_xdg_popup, delegate_xdg_shell,
    delegate_xdg_window, registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{
    wl_data_device_manager::DndAction, wl_data_source::WlDataSource, wl_keyboard, wl_output,
    wl_pointer, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::xdg::shell::client::{xdg_positioner, xdg_toplevel};

use crate::backend::{push_coalesced_event, Flow, PlatformApp};
use crate::floating::{FloatingWindowCapabilities, MonitorWorkArea, WindowRole};
use crate::native_window::NativeWindow;
use crate::pacing::PresentWaitPolicy;
use crate::popup_host::PopupWindowConfig;
use crate::resize::Extent;
use crate::wayland_logic::{
    compute_wayland_ime_cursor_rect, compute_wayland_popup_placement, csd_resize_direction_at,
    cursor_for_resize_direction, register_wayland_clipboard, set_primary_output_metrics,
    translate_evdev_key, KeyRepeatState, PrimaryOutputMetrics, WaylandClipboardBackend,
    WaylandFrameThrottle, WaylandImeProtocolAction, WaylandImeState,
};
use crate::win32_frame::FrameHitMetrics;
use crate::win32_window::{
    CursorIcon, PhysicalPosition, PhysicalSize, ResizeDirection, WindowId as NativeWindowId,
};
use crate::window::{
    apply, ButtonState, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformState,
    ScreenRect, WindowConfig,
};

const WAYLAND_CLIPBOARD_MIME: &str = "text/plain;charset=utf-8";
const WAYLAND_CSD_RESIZE_BORDER: f64 = 8.0;

/// 应用侧向 Wayland 事件循环发送的窗口控制命令。
enum WaylandCommand {
    SetTitle {
        id: isize,
        title: String,
    },
    SetMinimized {
        id: isize,
        minimized: bool,
    },
    SetMaximized {
        id: isize,
        maximized: bool,
    },
    SetVisible {
        id: isize,
        visible: bool,
    },
    SetClickThrough {
        id: isize,
        click_through: bool,
    },
    SetCursor {
        cursor: CursorIcon,
    },
    DragWindow {
        id: isize,
    },
    DragResizeWindow {
        id: isize,
        direction: ResizeDirection,
    },
    RequestClose {
        id: isize,
    },
    RequestRedraw {
        id: isize,
    },
    ClipboardWrite {
        text: String,
    },
    ClipboardWriteFiles {
        uri_list: String,
        gnome_copied_files: String,
    },
    SetImeAllowed {
        id: isize,
        allowed: bool,
    },
    SetImeCursorArea {
        id: isize,
        position: PhysicalPosition,
        size: PhysicalSize,
    },
    ThemeChanged {
        dark: bool,
    },
    FilesDropped {
        surface_id: u32,
        paths: Vec<std::path::PathBuf>,
    },
    StartOutboundFileDrag {
        uri_list: String,
        actions: DndAction,
        reply: mpsc::Sender<crate::ole_drag::OutboundFileDragOutcome>,
    },
}

/// 线程安全的管道唤醒信号句柄（持锁写入并在释放时持锁关闭，彻底消除 FD 复用竞态）。
#[derive(Clone)]
struct WakeSignal(Arc<Mutex<Option<RawFd>>>);

impl WakeSignal {
    fn signal(&self) {
        let Ok(guard) = self.0.lock() else {
            return;
        };
        let Some(fd) = *guard else {
            return;
        };
        let byte: u8 = 1;
        // SAFETY: 持锁期间 `WakePipe::drop` 不会关闭 `fd`，且该管道写端为非阻塞模式。
        unsafe {
            let _ = libc::write(fd, (&raw const byte).cast::<c_void>(), 1);
        }
    }
}

/// 跨线程唤醒事件循环的管道，释放时持锁摘除写端再关闭描述符。
struct WakePipe {
    read_fd: RawFd,
    write_fd: Arc<Mutex<Option<RawFd>>>,
}

impl WakePipe {
    fn new() -> Result<Self, PlatformError> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `fds` 为有效的双元素数组，创建非阻塞管道用于唤醒 `ppoll`。
        let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if rc != 0 {
            return Err(PlatformError::EventLoop(
                "failed to create Wayland wake pipe".into(),
            ));
        }
        unsafe {
            for fd in fds {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        Ok(Self {
            read_fd: fds[0],
            write_fd: Arc::new(Mutex::new(Some(fds[1]))),
        })
    }

    fn signal_handle(&self) -> WakeSignal {
        WakeSignal(Arc::clone(&self.write_fd))
    }

    fn drain(&self) {
        let mut buf = [0u8; 64];
        loop {
            // SAFETY: 读取自身持有的非阻塞管道读端以清空唤醒字节。
            let n =
                unsafe { libc::read(self.read_fd, buf.as_mut_ptr().cast::<c_void>(), buf.len()) };
            if n <= 0 {
                break;
            }
        }
    }
}

impl Drop for WakePipe {
    fn drop(&mut self) {
        let write_fd = self.write_fd.lock().ok().and_then(|mut g| g.take());
        // SAFETY: 写端已在互斥锁内摘除，释放自身持有的管道描述符各一次。
        unsafe {
            if let Some(fd) = write_fd {
                let _ = libc::close(fd);
            }
            let _ = libc::close(self.read_fd);
        }
    }
}

/// RAII 守卫：确保 `run_event_loop` 任意正常或异常退出路径都注销全局 Wayland 剪贴板桥接。
struct ClipboardRegistrationGuard;

impl Drop for ClipboardRegistrationGuard {
    fn drop(&mut self) {
        register_wayland_clipboard(None);
        crate::linux_dnd::register_linux_outbound_drag(None);
        crate::linux_backend::set_active_linux_backend(None);
    }
}

struct WaylandOutboundDragBridge {
    event_loop_thread: std::thread::ThreadId,
    cmd_tx: Mutex<mpsc::Sender<WaylandCommand>>,
    wake_signal: WakeSignal,
}

impl crate::linux_dnd::LinuxOutboundDragBackend for WaylandOutboundDragBridge {
    fn begin_file_drag(
        &self,
        request: &crate::ole_drag::OutboundFileDragRequest,
    ) -> Result<crate::ole_drag::OutboundFileDragOutcome, PlatformError> {
        let mut actions = DndAction::empty();
        if request.allow_copy {
            actions |= DndAction::Copy;
        }
        if request.allow_move {
            actions |= DndAction::Move;
        }
        if actions.is_empty() {
            actions = DndAction::Copy;
        }
        let uri_list = crate::linux_dnd::encode_uri_list(&request.paths);
        let (reply_tx, reply_rx) = mpsc::channel();
        let sent = self.cmd_tx.lock().ok().is_some_and(|tx| {
            tx.send(WaylandCommand::StartOutboundFileDrag {
                uri_list,
                actions,
                reply: reply_tx,
            })
            .is_ok()
        });
        if !sent {
            return Ok(crate::ole_drag::OutboundFileDragOutcome::Cancelled);
        }
        self.wake_signal.signal();
        // 若由事件循环自身线程调用，严禁同步阻塞等待，避免自死锁：拖拽已在命令队列排队，直接返回已启动结果。
        if std::thread::current().id() == self.event_loop_thread {
            let effect = if request.allow_copy {
                crate::overlay_host::FileDragEffect::Copy
            } else {
                crate::overlay_host::FileDragEffect::Move
            };
            return Ok(crate::ole_drag::OutboundFileDragOutcome::Completed(effect));
        }
        Ok(reply_rx
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or(crate::ole_drag::OutboundFileDragOutcome::Cancelled))
    }
}

/// 轻量探测当前环境是否可建立 Wayland 合成器连接（供 `auto` 后端回退决策使用）。
#[must_use]
pub(crate) fn can_connect_wayland() -> bool {
    Connection::connect_to_env().is_ok()
}

/// 窗口句柄与事件循环共享的状态快照。
struct WaylandWindowShared {
    id: isize,
    display_ptr: usize,
    surface_ptr: usize,
    decorated: bool,
    popup_id: Option<u64>,
    inner_size: Mutex<PhysicalSize>,
    scale_bits: AtomicU64,
    maximized: AtomicBool,
    visible: AtomicBool,
    destroyed: AtomicBool,
    redraw_requested: AtomicBool,
    cursor: AtomicU32,
    frame_metrics: Mutex<Option<Arc<FrameHitMetrics>>>,
    cmd_tx: Mutex<mpsc::Sender<WaylandCommand>>,
    wake_signal: WakeSignal,
}

/// Wayland 原生窗口句柄（由 [`NativeWindow`] 包装并暴露给上层应用）。
#[derive(Clone)]
pub(crate) struct WaylandWindowHandle {
    shared: Arc<WaylandWindowShared>,
}

impl WaylandWindowHandle {
    fn send(&self, cmd: WaylandCommand) {
        if let Ok(tx) = self.shared.cmd_tx.lock() {
            let _ = tx.send(cmd);
            self.shared.wake_signal.signal();
        }
    }

    pub(crate) fn is_popup(&self) -> bool {
        self.shared.popup_id.is_some()
    }

    pub(crate) fn popup_id(&self) -> Option<u64> {
        self.shared.popup_id
    }

    pub(crate) fn id(&self) -> NativeWindowId {
        NativeWindowId(self.shared.id)
    }

    pub(crate) fn current_cursor(&self) -> CursorIcon {
        CursorIcon::from_u32(self.shared.cursor.load(Ordering::Acquire))
    }

    pub(crate) fn inner_size(&self) -> PhysicalSize {
        self.shared
            .inner_size
            .lock()
            .map(|g| *g)
            .unwrap_or_else(|_| PhysicalSize::new(1, 1))
    }

    pub(crate) fn client_rect_on_screen(&self) -> ScreenRect {
        // Wayland 不暴露全局绝对坐标，客户区原点始终以局部 (0, 0) 对齐主屏壁纸采样。
        let size = self.inner_size();
        ScreenRect::new(0, 0, size.width, size.height)
    }

    pub(crate) fn scale_factor(&self) -> f64 {
        let bits = self.shared.scale_bits.load(Ordering::Acquire);
        let scale = f64::from_bits(bits);
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    pub(crate) fn refresh_rate_hz(&self) -> Option<f32> {
        crate::wayland_logic::primary_output_metrics().and_then(|m| m.refresh_rate_hz)
    }

    pub(crate) fn is_visible(&self) -> Option<bool> {
        Some(self.shared.visible.load(Ordering::Acquire))
    }

    pub(crate) fn is_maximized(&self) -> bool {
        self.shared.maximized.load(Ordering::Acquire)
    }

    pub(crate) fn is_minimized(&self) -> bool {
        false
    }

    pub(crate) fn outer_position(&self) -> Option<(i32, i32)> {
        None
    }

    pub(crate) fn normal_bounds(&self) -> Option<ScreenRect> {
        None
    }

    pub(crate) fn set_outer_position(&self, x: i32, y: i32) {
        let _ = (x, y);
    }

    pub(crate) fn set_inner_size(&self, width: u32, height: u32) {
        let _ = (width, height);
    }

    pub(crate) fn activate_foreground(&self) {
        if !self.shared.visible.load(Ordering::Acquire) {
            self.set_visible(true);
        }
    }

    pub(crate) fn is_decorated(&self) -> bool {
        self.shared.decorated
    }

    pub(crate) fn set_visible(&self, visible: bool) {
        self.shared.visible.store(visible, Ordering::Release);
        self.send(WaylandCommand::SetVisible {
            id: self.shared.id,
            visible,
        });
    }

    pub(crate) fn set_visible_no_activate(&self, visible: bool) {
        self.set_visible(visible);
    }

    pub(crate) fn set_always_on_top(&self, topmost: bool) {
        // Wayland `xdg_toplevel` does not support client-driven always-on-top; degrade safely.
        let _ = topmost;
    }

    pub(crate) fn is_always_on_top(&self) -> bool {
        false
    }

    pub(crate) fn set_click_through(&self, click_through: bool) {
        self.send(WaylandCommand::SetClickThrough {
            id: self.shared.id,
            click_through,
        });
    }

    pub(crate) fn set_outer_rect(&self, rect: ScreenRect) {
        // Wayland `xdg_toplevel` 不暴露全局坐标定位，安全无操作。
        let _ = rect;
    }

    pub(crate) fn capabilities(&self) -> FloatingWindowCapabilities {
        FloatingWindowCapabilities::wayland_xdg_fallback()
    }

    pub(crate) fn work_area(&self) -> Option<MonitorWorkArea> {
        crate::floating::work_area_for_point(0, 0)
    }

    pub(crate) fn set_minimized(&self, minimized: bool) {
        self.send(WaylandCommand::SetMinimized {
            id: self.shared.id,
            minimized,
        });
    }

    pub(crate) fn set_maximized(&self, maximized: bool) {
        self.send(WaylandCommand::SetMaximized {
            id: self.shared.id,
            maximized,
        });
    }

    pub(crate) fn set_title(&self, title: &str) {
        self.send(WaylandCommand::SetTitle {
            id: self.shared.id,
            title: title.to_owned(),
        });
    }

    pub(crate) fn set_cursor(&self, cursor: CursorIcon) {
        self.shared.cursor.store(cursor.to_u32(), Ordering::Release);
        self.send(WaylandCommand::SetCursor { cursor });
    }

    pub(crate) fn request_redraw(&self) {
        self.shared.redraw_requested.store(true, Ordering::Release);
        self.send(WaylandCommand::RequestRedraw { id: self.shared.id });
    }

    pub(crate) fn request_close(&self) {
        self.send(WaylandCommand::RequestClose { id: self.shared.id });
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        self.shared.destroyed.load(Ordering::Acquire)
    }

    pub(crate) fn drag_window(&self) -> Result<(), PlatformError> {
        self.send(WaylandCommand::DragWindow { id: self.shared.id });
        Ok(())
    }

    pub(crate) fn drag_resize_window(
        &self,
        direction: ResizeDirection,
    ) -> Result<(), PlatformError> {
        self.send(WaylandCommand::DragResizeWindow {
            id: self.shared.id,
            direction,
        });
        Ok(())
    }

    pub(crate) fn install_custom_frame(&self, metrics: Arc<FrameHitMetrics>) -> bool {
        if let Ok(mut guard) = self.shared.frame_metrics.lock() {
            *guard = Some(metrics);
            true
        } else {
            false
        }
    }

    pub(crate) fn set_ime_allowed(&self, allowed: bool) {
        self.send(WaylandCommand::SetImeAllowed {
            id: self.shared.id,
            allowed,
        });
    }

    pub(crate) fn set_ime_cursor_area(&self, position: PhysicalPosition, size: PhysicalSize) {
        self.send(WaylandCommand::SetImeCursorArea {
            id: self.shared.id,
            position,
            size,
        });
    }
}

impl HasWindowHandle for WaylandWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let ptr =
            NonNull::new(self.shared.surface_ptr as *mut c_void).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Wayland(RwhWaylandWindowHandle::new(ptr));
        // SAFETY: `wl_surface*` 指针在窗口存活期间有效。
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for WaylandWindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let ptr =
            NonNull::new(self.shared.display_ptr as *mut c_void).ok_or(HandleError::Unavailable)?;
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(ptr));
        // SAFETY: `wl_display*` 指针在连接存活期间有效。
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

/// 跨线程 Wayland 剪贴板桥接实现，挂接到 [`crate::SystemClipboard`]。
struct WaylandClipboardBridge {
    /// 最近一次读取或写入的缓存文本，供同步读路径无阻塞返回。
    cached_text: Arc<Mutex<Option<String>>>,
    cached_files: Arc<Mutex<Vec<std::path::PathBuf>>>,
    cached_file_effect: Arc<Mutex<Option<crate::clipboard::ClipboardFileDropEffect>>>,
    cmd_tx: Mutex<mpsc::Sender<WaylandCommand>>,
    wake_signal: WakeSignal,
}

impl WaylandClipboardBackend for WaylandClipboardBridge {
    fn read_text(&self) -> Option<String> {
        self.cached_text.lock().ok().and_then(|g| g.clone())
    }

    fn write_text(&self, text: &str) -> bool {
        if let Ok(mut guard) = self.cached_text.lock() {
            *guard = Some(text.to_owned());
        }
        if let Ok(tx) = self.cmd_tx.lock() {
            let ok = tx
                .send(WaylandCommand::ClipboardWrite {
                    text: text.to_owned(),
                })
                .is_ok();
            if ok {
                self.wake_signal.signal();
            }
            ok
        } else {
            false
        }
    }

    fn read_file_paths(&self) -> Option<Vec<std::path::PathBuf>> {
        self.cached_files.lock().ok().map(|g| g.clone())
    }

    fn read_file_drop_effect(&self) -> Option<crate::clipboard::ClipboardFileDropEffect> {
        self.cached_file_effect.lock().ok().and_then(|g| *g)
    }

    fn write_file_paths(
        &self,
        paths: &[std::path::PathBuf],
        effect: crate::clipboard::ClipboardFileDropEffect,
        uri_list: &str,
        gnome_copied_files: &str,
    ) -> bool {
        if let Ok(mut guard) = self.cached_files.lock() {
            *guard = paths.to_vec();
        }
        if let Ok(mut guard) = self.cached_file_effect.lock() {
            *guard = Some(effect);
        }
        if let Ok(tx) = self.cmd_tx.lock() {
            let ok = tx
                .send(WaylandCommand::ClipboardWriteFiles {
                    uri_list: uri_list.to_owned(),
                    gnome_copied_files: gnome_copied_files.to_owned(),
                })
                .is_ok();
            if ok {
                self.wake_signal.signal();
            }
            ok
        } else {
            false
        }
    }
}

/// 映射框架光标枚举至 `cursor_icon::CursorIcon`（供 `ThemedPointer` 统一走 `cursor-shape-v1` / `wl_cursor`）。
const fn to_sctk_cursor(icon: CursorIcon) -> smithay_client_toolkit::seat::pointer::CursorIcon {
    use smithay_client_toolkit::seat::pointer::CursorIcon as SctkIcon;
    match icon {
        CursorIcon::Default | CursorIcon::Custom(_) => SctkIcon::Default,
        CursorIcon::Pointer => SctkIcon::Pointer,
        CursorIcon::Text => SctkIcon::Text,
        CursorIcon::Grab => SctkIcon::Grab,
        CursorIcon::Grabbing => SctkIcon::Grabbing,
        CursorIcon::Crosshair => SctkIcon::Crosshair,
        CursorIcon::Move => SctkIcon::Move,
        CursorIcon::NsResize => SctkIcon::NsResize,
        CursorIcon::EwResize => SctkIcon::EwResize,
        CursorIcon::NwseResize => SctkIcon::NwseResize,
        CursorIcon::NeswResize => SctkIcon::NeswResize,
        CursorIcon::NotAllowed => SctkIcon::NotAllowed,
    }
}

/// 映射框架缩放方向至 `xdg_toplevel::ResizeEdge`。
const fn to_xdg_resize_edge(dir: ResizeDirection) -> xdg_toplevel::ResizeEdge {
    match dir {
        ResizeDirection::North => xdg_toplevel::ResizeEdge::Top,
        ResizeDirection::South => xdg_toplevel::ResizeEdge::Bottom,
        ResizeDirection::West => xdg_toplevel::ResizeEdge::Left,
        ResizeDirection::East => xdg_toplevel::ResizeEdge::Right,
        ResizeDirection::NorthWest => xdg_toplevel::ResizeEdge::TopLeft,
        ResizeDirection::NorthEast => xdg_toplevel::ResizeEdge::TopRight,
        ResizeDirection::SouthWest => xdg_toplevel::ResizeEdge::BottomLeft,
        ResizeDirection::SouthEast => xdg_toplevel::ResizeEdge::BottomRight,
    }
}

/// Linux 鼠标按钮编码（`linux/input-event-codes.h`）映射。
const fn translate_linux_button(button: u32) -> MouseButton {
    match button {
        0x110 => MouseButton::Left,
        0x111 => MouseButton::Right,
        0x112 => MouseButton::Middle,
        0x113 => MouseButton::Back,
        0x114 => MouseButton::Forward,
        other => MouseButton::Other(other as u16),
    }
}

/// 顶层窗口运行时条目。
struct ToplevelEntry {
    id: isize,
    sctk_window: SctkWindow,
    native_window: NativeWindow,
    shared: Arc<WaylandWindowShared>,
    state: PlatformState,
    configured: bool,
    resizable: bool,
    min_width: u32,
    min_height: u32,
    throttle: WaylandFrameThrottle,
    was_suspended: bool,
    csd_edge_active: bool,
    pending_events: Vec<PlatformEvent>,
    cursor_logical: (f64, f64),
    a11y: crate::a11y_platform::PlatformA11y,
}

/// 越界原生弹出窗（`xdg_popup`）运行时条目。
struct PopupEntry {
    owner_window_id: isize,
    popup: Popup,
    native_window: NativeWindow,
    shared: Arc<WaylandWindowShared>,
    state: PlatformState,
    pending_events: Vec<PlatformEvent>,
    configured: bool,
    dismissed: bool,
}

/// Wayland 客户端交互缩放兜底状态：优先调用 `xdg_toplevel::resize` 交由合成器接管；
/// 若合成器（如 WSLg Weston RDP RAIL）忽略无边框窗口的交互缩放请求而继续向本客户端派发
/// `PointerEventKind::Motion`，则客户端实时驱动 `set_window_geometry` 与 `Resized` 完成平滑缩放。
struct WaylandActiveResize {
    window_id: isize,
    dir: ResizeDirection,
    start_lx: f64,
    start_ly: f64,
    start_w: u32,
    start_h: u32,
    min_w: u32,
    min_h: u32,
}

/// Wayland 事件循环主状态（实现所有 `sctk` Handler trait）。
struct WaylandState {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: CompositorState,
    shm: Shm,
    xdg_shell: XdgShell,
    data_device_manager: Option<DataDeviceManagerState>,
    data_device: Option<DataDevice>,
    clipboard_source: Option<CopyPasteSource>,
    clipboard_owned_text: String,
    clipboard_owned_uri_list: String,
    clipboard_owned_gnome_files: String,
    clipboard_cache: Arc<Mutex<Option<String>>>,
    dnd_source: Option<DragSource>,
    dnd_outbound_uri_list: String,
    dnd_outbound_reply: Option<mpsc::Sender<crate::ole_drag::OutboundFileDragOutcome>>,
    dnd_outbound_action: DndAction,
    dnd_enter_surface_id: Option<u32>,
    dnd_offer_has_uri_list: bool,

    seat: Option<wl_seat::WlSeat>,
    themed_pointer: Option<ThemedPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    ime_state: WaylandImeState,

    modifiers: Modifiers,
    key_repeat: KeyRepeatState,
    focused_surface_id: Option<u32>,
    pointer_surface_id: Option<u32>,
    last_pointer_serial: u32,
    last_pointer_enter_serial: Option<u32>,
    custom_cursor_surface: Option<wl_surface::WlSurface>,
    custom_cursor_pool: Option<SlotPool>,
    custom_cursor_buffer: Option<ShmBuffer>,
    last_key_serial: u32,
    next_window_id: isize,

    windows: Vec<ToplevelEntry>,
    popups: HashMap<u64, PopupEntry>,
    surface_to_window: HashMap<u32, isize>,
    surface_to_popup: HashMap<u32, u64>,

    cmd_tx: mpsc::Sender<WaylandCommand>,
    wake_signal: WakeSignal,
    display_ptr: usize,
    active_resize: Option<WaylandActiveResize>,
    exit_requested: bool,
}

impl WaylandState {
    fn window_by_id_mut(&mut self, id: isize) -> Option<&mut ToplevelEntry> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    fn window_by_surface_mut(&mut self, surface_id: u32) -> Option<&mut ToplevelEntry> {
        let win_id = *self.surface_to_window.get(&surface_id)?;
        self.window_by_id_mut(win_id)
    }

    /// 向当前 `wl_pointer` 下发光标样式；自定义位图光标走 `wl_shm` 表面，失败时安全回退至默认光标。
    fn apply_cursor(&mut self, conn: &Connection, qh: &QueueHandle<Self>, cursor: CursorIcon) {
        if let CursorIcon::Custom(id) = cursor {
            if let (Some(bitmap), Some(serial)) = (
                crate::custom_cursor::lookup_custom_cursor_bitmap(unbound_input::CustomCursorId(
                    id,
                )),
                self.last_pointer_enter_serial,
            ) {
                if let Some(pointer) = &self.themed_pointer {
                    let width = bitmap.width as i32;
                    let height = bitmap.height as i32;
                    let stride = (bitmap.width as usize) * 4;
                    let byte_len = stride * (bitmap.height as usize);
                    if let Ok(mut pool) = SlotPool::new(byte_len.max(4096), &self.shm) {
                        if let Ok((buffer, canvas)) = pool.create_buffer(
                            width,
                            height,
                            stride as i32,
                            wl_shm::Format::Argb8888,
                        ) {
                            for (src_px, dst_px) in
                                bitmap.rgba.chunks_exact(4).zip(canvas.chunks_exact_mut(4))
                            {
                                let a = u16::from(src_px[3]);
                                let r = ((u16::from(src_px[0]) * a + 127) / 255) as u8;
                                let g = ((u16::from(src_px[1]) * a + 127) / 255) as u8;
                                let b = ((u16::from(src_px[2]) * a + 127) / 255) as u8;
                                dst_px[0] = b;
                                dst_px[1] = g;
                                dst_px[2] = r;
                                dst_px[3] = src_px[3];
                            }
                            let surface = self
                                .custom_cursor_surface
                                .get_or_insert_with(|| self.compositor_state.create_surface(qh));
                            if buffer.attach_to(surface).is_ok() {
                                surface.damage_buffer(0, 0, width, height);
                                surface.commit();
                                pointer.pointer().set_cursor(
                                    serial,
                                    Some(surface),
                                    bitmap.hotspot_x as i32,
                                    bitmap.hotspot_y as i32,
                                );
                                self.custom_cursor_buffer = Some(buffer);
                                self.custom_cursor_pool = Some(pool);
                                let _ = conn.flush();
                                return;
                            }
                        }
                    }
                }
            }
        }
        if let Some(pointer) = &self.themed_pointer {
            let _ = pointer.set_cursor(conn, to_sctk_cursor(cursor));
        }
    }

    fn create_toplevel(
        &mut self,
        qh: &QueueHandle<Self>,
        config: &WindowConfig,
    ) -> Result<usize, PlatformError> {
        let surface = self.compositor_state.create_surface(qh);
        let surface_obj_id = surface.id();
        let surface_proto_id = surface_obj_id.protocol_id();
        let surface_ptr = surface_obj_id.as_ptr() as usize;

        let borderless = !config.decorations || config.role != WindowRole::Standard;
        let decorations = if borderless {
            WindowDecorations::ClientOnly
        } else {
            WindowDecorations::RequestServer
        };
        if config.floating_policy.click_through {
            if let Ok(region) =
                smithay_client_toolkit::compositor::Region::new(&self.compositor_state)
            {
                surface.set_input_region(Some(region.wl_region()));
            }
        }
        let sctk_window = self.xdg_shell.create_window(surface, decorations, qh);
        sctk_window.set_title(&config.title);
        sctk_window.set_app_id("unbound-ui");
        let (min_w, min_h) = config.effective_min_size();
        sctk_window.set_min_size(Some((min_w, min_h)));
        sctk_window.xdg_surface().set_window_geometry(
            0,
            0,
            config.width.max(1) as i32,
            config.height.max(1) as i32,
        );
        sctk_window.commit();

        let id = self.next_window_id;
        self.next_window_id += 1;

        let init_scale = crate::wayland_logic::primary_output_metrics()
            .map_or(1.0, |m| f64::from(m.scale_factor));
        let phys_w = ((f64::from(config.width.max(1))) * init_scale).round() as u32;
        let phys_h = ((f64::from(config.height.max(1))) * init_scale).round() as u32;

        let shared = Arc::new(WaylandWindowShared {
            id,
            display_ptr: self.display_ptr,
            surface_ptr,
            decorated: !borderless,
            popup_id: None,
            inner_size: Mutex::new(PhysicalSize::new(phys_w.max(1), phys_h.max(1))),
            scale_bits: AtomicU64::new(init_scale.to_bits()),
            maximized: AtomicBool::new(false),
            visible: AtomicBool::new(config.visible),
            destroyed: AtomicBool::new(false),
            redraw_requested: AtomicBool::new(true),
            cursor: AtomicU32::new(CursorIcon::Default.to_u32()),
            frame_metrics: Mutex::new(None),
            cmd_tx: Mutex::new(self.cmd_tx.clone()),
            wake_signal: self.wake_signal.clone(),
        });
        let handle = WaylandWindowHandle {
            shared: Arc::clone(&shared),
        };
        let native_window = NativeWindow::from_wayland(handle);

        let mut state = PlatformState::new(
            config,
            Extent::new(phys_w.max(1), phys_h.max(1)),
            PresentWaitPolicy::from_extensions(&[]),
        );
        state.scale_factor = init_scale;

        self.surface_to_window.insert(surface_proto_id, id);
        let a11y_wake = self.wake_signal.clone();
        let a11y = crate::a11y_platform::PlatformA11y::for_linux(
            id as u64,
            Some(Arc::new(move || a11y_wake.signal())),
        );
        self.windows.push(ToplevelEntry {
            id,
            sctk_window,
            native_window,
            shared,
            state,
            configured: false,
            resizable: config.resizable,
            min_width: min_w,
            min_height: min_h,
            throttle: WaylandFrameThrottle::default(),
            was_suspended: false,
            csd_edge_active: false,
            pending_events: Vec::new(),
            cursor_logical: (0.0, 0.0),
            a11y,
        });
        Ok(self.windows.len() - 1)
    }

    fn sync_popup(
        &mut self,
        qh: &QueueHandle<Self>,
        popup_id: u64,
        config: &PopupWindowConfig,
    ) -> bool {
        let owner_entry = self
            .windows
            .iter()
            .find(|w| w.id as u64 == config.owner.0)
            .or_else(|| self.windows.first());
        let Some(owner_entry) = owner_entry else {
            return false;
        };
        let owner_window_id = owner_entry.id;
        let parent_xdg = owner_entry.sctk_window.xdg_surface().clone();
        let owner_scale = owner_entry.native_window.scale_factor();

        let placed =
            compute_wayland_popup_placement(config.anchor_rect, config.screen_rect, owner_scale);

        let Ok(positioner) =
            smithay_client_toolkit::shell::xdg::XdgPositioner::new(&self.xdg_shell)
        else {
            return false;
        };
        positioner.set_size(placed.width, placed.height);
        positioner.set_anchor_rect(
            placed.anchor_x,
            placed.anchor_y,
            placed.anchor_width,
            placed.anchor_height,
        );
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_offset(placed.offset_x, placed.offset_y);
        let mut constraint = xdg_positioner::ConstraintAdjustment::empty();
        if placed.flip_x {
            constraint |= xdg_positioner::ConstraintAdjustment::FlipX;
        }
        if placed.flip_y {
            constraint |= xdg_positioner::ConstraintAdjustment::FlipY;
        }
        if placed.slide_x {
            constraint |= xdg_positioner::ConstraintAdjustment::SlideX;
        }
        if placed.slide_y {
            constraint |= xdg_positioner::ConstraintAdjustment::SlideY;
        }
        if placed.resize {
            constraint |= xdg_positioner::ConstraintAdjustment::ResizeX
                | xdg_positioner::ConstraintAdjustment::ResizeY;
        }
        positioner.set_constraint_adjustment(constraint);

        if let Some(existing) = self.popups.get_mut(&popup_id) {
            existing.popup.reposition(&positioner, popup_id as u32);
            if let Ok(mut sz) = existing.shared.inner_size.lock() {
                *sz = PhysicalSize::new(
                    config.screen_rect.width.max(1),
                    config.screen_rect.height.max(1),
                );
            }
            return false;
        }

        let surface = self.compositor_state.create_surface(qh);
        let surface_obj_id = surface.id();
        let surface_proto_id = surface_obj_id.protocol_id();
        let surface_ptr = surface_obj_id.as_ptr() as usize;

        // ToolTip 等鼠标穿透浮层：设置空 `wl_region` 输入区，严格保持鼠标穿透且不抢焦点。
        if config.click_through {
            if let Ok(region) =
                smithay_client_toolkit::compositor::Region::new(&self.compositor_state)
            {
                surface.set_input_region(Some(region.wl_region()));
            }
        }

        let Ok(popup) =
            Popup::from_surface(Some(&parent_xdg), &positioner, qh, surface, &self.xdg_shell)
        else {
            return false;
        };
        popup.wl_surface().commit();

        let win_id = self.next_window_id;
        self.next_window_id += 1;

        let shared = Arc::new(WaylandWindowShared {
            id: win_id,
            display_ptr: self.display_ptr,
            surface_ptr,
            decorated: false,
            popup_id: Some(popup_id),
            inner_size: Mutex::new(PhysicalSize::new(
                config.screen_rect.width.max(1),
                config.screen_rect.height.max(1),
            )),
            scale_bits: AtomicU64::new(owner_scale.to_bits()),
            maximized: AtomicBool::new(false),
            visible: AtomicBool::new(true),
            destroyed: AtomicBool::new(false),
            redraw_requested: AtomicBool::new(true),
            cursor: AtomicU32::new(CursorIcon::Default.to_u32()),
            frame_metrics: Mutex::new(None),
            cmd_tx: Mutex::new(self.cmd_tx.clone()),
            wake_signal: self.wake_signal.clone(),
        });
        let handle = WaylandWindowHandle {
            shared: Arc::clone(&shared),
        };
        let native_window = NativeWindow::from_wayland(handle);
        let popup_cfg = WindowConfig::default().with_size(
            config.screen_rect.width.max(1),
            config.screen_rect.height.max(1),
        );
        let mut pstate = PlatformState::new(
            &popup_cfg,
            Extent::new(
                config.screen_rect.width.max(1),
                config.screen_rect.height.max(1),
            ),
            PresentWaitPolicy::from_extensions(&[]),
        );
        pstate.scale_factor = owner_scale;

        self.surface_to_popup.insert(surface_proto_id, popup_id);
        self.popups.insert(
            popup_id,
            PopupEntry {
                owner_window_id,
                popup,
                native_window,
                shared,
                state: pstate,
                pending_events: Vec::new(),
                configured: false,
                dismissed: false,
            },
        );
        true
    }

    fn refresh_primary_output(&self) {
        if let Some(output) = self.output_state.outputs().next() {
            if let Some(info) = self.output_state.info(&output) {
                let (phys_w, phys_h, refresh_hz) = info
                    .modes
                    .iter()
                    .find(|m| m.current)
                    .or_else(|| info.modes.first())
                    .map_or((1920, 1080, None), |m| {
                        let hz = (m.refresh_rate > 0).then_some(m.refresh_rate as f32 / 1000.0);
                        (
                            m.dimensions.0.max(1) as u32,
                            m.dimensions.1.max(1) as u32,
                            hz,
                        )
                    });
                let scale = (info.scale_factor.max(1)) as f32;
                set_primary_output_metrics(PrimaryOutputMetrics {
                    physical_width: phys_w,
                    physical_height: phys_h,
                    scale_factor: scale,
                    refresh_rate_hz: refresh_hz,
                });
            }
        }
    }

    fn apply_command(&mut self, conn: &Connection, qh: &QueueHandle<Self>, cmd: WaylandCommand) {
        match cmd {
            WaylandCommand::SetTitle { id, title } => {
                if let Some(entry) = self.window_by_id_mut(id) {
                    entry.sctk_window.set_title(&title);
                }
            }
            WaylandCommand::SetMinimized { id, minimized } => {
                if let Some(entry) = self.window_by_id_mut(id) {
                    if minimized {
                        entry.sctk_window.set_minimized();
                        entry.throttle.set_hidden_or_minimized(true);
                    } else {
                        entry.throttle.set_hidden_or_minimized(false);
                    }
                }
            }
            WaylandCommand::SetMaximized { id, maximized } => {
                if let Some(entry) = self.window_by_id_mut(id) {
                    if maximized {
                        entry.sctk_window.set_maximized();
                    } else {
                        entry.sctk_window.unset_maximized();
                    }
                }
            }
            WaylandCommand::SetVisible { id, visible } => {
                if let Some(entry) = self.window_by_id_mut(id) {
                    entry.throttle.set_hidden_or_minimized(!visible);
                }
            }
            WaylandCommand::SetClickThrough { id, click_through } => {
                if click_through {
                    if let Ok(region) =
                        smithay_client_toolkit::compositor::Region::new(&self.compositor_state)
                    {
                        if let Some(entry) = self.window_by_id_mut(id) {
                            entry
                                .sctk_window
                                .wl_surface()
                                .set_input_region(Some(region.wl_region()));
                            entry.sctk_window.wl_surface().commit();
                        }
                    }
                } else if let Some(entry) = self.window_by_id_mut(id) {
                    entry.sctk_window.wl_surface().set_input_region(None);
                    entry.sctk_window.wl_surface().commit();
                }
            }
            WaylandCommand::SetCursor { cursor } => {
                self.apply_cursor(conn, qh, cursor);
            }
            WaylandCommand::DragWindow { id } => {
                let serial = self.last_pointer_serial;
                if let Some(seat) = self.seat.clone() {
                    if let Some(entry) = self.window_by_id_mut(id) {
                        entry.sctk_window.move_(&seat, serial);
                        let _ = conn.flush();
                    }
                }
            }
            WaylandCommand::DragResizeWindow { id, direction } => {
                let serial = self.last_pointer_serial;
                if let Some(seat) = self.seat.clone() {
                    if let Some(entry) = self.window_by_id_mut(id) {
                        entry
                            .sctk_window
                            .resize(&seat, serial, to_xdg_resize_edge(direction));
                        let _ = conn.flush();
                    }
                }
            }
            WaylandCommand::RequestClose { id } => {
                if let Some(entry) = self.window_by_id_mut(id) {
                    entry.pending_events.push(PlatformEvent::CloseRequested);
                }
            }
            WaylandCommand::RequestRedraw { id } => {
                if let Some(entry) = self.window_by_id_mut(id) {
                    entry.shared.redraw_requested.store(true, Ordering::Release);
                }
            }
            WaylandCommand::ClipboardWrite { text } => {
                self.clipboard_owned_text = text;
                if let (Some(ddm), Some(dd)) = (&self.data_device_manager, &self.data_device) {
                    let source =
                        ddm.create_copy_paste_source(qh, [WAYLAND_CLIPBOARD_MIME, "text/plain"]);
                    let serial = self.last_key_serial.max(self.last_pointer_serial);
                    source.set_selection(dd, serial);
                    self.clipboard_source = Some(source);
                }
            }
            WaylandCommand::ClipboardWriteFiles {
                uri_list,
                gnome_copied_files,
            } => {
                self.clipboard_owned_uri_list = uri_list;
                self.clipboard_owned_gnome_files = gnome_copied_files;
                if let (Some(ddm), Some(dd)) = (&self.data_device_manager, &self.data_device) {
                    let source = ddm.create_copy_paste_source(
                        qh,
                        [
                            crate::linux_dnd::URI_LIST_MIME,
                            crate::clipboard::GNOME_COPIED_FILES_MIME,
                        ],
                    );
                    let serial = self.last_key_serial.max(self.last_pointer_serial);
                    source.set_selection(dd, serial);
                    self.clipboard_source = Some(source);
                }
            }
            WaylandCommand::SetImeAllowed { id, allowed } => {
                let (action, ime_ev) = self.ime_state.set_allowed(allowed);
                self.apply_ime_protocol_action(action);
                if let Some(ev) = ime_ev {
                    if let Some(entry) = self.window_by_id_mut(id) {
                        entry.pending_events.push(PlatformEvent::Ime(ev));
                    }
                }
            }
            WaylandCommand::SetImeCursorArea { id, position, size } => {
                let Some(entry) = self.windows.iter().find(|w| w.id == id) else {
                    return;
                };
                let scale = entry.native_window.scale_factor();
                let win_size = entry.native_window.inner_size();
                let rect = compute_wayland_ime_cursor_rect(position, size, scale, win_size);
                let action = self.ime_state.set_cursor_area(rect);
                self.apply_ime_protocol_action(action);
            }
            WaylandCommand::ThemeChanged { dark } => {
                for entry in &mut self.windows {
                    entry
                        .pending_events
                        .push(PlatformEvent::ThemeChanged { dark });
                }
            }
            WaylandCommand::FilesDropped { surface_id, paths } => {
                if let Some(entry) = self.window_by_surface_mut(surface_id) {
                    for path in paths {
                        entry.pending_events.push(PlatformEvent::FileDrop(path));
                    }
                }
            }
            WaylandCommand::StartOutboundFileDrag {
                uri_list,
                actions,
                reply,
            } => {
                let origin_surface = self
                    .pointer_surface_id
                    .or(self.focused_surface_id)
                    .and_then(|sid| {
                        self.surface_to_window
                            .get(&sid)
                            .and_then(|&wid| self.windows.iter().find(|w| w.id == wid))
                    })
                    .or_else(|| self.windows.first())
                    .map(|w| w.sctk_window.wl_surface().clone());
                if let (Some(ddm), Some(dd), Some(origin)) =
                    (&self.data_device_manager, &self.data_device, origin_surface)
                {
                    let source = ddm.create_drag_and_drop_source(
                        qh,
                        [crate::linux_dnd::URI_LIST_MIME],
                        actions,
                    );
                    self.dnd_outbound_uri_list = uri_list;
                    self.dnd_outbound_reply = Some(reply);
                    self.dnd_outbound_action = DndAction::empty();
                    source.start_drag(dd, &origin, None, self.last_pointer_serial);
                    self.dnd_source = Some(source);
                } else {
                    let _ = reply.send(crate::ole_drag::OutboundFileDragOutcome::Cancelled);
                }
            }
        }
    }

    fn apply_ime_protocol_action(&self, action: WaylandImeProtocolAction) {
        let Some(ti) = &self.text_input else {
            return;
        };
        match action {
            WaylandImeProtocolAction::None => {}
            WaylandImeProtocolAction::Enable { cursor_rect } => {
                ti.enable();
                if let Some(r) = cursor_rect {
                    ti.set_cursor_rectangle(r.x, r.y, r.width, r.height);
                }
                ti.commit();
            }
            WaylandImeProtocolAction::UpdateCursorRect(r) => {
                ti.set_cursor_rectangle(r.x, r.y, r.width, r.height);
                ti.commit();
            }
            WaylandImeProtocolAction::Disable => {
                ti.disable();
                ti.commit();
            }
        }
    }
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let factor = f64::from(new_factor.max(1));
        surface.set_buffer_scale(new_factor.max(1));
        let proto_id = surface.id().protocol_id();
        if let Some(entry) = self.window_by_surface_mut(proto_id) {
            let old = entry.native_window.scale_factor();
            if (old - factor).abs() > 1e-4 {
                entry
                    .shared
                    .scale_bits
                    .store(factor.to_bits(), Ordering::Release);
                let old_size = entry.native_window.inner_size();
                let logical_w = (f64::from(old_size.width) / old).round().max(1.0);
                let logical_h = (f64::from(old_size.height) / old).round().max(1.0);
                let new_w = (logical_w * factor).round().max(1.0) as u32;
                let new_h = (logical_h * factor).round().max(1.0) as u32;
                if let Ok(mut sz) = entry.shared.inner_size.lock() {
                    *sz = PhysicalSize::new(new_w, new_h);
                }
                entry
                    .pending_events
                    .push(PlatformEvent::ScaleFactorChanged(factor));
                push_coalesced_event(
                    &mut entry.pending_events,
                    PlatformEvent::Resized(Extent::new(new_w, new_h)),
                );
            }
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        let proto_id = surface.id().protocol_id();
        if let Some(entry) = self.window_by_surface_mut(proto_id) {
            entry.throttle.set_hidden_or_minimized(false);
            entry.throttle.on_frame_done();
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.refresh_primary_output();
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.refresh_primary_output();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.refresh_primary_output();
    }
}

impl WindowHandler for WaylandState {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, window: &SctkWindow) {
        let proto_id = window.wl_surface().id().protocol_id();
        if let Some(entry) = self.window_by_surface_mut(proto_id) {
            entry.pending_events.push(PlatformEvent::CloseRequested);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &SctkWindow,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let proto_id = window.wl_surface().id().protocol_id();
        if configure.is_resizing() {
            self.active_resize = None;
        }
        if let Some(entry) = self.window_by_surface_mut(proto_id) {
            let scale = entry.native_window.scale_factor();
            let maximized = configure.is_maximized();
            entry.shared.maximized.store(maximized, Ordering::Release);
            if configure.is_activated() {
                entry.throttle.set_hidden_or_minimized(false);
            }

            let (cur_w, cur_h) = {
                let sz = entry.native_window.inner_size();
                (sz.width, sz.height)
            };
            let new_w = configure.new_size.0.map_or(cur_w, |w| {
                (f64::from(w.get()) * scale).round().max(1.0) as u32
            });
            let new_h = configure.new_size.1.map_or(cur_h, |h| {
                (f64::from(h.get()) * scale).round().max(1.0) as u32
            });

            if new_w != cur_w || new_h != cur_h || !entry.configured {
                let logical_w = (f64::from(new_w) / scale).round().max(1.0) as i32;
                let logical_h = (f64::from(new_h) / scale).round().max(1.0) as i32;
                window
                    .xdg_surface()
                    .set_window_geometry(0, 0, logical_w, logical_h);
                if let Ok(mut sz) = entry.shared.inner_size.lock() {
                    *sz = PhysicalSize::new(new_w, new_h);
                }
                push_coalesced_event(
                    &mut entry.pending_events,
                    PlatformEvent::Resized(Extent::new(new_w, new_h)),
                );
            }
            entry.configured = true;
            entry.shared.redraw_requested.store(true, Ordering::Release);
        }
    }
}

impl PopupHandler for WaylandState {
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        popup: &Popup,
        config: PopupConfigure,
    ) {
        let proto_id = popup.wl_surface().id().protocol_id();
        if let Some(&popup_id) = self.surface_to_popup.get(&proto_id) {
            if let Some(entry) = self.popups.get_mut(&popup_id) {
                let scale = entry.native_window.scale_factor();
                let phys_w = (f64::from(config.width.max(1)) * scale).round().max(1.0) as u32;
                let phys_h = (f64::from(config.height.max(1)) * scale).round().max(1.0) as u32;
                if let Ok(mut sz) = entry.shared.inner_size.lock() {
                    *sz = PhysicalSize::new(phys_w, phys_h);
                }
                push_coalesced_event(
                    &mut entry.pending_events,
                    PlatformEvent::Resized(Extent::new(phys_w, phys_h)),
                );
                entry.configured = true;
            }
        }
    }

    fn done(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, popup: &Popup) {
        let proto_id = popup.wl_surface().id().protocol_id();
        if let Some(&popup_id) = self.surface_to_popup.get(&proto_id) {
            if let Some(entry) = self.popups.get_mut(&popup_id) {
                entry.dismissed = true;
            }
        }
    }
}

impl WaylandState {
    /// 确保主 `wl_seat` 及其依赖的 `wl_data_device` / `zwp_text_input_v3` 已完成绑定。
    ///
    /// `sctk` 的 `SeatState::new` 会在首轮 `roundtrip` 前同步绑定已有座席，因此不会对初始座席
    /// 回调 `SeatHandler::new_seat`；必须在 `new_capability` 与首轮 `roundtrip` 后兜底绑定，
    /// 否则 `self.seat` 恒为 `None`，导致拖拽移动、边缘缩放、剪贴板、DnD 与 IME 全部失效。
    fn ensure_seat_bound(&mut self, qh: &QueueHandle<Self>, seat: &wl_seat::WlSeat) {
        if self.seat.is_none() {
            self.seat = Some(seat.clone());
            if let Some(ddm) = &self.data_device_manager {
                self.data_device = Some(ddm.get_data_device(qh, seat));
            }
            if let Some(mgr) = &self.text_input_manager {
                self.text_input = Some(mgr.get_text_input(seat, qh, ()));
            }
        }
    }
}

impl SeatHandler for WaylandState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.ensure_seat_bound(qh, &seat);
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        self.ensure_seat_bound(qh, &seat);
        if capability == Capability::Pointer && self.themed_pointer.is_none() {
            let surface = self.compositor_state.create_surface(qh);
            if let Ok(pointer) = self.seat_state.get_pointer_with_theme(
                qh,
                &seat,
                self.shm.wl_shm(),
                surface,
                ThemeSpec::default(),
            ) {
                self.themed_pointer = Some(pointer);
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.themed_pointer = None;
            self.last_pointer_enter_serial = None;
            self.custom_cursor_buffer = None;
            self.custom_cursor_pool = None;
            self.custom_cursor_surface = None;
        }
        if capability == Capability::Keyboard {
            if let Some(kb) = self.keyboard.take() {
                kb.release();
            }
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl PointerHandler for WaylandState {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let proto_id = event.surface.id().protocol_id();
            let (lx, ly) = event.position;

            // 先检查是否命中弹出窗表面
            if let Some(&popup_id) = self.surface_to_popup.get(&proto_id) {
                if let Some(pentry) = self.popups.get_mut(&popup_id) {
                    let scale = pentry.native_window.scale_factor();
                    match event.kind {
                        PointerEventKind::Enter { serial } => {
                            self.pointer_surface_id = Some(proto_id);
                            self.last_pointer_serial = serial;
                            self.last_pointer_enter_serial = Some(serial);
                            push_coalesced_event(
                                &mut pentry.pending_events,
                                PlatformEvent::CursorMoved {
                                    x: lx * scale,
                                    y: ly * scale,
                                },
                            );
                        }
                        PointerEventKind::Leave { .. } => {}
                        PointerEventKind::Motion { .. } => {
                            push_coalesced_event(
                                &mut pentry.pending_events,
                                PlatformEvent::CursorMoved {
                                    x: lx * scale,
                                    y: ly * scale,
                                },
                            );
                        }
                        PointerEventKind::Press { button, serial, .. } => {
                            self.last_pointer_serial = serial;
                            pentry.pending_events.push(PlatformEvent::MouseInput {
                                button: translate_linux_button(button),
                                state: ButtonState::Pressed,
                            });
                        }
                        PointerEventKind::Release { button, serial, .. } => {
                            self.last_pointer_serial = serial;
                            pentry.pending_events.push(PlatformEvent::MouseInput {
                                button: translate_linux_button(button),
                                state: ButtonState::Released,
                            });
                        }
                        PointerEventKind::Axis {
                            horizontal,
                            vertical,
                            ..
                        } => {
                            let delta_x = -(horizontal.absolute / 20.0);
                            let delta_y = -(vertical.absolute / 20.0);
                            if delta_x != 0.0 || delta_y != 0.0 {
                                pentry
                                    .pending_events
                                    .push(PlatformEvent::MouseWheel { delta_x, delta_y });
                            }
                        }
                    }
                }
                continue;
            }

            let Some(win_id) = self.surface_to_window.get(&proto_id).copied() else {
                continue;
            };
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.pointer_surface_id = Some(proto_id);
                    self.last_pointer_serial = serial;
                    self.last_pointer_enter_serial = Some(serial);
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_surface_id == Some(proto_id) {
                        self.pointer_surface_id = None;
                    }
                }
                PointerEventKind::Press { serial, .. }
                | PointerEventKind::Release { serial, .. } => {
                    self.last_pointer_serial = serial;
                }
                _ => {}
            }
            let seat_clone = self.seat.clone();
            let mut pending_cursor: Option<CursorIcon> = None;
            let mut new_active_resize: Option<WaylandActiveResize> = None;
            let mut clear_active_resize = false;
            let active_resize_snap = self
                .active_resize
                .as_ref()
                .filter(|r| r.window_id == win_id)
                .map(|r| {
                    (
                        r.dir, r.start_lx, r.start_ly, r.start_w, r.start_h, r.min_w, r.min_h,
                    )
                });
            {
                let Some(entry) = self.window_by_id_mut(win_id) else {
                    continue;
                };
                let scale = entry.native_window.scale_factor();
                let size = entry.native_window.inner_size();
                let logical_w = f64::from(size.width) / scale;
                let logical_h = f64::from(size.height) / scale;
                let is_csd = !entry.native_window.is_decorated();
                let can_resize = is_csd && entry.resizable && !entry.native_window.is_maximized();

                match event.kind {
                    PointerEventKind::Enter { .. } => {
                        entry.cursor_logical = (lx, ly);
                        push_coalesced_event(
                            &mut entry.pending_events,
                            PlatformEvent::CursorMoved {
                                x: lx * scale,
                                y: ly * scale,
                            },
                        );
                    }
                    PointerEventKind::Leave { .. } => {}
                    PointerEventKind::Motion { .. } => {
                        entry.cursor_logical = (lx, ly);
                        if let Some((dir, start_lx, start_ly, start_w, start_h, min_w, min_h)) =
                            active_resize_snap
                        {
                            let dx = (lx - start_lx).round() as i32;
                            let dy = (ly - start_ly).round() as i32;
                            let target = crate::linux_backend::compute_interactive_resize_rect(
                                ScreenRect::new(0, 0, start_w, start_h),
                                dir,
                                dx,
                                dy,
                                min_w,
                                min_h,
                            );
                            let phys_w = (f64::from(target.width) * scale).round().max(1.0) as u32;
                            let phys_h = (f64::from(target.height) * scale).round().max(1.0) as u32;
                            if phys_w != size.width || phys_h != size.height {
                                entry.sctk_window.xdg_surface().set_window_geometry(
                                    0,
                                    0,
                                    target.width as i32,
                                    target.height as i32,
                                );
                                if let Ok(mut sz) = entry.shared.inner_size.lock() {
                                    *sz = PhysicalSize::new(phys_w, phys_h);
                                }
                                push_coalesced_event(
                                    &mut entry.pending_events,
                                    PlatformEvent::Resized(Extent::new(phys_w, phys_h)),
                                );
                                entry.shared.redraw_requested.store(true, Ordering::Release);
                            }
                            continue;
                        }
                        if can_resize {
                            if let Some(dir) = csd_resize_direction_at(
                                lx,
                                ly,
                                logical_w,
                                logical_h,
                                WAYLAND_CSD_RESIZE_BORDER,
                            ) {
                                entry.csd_edge_active = true;
                                pending_cursor = Some(cursor_for_resize_direction(dir));
                            } else if entry.csd_edge_active {
                                entry.csd_edge_active = false;
                                pending_cursor = Some(entry.native_window.current_cursor());
                            }
                        }
                        push_coalesced_event(
                            &mut entry.pending_events,
                            PlatformEvent::CursorMoved {
                                x: lx * scale,
                                y: ly * scale,
                            },
                        );
                    }
                    PointerEventKind::Press { button, serial, .. } => {
                        let mb = translate_linux_button(button);
                        if mb == MouseButton::Left && can_resize {
                            if let Some(dir) = csd_resize_direction_at(
                                lx,
                                ly,
                                logical_w,
                                logical_h,
                                WAYLAND_CSD_RESIZE_BORDER,
                            ) {
                                if let Some(seat) = seat_clone {
                                    entry.sctk_window.resize(
                                        &seat,
                                        serial,
                                        to_xdg_resize_edge(dir),
                                    );
                                    let _ = conn.flush();
                                    new_active_resize = Some(WaylandActiveResize {
                                        window_id: win_id,
                                        dir,
                                        start_lx: lx,
                                        start_ly: ly,
                                        start_w: logical_w.round().max(1.0) as u32,
                                        start_h: logical_h.round().max(1.0) as u32,
                                        min_w: entry.min_width,
                                        min_h: entry.min_height,
                                    });
                                }
                            }
                        }
                        if new_active_resize.is_none() {
                            entry.pending_events.push(PlatformEvent::MouseInput {
                                button: mb,
                                state: ButtonState::Pressed,
                            });
                        }
                    }
                    PointerEventKind::Release { button, .. } => {
                        if translate_linux_button(button) == MouseButton::Left
                            && active_resize_snap.is_some()
                        {
                            clear_active_resize = true;
                        }
                        entry.pending_events.push(PlatformEvent::MouseInput {
                            button: translate_linux_button(button),
                            state: ButtonState::Released,
                        });
                    }
                    PointerEventKind::Axis {
                        horizontal,
                        vertical,
                        ..
                    } => {
                        let delta_x = -(horizontal.absolute / 20.0);
                        let delta_y = -(vertical.absolute / 20.0);
                        if delta_x != 0.0 || delta_y != 0.0 {
                            entry
                                .pending_events
                                .push(PlatformEvent::MouseWheel { delta_x, delta_y });
                        }
                    }
                }
            }
            if let Some(ar) = new_active_resize {
                self.active_resize = Some(ar);
            } else if clear_active_resize {
                self.active_resize = None;
            }
            if let Some(edge_cursor) = pending_cursor {
                self.apply_cursor(conn, qh, edge_cursor);
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                let proto_id = surface.id().protocol_id();
                state.focused_surface_id = Some(proto_id);
                if let Some(entry) = state.window_by_surface_mut(proto_id) {
                    entry.a11y.set_focus(true);
                    entry.pending_events.push(PlatformEvent::Focused(true));
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                let proto_id = surface.id().protocol_id();
                if state.focused_surface_id == Some(proto_id) {
                    state.focused_surface_id = None;
                }
                state.key_repeat.clear();
                let (ime_action, ime_cancel) = state.ime_state.on_focus_lost();
                state.apply_ime_protocol_action(ime_action);
                if let Some(entry) = state.window_by_surface_mut(proto_id) {
                    if let Some(ev) = ime_cancel {
                        entry.pending_events.push(PlatformEvent::Ime(ev));
                    }
                    entry.a11y.set_focus(false);
                    entry.pending_events.push(PlatformEvent::Focused(false));
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                ..
            } => {
                let effective = mods_depressed | mods_latched;
                state.modifiers = Modifiers {
                    shift: (effective & 0x01) != 0,
                    control: (effective & 0x04) != 0,
                    alt: (effective & 0x08) != 0,
                    meta: (effective & 0x40) != 0,
                };
                let _ = mods_locked;
                if let Some(proto_id) = state.focused_surface_id {
                    let mods = state.modifiers;
                    if let Some(entry) = state.window_by_surface_mut(proto_id) {
                        entry
                            .pending_events
                            .push(PlatformEvent::ModifiersChanged(mods));
                    }
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.key_repeat.set_repeat_info(rate, delay);
            }
            wl_keyboard::Event::Key {
                serial,
                key,
                state: key_state,
                ..
            } => {
                state.last_key_serial = serial;
                let Some(proto_id) = state.focused_surface_id else {
                    return;
                };
                let modifiers = state.modifiers;
                let (platform_key, text) = translate_evdev_key(key, modifiers);
                let pressed = matches!(
                    key_state,
                    wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                );
                if pressed {
                    state.key_repeat.on_key_down(
                        key,
                        platform_key.clone(),
                        text.clone(),
                        Instant::now(),
                    );
                } else {
                    state.key_repeat.on_key_up(key);
                }
                if let Some(entry) = state.window_by_surface_mut(proto_id) {
                    entry.pending_events.push(PlatformEvent::KeyInput {
                        key: platform_key,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        repeat: false,
                    });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        _event: zwp_text_input_manager_v3::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _proxy: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { surface } => {
                let proto_id = surface.id().protocol_id();
                let action = state.ime_state.on_enter(proto_id);
                state.apply_ime_protocol_action(action);
            }
            zwp_text_input_v3::Event::Leave { surface } => {
                let proto_id = surface.id().protocol_id();
                let (action, cancel_ev) = state.ime_state.on_leave(proto_id);
                state.apply_ime_protocol_action(action);
                if let Some(ev) = cancel_ev {
                    if let Some(entry) = state.window_by_surface_mut(proto_id) {
                        entry.pending_events.push(PlatformEvent::Ime(ev));
                    }
                }
            }
            zwp_text_input_v3::Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                state.ime_state.on_preedit(text, cursor_begin, cursor_end);
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                state.ime_state.on_commit(text);
            }
            zwp_text_input_v3::Event::DeleteSurroundingText { .. } => {}
            zwp_text_input_v3::Event::Done { .. } => {
                let events = state.ime_state.on_done();
                if events.is_empty() {
                    return;
                }
                let target_surface = state
                    .ime_state
                    .focused_surface()
                    .or(state.focused_surface_id);
                if let Some(proto_id) = target_surface {
                    if let Some(entry) = state.window_by_surface_mut(proto_id) {
                        for ev in events {
                            entry.pending_events.push(PlatformEvent::Ime(ev));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl DataDeviceHandler for WaylandState {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
        _x: f64,
        _y: f64,
        wl_surface: &wl_surface::WlSurface,
    ) {
        let proto_id = wl_surface.id().protocol_id();
        self.dnd_enter_surface_id = Some(proto_id);
        self.dnd_offer_has_uri_list = false;
        if let Some(dd) = &self.data_device {
            if let Some(offer) = dd.data().drag_offer() {
                let has_uri = offer.with_mime_types(|mimes| {
                    mimes.iter().any(|m| m == crate::linux_dnd::URI_LIST_MIME)
                });
                if has_uri {
                    self.dnd_offer_has_uri_list = true;
                    offer.accept_mime_type(
                        offer.serial,
                        Some(crate::linux_dnd::URI_LIST_MIME.to_owned()),
                    );
                    offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Copy);
                    if let Some(entry) = self.window_by_surface_mut(proto_id) {
                        entry
                            .pending_events
                            .push(PlatformEvent::FileHover(std::path::PathBuf::new()));
                    }
                }
            }
        }
    }
    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
    ) {
        if let Some(proto_id) = self.dnd_enter_surface_id.take() {
            if self.dnd_offer_has_uri_list {
                if let Some(entry) = self.window_by_surface_mut(proto_id) {
                    entry.pending_events.push(PlatformEvent::FileHoverCancelled);
                }
            }
        }
        self.dnd_offer_has_uri_list = false;
    }
    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
    }
    fn selection(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
    ) {
        let Some(dd) = &self.data_device else {
            return;
        };
        let Some(offer) = dd.data().selection_offer() else {
            if let Ok(mut guard) = self.clipboard_cache.lock() {
                *guard = None;
            }
            return;
        };
        let mime = offer.with_mime_types(|mimes| {
            if mimes.iter().any(|m| m == WAYLAND_CLIPBOARD_MIME) {
                Some(WAYLAND_CLIPBOARD_MIME.to_owned())
            } else if mimes.iter().any(|m| m == "text/plain") {
                Some("text/plain".to_owned())
            } else {
                None
            }
        });
        let Some(mime) = mime else {
            return;
        };
        if let Ok(mut pipe) = offer.receive(mime) {
            let _ = conn.flush();
            let cache = Arc::clone(&self.clipboard_cache);
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if pipe.read_to_end(&mut bytes).is_ok() {
                    if let Ok(text) = String::from_utf8(bytes) {
                        if let Ok(mut guard) = cache.lock() {
                            *guard = Some(text);
                        }
                    }
                }
            });
        }
    }
    fn drop_performed(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
    ) {
        let Some(surface_id) = self.dnd_enter_surface_id.take() else {
            return;
        };
        let has_uri = self.dnd_offer_has_uri_list;
        self.dnd_offer_has_uri_list = false;
        if !has_uri {
            return;
        }
        if let Some(dd) = &self.data_device {
            if let Some(offer) = dd.data().drag_offer() {
                if let Ok(mut pipe) = offer.receive(crate::linux_dnd::URI_LIST_MIME.to_owned()) {
                    let _ = conn.flush();
                    let tx = self.cmd_tx.clone();
                    let wake = self.wake_signal.clone();
                    std::thread::spawn(move || {
                        let mut bytes = Vec::new();
                        if pipe.read_to_end(&mut bytes).is_ok() {
                            if let Ok(raw) = String::from_utf8(bytes) {
                                let paths = crate::linux_dnd::decode_uri_list(&raw);
                                if !paths.is_empty()
                                    && tx
                                        .send(WaylandCommand::FilesDropped { surface_id, paths })
                                        .is_ok()
                                {
                                    wake.signal();
                                }
                            }
                        }
                    });
                }
                offer.finish();
            }
        }
    }
}

impl DataOfferHandler for WaylandState {
    fn source_actions(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: DndAction,
    ) {
    }
}

impl DataSourceHandler for WaylandState {
    fn accept_mime(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
        _mime: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        mut fd: WritePipe,
    ) {
        let is_dnd = self
            .dnd_source
            .as_ref()
            .is_some_and(|s| s.inner() == source);
        let payload = if mime == crate::clipboard::GNOME_COPIED_FILES_MIME {
            self.clipboard_owned_gnome_files.as_bytes().to_vec()
        } else if mime == crate::linux_dnd::URI_LIST_MIME {
            if is_dnd {
                self.dnd_outbound_uri_list.as_bytes().to_vec()
            } else {
                self.clipboard_owned_uri_list.as_bytes().to_vec()
            }
        } else {
            self.clipboard_owned_text.as_bytes().to_vec()
        };
        std::thread::spawn(move || {
            let _ = fd.write_all(&payload);
        });
    }

    fn cancelled(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, source: &WlDataSource) {
        if self
            .dnd_source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            self.dnd_source = None;
            if let Some(reply) = self.dnd_outbound_reply.take() {
                let _ = reply.send(crate::ole_drag::OutboundFileDragOutcome::Cancelled);
            }
        } else {
            self.clipboard_source = None;
        }
    }

    fn dnd_dropped(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _source: &WlDataSource) {
    }

    fn dnd_finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
    ) {
        self.dnd_source = None;
        if let Some(reply) = self.dnd_outbound_reply.take() {
            let effect = if self.dnd_outbound_action.contains(DndAction::Move) {
                crate::overlay_host::FileDragEffect::Move
            } else {
                crate::overlay_host::FileDragEffect::Copy
            };
            let _ = reply.send(crate::ole_drag::OutboundFileDragOutcome::Completed(effect));
        }
    }

    fn action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
        action: DndAction,
    ) {
        self.dnd_outbound_action = action;
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(WaylandState);
delegate_output!(WaylandState);
delegate_shm!(WaylandState);
delegate_seat!(WaylandState);
delegate_pointer!(WaylandState);
delegate_xdg_shell!(WaylandState);
delegate_xdg_window!(WaylandState);
delegate_xdg_popup!(WaylandState);
delegate_data_device!(WaylandState);
delegate_registry!(WaylandState);

/// 使用高分辨率 `ppoll` 阻塞等待 Wayland 套接字事件、应用唤醒管道或精确单调帧截止时间。
fn wait_wayland_or_deadline(wayland_fd: RawFd, wake_read_fd: RawFd, wait: Option<Duration>) {
    let mut pollfds = [
        libc::pollfd {
            fd: wayland_fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake_read_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let ts = wait.map(|d| libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: d.subsec_nanos() as libc::c_long,
    });
    let ts_ptr = ts
        .as_ref()
        .map_or(core::ptr::null(), |t| t as *const libc::timespec);
    // SAFETY: `pollfds` 与 `ts_ptr` 指向当前栈帧有效内存，仅执行非破坏性等待。
    unsafe {
        let _ = libc::ppoll(
            pollfds.as_mut_ptr(),
            pollfds.len() as libc::nfds_t,
            ts_ptr,
            core::ptr::null(),
        );
    }
}

/// 运行原生 Wayland 事件循环直至全部顶层窗口关闭或应用返回 [`Flow::Exit`]。
///
/// # Errors
/// 无法连接 Wayland 合成器（如未设置 `WAYLAND_DISPLAY`）或必需协议全局对象缺失时返回 [`PlatformError`]。
pub(crate) fn run_event_loop<A: PlatformApp>(
    config: WindowConfig,
    mut app: A,
) -> Result<(), PlatformError> {
    let conn = Connection::connect_to_env().map_err(|e| {
        PlatformError::EventLoop(format!("failed to connect to Wayland display: {e}"))
    })?;
    let display_ptr = conn.display().id().as_ptr() as usize;
    let (globals, mut event_queue): (_, EventQueue<WaylandState>) = registry_queue_init(&conn)
        .map_err(|e| {
            PlatformError::EventLoop(format!("failed to initialize Wayland registry: {e}"))
        })?;
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let compositor_state = CompositorState::bind(&globals, &qh)
        .map_err(|e| PlatformError::EventLoop(format!("wl_compositor unavailable: {e}")))?;
    let xdg_shell = XdgShell::bind(&globals, &qh)
        .map_err(|e| PlatformError::EventLoop(format!("xdg_wm_base unavailable: {e}")))?;
    let shm = Shm::bind(&globals, &qh)
        .map_err(|e| PlatformError::EventLoop(format!("wl_shm unavailable: {e}")))?;
    let output_state = OutputState::new(&globals, &qh);
    let seat_state = SeatState::new(&globals, &qh);
    let data_device_manager = DataDeviceManagerState::bind(&globals, &qh).ok();
    let text_input_manager = globals
        .bind::<zwp_text_input_manager_v3::ZwpTextInputManagerV3, _, _>(&qh, 1..=1, ())
        .ok();

    let wake_pipe = WakePipe::new()?;
    let wake_signal = wake_pipe.signal_handle();
    let (cmd_tx, cmd_rx) = mpsc::channel::<WaylandCommand>();
    let clipboard_cache = Arc::new(Mutex::new(None));
    let clipboard_files_cache = Arc::new(Mutex::new(Vec::new()));
    let clipboard_file_effect_cache = Arc::new(Mutex::new(None));

    // 注册全局 Wayland 剪贴板后端，使 `SystemClipboard` 在 Linux/WSLg 下透明读写。
    let _clipboard_guard = ClipboardRegistrationGuard;
    crate::linux_backend::set_active_linux_backend(Some(
        crate::linux_backend::LinuxBackendKind::Wayland,
    ));
    register_wayland_clipboard(Some(Arc::new(WaylandClipboardBridge {
        cached_text: Arc::clone(&clipboard_cache),
        cached_files: clipboard_files_cache,
        cached_file_effect: clipboard_file_effect_cache,
        cmd_tx: Mutex::new(cmd_tx.clone()),
        wake_signal: wake_signal.clone(),
    })));
    crate::linux_dnd::register_linux_outbound_drag(Some(Arc::new(WaylandOutboundDragBridge {
        event_loop_thread: std::thread::current().id(),
        cmd_tx: Mutex::new(cmd_tx.clone()),
        wake_signal: wake_signal.clone(),
    })));

    let mut wl = WaylandState {
        registry_state,
        seat_state,
        output_state,
        compositor_state,
        shm,
        xdg_shell,
        data_device_manager,
        data_device: None,
        clipboard_source: None,
        clipboard_owned_text: String::new(),
        clipboard_owned_uri_list: String::new(),
        clipboard_owned_gnome_files: String::new(),
        clipboard_cache,
        dnd_source: None,
        dnd_outbound_uri_list: String::new(),
        dnd_outbound_reply: None,
        dnd_outbound_action: DndAction::empty(),
        dnd_enter_surface_id: None,
        dnd_offer_has_uri_list: false,
        seat: None,
        themed_pointer: None,
        keyboard: None,
        text_input_manager,
        text_input: None,
        ime_state: WaylandImeState::default(),
        modifiers: Modifiers::default(),
        key_repeat: KeyRepeatState::default(),
        focused_surface_id: None,
        pointer_surface_id: None,
        last_pointer_serial: 0,
        last_pointer_enter_serial: None,
        custom_cursor_surface: None,
        custom_cursor_pool: None,
        custom_cursor_buffer: None,
        last_key_serial: 0,
        next_window_id: 1,
        windows: Vec::new(),
        popups: HashMap::new(),
        surface_to_window: HashMap::new(),
        surface_to_popup: HashMap::new(),
        cmd_tx,
        wake_signal: wake_signal.clone(),
        display_ptr,
        active_resize: None,
        exit_requested: false,
    };

    // 首轮往返以采集 `wl_output`、Seat 与初始全局对象。
    let _ = event_queue.roundtrip(&mut wl);
    if let Some(initial_seat) = wl.seat_state.seats().next() {
        wl.ensure_seat_bound(&qh, &initial_seat);
    }
    wl.refresh_primary_output();

    // 安装跨线程事件循环唤醒回调。
    let waker_sig = wake_signal.clone();
    app.attach_waker(Arc::new(move || waker_sig.signal()));

    // 订阅桌面门户深色模式变更，异步通过命令通道与唤醒管道通知事件循环
    let theme_tx = Mutex::new(wl.cmd_tx.clone());
    let theme_wake = wake_signal.clone();
    crate::linux_dbus::register_theme_change_listener(Arc::new(move |dark| {
        if let Ok(tx) = theme_tx.lock() {
            if tx.send(WaylandCommand::ThemeChanged { dark }).is_ok() {
                theme_wake.signal();
            }
        }
    }));

    // 创建主顶层窗口并等待首次 configure。
    wl.create_toplevel(&qh, &config)?;
    let _ = event_queue.roundtrip(&mut wl);

    if let Some(primary) = wl.windows.first_mut() {
        app.resumed(&mut primary.state, &primary.native_window);
    }

    while !wl.exit_requested && !wl.windows.is_empty() {
        let _ = event_queue.dispatch_pending(&mut wl);
        wake_pipe.drain();

        // 1. 消费应用侧通过 `NativeWindow` 发来的异步控制命令。
        while let Ok(cmd) = cmd_rx.try_recv() {
            wl.apply_command(&conn, &qh, cmd);
        }

        // 2. 处理软件按键重复到期。
        let now = Instant::now();
        if let Some((rep_key, _rep_text)) = wl.key_repeat.poll_repeat(now) {
            if let Some(proto_id) = wl.focused_surface_id {
                if let Some(entry) = wl.window_by_surface_mut(proto_id) {
                    entry.pending_events.push(PlatformEvent::KeyInput {
                        key: rep_key,
                        state: ButtonState::Pressed,
                        repeat: true,
                    });
                }
            }
        }

        // 3. 派发所有顶层窗口的待处理事件；关闭请求统一交由应用层拦截或通过 `take_window_close_requests` 放行。
        for idx in 0..wl.windows.len() {
            let events: Vec<PlatformEvent> = std::mem::take(&mut wl.windows[idx].pending_events);
            let ev_now = Instant::now();
            for ev in events {
                let derived = apply(&mut wl.windows[idx].state, &ev, ev_now);
                let entry = &mut wl.windows[idx];
                for out_ev in derived {
                    if app.event(&mut entry.state, &entry.native_window, out_ev) == Flow::Exit {
                        wl.exit_requested = true;
                        break;
                    }
                }
                if wl.exit_requested {
                    break;
                }
            }
            if wl.exit_requested {
                break;
            }
        }
        if wl.exit_requested {
            break;
        }

        // 4. 派发所有越界弹出窗事件，并回收合成器已 `done` 关闭的弹出窗。
        let popup_ids: Vec<u64> = wl.popups.keys().copied().collect();
        let mut dismissed_popups: Vec<(u64, isize)> = Vec::new();
        for pid in popup_ids {
            let Some(pentry) = wl.popups.get_mut(&pid) else {
                continue;
            };
            if pentry.dismissed {
                dismissed_popups.push((pid, pentry.owner_window_id));
                continue;
            }
            let events: Vec<PlatformEvent> = std::mem::take(&mut pentry.pending_events);
            let ev_now = Instant::now();
            for ev in events {
                let derived = apply(&mut pentry.state, &ev, ev_now);
                for out_ev in derived {
                    if app.event(&mut pentry.state, &pentry.native_window, out_ev) == Flow::Exit {
                        wl.exit_requested = true;
                        break;
                    }
                }
                if wl.exit_requested {
                    break;
                }
            }
        }
        if wl.exit_requested {
            break;
        }
        for (pid, owner_id) in dismissed_popups {
            if let Some(pentry) = wl.popups.remove(&pid) {
                pentry.shared.destroyed.store(true, Ordering::Release);
                let proto_id = pentry.popup.wl_surface().id().protocol_id();
                wl.surface_to_popup.remove(&proto_id);
                app.window_closing(&pentry.native_window);
            }
            if let Some(owner) = wl.window_by_id_mut(owner_id) {
                owner
                    .pending_events
                    .push(PlatformEvent::PopupDismissed { id: pid });
                wl.wake_signal.signal();
            }
        }

        // 5. 处理应用请求的新副窗（`Second Window`）与关闭请求。
        while let Some(new_cfg) = app.take_window_request() {
            if let Ok(idx) = wl.create_toplevel(&qh, &new_cfg) {
                let _ = event_queue.roundtrip(&mut wl);
                if let Some(entry) = wl.windows.get_mut(idx) {
                    app.window_created(&mut entry.state, &entry.native_window);
                }
            }
        }
        for close_id in app.take_window_close_requests() {
            if let Some(pos) = wl.windows.iter().position(|w| w.id as u64 == close_id.0) {
                let target_id = wl.windows[pos].id;
                let owned_popups: Vec<u64> = wl
                    .popups
                    .iter()
                    .filter(|(_, p)| p.owner_window_id == target_id)
                    .map(|(&pid, _)| pid)
                    .collect();
                for pid in owned_popups {
                    if let Some(pentry) = wl.popups.remove(&pid) {
                        pentry.shared.destroyed.store(true, Ordering::Release);
                        let proto_id = pentry.popup.wl_surface().id().protocol_id();
                        wl.surface_to_popup.remove(&proto_id);
                        app.window_closing(&pentry.native_window);
                    }
                }
                let entry = wl.windows.remove(pos);
                entry.shared.destroyed.store(true, Ordering::Release);
                let proto_id = entry.sctk_window.wl_surface().id().protocol_id();
                wl.surface_to_window.remove(&proto_id);
                app.window_closing(&entry.native_window);
            }
        }

        for (sync_id, sync) in app.take_pending_floating_syncs() {
            if let Some(entry) = wl.windows.iter_mut().find(|w| w.id as u64 == sync_id.0) {
                crate::backend::apply_floating_live_sync(&entry.native_window, sync);
                if sync.visible == Some(true) {
                    entry.state.pacer.request_immediate(Instant::now());
                }
            }
        }
        // 6. 同步越界原生弹出窗（`xdg_popup` + `xdg_positioner`）。
        for (pid, pcfg) in app.take_popup_sync_requests() {
            let created = wl.sync_popup(&qh, pid, &pcfg);
            if created {
                let _ = event_queue.roundtrip(&mut wl);
                if let Some(pentry) = wl.popups.get_mut(&pid) {
                    app.window_created(&mut pentry.state, &pentry.native_window);
                }
            }
        }
        for close_pid in app.take_popup_close_requests() {
            if let Some(pentry) = wl.popups.remove(&close_pid) {
                pentry.shared.destroyed.store(true, Ordering::Release);
                let proto_id = pentry.popup.wl_surface().id().protocol_id();
                wl.surface_to_popup.remove(&proto_id);
                app.window_closing(&pentry.native_window);
            }
        }

        // 7. 可见性驱动节流与出帧（仅在状态跃迁至 Suspended 时边沿触发 `app.suspended`）。
        let frame_now = Instant::now();
        let mut min_wait: Option<Duration> = None;
        for idx in 0..wl.windows.len() {
            if wl.windows[idx].throttle.is_suspended(frame_now) {
                if !wl.windows[idx].was_suspended {
                    wl.windows[idx].was_suspended = true;
                    app.suspended(&mut wl.windows[idx].state);
                }
                continue;
            }
            wl.windows[idx].was_suspended = false;
            let redraw = wl.windows[idx]
                .shared
                .redraw_requested
                .swap(false, Ordering::AcqRel);
            let pacer_due = wl.windows[idx].state.pacer.should_render(frame_now);
            if redraw || pacer_due {
                let surf = wl.windows[idx].sctk_window.wl_surface().clone();
                surf.frame(&qh, surf.clone());
                if pacer_due {
                    wl.windows[idx].throttle.on_frame_requested(frame_now);
                }
                let entry = &mut wl.windows[idx];
                let sz = entry.native_window.inner_size();
                entry
                    .a11y
                    .set_window_bounds(crate::a11y_linux::A11yWindowBounds::new(
                        0.0,
                        0.0,
                        f64::from(sz.width),
                        f64::from(sz.height),
                    ));
                if crate::a11y_platform::pump_platform_a11y(
                    &mut app,
                    &mut entry.a11y,
                    &mut entry.state,
                    &entry.native_window,
                ) == Flow::Exit
                {
                    wl.exit_requested = true;
                    break;
                }
                if app.frame(&mut entry.state, &entry.native_window) == Flow::Exit {
                    wl.exit_requested = true;
                    break;
                }
                wl.windows[idx].state.pacer.on_present(Instant::now());
            } else if let Some(next_t) = wl.windows[idx].state.pacer.next_wake(frame_now) {
                let wait = next_t.saturating_duration_since(frame_now);
                min_wait = Some(min_wait.map_or(wait, |cur| cur.min(wait)));
            }
        }
        if wl.exit_requested {
            break;
        }

        // 弹出窗出帧
        let active_pids: Vec<u64> = wl.popups.keys().copied().collect();
        for pid in active_pids {
            if let Some(pentry) = wl.popups.get_mut(&pid) {
                if pentry.configured {
                    let _ = app.frame(&mut pentry.state, &pentry.native_window);
                }
            }
        }

        if let Some(app_wake) = app.next_wake() {
            let wait = app_wake.saturating_duration_since(Instant::now());
            min_wait = Some(min_wait.map_or(wait, |cur| cur.min(wait)));
        }
        if let Some(rep_wake) = wl.key_repeat.next_wake() {
            let wait = rep_wake.saturating_duration_since(Instant::now());
            min_wait = Some(min_wait.map_or(wait, |cur| cur.min(wait)));
        }

        let _ = conn.flush();
        let has_pending = wl.windows.iter().any(|w| !w.pending_events.is_empty());
        if !has_pending {
            if let Some(guard) = conn.prepare_read() {
                let wl_fd = guard.connection_fd().as_raw_fd();
                wait_wayland_or_deadline(wl_fd, wake_pipe.read_fd, min_wait);
                let _ = guard.read();
            }
        }
    }

    // 严格遵守 Vulkan 表面销毁契约：先通知应用销毁全部 `VkSurfaceKHR`，再释放 `wl_surface`/`wl_display`。
    for (_, pentry) in wl.popups.drain() {
        pentry.shared.destroyed.store(true, Ordering::Release);
        app.window_closing(&pentry.native_window);
    }
    for entry in wl.windows.drain(..) {
        entry.shared.destroyed.store(true, Ordering::Release);
        app.window_closing(&entry.native_window);
    }

    Ok(())
}

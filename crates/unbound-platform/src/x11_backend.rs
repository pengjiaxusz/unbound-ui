//! 原生 X11 备用窗口后端（基于 `x11rb` 纯 Rust 协议 + `dl-libxcb` 运行时动态加载）。
//!
//! 覆盖二期 Spec（#210）：
//! - 编译期零 `libxcb-dev` / `libx11-dev` 依赖，运行时 `dlopen("libxcb.so.1")` 提供 `RawDisplayHandle::Xcb` 对接 `VK_KHR_xcb_surface`；
//! - 单窗与多窗（`Second Window`）生命周期、EWMH 标题/最大化/无边框装饰与 CSD 拖拽缩放（`_NET_WM_MOVERESIZE`）；
//! - 键鼠输入（X11 keycode → evdev 统一映射）、剪贴板读写（`CLIPBOARD` 选择协议）；
//! - XRandR 显示器工作区查询与无激活越界弹出窗（ToolTip 通过 XFixes 空输入区域实现鼠标穿透）；
//! - 基于 `ppoll` 的低延迟事件等待与跨线程唤醒管道。

use core::ffi::c_void;
use core::num::NonZeroU32;
use core::ptr::NonNull;
use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle, XcbDisplayHandle, XcbWindowHandle,
};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::xfixes::ConnectionExt as XfixesConnectionExt;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageData, ClientMessageEvent,
    ConfigureWindowAux, ConnectionExt, CreateWindowAux, EventMask, Font, GrabMode, PropMode,
    Rectangle, SelectionNotifyEvent, Window as X11WindowId, WindowClass,
};
use x11rb::protocol::Event as X11Event;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::xcb_ffi::XCBConnection;
use x11rb::COPY_DEPTH_FROM_PARENT;

use crate::backend::{push_coalesced_event, Flow, PlatformApp};
use crate::floating::{FloatingWindowCapabilities, MonitorWorkArea, WindowRole};
use crate::linux_backend::{
    compute_interactive_resize_rect, place_x11_popup_in_work_area, set_active_linux_backend,
    LinuxBackendKind,
};
use crate::native_window::NativeWindow;
use crate::pacing::PresentWaitPolicy;
use crate::resize::Extent;
use crate::wayland_logic::{
    csd_resize_direction_at, cursor_for_resize_direction, register_wayland_clipboard,
    set_primary_output_metrics, translate_evdev_key, PrimaryOutputMetrics, WaylandClipboardBackend,
};
use crate::win32_frame::FrameHitMetrics;
use crate::win32_window::{
    CursorIcon, PhysicalPosition, PhysicalSize, ResizeDirection, WindowId as NativeWindowId,
};
use crate::window::{
    apply, resolve_window_placement, ButtonState, Modifiers, MouseButton, PlatformError,
    PlatformEvent, PlatformState, ScreenRect, WindowConfig,
};

const X11_CSD_RESIZE_BORDER: f64 = 8.0;

/// 应用侧向 X11 事件循环发送的窗口控制命令。
enum X11Command {
    SetTitle {
        window: X11WindowId,
        title: String,
    },
    SetMinimized {
        window: X11WindowId,
    },
    SetMaximized {
        window: X11WindowId,
        maximized: bool,
    },
    SetVisible {
        window: X11WindowId,
        visible: bool,
    },
    SetVisibleNoActivate {
        window: X11WindowId,
        visible: bool,
    },
    SetAlwaysOnTop {
        window: X11WindowId,
        topmost: bool,
    },
    SetClickThrough {
        window: X11WindowId,
        click_through: bool,
    },
    SetOuterPosition {
        window: X11WindowId,
        x: i32,
        y: i32,
    },
    SetOuterRect {
        window: X11WindowId,
        rect: ScreenRect,
    },
    SetCursor {
        window: X11WindowId,
        cursor: CursorIcon,
    },
    DragWindow {
        window: X11WindowId,
    },
    DragResizeWindow {
        window: X11WindowId,
        direction: ResizeDirection,
    },
    RequestClose {
        window: X11WindowId,
    },
    RequestRedraw {
        window: X11WindowId,
    },
    ClipboardWrite {
        text: String,
    },
    ClipboardWriteFiles {
        uri_list: String,
        gnome_copied_files: String,
    },
    ThemeChanged {
        dark: bool,
    },
    StartOutboundFileDrag {
        uri_list: String,
    },
}

/// 跨线程管道唤醒信号句柄（持锁写入并在释放时持锁关闭，避免 FD 复用竞态）。
#[derive(Clone)]
struct X11WakeSignal(Arc<Mutex<Option<RawFd>>>);

impl X11WakeSignal {
    fn signal(&self) {
        let Ok(guard) = self.0.lock() else {
            return;
        };
        let Some(fd) = *guard else {
            return;
        };
        let byte: u8 = 1;
        // SAFETY: 持锁期间 `X11WakePipe::drop` 不会关闭 `fd`，写端为非阻塞模式。
        unsafe {
            let _ = libc::write(fd, (&raw const byte).cast::<c_void>(), 1);
        }
    }
}

struct X11WakePipe {
    read_fd: RawFd,
    write_fd: Arc<Mutex<Option<RawFd>>>,
}

impl X11WakePipe {
    fn new() -> Result<Self, PlatformError> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `fds` 为有效的双元素数组，创建非阻塞管道用于唤醒 `ppoll`。
        let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if rc != 0 {
            return Err(PlatformError::EventLoop(
                "failed to create X11 wake pipe".into(),
            ));
        }
        // SAFETY: `fds` 为刚创建的有效管道描述符，仅设置非阻塞标志。
        unsafe {
            for fd in fds {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags >= 0 {
                    let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
        }
        Ok(Self {
            read_fd: fds[0],
            write_fd: Arc::new(Mutex::new(Some(fds[1]))),
        })
    }

    fn signal_handle(&self) -> X11WakeSignal {
        X11WakeSignal(Arc::clone(&self.write_fd))
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

impl Drop for X11WakePipe {
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

/// RAII 守卫：确保退出事件循环时清空剪贴板桥接与当前活跃后端标记。
struct X11SessionGuard;

impl Drop for X11SessionGuard {
    fn drop(&mut self) {
        register_wayland_clipboard(None);
        crate::linux_dnd::register_linux_outbound_drag(None);
        set_active_linux_backend(None);
    }
}

struct X11OutboundDragBridge {
    cmd_tx: Mutex<mpsc::Sender<X11Command>>,
    wake_signal: X11WakeSignal,
}

impl crate::linux_dnd::LinuxOutboundDragBackend for X11OutboundDragBridge {
    fn begin_file_drag(
        &self,
        request: &crate::ole_drag::OutboundFileDragRequest,
    ) -> Result<crate::ole_drag::OutboundFileDragOutcome, PlatformError> {
        let uri_list = crate::linux_dnd::encode_uri_list(&request.paths);
        let sent = self.cmd_tx.lock().ok().is_some_and(|tx| {
            tx.send(X11Command::StartOutboundFileDrag { uri_list })
                .is_ok()
        });
        if !sent {
            return Ok(crate::ole_drag::OutboundFileDragOutcome::Cancelled);
        }
        self.wake_signal.signal();
        let effect = if request.allow_copy {
            crate::overlay_host::FileDragEffect::Copy
        } else if request.allow_move {
            crate::overlay_host::FileDragEffect::Move
        } else {
            crate::overlay_host::FileDragEffect::Link
        };
        Ok(crate::ole_drag::OutboundFileDragOutcome::Completed(effect))
    }
}

/// X11 窗口句柄与事件循环共享的状态快照。
struct X11WindowShared {
    xid: X11WindowId,
    screen_num: i32,
    xcb_conn_ptr: usize,
    decorated: bool,
    popup_id: Option<u64>,
    inner_size: Mutex<PhysicalSize>,
    screen_origin: Mutex<(i32, i32)>,
    scale_bits: AtomicU64,
    maximized: AtomicBool,
    always_on_top: AtomicBool,
    visible: AtomicBool,
    destroyed: AtomicBool,
    redraw_requested: AtomicBool,
    cursor: AtomicU32,
    frame_metrics: Mutex<Option<Arc<FrameHitMetrics>>>,
    cmd_tx: Mutex<mpsc::Sender<X11Command>>,
    wake_signal: X11WakeSignal,
}

/// X11 原生窗口句柄（由 [`NativeWindow`] 包装并暴露给上层应用）。
#[derive(Clone)]
pub(crate) struct X11WindowHandle {
    shared: Arc<X11WindowShared>,
}

impl X11WindowHandle {
    fn send(&self, cmd: X11Command) {
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
        NativeWindowId(self.shared.xid as isize)
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
        let (x, y) = self
            .shared
            .screen_origin
            .lock()
            .map(|g| *g)
            .unwrap_or((0, 0));
        let size = self.inner_size();
        ScreenRect::new(x, y, size.width, size.height)
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
        self.shared.screen_origin.lock().ok().map(|origin| *origin)
    }

    pub(crate) fn normal_bounds(&self) -> Option<ScreenRect> {
        None
    }

    pub(crate) fn set_outer_position(&self, x: i32, y: i32) {
        if let Ok(mut origin) = self.shared.screen_origin.lock() {
            *origin = (x, y);
        }
        self.send(X11Command::SetOuterPosition {
            window: self.shared.xid,
            x,
            y,
        });
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
        self.send(X11Command::SetVisible {
            window: self.shared.xid,
            visible,
        });
    }

    pub(crate) fn set_visible_no_activate(&self, visible: bool) {
        self.shared.visible.store(visible, Ordering::Release);
        self.send(X11Command::SetVisibleNoActivate {
            window: self.shared.xid,
            visible,
        });
    }

    pub(crate) fn set_always_on_top(&self, topmost: bool) {
        self.shared.always_on_top.store(topmost, Ordering::Release);
        self.send(X11Command::SetAlwaysOnTop {
            window: self.shared.xid,
            topmost,
        });
    }

    pub(crate) fn is_always_on_top(&self) -> bool {
        self.shared.always_on_top.load(Ordering::Acquire)
    }

    pub(crate) fn set_click_through(&self, click_through: bool) {
        self.send(X11Command::SetClickThrough {
            window: self.shared.xid,
            click_through,
        });
    }

    pub(crate) fn set_outer_rect(&self, rect: ScreenRect) {
        if let Ok(mut org) = self.shared.screen_origin.lock() {
            *org = (rect.x, rect.y);
        }
        if let Ok(mut sz) = self.shared.inner_size.lock() {
            *sz = PhysicalSize::new(rect.width.max(1), rect.height.max(1));
        }
        self.send(X11Command::SetOuterRect {
            window: self.shared.xid,
            rect,
        });
    }

    pub(crate) fn capabilities(&self) -> FloatingWindowCapabilities {
        FloatingWindowCapabilities::x11_default()
    }

    pub(crate) fn work_area(&self) -> Option<MonitorWorkArea> {
        let rect = self.client_rect_on_screen();
        crate::floating::work_area_for_point(rect.x, rect.y)
    }

    pub(crate) fn set_minimized(&self, minimized: bool) {
        if minimized {
            self.send(X11Command::SetMinimized {
                window: self.shared.xid,
            });
        }
    }

    pub(crate) fn set_maximized(&self, maximized: bool) {
        self.shared.maximized.store(maximized, Ordering::Release);
        self.send(X11Command::SetMaximized {
            window: self.shared.xid,
            maximized,
        });
    }

    pub(crate) fn set_title(&self, title: &str) {
        self.send(X11Command::SetTitle {
            window: self.shared.xid,
            title: title.to_owned(),
        });
    }

    pub(crate) fn set_cursor(&self, cursor: CursorIcon) {
        self.shared.cursor.store(cursor.to_u32(), Ordering::Release);
        self.send(X11Command::SetCursor {
            window: self.shared.xid,
            cursor,
        });
    }

    pub(crate) fn request_redraw(&self) {
        self.shared.redraw_requested.store(true, Ordering::Release);
        self.send(X11Command::RequestRedraw {
            window: self.shared.xid,
        });
    }

    pub(crate) fn request_close(&self) {
        self.send(X11Command::RequestClose {
            window: self.shared.xid,
        });
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        self.shared.destroyed.load(Ordering::Acquire)
    }

    pub(crate) fn drag_window(&self) -> Result<(), PlatformError> {
        self.send(X11Command::DragWindow {
            window: self.shared.xid,
        });
        Ok(())
    }

    pub(crate) fn drag_resize_window(
        &self,
        direction: ResizeDirection,
    ) -> Result<(), PlatformError> {
        self.send(X11Command::DragResizeWindow {
            window: self.shared.xid,
            direction,
        });
        Ok(())
    }

    pub(crate) fn set_ime_allowed(&self, allowed: bool) {
        let _ = allowed;
    }

    pub(crate) fn set_ime_cursor_area(&self, position: PhysicalPosition, size: PhysicalSize) {
        let _ = (position, size);
    }

    pub(crate) fn install_custom_frame(&self, metrics: Arc<FrameHitMetrics>) -> bool {
        if let Ok(mut guard) = self.shared.frame_metrics.lock() {
            *guard = Some(metrics);
            true
        } else {
            false
        }
    }
}

impl HasWindowHandle for X11WindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let win = NonZeroU32::new(self.shared.xid).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Xcb(XcbWindowHandle::new(win));
        // SAFETY: X11 窗口 ID 在窗口存活期内有效。
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for X11WindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let conn = NonNull::new(self.shared.xcb_conn_ptr as *mut c_void);
        let raw = RawDisplayHandle::Xcb(XcbDisplayHandle::new(conn, self.shared.screen_num));
        // SAFETY: `xcb_connection_t*` 在 X11 事件循环连接存活期内有效。
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

/// X11 剪贴板跨线程桥接实现。
struct X11ClipboardBridge {
    cached_text: Arc<Mutex<Option<String>>>,
    cached_files: Arc<Mutex<Vec<std::path::PathBuf>>>,
    cached_file_effect: Arc<Mutex<Option<crate::clipboard::ClipboardFileDropEffect>>>,
    cmd_tx: Mutex<mpsc::Sender<X11Command>>,
    wake_signal: X11WakeSignal,
}

impl WaylandClipboardBackend for X11ClipboardBridge {
    fn read_text(&self) -> Option<String> {
        self.cached_text.lock().ok().and_then(|g| g.clone())
    }

    fn write_text(&self, text: &str) -> bool {
        if let Ok(mut guard) = self.cached_text.lock() {
            *guard = Some(text.to_owned());
        }
        if let Ok(tx) = self.cmd_tx.lock() {
            let ok = tx
                .send(X11Command::ClipboardWrite {
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
                .send(X11Command::ClipboardWriteFiles {
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

/// 将框架光标样式映射为标准 X11 核心字体光标字形编号（`X11/cursorfont.h`）。
///
/// `CursorIcon::Default` 返回 `None`，使用 `x11rb::NONE` 继承宿主/WSLg 原生默认箭头光标，
/// 彻底避免在 4K WSLg XWayland 下加载 24px+ Adwaita 光栅主题后被 2× 放大导致光标异常巨大。
const fn x11_core_cursor_glyph(icon: CursorIcon) -> Option<u16> {
    match icon {
        CursorIcon::Default | CursorIcon::Custom(_) => None,
        CursorIcon::Pointer => Some(60),    // XC_hand2
        CursorIcon::Text => Some(152),      // XC_xterm
        CursorIcon::Grab => Some(58),       // XC_hand1
        CursorIcon::Grabbing => Some(52),   // XC_fleur
        CursorIcon::Crosshair => Some(34),  // XC_crosshair
        CursorIcon::Move => Some(52),       // XC_fleur
        CursorIcon::NsResize => Some(116),  // XC_sb_v_double_arrow
        CursorIcon::EwResize => Some(108),  // XC_sb_h_double_arrow
        CursorIcon::NwseResize => Some(14), // XC_bottom_right_corner
        CursorIcon::NeswResize => Some(12), // XC_bottom_left_corner
        CursorIcon::NotAllowed => Some(0),  // XC_X_cursor
    }
}

/// X11 核心字体光标缓存：零磁盘主题扫描、按样式复用 `Cursor` XID、按窗口去重。
struct X11CursorCache {
    cursor_font: Option<Font>,
    cursors: HashMap<u32, u32>,
    applied: HashMap<X11WindowId, CursorIcon>,
}

impl X11CursorCache {
    fn new(conn: &XCBConnection) -> Self {
        let cursor_font = conn.generate_id().ok().and_then(|fid| {
            conn.open_font(fid, b"cursor").ok()?;
            Some(fid)
        });
        Self {
            cursor_font,
            cursors: HashMap::new(),
            applied: HashMap::new(),
        }
    }

    fn cursor_id(&mut self, conn: &XCBConnection, icon: CursorIcon) -> u32 {
        let Some(glyph) = x11_core_cursor_glyph(icon) else {
            return x11rb::NONE;
        };
        let key = icon.to_u32();
        if let Some(&existing) = self.cursors.get(&key) {
            return existing;
        }
        let Some(font) = self.cursor_font else {
            return x11rb::NONE;
        };
        let Ok(cid) = conn.generate_id() else {
            return x11rb::NONE;
        };
        if conn
            .create_glyph_cursor(
                cid,
                font,
                font,
                glyph,
                glyph + 1,
                0,
                0,
                0,
                0xFFFF,
                0xFFFF,
                0xFFFF,
            )
            .is_ok()
        {
            self.cursors.insert(key, cid);
            cid
        } else {
            x11rb::NONE
        }
    }

    fn apply(&mut self, conn: &XCBConnection, window: X11WindowId, icon: CursorIcon) {
        if self.applied.get(&window) == Some(&icon) {
            return;
        }
        let cid = self.cursor_id(conn, icon);
        let _ =
            conn.change_window_attributes(window, &ChangeWindowAttributesAux::new().cursor(cid));
        let _ = conn.flush();
        self.applied.insert(window, icon);
    }

    fn remove_window(&mut self, window: X11WindowId) {
        self.applied.remove(&window);
    }

    fn destroy_all(&mut self, conn: &XCBConnection) {
        for (_, cid) in self.cursors.drain() {
            let _ = conn.free_cursor(cid);
        }
        if let Some(font) = self.cursor_font.take() {
            let _ = conn.close_font(font);
        }
        self.applied.clear();
    }
}

/// X11 客户端即时边缘拖拽缩放状态（零 XWM 握手延迟，按下即响应）。
struct X11ActiveResize {
    xid: X11WindowId,
    dir: ResizeDirection,
    start_root_x: i16,
    start_root_y: i16,
    start_rect: ScreenRect,
    min_w: u32,
    min_h: u32,
}

/// 缓存的 X11 Atom 集合。
struct X11Atoms {
    wm_protocols: Atom,
    wm_delete_window: Atom,
    net_wm_name: Atom,
    net_wm_state: Atom,
    net_wm_state_maximized_horz: Atom,
    net_wm_state_maximized_vert: Atom,
    net_wm_state_above: Atom,
    net_wm_state_skip_taskbar: Atom,
    net_wm_state_skip_pager: Atom,
    wm_hints: Atom,
    net_wm_moveresize: Atom,
    motif_wm_hints: Atom,
    utf8_string: Atom,
    clipboard: Atom,
    targets: Atom,
    clipboard_prop: Atom,
    net_workarea: Atom,
    wm_change_state: Atom,
    xdnd_aware: Atom,
    xdnd_enter: Atom,
    xdnd_position: Atom,
    xdnd_status: Atom,
    xdnd_leave: Atom,
    xdnd_drop: Atom,
    xdnd_finished: Atom,
    xdnd_selection: Atom,
    xdnd_action_copy: Atom,
    uri_list: Atom,
    gnome_copied_files: Atom,
    xdnd_prop: Atom,
}

impl X11Atoms {
    fn intern(conn: &XCBConnection) -> Result<Self, PlatformError> {
        let intern = |name: &[u8]| -> Result<Atom, PlatformError> {
            conn.intern_atom(false, name)
                .map_err(|e| PlatformError::EventLoop(format!("X11 intern_atom failed: {e}")))?
                .reply()
                .map_err(|e| PlatformError::EventLoop(format!("X11 intern_atom reply failed: {e}")))
                .map(|r| r.atom)
        };
        Ok(Self {
            wm_protocols: intern(b"WM_PROTOCOLS")?,
            wm_delete_window: intern(b"WM_DELETE_WINDOW")?,
            net_wm_name: intern(b"_NET_WM_NAME")?,
            net_wm_state: intern(b"_NET_WM_STATE")?,
            net_wm_state_maximized_horz: intern(b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            net_wm_state_maximized_vert: intern(b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            net_wm_state_above: intern(b"_NET_WM_STATE_ABOVE")?,
            net_wm_state_skip_taskbar: intern(b"_NET_WM_STATE_SKIP_TASKBAR")?,
            net_wm_state_skip_pager: intern(b"_NET_WM_STATE_SKIP_PAGER")?,
            wm_hints: intern(b"WM_HINTS")?,
            net_wm_moveresize: intern(b"_NET_WM_MOVERESIZE")?,
            motif_wm_hints: intern(b"_MOTIF_WM_HINTS")?,
            utf8_string: intern(b"UTF8_STRING")?,
            clipboard: intern(b"CLIPBOARD")?,
            targets: intern(b"TARGETS")?,
            clipboard_prop: intern(b"UNBOUND_CLIPBOARD_PROP")?,
            net_workarea: intern(b"_NET_WORKAREA")?,
            wm_change_state: intern(b"WM_CHANGE_STATE")?,
            xdnd_aware: intern(b"XdndAware")?,
            xdnd_enter: intern(b"XdndEnter")?,
            xdnd_position: intern(b"XdndPosition")?,
            xdnd_status: intern(b"XdndStatus")?,
            xdnd_leave: intern(b"XdndLeave")?,
            xdnd_drop: intern(b"XdndDrop")?,
            xdnd_finished: intern(b"XdndFinished")?,
            xdnd_selection: intern(b"XdndSelection")?,
            xdnd_action_copy: intern(b"XdndActionCopy")?,
            uri_list: intern(b"text/uri-list")?,
            gnome_copied_files: intern(b"x-special/gnome-copied-files")?,
            xdnd_prop: intern(b"UNBOUND_XDND_PROP")?,
        })
    }
}

struct X11ToplevelEntry {
    xid: X11WindowId,
    native_window: NativeWindow,
    shared: Arc<X11WindowShared>,
    state: PlatformState,
    resizable: bool,
    min_width: u32,
    min_height: u32,
    csd_edge_active: bool,
    pending_events: Vec<PlatformEvent>,
    cursor_local: (f64, f64),
    last_pointer_root: (i16, i16),
    a11y: crate::a11y_platform::PlatformA11y,
}

struct X11PopupEntry {
    xid: X11WindowId,
    owner_xid: X11WindowId,
    native_window: NativeWindow,
    shared: Arc<X11WindowShared>,
    state: PlatformState,
    pending_events: Vec<PlatformEvent>,
}

const fn x11_modifiers(mask: u16) -> Modifiers {
    Modifiers {
        shift: (mask & 0x0001) != 0,
        control: (mask & 0x0004) != 0,
        alt: (mask & 0x0008) != 0,
        meta: (mask & 0x0040) != 0,
    }
}

const fn x11_button_to_mouse(detail: u8) -> Option<MouseButton> {
    match detail {
        1 => Some(MouseButton::Left),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::Right),
        8 => Some(MouseButton::Back),
        9 => Some(MouseButton::Forward),
        _ => None,
    }
}

/// 查询当前包含目标点的显示器工作区（优先 XRandR，失败时回退 `_NET_WORKAREA` 或根窗尺寸）。
fn query_work_area_for_point(
    conn: &XCBConnection,
    root: X11WindowId,
    root_w: u16,
    root_h: u16,
    atoms: &X11Atoms,
    pt_x: i32,
    pt_y: i32,
) -> ScreenRect {
    if let Ok(cookie) = conn.randr_get_monitors(root, true) {
        if let Ok(reply) = cookie.reply() {
            for mon in &reply.monitors {
                let left = i32::from(mon.x);
                let top = i32::from(mon.y);
                let right = left + i32::from(mon.width);
                let bottom = top + i32::from(mon.height);
                if pt_x >= left && pt_x < right && pt_y >= top && pt_y < bottom {
                    return ScreenRect::new(left, top, u32::from(mon.width), u32::from(mon.height));
                }
            }
            if let Some(first) = reply.monitors.first() {
                return ScreenRect::new(
                    i32::from(first.x),
                    i32::from(first.y),
                    u32::from(first.width),
                    u32::from(first.height),
                );
            }
        }
    }
    if let Ok(cookie) = conn.get_property(false, root, atoms.net_workarea, AtomEnum::CARDINAL, 0, 4)
    {
        if let Ok(reply) = cookie.reply() {
            if let Some(mut vals) = reply.value32() {
                if let (Some(x), Some(y), Some(w), Some(h)) =
                    (vals.next(), vals.next(), vals.next(), vals.next())
                {
                    if w > 0 && h > 0 {
                        return ScreenRect::new(x as i32, y as i32, w, h);
                    }
                }
            }
        }
    }
    ScreenRect::new(0, 0, u32::from(root_w.max(1)), u32::from(root_h.max(1)))
}

fn set_window_title(conn: &XCBConnection, atoms: &X11Atoms, window: X11WindowId, title: &str) {
    let bytes = title.as_bytes();
    let _ = conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        bytes,
    );
    let _ = conn.change_property8(
        PropMode::REPLACE,
        window,
        atoms.net_wm_name,
        atoms.utf8_string,
        bytes,
    );
}

fn set_motif_borderless(conn: &XCBConnection, atoms: &X11Atoms, window: X11WindowId) {
    // `_MOTIF_WM_HINTS`: flags = 2 (decorations), functions = 0, decorations = 0
    let hints: [u32; 5] = [2, 0, 0, 0, 0];
    let _ = conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.motif_wm_hints,
        atoms.motif_wm_hints,
        &hints,
    );
}

fn set_x11_no_activate_hints(conn: &XCBConnection, atoms: &X11Atoms, window: X11WindowId) {
    // ICCCM `WM_HINTS`: flags = 1 (InputHint), input = 0 (False)
    let hints: [u32; 9] = [1, 0, 0, 0, 0, 0, 0, 0, 0];
    let _ = conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.wm_hints,
        atoms.wm_hints,
        &hints,
    );
}

fn set_x11_click_through(conn: &XCBConnection, window: X11WindowId, click_through: bool) {
    if click_through {
        if let Ok(region_id) = conn.generate_id() {
            let empty: [Rectangle; 0] = [];
            if conn.xfixes_create_region(region_id, &empty).is_ok() {
                let _ = conn.xfixes_set_window_shape_region(
                    window,
                    x11rb::protocol::shape::SK::INPUT,
                    0,
                    0,
                    region_id,
                );
                let _ = conn.xfixes_destroy_region(region_id);
            }
        }
    } else {
        let _ = conn.xfixes_set_window_shape_region(
            window,
            x11rb::protocol::shape::SK::INPUT,
            0,
            0,
            x11rb::NONE,
        );
    }
}

fn send_net_wm_moveresize(
    conn: &XCBConnection,
    root: X11WindowId,
    atoms: &X11Atoms,
    window: X11WindowId,
    root_x: i16,
    root_y: i16,
    direction: u32,
) {
    let _ = conn.ungrab_pointer(x11rb::CURRENT_TIME);
    let event = ClientMessageEvent {
        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window,
        type_: atoms.net_wm_moveresize,
        data: ClientMessageData::from([
            u32::from(root_x.max(0) as u16),
            u32::from(root_y.max(0) as u16),
            direction,
            1, // 左键发起
            1, // 普通应用源
        ]),
    };
    let _ = conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    );
    let _ = conn.flush();
}

fn wait_x11_or_deadline(x11_fd: RawFd, wake_read_fd: RawFd, wait: Option<Duration>) {
    let mut pollfds = [
        libc::pollfd {
            fd: x11_fd,
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

/// 运行原生 X11 事件循环直至全部顶层窗口关闭或应用返回 [`Flow::Exit`]。
///
/// # Errors
/// 无法动态加载 `libxcb.so.1`、无法连接 X11 服务器或窗口创建失败时返回 [`PlatformError`]。
pub(crate) fn run_event_loop<A: PlatformApp>(
    config: WindowConfig,
    mut app: A,
) -> Result<(), PlatformError> {
    let (conn, screen_num) = XCBConnection::connect(None).map_err(|e| {
        PlatformError::EventLoop(format!("failed to connect to X11 display via libxcb: {e}"))
    })?;
    let atoms = X11Atoms::intern(&conn)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let root_w = screen.width_in_pixels;
    let root_h = screen.height_in_pixels;
    let xcb_conn_ptr = conn.get_raw_xcb_connection() as usize;

    set_primary_output_metrics(PrimaryOutputMetrics {
        physical_width: u32::from(root_w.max(1)),
        physical_height: u32::from(root_h.max(1)),
        scale_factor: 1.0,
        refresh_rate_hz: Some(60.0),
    });

    let wake_pipe = X11WakePipe::new()?;
    let wake_signal = wake_pipe.signal_handle();
    let (cmd_tx, cmd_rx) = mpsc::channel::<X11Command>();
    let clipboard_cache = Arc::new(Mutex::new(None));
    let clipboard_files_cache = Arc::new(Mutex::new(Vec::new()));
    let clipboard_file_effect_cache = Arc::new(Mutex::new(None));
    let mut clipboard_owned_text = String::new();
    let mut clipboard_owned_uri_list = String::new();
    let mut clipboard_owned_gnome_files = String::new();
    let mut clipboard_has_files = false;
    let mut xdnd_outbound_uri_list = String::new();

    let _session_guard = X11SessionGuard;
    set_active_linux_backend(Some(LinuxBackendKind::X11));
    register_wayland_clipboard(Some(Arc::new(X11ClipboardBridge {
        cached_text: Arc::clone(&clipboard_cache),
        cached_files: clipboard_files_cache,
        cached_file_effect: clipboard_file_effect_cache,
        cmd_tx: Mutex::new(cmd_tx.clone()),
        wake_signal: wake_signal.clone(),
    })));
    crate::linux_dnd::register_linux_outbound_drag(Some(Arc::new(X11OutboundDragBridge {
        cmd_tx: Mutex::new(cmd_tx.clone()),
        wake_signal: wake_signal.clone(),
    })));

    let waker_sig = wake_signal.clone();
    app.attach_waker(Arc::new(move || waker_sig.signal()));

    let theme_tx = Mutex::new(cmd_tx.clone());
    let theme_wake = wake_signal.clone();
    crate::linux_dbus::register_theme_change_listener(Arc::new(move |dark| {
        if let Ok(tx) = theme_tx.lock() {
            if tx.send(X11Command::ThemeChanged { dark }).is_ok() {
                theme_wake.signal();
            }
        }
    }));

    let mut windows: Vec<X11ToplevelEntry> = Vec::new();
    let mut popups: HashMap<u64, X11PopupEntry> = HashMap::new();
    let mut exit_requested = false;
    let mut xdnd_source_window: Option<X11WindowId> = None;
    let mut cursor_cache = X11CursorCache::new(&conn);
    let mut active_resize: Option<X11ActiveResize> = None;

    let create_toplevel = |cfg: &WindowConfig| -> Result<X11ToplevelEntry, PlatformError> {
        let xid = conn
            .generate_id()
            .map_err(|e| PlatformError::Window(format!("X11 generate_id failed: {e}")))?;
        let work = query_work_area_for_point(&conn, root, root_w, root_h, &atoms, 0, 0);
        let placement = resolve_window_placement(
            cfg,
            (
                work.x,
                work.y,
                work.x + work.width as i32,
                work.y + work.height as i32,
            ),
            1.0,
        );
        let width = placement.width.clamp(1, i32::from(u16::MAX)) as u16;
        let height = placement.height.clamp(1, i32::from(u16::MAX)) as u16;
        let x = placement.x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        let y = placement.y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;

        let event_mask = EventMask::EXPOSURE
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::KEY_PRESS
            | EventMask::KEY_RELEASE
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::FOCUS_CHANGE;

        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            xid,
            root,
            x,
            y,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(event_mask),
        )
        .map_err(|e| PlatformError::Window(format!("X11 create_window failed: {e}")))?;

        set_window_title(&conn, &atoms, xid, &cfg.title);
        let _ = conn.change_property32(
            PropMode::REPLACE,
            xid,
            atoms.wm_protocols,
            AtomEnum::ATOM,
            &[atoms.wm_delete_window],
        );
        // 宣告支持 XDND v5 跨应用文件拖入
        let _ = conn.change_property32(
            PropMode::REPLACE,
            xid,
            atoms.xdnd_aware,
            AtomEnum::ATOM,
            &[5u32],
        );
        if !cfg.decorations || cfg.role != WindowRole::Standard {
            set_motif_borderless(&conn, &atoms, xid);
        }
        let mut initial_states = Vec::new();
        if cfg.floating_policy.always_on_top {
            initial_states.push(atoms.net_wm_state_above);
        }
        if cfg.floating_policy.skip_taskbar {
            initial_states.push(atoms.net_wm_state_skip_taskbar);
            initial_states.push(atoms.net_wm_state_skip_pager);
        }
        if !initial_states.is_empty() {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                xid,
                atoms.net_wm_state,
                AtomEnum::ATOM,
                &initial_states,
            );
        }
        if cfg.floating_policy.no_activate {
            set_x11_no_activate_hints(&conn, &atoms, xid);
        }
        if cfg.floating_policy.click_through {
            set_x11_click_through(&conn, xid, true);
        }
        if cfg.visible {
            let _ = conn.map_window(xid);
        }
        let _ = conn.flush();

        let shared = Arc::new(X11WindowShared {
            xid,
            screen_num: screen_num as i32,
            xcb_conn_ptr,
            decorated: cfg.decorations,
            popup_id: None,
            inner_size: Mutex::new(PhysicalSize::new(u32::from(width), u32::from(height))),
            screen_origin: Mutex::new((i32::from(x), i32::from(y))),
            scale_bits: AtomicU64::new(1.0_f64.to_bits()),
            maximized: AtomicBool::new(false),
            always_on_top: AtomicBool::new(cfg.floating_policy.always_on_top),
            visible: AtomicBool::new(cfg.visible),
            destroyed: AtomicBool::new(false),
            redraw_requested: AtomicBool::new(true),
            cursor: AtomicU32::new(CursorIcon::Default.to_u32()),
            frame_metrics: Mutex::new(None),
            cmd_tx: Mutex::new(cmd_tx.clone()),
            wake_signal: wake_signal.clone(),
        });
        let handle = X11WindowHandle {
            shared: Arc::clone(&shared),
        };
        let native_window = NativeWindow::from_x11(handle);
        let state = PlatformState::new(
            cfg,
            Extent::new(u32::from(width), u32::from(height)),
            PresentWaitPolicy::from_extensions(&[]),
        );
        let a11y_wake = wake_signal.clone();
        let a11y = crate::a11y_platform::PlatformA11y::for_linux(
            u64::from(xid),
            Some(Arc::new(move || a11y_wake.signal())),
        );
        Ok(X11ToplevelEntry {
            xid,
            native_window,
            shared,
            state,
            resizable: cfg.resizable,
            min_width: cfg.effective_min_size().0,
            min_height: cfg.effective_min_size().1,
            csd_edge_active: false,
            pending_events: Vec::new(),
            cursor_local: (0.0, 0.0),
            last_pointer_root: (0, 0),
            a11y,
        })
    };

    let primary = create_toplevel(&config)?;
    cursor_cache.apply(&conn, primary.xid, CursorIcon::Default);
    windows.push(primary);
    if let Some(first) = windows.first_mut() {
        app.resumed(&mut first.state, &first.native_window);
    }

    let x11_fd = conn.as_raw_fd();

    while !exit_requested && !windows.is_empty() {
        wake_pipe.drain();

        // 1. 排空所有待处理 X11 事件（交互缩放期间将高频 MotionNotify 合并为单次 configure_window）
        let mut pending_resize_cfg: Option<(X11WindowId, ScreenRect)> = None;
        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                X11Event::ConfigureNotify(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.window) {
                        let new_w = u32::from(ev.width.max(1));
                        let new_h = u32::from(ev.height.max(1));
                        let prev = entry.native_window.inner_size();
                        if new_w != prev.width || new_h != prev.height {
                            if let Ok(mut sz) = entry.shared.inner_size.lock() {
                                *sz = PhysicalSize::new(new_w, new_h);
                            }
                            push_coalesced_event(
                                &mut entry.pending_events,
                                PlatformEvent::Resized(Extent::new(new_w, new_h)),
                            );
                        }
                        if let Ok(mut org) = entry.shared.screen_origin.lock() {
                            if *org != (i32::from(ev.x), i32::from(ev.y)) {
                                *org = (i32::from(ev.x), i32::from(ev.y));
                                push_coalesced_event(
                                    &mut entry.pending_events,
                                    PlatformEvent::Moved {
                                        x: i32::from(ev.x),
                                        y: i32::from(ev.y),
                                    },
                                );
                            }
                        }
                    }
                }
                X11Event::Expose(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.window) {
                        entry.shared.redraw_requested.store(true, Ordering::Release);
                    }
                }
                X11Event::FocusIn(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                        entry.a11y.set_focus(true);
                        entry.pending_events.push(PlatformEvent::Focused(true));
                        let _ = conn.convert_selection(
                            entry.xid,
                            atoms.clipboard,
                            atoms.utf8_string,
                            atoms.clipboard_prop,
                            x11rb::CURRENT_TIME,
                        );
                    }
                }
                X11Event::FocusOut(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                        entry.a11y.set_focus(false);
                        entry.pending_events.push(PlatformEvent::Focused(false));
                    }
                }
                X11Event::KeyPress(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                        let mods = x11_modifiers(ev.state.into());
                        entry
                            .pending_events
                            .push(PlatformEvent::ModifiersChanged(mods));
                        // X11 evdev 键码固定为 Linux evdev + 8
                        let (key, _text) =
                            translate_evdev_key(u32::from(ev.detail.saturating_sub(8)), mods);
                        entry.pending_events.push(PlatformEvent::KeyInput {
                            state: ButtonState::Pressed,
                            key,
                            repeat: false,
                        });
                    }
                }
                X11Event::KeyRelease(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                        let mods = x11_modifiers(ev.state.into());
                        let (key, _text) =
                            translate_evdev_key(u32::from(ev.detail.saturating_sub(8)), mods);
                        entry.pending_events.push(PlatformEvent::KeyInput {
                            state: ButtonState::Released,
                            key,
                            repeat: false,
                        });
                    }
                }
                X11Event::MotionNotify(ev) => {
                    if let Some(res) = &active_resize {
                        let dx = i32::from(ev.root_x) - i32::from(res.start_root_x);
                        let dy = i32::from(ev.root_y) - i32::from(res.start_root_y);
                        let target = compute_interactive_resize_rect(
                            res.start_rect,
                            res.dir,
                            dx,
                            dy,
                            res.min_w,
                            res.min_h,
                        );
                        pending_resize_cfg = Some((res.xid, target));
                        continue;
                    }
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                        let x = f64::from(ev.event_x);
                        let y = f64::from(ev.event_y);
                        entry.cursor_local = (x, y);
                        entry.last_pointer_root = (ev.root_x, ev.root_y);
                        let size = entry.native_window.inner_size();
                        let is_csd = !entry.native_window.is_decorated();
                        let can_resize =
                            is_csd && entry.resizable && !entry.native_window.is_maximized();
                        if can_resize {
                            if let Some(dir) = csd_resize_direction_at(
                                x,
                                y,
                                f64::from(size.width),
                                f64::from(size.height),
                                X11_CSD_RESIZE_BORDER,
                            ) {
                                entry.csd_edge_active = true;
                                cursor_cache.apply(
                                    &conn,
                                    entry.xid,
                                    cursor_for_resize_direction(dir),
                                );
                            } else if entry.csd_edge_active {
                                entry.csd_edge_active = false;
                                cursor_cache.apply(
                                    &conn,
                                    entry.xid,
                                    entry.native_window.current_cursor(),
                                );
                            }
                        }
                        push_coalesced_event(
                            &mut entry.pending_events,
                            PlatformEvent::CursorMoved { x, y },
                        );
                    } else if let Some(pentry) = popups.values_mut().find(|p| p.xid == ev.event) {
                        push_coalesced_event(
                            &mut pentry.pending_events,
                            PlatformEvent::CursorMoved {
                                x: f64::from(ev.event_x),
                                y: f64::from(ev.event_y),
                            },
                        );
                    }
                }
                X11Event::ButtonPress(ev) => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                        entry.last_pointer_root = (ev.root_x, ev.root_y);
                        match ev.detail {
                            4 => entry.pending_events.push(PlatformEvent::MouseWheel {
                                delta_x: 0.0,
                                delta_y: 1.0,
                            }),
                            5 => entry.pending_events.push(PlatformEvent::MouseWheel {
                                delta_x: 0.0,
                                delta_y: -1.0,
                            }),
                            6 => entry.pending_events.push(PlatformEvent::MouseWheel {
                                delta_x: 1.0,
                                delta_y: 0.0,
                            }),
                            7 => entry.pending_events.push(PlatformEvent::MouseWheel {
                                delta_x: -1.0,
                                delta_y: 0.0,
                            }),
                            other => {
                                if let Some(mb) = x11_button_to_mouse(other) {
                                    let is_csd = !entry.native_window.is_decorated();
                                    let can_resize = is_csd
                                        && entry.resizable
                                        && !entry.native_window.is_maximized();
                                    if mb == MouseButton::Left && can_resize {
                                        let size = entry.native_window.inner_size();
                                        if let Some(dir) = csd_resize_direction_at(
                                            f64::from(ev.event_x),
                                            f64::from(ev.event_y),
                                            f64::from(size.width),
                                            f64::from(size.height),
                                            X11_CSD_RESIZE_BORDER,
                                        ) {
                                            let start_rect =
                                                entry.native_window.client_rect_on_screen();
                                            let cid = cursor_cache
                                                .cursor_id(&conn, cursor_for_resize_direction(dir));
                                            let _ = conn.grab_pointer(
                                                false,
                                                entry.xid,
                                                EventMask::POINTER_MOTION
                                                    | EventMask::BUTTON_RELEASE,
                                                GrabMode::ASYNC,
                                                GrabMode::ASYNC,
                                                x11rb::NONE,
                                                cid,
                                                ev.time,
                                            );
                                            let _ = conn.flush();
                                            active_resize = Some(X11ActiveResize {
                                                xid: entry.xid,
                                                dir,
                                                start_root_x: ev.root_x,
                                                start_root_y: ev.root_y,
                                                start_rect,
                                                min_w: entry.min_width,
                                                min_h: entry.min_height,
                                            });
                                            continue;
                                        }
                                    }
                                    entry.pending_events.push(PlatformEvent::MouseInput {
                                        button: mb,
                                        state: ButtonState::Pressed,
                                    });
                                }
                            }
                        }
                    } else if let Some(pentry) = popups.values_mut().find(|p| p.xid == ev.event) {
                        if let Some(mb) = x11_button_to_mouse(ev.detail) {
                            pentry.pending_events.push(PlatformEvent::MouseInput {
                                button: mb,
                                state: ButtonState::Pressed,
                            });
                        }
                    }
                }
                X11Event::ButtonRelease(ev) => {
                    if ev.detail == 1 && active_resize.take().is_some() {
                        let _ = conn.ungrab_pointer(ev.time);
                        let _ = conn.flush();
                        continue;
                    }
                    if let Some(mb) = x11_button_to_mouse(ev.detail) {
                        if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.event) {
                            entry.pending_events.push(PlatformEvent::MouseInput {
                                button: mb,
                                state: ButtonState::Released,
                            });
                        } else if let Some(pentry) = popups.values_mut().find(|p| p.xid == ev.event)
                        {
                            pentry.pending_events.push(PlatformEvent::MouseInput {
                                button: mb,
                                state: ButtonState::Released,
                            });
                        }
                    }
                }
                X11Event::ClientMessage(ev) => {
                    let d = ev.data.as_data32();
                    if ev.type_ == atoms.wm_protocols && d[0] == atoms.wm_delete_window {
                        if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.window) {
                            entry.pending_events.push(PlatformEvent::CloseRequested);
                        }
                    } else if ev.type_ == atoms.xdnd_enter {
                        xdnd_source_window = Some(d[0]);
                        if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.window) {
                            entry
                                .pending_events
                                .push(PlatformEvent::FileHover(std::path::PathBuf::new()));
                        }
                    } else if ev.type_ == atoms.xdnd_position {
                        let source = d[0];
                        xdnd_source_window = Some(source);
                        let status = ClientMessageEvent {
                            response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                            format: 32,
                            sequence: 0,
                            window: source,
                            type_: atoms.xdnd_status,
                            data: ClientMessageData::from([
                                ev.window,
                                1u32, // 接受拖放
                                0,
                                0,
                                atoms.xdnd_action_copy,
                            ]),
                        };
                        let _ = conn.send_event(false, source, EventMask::NO_EVENT, status);
                    } else if ev.type_ == atoms.xdnd_leave {
                        xdnd_source_window = None;
                        if let Some(entry) = windows.iter_mut().find(|w| w.xid == ev.window) {
                            entry.pending_events.push(PlatformEvent::FileHoverCancelled);
                        }
                    } else if ev.type_ == atoms.xdnd_drop {
                        let ts = d[2];
                        let _ = conn.convert_selection(
                            ev.window,
                            atoms.xdnd_selection,
                            atoms.uri_list,
                            atoms.xdnd_prop,
                            ts,
                        );
                    }
                }
                X11Event::SelectionRequest(ev) => {
                    let mut property = ev.property;
                    if property == x11rb::NONE {
                        property = ev.target;
                    }
                    if ev.target == atoms.targets {
                        let mut supported: Vec<u32> =
                            vec![atoms.targets, atoms.utf8_string, AtomEnum::STRING.into()];
                        if ev.selection == atoms.clipboard && clipboard_has_files {
                            supported.push(atoms.uri_list);
                            supported.push(atoms.gnome_copied_files);
                        }
                        let _ = conn.change_property32(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            AtomEnum::ATOM,
                            &supported,
                        );
                    } else if ev.selection == atoms.xdnd_selection && ev.target == atoms.uri_list {
                        let _ = conn.change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            ev.target,
                            xdnd_outbound_uri_list.as_bytes(),
                        );
                    } else if ev.selection == atoms.clipboard
                        && clipboard_has_files
                        && ev.target == atoms.uri_list
                    {
                        let _ = conn.change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            ev.target,
                            clipboard_owned_uri_list.as_bytes(),
                        );
                    } else if ev.selection == atoms.clipboard
                        && clipboard_has_files
                        && ev.target == atoms.gnome_copied_files
                    {
                        let _ = conn.change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            ev.target,
                            clipboard_owned_gnome_files.as_bytes(),
                        );
                    } else if ev.target == atoms.utf8_string
                        || ev.target == Atom::from(AtomEnum::STRING)
                    {
                        let _ = conn.change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            ev.target,
                            clipboard_owned_text.as_bytes(),
                        );
                    } else {
                        property = x11rb::NONE;
                    }
                    let notify = SelectionNotifyEvent {
                        response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: ev.time,
                        requestor: ev.requestor,
                        selection: ev.selection,
                        target: ev.target,
                        property,
                    };
                    let _ = conn.send_event(false, ev.requestor, EventMask::NO_EVENT, notify);
                }
                X11Event::SelectionNotify(ev) => {
                    if ev.property == atoms.xdnd_prop {
                        if let Ok(cookie) = conn.get_property(
                            true,
                            ev.requestor,
                            atoms.xdnd_prop,
                            AtomEnum::ANY,
                            0,
                            1024 * 256,
                        ) {
                            if let Ok(reply) = cookie.reply() {
                                if let Ok(raw) = String::from_utf8(reply.value) {
                                    let paths = crate::linux_dnd::decode_uri_list(&raw);
                                    if let Some(entry) =
                                        windows.iter_mut().find(|w| w.xid == ev.requestor)
                                    {
                                        for path in paths {
                                            entry
                                                .pending_events
                                                .push(PlatformEvent::FileDrop(path));
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(source) = xdnd_source_window.take() {
                            let finished = ClientMessageEvent {
                                response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                                format: 32,
                                sequence: 0,
                                window: source,
                                type_: atoms.xdnd_finished,
                                data: ClientMessageData::from([
                                    ev.requestor,
                                    1u32,
                                    atoms.xdnd_action_copy,
                                    0,
                                    0,
                                ]),
                            };
                            let _ = conn.send_event(false, source, EventMask::NO_EVENT, finished);
                        }
                    } else if ev.property == atoms.clipboard_prop {
                        if let Ok(cookie) = conn.get_property(
                            true,
                            ev.requestor,
                            atoms.clipboard_prop,
                            AtomEnum::ANY,
                            0,
                            1024 * 256,
                        ) {
                            if let Ok(reply) = cookie.reply() {
                                if let Ok(text) = String::from_utf8(reply.value) {
                                    if let Ok(mut guard) = clipboard_cache.lock() {
                                        *guard = Some(text);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some((rxid, target)) = pending_resize_cfg {
            let _ = conn.configure_window(
                rxid,
                &ConfigureWindowAux::new()
                    .x(target.x)
                    .y(target.y)
                    .width(target.width.max(1))
                    .height(target.height.max(1)),
            );
            let _ = conn.flush();
        }

        // 2. 消费应用侧控制命令
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                X11Command::SetTitle { window, title } => {
                    set_window_title(&conn, &atoms, window, &title);
                }
                X11Command::SetMinimized { window } => {
                    let ev = ClientMessageEvent {
                        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                        format: 32,
                        sequence: 0,
                        window,
                        type_: atoms.wm_change_state,
                        data: ClientMessageData::from([3u32, 0, 0, 0, 0]),
                    };
                    let _ = conn.send_event(
                        false,
                        root,
                        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                        ev,
                    );
                }
                X11Command::SetMaximized { window, maximized } => {
                    let action = u32::from(maximized);
                    let ev = ClientMessageEvent {
                        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                        format: 32,
                        sequence: 0,
                        window,
                        type_: atoms.net_wm_state,
                        data: ClientMessageData::from([
                            action,
                            atoms.net_wm_state_maximized_horz,
                            atoms.net_wm_state_maximized_vert,
                            1,
                            0,
                        ]),
                    };
                    let _ = conn.send_event(
                        false,
                        root,
                        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                        ev,
                    );
                }
                X11Command::SetVisible { window, visible } => {
                    if visible {
                        let _ = conn.map_window(window);
                    } else {
                        let _ = conn.unmap_window(window);
                    }
                }
                X11Command::SetVisibleNoActivate { window, visible } => {
                    if visible {
                        let _ = conn.map_window(window);
                    } else {
                        let _ = conn.unmap_window(window);
                    }
                }
                X11Command::SetAlwaysOnTop { window, topmost } => {
                    let action = u32::from(topmost);
                    let ev = ClientMessageEvent {
                        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
                        format: 32,
                        sequence: 0,
                        window,
                        type_: atoms.net_wm_state,
                        data: ClientMessageData::from([action, atoms.net_wm_state_above, 0, 1, 0]),
                    };
                    let _ = conn.send_event(
                        false,
                        root,
                        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                        ev,
                    );
                }
                X11Command::SetClickThrough {
                    window,
                    click_through,
                } => {
                    set_x11_click_through(&conn, window, click_through);
                }
                X11Command::SetOuterPosition { window, x, y } => {
                    let _ = conn.configure_window(window, &ConfigureWindowAux::new().x(x).y(y));
                }
                X11Command::SetOuterRect { window, rect } => {
                    let _ = conn.configure_window(
                        window,
                        &ConfigureWindowAux::new()
                            .x(rect.x)
                            .y(rect.y)
                            .width(rect.width.max(1))
                            .height(rect.height.max(1)),
                    );
                }
                X11Command::SetCursor { window, cursor } => {
                    cursor_cache.apply(&conn, window, cursor);
                }
                X11Command::DragWindow { window } => {
                    let (rx, ry) = windows
                        .iter()
                        .find(|w| w.xid == window)
                        .map_or((0, 0), |w| w.last_pointer_root);
                    send_net_wm_moveresize(&conn, root, &atoms, window, rx, ry, 8);
                }
                X11Command::DragResizeWindow { window, direction } => {
                    if let Some(entry) = windows.iter().find(|w| w.xid == window) {
                        let (rx, ry) = entry.last_pointer_root;
                        let start_rect = entry.native_window.client_rect_on_screen();
                        let cid =
                            cursor_cache.cursor_id(&conn, cursor_for_resize_direction(direction));
                        let _ = conn.grab_pointer(
                            false,
                            entry.xid,
                            EventMask::POINTER_MOTION | EventMask::BUTTON_RELEASE,
                            GrabMode::ASYNC,
                            GrabMode::ASYNC,
                            x11rb::NONE,
                            cid,
                            x11rb::CURRENT_TIME,
                        );
                        let _ = conn.flush();
                        active_resize = Some(X11ActiveResize {
                            xid: entry.xid,
                            dir: direction,
                            start_root_x: rx,
                            start_root_y: ry,
                            start_rect,
                            min_w: entry.min_width,
                            min_h: entry.min_height,
                        });
                    }
                }
                X11Command::RequestClose { window } => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == window) {
                        entry.pending_events.push(PlatformEvent::CloseRequested);
                    }
                }
                X11Command::RequestRedraw { window } => {
                    if let Some(entry) = windows.iter_mut().find(|w| w.xid == window) {
                        entry.shared.redraw_requested.store(true, Ordering::Release);
                    }
                }
                X11Command::ClipboardWrite { text } => {
                    clipboard_owned_text = text;
                    clipboard_has_files = false;
                    if let Some(first) = windows.first() {
                        let _ = conn.set_selection_owner(
                            first.xid,
                            atoms.clipboard,
                            x11rb::CURRENT_TIME,
                        );
                    }
                }
                X11Command::ClipboardWriteFiles {
                    uri_list,
                    gnome_copied_files,
                } => {
                    clipboard_owned_uri_list = uri_list;
                    clipboard_owned_gnome_files = gnome_copied_files;
                    clipboard_has_files = true;
                    if let Some(first) = windows.first() {
                        let _ = conn.set_selection_owner(
                            first.xid,
                            atoms.clipboard,
                            x11rb::CURRENT_TIME,
                        );
                    }
                }
                X11Command::ThemeChanged { dark } => {
                    for entry in &mut windows {
                        entry
                            .pending_events
                            .push(PlatformEvent::ThemeChanged { dark });
                    }
                }
                X11Command::StartOutboundFileDrag { uri_list } => {
                    xdnd_outbound_uri_list = uri_list;
                    if let Some(first) = windows.first() {
                        let _ = conn.set_selection_owner(
                            first.xid,
                            atoms.xdnd_selection,
                            x11rb::CURRENT_TIME,
                        );
                    }
                }
            }
        }

        // 3. 派发顶层窗口事件；关闭请求交由应用层拦截或通过 `take_window_close_requests` 放行。
        for entry in windows.iter_mut() {
            let events = std::mem::take(&mut entry.pending_events);
            let ev_now = Instant::now();
            for ev in events {
                let derived = apply(&mut entry.state, &ev, ev_now);
                for out_ev in derived {
                    if app.event(&mut entry.state, &entry.native_window, out_ev) == Flow::Exit {
                        exit_requested = true;
                        break;
                    }
                }
                if exit_requested {
                    break;
                }
            }
            if exit_requested {
                break;
            }
        }
        if exit_requested {
            break;
        }

        // 4. 派发越界弹出窗事件
        let pids: Vec<u64> = popups.keys().copied().collect();
        for pid in pids {
            if let Some(pentry) = popups.get_mut(&pid) {
                let events = std::mem::take(&mut pentry.pending_events);
                let ev_now = Instant::now();
                for ev in events {
                    let derived = apply(&mut pentry.state, &ev, ev_now);
                    for out_ev in derived {
                        if app.event(&mut pentry.state, &pentry.native_window, out_ev) == Flow::Exit
                        {
                            exit_requested = true;
                            break;
                        }
                    }
                    if exit_requested {
                        break;
                    }
                }
            }
        }
        if exit_requested {
            break;
        }

        // 5. 处理应用请求的新副窗与关闭请求
        while let Some(new_cfg) = app.take_window_request() {
            if let Ok(entry) = create_toplevel(&new_cfg) {
                windows.push(entry);
                if let Some(last) = windows.last_mut() {
                    app.window_created(&mut last.state, &last.native_window);
                }
            }
        }
        for close_id in app.take_window_close_requests() {
            if let Some(pos) = windows.iter().position(|w| u64::from(w.xid) == close_id.0) {
                let target_xid = windows[pos].xid;
                let owned_pids: Vec<u64> = popups
                    .iter()
                    .filter(|(_, p)| p.owner_xid == target_xid)
                    .map(|(&pid, _)| pid)
                    .collect();
                for pid in owned_pids {
                    if let Some(pentry) = popups.remove(&pid) {
                        pentry.shared.destroyed.store(true, Ordering::Release);
                        app.window_closing(&pentry.native_window);
                        let _ = conn.destroy_window(pentry.xid);
                    }
                }
                let entry = windows.remove(pos);
                entry.shared.destroyed.store(true, Ordering::Release);
                cursor_cache.remove_window(entry.xid);
                app.window_closing(&entry.native_window);
                let _ = conn.destroy_window(entry.xid);
            }
        }

        // 6. 同步 X11 无激活越界弹出窗（XRandR 工作区夹紧 + ToolTip 鼠标穿透）
        for (pid, pcfg) in app.take_popup_sync_requests() {
            let owner_xid = windows
                .iter()
                .find(|w| u64::from(w.xid) == pcfg.owner.0)
                .or_else(|| windows.first())
                .map_or(root, |w| w.xid);
            let work = query_work_area_for_point(
                &conn,
                root,
                root_w,
                root_h,
                &atoms,
                pcfg.screen_rect.x,
                pcfg.screen_rect.y,
            );
            let placed = place_x11_popup_in_work_area(pcfg.anchor_rect, pcfg.screen_rect, work);
            if let Some(existing) = popups.get_mut(&pid) {
                let _ = conn.configure_window(
                    existing.xid,
                    &ConfigureWindowAux::new()
                        .x(placed.x)
                        .y(placed.y)
                        .width(placed.width.max(1))
                        .height(placed.height.max(1)),
                );
                if let Ok(mut org) = existing.shared.screen_origin.lock() {
                    *org = (placed.x, placed.y);
                }
                if let Ok(mut sz) = existing.shared.inner_size.lock() {
                    *sz = PhysicalSize::new(placed.width.max(1), placed.height.max(1));
                }
            } else if let Ok(pxid) = conn.generate_id() {
                let aux = CreateWindowAux::new().override_redirect(1).event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION,
                );
                if conn
                    .create_window(
                        COPY_DEPTH_FROM_PARENT,
                        pxid,
                        root,
                        placed.x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        placed.y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        placed.width.clamp(1, u32::from(u16::MAX)) as u16,
                        placed.height.clamp(1, u32::from(u16::MAX)) as u16,
                        0,
                        WindowClass::INPUT_OUTPUT,
                        0,
                        &aux,
                    )
                    .is_ok()
                {
                    if pcfg.click_through {
                        if let Ok(region_id) = conn.generate_id() {
                            let empty: [Rectangle; 0] = [];
                            if conn.xfixes_create_region(region_id, &empty).is_ok() {
                                let _ = conn.xfixes_set_window_shape_region(
                                    pxid,
                                    x11rb::protocol::shape::SK::INPUT,
                                    0,
                                    0,
                                    region_id,
                                );
                                let _ = conn.xfixes_destroy_region(region_id);
                            }
                        }
                    }
                    let _ = conn.map_window(pxid);
                    let pshared = Arc::new(X11WindowShared {
                        xid: pxid,
                        screen_num: screen_num as i32,
                        xcb_conn_ptr,
                        decorated: false,
                        popup_id: Some(pid),
                        inner_size: Mutex::new(PhysicalSize::new(
                            placed.width.max(1),
                            placed.height.max(1),
                        )),
                        screen_origin: Mutex::new((placed.x, placed.y)),
                        scale_bits: AtomicU64::new(1.0_f64.to_bits()),
                        maximized: AtomicBool::new(false),
                        always_on_top: AtomicBool::new(false),
                        visible: AtomicBool::new(true),
                        destroyed: AtomicBool::new(false),
                        redraw_requested: AtomicBool::new(true),
                        cursor: AtomicU32::new(CursorIcon::Default.to_u32()),
                        frame_metrics: Mutex::new(None),
                        cmd_tx: Mutex::new(cmd_tx.clone()),
                        wake_signal: wake_signal.clone(),
                    });
                    let pnative = NativeWindow::from_x11(X11WindowHandle {
                        shared: Arc::clone(&pshared),
                    });
                    let pstate = PlatformState::new(
                        &WindowConfig::default()
                            .with_size(placed.width.max(1), placed.height.max(1)),
                        Extent::new(placed.width.max(1), placed.height.max(1)),
                        PresentWaitPolicy::from_extensions(&[]),
                    );
                    let mut entry = X11PopupEntry {
                        xid: pxid,
                        owner_xid,
                        native_window: pnative,
                        shared: pshared,
                        state: pstate,
                        pending_events: Vec::new(),
                    };
                    app.window_created(&mut entry.state, &entry.native_window);
                    popups.insert(pid, entry);
                }
            }
        }
        for close_pid in app.take_popup_close_requests() {
            if let Some(pentry) = popups.remove(&close_pid) {
                pentry.shared.destroyed.store(true, Ordering::Release);
                app.window_closing(&pentry.native_window);
                let _ = conn.destroy_window(pentry.xid);
            }
        }

        // 7. 出帧与休眠等待
        let frame_now = Instant::now();
        let mut min_wait: Option<Duration> = None;
        for entry in &mut windows {
            let redraw = entry.shared.redraw_requested.swap(false, Ordering::AcqRel);
            let pacer_due = entry.state.pacer.should_render(frame_now);
            if redraw || pacer_due {
                let rect = entry.native_window.client_rect_on_screen();
                entry
                    .a11y
                    .set_window_bounds(crate::a11y_linux::A11yWindowBounds::new(
                        f64::from(rect.x),
                        f64::from(rect.y),
                        f64::from(rect.width),
                        f64::from(rect.height),
                    ));
                if crate::a11y_platform::pump_platform_a11y(
                    &mut app,
                    &mut entry.a11y,
                    &mut entry.state,
                    &entry.native_window,
                ) == Flow::Exit
                {
                    exit_requested = true;
                    break;
                }
                if app.frame(&mut entry.state, &entry.native_window) == Flow::Exit {
                    exit_requested = true;
                    break;
                }
                entry.state.pacer.on_present(Instant::now());
            } else if let Some(next_t) = entry.state.pacer.next_wake(frame_now) {
                let wait = next_t.saturating_duration_since(frame_now);
                min_wait = Some(min_wait.map_or(wait, |cur| cur.min(wait)));
            }
        }
        if exit_requested {
            break;
        }
        for pentry in popups.values_mut() {
            let _ = app.frame(&mut pentry.state, &pentry.native_window);
        }
        if let Some(app_wake) = app.next_wake() {
            let wait = app_wake.saturating_duration_since(Instant::now());
            min_wait = Some(min_wait.map_or(wait, |cur| cur.min(wait)));
        }
        let _ = conn.flush();
        let has_pending = windows.iter().any(|w| !w.pending_events.is_empty());
        if !has_pending {
            wait_x11_or_deadline(x11_fd, wake_pipe.read_fd, min_wait);
        }
    }

    // 严格遵守 Vulkan 表面销毁顺序：先通知应用销毁所有 `VkSurfaceKHR`，再销毁 X11 窗口与连接。
    for (_, pentry) in popups.drain() {
        pentry.shared.destroyed.store(true, Ordering::Release);
        app.window_closing(&pentry.native_window);
        let _ = conn.destroy_window(pentry.xid);
    }
    for entry in windows.drain(..) {
        entry.shared.destroyed.store(true, Ordering::Release);
        app.window_closing(&entry.native_window);
        let _ = conn.destroy_window(entry.xid);
    }
    cursor_cache.destroy_all(&conn);
    let _ = conn.flush();
    Ok(())
}

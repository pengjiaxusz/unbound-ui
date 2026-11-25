//! 平台层。
//!
//! 所有与 Windows 帧呈现相关的平台逻辑均集中于此：
//!
//! * **实时缩放。** 拖拽窗口边缘会产生大量尺寸消息。交换链**立即**跟随
//!   窗口尺寸：每次上报的新尺寸都会被立即提交，从而按新分辨率绘制帧，
//!   合成器无需拉伸上一帧。对于偏好减少重建次数的宿主，仍保留了延迟模式。
//!   参见 [`resize::ResizeTracker`]。
//! * **无撕裂、最低延迟呈现。** `VK_EXT_swapchain_maintenance1` 提供
//!   `FIFO_LATEST_READY`，而 `VK_KHR_present_wait` 允许 CPU 阻塞直至图像
//!   真正扫描输出，从而将输入到光子延迟控制在单帧之内。
//!   参见 [`pacing::PresentWaitPolicy`]。
//! * **实测延迟。** [`latency::LatencyTracker`] 对输入与呈现打时间戳，
//!   以实测验证 8.3 ms 红线而非凭空假设。
//!
//! 本 crate 拆分为**策略层**（纯逻辑，完整单元测试）与**原生 Win32 后端层**，
//! 从而使核心行为可在无显示器的机器上完成测试。

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod a11y_bridge;
pub mod a11y_linux;
pub mod a11y_platform;
pub mod a11y_win32;
pub mod backend;
pub mod clipboard;
pub mod dialogs;
pub mod floating;
pub mod latency;
pub mod linux_backend;
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub mod linux_dbus;
pub mod linux_dnd;
pub mod native_window;
pub mod ole_drag;
pub mod overlay_host;
pub mod pacing;
pub mod popup_host;
pub mod portal_logic;
pub mod reduced_motion;
pub mod resize;
pub mod screen_capture;
pub mod system_accent;
pub mod tray;
#[cfg(windows)]
pub mod uia_win32;
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub mod wayland_backend;
pub mod wayland_logic;
pub mod win32_frame;
pub mod win32_window;
pub mod window;
pub mod windows;
pub mod windows_build;
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
pub mod x11_backend;

pub use a11y_bridge::{action_request, A11yBridge, A11yRequest, A11yTransport};
pub use a11y_linux::{A11yWindowBounds, LinuxA11y};
pub use a11y_platform::PlatformA11y;
pub use a11y_win32::Win32A11y;
pub use backend::{Flow, PlatformApp, PlatformWindow};
pub use clipboard::{
    decode_gnome_copied_files, encode_gnome_copied_files, is_headless_clipboard_loopback_enabled,
    set_headless_clipboard_loopback, ClipboardError, ClipboardFileDropEffect, ClipboardProvider,
    MemoryClipboard, SystemClipboard, GNOME_COPIED_FILES_MIME, MAX_CLIPBOARD_BYTES,
};
pub use dialogs::{
    open_file, open_file_async, open_files, pick_folder, pick_folder_async, save_file,
    save_file_async, set_mock_dialog_path, FileDialogConfig,
};
pub use floating::{
    all_monitors, clamp_rect_to_work_area, place_anchor_rect_avoid, place_cursor_monitor_avoid,
    resolve_floating_placement, restore_from_relative_anchor, solve_edge_snap, work_area_for_point,
    EdgeSnapResult, FloatingPlacementStrategy, FloatingWindowCapabilities, FloatingWindowPolicy,
    MonitorWorkArea, RelativeMonitorAnchor, SnapEdge, WindowRole,
};
pub use latency::{LatencyTracker, BUDGET as LATENCY_BUDGET};
pub use linux_backend::{
    active_linux_backend, parse_linux_backend_env, place_x11_popup_in_work_area,
    select_linux_backend_order, set_active_linux_backend, LinuxBackendKind, LinuxBackendPreference,
    LinuxDisplayEnv,
};
pub use linux_dnd::{
    decode_uri_list, dispatch_linux_outbound_file_drag, encode_uri_list,
    register_linux_outbound_drag, LinuxOutboundDragBackend, TabTearDragState, TabTearDropOutcome,
    TabTearPayload, UNBOUND_TAB_TEAR_MIME, URI_LIST_MIME,
};
pub use native_window::NativeWindow;
pub use ole_drag::{
    begin_outbound_file_drag, build_cf_hdrop_payload, OutboundFileDragOutcome,
    OutboundFileDragRequest,
};
pub use pacing::{
    renderer_present_config, FramePacer, PresentSync, PresentWaitPolicy, RenderThrottlePolicy,
    RenderThrottleState, IDLE_FPS_LIMIT, MIN_FPS_LIMIT,
};
pub use popup_host::{
    clamp_popup_to_work_area, PopupHostEntry, PopupHostRegistry, PopupSyncAction, PopupWindowConfig,
};
pub use portal_logic::{
    convert_dialog_filters, decode_file_uri, parse_portal_accent_color, parse_portal_color_scheme,
    percent_decode_bytes, LinuxAppearanceSnapshot, PortalColorScheme, PortalFileFilter,
};
pub use reduced_motion::{
    parse_gnome_enable_animations, parse_reduced_motion_env, prefers_reduced_motion,
};
pub use resize::{Extent, ResizeAction, ResizeTracker};
pub use screen_capture::{
    capture_screen_region, clamp_to_screen, normalize_x11_bgrx_to_bgra, screen_rect_uv,
    CaptureError, CapturedScreen,
};
pub use system_accent::{
    decode_abgr_accent, decode_argb_colorization, read_system_accent_color, system_accent_color,
    DEFAULT_SYSTEM_ACCENT_SRGB,
};
pub use tray::{
    active_tray_menu_items, is_tray_heartbeat_stalled, notification_urgency, poll_tray_events,
    push_tray_event_for_test, reattach_tray_icon, set_taskbar_progress,
    set_tray_heartbeat_stalled_for_test, set_tray_icon, set_tray_menu, set_tray_rescue_menu,
    show_tray_balloon, update_tray_heartbeat, TaskbarProgressState, TrayEvent, TrayIconState,
    TrayMenuItem, TrayNotificationSeverity, WM_APP_TRAY,
};
pub use wayland_logic::{
    compute_wayland_ime_cursor_rect, compute_wayland_popup_placement, csd_resize_direction_at,
    cursor_for_resize_direction, primary_output_metrics, set_primary_output_metrics,
    translate_evdev_key, wayland_cursor_name, KeyRepeatState, PrimaryOutputMetrics,
    WaylandFrameThrottle, WaylandImeCursorRect, WaylandImeProtocolAction, WaylandImeState,
    WaylandPopupPlacement,
};
pub use win32_frame::{
    accent_policy_export_available, clear_window_accent_tint, install_custom_frame,
    probe_immersive_backdrop, set_window_accent_tint, set_window_backdrop, set_window_dark_mode,
    FrameHitMetrics, WindowBackdropMaterial,
};
pub use win32_window::{
    CursorIcon, PhysicalPosition, PhysicalSize, ResizeDirection, Window, WindowId as NativeWindowId,
};
pub use window::{
    apply, enumerate_monitor_work_areas, resolve_window_placement, ButtonState, DisplayWorkArea,
    ImeEvent, ImeState, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey,
    PlatformState, ScreenRect, SurfaceAlpha, WindowConfig, WindowPlacement, WindowPosition,
};
pub use windows::{WindowId, WindowRegistry, WindowSlot};
pub use windows_build::windows_build;

pub use overlay_host::{
    is_rect_fully_covered, place_popup_in_work_area, should_use_popup_host, FileDragDropBridge,
    FileDragEffect, HitTestPassThroughPolicy, MultiWindowHostPool, OcclusionMonitor,
    OverlayHitDisposition, OverlayWindowSpec, ScreenSleepInhibitor, TargetWindowFollower,
};

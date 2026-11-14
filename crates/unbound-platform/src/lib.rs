//! 平台层（`docs` §2.3）。
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
pub mod a11y_win32;
pub mod backend;
pub mod clipboard;
pub mod dialogs;
pub mod latency;
pub mod pacing;
pub mod resize;
pub mod tray;
pub mod win32_frame;
pub mod win32_window;
pub mod window;
pub mod windows;

pub use a11y_bridge::{action_request, A11yBridge, A11yRequest, A11yTransport};
pub use a11y_win32::Win32A11y;
pub use backend::{Flow, PlatformApp, PlatformWindow};
pub use clipboard::{
    ClipboardError, ClipboardProvider, MemoryClipboard, SystemClipboard, MAX_CLIPBOARD_BYTES,
};
pub use dialogs::{
    open_file, open_file_async, open_files, save_file, save_file_async, FileDialogConfig,
};
pub use latency::{LatencyTracker, BUDGET as LATENCY_BUDGET};
pub use pacing::{
    renderer_present_config, FramePacer, PresentSync, PresentWaitPolicy, MIN_FPS_LIMIT,
};
pub use resize::{Extent, ResizeAction, ResizeTracker};
pub use tray::{
    set_taskbar_progress, set_tray_icon, show_tray_balloon, TaskbarProgressState,
    TrayNotificationSeverity,
};
pub use win32_frame::{install_custom_frame, set_window_dark_mode, FrameHitMetrics};
pub use win32_window::{
    CursorIcon, PhysicalPosition, PhysicalSize, ResizeDirection, Window, WindowId as NativeWindowId,
};
pub use window::{
    apply, ButtonState, ImeEvent, ImeState, Modifiers, MouseButton, PlatformError, PlatformEvent,
    PlatformKey, PlatformState, WindowConfig,
};
pub use windows::{WindowId, WindowRegistry, WindowSlot};

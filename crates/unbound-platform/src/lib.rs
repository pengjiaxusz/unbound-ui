//! The platform layer (`docs` §2.3).
//!
//! Everything Windows-specific about presenting a frame lives here:
//!
//! * **Real-time resize.** A window-edge drag produces a burst of size
//!   messages. The swapchain follows the window size *immediately*: every
//!   reported size is committed, so the frame is drawn at the new
//!   resolution and the compositor never has to stretch the previous one.
//!   The deferred mode is still available for a host that prefers fewer
//!   rebuilds. See [`resize::ResizeTracker`].
//! * **Tear-free, lowest-latency presentation.** `VK_EXT_swapchain_maintenance1`
//!   provides `FIFO_LATEST_READY`, and `VK_KHR_present_wait` lets the CPU
//!   block until the image is actually scanned out, which is what keeps the
//!   input-to-photon delay at one frame. See [`pacing::PresentWaitPolicy`].
//! * **Measured latency.** [`latency::LatencyTracker`] timestamps the input
//!   and the present so the 8.3 ms red line can be checked instead of
//!   assumed.
//!
//! The crate is split into a *policy* half (pure logic, fully unit-tested)
//! and a *backend* half (`winit`), so the interesting behaviour can be tested
//! on a machine without a display.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod a11y_bridge;
pub mod a11y_winit;
pub mod backend;
pub mod clipboard;
pub mod dialogs;
pub mod latency;
pub mod pacing;
pub mod resize;
pub mod tray;
pub mod win32_frame;
pub mod window;
pub mod windows;

pub use a11y_bridge::{action_request, A11yBridge, A11yRequest, A11yTransport};
pub use a11y_winit::WinitA11y;
pub use backend::{Flow, PlatformWindow};
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
pub use window::{
    apply, translate_ime_event, translate_key, translate_window_event, ButtonState, ImeEvent,
    ImeState, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey, PlatformState,
    WindowConfig,
};
pub use windows::{WindowId, WindowRegistry, WindowSlot};

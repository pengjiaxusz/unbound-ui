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

pub mod backend;
pub mod latency;
pub mod pacing;
pub mod resize;
pub mod window;

pub use backend::{Flow, PlatformWindow};
pub use latency::{LatencyTracker, BUDGET as LATENCY_BUDGET};
pub use pacing::{
    renderer_present_config, FramePacer, PresentSync, PresentWaitPolicy, MIN_FPS_LIMIT,
};
pub use resize::{Extent, ResizeAction, ResizeTracker};
pub use window::{
    apply, translate_ime_event, translate_key, translate_window_event, ButtonState, ImeEvent,
    ImeState, Modifiers, MouseButton, PlatformError, PlatformEvent, PlatformKey, PlatformState,
    WindowConfig,
};

//! Frame pacing and present-wait policy (`docs` §2.3).
//!
//! Two independent mechanisms meet here:
//!
//! * **`VK_KHR_present_wait`** lets the CPU block until the image it just
//!   submitted is *actually* being scanned out. Sampling the input queue
//!   right after that point is what keeps the input-to-photon delay at one
//!   frame instead of three.
//! * **`VK_EXT_swapchain_maintenance1`** provides
//!   `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`: vsync without queueing stale
//!   frames, so a "present" really is the newest image.
//!
//! The pacer itself is pure arithmetic: it decides when the next frame is
//! due and skips frames when nothing changed, which is what keeps the idle
//! CPU and GPU load at zero.

use std::time::{Duration, Instant};

/// 120 Hz, the refresh rate the acceptance table targets.
pub const REFRESH_120: Duration = Duration::from_nanos(8_333_333);

/// 60 Hz.
pub const REFRESH_60: Duration = Duration::from_nanos(16_666_667);

/// How the CPU synchronises with the display.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentSync {
    /// Block on `vkWaitForPresentKHR` until the image is scanned out.
    ///
    /// This is the low-latency path: the input queue is sampled right after a
    /// real present, so the reaction is picked up by the very next frame.
    WaitForPresent,
    /// Present and continue; the driver decides when the image is shown.
    ///
    /// Used when `VK_KHR_present_wait` is unavailable.
    Immediate,
    /// The window is not visible: render nothing at all.
    Suspended,
}

/// The presentation policy derived from the device extensions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentWaitPolicy {
    /// `VK_KHR_present_wait` is available.
    pub has_present_wait: bool,
    /// `VK_EXT_swapchain_maintenance1` is available.
    pub has_maintenance1: bool,
}

impl PresentWaitPolicy {
    /// Build the policy from the extension list.
    #[must_use]
    pub fn from_extensions(extensions: &[String]) -> Self {
        let has = |name: &str| extensions.iter().any(|e| e == name);
        Self {
            has_present_wait: has("VK_KHR_present_wait"),
            has_maintenance1: has("VK_EXT_swapchain_maintenance1"),
        }
    }

    /// The synchronisation mode to use.
    ///
    /// `maintenance1` alone is enough for tear-free *latest-ready* presentation;
    /// `present_wait` additionally lets the CPU know when the image is on
    /// screen, which is what the input sampler needs.
    #[must_use]
    pub const fn sync(&self) -> PresentSync {
        // Only `present_wait` changes the CPU behaviour: `maintenance1` is
        // about the *present mode* the renderer picks, not about how the CPU
        // waits for the frame.
        if self.has_present_wait {
            PresentSync::WaitForPresent
        } else {
            PresentSync::Immediate
        }
    }

    /// True when the CPU may block on the present signal.
    #[must_use]
    pub const fn can_wait_for_present(&self) -> bool {
        self.has_present_wait
    }
}

/// The renderer's presentation policy for the same extension list.
///
/// The renderer owns the swapchain, so the *mode* is decided there; the
/// platform layer only decides how the CPU synchronises with it. Both must
/// agree, and this function is the single place that ties them together.
#[must_use]
pub fn renderer_present_config(
    extensions: &[String],
    allow_tearing: bool,
) -> unbound_gpu::PresentConfig {
    unbound_gpu::PresentConfig::from_capabilities(extensions, allow_tearing)
}

/// Decides when the next frame is due.
#[derive(Copy, Clone, Debug)]
pub struct FramePacer {
    /// Target frame interval.
    interval: Duration,
    /// When the last frame was presented.
    last_present: Option<Instant>,
    /// Whether the window is visible.
    visible: bool,
}

impl FramePacer {
    /// Create a pacer for a refresh rate.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_present: None,
            visible: true,
        }
    }

    /// Create a pacer for 120 Hz.
    #[must_use]
    pub fn at_120_hz() -> Self {
        Self::new(REFRESH_120)
    }

    /// Create a pacer for 60 Hz.
    #[must_use]
    pub fn at_60_hz() -> Self {
        Self::new(REFRESH_60)
    }

    /// The target interval.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }

    /// The window became visible or hidden.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            // A hidden window must not keep a deadline in the past.
            self.last_present = None;
        }
    }

    /// True when the window is visible.
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// Record that a frame was presented.
    pub fn on_present(&mut self, now: Instant) {
        self.last_present = Some(now);
    }

    /// The instant the next frame becomes due, if the pacer is running.
    #[must_use]
    pub fn next_deadline(&self, _now: Instant) -> Option<Instant> {
        if !self.visible {
            return None;
        }
        let last = self.last_present?;
        Some(last + self.interval)
    }

    /// True when a frame is due.
    #[must_use]
    pub fn should_render(&self, now: Instant) -> bool {
        if !self.visible {
            return false;
        }
        match self.last_present {
            // Nothing was ever presented: draw the first frame immediately.
            None => true,
            Some(last) => now.duration_since(last) >= self.interval,
        }
    }

    /// Skip the wait for the next frame: a frame is due right now.
    ///
    /// Used when the user interacts — the reaction must not wait for the
    /// regular cadence.
    pub fn request_immediate(&mut self, now: Instant) {
        self.last_present = Some(now - self.interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_wait_is_preferred_when_the_extension_exists() {
        let policy = PresentWaitPolicy {
            has_present_wait: true,
            has_maintenance1: true,
        };
        assert_eq!(policy.sync(), PresentSync::WaitForPresent);
        assert!(policy.can_wait_for_present());
    }

    #[test]
    fn the_policy_is_derived_from_the_extension_list() {
        let extensions = vec![
            "VK_KHR_swapchain".to_string(),
            "VK_KHR_present_wait".to_string(),
        ];
        let policy = PresentWaitPolicy::from_extensions(&extensions);
        assert!(policy.has_present_wait);
        assert!(!policy.has_maintenance1);
        assert_eq!(policy.sync(), PresentSync::WaitForPresent);
    }

    #[test]
    fn the_platform_and_the_renderer_agree_on_the_present_mode() {
        let extensions = vec![
            "VK_KHR_swapchain".to_string(),
            "VK_EXT_swapchain_maintenance1".to_string(),
            "VK_KHR_present_wait".to_string(),
        ];
        let policy = PresentWaitPolicy::from_extensions(&extensions);
        let config = renderer_present_config(&extensions, false);
        // Both halves must pick the same present mode, otherwise the CPU
        // waits for a present the GPU never performs.
        assert_eq!(policy.sync(), PresentSync::WaitForPresent);
        assert_eq!(config.mode, unbound_gpu::PresentMode::FifoLatestReady);
        assert!(config.present_fence);
    }

    #[test]
    fn without_any_extension_the_policy_falls_back_to_immediate() {
        let policy = PresentWaitPolicy::from_extensions(&[]);
        assert_eq!(policy.sync(), PresentSync::Immediate);
        assert!(!policy.can_wait_for_present());
    }

    #[test]
    fn the_pacer_waits_for_the_refresh_interval() {
        let mut pacer = FramePacer::at_120_hz();
        let now = Instant::now();
        assert!(pacer.should_render(now), "the first frame is due immediately");
        pacer.on_present(now);
        assert!(!pacer.should_render(now + Duration::from_millis(2)));
        assert!(pacer.should_render(now + Duration::from_millis(9)));
    }

    #[test]
    fn a_hidden_window_never_renders() {
        let mut pacer = FramePacer::at_120_hz();
        pacer.set_visible(false);
        assert!(!pacer.should_render(Instant::now()));
        assert!(pacer.next_deadline(Instant::now()).is_none());
    }

    #[test]
    fn an_interaction_bypasses_the_cadence() {
        let mut pacer = FramePacer::at_120_hz();
        let now = Instant::now();
        pacer.on_present(now);
        assert!(!pacer.should_render(now + Duration::from_millis(1)));
        pacer.request_immediate(now + Duration::from_millis(1));
        assert!(pacer.should_render(now + Duration::from_millis(1)));
    }

    #[test]
    fn the_next_deadline_follows_the_refresh_rate() {
        let mut pacer = FramePacer::new(REFRESH_60);
        let now = Instant::now();
        pacer.on_present(now);
        let deadline = pacer.next_deadline(now).expect("deadline");
        let delta = deadline.duration_since(now);
        assert!(
            delta > REFRESH_60 - Duration::from_micros(1)
                && delta < REFRESH_60 + Duration::from_micros(1),
            "deadline drifted by {delta:?}"
        );
    }
}

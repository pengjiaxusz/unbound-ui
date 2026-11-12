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

/// The smallest non-zero frame rate limit the engine accepts.
pub const MIN_FPS_LIMIT: u32 = 60;

/// How early the event loop's OS sleep is cut short before the exact frame
/// deadline.
///
/// `ControlFlow::WaitUntil` lands in `MsgWaitForMultipleObjectsEx` on Windows,
/// whose timeout is a whole number of milliseconds and is additionally delayed
/// by the system timer granularity (1–2 ms). Sleeping to the exact instant
/// therefore always oversleeps; waking this much earlier and spinning the
/// remaining gap out gets the cadence to the requested frame rate instead of
/// a few percent below it.
pub const SPIN_LEAD: Duration = Duration::from_micros(1200);

/// No spin is attempted for intervals at or below this: a sub-millisecond
/// busy-wait would cost more CPU than the jitter it removes.
const MIN_SPIN_INTERVAL: Duration = Duration::from_millis(2);

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
    /// Whether the application still needs frames.
    ///
    /// A static page sets this to `false`; the event loop then parks in
    /// `ControlFlow::Wait` instead of waking at the refresh rate.
    animating: bool,
}

impl FramePacer {
    /// Create a pacer for a refresh rate.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_present: None,
            visible: true,
            animating: true,
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

    /// Set the frame rate limit in FPS.
    ///
    /// * `0` means **unlimited**: the interval is zero, so every frame is due
    ///   and the event loop polls instead of sleeping.
    /// * Any other value is clamped to at least [`MIN_FPS_LIMIT`].
    pub fn set_fps_limit(&mut self, fps: u32) {
        self.interval = if fps == 0 {
            Duration::ZERO
        } else {
            let clamped = fps.max(MIN_FPS_LIMIT);
            Duration::from_nanos((1_000_000_000u64 / u64::from(clamped)).max(1))
        };
    }

    /// The current limit in FPS (`0` when unlimited).
    #[must_use]
    pub fn fps_limit(&self) -> u32 {
        if self.interval.is_zero() {
            return 0;
        }
        let nanos = self.interval.as_nanos().max(1) as u64;
        // Round to the nearest integer so setting 144 and reading it back does
        // not report 143.
        ((1_000_000_000u64 + nanos / 2) / nanos).max(u64::from(MIN_FPS_LIMIT)) as u32
    }

    /// Tell the pacer whether the application still needs frames.
    ///
    /// When this is `false` and the window is visible, [`Self::next_deadline`]
    /// returns `None` and the backend parks the loop until an event arrives.
    pub fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    /// Whether the application still needs frames.
    #[must_use]
    pub const fn animating(&self) -> bool {
        self.animating
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
    ///
    /// The next deadline is advanced by exactly one interval from the
    /// *previous deadline*, not from the wake-up instant. Resetting to `now`
    /// makes every oversleep permanent: at 6.944 ms (144 FPS) a 0.8 ms late
    /// wake-up and a 0.8 ms timer rounding add up frame after frame until the
    /// measured rate sits at 115 FPS instead of 144.
    ///
    /// A hitch longer than a whole interval restarts the phase instead of
    /// racing to catch up, so a drag or a stalled GPU does not produce a burst
    /// of back-to-back frames.
    pub fn on_present(&mut self, now: Instant) {
        self.last_present = Some(match self.last_present {
            Some(previous) if !self.interval.is_zero() => {
                let next = previous + self.interval;
                if now > next + self.interval {
                    now
                } else {
                    next
                }
            }
            _ => now,
        });
    }

    /// The instant the next frame becomes due, if the pacer is running.
    #[must_use]
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.visible || !self.animating {
            return None;
        }
        let last = self.last_present?;
        // An unlimited pacer is always due; a deadline in the past would also
        // work, but returning `now` keeps `ControlFlow::Poll` explicit.
        Some((last + self.interval).max(now))
    }

    /// The instant the event loop should wake up.
    ///
    /// This is [`Self::next_deadline`] moved [`SPIN_LEAD`] earlier so that
    /// [`Self::spin_to_deadline`] can close the remaining sub-millisecond gap
    /// instead of the OS timer deciding where the frame lands.
    #[must_use]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        let deadline = self.next_deadline(now)?;
        if self.interval <= MIN_SPIN_INTERVAL {
            return Some(deadline);
        }
        Some(deadline.checked_sub(SPIN_LEAD).unwrap_or(now))
    }

    /// Whether the exact deadline is close enough to be spun on.
    #[must_use]
    pub fn within_spin_lead(&self, now: Instant) -> bool {
        let Some(last) = self.last_present else {
            return false;
        };
        if self.interval <= MIN_SPIN_INTERVAL {
            return false;
        }
        let deadline = last + self.interval;
        now < deadline && deadline.saturating_duration_since(now) <= SPIN_LEAD
    }

    /// Busy-wait until the exact deadline and return the instant it was hit.
    ///
    /// Bounded by [`SPIN_LEAD`] because the caller only spins when
    /// [`Self::within_spin_lead`] is true.
    #[must_use]
    pub fn spin_to_deadline(&self) -> Instant {
        let Some(last) = self.last_present else {
            return Instant::now();
        };
        let deadline = last + self.interval;
        let mut now = Instant::now();
        while now < deadline {
            std::hint::spin_loop();
            now = Instant::now();
        }
        now
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
        // Both halves must also agree on the present-wait: the CPU waits for
        // the presentation only if the renderer can identify it.
        assert!(config.wait_for_present);
    }

    #[test]
    fn without_any_extension_the_policy_falls_back_to_immediate() {
        let policy = PresentWaitPolicy::from_extensions(&[]);
        assert_eq!(policy.sync(), PresentSync::Immediate);
        assert!(!policy.can_wait_for_present());
    }

    #[test]
    fn fps_limits_are_clamped_and_zero_means_unlimited() {
        let mut pacer = FramePacer::at_120_hz();
        pacer.set_fps_limit(30);
        assert_eq!(pacer.fps_limit(), MIN_FPS_LIMIT, "30 must clamp to 60");
        pacer.set_fps_limit(144);
        assert_eq!(pacer.fps_limit(), 144);
        pacer.set_fps_limit(0);
        assert_eq!(pacer.fps_limit(), 0);
        let now = Instant::now();
        pacer.on_present(now);
        assert!(pacer.should_render(now), "unlimited renders immediately");
        assert_eq!(pacer.next_deadline(now), Some(now));
    }

    #[test]
    fn the_pacer_waits_for_the_refresh_interval() {
        let mut pacer = FramePacer::at_120_hz();
        let now = Instant::now();
        assert!(
            pacer.should_render(now),
            "the first frame is due immediately"
        );
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

    #[test]
    fn a_late_wake_up_does_not_accumulate_into_a_frame_rate_deficit() {
        let mut pacer = FramePacer::new(REFRESH_120);
        let start = Instant::now();
        pacer.on_present(start);
        // Frame 2 woke up 0.5 ms late. The phase must stay on the grid, so the
        // next deadline is still exactly two intervals after the start.
        pacer.on_present(start + REFRESH_120 + Duration::from_micros(500));
        assert_eq!(
            pacer.next_deadline(start).expect("deadline"),
            start + REFRESH_120 * 2
        );
    }

    #[test]
    fn a_hitch_restarts_the_phase_instead_of_catching_up() {
        let mut pacer = FramePacer::new(REFRESH_120);
        let start = Instant::now();
        pacer.on_present(start);
        let late = start + REFRESH_120 * 3;
        pacer.on_present(late);
        assert_eq!(pacer.next_deadline(late), Some(late + REFRESH_120));
    }

    #[test]
    fn the_event_loop_wakes_early_enough_to_spin() {
        let mut pacer = FramePacer::new(REFRESH_60);
        let now = Instant::now();
        pacer.on_present(now);
        let deadline = pacer.next_deadline(now).expect("deadline");
        let wake = pacer.next_wake(now).expect("wake");
        assert_eq!(wake + SPIN_LEAD, deadline);
        assert!(!pacer.within_spin_lead(now));
        assert!(pacer.within_spin_lead(wake));
    }
}

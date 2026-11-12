//! Deferred swapchain recreation (`docs` §2.3).
//!
//! On Windows, dragging a window edge produces a burst of `WM_SIZE`
//! messages. Rebuilding the swapchain for every one of them is what produces
//! black borders and stretched frames: the new swapchain is not ready when the
//! compositor asks for it, and the old one has a different aspect ratio.
//!
//! The policy implemented here is the one the design asks for:
//!
//! * **Never block the message loop.** A resize only *records* the new size.
//! * **Render at the committed size** while the drag is in flight; the
//!   compositor scales the last frame to the new window size, so there is
//!   neither a black border nor a rebuild storm.
//! * **Recreate once the size settles.** The swapchain is rebuilt after the
//!   size has been stable for a short window, or immediately when the drag
//!   ends, so the final size is exact.

use std::time::{Duration, Instant};

/// How long the window size must stay unchanged before the swapchain is
/// rebuilt.
///
/// Two frames at 120 Hz is ~16 ms: long enough to swallow the event burst of
/// a drag, short enough that the user never notices a lag.
pub const DEFAULT_SETTLE: Duration = Duration::from_millis(16);

/// A physical pixel extent.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Extent {
    pub width: u32,
    pub height: u32,
}

impl Extent {
    /// Build an extent.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// True when the extent has no area and cannot back a swapchain.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Number of pixels.
    #[must_use]
    pub const fn pixels(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// What the platform layer should do about the window size.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeAction {
    /// Nothing to do.
    None,
    /// Recreate the swapchain at this extent.
    Recreate(Extent),
}

/// Tracks the window size and decides when the swapchain follows.
#[derive(Debug)]
pub struct ResizeTracker {
    /// The extent the swapchain is currently at.
    committed: Extent,
    /// The most recent extent the OS reported.
    pending: Extent,
    /// When `pending` first changed.
    pending_since: Option<Instant>,
    /// How long the size must be stable before it is committed.
    settle: Duration,
    /// Set while the user is dragging a window edge.
    dragging: bool,
    /// Set when the driver rejected the swapchain.
    ///
    /// The settle window exists to avoid rebuilding the chain while the user
    /// is still dragging. But once the driver has said `OUT_OF_DATE`, the
    /// chain is already unusable — waiting for the window to settle would
    /// leave the window showing the old resolution for another frame.
    forced: bool,
    /// Number of committed recreations, for diagnostics.
    recreations: u32,
    /// Number of resize messages that were absorbed instead of acted on.
    absorbed: u32,
}

impl ResizeTracker {
    /// Create a tracker for an initial extent.
    #[must_use]
    pub fn new(initial: Extent, settle: Duration) -> Self {
        Self {
            committed: initial,
            pending: initial,
            pending_since: None,
            settle,
            dragging: false,
            forced: false,
            recreations: 0,
            absorbed: 0,
        }
    }

    /// Create a tracker with the default settle window.
    #[must_use]
    pub fn with_default_settle(initial: Extent) -> Self {
        Self::new(initial, DEFAULT_SETTLE)
    }

    /// The extent the swapchain is at.
    #[must_use]
    pub const fn committed(&self) -> Extent {
        self.committed
    }

    /// The extent the window currently has.
    #[must_use]
    pub const fn pending(&self) -> Extent {
        self.pending
    }

    /// True while the swapchain does not match the window.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.pending != self.committed
    }

    /// Number of swapchain recreations.
    #[must_use]
    pub const fn recreations(&self) -> u32 {
        self.recreations
    }

    /// Number of resize messages that were absorbed.
    #[must_use]
    pub const fn absorbed(&self) -> u32 {
        self.absorbed
    }

    /// The user started dragging a window edge.
    pub fn on_drag_started(&mut self) {
        self.dragging = true;
    }

    /// The user finished dragging a window edge.
    ///
    /// The pending size is committed immediately: the user expects the window
    /// to match the content the moment they let go.
    pub fn on_drag_ended(&mut self) -> ResizeAction {
        self.dragging = false;
        self.commit()
    }

    /// Record a new window size.
    pub fn on_resized(&mut self, extent: Extent, now: Instant) -> ResizeAction {
        if extent.is_empty() {
            // A minimised window reports a zero extent. Rebuilding a swapchain
            // for it is illegal, so the size is ignored until it comes back.
            return ResizeAction::None;
        }
        if extent != self.pending {
            if extent != self.committed {
                self.absorbed += 1;
            }
            self.pending = extent;
            self.pending_since = Some(now);
        }
        if self.dragging {
            // During a drag the swapchain stays where it is.
            return ResizeAction::None;
        }
        // The size may already have settled: the settle timer keeps running
        // even when the OS repeats the same size.
        self.poll(now)
    }

    /// Mark the swapchain as out of date.
    ///
    /// The driver already rejected the old swapchain, so the settle window
    /// is bypassed: the next [`ResizeTracker::poll`] commits immediately.
    ///
    /// Without this the window keeps the old resolution until the user stops
    /// dragging for the settle window — and every frame in between reports
    /// `ERROR_OUT_OF_DATE_KHR` again.
    pub fn mark_out_of_date(&mut self, extent: Extent) {
        if extent.is_empty() {
            return;
        }
        if extent != self.pending {
            if extent != self.committed {
                self.absorbed += 1;
            }
            self.pending = extent;
        }
        self.forced = true;
        self.pending_since = Some(Instant::now());
    }

    /// Check whether the pending size has settled and must be committed.
    pub fn poll(&mut self, now: Instant) -> ResizeAction {
        if !self.is_dirty() {
            self.forced = false;
            return ResizeAction::None;
        }
        if self.dragging {
            return ResizeAction::None;
        }
        if self.forced {
            // The driver rejected the chain: rebuild now.
            return self.commit();
        }
        let Some(since) = self.pending_since else {
            return ResizeAction::None;
        };
        if now.duration_since(since) < self.settle {
            return ResizeAction::None;
        }
        self.commit()
    }

    /// Commit the pending extent.
    fn commit(&mut self) -> ResizeAction {
        if self.pending == self.committed {
            self.forced = false;
            return ResizeAction::None;
        }
        self.committed = self.pending;
        self.pending_since = None;
        self.forced = false;
        self.recreations += 1;
        ResizeAction::Recreate(self.committed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(w: u32, h: u32) -> Extent {
        Extent::new(w, h)
    }

    #[test]
    fn a_quiet_window_never_recreates_the_swapchain() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        assert_eq!(
            tracker.on_resized(extent(800, 600), now),
            ResizeAction::None
        );
        assert_eq!(
            tracker.poll(now + Duration::from_millis(100)),
            ResizeAction::None
        );
        assert_eq!(tracker.recreations(), 0);
    }

    #[test]
    fn a_resize_is_committed_after_the_size_settles() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        assert_eq!(
            tracker.on_resized(extent(900, 600), now),
            ResizeAction::None,
            "the first message must not trigger a rebuild"
        );
        assert!(tracker.is_dirty());
        assert_eq!(
            tracker.poll(now + Duration::from_millis(5)),
            ResizeAction::None,
            "the size has not settled yet"
        );
        assert_eq!(
            tracker.poll(now + Duration::from_millis(20)),
            ResizeAction::Recreate(extent(900, 600))
        );
        assert!(!tracker.is_dirty());
        assert_eq!(tracker.recreations(), 1);
    }

    #[test]
    fn a_drag_burst_is_absorbed_into_a_single_recreation() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        tracker.on_drag_started();
        for width in 801..=820 {
            assert_eq!(
                tracker.on_resized(extent(width, 600), now),
                ResizeAction::None,
                "no rebuild may happen while the edge is being dragged"
            );
        }
        assert_eq!(tracker.recreations(), 0);
        // The swapchain still has the old size: the compositor scales the last
        // frame, so there is no black border.
        assert_eq!(tracker.committed(), extent(800, 600));

        assert_eq!(
            tracker.on_drag_ended(),
            ResizeAction::Recreate(extent(820, 600)),
            "the final size is applied the moment the drag ends"
        );
        assert_eq!(tracker.recreations(), 1);
    }

    /// The driver has already rejected the swapchain: the settle window is
    /// pointless, the chain has to be rebuilt on the next poll.
    #[test]
    fn out_of_date_marks_the_resize_tracker_dirty_immediately() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        assert_eq!(
            tracker.on_resized(extent(900, 600), now),
            ResizeAction::None,
            "the first message must not trigger a rebuild"
        );
        // The driver rejects the chain before the size has settled.
        tracker.mark_out_of_date(extent(900, 600));
        assert_eq!(
            tracker.poll(now + Duration::from_millis(1)),
            ResizeAction::Recreate(extent(900, 600)),
            "an out-of-date swapchain is rebuilt without waiting for the settle window"
        );
        assert!(!tracker.is_dirty());
        assert_eq!(tracker.recreations(), 1);
    }

    /// A chain the driver rejected at a size the OS has not reported yet is
    /// still rebuilt — otherwise the window keeps the stale resolution.
    #[test]
    fn an_out_of_date_chain_is_rebuilt_even_without_a_resize_message() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        tracker.mark_out_of_date(extent(1024, 768));
        assert_eq!(tracker.poll(now), ResizeAction::Recreate(extent(1024, 768)));
    }

    #[test]
    fn a_minimised_window_never_produces_an_empty_swapchain() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        assert_eq!(tracker.on_resized(extent(0, 0), now), ResizeAction::None);
        assert_eq!(tracker.committed(), extent(800, 600));
        assert_eq!(
            tracker.poll(now + Duration::from_millis(50)),
            ResizeAction::None
        );
    }

    #[test]
    fn the_render_extent_never_changes_before_the_commit() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        tracker.on_resized(extent(1024, 768), now);
        // While the size is settling the renderer keeps using the old extent.
        assert_eq!(tracker.committed(), extent(800, 600));
        assert_eq!(tracker.pending(), extent(1024, 768));
        tracker.poll(now + Duration::from_millis(50));
        assert_eq!(tracker.committed(), extent(1024, 768));
    }

    #[test]
    fn repeated_identical_messages_do_not_restart_the_settle_timer() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        tracker.on_resized(extent(900, 600), now);
        // The same size again: the timer must not be restarted.
        tracker.on_resized(extent(900, 600), now + Duration::from_millis(5));
        assert_eq!(
            tracker.poll(now + Duration::from_millis(20)),
            ResizeAction::Recreate(extent(900, 600))
        );
    }
}

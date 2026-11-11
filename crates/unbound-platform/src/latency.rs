//! Input-to-photon accounting (`docs` §2.3, §10.2).
//!
//! The red line is 8.3 ms — one frame at 120 Hz. Measuring it honestly means
//! timestamping two different moments:
//!
//! * **input** — when the OS delivered the event to the application,
//! * **photon** — when the frame that contains the reaction left the GPU.
//!
//! The tracker keeps the *oldest* un-presented input, because a frame
//! reacts to every input that arrived before it was rendered: averaging
//! would hide a stall behind a fast frame.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The budget from the acceptance table: one frame at 120 Hz.
pub const BUDGET: Duration = Duration::from_micros(8_300);

/// Number of samples kept for the worst-case statistic.
const WINDOW: usize = 240;

/// Tracks the delay between an input event and the frame that showed it.
#[derive(Debug, Default)]
pub struct LatencyTracker {
    /// The oldest input that has not been presented yet.
    oldest_input: Option<Instant>,
    /// Recent samples, oldest first.
    samples: VecDeque<Duration>,
    /// The most recent sample.
    last: Option<Duration>,
    /// The worst sample in the window.
    worst: Option<Duration>,
    /// Number of presented frames that carried a reaction.
    presented: u64,
    /// Number of presented frames without any pending input.
    idle: u64,
}

impl LatencyTracker {
    /// Create an empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an input event.
    ///
    /// Only the oldest pending input matters; later ones ride along in the
    /// same frame.
    pub fn on_input(&mut self, now: Instant) {
        if self.oldest_input.is_none() {
            self.oldest_input = Some(now);
        }
    }

    /// Record that a frame was presented.
    pub fn on_present(&mut self, now: Instant) {
        let Some(input) = self.oldest_input.take() else {
            self.idle += 1;
            return;
        };
        let latency = now.saturating_duration_since(input);
        self.presented += 1;
        self.last = Some(latency);
        if self.worst.map_or(true, |w| latency > w) {
            self.worst = Some(latency);
        }
        self.samples.push_back(latency);
        while self.samples.len() > WINDOW {
            self.samples.pop_front();
        }
    }

    /// The most recent measured latency.
    #[must_use]
    pub const fn last(&self) -> Option<Duration> {
        self.last
    }

    /// The worst latency ever measured.
    #[must_use]
    pub const fn worst(&self) -> Option<Duration> {
        self.worst
    }

    /// The mean latency over the sample window.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let total: Duration = self.samples.iter().sum();
        Some(total / self.samples.len() as u32)
    }

    /// The 99th percentile over the sample window.
    #[must_use]
    pub fn p99(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<Duration> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let index = (sorted.len() * 99 / 100).min(sorted.len() - 1);
        Some(sorted[index])
    }

    /// Number of frames that carried a reaction.
    #[must_use]
    pub const fn presented(&self) -> u64 {
        self.presented
    }

    /// Number of frames presented without any pending input.
    #[must_use]
    pub const fn idle(&self) -> u64 {
        self.idle
    }

    /// True when the worst measured latency fits into the budget.
    #[must_use]
    pub fn within_budget(&self) -> bool {
        self.worst().is_some_and(|w| w <= BUDGET)
    }

    /// The budget.
    #[must_use]
    pub const fn budget() -> Duration {
        BUDGET
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_without_input_is_not_a_latency_sample() {
        let mut tracker = LatencyTracker::new();
        let now = Instant::now();
        tracker.on_present(now);
        assert_eq!(tracker.idle(), 1);
        assert_eq!(tracker.presented(), 0);
        assert!(tracker.last().is_none());
    }

    #[test]
    fn the_latency_is_measured_from_the_oldest_pending_input() {
        let mut tracker = LatencyTracker::new();
        let start = Instant::now();
        tracker.on_input(start);
        tracker.on_input(start + Duration::from_millis(2));
        tracker.on_present(start + Duration::from_millis(5));
        assert_eq!(tracker.last(), Some(Duration::from_millis(5)));
        assert_eq!(tracker.presented(), 1);
    }

    #[test]
    fn the_budget_is_one_frame_at_120_hertz() {
        assert_eq!(BUDGET.as_micros(), 8_300);
    }

    #[test]
    fn a_within_budget_pipeline_is_reported_as_such() {
        let mut tracker = LatencyTracker::new();
        let now = Instant::now();
        tracker.on_input(now);
        tracker.on_present(now + Duration::from_millis(3));
        assert!(tracker.within_budget());
        assert!(tracker.p99().is_some());
        assert!(tracker.mean().is_some());
    }

    #[test]
    fn a_stall_is_remembered_as_the_worst_case() {
        let mut tracker = LatencyTracker::new();
        let now = Instant::now();
        tracker.on_input(now);
        tracker.on_present(now + Duration::from_millis(20));
        assert!(!tracker.within_budget());
        assert_eq!(tracker.worst(), Some(Duration::from_millis(20)));
    }

    #[test]
    fn the_sample_window_is_bounded() {
        let mut tracker = LatencyTracker::new();
        let now = Instant::now();
        for i in 0..1000 {
            tracker.on_input(now);
            tracker.on_present(now + Duration::from_millis(i % 7 + 1));
        }
        assert_eq!(tracker.presented(), 1000);
        // The tracker keeps a bounded window, not every sample ever.
        assert!(tracker.samples.len() <= WINDOW);
    }
}

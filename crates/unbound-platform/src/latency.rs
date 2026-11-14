//! 输入到光子（Input-to-photon）延迟统计（`docs` §2.3、§10.2）。
//!
//! 红线指标为 8.3 ms——即 120 Hz 下的单帧时长。如实测量该延迟需要
//! 记录两个不同时刻的时间戳：
//!
//! * **输入（input）**——操作系统将事件投递给应用的时刻；
//! * **光子（photon）**——包含该响应的帧离开 GPU 的时刻。
//!
//! 追踪器保留**最早**的未呈现输入，因为一帧会响应渲染前到达的所有输入：
//! 若取平均值会将卡顿隐藏在快速帧背后。
//!

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 验收指标表中的延迟预算：120 Hz 下的单帧时长。
pub const BUDGET: Duration = Duration::from_micros(8_300);

/// 用于最坏情况统计的保留样本数。
const WINDOW: usize = 240;

/// 追踪输入事件与呈现该输入结果的帧之间的延迟。
#[derive(Debug, Default)]
pub struct LatencyTracker {
    /// 尚未呈现的最早输入时间戳。
    oldest_input: Option<Instant>,
    /// 近期延迟样本，按时间从旧到新排列。
    samples: VecDeque<Duration>,
    /// 最近一次测量的样本。
    last: Option<Duration>,
    /// 统计窗口内的最坏样本。
    worst: Option<Duration>,
    /// 携带了输入响应的已呈现帧数。
    presented: u64,
    /// 没有任何挂起输入时呈现的帧数。
    idle: u64,
}

impl LatencyTracker {
    /// 创建一个空的延迟追踪器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次输入事件。
    ///
    /// 仅需保留最早的挂起输入；后续输入会搭便车在同一帧中呈现。
    ///
    pub fn on_input(&mut self, now: Instant) {
        if self.oldest_input.is_none() {
            self.oldest_input = Some(now);
        }
    }

    /// 记录一帧已完成呈现。
    pub fn on_present(&mut self, now: Instant) {
        let Some(input) = self.oldest_input.take() else {
            self.idle += 1;
            return;
        };
        let latency = now.saturating_duration_since(input);
        self.presented += 1;
        self.last = Some(latency);
        if self.worst.is_none_or(|w| latency > w) {
            self.worst = Some(latency);
        }
        self.samples.push_back(latency);
        while self.samples.len() > WINDOW {
            self.samples.pop_front();
        }
    }

    /// 最近一次测量的延迟。
    #[must_use]
    pub const fn last(&self) -> Option<Duration> {
        self.last
    }

    /// 迄今测量的最坏延迟。
    #[must_use]
    pub const fn worst(&self) -> Option<Duration> {
        self.worst
    }

    /// 样本窗口内的平均延迟。
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let total: Duration = self.samples.iter().sum();
        Some(total / self.samples.len() as u32)
    }

    /// 样本窗口内的 P99 延迟。
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

    /// 携带了输入响应的帧数。
    #[must_use]
    pub const fn presented(&self) -> u64 {
        self.presented
    }

    /// 没有任何挂起输入时呈现的帧数。
    #[must_use]
    pub const fn idle(&self) -> u64 {
        self.idle
    }

    /// 当测得的最坏延迟仍在预算范围内时返回 `true`。
    #[must_use]
    pub fn within_budget(&self) -> bool {
        self.worst().is_some_and(|w| w <= BUDGET)
    }

    /// 延迟预算值。
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
        // 追踪器仅保留有界窗口，而非永久保留所有样本。
        assert!(tracker.samples.len() <= WINDOW);
    }
}

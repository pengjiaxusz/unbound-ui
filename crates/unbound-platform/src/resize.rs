//! 延迟交换链重建策略（`docs` §2.3）。
//!
//! 在 Windows 上，拖拽窗口边缘会产生大量 `WM_SIZE` 消息。
//! 此处实现的策略决定交换链**何时**跟随窗口尺寸：
//!
//!
//! * **实时模式（默认）。** 针对系统上报的每一个新尺寸重建交换链，
//!   使帧按新分辨率渲染，合成器无需拉伸上一帧。
//! * **延迟模式。** 在尺寸仍在变化时保留已提交尺寸，待尺寸稳定后再重建。
//!   该模式以拉伸帧换取更少的重建次数，适合无法承担每条消息都重建的宿主。
//!
//!
//!
//!
//! 两种模式均绝不阻塞消息循环：尺寸调整仅**记录**新尺寸，实际重建在帧内完成。
//!

use std::time::{Duration, Instant};

/// 默认使用的稳定窗口时长：交换链立即跟随窗口尺寸。
///
/// 零表示“完全无稳定等待窗口”：系统上报的每个尺寸都会当场提交。
/// 这正是实时缩放保持锐利、不会出现上一帧拉伸副本的原因。
///
///
pub const DEFAULT_SETTLE: Duration = Duration::ZERO;

/// 延迟模式的稳定等待窗口。
///
/// 120 Hz 下的两帧约为 16 ms：既足以吞并拖拽时的事件风暴，
/// 又短到用户完全察觉不到延迟。
pub const DEFERRED_SETTLE: Duration = Duration::from_millis(16);

/// 物理像素尺寸范围。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Extent {
    pub width: u32,
    pub height: u32,
}

impl Extent {
    /// 构建尺寸范围。
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// 当面积为零、无法支撑交换链时返回 `true`。
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 总像素数量。
    #[must_use]
    pub const fn pixels(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// 平台层针对窗口尺寸变化应采取的动作。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeAction {
    /// 无需操作。
    None,
    /// 按该尺寸重建交换链。
    Recreate(Extent),
}

/// 追踪窗口尺寸并决定交换链何时跟随重建。
#[derive(Debug)]
pub struct ResizeTracker {
    /// 交换链当前所处的尺寸。
    committed: Extent,
    /// 操作系统最近一次上报的尺寸。
    pending: Extent,
    /// `pending` 首次发生变化的时刻。
    pending_since: Option<Instant>,
    /// 尺寸在提交前必须保持稳定的时长。
    settle: Duration,
    /// 用户正在拖拽窗口边缘时置位。
    dragging: bool,
    /// 当驱动拒绝现有交换链时置位。
    ///
    /// 稳定窗口的存在是为了避免在用户仍在拖拽时频繁重建交换链。
    /// 但一旦驱动返回 `OUT_OF_DATE`，旧链已不可用——继续等待稳定只会让
    /// 窗口多一帧停留在旧分辨率上。
    ///
    forced: bool,
    /// 已提交的重建次数，供诊断使用。
    recreations: u32,
    /// 被吸收而非触发重建的尺寸调整消息数。
    absorbed: u32,
}

impl ResizeTracker {
    /// 为初始尺寸创建追踪器。
    ///
    /// `settle` 为 [`Duration::ZERO`] 时选择实时模式：系统上报的每个尺寸
    /// 立即提交。更大的窗口时长选择延迟模式，等待尺寸稳定后再重建。
    ///
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

    /// 使用默认稳定窗口创建追踪器。
    #[must_use]
    pub fn with_default_settle(initial: Extent) -> Self {
        Self::new(initial, DEFAULT_SETTLE)
    }

    /// 交换链当前所处的尺寸。
    #[must_use]
    pub const fn committed(&self) -> Extent {
        self.committed
    }

    /// 窗口当前实际尺寸。
    #[must_use]
    pub const fn pending(&self) -> Extent {
        self.pending
    }

    /// 当交换链与窗口尺寸不一致时返回 `true`。
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.pending != self.committed
    }

    /// 交换链重建次数。
    #[must_use]
    pub const fn recreations(&self) -> u32 {
        self.recreations
    }

    /// 被吸收合并的尺寸调整消息数。
    #[must_use]
    pub const fn absorbed(&self) -> u32 {
        self.absorbed
    }

    /// 用户开始拖拽窗口边缘。
    pub fn on_drag_started(&mut self) {
        self.dragging = true;
    }

    /// 用户结束拖拽窗口边缘。
    ///
    /// 立即提交挂起尺寸：用户期望在松手的瞬间窗口即匹配内容。
    ///
    pub fn on_drag_ended(&mut self) -> ResizeAction {
        self.dragging = false;
        self.commit()
    }

    /// 记录新的窗口尺寸。
    pub fn on_resized(&mut self, extent: Extent, now: Instant) -> ResizeAction {
        if extent.is_empty() {
            // 最小化窗口会上报零尺寸。为其重建交换链是非法的，因此忽略该尺寸
            // 直至窗口恢复。
            return ResizeAction::None;
        }
        let fresh = extent != self.pending;
        if fresh {
            self.pending = extent;
            self.pending_since = Some(now);
        }
        if self.settle.is_zero() {
            // 实时模式：交换链立即跟随窗口尺寸。因此拖拽期间绝不会出现上一帧的
            // 拉伸副本，即使在边缘拖拽过程中尺寸也会立即提交。
            //
            //
            return self.commit();
        }
        if fresh && extent != self.committed {
            self.absorbed += 1;
        }
        if self.dragging {
            // 拖拽期间交换链保持原尺寸不变。
            return ResizeAction::None;
        }
        // 尺寸可能已经稳定：即使操作系统重复上报相同尺寸，稳定计时器仍继续计时。
        //
        self.poll(now)
    }

    /// 将交换链标记为已过期（out of date）。
    ///
    /// 驱动已拒绝旧交换链，因此绕过稳定窗口：下一次 [`ResizeTracker::poll`]
    /// 将立即提交重建。
    ///
    /// 否则窗口会一直维持旧分辨率，直到用户停止拖拽并熬过稳定窗口——
    /// 而这期间的每一帧都会反复上报 `ERROR_OUT_OF_DATE_KHR`。
    ///
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

    /// 检查挂起尺寸是否已稳定并需要提交重建。
    pub fn poll(&mut self, now: Instant) -> ResizeAction {
        if !self.is_dirty() {
            self.forced = false;
            return ResizeAction::None;
        }
        if self.settle.is_zero() {
            // 实时模式：无需任何等待，拖拽也绝不会冻结已提交尺寸。
            //
            return self.commit();
        }
        if self.dragging {
            return ResizeAction::None;
        }
        if self.forced {
            // 驱动已拒绝旧链：立即重建。
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

    /// 提交挂起的尺寸。
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

    /// 实时模式为默认行为：系统上报尺寸后立即提交，使帧按新分辨率绘制，
    /// 而非显示上一帧的拉伸副本。
    ///
    #[test]
    fn a_resize_is_committed_immediately_in_real_time_mode() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        assert_eq!(
            tracker.on_resized(extent(900, 600), now),
            ResizeAction::Recreate(extent(900, 600)),
            "the swapchain must follow the window size right away"
        );
        assert!(!tracker.is_dirty());
        assert_eq!(tracker.recreations(), 1);
        assert_eq!(
            tracker.absorbed(),
            0,
            "the real-time mode never absorbs a resize message"
        );
    }

    /// 拖拽会产生大量尺寸事件。在实时模式下每一个都会被提交：
    /// 冻结尺寸会导致整个拖拽期间都显示拉伸帧。
    ///
    #[test]
    fn every_size_of_a_drag_burst_is_committed_in_real_time_mode() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        tracker.on_drag_started();
        for width in 801..=820 {
            assert_eq!(
                tracker.on_resized(extent(width, 600), now),
                ResizeAction::Recreate(extent(width, 600)),
                "the swapchain must not freeze while the edge is dragged"
            );
        }
        assert_eq!(tracker.recreations(), 20);
        assert_eq!(tracker.committed(), extent(820, 600));
    }

    /// 延迟模式保留旧行为：在尺寸仍在变化时将其吸收合并。
    ///
    #[test]
    fn a_resize_is_committed_after_the_size_settles() {
        let mut tracker = ResizeTracker::new(extent(800, 600), DEFERRED_SETTLE);
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
        let mut tracker = ResizeTracker::new(extent(800, 600), DEFERRED_SETTLE);
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
        // 交换链仍保持旧尺寸：合成器缩放上一帧，因此不会出现黑边。
        //
        assert_eq!(tracker.committed(), extent(800, 600));

        assert_eq!(
            tracker.on_drag_ended(),
            ResizeAction::Recreate(extent(820, 600)),
            "the final size is applied the moment the drag ends"
        );
        assert_eq!(tracker.recreations(), 1);
    }

    /// 驱动已拒绝交换链：稳定窗口已无意义，必须在下一次轮询时立即重建。
    ///
    #[test]
    fn out_of_date_marks_the_resize_tracker_dirty_immediately() {
        let mut tracker = ResizeTracker::new(extent(800, 600), DEFERRED_SETTLE);
        let now = Instant::now();
        assert_eq!(
            tracker.on_resized(extent(900, 600), now),
            ResizeAction::None,
            "the first message must not trigger a rebuild"
        );
        // 在尺寸稳定前驱动拒绝了交换链。
        tracker.mark_out_of_date(extent(900, 600));
        assert_eq!(
            tracker.poll(now + Duration::from_millis(1)),
            ResizeAction::Recreate(extent(900, 600)),
            "an out-of-date swapchain is rebuilt without waiting for the settle window"
        );
        assert!(!tracker.is_dirty());
        assert_eq!(tracker.recreations(), 1);
    }

    /// 即使操作系统尚未上报新尺寸，被驱动拒绝的链仍会触发重建——
    /// 否则窗口将继续停留在陈旧分辨率。
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

    /// 延迟模式在提交前保持渲染尺寸稳定。
    #[test]
    fn the_render_extent_never_changes_before_the_commit() {
        let mut tracker = ResizeTracker::new(extent(800, 600), DEFERRED_SETTLE);
        let now = Instant::now();
        tracker.on_resized(extent(1024, 768), now);
        // 在尺寸稳定期间渲染器继续使用旧尺寸。
        assert_eq!(tracker.committed(), extent(800, 600));
        assert_eq!(tracker.pending(), extent(1024, 768));
        tracker.poll(now + Duration::from_millis(50));
        assert_eq!(tracker.committed(), extent(1024, 768));
    }

    /// 实时模式在系统一上报新尺寸时即切换渲染尺寸——这正是消除拉伸帧的关键。
    ///
    #[test]
    fn the_render_extent_follows_the_window_in_real_time_mode() {
        let mut tracker = ResizeTracker::with_default_settle(extent(800, 600));
        let now = Instant::now();
        tracker.on_resized(extent(1024, 768), now);
        assert_eq!(tracker.committed(), extent(1024, 768));
        assert_eq!(tracker.pending(), extent(1024, 768));
    }

    #[test]
    fn repeated_identical_messages_do_not_restart_the_settle_timer() {
        let mut tracker = ResizeTracker::new(extent(800, 600), DEFERRED_SETTLE);
        let now = Instant::now();
        tracker.on_resized(extent(900, 600), now);
        // 再次收到相同尺寸：不得重启计时器。
        tracker.on_resized(extent(900, 600), now + Duration::from_millis(5));
        assert_eq!(
            tracker.poll(now + Duration::from_millis(20)),
            ResizeAction::Recreate(extent(900, 600))
        );
    }
}

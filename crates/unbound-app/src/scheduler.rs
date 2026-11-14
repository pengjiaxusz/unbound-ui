//! 应用运行时的定时器与跨线程任务调度（`docs` §5.5）。
//!
//! 同步 `reduce` 契约原本缺少两项能力：
//!
//! * **定时器队列**。光标需每 500 ms 闪烁一次，提示框需在 400 ms 后出现，
//!   防抖搜索需在用户停止输入后触发。若无定时器队列，唯一做法是每帧轮询
//!   `Tick`，白白唤醒 GPU。
//!
//! * **异步唤醒**。后台任务（资产解码、网络响应）在另一线程完成时，
//!   事件循环正休眠在 `ControlFlow::Wait` 中，必须被唤醒才能 reduce 结果。
//!
//!
//! [`TimerQueue`] 是以截止时间为键的最小堆，因此 [`TimerQueue::next_deadline_ms`]
//! 能在 O(n)（n 为较小的活跃集）内回答“事件循环何时需再次唤醒”。
//! [`AppWaker`] 则是平台后端通过 `EventLoopProxy` 实现的接缝。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use std::cell::Cell;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::rc::Rc;

use rustc_hash::FxHashSet;

use crate::AppEvent;

/// 单个已调度定时器的句柄。
///
/// token 在重调度期间保持稳定，因此周期定时器始终保留同一 token；
/// 应用负责解释它，队列仅负责路由。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TimerToken(u64);

impl TimerToken {
    /// 从原始标识构造 token；运行时计数器是唯一合法来源。
    ///
    #[must_use]
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }

    /// 原始标识，供自行维护映射表的应用使用。
    #[must_use]
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// 应用在 `reduce` 或 `view` 期间请求的单个定时器。
///
/// 运行时在事件处理结束后将其排入 [`TimerQueue`]，因为上下文无法直接
/// 持有队列：上下文实现了 `Clone`，克隆不应复制定时器调度。
///
#[derive(Clone, Debug, PartialEq)]
pub struct TimerRequest {
    /// 分配给应用的 token。
    pub token: TimerToken,
    /// 距离*现在*首次触发的毫秒数。
    pub delay_ms: u64,
    /// 周期定时器为 `Some(period)`，单次定时器为 `None`。
    pub period_ms: Option<u64>,
    /// 定时器触发时派发的事件。
    pub event: AppEvent,
}

/// 可从其他线程唤醒休眠事件循环的抽象。
///
/// `unbound-app` 保持不依赖具体窗口消息细节：平台后端用 `PostMessageW(WM_APP_WAKE)` 实现，
/// 测试用 channel 实现，服务端用无操作实现。
pub trait AppWaker: Send + Sync + 'static {
    /// 请求事件循环再运行一次迭代以 reduce 待处理任务。
    fn wake(&self);
}

struct TimerEntry {
    deadline_ms: u64,
    seq: u64,
    token: TimerToken,
    period_ms: Option<u64>,
    event: AppEvent,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline_ms == other.deadline_ms && self.seq == other.seq
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.deadline_ms, self.seq).cmp(&(other.deadline_ms, other.seq))
    }
}

/// 待触发定时器的最小堆。
pub struct TimerQueue {
    heap: BinaryHeap<Reverse<TimerEntry>>,
    cancelled: FxHashSet<u64>,
    /// 与 [`crate::AppContext`] 共享，使处理器无需第二个计数器即可
    /// 获得队列尚未见过的新 token。
    next_id: Rc<Cell<u64>>,
    next_seq: u64,
}

impl Default for TimerQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for TimerQueue {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TimerQueue")
            .field("pending", &self.heap.len())
            .field("cancelled", &self.cancelled.len())
            .finish()
    }
}

impl TimerQueue {
    /// 创建空队列。
    #[must_use]
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            cancelled: FxHashSet::default(),
            next_id: Rc::new(Cell::new(0)),
            next_seq: 0,
        }
    }

    /// 共享的 token 计数器，使应用上下文在同一空间内分配 token。
    ///
    #[must_use]
    pub fn tokens(&self) -> Rc<Cell<u64>> {
        Rc::clone(&self.next_id)
    }

    /// 分配一个新 token。
    #[must_use]
    pub fn next_token(&self) -> TimerToken {
        let id = self.next_id.get().saturating_add(1);
        self.next_id.set(id);
        TimerToken(id)
    }

    /// 调度单次定时器并返回其 token。
    pub fn set_timeout(&mut self, now_ms: u64, delay_ms: u64, event: AppEvent) -> TimerToken {
        let token = self.next_token();
        self.schedule(token, now_ms, delay_ms, None, event);
        token
    }

    /// 调度重复定时器并返回其 token。
    pub fn set_interval(&mut self, now_ms: u64, period_ms: u64, event: AppEvent) -> TimerToken {
        let token = self.next_token();
        self.schedule(token, now_ms, period_ms, Some(period_ms), event);
        token
    }

    /// 插入应用已持有 token 的定时器。
    pub fn schedule(
        &mut self,
        token: TimerToken,
        now_ms: u64,
        delay_ms: u64,
        period_ms: Option<u64>,
        event: AppEvent,
    ) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.heap.push(Reverse(TimerEntry {
            deadline_ms: now_ms.saturating_add(delay_ms),
            seq,
            token,
            period_ms,
            event,
        }));
    }

    /// 取消定时器。对已触发的 token 调用为空操作。
    pub fn cancel(&mut self, token: TimerToken) {
        self.cancelled.insert(token.id());
    }

    /// 返回最早的截止时间，忽略已取消条目。
    #[must_use]
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.heap
            .iter()
            .filter(|Reverse(entry)| !self.cancelled.contains(&entry.token.id()))
            .map(|Reverse(entry)| entry.deadline_ms)
            .min()
    }

    /// 是否没有待触发的定时器。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.next_deadline_ms().is_none()
    }

    /// 弹出在 `now_ms` 到期的所有定时器，并重调度周期定时器。
    pub fn due(&mut self, now_ms: u64) -> Vec<AppEvent> {
        let mut out = Vec::new();
        while let Some(Reverse(top)) = self.heap.peek() {
            if top.deadline_ms > now_ms {
                break;
            }
            let Some(Reverse(entry)) = self.heap.pop() else {
                break;
            };
            if self.cancelled.remove(&entry.token.id()) {
                continue;
            }
            out.push(entry.event.clone());
            if let Some(period) = entry.period_ms {
                // 延迟的帧绝不能突发补发所有错过的 tick：
                // 下一次触发从当前时刻起算，而非从错过的槽位起算。
                let deadline = now_ms.saturating_add(period.max(1));
                let seq = self.next_seq;
                self.next_seq = self.next_seq.saturating_add(1);
                self.heap.push(Reverse(TimerEntry {
                    deadline_ms: deadline,
                    seq,
                    token: entry.token,
                    period_ms: entry.period_ms,
                    event: entry.event,
                }));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick() -> AppEvent {
        AppEvent::Tick { dt: 0.0 }
    }

    #[test]
    fn a_timeout_fires_once_at_its_deadline() {
        let mut queue = TimerQueue::new();
        let token = queue.set_timeout(1_000, 400, tick());
        assert_eq!(queue.next_deadline_ms(), Some(1_400));
        assert!(queue.due(1_399).is_empty());
        assert_eq!(queue.due(1_400).len(), 1);
        assert!(queue.next_deadline_ms().is_none(), "a timeout is one-shot");
        assert_eq!(token.id(), 1);
    }

    #[test]
    fn an_interval_reschedules_itself() {
        let mut queue = TimerQueue::new();
        queue.set_interval(0, 500, tick());
        assert!(queue.due(0).is_empty(), "the first tick is one period away");
        assert_eq!(queue.next_deadline_ms(), Some(500));
        assert!(queue.due(499).is_empty());
        assert_eq!(queue.due(500).len(), 1);
        assert_eq!(queue.next_deadline_ms(), Some(1_000));
    }

    #[test]
    fn a_late_frame_does_not_burst_the_missed_ticks() {
        let mut queue = TimerQueue::new();
        queue.set_interval(0, 100, tick());
        let fired = queue.due(5_000);
        assert_eq!(fired.len(), 1, "one fire per poll, not fifty");
        assert_eq!(queue.next_deadline_ms(), Some(5_100));
    }

    #[test]
    fn a_cancelled_timer_never_fires() {
        let mut queue = TimerQueue::new();
        let token = queue.set_timeout(0, 10, tick());
        queue.cancel(token);
        assert_eq!(queue.next_deadline_ms(), None);
        assert!(queue.due(100).is_empty());
    }

    #[test]
    fn timers_fire_in_deadline_order() {
        let mut queue = TimerQueue::new();
        queue.set_timeout(0, 300, AppEvent::Tick { dt: 3.0 });
        queue.set_timeout(0, 100, AppEvent::Tick { dt: 1.0 });
        queue.set_timeout(0, 200, AppEvent::Tick { dt: 2.0 });
        let fired: Vec<f32> = queue
            .due(1_000)
            .into_iter()
            .map(|event| match event {
                AppEvent::Tick { dt } => dt,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(fired, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn the_shared_counter_hands_out_unique_tokens() {
        let queue = TimerQueue::new();
        let tokens = queue.tokens();
        let first = queue.next_token();
        let second = queue.next_token();
        assert_ne!(first, second);
        assert_eq!(tokens.get(), 2);
    }
}

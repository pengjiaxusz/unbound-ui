//! Timers and cross-thread work for the application runtime (`docs` §5.5).
//!
//! Two things were missing from the synchronous `reduce` contract:
//!
//! * a **timer queue**. A caret has to blink every 500 ms, a tooltip has to
//!   appear after 400 ms, and a debounced search has to fire once the user
//!   stops typing. Without a queue the only way to do that is to poll a `Tick`
//!   every frame, which wakes the GPU for nothing.
//! * an **async wake-up**. A background task (an asset decode, a network
//!   reply) finishes on another thread; the event loop is parked in
//!   `ControlFlow::Wait` and has to be woken so the result is reduced.
//!
//! [`TimerQueue`] is a min-heap keyed on the deadline, so
//! [`TimerQueue::next_deadline_ms`] answers "when does the loop have to wake
//! up again" in O(n) over the (small) live set. [`AppWaker`] is the seam the
//! platform backend implements with an `EventLoopProxy`.

#![forbid(unsafe_op_in_unsafe_fn)]

use std::cell::Cell;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::rc::Rc;

use rustc_hash::FxHashSet;

use crate::AppEvent;

/// A handle to one scheduled timer.
///
/// The token is stable across reschedules, so an interval keeps the same
/// token; the application interprets it, the queue only routes it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TimerToken(u64);

impl TimerToken {
    /// Build a token from raw identity; the runtime's counter is the only
    /// legitimate source.
    #[must_use]
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }

    /// The raw identity, for an application that stores its own table.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// One timer the application asked for during `reduce` or `view`.
///
/// The runtime drains these into its [`TimerQueue`] after the event, because
/// the context cannot own the queue: it is `Clone`, and a clone must not
/// duplicate the schedule.
#[derive(Clone, Debug, PartialEq)]
pub struct TimerRequest {
    /// The token the application was handed.
    pub token: TimerToken,
    /// Milliseconds from *now* until the first fire.
    pub delay_ms: u64,
    /// `Some(period)` for an interval, `None` for a one-shot.
    pub period_ms: Option<u64>,
    /// The event to dispatch when the timer fires.
    pub event: AppEvent,
}

/// Something that can wake a parked event loop from another thread.
///
/// `unbound-app` stays free of `winit`: the platform backend implements this
/// with an `EventLoopProxy`, a test with a channel, a server with a no-op.
pub trait AppWaker: Send + Sync + 'static {
    /// Ask the loop to run one more iteration so pending work is reduced.
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

/// A min-heap of pending timers.
pub struct TimerQueue {
    heap: BinaryHeap<Reverse<TimerEntry>>,
    cancelled: FxHashSet<u64>,
    /// Shared with the [`crate::AppContext`], so a handler can be handed a
    /// token the queue has not seen yet without a second counter.
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
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            cancelled: FxHashSet::default(),
            next_id: Rc::new(Cell::new(0)),
            next_seq: 0,
        }
    }

    /// The shared token counter, so the application context allocates from the
    /// same space.
    #[must_use]
    pub fn tokens(&self) -> Rc<Cell<u64>> {
        Rc::clone(&self.next_id)
    }

    /// Allocate a fresh token.
    #[must_use]
    pub fn next_token(&self) -> TimerToken {
        let id = self.next_id.get().saturating_add(1);
        self.next_id.set(id);
        TimerToken(id)
    }

    /// Schedule a one-shot timer, returning its token.
    pub fn set_timeout(&mut self, now_ms: u64, delay_ms: u64, event: AppEvent) -> TimerToken {
        let token = self.next_token();
        self.schedule(token, now_ms, delay_ms, None, event);
        token
    }

    /// Schedule a repeating timer, returning its token.
    pub fn set_interval(&mut self, now_ms: u64, period_ms: u64, event: AppEvent) -> TimerToken {
        let token = self.next_token();
        self.schedule(token, now_ms, period_ms, Some(period_ms), event);
        token
    }

    /// Insert a timer the application already owns a token for.
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

    /// Cancel a timer. A token that already fired is a no-op.
    pub fn cancel(&mut self, token: TimerToken) {
        self.cancelled.insert(token.id());
    }

    /// The earliest deadline, ignoring cancelled entries.
    #[must_use]
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.heap
            .iter()
            .filter(|Reverse(entry)| !self.cancelled.contains(&entry.token.id()))
            .map(|Reverse(entry)| entry.deadline_ms)
            .min()
    }

    /// Whether no timer is pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.next_deadline_ms().is_none()
    }

    /// Pop every timer due at `now_ms`, rescheduling the intervals.
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
                // A frame that ran late must not burst every missed tick: the
                // next fire is measured from now, not from the missed slot.
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

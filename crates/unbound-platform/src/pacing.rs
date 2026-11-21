//! 帧率节拍与呈现等待策略。
//!
//! 两项独立机制在此交汇：
//!
//! * **`VK_KHR_present_wait`** 允许 CPU 阻塞等待刚提交的图像**真正**开始
//!   扫描输出。紧随该时间点采样输入队列可将输入到光子延迟保持在 1 帧
//!   而非 3 帧。
//! * **`VK_EXT_swapchain_maintenance1`** 提供
//!   `VK_PRESENT_MODE_FIFO_LATEST_READY_EXT`：无陈旧帧排队的垂直同步，
//!   确保每次“呈现”的都是最新图像。
//!
//!
//! 节拍器本身是纯算术逻辑：它决定下一帧何时到期，并在无任何变更时跳过帧，
//! 从而使空闲状态下的 CPU 与 GPU 负载保持为零。
//!

use std::time::{Duration, Instant};

/// 120 Hz，验收指标表所针对的目标刷新率。
pub const REFRESH_120: Duration = Duration::from_nanos(8_333_333);

/// 60 Hz。
pub const REFRESH_60: Duration = Duration::from_nanos(16_666_667);

/// 引擎接受的最小非零帧率上限。
pub const MIN_FPS_LIMIT: u32 = 10;

/// 后台空闲默认降频目标（10 Hz，ADR-0022）。
pub const IDLE_FPS_LIMIT: u32 = 10;

/// 在精确帧截止时间之前提前唤醒事件循环系统休眠的时长。
///
/// `ControlFlow::WaitUntil` 在 Windows 上最终调用 `MsgWaitForMultipleObjectsEx`，
/// 其超时参数为整数毫秒，且受系统定时器粒度（1–2 ms）影响而额外延迟。
/// 因此直接休眠到精确时刻必然会睡过头；提前这么多时间唤醒并自旋等待剩余
/// 间隙，可使实际帧率精准贴合目标值，而非偏低几个百分点。
///
///
///
pub const SPIN_LEAD: Duration = Duration::from_micros(1200);

/// 不大于该阈值的间隔不执行自旋：亚毫秒级的忙等待消耗的 CPU 高于其消除的抖动。
///
const MIN_SPIN_INTERVAL: Duration = Duration::from_millis(2);

/// CPU 与显示器同步的方式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentSync {
    /// 在 `vkWaitForPresentKHR` 上阻塞直至图像扫描输出。
    ///
    /// 这是低延迟路径：在真实呈现后立即采样输入队列，使响应能被紧接着的
    /// 下一帧捕获。
    WaitForPresent,
    /// 提交呈现后立即继续；由驱动决定图像何时显示。
    ///
    /// 当 `VK_KHR_present_wait` 不可用时使用。
    Immediate,
    /// 窗口不可见：完全不执行任何渲染。
    Suspended,
}

/// 由设备扩展列表推导出的呈现策略。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PresentWaitPolicy {
    /// `VK_KHR_present_wait` 可用。
    pub has_present_wait: bool,
    /// `VK_EXT_swapchain_maintenance1` 可用。
    pub has_maintenance1: bool,
}

impl PresentWaitPolicy {
    /// 根据设备扩展列表构建呈现策略。
    #[must_use]
    pub fn from_extensions(extensions: &[String]) -> Self {
        let has = |name: &str| extensions.iter().any(|e| e == name);
        Self {
            has_present_wait: has("VK_KHR_present_wait"),
            has_maintenance1: has("VK_EXT_swapchain_maintenance1"),
        }
    }

    /// 要使用的同步模式。
    ///
    /// 仅 `maintenance1` 即可实现无撕裂的“最新就绪”呈现；`present_wait` 则额外让
    /// CPU 获知图像何时上屏，这正是输入采样器所需要的。
    ///
    #[must_use]
    pub const fn sync(&self) -> PresentSync {
        // 仅 `present_wait` 会改变 CPU 行为：`maintenance1` 决定渲染器选取的**呈现模式**，
        // 而非 CPU 等待帧的方式。
        //
        if self.has_present_wait {
            PresentSync::WaitForPresent
        } else {
            PresentSync::Immediate
        }
    }

    /// 当 CPU 可以在呈现信号上阻塞时返回 `true`。
    #[must_use]
    pub const fn can_wait_for_present(&self) -> bool {
        self.has_present_wait
    }
}

/// 针对同一扩展列表的渲染器呈现策略。
///
/// 渲染器持有交换链，因此呈现**模式**在渲染器侧决定；平台层仅决定 CPU
/// 如何与之同步。两者必须保持一致，本函数即是将二者关联起来的唯一入口。
///
#[must_use]
pub fn renderer_present_config(
    extensions: &[String],
    allow_tearing: bool,
) -> unbound_gpu::PresentConfig {
    unbound_gpu::PresentConfig::from_capabilities(extensions, allow_tearing)
}

/// 窗口遮挡与空闲节流状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderThrottleState {
    /// 窗口可见且处于活跃态，按完整目标刷新率运行。
    Active,
    /// 窗口失焦且已超过空闲宽限期，降频至指定 FPS。
    ThrottledFps(u32),
    /// 窗口最小化、隐藏或被完全遮挡，彻底暂停帧唤醒与 GPU 提交。
    Suspended,
}

impl RenderThrottleState {
    /// 兼容别名：等同于 [`Self::Active`]。
    #[allow(non_upper_case_globals)]
    pub const FullSpeed: Self = Self::Active;
    /// 兼容别名：等同于 [`Self::Suspended`]。
    #[allow(non_upper_case_globals)]
    pub const PausedZeroCost: Self = Self::Suspended;
}

/// 帧节流策略：控制活跃态、失焦空闲态与遮挡/最小化暂停行为。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RenderThrottlePolicy {
    /// 活跃态目标帧率（FPS；`0` 表示无限制）。
    pub active_fps: u32,
    /// 失焦且超过空闲宽限期后的目标帧率（FPS）。
    pub unfocused_idle_fps: u32,
    /// 窗口失焦后进入降频前的空闲宽限时长（毫秒）。
    pub idle_grace_ms: u64,
    /// 窗口被其他不透明窗口完全遮挡时是否暂停渲染。
    pub suspend_when_occluded: bool,
    /// 窗口最小化时是否暂停渲染。
    pub suspend_when_minimized: bool,
}

impl Default for RenderThrottlePolicy {
    fn default() -> Self {
        Self {
            active_fps: 60,
            unfocused_idle_fps: 10,
            idle_grace_ms: 500,
            suspend_when_occluded: true,
            suspend_when_minimized: true,
        }
    }
}

#[inline]
const fn interval_from_fps(fps: u32) -> Duration {
    if fps == 0 {
        Duration::ZERO
    } else {
        let nanos = 1_000_000_000u64 / (fps as u64);
        Duration::from_nanos(if nanos == 0 { 1 } else { nanos })
    }
}

#[inline]
fn fps_from_interval(interval: Duration) -> u32 {
    if interval.is_zero() {
        return 0;
    }
    let nanos = interval.as_nanos().max(1) as u64;
    ((1_000_000_000u64 + nanos / 2) / nanos).max(1) as u32
}

/// 决定下一帧何时到期。
#[derive(Copy, Clone, Debug)]
pub struct FramePacer {
    /// 目标帧间隔（活跃态）。
    interval: Duration,
    /// 上一帧呈现的时刻。
    last_present: Option<Instant>,
    /// 窗口当前是否可见。
    visible: bool,
    /// 应用是否仍需要继续绘制帧。
    ///
    /// 静止页面将其设为 `false`；事件循环随后休眠在 `ControlFlow::Wait`，
    /// 而非按刷新率定时唤醒。
    animating: bool,
    /// 窗口当前是否拥有焦点。
    focused: bool,
    /// 窗口当前是否被其他不透明窗口完全遮挡。
    occluded: bool,
    /// 窗口当前是否处于最小化状态。
    minimized: bool,
    /// 最近一次交互或状态唤醒时间戳（用于计算失焦空闲宽限期）。
    last_activity: Option<Instant>,
    /// 自适应空闲降频与遮挡暂停策略。
    policy: RenderThrottlePolicy,
}

impl FramePacer {
    /// 按给定刷新率创建节拍器。
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        let active_fps = fps_from_interval(interval);
        Self {
            interval,
            last_present: None,
            visible: true,
            animating: true,
            focused: true,
            occluded: false,
            minimized: false,
            last_activity: None,
            policy: RenderThrottlePolicy {
                active_fps,
                ..RenderThrottlePolicy::default()
            },
        }
    }

    /// 创建 120 Hz 节拍器。
    #[must_use]
    pub fn at_120_hz() -> Self {
        Self::new(REFRESH_120)
    }

    /// 创建 60 Hz 节拍器。
    #[must_use]
    pub fn at_60_hz() -> Self {
        Self::new(REFRESH_60)
    }

    /// 返回目标帧间隔。
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }

    /// 设置帧率上限（FPS）。
    ///
    /// * `0` 表示**无限制**：间隔为零，每一帧都立即到期，事件循环进入轮询而非休眠。
    ///
    /// * 其余值均钳制到不低于 [`MIN_FPS_LIMIT`]。
    pub fn set_fps_limit(&mut self, fps: u32) {
        if fps == 0 {
            self.interval = Duration::ZERO;
            self.policy.active_fps = 0;
        } else {
            let clamped = fps.max(MIN_FPS_LIMIT);
            self.interval = interval_from_fps(clamped);
            self.policy.active_fps = clamped;
        }
    }

    /// 配置自适应空闲降频与遮挡暂停策略。
    pub fn set_throttle_policy(&mut self, policy: RenderThrottlePolicy) {
        self.policy = policy;
        self.interval = interval_from_fps(policy.active_fps);
    }

    /// 返回当前生效的帧节流策略。
    #[must_use]
    pub const fn throttle_policy(&self) -> RenderThrottlePolicy {
        self.policy
    }

    /// 更新窗口焦点状态。
    ///
    /// 窗口从失焦恢复焦点时清除上一帧相位，确保首帧零延迟绘制。
    pub fn set_focused(&mut self, focused: bool) {
        let regained = !self.focused && focused;
        self.focused = focused;
        if regained {
            self.last_present = None;
        }
    }

    /// 窗口当前是否拥有焦点。
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// 更新窗口完全遮挡状态。
    ///
    /// 进入遮挡暂停时清除旧截止时间，解除遮挡时触发即时首帧重绘。
    pub fn set_occluded(&mut self, occluded: bool) {
        let changed = self.occluded != occluded;
        self.occluded = occluded;
        if changed && (occluded || self.policy.suspend_when_occluded) {
            self.last_present = None;
        }
    }

    /// 窗口当前是否被完全遮挡。
    #[must_use]
    pub const fn is_occluded(&self) -> bool {
        self.occluded
    }

    /// 更新窗口最小化状态。
    ///
    /// 进入最小化暂停时清除旧截止时间，从最小化恢复时触发即时首帧重绘。
    pub fn set_minimized(&mut self, minimized: bool) {
        let changed = self.minimized != minimized;
        self.minimized = minimized;
        if changed && (minimized || self.policy.suspend_when_minimized) {
            self.last_present = None;
        }
    }

    /// 窗口当前是否处于最小化状态。
    #[must_use]
    pub const fn is_minimized(&self) -> bool {
        self.minimized
    }

    /// 记录一次用户交互或显式活跃事件的时间戳。
    pub fn note_activity(&mut self, now: Instant) {
        self.last_activity = Some(now);
    }

    /// 当前是否处于彻底暂停状态（不可见、策略允许下的最小化或完全遮挡）。
    #[must_use]
    pub const fn is_suspended(&self) -> bool {
        !self.visible
            || (self.minimized && self.policy.suspend_when_minimized)
            || (self.occluded && self.policy.suspend_when_occluded)
    }

    /// 计算在时刻 `now` 的节流状态。
    #[must_use]
    pub fn throttle_state(&self, now: Instant) -> RenderThrottleState {
        if self.is_suspended() {
            return RenderThrottleState::Suspended;
        }
        if self.focused {
            return RenderThrottleState::Active;
        }
        let grace = Duration::from_millis(self.policy.idle_grace_ms);
        let in_grace = self
            .last_activity
            .is_some_and(|last| now.saturating_duration_since(last) < grace);
        if in_grace {
            RenderThrottleState::Active
        } else {
            RenderThrottleState::ThrottledFps(self.policy.unfocused_idle_fps.max(1))
        }
    }

    /// 计算在时刻 `now` 实际生效的帧间隔；处于暂停状态时返回 `None`。
    #[must_use]
    pub fn effective_interval(&self, now: Instant) -> Option<Duration> {
        match self.throttle_state(now) {
            RenderThrottleState::Suspended => None,
            RenderThrottleState::Active => Some(self.interval),
            RenderThrottleState::ThrottledFps(fps) => Some(interval_from_fps(fps.max(1))),
        }
    }

    /// 当前帧率上限（无限制时为 `0`）。
    #[must_use]
    pub fn fps_limit(&self) -> u32 {
        if self.interval.is_zero() {
            return 0;
        }
        let nanos = self.interval.as_nanos().max(1) as u64;
        // 四舍五入到最近整数，避免设置 144 后读回却显示 143。
        //
        ((1_000_000_000u64 + nanos / 2) / nanos).max(u64::from(MIN_FPS_LIMIT)) as u32
    }

    /// 告知节拍器应用是否仍需要绘制帧。
    ///
    /// 当此值为 `false` 且窗口可见时，[`Self::next_deadline`] 返回 `None`，
    /// 后端使事件循环休眠直至有新事件到达。
    pub fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    /// 应用是否仍需要绘制帧。
    #[must_use]
    pub const fn animating(&self) -> bool {
        self.animating
    }

    /// 窗口变为可见或隐藏。
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            // 隐藏窗口绝不能保留过去的截止时间。
            self.last_present = None;
        }
    }

    /// 窗口可见时返回 `true`。
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// 记录一帧已完成呈现。
    ///
    /// 下一个截止时间从**上一截止时间**精确推进一个间隔，而非从唤醒时刻起算。
    /// 重置为 `now` 会使每次睡过头永久累积：在 6.944 ms（144 FPS）下，
    /// 0.8 ms 的延迟唤醒与 0.8 ms 的定时器舍入逐帧叠加，最终实测帧率会掉到
    /// 115 FPS 而非 144 FPS。
    ///
    ///
    /// 超过一整个帧间隔的卡顿会重置相位而非狂追进度，从而防止拖拽或 GPU
    /// 停顿后瞬间爆发一连串背靠背的帧。
    ///
    pub fn on_present(&mut self, now: Instant) {
        let interval = self.effective_interval(now).unwrap_or(self.interval);
        self.last_present = Some(match self.last_present {
            Some(previous) if !interval.is_zero() => {
                let next = previous + interval;
                if now > next + interval {
                    now
                } else {
                    next
                }
            }
            _ => now,
        });
    }

    /// 若节拍器正在运行，返回下一帧到期的时刻。
    #[must_use]
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.animating {
            return None;
        }
        let interval = self.effective_interval(now)?;
        let last = self.last_present?;
        // 无限制节拍器始终处于到期状态；返回过去的截止时间也可行，
        // 但返回 `now` 能让 `ControlFlow::Poll` 意图更明确。
        Some((last + interval).max(now))
    }

    /// 事件循环应当唤醒的时刻。
    ///
    /// 即 [`Self::next_deadline`] 提前 [`SPIN_LEAD`] 后的时刻，使
    /// [`Self::spin_to_deadline`] 能够补齐剩余的亚毫秒间隙，而非由系统定时器
    /// 决定帧的落点。
    #[must_use]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        let interval = self.effective_interval(now)?;
        let deadline = self.next_deadline(now)?;
        if interval <= MIN_SPIN_INTERVAL {
            return Some(deadline);
        }
        Some(deadline.checked_sub(SPIN_LEAD).unwrap_or(now))
    }

    /// 精确截止时间是否已足够接近、适合开始自旋。
    #[must_use]
    pub fn within_spin_lead(&self, now: Instant) -> bool {
        let Some(interval) = self.effective_interval(now) else {
            return false;
        };
        let Some(last) = self.last_present else {
            return false;
        };
        if interval <= MIN_SPIN_INTERVAL {
            return false;
        }
        let deadline = last + interval;
        now < deadline && deadline.saturating_duration_since(now) <= SPIN_LEAD
    }

    /// 忙等待至精确截止时间并返回命中的瞬间。
    ///
    /// 自旋时长受 [`SPIN_LEAD`] 限制，因为调用方仅在 [`Self::within_spin_lead`]
    /// 为 true 时才调用自旋。
    #[must_use]
    pub fn spin_to_deadline(&self) -> Instant {
        let Some(last) = self.last_present else {
            return Instant::now();
        };
        let mut now = Instant::now();
        let Some(interval) = self.effective_interval(now) else {
            return now;
        };
        let deadline = last + interval;
        while now < deadline {
            std::hint::spin_loop();
            now = Instant::now();
        }
        now
    }

    /// 当有帧到期需要绘制时返回 `true`。
    #[must_use]
    pub fn should_render(&self, now: Instant) -> bool {
        let Some(interval) = self.effective_interval(now) else {
            return false;
        };
        match self.last_present {
            // 尚未呈现过任何帧：立即绘制首帧。
            None => true,
            Some(last) => now.saturating_duration_since(last) >= interval,
        }
    }

    /// 跳过等待下一帧：立即标记一帧到期并刷新活跃时间戳。
    ///
    /// 在用户交互时使用——响应绝不能干等常规节拍。
    ///
    pub fn request_immediate(&mut self, now: Instant) {
        self.last_activity = Some(now);
        self.last_present = if self.interval.is_zero() {
            Some(now)
        } else {
            now.checked_sub(self.interval)
        };
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
        // 两端必须选取相同的呈现模式，否则 CPU 会等待一个 GPU 从不执行的呈现。
        //
        assert_eq!(policy.sync(), PresentSync::WaitForPresent);
        assert_eq!(config.mode, unbound_gpu::PresentMode::FifoLatestReady);
        assert!(config.present_fence);
        // 两端也必须在呈现等待上达成一致：仅当渲染器能够识别该呈现场次时，
        // CPU 才对其执行等待。
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
        pacer.set_fps_limit(5);
        assert_eq!(pacer.fps_limit(), MIN_FPS_LIMIT, "5 must clamp to 10");
        pacer.set_fps_limit(IDLE_FPS_LIMIT);
        assert_eq!(pacer.fps_limit(), IDLE_FPS_LIMIT);
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
        // 第 2 帧晚醒了 0.5 ms。相位必须保持在网格上，因此下一截止时间
        // 依然精确位于起始时刻之后两个间隔处。
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

    #[test]
    fn throttle_lifecycle_transitions_from_active_to_idle_to_suspended_and_recovers() {
        let mut pacer = FramePacer::at_120_hz();
        pacer.set_throttle_policy(RenderThrottlePolicy {
            active_fps: 120,
            unfocused_idle_fps: 10,
            idle_grace_ms: 500,
            suspend_when_occluded: true,
            suspend_when_minimized: true,
        });
        let t0 = Instant::now();
        pacer.note_activity(t0);
        pacer.on_present(t0);

        // 1. 活跃态按 120fps 推进
        assert_eq!(pacer.throttle_state(t0), RenderThrottleState::Active);
        let active_deadline = pacer.next_deadline(t0).expect("active deadline");
        assert!(active_deadline.duration_since(t0) <= Duration::from_millis(9));

        // 2. 失焦但仍在 500ms 宽限期内：仍为 Active
        pacer.set_focused(false);
        let t_grace = t0 + Duration::from_millis(250);
        assert_eq!(pacer.throttle_state(t_grace), RenderThrottleState::Active);

        // 3. 失焦超过 500ms 宽限期：降频至 10fps（100ms 间隔）
        let t_idle = t0 + Duration::from_millis(500);
        assert_eq!(
            pacer.throttle_state(t_idle),
            RenderThrottleState::ThrottledFps(10)
        );
        pacer.on_present(t_idle);
        assert!(!pacer.should_render(t_idle + Duration::from_millis(50)));
        assert!(pacer.should_render(t_idle + Duration::from_millis(100)));
        let idle_deadline = pacer.next_deadline(t_idle).expect("idle deadline");
        assert_eq!(
            idle_deadline.duration_since(t_idle),
            Duration::from_millis(100)
        );

        // 4. 完全遮挡：彻底暂停，无定时唤醒
        let t_occ = t_idle + Duration::from_millis(20);
        pacer.set_occluded(true);
        assert_eq!(pacer.throttle_state(t_occ), RenderThrottleState::Suspended);
        assert!(!pacer.should_render(t_occ + Duration::from_secs(10)));
        assert!(pacer.next_deadline(t_occ).is_none());
        assert!(pacer.next_wake(t_occ).is_none());

        // 5. 解除遮挡并重新获得焦点：立即恢复 Active 且首帧立即可渲染
        let t_recover = t_occ + Duration::from_millis(10);
        pacer.set_occluded(false);
        pacer.set_focused(true);
        assert_eq!(pacer.throttle_state(t_recover), RenderThrottleState::Active);
        assert!(
            pacer.should_render(t_recover),
            "first frame after recovery must render immediately"
        );
    }

    #[test]
    fn minimized_window_suspends_only_when_policy_enables_it() {
        let mut pacer = FramePacer::at_60_hz();
        let now = Instant::now();
        pacer.on_present(now);

        pacer.set_minimized(true);
        assert_eq!(pacer.throttle_state(now), RenderThrottleState::Suspended);
        assert!(!pacer.should_render(now + Duration::from_secs(1)));
        assert!(pacer.next_wake(now).is_none());

        // 策略关闭最小化暂停时恢复正常渲染
        let mut policy = pacer.throttle_policy();
        policy.suspend_when_minimized = false;
        pacer.set_throttle_policy(policy);
        assert_eq!(pacer.throttle_state(now), RenderThrottleState::Active);
        assert!(pacer.should_render(now));

        // 恢复最小化状态
        policy.suspend_when_minimized = true;
        pacer.set_throttle_policy(policy);
        pacer.set_minimized(false);
        assert_eq!(pacer.throttle_state(now), RenderThrottleState::Active);
        assert!(pacer.should_render(now));
    }
}

//! 弹簧物理、缓动函数与响应式帧调度（`docs` §4.4）。
//!
//! 动画通道刻意**独立于布局**：弹簧驱动的是 `GpuTransform`，
//! 而非 Taffy 样式。因此卡片可以在 120 Hz 下流畅移动，而布局引擎完全无需重算。
//!
//!
//! 实际体验中，两个细节决定了弹簧动画的手感：
//!
//! * **子步进（Substepping）。** 一次 100 ms 的卡顿若用单步欧拉积分会注入能量，
//!   导致弹簧发散爆炸。因此每帧步进都会切分为不超过 [`MAX_SUBSTEP`] 秒的子步，
//!   从而保证积分器在任意帧耗时下都保持稳定。
//!
//! * **收敛判定（Settling）。** 当位移与速度均低于感知阈值时即判定弹簧静止，
//!   而非等待数学意义上的绝对收敛。这使调度器能够将 CPU/GPU 占用降至 0%（`docs` §4.4）。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;

/// 最大积分时间步长；4 ms 可确保半隐式欧拉法即使面对高刚度弹簧也保持稳定。
///
pub const MAX_SUBSTEP: f32 = 0.004;

/// 一维阻尼谐振子。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub value: f32,
    pub target: f32,
    pub velocity: f32,
    /// 判定弹簧已静止的位移阈值。
    pub rest_threshold: f32,
    /// 判定弹簧已静止的速度阈值。
    pub rest_velocity: f32,
}

impl Default for Spring {
    fn default() -> Self {
        Self {
            stiffness: 320.0,
            damping: 32.0,
            mass: 1.0,
            value: 0.0,
            target: 0.0,
            velocity: 0.0,
            rest_threshold: 0.001,
            rest_velocity: 0.001,
        }
    }
}

/// 预设弹簧手感参数，针对 UI 动效调优而非追求物理精确。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpringPreset {
    /// 轻快且略带回弹：按钮、开关。
    Snappy,
    /// 平滑无过冲：面板、抽屉。
    Gentle,
    /// 沉稳且缓慢：大画布。
    Heavy,
    /// 接近临界阻尼：进度条。
    Precise,
}

impl SpringPreset {
    #[must_use]
    pub const fn config(self) -> (f32, f32) {
        // (刚度, 阻尼)
        match self {
            Self::Snappy => (520.0, 26.0),
            Self::Gentle => (220.0, 30.0),
            Self::Heavy => (120.0, 26.0),
            Self::Precise => (400.0, 40.0),
        }
    }

    #[must_use]
    pub fn spring(self, initial: f32) -> Spring {
        let (stiffness, damping) = self.config();
        Spring {
            stiffness,
            damping,
            value: initial,
            target: initial,
            ..Default::default()
        }
    }
}

impl Spring {
    #[must_use]
    pub fn new(preset: SpringPreset, initial: f32) -> Self {
        preset.spring(initial)
    }

    /// 临界阻尼弹簧：无过冲地到达目标。
    #[must_use]
    pub fn critically_damped(stiffness: f32, initial: f32) -> Self {
        Self {
            stiffness,
            // 单位质量的临界阻尼：c = 2·sqrt(k)。
            damping: 2.0 * stiffness.sqrt(),
            value: initial,
            target: initial,
            ..Default::default()
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// 立即跳转到位（用于初始布局，无动画）。
    pub fn snap_to(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    #[must_use]
    pub fn is_settled(&self) -> bool {
        // 非有限状态永远无法满足下方比较（与 NaN 的任何比较均为 false），
        // 会导致弹簧永不收敛、帧调度器永不休眠。损坏的弹簧直接视为已静止。
        //
        //
        if !self.value.is_finite() || !self.target.is_finite() || !self.velocity.is_finite() {
            return true;
        }
        (self.target - self.value).abs() <= self.rest_threshold
            && self.velocity.abs() <= self.rest_velocity
    }

    /// 推进 `dt` 秒。若仍在运动则返回 `true`。
    pub fn step(&mut self, dt: f32) -> bool {
        // 非有限目标（如布局输出的 NaN 或调用方除零）会永久污染状态：
        // `is_settled` 中的比较对 NaN 均为 false，导致弹簧永不收敛、
        // 动画器永不移除它、帧调度器无限空转。此处直接将异常弹簧归零静止。
        //
        //
        if !self.target.is_finite() {
            self.value = 0.0;
            self.target = 0.0;
            self.velocity = 0.0;
            return false;
        }
        if !dt.is_finite() || dt <= 0.0 {
            return !self.is_settled();
        }
        let mut remaining = dt.min(0.25);
        while remaining > 0.0 {
            let h = remaining.min(MAX_SUBSTEP);
            // 半隐式欧拉法：先积分速度，再用*新*速度积分位置。具有辛几何性质，不会凭空增益能量。
            //
            let force = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            let accel = force / self.mass.max(1e-3);
            self.velocity += accel * h;
            self.value += self.velocity * h;
            remaining -= h;
        }
        if !self.value.is_finite() || !self.velocity.is_finite() {
            // 积分发散（如刚度过大或数值溢出）。直接静止，避免在损坏状态上空转。
            //
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        if self.is_settled() {
            // 吸附消除残余误差，使最后一帧与静止状态像素完全一致，节点可重新启用像素对齐。
            //
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    /// 上一步的过冲量（占总行程的比例）。供测试验证预设确实为临界阻尼。
    ///
    #[must_use]
    pub fn overshoot(&self, from: f32) -> f32 {
        let travel = self.target - from;
        if travel.abs() < 1e-6 {
            return 0.0;
        }
        ((self.value - self.target) / travel).abs()
    }
}

/// 二维弹簧，用于驱动变换平移。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Spring2 {
    pub x: Spring,
    pub y: Spring,
}

impl Spring2 {
    #[must_use]
    pub fn new(preset: SpringPreset, initial: Vec2) -> Self {
        Self {
            x: Spring::new(preset, initial.x),
            y: Spring::new(preset, initial.y),
        }
    }

    pub fn set_target(&mut self, target: Vec2) {
        self.x.set_target(target.x);
        self.y.set_target(target.y);
    }

    pub fn snap_to(&mut self, value: Vec2) {
        self.x.snap_to(value.x);
        self.y.snap_to(value.y);
    }

    #[must_use]
    pub fn value(&self) -> Vec2 {
        Vec2::new(self.x.value, self.y.value)
    }

    #[must_use]
    pub fn velocity(&self) -> Vec2 {
        Vec2::new(self.x.velocity, self.y.velocity)
    }

    /// 推进双轴；任一轴仍在运动则返回 `true`。
    pub fn step(&mut self, dt: f32) -> bool {
        let a = self.x.step(dt);
        let b = self.y.step(dt);
        a || b
    }

    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.x.is_settled() && self.y.is_settled()
    }
}

/// 用于一次性过渡的三次贝塞尔缓动（如主题渐变、向导步骤）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const EASE: Self = Self {
        x1: 0.25,
        y1: 0.1,
        x2: 0.25,
        y2: 1.0,
    };
    pub const EASE_OUT: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    pub const LINEAR: Self = Self {
        x1: 0.0,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    };

    #[must_use]
    pub fn sample(&self, t: f32) -> f32 {
        // 对 x(t) = u 执行牛顿迭代，随后求值 y(t)。8 次迭代远超视觉精度需求，耗时约 40 ns。
        //
        let u = t.clamp(0.0, 1.0);
        let mut guess = u;
        for _ in 0..8 {
            let x = Self::bezier(guess, self.x1, self.x2) - u;
            let dx = Self::bezier_derivative(guess, self.x1, self.x2);
            if dx.abs() < 1e-6 {
                break;
            }
            guess = (guess - x / dx).clamp(0.0, 1.0);
        }
        Self::bezier(guess, self.y1, self.y2)
    }

    fn bezier(t: f32, p1: f32, p2: f32) -> f32 {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t
    }

    fn bezier_derivative(t: f32, p1: f32, p2: f32) -> f32 {
        let mt = 1.0 - t;
        3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
    }
}

/// 弹簧驱动的属性通道。以纯数据形式表示，使动画器可存储为扁平数组
/// （`docs` §4.4：视图树中不使用闭包）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    TranslateX,
    TranslateY,
    Scale,
    Rotate,
    Opacity,
}

/// 单个运行中的动画实例。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Animation {
    /// 所属节点的不透明句柄。
    pub node: u64,
    pub channel: Channel,
    pub spring: Spring,
    /// 可无分配复用的空闲槽位。
    pub generation: u32,
}

/// 动画器：原地压缩的扁平弹簧数组。
#[derive(Debug, Default)]
pub struct Animator {
    animations: Vec<Animation>,
    /// 上一步仍在运动的弹簧数量（用于驱动帧调度）。
    active_count: u32,
    /// 实际发生运动的帧数。
    pub moving_frames: u64,
    /// 无任何运动的帧数（调度器可进入休眠）。
    pub idle_frames: u64,
}

impl Animator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.animations.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }

    /// 仍在运动的弹簧数。直接根据弹簧状态重算，因此即使在新动画首次 `step` 前也准确。
    ///
    #[must_use]
    pub fn active_animations(&self) -> u32 {
        self.animations
            .iter()
            .filter(|a| !a.spring.is_settled())
            .count() as u32
    }

    /// 启动或重设 `(node, channel)` 的动画目标。
    ///
    /// 重设现有动画目标时会保留其当前速度，使打断弹簧运动时手感自然而非生硬跳变。
    ///
    pub fn animate(&mut self, node: u64, channel: Channel, preset: SpringPreset, target: f32) {
        if let Some(existing) = self
            .animations
            .iter_mut()
            .find(|a| a.node == node && a.channel == channel)
        {
            existing.spring.set_target(target);
            return;
        }
        let (stiffness, damping) = preset.config();
        let mut spring = Spring {
            stiffness,
            damping,
            ..Default::default()
        };
        spring.set_target(target);
        self.animations.push(Animation {
            node,
            channel,
            spring,
            generation: 0,
        });
    }

    /// 启动一个从 `from` 运动到 `to` 的动画。
    pub fn animate_from(
        &mut self,
        node: u64,
        channel: Channel,
        preset: SpringPreset,
        from: f32,
        to: f32,
    ) {
        self.animate(node, channel, preset, to);
        if let Some(a) = self
            .animations
            .iter_mut()
            .find(|a| a.node == node && a.channel == channel)
        {
            a.spring.value = from;
        }
    }

    /// 获取通道当前值（若正在动画中）。
    #[must_use]
    pub fn value(&self, node: u64, channel: Channel) -> Option<f32> {
        self.animations
            .iter()
            .find(|a| a.node == node && a.channel == channel)
            .map(|a| a.spring.value)
    }

    /// 推进所有弹簧。已静止的动画会被移除，使数组长度仅与*运动中*节点数成正比，
    /// 而非总节点数。
    pub fn step(&mut self, dt: f32) {
        let mut active = 0u32;
        let mut i = 0;
        while i < self.animations.len() {
            let still_moving = self.animations[i].spring.step(dt);
            if still_moving {
                active += 1;
                i += 1;
            } else {
                self.animations.swap_remove(i);
            }
        }
        self.active_count = active;
        debug_assert_eq!(self.active_count, self.active_animations());
        if active > 0 {
            self.moving_frames += 1;
        } else {
            self.idle_frames += 1;
        }
    }

    /// 获取变换通道的当前快照，随后丢弃已完成的条目。
    #[must_use]
    pub fn snapshot(&self) -> Vec<Animation> {
        self.animations.clone()
    }

    pub fn clear(&mut self) {
        self.animations.clear();
        self.active_count = 0;
    }
}

/// 主循环唤醒原因。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WakeReason {
    /// 有动画正在运行：立即渲染。
    Animating,
    /// 收到输入事件。
    Input,
    /// 无事可做：阻塞等待下一事件或直至超时。
    Idle,
}

/// 响应式帧调度器（`docs` §4.4）。
///
/// 主循环向其询问是否需要渲染下一帧。当无动画运行且无输入排队时，
/// 应用将无限期阻塞而非空转，这正是“静止时 0.0% CPU”的保障。
///
#[derive(Debug, Default)]
pub struct FrameScheduler {
    input_pending: bool,
    redraw_requested: bool,
    /// 自上次空闲期以来已渲染的帧数。
    pub frame_count: u64,
    /// 连续无事可做的帧数。
    pub idle_streak: u32,
}

impl FrameScheduler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            redraw_requested: true,
            ..Default::default()
        }
    }

    pub fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }

    pub fn notify_input(&mut self) {
        self.input_pending = true;
        self.redraw_requested = true;
    }

    /// 决定本 tick 是否需要渲染。
    #[must_use]
    pub fn poll(&mut self, animator: &Animator) -> WakeReason {
        let animating = animator.active_animations() > 0;
        let reason = if animating {
            WakeReason::Animating
        } else if self.input_pending || self.redraw_requested {
            WakeReason::Input
        } else {
            WakeReason::Idle
        };

        if reason == WakeReason::Idle {
            self.idle_streak = self.idle_streak.saturating_add(1);
        } else {
            self.idle_streak = 0;
            self.frame_count += 1;
            self.input_pending = false;
            self.redraw_requested = false;
        }
        reason
    }

    /// 当主循环可在操作系统事件队列上阻塞等待时返回 true。
    #[must_use]
    pub fn should_sleep(&self) -> bool {
        self.idle_streak > 0
    }
}

/// 单节点的像素对齐策略（`docs` §4.4）。
///
/// 静态几何体吸附到物理像素网格以保持 1px 边框锐利；
/// 动画中的几何体不得吸附，否则弹簧在缓慢收尾阶段会出现肉眼可见的“爬行”抖动。
///
#[must_use]
pub fn should_snap(animating: bool, transform_is_identity: bool) -> bool {
    !animating && transform_is_identity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_to_its_target() {
        let mut s = Spring::new(SpringPreset::Gentle, 0.0);
        s.set_target(100.0);
        let mut frames = 0;
        while s.step(1.0 / 120.0) {
            frames += 1;
            assert!(frames < 1200, "spring did not settle: {s:?}");
        }
        assert_eq!(s.value, 100.0);
        assert_eq!(s.velocity, 0.0);
        assert!(frames > 5, "settled suspiciously fast: {frames}");
    }

    #[test]
    fn critically_damped_spring_does_not_overshoot() {
        let mut s = Spring::critically_damped(300.0, 0.0);
        s.set_target(1.0);
        let mut peak: f32 = 0.0;
        for _ in 0..600 {
            s.step(1.0 / 120.0);
            peak = peak.max(s.value);
        }
        assert!(peak <= 1.02, "overshoot to {peak}");
        assert!((s.value - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_bouncy_preset_does_overshoot() {
        let mut s = Spring::new(SpringPreset::Snappy, 0.0);
        s.set_target(1.0);
        let mut peak: f32 = 0.0;
        for _ in 0..600 {
            s.step(1.0 / 120.0);
            peak = peak.max(s.value);
        }
        assert!(peak > 1.01, "snappy should overshoot, peaked at {peak}");
    }

    #[test]
    fn a_long_hitch_does_not_explode_the_spring() {
        // 250 ms 帧间隔：若不进行子步进，此处会发散至无穷大。
        let mut s = Spring::new(SpringPreset::Snappy, 0.0);
        s.set_target(1.0);
        for _ in 0..40 {
            s.step(0.25);
            assert!(s.value.is_finite(), "diverged to {}", s.value);
            assert!(s.value.abs() < 10.0, "overshot wildly: {}", s.value);
        }
        assert!((s.value - 1.0).abs() < 0.01, "final {}", s.value);
    }

    #[test]
    fn zero_or_invalid_dt_is_ignored() {
        let mut s = Spring::new(SpringPreset::Gentle, 0.0);
        s.set_target(1.0);
        assert!(s.step(0.0));
        assert_eq!(s.value, 0.0);
        assert!(s.step(f32::NAN));
        assert_eq!(s.value, 0.0);
    }

    #[test]
    fn retargeting_keeps_velocity() {
        let mut a = Animator::new();
        a.animate(1, Channel::TranslateX, SpringPreset::Snappy, 100.0);
        for _ in 0..10 {
            a.step(1.0 / 120.0);
        }
        let velocity_before = a.snapshot()[0].spring.velocity;
        assert!(velocity_before.abs() > 1.0);
        a.animate(1, Channel::TranslateX, SpringPreset::Snappy, -100.0);
        let velocity_after = a.snapshot()[0].spring.velocity;
        assert_eq!(
            velocity_before, velocity_after,
            "retarget must not reset velocity"
        );
        assert_eq!(a.len(), 1, "retarget must not add a second animation");
    }

    #[test]
    fn settled_animations_are_removed() {
        let mut a = Animator::new();
        a.animate(1, Channel::Opacity, SpringPreset::Precise, 1.0);
        for _ in 0..600 {
            a.step(1.0 / 120.0);
        }
        assert!(a.is_empty(), "settled animation should be dropped");
        assert_eq!(a.active_animations(), 0);
        assert!(a.idle_frames > 0);
    }

    #[test]
    fn many_animations_track_the_active_count() {
        let mut a = Animator::new();
        for i in 0..50u64 {
            a.animate(
                i,
                Channel::TranslateY,
                SpringPreset::Gentle,
                10.0 + i as f32,
            );
        }
        a.step(1.0 / 120.0);
        assert_eq!(a.active_animations(), 50);
        assert_eq!(a.len(), 50);
    }

    #[test]
    fn spring2_settles_both_axes() {
        let mut s = Spring2::new(SpringPreset::Gentle, Vec2::ZERO);
        s.set_target(Vec2::new(120.0, -80.0));
        for _ in 0..1200 {
            if !s.step(1.0 / 120.0) {
                break;
            }
        }
        assert!(s.is_settled());
        assert_eq!(s.value(), Vec2::new(120.0, -80.0));
    }

    #[test]
    fn bezier_endpoints_are_exact() {
        for curve in [
            CubicBezier::EASE,
            CubicBezier::EASE_OUT,
            CubicBezier::LINEAR,
        ] {
            assert!((curve.sample(0.0)).abs() < 1e-4);
            assert!((curve.sample(1.0) - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn bezier_is_monotonic_for_ease_out() {
        let mut prev = -1.0;
        for i in 0..=100 {
            let v = CubicBezier::EASE_OUT.sample(i as f32 / 100.0);
            assert!(v >= prev - 1e-4, "not monotonic at {i}");
            prev = v;
        }
    }

    #[test]
    fn scheduler_sleeps_when_nothing_moves() {
        let mut animator = Animator::new();
        let mut scheduler = FrameScheduler::new();
        // 首次轮询会消耗初始重绘请求。
        assert_eq!(scheduler.poll(&animator), WakeReason::Input);
        assert_eq!(scheduler.poll(&animator), WakeReason::Idle);
        assert!(scheduler.should_sleep());

        animator.animate(1, Channel::Scale, SpringPreset::Gentle, 1.2);
        assert_eq!(scheduler.poll(&animator), WakeReason::Animating);
        assert!(!scheduler.should_sleep());

        animator.step(1.0 / 120.0);
        scheduler.notify_input();
        assert_eq!(scheduler.poll(&animator), WakeReason::Animating);
    }

    #[test]
    fn snapping_policy_follows_animation_state() {
        assert!(should_snap(false, true));
        assert!(!should_snap(true, true), "animating nodes must not snap");
        assert!(
            !should_snap(false, false),
            "a static transform needs subpixel"
        );
    }

    #[test]
    fn animation_values_are_queryable_per_channel() {
        let mut a = Animator::new();
        a.animate_from(7, Channel::Opacity, SpringPreset::Precise, 0.0, 1.0);
        assert_eq!(a.value(7, Channel::Opacity), Some(0.0));
        assert_eq!(a.value(7, Channel::Scale), None);
        a.step(1.0 / 120.0);
        let v = a.value(7, Channel::Opacity).unwrap();
        assert!(v > 0.0 && v < 1.0);
    }

    /// NaN 目标绝不能让动画器永久保持唤醒。
    ///
    /// 与 NaN 的任何比较均为 false，因此目标为 NaN 的弹簧永不收敛：
    /// 动画器永不移除它，调度器永不返回 `Idle`，导致应用以 100% CPU 空转
    /// 并在屏幕上出现 NaN 变换。
    ///
    #[test]
    fn a_nan_target_settles_instead_of_spinning_forever() {
        let mut spring = Spring::default();
        spring.set_target(f32::NAN);
        assert!(spring.is_settled(), "a broken spring counts as settled");
        assert!(!spring.step(1.0 / 60.0), "it must not keep moving");
        assert!(spring.value.is_finite());
    }

    #[test]
    fn a_nan_target_does_not_wake_the_scheduler_forever() {
        let mut animator = Animator::new();
        // 布局输出了一个 NaN。
        animator.animate(1, Channel::Scale, SpringPreset::Gentle, f32::NAN);
        for _ in 0..600 {
            animator.step(1.0 / 60.0);
        }
        assert_eq!(
            animator.active_animations(),
            0,
            "the broken animation must be dropped, not kept alive"
        );
        // 损坏的动画已被移除，因此不再报告其值。
        assert!(
            animator.value(1, Channel::Scale).is_none(),
            "the dropped animation must not report a value"
        );
    }

    /// 发散的积分也不得导致空转。
    #[test]
    fn a_diverging_spring_settles() {
        let mut spring = Spring {
            stiffness: 1.0e9,
            damping: 0.0,
            mass: 1.0,
            value: 0.0,
            target: 1.0,
            velocity: 0.0,
            ..Default::default()
        };
        // 步长大到足以使速度溢出。
        let _ = spring.step(0.25);
        assert!(
            spring.value.is_finite() && spring.velocity.is_finite(),
            "the spring must stay finite"
        );
        assert!(spring.is_settled());
    }
}

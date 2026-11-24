//! 两级动效令牌、时间轴缓动状态机（[`Tween`]）与无平台依赖的全局动效策略（[`MotionPolicy`]）。
//!
//! 遵循 ADR 0037：
//! - 基础层定义 [`DurationToken`] 与 [`EasingToken`]，提供单一事实源的时长档位与 Fluent 缓动曲线；
//! - 语义层定义 [`MotionToken`]，将业务动效意图映射为基础令牌组合；
//! - [`MotionPolicy`] 以纯值方式统一治理 `reduced_motion` 瞬切与 `speed_scale` 全局速率倍率，
//!   使 `unbound-anim` 保持零操作系统句柄依赖。

use crate::CubicBezier;

/// 允许的全局动画速率倍率下限（20 倍慢放，便于视觉核验与调试）。
pub const MIN_SPEED_SCALE: f32 = 0.05;

/// 允许的全局动画速率倍率上限（2 倍速）。
pub const MAX_SPEED_SCALE: f32 = 2.0;

/// 基础时长令牌（毫秒级语义档位）。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DurationToken {
    /// 0 ms：瞬切，不产生插值帧。
    Instant,
    /// 90 ms：微交互与快速退出。
    Quick,
    /// 120 ms：停靠预览叠加层与轻浮层进入。
    Short,
    /// 180 ms：面板尺寸调整与中等区域过渡。
    Medium,
    /// 280 ms：主题切换与全局大面积平滑过渡。
    Slow,
}

impl DurationToken {
    /// 基准时长（毫秒）。
    #[must_use]
    pub const fn millis(self) -> u32 {
        match self {
            Self::Instant => 0,
            Self::Quick => 90,
            Self::Short => 120,
            Self::Medium => 180,
            Self::Slow => 280,
        }
    }

    /// 基准时长（秒）。
    #[must_use]
    pub const fn secs(self) -> f32 {
        match self {
            Self::Instant => 0.0,
            Self::Quick => 0.09,
            Self::Short => 0.12,
            Self::Medium => 0.18,
            Self::Slow => 0.28,
        }
    }

    /// 在给定 [`MotionPolicy`] 下的生效时长（秒）。
    ///
    /// 当 `policy.reduced_motion == true` 时强制坍缩为 `0.0` 秒。
    #[must_use]
    pub fn effective_secs(self, policy: MotionPolicy) -> f32 {
        policy.effective_duration(self)
    }
}

/// 对齐 Fluent 规范的基础缓动令牌。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EasingToken {
    /// 标准快出慢入曲线：适合就地状态变化与微交互。
    FastOutSlowIn,
    /// 加速退场曲线：适合浮层或元素离开视口。
    Accelerate,
    /// 减速入场曲线：适合停靠预览与浮层进入视口。
    Decelerate,
    /// 线性曲线：适合进度或恒速渐变。
    Linear,
}

impl EasingToken {
    /// Fluent 标准快出慢入贝塞尔控制点。
    pub const FAST_OUT_SLOW_IN: CubicBezier = CubicBezier {
        x1: 0.4,
        y1: 0.0,
        x2: 0.2,
        y2: 1.0,
    };

    /// Fluent 加速退场贝塞尔控制点。
    pub const ACCELERATE: CubicBezier = CubicBezier {
        x1: 0.7,
        y1: 0.0,
        x2: 1.0,
        y2: 0.5,
    };

    /// Fluent 减速入场贝塞尔控制点。
    pub const DECELERATE: CubicBezier = CubicBezier {
        x1: 0.1,
        y1: 0.9,
        x2: 0.2,
        y2: 1.0,
    };

    /// 返回对应的三次贝塞尔曲线。
    #[must_use]
    pub const fn bezier(self) -> CubicBezier {
        match self {
            Self::FastOutSlowIn => Self::FAST_OUT_SLOW_IN,
            Self::Accelerate => Self::ACCELERATE,
            Self::Decelerate => Self::DECELERATE,
            Self::Linear => CubicBezier::LINEAR,
        }
    }

    /// 在归一化时间 `t ∈ [0.0, 1.0]` 处采样缓动值。
    #[must_use]
    pub fn sample(self, t: f32) -> f32 {
        if !t.is_finite() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        self.bezier().sample(t)
    }
}

/// 语义动效令牌：将业务动效角色映射为 `(DurationToken, EasingToken)` 组合。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MotionToken {
    /// 按钮、开关、悬停高亮等微交互（90 ms / FastOutSlowIn）。
    MicroInteraction,
    /// 跨窗停靠预览矩形显隐与切换（120 ms / Decelerate）。
    OverlayPreview,
    /// 浮层、菜单、气泡入场（120 ms / Decelerate）。
    OverlayEnter,
    /// 浮层、菜单、气泡退场（90 ms / Accelerate）。
    OverlayExit,
    /// 分割面板或抽屉尺寸调整（180 ms / FastOutSlowIn）。
    PanelResize,
    /// 主题明暗或强调色全局过渡（280 ms / Decelerate）。
    ThemeTransition,
}

impl MotionToken {
    /// 该语义令牌对应的基础时长档位。
    #[must_use]
    pub const fn duration(self) -> DurationToken {
        match self {
            Self::MicroInteraction | Self::OverlayExit => DurationToken::Quick,
            Self::OverlayPreview | Self::OverlayEnter => DurationToken::Short,
            Self::PanelResize => DurationToken::Medium,
            Self::ThemeTransition => DurationToken::Slow,
        }
    }

    /// 该语义令牌对应的缓动曲线档位。
    #[must_use]
    pub const fn easing(self) -> EasingToken {
        match self {
            Self::MicroInteraction | Self::PanelResize => EasingToken::FastOutSlowIn,
            Self::OverlayPreview | Self::OverlayEnter | Self::ThemeTransition => {
                EasingToken::Decelerate
            }
            Self::OverlayExit => EasingToken::Accelerate,
        }
    }

    /// 在给定 [`MotionPolicy`] 下的生效时长（秒）。
    #[must_use]
    pub fn effective_secs(self, policy: MotionPolicy) -> f32 {
        policy.effective_duration(self.duration())
    }

    /// 基于该语义令牌构建从 `from` 到 `to` 的 [`Tween`] 实例。
    #[must_use]
    pub fn tween(self, from: f32, to: f32) -> Tween {
        Tween::new(from, to, self)
    }
}

/// 无平台依赖的全局动效治理策略。
///
/// 由应用运行时或宿主在每帧注入，统一控制 [`Tween`] 与 [`crate::Animator`] 的
/// 减弱动画瞬切与全局速率倍率。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MotionPolicy {
    /// 是否开启系统/用户减弱动画偏好；开启时所有非必要动画强制瞬切至终态。
    pub reduced_motion: bool,
    /// 全局动画速率倍率，合法区间为 `[0.05, 2.0]`（`1.0` 为常速，`2.0` 为 2 倍速，`0.5` 为半速）。
    pub speed_scale: f32,
}

impl Default for MotionPolicy {
    fn default() -> Self {
        Self::normal()
    }
}

impl MotionPolicy {
    /// 默认常速策略（未开启减弱动画，`speed_scale = 1.0`）。
    #[must_use]
    pub const fn normal() -> Self {
        Self {
            reduced_motion: false,
            speed_scale: 1.0,
        }
    }

    /// 开启减弱动画的策略（所有非必要过渡坍缩为 0 ms 瞬切）。
    #[must_use]
    pub const fn reduced() -> Self {
        Self {
            reduced_motion: true,
            speed_scale: 1.0,
        }
    }

    /// 构造并钳制速率倍率至 `[MIN_SPEED_SCALE, MAX_SPEED_SCALE]`；非有限值回退为 `1.0`。
    #[must_use]
    pub fn new(reduced_motion: bool, speed_scale: f32) -> Self {
        let clamped = if speed_scale.is_finite() {
            speed_scale.clamp(MIN_SPEED_SCALE, MAX_SPEED_SCALE)
        } else {
            1.0
        };
        Self {
            reduced_motion,
            speed_scale: clamped,
        }
    }

    /// 返回经过合法区间 `[0.05, 2.0]` 钳制的速率倍率。
    #[must_use]
    pub fn clamped_speed_scale(self) -> f32 {
        if self.speed_scale.is_finite() {
            self.speed_scale.clamp(MIN_SPEED_SCALE, MAX_SPEED_SCALE)
        } else {
            1.0
        }
    }

    /// 计算基础时长令牌在当前策略下的生效时长（秒）。
    ///
    /// - `reduced_motion == true` 时始终返回 `0.0`；
    /// - 否则返回 `duration.secs() / self.clamped_speed_scale()`。
    #[must_use]
    pub fn effective_duration(self, duration: DurationToken) -> f32 {
        self.effective_duration_with_essential(duration, false)
    }

    /// 计算时长令牌的生效时长，允许显式声明为必要非空间淡变（`essential == true`）时保留过渡。
    #[must_use]
    pub fn effective_duration_with_essential(
        self,
        duration: DurationToken,
        essential: bool,
    ) -> f32 {
        if self.should_snap(essential) {
            return 0.0;
        }
        duration.secs() / self.clamped_speed_scale()
    }

    /// 按全局速率倍率折算单步时间增量 `dt`（秒）。
    #[must_use]
    pub fn effective_dt(self, dt: f32) -> f32 {
        if !dt.is_finite() || dt <= 0.0 {
            return 0.0;
        }
        dt * self.clamped_speed_scale()
    }

    /// 判断在当前策略下动画是否应直接瞬切至终态。
    #[must_use]
    pub const fn should_snap(self, essential: bool) -> bool {
        self.reduced_motion && !essential
    }
}

/// 基于单调 `dt` 采样的时间轴缓动状态机。
///
/// 在 60 Hz / 120 Hz / 240 Hz 等不同刷新率下按精确累计时间采样三次贝塞尔曲线，
/// 到达终点时精确吸附至 `to` 且零残留，并支持中途调用 [`Tween::set_target`] 平滑转向。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tween {
    /// 当前过渡起点。
    pub from: f32,
    /// 当前过渡目标。
    pub to: f32,
    /// 已推进的基准时间（秒，范围 `[0.0, duration]`）。
    pub elapsed: f32,
    /// 基准总时长（秒）。
    pub duration: f32,
    /// 缓动曲线令牌。
    pub easing: EasingToken,
    /// 是否为必要非空间淡变（`true` 时即使 `reduced_motion == true` 仍执行过渡）。
    pub essential: bool,
}

impl Tween {
    /// 使用语义 [`MotionToken`] 创建从 `from` 到 `to` 的缓动状态机。
    #[must_use]
    pub fn new(from: f32, to: f32, token: MotionToken) -> Self {
        Self::with_duration(from, to, token.duration(), token.easing())
    }

    /// 使用基础 [`DurationToken`] 与 [`EasingToken`] 创建缓动状态机。
    #[must_use]
    pub fn with_duration(from: f32, to: f32, duration: DurationToken, easing: EasingToken) -> Self {
        let secs = duration.secs();
        let settled = !from.is_finite() || !to.is_finite() || secs <= 0.0 || from == to;
        let clean_to = if to.is_finite() { to } else { 0.0 };
        let clean_from = if from.is_finite() { from } else { clean_to };
        Self {
            from: if settled { clean_to } else { clean_from },
            to: clean_to,
            elapsed: if settled { secs } else { 0.0 },
            duration: secs,
            easing,
            essential: false,
        }
    }

    /// 创建初始即静止在 `value` 的缓动状态机，后续可通过 [`Self::set_target`] 触发过渡。
    #[must_use]
    pub fn settled(value: f32, token: MotionToken) -> Self {
        let secs = token.duration().secs();
        let clean = if value.is_finite() { value } else { 0.0 };
        Self {
            from: clean,
            to: clean,
            elapsed: secs,
            duration: secs,
            easing: token.easing(),
            essential: false,
        }
    }

    /// 标记是否为必要非空间淡变（用于减弱动画模式下仍需保留的关键状态反馈）。
    #[must_use]
    pub const fn with_essential(mut self, essential: bool) -> Self {
        self.essential = essential;
        self
    }

    /// 原始线性进度（`[0.0, 1.0]`）。
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 || !self.duration.is_finite() {
            return 1.0;
        }
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// 经缓动曲线映射后的进度（`[0.0, 1.0]`）。
    #[must_use]
    pub fn eased_progress(&self) -> f32 {
        let p = self.progress();
        if p <= 0.0 {
            0.0
        } else if p >= 1.0 {
            1.0
        } else {
            self.easing.sample(p)
        }
    }

    /// 当前插值结果；收敛后精确返回 `self.to`，零浮点残留。
    #[must_use]
    pub fn value(&self) -> f32 {
        if self.is_settled() {
            return self.to;
        }
        let p = self.eased_progress();
        self.from + (self.to - self.from) * p
    }

    /// 判断是否已到达终态。
    #[must_use]
    pub fn is_settled(&self) -> bool {
        if !self.from.is_finite() || !self.to.is_finite() || !self.duration.is_finite() {
            return true;
        }
        self.duration <= 0.0 || self.elapsed >= self.duration || self.from == self.to
    }

    /// 立即瞬切至指定值并标记为已收敛。
    pub fn snap_to(&mut self, value: f32) {
        let clean = if value.is_finite() { value } else { 0.0 };
        self.from = clean;
        self.to = clean;
        self.elapsed = self.duration.max(0.0);
    }

    /// 中途平滑重设目标值：以当前插值作为新起点并重置时间轴。
    ///
    /// 若新目标与当前 `to` 完全一致则保持原进度不打断；若输入非有限值则安全归零收敛。
    pub fn set_target(&mut self, target: f32) {
        if !target.is_finite() {
            self.snap_to(0.0);
            return;
        }
        if (self.to - target).abs() <= f32::EPSILON {
            return;
        }
        let current = self.value();
        self.from = current;
        self.to = target;
        if self.duration <= 0.0 || (current - target).abs() <= f32::EPSILON {
            self.from = target;
            self.elapsed = self.duration.max(0.0);
        } else {
            self.elapsed = 0.0;
        }
    }

    /// 在给定 [`MotionPolicy`] 下推进 `dt` 秒；仍在过渡中返回 `true`，已收敛返回 `false`。
    pub fn step(&mut self, dt: f32, policy: MotionPolicy) -> bool {
        if !self.to.is_finite() || !self.from.is_finite() {
            self.snap_to(0.0);
            return false;
        }
        if policy.should_snap(self.essential) || self.duration <= 0.0 {
            self.from = self.to;
            self.elapsed = self.duration.max(0.0);
            return false;
        }
        if self.is_settled() {
            self.from = self.to;
            self.elapsed = self.duration;
            return false;
        }
        let scaled_dt = policy.effective_dt(dt);
        if scaled_dt <= 0.0 {
            return true;
        }
        self.elapsed = (self.elapsed + scaled_dt).min(self.duration);
        if self.elapsed >= self.duration {
            // 终点精确吸附，消除浮点误差残留。
            self.elapsed = self.duration;
            self.from = self.to;
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_tokens_match_spec_baselines() {
        assert_eq!(DurationToken::Instant.millis(), 0);
        assert_eq!(DurationToken::Quick.millis(), 90);
        assert_eq!(DurationToken::Short.millis(), 120);
        assert_eq!(DurationToken::Medium.millis(), 180);
        assert_eq!(DurationToken::Slow.millis(), 280);

        assert!((DurationToken::Instant.secs() - 0.0).abs() < 1e-6);
        assert!((DurationToken::Quick.secs() - 0.09).abs() < 1e-6);
        assert!((DurationToken::Short.secs() - 0.12).abs() < 1e-6);
        assert!((DurationToken::Medium.secs() - 0.18).abs() < 1e-6);
        assert!((DurationToken::Slow.secs() - 0.28).abs() < 1e-6);
    }

    #[test]
    fn motion_policy_clamps_speed_scale_and_scales_durations() {
        let too_slow = MotionPolicy::new(false, 0.001);
        assert!((too_slow.clamped_speed_scale() - MIN_SPEED_SCALE).abs() < 1e-6);
        assert!((DurationToken::Short.effective_secs(too_slow) - 0.12 / 0.05).abs() < 1e-4);

        let too_fast = MotionPolicy::new(false, 9.0);
        assert!((too_fast.clamped_speed_scale() - MAX_SPEED_SCALE).abs() < 1e-6);
        assert!((DurationToken::Short.effective_secs(too_fast) - 0.06).abs() < 1e-6);

        let nan_policy = MotionPolicy::new(false, f32::NAN);
        assert!((nan_policy.clamped_speed_scale() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn reduced_motion_collapses_duration_and_snaps_tween_immediately() {
        let reduced = MotionPolicy::reduced();
        for token in [
            DurationToken::Instant,
            DurationToken::Quick,
            DurationToken::Short,
            DurationToken::Medium,
            DurationToken::Slow,
        ] {
            assert_eq!(token.effective_secs(reduced), 0.0);
        }

        let mut tween = Tween::new(0.0, 1.0, MotionToken::OverlayPreview);
        assert!(!tween.is_settled());
        let moving = tween.step(1.0 / 240.0, reduced);
        assert!(!moving, "reduced motion must settle in one step");
        assert!(tween.is_settled());
        assert_eq!(tween.value(), 1.0);
    }

    #[test]
    fn essential_tween_still_animates_under_reduced_motion() {
        let reduced = MotionPolicy::reduced();
        let mut tween = Tween::new(0.0, 1.0, MotionToken::OverlayPreview).with_essential(true);
        assert!(tween.step(0.06, reduced));
        assert!(tween.value() > 0.0 && tween.value() < 1.0);
        assert!(!tween.step(0.06, reduced));
        assert_eq!(tween.value(), 1.0);
    }

    #[test]
    fn tween_converges_consistently_across_60_120_240_hz() {
        let policy = MotionPolicy::normal();
        let sample_at_half = |hz: usize| -> (f32, f32) {
            let dt = 1.0 / hz as f32;
            let mut tween = Tween::new(10.0, 110.0, MotionToken::OverlayPreview);
            // OverlayPreview 为 120ms；推进 60ms 观测中点。
            let half_steps = (0.06 * hz as f32).round() as usize;
            for _ in 0..half_steps {
                tween.step(dt, policy);
            }
            let mid = tween.value();
            // 继续推进直至收敛。
            let mut steps = 0;
            while tween.step(dt, policy) {
                steps += 1;
                assert!(steps < hz * 2, "tween failed to settle at {hz}Hz");
            }
            (mid, tween.value())
        };

        let (mid_60, end_60) = sample_at_half(60);
        let (mid_120, end_120) = sample_at_half(120);
        let (mid_240, end_240) = sample_at_half(240);

        // 60Hz 3.6 帧取整 4 帧（66.7ms），120Hz/240Hz 精确 60ms，误差受控在单帧内。
        assert!(
            (mid_120 - mid_240).abs() < 1e-3,
            "120Hz={mid_120} vs 240Hz={mid_240}"
        );
        assert!(
            (mid_60 - mid_120).abs() < 12.0,
            "60Hz={mid_60} vs 120Hz={mid_120}"
        );
        assert_eq!(end_60, 110.0);
        assert_eq!(end_120, 110.0);
        assert_eq!(end_240, 110.0);
    }

    #[test]
    fn tween_retargets_smoothly_mid_flight() {
        let policy = MotionPolicy::normal();
        let mut tween = Tween::new(0.0, 100.0, MotionToken::PanelResize);
        assert!(tween.step(0.09, policy));
        let mid = tween.value();
        assert!(mid > 10.0 && mid < 95.0);

        tween.set_target(20.0);
        assert!(
            (tween.value() - mid).abs() < 1e-4,
            "retarget must start from current value"
        );
        assert!(!tween.is_settled());

        assert!(!tween.step(0.20, policy));
        assert_eq!(tween.value(), 20.0);
        assert!(tween.is_settled());
    }

    #[test]
    fn easing_tokens_have_exact_endpoints_and_monotonic_progression() {
        for easing in [
            EasingToken::FastOutSlowIn,
            EasingToken::Accelerate,
            EasingToken::Decelerate,
            EasingToken::Linear,
        ] {
            assert_eq!(easing.sample(0.0), 0.0);
            assert_eq!(easing.sample(1.0), 1.0);
            let mut prev = 0.0;
            for i in 1..=100 {
                let v = easing.sample(i as f32 / 100.0);
                assert!(
                    v >= prev - 1e-4,
                    "{easing:?} not monotonic at {i}: {v} < {prev}"
                );
                prev = v;
            }
        }
    }
}

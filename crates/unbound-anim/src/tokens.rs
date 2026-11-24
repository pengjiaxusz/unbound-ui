//! 两级动效令牌、单调 `dt` 缓动状态机与无平台依赖策略。
//!
//! 本模块为 `unbound-anim` 提供统一的动效单一事实源：
//! * **基础令牌层**：[`DurationToken`] 定义标准化毫秒档位，[`EasingToken`] 对齐 Fluent 贝塞尔曲线。
//! * **语义令牌层**：[`MotionToken`] 将常见交互与浮层过渡映射为基础时长与缓动组合，并支持区分必要非空间淡变。
//! * **统一策略层**：[`MotionPolicy`] 以纯值形式集中治理 `reduced_motion` 与 `speed_scale`，保持零平台依赖。
//! * **时间轴插值**：[`Tween`] 基于单调 `dt` 步进与精确 [`CubicBezier`] 采样，保证高刷新率下行为一致且终点精确吸附零残留。

use crate::CubicBezier;

/// 基础时长令牌。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DurationToken {
    /// 瞬切：0 ms。
    Instant,
    /// 快速微交互与退出：90 ms。
    Quick,
    /// 短过渡与浮层预览：120 ms。
    Short,
    /// 中等布局与面板调整：180 ms。
    Medium,
    /// 较慢的全局或主题过渡：280 ms。
    Slow,
}

impl DurationToken {
    /// 返回令牌对应的基准毫秒数。
    #[must_use]
    pub const fn as_millis(self) -> u32 {
        match self {
            Self::Instant => 0,
            Self::Quick => 90,
            Self::Short => 120,
            Self::Medium => 180,
            Self::Slow => 280,
        }
    }

    /// 返回令牌对应的基准秒数。
    #[must_use]
    pub const fn as_secs_f32(self) -> f32 {
        match self {
            Self::Instant => 0.0,
            Self::Quick => 0.09,
            Self::Short => 0.12,
            Self::Medium => 0.18,
            Self::Slow => 0.28,
        }
    }

    /// 按 [`MotionPolicy`] 计算生效时长（秒）。
    ///
    /// 当 `policy.reduced_motion` 为 `true` 时，所有时长令牌均强制坍缩为 `0.0`。
    #[must_use]
    pub fn effective_secs(self, policy: MotionPolicy) -> f32 {
        if policy.reduced_motion || matches!(self, Self::Instant) {
            return 0.0;
        }
        self.as_secs_f32() / policy.speed_scale()
    }
}

/// 基础缓动令牌，对齐 Fluent 设计体系三次贝塞尔曲线。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EasingToken {
    /// 标准点对点曲线（Fluent FastOutSlowIn）：快速启动、平滑减速收敛。
    FastOutSlowIn,
    /// 退出加速曲线（Fluent Accelerate）：用于元素离场与收起。
    Accelerate,
    /// 进入减速曲线（Fluent Decelerate）：用于元素入场与浮层展开。
    Decelerate,
    /// 线性曲线：用于透明度淡变或进度指示。
    Linear,
}

impl EasingToken {
    /// 返回对应的 [`CubicBezier`] 控制点曲线。
    #[must_use]
    pub const fn curve(self) -> CubicBezier {
        match self {
            Self::FastOutSlowIn => CubicBezier {
                x1: 0.8,
                y1: 0.0,
                x2: 0.2,
                y2: 1.0,
            },
            Self::Accelerate => CubicBezier {
                x1: 0.7,
                y1: 0.0,
                x2: 1.0,
                y2: 0.5,
            },
            Self::Decelerate => CubicBezier {
                x1: 0.1,
                y1: 0.9,
                x2: 0.2,
                y2: 1.0,
            },
            Self::Linear => CubicBezier::LINEAR,
        }
    }

    /// 对归一化时间 `t`（`0.0..=1.0`）执行精确缓动采样。
    ///
    /// 边界处严格返回 `0.0` 与 `1.0`，避免端点浮点残留。
    #[must_use]
    pub fn sample(self, t: f32) -> f32 {
        if !t.is_finite() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        if matches!(self, Self::Linear) {
            return t;
        }
        self.curve().sample(t)
    }
}

/// 语义动效令牌，将界面交互场景映射为基础时长与缓动组合。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MotionToken {
    /// 按钮悬停、按压与状态切换等微交互（90 ms / FastOutSlowIn）。
    MicroInteraction,
    /// 停靠区域预览叠加层显隐与目标切换（120 ms / Decelerate）。
    OverlayPreview,
    /// 浮层、菜单与弹出面板切入（120 ms / Decelerate）。
    OverlayEnter,
    /// 浮层、菜单与弹出面板退出（90 ms / Accelerate）。
    OverlayExit,
    /// 分栏与面板尺寸调整过渡（180 ms / FastOutSlowIn）。
    PaneResize,
    /// 主题色板与明暗模式切换过渡（280 ms / FastOutSlowIn）。
    ThemeTransition,
    /// 必要的非空间透明度淡变（120 ms / Linear），减弱动画开启时仍允许平滑淡变。
    EssentialFade,
}

impl MotionToken {
    /// 返回语义令牌对应的基础时长令牌。
    #[must_use]
    pub const fn duration(self) -> DurationToken {
        match self {
            Self::MicroInteraction | Self::OverlayExit => DurationToken::Quick,
            Self::OverlayPreview | Self::OverlayEnter | Self::EssentialFade => DurationToken::Short,
            Self::PaneResize => DurationToken::Medium,
            Self::ThemeTransition => DurationToken::Slow,
        }
    }

    /// 返回语义令牌对应的基础缓动令牌。
    #[must_use]
    pub const fn easing(self) -> EasingToken {
        match self {
            Self::MicroInteraction | Self::PaneResize | Self::ThemeTransition => {
                EasingToken::FastOutSlowIn
            }
            Self::OverlayPreview | Self::OverlayEnter => EasingToken::Decelerate,
            Self::OverlayExit => EasingToken::Accelerate,
            Self::EssentialFade => EasingToken::Linear,
        }
    }

    /// 指示该令牌是否属于显式声明的“必要非空间淡变”。
    ///
    /// 当 `reduced_motion == true` 时，仅此类令牌保留非零过渡时长，其余令牌一律坍缩为瞬切。
    #[must_use]
    pub const fn is_essential_non_spatial(self) -> bool {
        matches!(self, Self::EssentialFade)
    }

    /// 按 [`MotionPolicy`] 计算该语义令牌的生效时长（秒）。
    #[must_use]
    pub fn effective_secs(self, policy: MotionPolicy) -> f32 {
        if self.is_essential_non_spatial() {
            let base = self.duration().as_secs_f32();
            if base <= 0.0 {
                0.0
            } else {
                base / policy.speed_scale()
            }
        } else {
            self.duration().effective_secs(policy)
        }
    }
}

/// 全局动效治理策略（纯值类型，零平台依赖）。
///
/// 通过 `reduced_motion` 与 `speed_scale` 统一约束 [`Tween`] 与 [`crate::Animator`]。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MotionPolicy {
    /// 是否开启减弱动画；为 `true` 时时长令牌坍缩为 `0ms` 并触发瞬切。
    pub reduced_motion: bool,
    /// 全局动效速率倍率，严格钳制在 `0.05..=2.0` 区间。
    pub speed_scale: f32,
}

impl Default for MotionPolicy {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            speed_scale: 1.0,
        }
    }
}

impl MotionPolicy {
    /// 允许的最小速率倍率（20 倍慢放）。
    pub const MIN_SPEED_SCALE: f32 = 0.05;
    /// 允许的最大速率倍率（2 倍加速）。
    pub const MAX_SPEED_SCALE: f32 = 2.0;

    /// 构造动效策略并立即将 `speed_scale` 钳制至合法区间。
    #[must_use]
    pub fn new(reduced_motion: bool, speed_scale: f32) -> Self {
        Self {
            reduced_motion,
            speed_scale: Self::clamp_speed_scale(speed_scale),
        }
    }

    /// 构造已开启减弱动画的策略。
    #[must_use]
    pub const fn reduced() -> Self {
        Self {
            reduced_motion: true,
            speed_scale: 1.0,
        }
    }

    /// 将任意速率倍率严格钳制到 `0.05..=2.0`；非有限值回退为 `1.0`。
    #[must_use]
    pub fn clamp_speed_scale(scale: f32) -> f32 {
        if !scale.is_finite() {
            return 1.0;
        }
        scale.clamp(Self::MIN_SPEED_SCALE, Self::MAX_SPEED_SCALE)
    }

    /// 返回经过合法性钳制后的速率倍率。
    #[must_use]
    pub fn speed_scale(self) -> f32 {
        Self::clamp_speed_scale(self.speed_scale)
    }

    /// 返回规范化后的策略副本（确保直接字面量构造的字段也被钳制）。
    #[must_use]
    pub fn normalized(self) -> Self {
        Self::new(self.reduced_motion, self.speed_scale)
    }

    /// 返回更新了 `reduced_motion` 标志的策略副本。
    #[must_use]
    pub fn with_reduced_motion(mut self, reduced_motion: bool) -> Self {
        self.reduced_motion = reduced_motion;
        self.normalized()
    }

    /// 返回更新并钳制了 `speed_scale` 的策略副本。
    #[must_use]
    pub fn with_speed_scale(mut self, speed_scale: f32) -> Self {
        self.speed_scale = Self::clamp_speed_scale(speed_scale);
        self
    }

    /// 原地更新并钳制 `speed_scale`。
    pub fn set_speed_scale(&mut self, speed_scale: f32) {
        self.speed_scale = Self::clamp_speed_scale(speed_scale);
    }

    /// 按策略速率倍率缩放帧时间步长 `dt`；负值或非有限值归零。
    #[must_use]
    pub fn scale_dt(self, dt: f32) -> f32 {
        if !dt.is_finite() || dt <= 0.0 {
            return 0.0;
        }
        dt * self.speed_scale()
    }
}

/// 基于单调 `dt` 步进与精确 [`CubicBezier`] 采样的时间轴缓动状态机。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    elapsed: f32,
    duration: f32,
    easing: EasingToken,
    essential_non_spatial: bool,
}

impl Tween {
    /// 创建一个初始已静止在 `initial` 处的 [`Tween`]。
    #[must_use]
    pub fn new(initial: f32, duration_secs: f32, easing: EasingToken) -> Self {
        let initial = sanitize_finite(initial);
        let duration = sanitize_duration(duration_secs);
        Self {
            from: initial,
            to: initial,
            elapsed: duration,
            duration,
            easing,
            essential_non_spatial: false,
        }
    }

    /// 创建一个从 `from` 过渡到 `to` 的 [`Tween`]。
    #[must_use]
    pub fn between(from: f32, to: f32, duration_secs: f32, easing: EasingToken) -> Self {
        let from = sanitize_finite(from);
        let to = sanitize_finite(to);
        let duration = sanitize_duration(duration_secs);
        let elapsed = if duration <= 0.0 || from == to {
            duration
        } else {
            0.0
        };
        Self {
            from,
            to,
            elapsed,
            duration,
            easing,
            essential_non_spatial: false,
        }
    }

    /// 基于语义 [`MotionToken`] 创建初始已静止在 `initial` 处的 [`Tween`]。
    #[must_use]
    pub fn from_token(initial: f32, token: MotionToken) -> Self {
        Self::new(initial, token.duration().as_secs_f32(), token.easing())
            .with_essential_non_spatial(token.is_essential_non_spatial())
    }

    /// 基于语义 [`MotionToken`] 创建从 `from` 过渡到 `to` 的 [`Tween`]。
    #[must_use]
    pub fn from_token_between(from: f32, to: f32, token: MotionToken) -> Self {
        Self::between(from, to, token.duration().as_secs_f32(), token.easing())
            .with_essential_non_spatial(token.is_essential_non_spatial())
    }

    /// 显式标记该插值器是否为必要非空间淡变。
    #[must_use]
    pub const fn with_essential_non_spatial(mut self, essential: bool) -> Self {
        self.essential_non_spatial = essential;
        self
    }

    /// 返回该插值器是否标记为必要非空间淡变。
    #[must_use]
    pub const fn is_essential_non_spatial(&self) -> bool {
        self.essential_non_spatial
    }

    /// 返回当前过渡段的起点值。
    #[must_use]
    pub const fn from(&self) -> f32 {
        self.from
    }

    /// 返回当前目标值。
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.to
    }

    /// 返回当前配置的基准时长（秒）。
    #[must_use]
    pub const fn duration_secs(&self) -> f32 {
        self.duration
    }

    /// 返回当前已累计的逻辑推进时间（秒）。
    #[must_use]
    pub const fn elapsed_secs(&self) -> f32 {
        self.elapsed
    }

    /// 返回当前使用的缓动令牌。
    #[must_use]
    pub const fn easing(&self) -> EasingToken {
        self.easing
    }

    /// 返回归一化线性进度（`0.0..=1.0`）。
    #[must_use]
    pub fn progress(&self) -> f32 {
        if !self.duration.is_finite() || self.duration <= 0.0 {
            return 1.0;
        }
        if !self.elapsed.is_finite() {
            return 1.0;
        }
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// 返回经缓动曲线映射后的进度（`0.0..=1.0`）。
    #[must_use]
    pub fn eased_progress(&self) -> f32 {
        self.easing.sample(self.progress())
    }

    /// 返回当前插值结果；已完成时精确返回目标值，零浮点残留。
    #[must_use]
    pub fn value(&self) -> f32 {
        if self.is_finished() {
            return sanitize_finite(self.to);
        }
        let p = self.eased_progress();
        self.from + (self.to - self.from) * p
    }

    /// 判断过渡是否已到达终态。
    #[must_use]
    pub fn is_finished(&self) -> bool {
        if !self.from.is_finite()
            || !self.to.is_finite()
            || !self.elapsed.is_finite()
            || !self.duration.is_finite()
        {
            return true;
        }
        self.duration <= 0.0 || self.elapsed >= self.duration || self.from == self.to
    }

    /// [`Self::is_finished`] 的语义别名，与 [`crate::Spring::is_settled`] 保持命名一致。
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.is_finished()
    }

    /// 立即吸附到指定值，无过渡过程。
    pub fn snap_to(&mut self, value: f32) {
        let value = sanitize_finite(value);
        self.from = value;
        self.to = value;
        self.elapsed = self.duration;
    }

    /// 重设过渡目标；若中途打断，以当前插值位置为新起点平滑过渡。
    ///
    /// 若 `target` 与当前目标相同则幂等返回；若 `target` 非有限则安全吸附并保持静止。
    pub fn set_target(&mut self, target: f32) {
        if !target.is_finite() {
            let safe = self.value();
            self.from = safe;
            self.to = safe;
            self.elapsed = self.duration;
            return;
        }
        if target == self.to {
            return;
        }
        let current = self.value();
        self.from = current;
        self.to = target;
        self.elapsed = if self.duration <= 0.0 || current == target {
            self.duration
        } else {
            0.0
        };
    }

    /// [`Self::set_target`] 的语义别名，便于中途重设目标。
    pub fn retarget(&mut self, target: f32) {
        self.set_target(target);
    }

    /// 使用新的语义 [`MotionToken`] 重设过渡目标。
    pub fn set_target_with_token(&mut self, target: f32, token: MotionToken) {
        let was_finished = self.is_finished();
        self.duration = sanitize_duration(token.duration().as_secs_f32());
        self.easing = token.easing();
        self.essential_non_spatial = token.is_essential_non_spatial();
        if !target.is_finite() {
            let safe = sanitize_finite(self.to);
            self.from = safe;
            self.to = safe;
            self.elapsed = self.duration;
            return;
        }
        if target == self.to {
            if was_finished || self.duration <= 0.0 {
                self.elapsed = self.duration;
            } else {
                self.elapsed = self.elapsed.min(self.duration);
            }
            return;
        }
        let current = self.value();
        self.from = current;
        self.to = target;
        self.elapsed = if self.duration <= 0.0 || current == target {
            self.duration
        } else {
            0.0
        };
    }

    /// 反转当前过渡方向，从当前值平滑返回原起点 `from`。
    pub fn reverse(&mut self) {
        let current = self.value();
        let next_target = sanitize_finite(self.from);
        self.from = current;
        self.to = next_target;
        self.elapsed = if self.duration <= 0.0 || current == next_target {
            self.duration
        } else {
            0.0
        };
    }

    /// 按单调 `dt`（秒）与 [`MotionPolicy`] 推进一步。
    ///
    /// 若仍在过渡中返回 `true`，到达终态（并精确吸附至 `target`）时返回 `false`。
    pub fn step(&mut self, dt: f32, policy: MotionPolicy) -> bool {
        if !self.from.is_finite() || !self.to.is_finite() || !self.duration.is_finite() {
            self.from = 0.0;
            self.to = 0.0;
            self.duration = sanitize_duration(self.duration);
            self.elapsed = self.duration;
            return false;
        }
        if self.is_finished() {
            self.elapsed = self.duration;
            return false;
        }
        if (policy.reduced_motion && !self.essential_non_spatial) || self.duration <= 0.0 {
            self.elapsed = self.duration;
            return false;
        }
        let scaled_dt = policy.scale_dt(dt);
        if scaled_dt <= 0.0 {
            return true;
        }
        self.elapsed = (self.elapsed + scaled_dt).min(self.duration);
        !self.is_finished()
    }
}

#[inline]
fn sanitize_finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[inline]
fn sanitize_duration(duration_secs: f32) -> f32 {
    if duration_secs.is_finite() && duration_secs > 0.0 {
        duration_secs
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_token_baselines_match_contract() {
        assert_eq!(DurationToken::Instant.as_millis(), 0);
        assert_eq!(DurationToken::Quick.as_millis(), 90);
        assert_eq!(DurationToken::Short.as_millis(), 120);
        assert_eq!(DurationToken::Medium.as_millis(), 180);
        assert_eq!(DurationToken::Slow.as_millis(), 280);

        assert_eq!(DurationToken::Instant.as_secs_f32(), 0.0);
        assert!((DurationToken::Quick.as_secs_f32() - 0.09).abs() < 1e-6);
        assert!((DurationToken::Short.as_secs_f32() - 0.12).abs() < 1e-6);
        assert!((DurationToken::Medium.as_secs_f32() - 0.18).abs() < 1e-6);
        assert!((DurationToken::Slow.as_secs_f32() - 0.28).abs() < 1e-6);
    }

    #[test]
    fn motion_policy_clamps_speed_scale_and_collapses_reduced_motion() {
        let too_slow = MotionPolicy::new(false, 0.001);
        assert_eq!(too_slow.speed_scale(), MotionPolicy::MIN_SPEED_SCALE);

        let too_fast = MotionPolicy::new(false, 9.5);
        assert_eq!(too_fast.speed_scale(), MotionPolicy::MAX_SPEED_SCALE);

        let nan_scale = MotionPolicy::new(false, f32::NAN);
        assert_eq!(nan_scale.speed_scale(), 1.0);

        let inf_scale = MotionPolicy::new(false, f32::INFINITY);
        assert_eq!(inf_scale.speed_scale(), 1.0);

        // 直接字面量构造时，访问器也必须严格钳制。
        let raw_policy = MotionPolicy {
            reduced_motion: false,
            speed_scale: -5.0,
        };
        assert_eq!(raw_policy.speed_scale(), 0.05);

        let fast = MotionPolicy::default().with_speed_scale(2.0);
        assert!((DurationToken::Short.effective_secs(fast) - 0.06).abs() < 1e-6);

        let slow = MotionPolicy::default().with_speed_scale(0.5);
        assert!((DurationToken::Short.effective_secs(slow) - 0.24).abs() < 1e-6);

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
    }

    #[test]
    fn easing_tokens_have_exact_endpoints_and_monotonic_curves() {
        for easing in [
            EasingToken::FastOutSlowIn,
            EasingToken::Accelerate,
            EasingToken::Decelerate,
            EasingToken::Linear,
        ] {
            assert_eq!(easing.sample(-0.5), 0.0);
            assert_eq!(easing.sample(0.0), 0.0);
            assert_eq!(easing.sample(1.0), 1.0);
            assert_eq!(easing.sample(1.5), 1.0);
            assert_eq!(easing.sample(f32::NAN), 0.0);

            let mut prev = 0.0;
            for i in 1..=100 {
                let t = i as f32 / 100.0;
                let y = easing.sample(t);
                assert!(y >= prev - 1e-4, "easing {easing:?} regressed at {t}: {y}");
                assert!((0.0..=1.0).contains(&y), "easing {easing:?} out of range");
                prev = y;
            }
        }

        // 减速曲线在中段应高于线性，加速曲线在中段应低于线性。
        assert!(EasingToken::Decelerate.sample(0.5) > 0.5);
        assert!(EasingToken::Accelerate.sample(0.5) < 0.5);
        assert_eq!(EasingToken::Linear.sample(0.5), 0.5);
    }

    #[test]
    fn motion_token_semantics_and_essential_non_spatial() {
        assert_eq!(MotionToken::OverlayPreview.duration(), DurationToken::Short);
        assert_eq!(
            MotionToken::OverlayPreview.easing(),
            EasingToken::Decelerate
        );
        assert!(!MotionToken::OverlayPreview.is_essential_non_spatial());
        assert!(MotionToken::EssentialFade.is_essential_non_spatial());

        let reduced = MotionPolicy::reduced();
        assert_eq!(MotionToken::OverlayPreview.effective_secs(reduced), 0.0);
        assert_eq!(MotionToken::MicroInteraction.effective_secs(reduced), 0.0);
        assert_eq!(MotionToken::ThemeTransition.effective_secs(reduced), 0.0);
        assert!(
            (MotionToken::EssentialFade.effective_secs(reduced) - 0.12).abs() < 1e-6,
            "essential non-spatial fade should keep its duration"
        );
    }

    #[test]
    fn tween_converges_consistently_across_60_120_240_hz() {
        let policy = MotionPolicy::default();
        let mut t60 = Tween::from_token_between(0.0, 100.0, MotionToken::OverlayPreview);
        let mut t120 = Tween::from_token_between(0.0, 100.0, MotionToken::OverlayPreview);
        let mut t240 = Tween::from_token_between(0.0, 100.0, MotionToken::OverlayPreview);

        // 推进完全相同的 0.05s（60Hz 3 帧、120Hz 6 帧、240Hz 12 帧）。
        for _ in 0..3 {
            assert!(t60.step(1.0 / 60.0, policy));
        }
        for _ in 0..6 {
            assert!(t120.step(1.0 / 120.0, policy));
        }
        for _ in 0..12 {
            assert!(t240.step(1.0 / 240.0, policy));
        }

        assert!(
            (t60.value() - t120.value()).abs() < 1e-3,
            "60Hz vs 120Hz mismatch: {} vs {}",
            t60.value(),
            t120.value()
        );
        assert!(
            (t120.value() - t240.value()).abs() < 1e-3,
            "120Hz vs 240Hz mismatch: {} vs {}",
            t120.value(),
            t240.value()
        );

        // 推进至超过 120ms，三者必须精确吸附至 100.0 且零残留。
        for _ in 0..20 {
            t60.step(1.0 / 60.0, policy);
        }
        for _ in 0..40 {
            t120.step(1.0 / 120.0, policy);
        }
        for _ in 0..80 {
            t240.step(1.0 / 240.0, policy);
        }

        for tween in [&t60, &t120, &t240] {
            assert!(tween.is_finished());
            assert_eq!(tween.progress(), 1.0);
            assert_eq!(tween.value(), 100.0);
        }
    }

    #[test]
    fn tween_respects_reduced_motion_and_speed_scale() {
        let mut spatial = Tween::from_token_between(10.0, 90.0, MotionToken::OverlayPreview);
        assert!(!spatial.step(1.0 / 120.0, MotionPolicy::reduced()));
        assert!(spatial.is_finished());
        assert_eq!(spatial.value(), 90.0);

        // 必要非空间淡变在 reduced_motion 下保持平滑过渡。
        let mut essential = Tween::from_token_between(0.0, 1.0, MotionToken::EssentialFade);
        assert!(essential.step(0.06, MotionPolicy::reduced()));
        assert!(!essential.is_finished());
        assert!((essential.value() - 0.5).abs() < 1e-5);
        assert!(!essential.step(0.06, MotionPolicy::reduced()));
        assert_eq!(essential.value(), 1.0);

        // 2x 速率应在 60ms 内完成 120ms 令牌。
        let fast = MotionPolicy::default().with_speed_scale(2.0);
        let mut scaled = Tween::from_token_between(0.0, 1.0, MotionToken::OverlayPreview);
        assert!(scaled.step(0.03, fast));
        assert!((scaled.progress() - 0.5).abs() < 1e-5);
        assert!(!scaled.step(0.03, fast));
        assert_eq!(scaled.value(), 1.0);
    }

    #[test]
    fn tween_retarget_reverse_and_nan_guards() {
        let policy = MotionPolicy::default();
        let mut tween = Tween::between(0.0, 100.0, 0.1, EasingToken::Linear);
        assert!(tween.step(0.05, policy));
        assert!((tween.value() - 50.0).abs() < 1e-4);

        // 中途重设目标应从当前值平滑启动。
        tween.retarget(200.0);
        assert_eq!(tween.from(), 50.0);
        assert_eq!(tween.target(), 200.0);
        assert_eq!(tween.progress(), 0.0);
        assert!((tween.value() - 50.0).abs() < 1e-4);

        // 幂等重设同一目标不得重置进度。
        assert!(tween.step(0.05, policy));
        let mid_progress = tween.progress();
        tween.retarget(200.0);
        assert_eq!(tween.progress(), mid_progress);

        // 反转方向应从当前值平滑返回当前段起点。
        let before_reverse = tween.value();
        let prev_from = tween.from();
        tween.reverse();
        assert_eq!(tween.from(), before_reverse);
        assert_eq!(tween.target(), prev_from);
        assert_eq!(tween.progress(), 0.0);
        assert!(!tween.step(0.1, policy));
        assert_eq!(tween.value(), prev_from);

        // 非法 dt 与 NaN 目标不得导致永不收敛或污染状态。
        let mut guarded = Tween::between(0.0, 10.0, 0.1, EasingToken::FastOutSlowIn);
        assert!(guarded.step(-0.5, policy));
        assert_eq!(guarded.progress(), 0.0);
        assert!(guarded.step(f32::NAN, policy));
        assert_eq!(guarded.progress(), 0.0);
        guarded.set_target(f32::NAN);
        assert!(guarded.is_finished());
        assert!(guarded.value().is_finite());
    }
}

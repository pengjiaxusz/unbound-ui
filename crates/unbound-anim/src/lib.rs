//! Spring physics, easing and reactive frame pacing (`docs` §4.4).
//!
//! The animation channel is deliberately **independent of layout**: a spring
//! writes a `GpuTransform`, never a Taffy style. That is what lets a card move at
//! 120 Hz while the layout engine does nothing at all.
//!
//! Two details decide whether spring animations feel right in practice:
//!
//! * **Substepping.** A single Euler step with a 100 ms hitch injects energy and
//!   makes the spring explode. Every step is therefore split into slices no
//!   larger than [`MAX_SUBSTEP`] seconds, which keeps the integrator stable for
//!   any frame time.
//! * **Settling.** A spring is declared done when both displacement and velocity
//!   are below a perceptual threshold, not when it mathematically converges. That
//!   is what allows the scheduler to park the CPU/GPU at 0% (`docs` §4.4).

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;

/// Largest integration slice; 4 ms keeps semi-implicit Euler stable even for
/// stiff springs.
pub const MAX_SUBSTEP: f32 = 0.004;

/// A one-dimensional damped harmonic oscillator.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
    pub value: f32,
    pub target: f32,
    pub velocity: f32,
    /// Displacement below which the spring is considered settled.
    pub rest_threshold: f32,
    /// Velocity below which the spring is considered settled.
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

/// Named spring feels, tuned for UI motion rather than for physical accuracy.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpringPreset {
    /// Quick and slightly bouncy: buttons, toggles.
    Snappy,
    /// Smooth without overshoot: panels, sheets.
    Gentle,
    /// Heavy and slow: large canvases.
    Heavy,
    /// Nearly critically damped: progress bars.
    Precise,
}

impl SpringPreset {
    #[must_use]
    pub const fn config(self) -> (f32, f32) {
        // (stiffness, damping)
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

    /// A critically damped spring: reaches the target without overshoot.
    #[must_use]
    pub fn critically_damped(stiffness: f32, initial: f32) -> Self {
        Self {
            stiffness,
            // Critical damping for a unit mass: c = 2·sqrt(k).
            damping: 2.0 * stiffness.sqrt(),
            value: initial,
            target: initial,
            ..Default::default()
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Jump instantly (initial layout, no animation).
    pub fn snap_to(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    #[must_use]
    pub fn is_settled(&self) -> bool {
        // A non-finite state can never satisfy the comparisons below (every
        // comparison with NaN is false), so the spring would never settle and
        // the frame scheduler would never sleep. A broken spring counts as
        // settled: it has nothing left to animate.
        if !self.value.is_finite() || !self.target.is_finite() || !self.velocity.is_finite() {
            return true;
        }
        (self.target - self.value).abs() <= self.rest_threshold
            && self.velocity.abs() <= self.rest_velocity
    }

    /// Advance by `dt` seconds. Returns `true` while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        // A non-finite target (a NaN out of layout, a division by zero in the
        // caller) would poison the state forever: the comparisons in
        // `is_settled` are false for NaN, so the spring would never settle,
        // the animator would never drop it, and the frame scheduler would
        // spin forever. The broken spring is snapped to rest instead.
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
            // Semi-implicit Euler: integrate velocity first, then position with
            // the *new* velocity. Symplectic, so it does not gain energy.
            let force = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            let accel = force / self.mass.max(1e-3);
            self.velocity += accel * h;
            self.value += self.velocity * h;
            remaining -= h;
        }
        if !self.value.is_finite() || !self.velocity.is_finite() {
            // The integration diverged (an absurd stiffness, or a value that
            // overflowed). Rest instead of spinning on a broken state.
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        if self.is_settled() {
            // Snap the residual so the last frame is pixel-identical to rest and
            // the node can re-enable pixel snapping.
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    /// Overshoot of the last step, as a fraction of the travel. Used by tests to
    /// prove a preset really is critically damped.
    #[must_use]
    pub fn overshoot(&self, from: f32) -> f32 {
        let travel = self.target - from;
        if travel.abs() < 1e-6 {
            return 0.0;
        }
        ((self.value - self.target) / travel).abs()
    }
}

/// A two-axis spring, which is what a transform translation actually needs.
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

    /// Advance both axes; returns `true` while either is moving.
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

/// Cubic-bezier easing for one-shot transitions (theme fades, wizard steps).
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
        // Newton iterations on x(t) = u, then evaluate y(t). Eight iterations is
        // far past visual precision and costs ~40 ns.
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

/// Which property a spring drives. Kept as data so the animator can be a flat
/// array (`docs` §4.4: no closures in the view tree).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    TranslateX,
    TranslateY,
    Scale,
    Rotate,
    Opacity,
}

/// One running animation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Animation {
    /// Opaque handle of the owning node.
    pub node: u64,
    pub channel: Channel,
    pub spring: Spring,
    /// Slots that can be reused without allocation.
    pub generation: u32,
}

/// The animator: a flat array of springs, compacted in place.
#[derive(Debug, Default)]
pub struct Animator {
    animations: Vec<Animation>,
    /// Number of springs still moving last step (drives frame pacing).
    active_count: u32,
    /// Frames where something actually moved.
    pub moving_frames: u64,
    /// Frames where nothing moved (the scheduler can sleep).
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

    /// Springs still moving. Recomputed from the springs themselves so it is
    /// correct even before the first `step` of a newly added animation.
    #[must_use]
    pub fn active_animations(&self) -> u32 {
        self.animations
            .iter()
            .filter(|a| !a.spring.is_settled())
            .count() as u32
    }

    /// Start (or retarget) an animation for `(node, channel)`.
    ///
    /// Retargeting an existing animation keeps its velocity, which is what makes
    /// interrupting a spring feel natural instead of jerky.
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

    /// Start an animation that begins at `from` and moves to `to`.
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

    /// Current value for a channel, if animated.
    #[must_use]
    pub fn value(&self, node: u64, channel: Channel) -> Option<f32> {
        self.animations
            .iter()
            .find(|a| a.node == node && a.channel == channel)
            .map(|a| a.spring.value)
    }

    /// Advance every spring. Settled animations are removed, so the array stays
    /// proportional to the number of *moving* nodes, not the number of nodes.
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

    /// Snapshot for the transform channel, then drop finished entries.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Animation> {
        self.animations.clone()
    }

    pub fn clear(&mut self) {
        self.animations.clear();
        self.active_count = 0;
    }
}

/// Why the main loop woke up.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WakeReason {
    /// Something is animating: render immediately.
    Animating,
    /// Input arrived.
    Input,
    /// Nothing to do: block until the next event with this timeout.
    Idle,
}

/// Reactive frame pacing (`docs` §4.4).
///
/// The loop asks this whether it needs a frame. When nothing is animating and no
/// input is queued, the app blocks indefinitely instead of spinning, which is
/// what makes the "0.0% CPU at rest" line real.
#[derive(Debug, Default)]
pub struct FrameScheduler {
    input_pending: bool,
    redraw_requested: bool,
    /// Frames rendered since the last idle period.
    pub frame_count: u64,
    /// Consecutive frames with nothing to do.
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

    /// Decide whether to render this tick.
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

    /// True when the loop can block on the OS event queue.
    #[must_use]
    pub fn should_sleep(&self) -> bool {
        self.idle_streak > 0
    }
}

/// Pixel-snapping policy for one node (`docs` §4.4).
///
/// Static geometry snaps to the physical pixel grid so 1px borders stay sharp;
/// animating geometry must not snap, or the spring visibly "crawls" during its
/// slow tail.
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
        // 250 ms frame: without substepping this diverges to infinity.
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
        assert_eq!(velocity_before, velocity_after, "retarget must not reset velocity");
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
            a.animate(i, Channel::TranslateY, SpringPreset::Gentle, 10.0 + i as f32);
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
        for curve in [CubicBezier::EASE, CubicBezier::EASE_OUT, CubicBezier::LINEAR] {
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
        // First poll consumes the initial redraw request.
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
        assert!(!should_snap(false, false), "a static transform needs subpixel");
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

    /// A NaN target must not keep the animator awake forever.
    ///
    /// Every comparison with NaN is false, so a spring whose target is NaN
    /// never settles: the animator never drops it, the scheduler never
    /// reports `Idle`, and the application spins at 100% CPU with a NaN
    /// transform on screen.
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
        // A NaN arrives from the layout.
        animator.animate(1, Channel::Scale, SpringPreset::Gentle, f32::NAN);
        for _ in 0..600 {
            animator.step(1.0 / 60.0);
        }
        assert_eq!(
            animator.active_animations(),
            0,
            "the broken animation must be dropped, not kept alive"
        );
        // The broken animation is gone, so it has no value to report.
        assert!(
            animator.value(1, Channel::Scale).is_none(),
            "the dropped animation must not report a value"
        );
    }

    /// A diverging integration must not spin either.
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
        // A step large enough to overflow the velocity.
        let _ = spring.step(0.25);
        assert!(
            spring.value.is_finite() && spring.velocity.is_finite(),
            "the spring must stay finite"
        );
        assert!(spring.is_settled());
    }
}

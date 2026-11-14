//! 确定进度与不确定进度指示器。
//!
//! 进度值是*展示值*而非简单的最小/最大值：[`ProgressState::shown`]
//! 会向目标值平滑缓动，使 20% 到 80% 的跳变呈现为连续移动而非瞬移。
//! 不确定状态的进度条或圆环忽略目标值，以固定速率扫过一段亮带——
//! 这是表达“未知耗时”最诚实的方式。
//!
//!
//! 圆形变体由短圆角线段拼接而非解析圆弧构建：这样圆环可与其余图元共享圆角矩形着色器，
//! 且已填充与未填充的圆环在图元池中具有完全相同的结构。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::surface;
use crate::WidgetOutput;

/// 近似完整圆环所用的分段数。
pub const RING_SEGMENTS: usize = 48;
/// 展示值每秒追赶目标值的速率。
pub const PROGRESS_RATE: f32 = 12.0;

/// 线性进度条或圆形进度环。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressKind {
    /// 水平进度条。
    #[default]
    Linear,
    /// 圆形进度环。
    Circular,
}

/// 进度指示器的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressDesc {
    /// 指示器矩形。
    pub rect: Rect,
    /// 进度条或圆环形态。
    pub kind: ProgressKind,
    /// 轨道粗细。
    pub thickness: f32,
    /// 已知进度时为 `Some(0.0..=1.0)`，不确定进度时为 `None`。
    pub value: Option<f32>,
    /// 无障碍名称。
    pub label: String,
    /// 绘制顺序键。
    pub z: u16,
}

impl ProgressDesc {
    /// 创建线性进度条。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            kind: ProgressKind::Linear,
            thickness: 6.0,
            value: Some(0.0),
            label: String::new(),
            z: 500,
        }
    }

    /// 创建内切于正方形的圆形进度环。
    #[must_use]
    pub fn circular(rect: Rect) -> Self {
        Self {
            kind: ProgressKind::Circular,
            thickness: 4.0,
            ..Self::new(rect)
        }
    }

    /// 设置目标进度值。
    #[must_use]
    pub fn value(mut self, value: f32) -> Self {
        self.value = Some(value.clamp(0.0, 1.0));
        self
    }

    /// 将指示器设为不确定进度模式。
    #[must_use]
    pub fn indeterminate(mut self) -> Self {
        self.value = None;
        self
    }

    /// 设置无障碍名称。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 圆环中心点。
    #[must_use]
    pub fn centre(&self) -> Vec2 {
        self.rect.center()
    }

    /// 圆环半径。
    #[must_use]
    pub fn radius(&self) -> f32 {
        (self.rect.size.x.min(self.rect.size.y) - self.thickness).max(0.0) * 0.5
    }

    /// 线性进度条的已填充部分矩形。
    #[must_use]
    pub fn fill_rect(&self, fraction: f32) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x * fraction.clamp(0.0, 1.0),
            self.rect.size.y,
        )
    }

    /// 圆环的单个分段，`sweep` 为占整圆周的比例。
    #[must_use]
    pub fn segment_rect(&self, start: f32, sweep: f32, segment: usize) -> Rect {
        let centre = self.centre();
        let step = sweep / RING_SEGMENTS.max(1) as f32;
        let fraction = start + step * (segment as f32 + 0.5);
        let angle = fraction * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let radius = self.radius();
        let x = centre.x + angle.cos() * radius;
        let y = centre.y + angle.sin() * radius;
        let thickness = self.thickness.max(1.0);
        Rect::new(
            x - thickness * 0.5,
            y - thickness * 0.5,
            thickness,
            thickness,
        )
    }

    /// 旋转中的不确定亮带矩形。
    #[must_use]
    pub fn slider_rect(&self, phase: f32) -> Rect {
        let width = (self.rect.size.x * 0.32).max(12.0);
        let travel = (self.rect.size.x - width).max(0.0);
        let t = phase.rem_euclid(1.0);
        Rect::new(
            self.rect.origin.x + travel * t,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }
}

/// 进度指示器的动画状态。
#[derive(Clone, Debug)]
pub struct ProgressState {
    shown: f32,
    phase: f32,
}

impl Default for ProgressState {
    fn default() -> Self {
        Self {
            shown: 0.0,
            phase: 0.0,
        }
    }
}

impl ProgressState {
    /// 创建归零的初始状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前展示的进度比例。
    #[must_use]
    pub fn shown(&self) -> f32 {
        self.shown
    }

    /// 不确定亮带的扫描相位（以圈为单位）。
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// 立即对齐到目标值而不播放动画（用于恢复会话）。
    pub fn jump_to(&mut self, value: f32) {
        self.shown = value.clamp(0.0, 1.0);
    }

    /// 推进展示值向 `target` 靠拢及扫描动画；返回是否需要重绘。
    pub fn tick(&mut self, dt: f32, target: Option<f32>, animated: bool) -> bool {
        let mut changed = false;
        if animated && dt > 0.0 {
            self.phase = (self.phase + dt * 0.6).rem_euclid(1.0);
            changed = true;
        }
        if let Some(target) = target {
            let target = target.clamp(0.0, 1.0);
            let blend = (dt * PROGRESS_RATE).clamp(0.0, 1.0);
            let next = self.shown + (target - self.shown) * blend;
            let next = if (target - next).abs() < 0.001 {
                target
            } else {
                next
            };
            if (next - self.shown).abs() > f32::EPSILON {
                self.shown = next;
                changed = true;
            }
        }
        changed
    }
}

/// 进度指示器绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressTokens {
    /// 未填充轨道。
    pub track: Token,
    /// 已填充部分。
    pub fill: Token,
    /// 不确定亮带。
    pub sweep: Token,
}

impl ProgressTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::CONTROL_TRACK,
            fill: Token::PRIMARY,
            sweep: Token::PRIMARY_SOFT,
        }
    }
}

/// 输出进度条或进度环。
#[must_use]
pub fn emit_progress(desc: &ProgressDesc, state: &ProgressState) -> WidgetOutput {
    let tokens = ProgressTokens::standard();
    let mut primitives = Vec::with_capacity(RING_SEGMENTS + 2);
    match desc.kind {
        ProgressKind::Linear => {
            let radius = desc.rect.size.y * 0.5;
            primitives.push(surface(desc.rect, radius, tokens.track, desc.z));
            match desc.value {
                Some(_) => primitives.push(surface(
                    desc.fill_rect(state.shown),
                    radius,
                    tokens.fill,
                    desc.z + 1,
                )),
                None => primitives.push(surface(
                    desc.slider_rect(state.phase),
                    radius,
                    tokens.fill,
                    desc.z + 1,
                )),
            }
        }
        ProgressKind::Circular => {
            let centre = desc.centre();
            let radius = desc.radius();
            let centre_rect = Rect::new(
                centre.x - radius - desc.thickness * 0.5,
                centre.y - radius - desc.thickness * 0.5,
                radius * 2.0 + desc.thickness,
                radius * 2.0 + desc.thickness,
            );
            primitives.push(surface(centre_rect, radius, tokens.track, desc.z));
            let (start, sweep) = match desc.value {
                Some(_) => (0.0, state.shown),
                None => (state.phase, 0.25),
            };
            for segment in 0..RING_SEGMENTS {
                if segment as f32 / RING_SEGMENTS as f32 >= sweep {
                    break;
                }
                primitives.push(surface(
                    desc.segment_rect(start, sweep, segment),
                    desc.thickness * 0.5,
                    tokens.fill,
                    desc.z + 1,
                ));
            }
        }
    }

    let value = desc.value.map_or_else(
        || "进行中".to_owned(),
        |value| format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0),
    );
    let mut a11y = A11yNode::new(Role::ProgressIndicator, desc.label.clone(), desc.rect);
    a11y.value = value;
    a11y.description = "进度".to_owned();

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_eases_toward_the_target() {
        let mut state = ProgressState::new();
        assert!(state.tick(1.0 / 60.0, Some(0.5), false));
        assert!(state.shown() > 0.0);
        assert!(state.shown() < 0.5);
        state.tick(1.0, Some(0.5), false);
        assert_eq!(state.shown(), 0.5);
    }

    #[test]
    fn the_display_clamps_to_the_unit_interval() {
        let mut state = ProgressState::new();
        state.jump_to(2.0);
        assert_eq!(state.shown(), 1.0);
        state.jump_to(-1.0);
        assert_eq!(state.shown(), 0.0);
    }

    #[test]
    fn the_indeterminate_phase_advances_only_when_animated() {
        let mut state = ProgressState::new();
        assert!(!state.tick(0.5, None, false));
        assert_eq!(state.phase(), 0.0);
        assert!(state.tick(0.5, None, true));
        assert!(state.phase() > 0.0);
    }

    #[test]
    fn the_linear_fill_is_proportional() {
        let desc = ProgressDesc::new(Rect::new(0.0, 0.0, 200.0, 8.0)).value(0.25);
        assert_eq!(desc.fill_rect(0.25).size.x, 50.0);
        assert_eq!(desc.fill_rect(2.0).size.x, 200.0);
    }

    #[test]
    fn the_sweep_stays_inside_the_bar() {
        let desc = ProgressDesc::new(Rect::new(0.0, 0.0, 200.0, 8.0)).indeterminate();
        for phase in [0.0, 0.5, 0.99, 1.0, 7.25] {
            let band = desc.slider_rect(phase);
            assert!(band.origin.x >= 0.0);
            assert!(band.max().x <= 200.0 + 0.01);
        }
    }

    #[test]
    fn ring_segments_sit_on_the_radius() {
        let desc = ProgressDesc::circular(Rect::new(0.0, 0.0, 40.0, 40.0));
        let centre = desc.centre();
        let segment = desc.segment_rect(0.0, 1.0, 0);
        let dx = segment.center().x - centre.x;
        let dy = segment.center().y - centre.y;
        let distance = dx.hypot(dy);
        assert!(
            (distance - desc.radius()).abs() < 0.75,
            "distance = {distance}"
        );
    }

    #[test]
    fn the_emitter_reports_progress_and_a_percentage() {
        let desc = ProgressDesc::new(Rect::new(0.0, 0.0, 200.0, 8.0))
            .value(0.4)
            .with_label("Upload");
        let output = emit_progress(&desc, &ProgressState::new());
        let a11y = output.a11y.expect("progress");
        assert_eq!(a11y.role, Role::ProgressIndicator);
        assert_eq!(a11y.value, "40%");
        assert_eq!(a11y.label, "Upload");
        assert_eq!(output.primitives.len(), 2);
    }

    #[test]
    fn an_indeterminate_ring_emits_a_quarter_arc() {
        let desc = ProgressDesc::circular(Rect::new(0.0, 0.0, 40.0, 40.0)).indeterminate();
        let output = emit_progress(&desc, &ProgressState::new());
        assert_eq!(output.primitives.len(), 1 + RING_SEGMENTS / 4);
        assert_eq!(output.a11y.expect("progress").value, "进行中");
    }
}

//! Deterministic and indeterminate progress indicators.
//!
//! The progress value is *displayed*, not min/max: [`ProgressState::shown`]
//! eases toward the target so a jump from 20% to 80% reads as movement instead
//! of a teleport. An indeterminate bar or ring ignores the target and sweeps a
//! band at a fixed rate, which is the only honest way to say "we do not know
//! how long this takes".
//!
//! The circular variant is built from short rounded segments rather than an
//! analytic arc: the ring then shares the rounded-rectangle shader with every
//! other primitive, and a filled and an empty ring are the same pool shape.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::surface;
use crate::WidgetOutput;

/// How many segments approximate a full ring.
pub const RING_SEGMENTS: usize = 48;
/// The target the display chases per second.
pub const PROGRESS_RATE: f32 = 12.0;

/// A linear bar or a circular ring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressKind {
    /// A horizontal bar.
    #[default]
    Linear,
    /// A ring.
    Circular,
}

/// The pure descriptor of a progress indicator.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressDesc {
    /// The indicator rectangle.
    pub rect: Rect,
    /// The bar or ring form.
    pub kind: ProgressKind,
    /// The track thickness.
    pub thickness: f32,
    /// `Some(0.0..=1.0)` for a known value, `None` for indeterminate.
    pub value: Option<f32>,
    /// The accessible name.
    pub label: String,
    /// The draw order key.
    pub z: u16,
}

impl ProgressDesc {
    /// A linear bar.
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

    /// A ring inscribed in a square.
    #[must_use]
    pub fn circular(rect: Rect) -> Self {
        Self {
            kind: ProgressKind::Circular,
            thickness: 4.0,
            ..Self::new(rect)
        }
    }

    /// The same indicator with a target value.
    #[must_use]
    pub fn value(mut self, value: f32) -> Self {
        self.value = Some(value.clamp(0.0, 1.0));
        self
    }

    /// The same indicator, indeterminate.
    #[must_use]
    pub fn indeterminate(mut self) -> Self {
        self.value = None;
        self
    }

    /// The same indicator with an accessible name.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The ring centre.
    #[must_use]
    pub fn centre(&self) -> Vec2 {
        self.rect.center()
    }

    /// The ring radius.
    #[must_use]
    pub fn radius(&self) -> f32 {
        (self.rect.size.x.min(self.rect.size.y) - self.thickness).max(0.0) * 0.5
    }

    /// The filled part of a linear bar.
    #[must_use]
    pub fn fill_rect(&self, fraction: f32) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x * fraction.clamp(0.0, 1.0),
            self.rect.size.y,
        )
    }

    /// A segment of the ring, `sweep` being the fraction of a full turn.
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

    /// The rectangle of a rotating indeterminate band.
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

/// The animated state of a progress indicator.
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
    /// A zeroed display.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The displayed fraction.
    #[must_use]
    pub fn shown(&self) -> f32 {
        self.shown
    }

    /// The indeterminate sweep phase, in turns.
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Snap the display without animating (a restored session).
    pub fn jump_to(&mut self, value: f32) {
        self.shown = value.clamp(0.0, 1.0);
    }

    /// Advance the display toward `target` and the sweep; returns repaint.
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

/// The colour slots a progress indicator paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressTokens {
    /// The unfilled track.
    pub track: Token,
    /// The filled part.
    pub fill: Token,
    /// The indeterminate band.
    pub sweep: Token,
}

impl ProgressTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::CONTROL_TRACK,
            fill: Token::PRIMARY,
            sweep: Token::PRIMARY_SOFT,
        }
    }
}

/// Emit a bar or a ring.
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

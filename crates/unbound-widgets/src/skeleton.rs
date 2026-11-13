//! Skeleton placeholders.
//!
//! A skeleton is deliberately *not* a spinner: it mirrors the shape of the
//! content that is about to arrive, so the layout does not jump when the data
//! lands. The animation is a single moving highlight, and it is purely
//! decorative — the widget emits no semantic node, so assistive technology
//! hears the surrounding "loading" announcement instead of a rectangle.

use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::surface;
use crate::WidgetOutput;

/// The shimmer width, as a fraction of the skeleton.
pub const SKELETON_SHIMMER: f32 = 0.32;
/// The shimmer period at the default speed, in seconds.
pub const SKELETON_PERIOD: f32 = 1.6;

/// A skeleton block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletonDesc {
    /// The block rectangle.
    pub rect: Rect,
    /// The corner radius; defaults to a small block radius.
    pub radius: f32,
    /// Whether the shimmer runs.
    pub animated: bool,
    /// The draw order key.
    pub z: u16,
}

impl SkeletonDesc {
    /// A static block.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            radius: 6.0,
            animated: false,
            z: 400,
        }
    }

    /// The same block with the shimmer enabled.
    #[must_use]
    pub fn animated(mut self) -> Self {
        self.animated = true;
        self
    }

    /// The same block with an explicit radius.
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius.max(0.0);
        self
    }

    /// The shimmer band at `phase`, bouncing between the two edges.
    #[must_use]
    pub fn shimmer_rect(&self, phase: f32) -> Rect {
        let width = (self.rect.size.x * SKELETON_SHIMMER)
            .min(self.rect.size.x)
            .max(0.0);
        let travel = (self.rect.size.x - width).max(0.0);
        let t = phase.rem_euclid(1.0);
        let t = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
        Rect::new(
            self.rect.origin.x + travel * t,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }
}

/// The animation clock of one skeleton.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkeletonState {
    phase: f32,
}

impl SkeletonState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current shimmer phase.
    #[must_use]
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Set the phase directly (a restored or captured frame).
    pub fn set_phase(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// Advance the phase; returns whether it moved.
    pub fn tick(&mut self, dt: f32, animated: bool) -> bool {
        if !animated || dt <= 0.0 {
            return false;
        }
        self.phase = (self.phase + dt / SKELETON_PERIOD).rem_euclid(1.0);
        true
    }
}

/// Emit a skeleton block.
#[must_use]
pub fn emit_skeleton(desc: &SkeletonDesc, state: &SkeletonState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2);
    primitives.push(surface(
        desc.rect,
        desc.radius,
        Token::SURFACE_SUNKEN,
        desc.z,
    ));
    if desc.animated {
        primitives.push(surface(
            desc.shimmer_rect(state.phase()),
            desc.radius,
            Token::CONTROL_HOVER,
            desc.z + 1,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        // Decorative: the surrounding live region owns the announcement.
        a11y: None,
        focusable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SkeletonDesc {
        SkeletonDesc::new(Rect::new(10.0, 20.0, 200.0, 16.0))
    }

    #[test]
    fn a_static_block_paints_one_surface() {
        let output = emit_skeleton(&desc(), &SkeletonState::new());
        assert_eq!(output.primitives.len(), 1);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            u32::from(Token::SURFACE_SUNKEN.raw())
        );
        assert!(output.a11y.is_none());
        assert!(output.hit.is_none());
    }

    #[test]
    fn an_animated_block_adds_a_shimmer() {
        let output = emit_skeleton(&desc().animated(), &SkeletonState::new());
        assert_eq!(output.primitives.len(), 2);
        assert_eq!(
            output.primitives[1].fill_token_or_uv,
            u32::from(Token::CONTROL_HOVER.raw())
        );
    }

    #[test]
    fn the_shimmer_bounces_between_the_edges() {
        let desc = desc();
        let start = desc.shimmer_rect(0.0);
        let end = desc.shimmer_rect(0.5);
        let back = desc.shimmer_rect(1.0);
        assert_eq!(start.origin.x, desc.rect.origin.x);
        assert!((end.max().x - desc.rect.max().x).abs() < 0.001);
        assert_eq!(back.origin.x, start.origin.x);
        let middle = desc.shimmer_rect(0.25);
        assert!(middle.origin.x > start.origin.x);
        assert!(middle.origin.x < end.origin.x);
    }

    #[test]
    fn the_shimmer_never_escapes_the_block() {
        let desc = desc();
        for phase in [0.0, 0.1, 0.33, 0.5, 0.75, 0.99, 7.4] {
            let shimmer = desc.shimmer_rect(phase);
            assert!(desc.rect.inflate(0.01).contains_rect(&shimmer));
        }
    }

    #[test]
    fn only_an_animated_block_advances_its_clock() {
        let mut state = SkeletonState::new();
        assert!(!state.tick(0.5, false));
        assert_eq!(state.phase(), 0.0);
        assert!(state.tick(SKELETON_PERIOD * 0.5, true));
        assert!((state.phase() - 0.5).abs() < 0.001);
    }

    #[test]
    fn a_zero_radius_block_is_a_plain_rectangle() {
        let desc = desc().with_radius(-4.0);
        assert_eq!(desc.radius, 0.0);
    }
}

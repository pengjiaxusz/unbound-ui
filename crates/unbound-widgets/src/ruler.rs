//! Canvas rulers and guide lines.
//!
//! A ruler is a scale bar, not a list of numbers: it chooses a "nice" step
//! (1, 2 or 5 times a power of ten) so that labels never collide at the current
//! zoom, then paints a tick for every step. Dragging out of the ruler creates a
//! guide, and a guide is just a world coordinate the snapping pass may use.

use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::surface;
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The smallest visible tick spacing, in screen pixels.
pub const RULER_MIN_STEP: f32 = 8.0;
/// The ruler band thickness.
pub const RULER_BAND: f32 = 22.0;
/// How many minor ticks sit between two major ticks.
pub const RULER_MINOR_PER_MAJOR: u32 = 5;

/// The axis a ruler or guide follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideAxis {
    /// Vertical guides, created on the top ruler.
    X,
    /// Horizontal guides, created on the left ruler.
    Y,
}

/// A guide line in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideLine {
    /// The axis the line runs along.
    pub axis: GuideAxis,
    /// The world coordinate.
    pub world: f32,
}

/// The rulers around a canvas viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerDesc {
    /// The content area (excluding the rulers).
    pub viewport: Rect,
    /// The zoom factor.
    pub zoom: f32,
    /// The world point at the viewport's top-left corner.
    pub origin: Vec2,
    /// The draw order key.
    pub z: u16,
}

impl RulerDesc {
    #[must_use]
    pub fn new(viewport: Rect, zoom: f32, origin: Vec2) -> Self {
        Self {
            viewport,
            zoom: if zoom > 0.0 { zoom } else { 1.0 },
            origin,
            z: 875,
        }
    }

    /// The top band.
    #[must_use]
    pub fn top_rect(&self) -> Rect {
        Rect::new(
            self.viewport.origin.x,
            self.viewport.origin.y - RULER_BAND,
            self.viewport.size.x,
            RULER_BAND,
        )
    }

    /// The left band.
    #[must_use]
    pub fn left_rect(&self) -> Rect {
        Rect::new(
            self.viewport.origin.x - RULER_BAND,
            self.viewport.origin.y,
            RULER_BAND,
            self.viewport.size.y,
        )
    }

    /// The square where the two bands meet.
    #[must_use]
    pub fn corner_rect(&self) -> Rect {
        Rect::new(
            self.viewport.origin.x - RULER_BAND,
            self.viewport.origin.y - RULER_BAND,
            RULER_BAND,
            RULER_BAND,
        )
    }

    /// The visible world extent along one axis.
    #[must_use]
    pub fn world_span(&self, axis: GuideAxis) -> (f32, f32) {
        match axis {
            GuideAxis::X => (
                self.origin.x,
                self.origin.x + self.viewport.size.x / self.zoom,
            ),
            GuideAxis::Y => (
                self.origin.y,
                self.origin.y + self.viewport.size.y / self.zoom,
            ),
        }
    }

    /// Convert a world coordinate to a screen offset inside the viewport.
    #[must_use]
    pub fn screen(&self, axis: GuideAxis, world: f32) -> f32 {
        match axis {
            GuideAxis::X => self.viewport.origin.x + (world - self.origin.x) * self.zoom,
            GuideAxis::Y => self.viewport.origin.y + (world - self.origin.y) * self.zoom,
        }
    }

    /// Convert a screen offset inside the viewport to a world coordinate.
    #[must_use]
    pub fn world(&self, axis: GuideAxis, screen: f32) -> f32 {
        match axis {
            GuideAxis::X => self.origin.x + (screen - self.viewport.origin.x) / self.zoom,
            GuideAxis::Y => self.origin.y + (screen - self.viewport.origin.y) / self.zoom,
        }
    }

    /// The ticks along one axis: `(world, major)`.
    #[must_use]
    pub fn ticks(&self, axis: GuideAxis) -> Vec<(f32, bool)> {
        let step = nice_step(self.zoom);
        let (start, end) = self.world_span(axis);
        let first = (start / step).floor() as i64;
        let last = (end / step).ceil() as i64;
        let mut ticks = Vec::new();
        for index in first..=last {
            let world = index as f32 * step;
            ticks.push((
                world,
                index.rem_euclid(i64::from(RULER_MINOR_PER_MAJOR)) == 0,
            ));
        }
        ticks
    }

    /// The label text for a tick, rounded to the step's precision.
    #[must_use]
    pub fn label(&self, world: f32) -> String {
        let step = nice_step(self.zoom);
        if step >= 1.0 {
            format!("{world:.0}")
        } else {
            let decimals = (-step.log10()).ceil().max(0.0) as usize;
            format!("{world:.decimals$}")
        }
    }
}

/// A "nice" world step: 1, 2 or 5 times a power of ten at the current zoom.
#[must_use]
pub fn nice_step(zoom: f32) -> f32 {
    let zoom = if zoom > 0.0 { zoom } else { 1.0 };
    let raw = 50.0 / zoom;
    let magnitude = 10f32.powf(raw.log10().floor());
    let normalised = raw / magnitude;
    let snapped = if normalised <= 1.0 {
        1.0
    } else if normalised <= 2.0 {
        2.0
    } else if normalised <= 5.0 {
        5.0
    } else {
        10.0
    };
    (snapped * magnitude).max(f32::EPSILON)
}

/// The guide state of one canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RulerState {
    guides: Vec<GuideLine>,
    pending: Option<GuideLine>,
}

impl RulerState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The committed guides.
    #[must_use]
    pub fn guides(&self) -> &[GuideLine] {
        &self.guides
    }

    /// The guide being dragged out, if any.
    #[must_use]
    pub fn pending(&self) -> Option<GuideLine> {
        self.pending
    }

    /// Start dragging a new guide from a ruler band.
    pub fn begin(&mut self, axis: GuideAxis, world: f32) {
        self.pending = Some(GuideLine { axis, world });
    }

    /// Move the pending guide.
    pub fn update(&mut self, world: f32) {
        if let Some(pending) = &mut self.pending {
            pending.world = world;
        }
    }

    /// Commit the pending guide; returns whether one existed.
    pub fn end(&mut self) -> bool {
        match self.pending.take() {
            Some(guide) => {
                self.guides.push(guide);
                true
            }
            None => false,
        }
    }

    /// Cancel the pending guide.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// Remove the guide closest to `world` on `axis`.
    pub fn remove_near(&mut self, axis: GuideAxis, world: f32, tolerance: f32) -> bool {
        let mut best: Option<(usize, f32)> = None;
        for (index, guide) in self.guides.iter().enumerate() {
            if guide.axis != axis {
                continue;
            }
            let distance = (guide.world - world).abs();
            if distance <= tolerance && best.is_none_or(|(_, d)| distance < d) {
                best = Some((index, distance));
            }
        }
        match best {
            Some((index, _)) => {
                self.guides.remove(index);
                true
            }
            None => false,
        }
    }

    /// Remove every guide.
    pub fn clear(&mut self) {
        self.guides.clear();
        self.pending = None;
    }
}

/// What one ruler emits.
#[derive(Clone, Debug)]
pub struct RulerOutput {
    /// The bands, ticks and guides.
    pub output: WidgetOutput,
    /// The two ruler bands, for starting a guide drag.
    pub bands: Vec<(GuideAxis, HitRegion)>,
}

/// Emit the rulers and guides.
#[must_use]
pub fn emit_rulers(
    node: u64,
    desc: &RulerDesc,
    state: &RulerState,
    _control: ControlState,
) -> RulerOutput {
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        surface(desc.top_rect(), 0.0, Token::SURFACE_SUNKEN, desc.z),
        surface(desc.left_rect(), 0.0, Token::SURFACE_SUNKEN, desc.z),
        surface(desc.corner_rect(), 0.0, Token::SURFACE_SUNKEN, desc.z),
    ];

    for (axis, band) in [
        (GuideAxis::X, desc.top_rect()),
        (GuideAxis::Y, desc.left_rect()),
    ] {
        for (world, major) in desc.ticks(axis) {
            let screen = desc.screen(axis, world);
            let inside = match axis {
                GuideAxis::X => screen >= desc.viewport.origin.x && screen <= desc.viewport.max().x,
                GuideAxis::Y => screen >= desc.viewport.origin.y && screen <= desc.viewport.max().y,
            };
            if !inside {
                continue;
            }
            let (origin, size) = match axis {
                GuideAxis::X => (
                    Vec2::new(screen, band.max().y - if major { 9.0 } else { 4.0 }),
                    Vec2::new(1.0, if major { 9.0 } else { 4.0 }),
                ),
                GuideAxis::Y => (
                    Vec2::new(band.max().x - if major { 9.0 } else { 4.0 }, screen),
                    Vec2::new(if major { 9.0 } else { 4.0 }, 1.0),
                ),
            };
            primitives.push(surface(
                Rect::new(origin.x, origin.y, size.x, size.y),
                0.0,
                if major {
                    Token::TEXT_SECONDARY
                } else {
                    Token::OUTLINE
                },
                desc.z + 1,
            ));
        }
    }

    for guide in state.guides().iter().chain(state.pending().iter()) {
        let (rect, color) = match guide.axis {
            GuideAxis::X => (
                Rect::new(
                    desc.screen(GuideAxis::X, guide.world),
                    desc.viewport.origin.y,
                    1.0,
                    desc.viewport.size.y,
                ),
                Token::SNAP_GUIDE,
            ),
            GuideAxis::Y => (
                Rect::new(
                    desc.viewport.origin.x,
                    desc.screen(GuideAxis::Y, guide.world),
                    desc.viewport.size.x,
                    1.0,
                ),
                Token::SNAP_GUIDE,
            ),
        };
        primitives.push(surface(rect, 0.0, color, desc.z + 2));
    }

    let mut bands = vec![
        (
            GuideAxis::X,
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.top_rect())
                .with_cursor(CursorKind::ResizeNorthSouth),
        ),
        (
            GuideAxis::Y,
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.left_rect())
                .with_cursor(CursorKind::ResizeEastWest),
        ),
    ];
    bands.retain(|(axis, _)| match axis {
        GuideAxis::X => desc.top_rect().size.x > 0.0,
        GuideAxis::Y => desc.left_rect().size.y > 0.0,
    });

    RulerOutput {
        output: WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit: None,
            a11y: None,
            focusable: false,
            ..Default::default()
        },
        bands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> RulerDesc {
        RulerDesc::new(
            Rect::new(60.0, 60.0, 800.0, 600.0),
            1.0,
            Vec2::new(0.0, 0.0),
        )
    }

    #[test]
    fn the_nice_step_follows_one_two_five() {
        assert_eq!(nice_step(1.0), 50.0);
        assert_eq!(nice_step(0.5), 100.0);
        assert_eq!(nice_step(0.01), 5000.0);
        assert_eq!(nice_step(100.0), 0.5);
    }

    #[test]
    fn ticks_are_at_least_the_minimum_apart() {
        for zoom in [0.1, 0.5, 1.0, 4.0, 32.0] {
            let desc = RulerDesc::new(desc().viewport, zoom, Vec2::ZERO);
            let ticks = desc.ticks(GuideAxis::X);
            for pair in ticks.windows(2) {
                assert!((pair[1].0 - pair[0].0) * zoom >= RULER_MIN_STEP * 0.999);
            }
        }
    }

    #[test]
    fn screen_and_world_round_trip() {
        let desc = desc();
        for value in [0.0, 100.0, 375.5] {
            assert!(
                (desc.world(GuideAxis::X, desc.screen(GuideAxis::X, value)) - value).abs() < 0.001
            );
            assert!(
                (desc.world(GuideAxis::Y, desc.screen(GuideAxis::Y, value)) - value).abs() < 0.001
            );
        }
    }

    #[test]
    fn dragging_out_a_guide_commits_it() {
        let mut state = RulerState::new();
        state.begin(GuideAxis::X, 12.0);
        state.update(40.0);
        assert_eq!(state.pending().unwrap().world, 40.0);
        assert!(state.end());
        assert_eq!(state.guides().len(), 1);
        assert!(!state.end());
    }

    #[test]
    fn a_guide_near_the_pointer_can_be_removed() {
        let mut state = RulerState::new();
        state.begin(GuideAxis::Y, 100.0);
        state.end();
        assert!(!state.remove_near(GuideAxis::X, 100.0, 4.0));
        assert!(state.remove_near(GuideAxis::Y, 102.0, 4.0));
        assert!(state.guides().is_empty());
    }

    #[test]
    fn the_emitter_paints_two_bands_and_guides() {
        let desc = desc();
        let mut state = RulerState::new();
        state.begin(GuideAxis::X, 200.0);
        state.end();
        let output = emit_rulers(8, &desc, &state, ControlState::default());
        assert_eq!(output.bands.len(), 2);
        assert!(output.output.primitives.len() > 4);
    }

    #[test]
    fn labels_round_to_the_step_precision() {
        let coarse = desc();
        assert_eq!(coarse.label(125.0), "125");
        let fine = RulerDesc::new(desc().viewport, 100.0, Vec2::ZERO);
        assert_eq!(fine.label(0.25), "0.2");
    }
}

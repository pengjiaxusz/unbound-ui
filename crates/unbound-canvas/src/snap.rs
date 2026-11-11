//! Viewport-bounded smart snapping (`docs` §5.3).
//!
//! When an entity is dragged, the canvas does not snap against the world: it
//! snaps against what the user can *see*. The solver therefore
//!
//! 1. asks the loose quadtree for everything inside the viewport inflated by
//!    [`SnapConfig::padding_px`] (64 px by default),
//! 2. ranks those candidates by `area / distance` and keeps the first
//!    [`SnapConfig::max_candidates`] (128) — a 1 M element canvas must not turn
//!    a drag into a million-edge scan,
//! 3. projects the survivors onto two sorted 1D interval tables (X and Y),
//! 4. looks up left/centre/right and top/middle/bottom alignment with a binary
//!    search, and finally
//! 5. looks for equal-spacing runs on both axes.
//!
//! Distances are *screen* pixels: the threshold is divided by the camera zoom so
//! that a 5 px magnet stays 5 px whether the user is at 10 % or 6400 %.
//!
//! The output is a pure correction plus a list of transient guides. Guides are
//! ephemeral by construction (`docs` §5.3): they are returned in a reusable
//! buffer that the caller copies straight into the frame's transient ring and
//! never into the reducer state or the undo stack.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Rect, Vec2};

use crate::quadtree::LooseQuadtree;

/// Axis a guide runs along. `X` means a *vertical* line at `coordinate`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// Which feature produced a snap.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// Moving left edge met a left edge.
    Left,
    /// Moving centre met a centre.
    Centre,
    /// Moving right edge met a right edge.
    Right,
    Top,
    Middle,
    Bottom,
    /// Equal-spacing run.
    Spacing,
}

impl SnapKind {
    /// True for the X-axis variants.
    #[must_use]
    pub fn is_x(self) -> bool {
        matches!(self, Self::Left | Self::Centre | Self::Right)
    }
}

/// A transient alignment line, in world space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SnapGuide {
    pub axis: Axis,
    /// World coordinate of the line (X for a vertical line).
    pub coordinate: f32,
    /// Span along the other axis, so the renderer can draw a segment instead of
    /// an infinite rule.
    pub start: f32,
    pub end: f32,
    pub kind: SnapKind,
}

/// Tuning knobs. Defaults are the values from `docs` §5.3.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SnapConfig {
    /// Screen-space magnet radius.
    pub threshold_px: f32,
    /// Viewport inflation used when gathering candidates.
    pub padding_px: f32,
    /// Hard cap on candidates; the ranking keeps the most significant ones.
    pub max_candidates: usize,
    /// Enable equal-spacing detection.
    pub equal_spacing: bool,
    /// Gaps narrower than this (in screen px) are ignored as noise.
    pub min_spacing_px: f32,
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            threshold_px: 5.0,
            padding_px: 64.0,
            max_candidates: 128,
            equal_spacing: true,
            min_spacing_px: 4.0,
        }
    }
}

/// Result of one drag frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapResult {
    /// Correction to add to the dragged rectangle's origin, in world units.
    pub offset: Vec2,
    /// True when the X axis found a target.
    pub snapped_x: bool,
    /// True when the Y axis found a target.
    pub snapped_y: bool,
    /// Transient guides for this frame, deduplicated by `(axis, coordinate)`.
    pub guides: Vec<SnapGuide>,
}

impl SnapResult {
    /// True when any axis moved the rectangle.
    #[must_use]
    pub fn is_snapped(&self) -> bool {
        self.snapped_x || self.snapped_y
    }

    fn reset(&mut self) {
        self.offset = Vec2::ZERO;
        self.snapped_x = false;
        self.snapped_y = false;
        self.guides.clear();
    }

    fn push_guide(&mut self, guide: SnapGuide) {
        for existing in &mut self.guides {
            if existing.axis == guide.axis && (existing.coordinate - guide.coordinate).abs() < 0.01 {
                existing.start = existing.start.min(guide.start);
                existing.end = existing.end.max(guide.end);
                return;
            }
        }
        self.guides.push(guide);
    }
}

/// One ranked candidate rect.
#[derive(Copy, Clone, Debug)]
struct Candidate {
    rect: Rect,
    score: f32,
}

/// One edge on an axis table.
#[derive(Copy, Clone, Debug)]
struct Edge {
    coord: f32,
}

/// Reusable snapping solver: owns its buffers so a drag allocates nothing.
pub struct Snapper {
    config: SnapConfig,
    candidates: Vec<Candidate>,
    edges_x: Vec<Edge>,
    edges_y: Vec<Edge>,
    result: SnapResult,
    /// Reused id buffer for quadtree queries.
    ids: Vec<u32>,
    /// Reused rect buffer for the equal-spacing pass.
    scratch: Vec<Rect>,
    /// Number of candidates that survived ranking in the last call.
    retained: usize,
}

impl core::fmt::Debug for Snapper {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Snapper")
            .field("config", &self.config)
            .field("retained", &self.retained)
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
}

impl Snapper {
    /// Build a solver with the given configuration.
    #[must_use]
    pub fn new(config: SnapConfig) -> Self {
        Self {
            config,
            candidates: Vec::with_capacity(config.max_candidates),
            edges_x: Vec::with_capacity(config.max_candidates * 3),
            edges_y: Vec::with_capacity(config.max_candidates * 3),
            result: SnapResult::default(),
            ids: Vec::new(),
            scratch: Vec::new(),
            retained: 0,
        }
    }

    /// Configuration in use.
    #[must_use]
    pub fn config(&self) -> &SnapConfig {
        &self.config
    }

    /// Candidates retained by the last call, exposed for the KPI tests.
    #[must_use]
    pub fn retained_candidates(&self) -> usize {
        self.retained
    }

    /// Gather candidates around `viewport` from `tree` and solve for `moving`.
    ///
    /// `moving_id` is excluded from the candidate set so an entity never snaps
    /// to its own stale bounds.
    pub fn snap_against_tree(
        &mut self,
        tree: &mut LooseQuadtree,
        moving: Rect,
        moving_id: Option<u32>,
        viewport: Rect,
        zoom: f32,
    ) -> &SnapResult {
        let padded = viewport.inflate(self.config.padding_px / zoom.max(1e-6));
        let mut ids = core::mem::take(&mut self.ids);
        tree.query_exact(padded, &mut ids);
        self.candidates.clear();
        for id in &ids {
            if Some(*id) == moving_id {
                continue;
            }
            if let Some(rect) = tree.bounds_of(*id) {
                if rect.intersects(&padded) {
                    self.candidates.push(Candidate { rect, score: 0.0 });
                }
            }
        }
        self.rank(moving, zoom);
        self.ids = ids;
        self.solve(moving, zoom)
    }

    /// Solve against an explicit candidate list (headless harness and tests).
    pub fn snap(&mut self, moving: Rect, others: &[Rect], zoom: f32) -> &SnapResult {
        self.candidates.clear();
        self.candidates
            .extend(others.iter().map(|rect| Candidate {
                rect: *rect,
                score: 0.0,
            }));
        self.rank(moving, zoom);
        self.solve(moving, zoom)
    }

    /// Result of the last call.
    #[must_use]
    pub fn result(&self) -> &SnapResult {
        &self.result
    }

    /// Rank by significance (`area / distance`) and truncate.
    ///
    /// Area matters because a 40 000 px group frame is a far better alignment
    /// reference than an 8 px icon; distance matters because off-screen-in-
    /// spirit references are noise. The quotient is the cheapest score that
    /// respects both, and it is computed once per candidate.
    ///
    /// Both terms are converted to *screen* pixels first: the area is in
    /// world units squared and the distance in world units, so mixing them
    /// raw would make the score depend on the zoom and change which
    /// candidates survive the cut.
    fn rank(&mut self, moving: Rect, zoom: f32) {
        let centre = moving.center();
        // One world unit is `zoom` screen pixels.
        let to_screen = zoom.max(1e-6);
        for candidate in &mut self.candidates {
            let delta = candidate.rect.center() - centre;
            let distance = delta.length() * to_screen;
            let area = candidate.rect.area().max(1.0) * to_screen * to_screen;
            candidate.score = area / (distance + 16.0);
        }
        self.candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        if self.candidates.len() > self.config.max_candidates {
            self.candidates.truncate(self.config.max_candidates);
        }
        self.retained = self.candidates.len();
    }

    fn solve(&mut self, moving: Rect, zoom: f32) -> &SnapResult {
        self.result.reset();
        let inv_zoom = 1.0 / zoom.max(1e-6);
        let threshold = self.config.threshold_px * inv_zoom;
        let min_gap = self.config.min_spacing_px * inv_zoom;

        self.edges_x.clear();
        self.edges_y.clear();
        for candidate in &self.candidates {
            let r = candidate.rect;
            self.edges_x.push(Edge { coord: r.min().x });
            self.edges_x.push(Edge {
                coord: r.center().x,
            });
            self.edges_x.push(Edge { coord: r.max().x });
            self.edges_y.push(Edge { coord: r.min().y });
            self.edges_y.push(Edge {
                coord: r.center().y,
            });
            self.edges_y.push(Edge { coord: r.max().y });
        }
        self.edges_x.sort_by(|a, b| a.coord.total_cmp(&b.coord));
        self.edges_y.sort_by(|a, b| a.coord.total_cmp(&b.coord));

        let min = moving.min();
        let max = moving.max();
        let centre = moving.center();

        let x = self.best_axis(
            [min.x, centre.x, max.x],
            [SnapKind::Left, SnapKind::Centre, SnapKind::Right],
            threshold,
        );
        let y = self.best_axis(
            [min.y, centre.y, max.y],
            [SnapKind::Top, SnapKind::Middle, SnapKind::Bottom],
            threshold,
        );

        let x = self.blend_spacing_x(moving, x, threshold, min_gap);
        let y = self.blend_spacing_y(moving, y, threshold, min_gap);

        if let Some(hit) = x {
            self.result.offset.x = hit.delta;
            self.result.snapped_x = true;
            let (start, end) = self.span_y(&hit, moving);
            self.result.push_guide(SnapGuide {
                axis: Axis::X,
                coordinate: hit.coord,
                start,
                end,
                kind: hit.kind,
            });
        }
        if let Some(hit) = y {
            self.result.offset.y = hit.delta;
            self.result.snapped_y = true;
            let (start, end) = self.span_x(&hit, moving);
            self.result.push_guide(SnapGuide {
                axis: Axis::Y,
                coordinate: hit.coord,
                start,
                end,
                kind: hit.kind,
            });
        }
        &self.result
    }

    /// Nearest edge to any of `values`, within `threshold`.
    fn best_axis(
        &self,
        values: [f32; 3],
        kinds: [SnapKind; 3],
        threshold: f32,
    ) -> Option<Hit> {
        let table: &[Edge] = if kinds[0].is_x() {
            &self.edges_x
        } else {
            &self.edges_y
        };
        if table.is_empty() {
            return None;
        }
        let mut best: Option<Hit> = None;
        for (value, kind) in values.iter().zip(kinds) {
            if !value.is_finite() {
                continue;
            }
            let start = table.partition_point(|e| e.coord < *value - threshold);
            for edge in &table[start..] {
                if edge.coord > *value + threshold {
                    break;
                }
                let delta = edge.coord - *value;
                if delta.abs() > threshold {
                    continue;
                }
                // Prefer the smallest correction. On a tie the earlier entry
                // wins, and the table is sorted by coordinate — not by the
                // significance ranking, which only decided which candidates
                // survived the cut.
                if best.as_ref().is_none_or(|b| delta.abs() < b.delta.abs()) {
                    best = Some(Hit {
                        coord: edge.coord,
                        delta,
                        kind,
                    });
                }
            }
        }
        best
    }

    /// Equal-spacing pass on X, keeping whichever correction moves less.
    fn blend_spacing_x(
        &mut self,
        moving: Rect,
        aligned: Option<Hit>,
        threshold: f32,
        min_gap: f32,
    ) -> Option<Hit> {
        if !self.config.equal_spacing {
            return aligned;
        }
        let mut rects = core::mem::take(&mut self.scratch);
        rects.clear();
        rects.extend(self.candidates.iter().map(|c| c.rect));
        rects.sort_by(|a, b| a.min().x.total_cmp(&b.min().x));
        let mut best = aligned;
        for pair in rects.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let gap = b.min().x - a.max().x;
            if gap < min_gap {
                continue;
            }
            // Place the moving rect after `b`, or before `a`, at the same gap.
            let after = b.max().x + gap - moving.min().x;
            let before = a.min().x - gap - moving.max().x;
            for delta in [after, before] {
                if delta.abs() <= threshold
                    && best.as_ref().is_none_or(|h| delta.abs() < h.delta.abs())
                {
                    best = Some(Hit {
                        coord: moving.min().x + delta,
                        delta,
                        kind: SnapKind::Spacing,
                    });
                }
            }
        }
        self.scratch = rects;
        best
    }

    /// Equal-spacing pass on Y.
    fn blend_spacing_y(
        &mut self,
        moving: Rect,
        aligned: Option<Hit>,
        threshold: f32,
        min_gap: f32,
    ) -> Option<Hit> {
        if !self.config.equal_spacing {
            return aligned;
        }
        let mut rects = core::mem::take(&mut self.scratch);
        rects.clear();
        rects.extend(self.candidates.iter().map(|c| c.rect));
        rects.sort_by(|a, b| a.min().y.total_cmp(&b.min().y));
        let mut best = aligned;
        for pair in rects.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let gap = b.min().y - a.max().y;
            if gap < min_gap {
                continue;
            }
            let after = b.max().y + gap - moving.min().y;
            let before = a.min().y - gap - moving.max().y;
            for delta in [after, before] {
                if delta.abs() <= threshold
                    && best.as_ref().is_none_or(|h| delta.abs() < h.delta.abs())
                {
                    best = Some(Hit {
                        coord: moving.min().y + delta,
                        delta,
                        kind: SnapKind::Spacing,
                    });
                }
            }
        }
        self.scratch = rects;
        best
    }

    /// Guide span on Y for an X-axis snap.
    ///
    /// The span covers the dragged rect *and every* candidate that shares the
    /// matched coordinate, so three boxes aligned on one edge produce one line
    /// that visibly touches all of them instead of a stub between two.
    fn span_y(&self, hit: &Hit, moving: Rect) -> (f32, f32) {
        let mut start = moving.min().y;
        let mut end = moving.max().y;
        for candidate in &self.candidates {
            let r = candidate.rect;
            let aligned = match hit.kind {
                SnapKind::Left => (r.min().x - hit.coord).abs() < 0.01,
                SnapKind::Centre => (r.center().x - hit.coord).abs() < 0.01,
                SnapKind::Right => (r.max().x - hit.coord).abs() < 0.01,
                _ => false,
            };
            if aligned {
                start = start.min(r.min().y);
                end = end.max(r.max().y);
            }
        }
        (start, end)
    }

    /// Guide span on X for a Y-axis snap, unioned over every co-aligned rect.
    fn span_x(&self, hit: &Hit, moving: Rect) -> (f32, f32) {
        let mut start = moving.min().x;
        let mut end = moving.max().x;
        for candidate in &self.candidates {
            let r = candidate.rect;
            let aligned = match hit.kind {
                SnapKind::Top => (r.min().y - hit.coord).abs() < 0.01,
                SnapKind::Middle => (r.center().y - hit.coord).abs() < 0.01,
                SnapKind::Bottom => (r.max().y - hit.coord).abs() < 0.01,
                _ => false,
            };
            if aligned {
                start = start.min(r.min().x);
                end = end.max(r.max().x);
            }
        }
        (start, end)
    }
}

/// One accepted alignment.
#[derive(Copy, Clone, Debug)]
struct Hit {
    /// World coordinate the edge landed on.
    coord: f32,
    /// Correction applied to the moving rect.
    delta: f32,
    kind: SnapKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn left_edges_align_within_threshold() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        // Moving left edge is 3 px right of the neighbour's left edge; the
        // 10 px width keeps the other two edges out of magnet range.
        let moving = rect(103.0, 200.0, 10.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(r.snapped_x);
        assert!((r.offset.x - -3.0).abs() < 1e-4, "{:?}", r.offset);
        assert!(r.guides.iter().any(|g| g.axis == Axis::X
            && (g.coordinate - 100.0).abs() < 1e-4
            && g.kind == SnapKind::Left));
    }

    #[test]
    fn nothing_snaps_beyond_the_threshold() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        let moving = rect(180.0, 200.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(!r.snapped_x, "80 px away must not snap");
        assert!(r.guides.is_empty());
    }

    #[test]
    fn centre_to_centre_alignment_wins_over_a_farther_edge() {
        let mut s = Snapper::new(SnapConfig::default());
        // The centre of this rect is at x = 125.
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        // Moving centre at 127 → 2 px off centre; moving left at 117 → 17 px off
        // the left edge, out of range. Only the centre can match.
        let moving = rect(117.0, 200.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(r.snapped_x);
        assert!((r.offset.x - -2.0).abs() < 1e-4, "{:?}", r.offset);
        assert_eq!(r.guides[0].kind, SnapKind::Centre);
    }

    #[test]
    fn threshold_is_measured_in_screen_pixels() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 0.0, 50.0, 50.0)];
        // 4 world units = 4 px at zoom 1 (snaps), 0.4 px at zoom 10 (does not).
        let moving = rect(104.0, 200.0, 20.0, 20.0);
        assert!(s.snap(moving, &others, 1.0).snapped_x);
        assert!(!s.snap(moving, &others, 10.0).snapped_x);
    }

    #[test]
    fn equal_spacing_produces_a_third_slot() {
        let mut s = Snapper::new(SnapConfig::default());
        // Two 100 px boxes with a 40 px gap: 0..100 and 140..240.
        let others = [
            rect(0.0, 0.0, 100.0, 100.0),
            rect(140.0, 0.0, 100.0, 100.0),
        ];
        // The third box is 3 px away from the ideal slot at 280.
        let moving = rect(283.0, 0.0, 100.0, 100.0);
        let r = s.snap(moving, &others, 1.0);
        assert!((r.offset.x - -3.0).abs() < 1e-4, "{:?}", r.offset);
        assert!(r.guides.iter().any(|g| g.kind == SnapKind::Spacing));
    }

    #[test]
    fn equal_spacing_can_be_disabled() {
        let mut s = Snapper::new(SnapConfig {
            equal_spacing: false,
            ..SnapConfig::default()
        });
        let others = [
            rect(0.0, 0.0, 100.0, 100.0),
            rect(140.0, 0.0, 100.0, 100.0),
        ];
        // 280 is exactly the spacing slot, but the feature is off, and no edge
        // is within 5 px.
        let moving = rect(283.0, 0.0, 100.0, 100.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(!r.snapped_x);
    }

    #[test]
    fn both_axes_can_snap_at_once() {
        let mut s = Snapper::new(SnapConfig::default());
        let others = [rect(100.0, 100.0, 50.0, 50.0)];
        let moving = rect(102.0, 98.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        assert!(r.snapped_x && r.snapped_y);
        assert_eq!(r.guides.len(), 2, "one guide per axis");
    }

    #[test]
    fn guides_merge_instead_of_piling_up() {
        let mut s = Snapper::new(SnapConfig::default());
        // Three rects sharing the same left edge.
        let others = [
            rect(100.0, 0.0, 10.0, 10.0),
            rect(100.0, 200.0, 10.0, 10.0),
            rect(100.0, 400.0, 10.0, 10.0),
        ];
        let moving = rect(102.0, 300.0, 20.0, 20.0);
        let r = s.snap(moving, &others, 1.0);
        let guides: Vec<_> = r.guides.iter().filter(|g| g.axis == Axis::X).collect();
        assert_eq!(guides.len(), 1, "same coordinate must collapse to one guide");
        // The surviving guide spans from the first reference to the dragged box.
        assert!(guides[0].start <= 0.0 && guides[0].end >= 320.0);
    }

    #[test]
    fn candidate_ranking_truncates_to_the_cap() {
        let mut s = Snapper::new(SnapConfig {
            max_candidates: 8,
            ..SnapConfig::default()
        });
        let others: Vec<Rect> = (0..1000)
            .map(|i| rect((i % 40) as f32 * 32.0, (i / 40) as f32 * 32.0, 24.0, 24.0))
            .collect();
        s.snap(rect(500.0, 500.0, 20.0, 20.0), &others, 1.0);
        assert_eq!(s.retained_candidates(), 8);
    }

    #[test]
    fn bigger_and_closer_candidates_outrank_small_distant_ones() {
        let mut s = Snapper::new(SnapConfig {
            max_candidates: 1,
            ..SnapConfig::default()
        });
        let huge_far = rect(5000.0, 5000.0, 2000.0, 2000.0);
        let small_near = rect(0.0, 0.0, 8.0, 8.0);
        s.snap(rect(0.0, 0.0, 20.0, 20.0), &[small_near, huge_far], 1.0);
        // 8 px at distance ~14 scores ~64/30 = 2; the 4e6 px frame at distance
        // ~7000 scores ~4e6/7000 = 570, so the significant object survives the
        // cap. That is the intended tie-break.
        assert_eq!(s.retained_candidates(), 1);
    }

    #[test]
    fn the_dragged_entity_does_not_snap_to_itself() {
        let mut tree = LooseQuadtree::new(4096.0);
        tree.insert(1, rect(100.0, 100.0, 50.0, 50.0), 0);
        tree.insert(2, rect(400.0, 400.0, 50.0, 50.0), 0);
        let mut s = Snapper::new(SnapConfig::default());
        let viewport = rect(0.0, 0.0, 1000.0, 1000.0);
        // Drag entity 1 to 2 px away from its own old position.
        let r = s.snap_against_tree(
            &mut tree,
            rect(102.0, 100.0, 50.0, 50.0),
            Some(1),
            viewport,
            1.0,
        );
        assert!(!r.is_snapped(), "self-snap is a bug: {r:?}");
    }

    #[test]
    fn tree_candidates_respect_the_viewport_padding() {
        let mut tree = LooseQuadtree::new(65_536.0);
        tree.insert(1, rect(0.0, 0.0, 10.0, 10.0), 0);
        tree.insert(2, rect(9000.0, 0.0, 10.0, 10.0), 0);
        let mut s = Snapper::new(SnapConfig::default());
        let viewport = rect(-100.0, -100.0, 200.0, 200.0);
        s.snap_against_tree(&mut tree, rect(0.0, 300.0, 10.0, 10.0), None, viewport, 1.0);
        assert_eq!(s.retained_candidates(), 1, "only the near rect qualifies");
    }

    #[test]
    fn a_blank_canvas_never_snaps() {
        let mut s = Snapper::new(SnapConfig::default());
        let r = s.snap(rect(1.0, 2.0, 3.0, 4.0), &[], 1.0);
        assert!(!r.is_snapped());
        assert!(r.guides.is_empty());
    }
}

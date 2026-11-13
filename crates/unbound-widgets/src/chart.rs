//! Line, area and bar charts on the canvas.
//!
//! The chart is a *view*: it owns ranges and hover state, never the data. The
//! axes choose round-number guides, and every series is mapped through the same
//! [`ChartDesc::map_point`] so a tooltip and a plotted dot can never disagree.
//! Curves are flattened to quadratic segments on the CPU, which keeps the GPU
//! path identical to the wire renderer.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_vector::cubic_to_quads;

use crate::primitive::{glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The plot inset: left for value labels, bottom for category labels.
pub const CHART_INSET: (f32, f32, f32, f32) = (36.0, 10.0, 10.0, 20.0);
/// The curve width.
pub const CHART_STROKE: f32 = 2.0;
/// The tooltip size.
pub const TOOLTIP_SIZE: (f32, f32) = (140.0, 44.0);
/// How close the pointer must be to select a point, in screen pixels.
pub const HOVER_RADIUS: f32 = 18.0;

/// How a series is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeriesKind {
    /// A smooth line.
    #[default]
    Line,
    /// A line with a filled baseline.
    Area,
    /// One bar per point.
    Bar,
}

/// One series.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSeries {
    /// The points, in data space.
    pub points: Vec<Vec2>,
    /// The series colour.
    pub token: Token,
    /// How to draw it.
    pub kind: SeriesKind,
}

impl ChartSeries {
    #[must_use]
    pub fn line(points: Vec<Vec2>, token: Token) -> Self {
        Self {
            points,
            token,
            kind: SeriesKind::Line,
        }
    }

    #[must_use]
    pub fn area(points: Vec<Vec2>, token: Token) -> Self {
        Self {
            points,
            token,
            kind: SeriesKind::Area,
        }
    }

    #[must_use]
    pub fn bars(points: Vec<Vec2>, token: Token) -> Self {
        Self {
            points,
            token,
            kind: SeriesKind::Bar,
        }
    }
}

/// A chart descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartDesc {
    /// The full chart rectangle.
    pub rect: Rect,
    /// The series.
    pub series: Vec<ChartSeries>,
    /// The x data range.
    pub x_range: (f32, f32),
    /// The y data range.
    pub y_range: (f32, f32),
    /// The number of guide lines per axis.
    pub divisions: u32,
    /// Whether to paint the grid.
    pub show_grid: bool,
    /// The draw order key.
    pub z: u16,
}

impl ChartDesc {
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            series: Vec::new(),
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
            divisions: 4,
            show_grid: true,
            z: 570,
        }
    }

    /// The same chart with series.
    #[must_use]
    pub fn with_series(mut self, series: Vec<ChartSeries>) -> Self {
        // Derive the ranges from the data when they have not been set.
        let bounds = series.iter().flat_map(|series| series.points.iter()).fold(
            None::<(f32, f32, f32, f32)>,
            |acc, point| {
                Some(match acc {
                    None => (point.x, point.x, point.y, point.y),
                    Some((min_x, max_x, min_y, max_y)) => (
                        min_x.min(point.x),
                        max_x.max(point.x),
                        min_y.min(point.y),
                        max_y.max(point.y),
                    ),
                })
            },
        );
        if let Some((min_x, max_x, min_y, max_y)) = bounds {
            if self.x_range == (0.0, 1.0) {
                self.x_range = (min_x, max_x);
            }
            if self.y_range == (0.0, 1.0) {
                self.y_range = (min_y, max_y);
            }
        }
        self.series = series;
        self
    }

    /// The plot area, inside the axes.
    #[must_use]
    pub fn plot_rect(&self) -> Rect {
        let (left, top, right, bottom) = CHART_INSET;
        Rect::new(
            self.rect.origin.x + left,
            self.rect.origin.y + top,
            (self.rect.size.x - left - right).max(1.0),
            (self.rect.size.y - top - bottom).max(1.0),
        )
    }

    /// Map a data point into the plot area.
    #[must_use]
    pub fn map_point(&self, point: Vec2) -> Vec2 {
        let plot = self.plot_rect();
        let span_x = (self.x_range.1 - self.x_range.0).abs().max(f32::EPSILON);
        let span_y = (self.y_range.1 - self.y_range.0).abs().max(f32::EPSILON);
        let tx = (point.x - self.x_range.0) / span_x;
        let ty = (point.y - self.y_range.0) / span_y;
        Vec2::new(
            plot.origin.x + tx.clamp(0.0, 1.0) * plot.size.x,
            plot.max().y - ty.clamp(0.0, 1.0) * plot.size.y,
        )
    }

    /// The horizontal guide positions.
    #[must_use]
    pub fn y_guides(&self) -> Vec<f32> {
        let plot = self.plot_rect();
        (0..=self.divisions)
            .map(|index| plot.origin.y + plot.size.y * index as f32 / self.divisions.max(1) as f32)
            .collect()
    }

    /// The vertical guide positions.
    #[must_use]
    pub fn x_guides(&self) -> Vec<f32> {
        let plot = self.plot_rect();
        (0..=self.divisions)
            .map(|index| plot.origin.x + plot.size.x * index as f32 / self.divisions.max(1) as f32)
            .collect()
    }

    /// The nearest `(series, index)` to a screen point.
    #[must_use]
    pub fn nearest(&self, pointer: Vec2) -> Option<(usize, usize)> {
        let mut best: Option<(f32, usize, usize)> = None;
        for (series_index, series) in self.series.iter().enumerate() {
            for (point_index, point) in series.points.iter().enumerate() {
                let screen = self.map_point(*point);
                let distance = (screen - pointer).length_squared();
                if distance <= HOVER_RADIUS * HOVER_RADIUS
                    && best.is_none_or(|(d, _, _)| distance < d)
                {
                    best = Some((distance, series_index, point_index));
                }
            }
        }
        best.map(|(_, series, point)| (series, point))
    }
}

/// The hover state of one chart.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChartState {
    /// The pointer position, when it is inside the chart.
    pub pointer: Option<Vec2>,
}

impl ChartState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the pointer position.
    pub fn hover(&mut self, pointer: Vec2) {
        self.pointer = Some(pointer);
    }

    /// Clear the hover.
    pub fn clear(&mut self) {
        self.pointer = None;
    }
}

/// What one chart emits.
#[derive(Clone, Debug)]
pub struct ChartOutput {
    /// The grid, series, markers and tooltip.
    pub output: WidgetOutput,
    /// The tooltip rectangle, when a point is hovered.
    pub tooltip: Option<Rect>,
    /// The hovered data point.
    pub hovered: Option<Vec2>,
}

/// Emit a chart. `tooltip_glyphs` are painted inside the tooltip.
#[must_use]
pub fn emit_chart(
    node: u64,
    desc: &ChartDesc,
    state: &ChartState,
    tooltip_glyphs: &[GlyphQuad],
    control: ControlState,
) -> ChartOutput {
    let plot = desc.plot_rect();
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::new();
    let mut payloads: Vec<GpuPayload> = Vec::new();
    let z = desc.z;

    if desc.show_grid {
        for y in desc.y_guides() {
            primitives.push(surface(
                Rect::new(plot.origin.x, y - 0.5, plot.size.x, 1.0),
                0.0,
                Token::DIVIDER,
                z,
            ));
        }
        for x in desc.x_guides() {
            primitives.push(surface(
                Rect::new(x - 0.5, plot.origin.y, 1.0, plot.size.y),
                0.0,
                Token::DIVIDER,
                z,
            ));
        }
    }

    let slot = if desc.series.is_empty() {
        1.0
    } else {
        plot.size.x
            / desc
                .series
                .iter()
                .map(|series| series.points.len())
                .max()
                .unwrap_or(1)
                .max(1) as f32
    };

    for series in &desc.series {
        match series.kind {
            SeriesKind::Bar => {
                let width = (slot * 0.6).max(1.0);
                for point in &series.points {
                    let mapped = desc.map_point(*point);
                    primitives.push(surface(
                        Rect::new(
                            mapped.x - width * 0.5,
                            mapped.y,
                            width,
                            (plot.max().y - mapped.y).max(1.0),
                        ),
                        2.0,
                        series.token,
                        z + 1,
                    ));
                }
            }
            SeriesKind::Line | SeriesKind::Area => {
                if series.kind == SeriesKind::Area {
                    for point in &series.points {
                        let mapped = desc.map_point(*point);
                        primitives.push(surface(
                            Rect::new(
                                mapped.x - slot * 0.5,
                                mapped.y,
                                slot,
                                plot.max().y - mapped.y,
                            ),
                            0.0,
                            Token::PRIMARY_SOFT,
                            z + 1,
                        ));
                    }
                }
                push_line(&mut primitives, &mut payloads, desc, series, z + 2);
            }
        }
    }

    let hovered = state.pointer.and_then(|pointer| desc.nearest(pointer));
    // The crosshair, highlight dot and tooltip keep fixed slots even when
    // nothing is hovered: opacity alone carries their visibility, so moving
    // the pointer never reshapes the primitive pool.
    let hover_paint = hovered.map(|(series_index, point_index)| {
        let series = &desc.series[series_index];
        let mapped = desc.map_point(series.points[point_index]);
        let bubble = Rect::new(
            (mapped.x + 10.0)
                .min(plot.max().x - TOOLTIP_SIZE.0)
                .max(plot.origin.x),
            (mapped.y - TOOLTIP_SIZE.1 - 8.0).max(plot.origin.y),
            TOOLTIP_SIZE.0,
            TOOLTIP_SIZE.1,
        );
        (mapped, series.token, bubble)
    });
    let (mapped, dot_token, bubble) = hover_paint.unwrap_or((Vec2::ZERO, Token::NONE, Rect::ZERO));
    let hover_opacity = if hover_paint.is_some() { 1.0 } else { 0.0 };

    let mut crosshair = surface(
        Rect::new(mapped.x - 0.5, plot.origin.y, 1.0, plot.size.y),
        0.0,
        Token::TEXT_SECONDARY,
        z + 3,
    );
    crosshair.z_and_opacity =
        GpuPrimitiveInstance::pack_z_opacity(crosshair.depth_u16(), hover_opacity);
    primitives.push(crosshair);

    let mut dot = surface(
        Rect::new(mapped.x - 4.0, mapped.y - 4.0, 8.0, 8.0),
        4.0,
        dot_token,
        z + 3,
    );
    dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), hover_opacity);
    primitives.push(dot);

    let mut tooltip_bg = surface(bubble, 8.0, Token::HUD_BG, z + 4);
    tooltip_bg.z_and_opacity =
        GpuPrimitiveInstance::pack_z_opacity(tooltip_bg.depth_u16(), hover_opacity);
    primitives.push(tooltip_bg);

    let glyph_start = primitives.len();
    glyphs(
        tooltip_glyphs,
        Token::HUD_TEXT,
        z + 5,
        &mut primitives,
        &mut payloads,
    );
    if hover_paint.is_none() {
        for primitive in &mut primitives[glyph_start..] {
            primitive.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(primitive.depth_u16(), 0.0);
        }
    }
    let tooltip = hover_paint.map(|(_, _, bubble)| bubble);

    let mut a11y = A11yNode::new(Role::Group, "图表", desc.rect).disabled(control.disabled);
    a11y.value = hovered
        .and_then(|(series, point)| {
            desc.series
                .get(series)
                .and_then(|series| series.points.get(point))
                .map(|point| format!("{:.2}, {:.2}", point.x, point.y))
        })
        .unwrap_or_default();

    ChartOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: Some(HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)),
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        tooltip,
        hovered: hovered.and_then(|(series, point)| {
            desc.series
                .get(series)
                .and_then(|series| series.points.get(point))
                .copied()
        }),
    }
}

fn push_line(
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
    desc: &ChartDesc,
    series: &ChartSeries,
    z: u16,
) {
    let mapped: Vec<Vec2> = series
        .points
        .iter()
        .map(|point| desc.map_point(*point))
        .collect();
    if mapped.len() < 2 {
        return;
    }
    for index in 0..mapped.len() - 1 {
        let p0 = mapped[index.saturating_sub(1)];
        let p1 = mapped[index];
        let p2 = mapped[index + 1];
        let p3 = mapped[(index + 2).min(mapped.len() - 1)];
        let c1 = p1 + (p2 - p0) / 6.0;
        let c2 = p2 - (p3 - p1) / 6.0;
        for quad in cubic_to_quads(p1, c1, c2, p2, 0.2) {
            let min = Vec2::new(
                quad.p0.x.min(quad.p1.x).min(quad.p2.x),
                quad.p0.y.min(quad.p1.y).min(quad.p2.y),
            );
            let max = Vec2::new(
                quad.p0.x.max(quad.p1.x).max(quad.p2.x),
                quad.p0.y.max(quad.p1.y).max(quad.p2.y),
            );
            let origin = Rect::new(
                min.x,
                min.y,
                (max.x - min.x).max(1.0),
                (max.y - min.y).max(1.0),
            );
            let payload_idx = payloads.len() as u32;
            payloads.push(GpuPayload::curve(
                [quad.p0.x - origin.origin.x, quad.p0.y - origin.origin.y],
                [quad.p1.x - origin.origin.x, quad.p1.y - origin.origin.y],
                [quad.p2.x - origin.origin.x, quad.p2.y - origin.origin.y],
                CHART_STROKE,
            ));
            let mut primitive = instance(
                origin,
                [0.0; 4],
                series.token,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Curve,
                SubElement::Content,
                z,
            );
            primitive.extra_payload_idx = payload_idx;
            primitives.push(primitive);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ChartDesc {
        ChartDesc::new(Rect::new(0.0, 0.0, 400.0, 240.0)).with_series(vec![ChartSeries::line(
            vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 5.0),
                Vec2::new(2.0, 2.0),
            ],
            Token::PRIMARY,
        )])
    }

    #[test]
    fn the_ranges_follow_the_data() {
        let desc = desc();
        assert_eq!(desc.x_range, (0.0, 2.0));
        assert_eq!(desc.y_range, (0.0, 5.0));
    }

    #[test]
    fn the_corners_map_to_the_plot_corners() {
        let desc = desc();
        let plot = desc.plot_rect();
        let low = desc.map_point(Vec2::new(0.0, 0.0));
        let high = desc.map_point(Vec2::new(2.0, 5.0));
        assert!((low.x - plot.origin.x).abs() < 0.001);
        assert!((low.y - plot.max().y).abs() < 0.001);
        assert!((high.x - plot.max().x).abs() < 0.001);
        assert!((high.y - plot.origin.y).abs() < 0.001);
    }

    #[test]
    fn the_guides_bracket_the_plot() {
        let desc = desc();
        let ys = desc.y_guides();
        let xs = desc.x_guides();
        assert_eq!(ys.len(), desc.divisions as usize + 1);
        assert_eq!(xs.len(), desc.divisions as usize + 1);
        assert_eq!(ys[0], desc.plot_rect().origin.y);
        assert_eq!(xs[0], desc.plot_rect().origin.x);
    }

    #[test]
    fn hovering_selects_the_nearest_point() {
        let desc = desc();
        let target = desc.map_point(Vec2::new(1.0, 5.0));
        assert_eq!(desc.nearest(target + Vec2::new(3.0, 2.0)), Some((0, 1)));
        assert_eq!(desc.nearest(target + Vec2::new(100.0, 100.0)), None);
    }

    #[test]
    fn a_bar_series_emits_one_rect_per_point() {
        let series = ChartSeries::bars(
            vec![
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 2.0),
                Vec2::new(2.0, 3.0),
            ],
            Token::SUCCESS,
        );
        let desc = ChartDesc::new(Rect::new(0.0, 0.0, 400.0, 240.0)).with_series(vec![series]);
        let output = emit_chart(3, &desc, &ChartState::new(), &[], ControlState::default());
        let bars = output
            .output
            .primitives
            .iter()
            .filter(|primitive| primitive.fill_token_or_uv == u32::from(Token::SUCCESS.raw()))
            .count();
        assert_eq!(bars, 3);
    }

    #[test]
    fn hovering_adds_a_tooltip_inside_the_chart() {
        let desc = desc();
        let mut state = ChartState::new();
        state.hover(desc.map_point(Vec2::new(1.0, 5.0)));
        let output = emit_chart(3, &desc, &state, &[], ControlState::default());
        let tooltip = output.tooltip.expect("tooltip");
        assert!(desc.rect.contains_rect(&tooltip));
        assert_eq!(output.hovered, Some(Vec2::new(1.0, 5.0)));
        assert!(output.output.a11y.as_ref().unwrap().value.contains(','));
    }

    #[test]
    fn an_empty_chart_still_paints_its_grid() {
        let desc = ChartDesc::new(Rect::new(0.0, 0.0, 400.0, 240.0));
        let output = emit_chart(3, &desc, &ChartState::new(), &[], ControlState::default());
        // Ten grid lines plus the three always-present hover slots
        // (crosshair, highlight dot, tooltip background), hidden at opacity 0.
        assert_eq!(output.output.primitives.len(), 13);
        assert!(output.tooltip.is_none());
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let glyphs = [GlyphQuad {
            rect: Rect::new(0.0, 0.0, 8.0, 12.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        }];
        let mut counts = Vec::new();
        for pointer in [None, Some(desc.map_point(Vec2::new(1.0, 5.0)))] {
            let mut state = ChartState::new();
            if let Some(point) = pointer {
                state.hover(point);
            }
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output = emit_chart(3, &desc, &state, &glyphs, control);
                            counts.push(output.output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

//! 画布上的折线图、面积图与柱状图（Chart）。
//!
//! 图表是一个*视图*：它持有坐标范围与悬停状态，从不持有原始数据。
//! 坐标轴会选择整数刻度参考线，且每个序列都通过同一 [`ChartDesc::map_point`]
//! 映射，因此提示框与绘制点绝不会出现位置偏差。曲线在 CPU 上展平为二次段，
//! 从而使 GPU 路径与线框渲染器保持一致。

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

/// 绘图区内边距：左侧留给数值标签，底部留给分类标签。
pub const CHART_INSET: (f32, f32, f32, f32) = (36.0, 10.0, 10.0, 20.0);
/// 曲线宽度。
pub const CHART_STROKE: f32 = 2.0;
/// 提示框尺寸。
pub const TOOLTIP_SIZE: (f32, f32) = (140.0, 44.0);
/// 选中数据点所需的最大指针距离（屏幕像素）。
pub const HOVER_RADIUS: f32 = 18.0;

/// 序列的绘制方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeriesKind {
    /// 平滑折线。
    #[default]
    Line,
    /// 带填充基线的折线（面积图）。
    Area,
    /// 每个数据点对应一根柱子。
    Bar,
}

/// 单个数据序列。
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSeries {
    /// 数据空间中的点集。
    pub points: Vec<Vec2>,
    /// 序列颜色。
    pub token: Token,
    /// 绘制方式。
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

/// 图表描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ChartDesc {
    /// 完整图表矩形。
    pub rect: Rect,
    /// 数据序列列表。
    pub series: Vec<ChartSeries>,
    /// X 轴数据范围。
    pub x_range: (f32, f32),
    /// Y 轴数据范围。
    pub y_range: (f32, f32),
    /// 每个坐标轴的参考线数量。
    pub divisions: u32,
    /// 是否绘制网格。
    pub show_grid: bool,
    /// 绘制顺序键。
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

    /// 附加数据序列。
    #[must_use]
    pub fn with_series(mut self, series: Vec<ChartSeries>) -> Self {
        // 当未显式设置范围时，从数据自动派生。
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

    /// 坐标轴内部的绘图区域。
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

    /// 将数据点映射到绘图区域坐标。
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

    /// 水平参考线位置。
    #[must_use]
    pub fn y_guides(&self) -> Vec<f32> {
        let plot = self.plot_rect();
        (0..=self.divisions)
            .map(|index| plot.origin.y + plot.size.y * index as f32 / self.divisions.max(1) as f32)
            .collect()
    }

    /// 垂直参考线位置。
    #[must_use]
    pub fn x_guides(&self) -> Vec<f32> {
        let plot = self.plot_rect();
        (0..=self.divisions)
            .map(|index| plot.origin.x + plot.size.x * index as f32 / self.divisions.max(1) as f32)
            .collect()
    }

    /// 距离屏幕坐标最近的 `(series, index)`。
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

/// 单个图表的悬停状态。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChartState {
    /// 指针位于图表内时的坐标位置。
    pub pointer: Option<Vec2>,
}

impl ChartState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录指针位置。
    pub fn hover(&mut self, pointer: Vec2) {
        self.pointer = Some(pointer);
    }

    /// 清除悬停状态。
    pub fn clear(&mut self) {
        self.pointer = None;
    }
}

/// 单个图表的发射结果。
#[derive(Clone, Debug)]
pub struct ChartOutput {
    /// 网格、序列、标记点与提示框。
    pub output: WidgetOutput,
    /// 悬停某点时的提示框矩形。
    pub tooltip: Option<Rect>,
    /// 当前悬停的数据点。
    pub hovered: Option<Vec2>,
}

/// 发射图表。`tooltip_glyphs` 绘制在提示框内部。
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
    // 即使未悬停任何点，十字准线、高亮圆点与提示框也保留固定槽位：
    // 仅通过不透明度控制可见性，因此移动指针绝不会重排图元池。
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
        // 十条网格线加上三个始终存在的悬停槽位
        // （十字准线、高亮圆点、提示框背景），未悬停时以不透明度 0 隐藏。
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

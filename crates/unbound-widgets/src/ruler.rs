//! 画布标尺与参考线。
//!
//! 标尺是刻度尺而非数字列表：它会在当前缩放下选取“整洁”的步长
//! （1、2 或 5 乘以 10 的幂）以避免标签重叠，随后为每个步长绘制刻度。
//! 从标尺拖出即可创建参考线，而参考线本质上是吸附 Pass 可用的世界坐标。
//!

use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::surface;
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 最小可见刻度间距（屏幕像素）。
pub const RULER_MIN_STEP: f32 = 8.0;
/// 标尺条带厚度。
pub const RULER_BAND: f32 = 22.0;
/// 两个主刻度之间的次刻度数量。
pub const RULER_MINOR_PER_MAJOR: u32 = 5;

/// 标尺或参考线所沿的坐标轴。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideAxis {
    /// 垂直参考线，从顶部标尺拖出。
    X,
    /// 水平参考线，从左侧标尺拖出。
    Y,
}

/// 世界坐标下的参考线。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideLine {
    /// 线段所沿的坐标轴。
    pub axis: GuideAxis,
    /// 世界坐标值。
    pub world: f32,
}

/// 环绕画布视口的标尺描述符。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerDesc {
    /// 内容区域（不含标尺）。
    pub viewport: Rect,
    /// 缩放系数。
    pub zoom: f32,
    /// 视口左上角对应的世界坐标点。
    pub origin: Vec2,
    /// 绘制顺序键。
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

    /// 顶部标尺条带矩形。
    #[must_use]
    pub fn top_rect(&self) -> Rect {
        Rect::new(
            self.viewport.origin.x,
            self.viewport.origin.y - RULER_BAND,
            self.viewport.size.x,
            RULER_BAND,
        )
    }

    /// 左侧标尺条带矩形。
    #[must_use]
    pub fn left_rect(&self) -> Rect {
        Rect::new(
            self.viewport.origin.x - RULER_BAND,
            self.viewport.origin.y,
            RULER_BAND,
            self.viewport.size.y,
        )
    }

    /// 两条标尺条带交汇处的角落方块。
    #[must_use]
    pub fn corner_rect(&self) -> Rect {
        Rect::new(
            self.viewport.origin.x - RULER_BAND,
            self.viewport.origin.y - RULER_BAND,
            RULER_BAND,
            RULER_BAND,
        )
    }

    /// 沿单轴的可见世界坐标范围。
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

    /// 将世界坐标转换为视口内的屏幕偏移。
    #[must_use]
    pub fn screen(&self, axis: GuideAxis, world: f32) -> f32 {
        match axis {
            GuideAxis::X => self.viewport.origin.x + (world - self.origin.x) * self.zoom,
            GuideAxis::Y => self.viewport.origin.y + (world - self.origin.y) * self.zoom,
        }
    }

    /// 将视口内的屏幕偏移转换为世界坐标。
    #[must_use]
    pub fn world(&self, axis: GuideAxis, screen: f32) -> f32 {
        match axis {
            GuideAxis::X => self.origin.x + (screen - self.viewport.origin.x) / self.zoom,
            GuideAxis::Y => self.origin.y + (screen - self.viewport.origin.y) / self.zoom,
        }
    }

    /// 单轴上的刻度列表：`(世界坐标, 是否为主刻度)`。
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

    /// 按步长精度舍入后的刻度标签文本。
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

/// 当前缩放下的“整洁”世界步长：1、2 或 5 乘以 10 的幂。
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

/// 单个画布的参考线状态。
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

    /// 已提交的参考线列表。
    #[must_use]
    pub fn guides(&self) -> &[GuideLine] {
        &self.guides
    }

    /// 正在拖出的参考线（若有）。
    #[must_use]
    pub fn pending(&self) -> Option<GuideLine> {
        self.pending
    }

    /// 从标尺条带开始拖拽一条新参考线。
    pub fn begin(&mut self, axis: GuideAxis, world: f32) {
        self.pending = Some(GuideLine { axis, world });
    }

    /// 移动待提交的参考线。
    pub fn update(&mut self, world: f32) {
        if let Some(pending) = &mut self.pending {
            pending.world = world;
        }
    }

    /// 提交待定参考线；返回此前是否存在待定参考线。
    pub fn end(&mut self) -> bool {
        match self.pending.take() {
            Some(guide) => {
                self.guides.push(guide);
                true
            }
            None => false,
        }
    }

    /// 取消待定参考线。
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// 移除 `axis` 上最接近 `world` 的参考线。
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

    /// 移除所有参考线。
    pub fn clear(&mut self) {
        self.guides.clear();
        self.pending = None;
    }
}

/// 标尺控件的输出结果。
#[derive(Clone, Debug)]
pub struct RulerOutput {
    /// 条带、刻度与参考线图元。
    pub output: WidgetOutput,
    /// 两条标尺条带命中区域，用于启动参考线拖拽。
    pub bands: Vec<(GuideAxis, HitRegion)>,
}

/// 输出标尺与参考线。
#[must_use]
pub fn emit_rulers(
    node: u64,
    desc: &RulerDesc,
    state: &RulerState,
    control: ControlState,
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
    if control.disabled {
        bands.clear();
    } else {
        bands.retain(|(axis, _)| match axis {
            GuideAxis::X => desc.top_rect().size.x > 0.0,
            GuideAxis::Y => desc.left_rect().size.y > 0.0,
        });
    }

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

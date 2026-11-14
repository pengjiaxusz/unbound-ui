//! 贝塞尔曲线与渐变色带编辑器（CurveEditor / GradientRampEditor）。
//!
//! 两者共享同一核心思路：将 `0..=1` 归一化值映射到屏幕轨道上。
//! 曲线编辑器存储三次缓动曲线 `(0,0) - p1 - p2 - (1,1)` 的两个控制点；
//! 渐变编辑器存储按偏移排序的色标。
//!
//! 曲线绘制为点状折线。这是刻意为之的设计，而非使用描边路径图元：
//! 点状曲线无需变换表和曲面细分缓冲区，使编辑器保持纯发射器特性，同时视觉上依然清晰可辨。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Color, Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{focus_ring, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 两个编辑器的内边距。
pub const CURVE_PADDING: f32 = 18.0;
/// 用于逼近曲线的采样点数。
pub const CURVE_DOTS: usize = 40;
/// 控制点手柄半径。
pub const CURVE_HANDLE: f32 = 5.0;
/// 渐变色标正方形边长。
pub const STOP_SIZE: f32 = 12.0;

/// 手势操作的目标控制点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveHandle {
    /// 第一个控制点。
    P1,
    /// 第二个控制点。
    P2,
}

/// 曲线编辑器的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CurveEditorDesc {
    /// 编辑器矩形。
    pub rect: Rect,
    /// 归一化坐标系框使用的内边距。
    pub padding: f32,
    /// 无障碍标签。
    pub label: String,
    /// 绘制顺序键。
    pub z: u16,
}

impl CurveEditorDesc {
    /// 使用默认内边距创建曲线编辑器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            padding: CURVE_PADDING,
            label: String::new(),
            z: 748,
        }
    }

    /// 可绘制区域框架。
    #[must_use]
    pub fn frame(&self) -> Rect {
        self.rect.inflate(-self.padding)
    }

    /// 将归一化点映射到屏幕空间。
    #[must_use]
    pub fn to_screen(&self, point: Vec2) -> Vec2 {
        let frame = self.frame();
        Vec2::new(
            frame.origin.x + point.x * frame.size.x,
            frame.origin.y + (1.0 - point.y) * frame.size.y,
        )
    }

    /// 将屏幕坐标映射到归一化空间，x 轴限制在 `0..=1`。
    #[must_use]
    pub fn from_screen(&self, position: Vec2) -> Vec2 {
        let frame = self.frame();
        let x = if frame.size.x <= 0.0 {
            0.0
        } else {
            ((position.x - frame.origin.x) / frame.size.x).clamp(0.0, 1.0)
        };
        let y = if frame.size.y <= 0.0 {
            0.0
        } else {
            1.0 - (position.y - frame.origin.y) / frame.size.y
        };
        Vec2::new(x, y)
    }
}

/// 曲线编辑器的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct CurveEditorState {
    p1: Vec2,
    p2: Vec2,
    active: Option<CurveHandle>,
}

impl CurveEditorState {
    /// 标准 `ease` 缓动曲线。
    #[must_use]
    pub fn new() -> Self {
        Self {
            p1: Vec2::new(0.25, 0.1),
            p2: Vec2::new(0.25, 1.0),
            active: None,
        }
    }

    /// 使用显式控制点创建曲线。
    #[must_use]
    pub fn with_points(p1: Vec2, p2: Vec2) -> Self {
        Self {
            p1: Vec2::new(p1.x.clamp(0.0, 1.0), p1.y),
            p2: Vec2::new(p2.x.clamp(0.0, 1.0), p2.y),
            active: None,
        }
    }

    /// 第一个控制点。
    #[must_use]
    pub fn p1(&self) -> Vec2 {
        self.p1
    }

    /// 第二个控制点。
    #[must_use]
    pub fn p2(&self) -> Vec2 {
        self.p2
    }

    /// 当前正在拖拽的手柄。
    #[must_use]
    pub fn active(&self) -> Option<CurveHandle> {
        self.active
    }

    /// 曲线在时间 `t` 处的值。
    #[must_use]
    pub fn sample(&self, t: f32) -> Vec2 {
        let t = t.clamp(0.0, 1.0);
        let u = 1.0 - t;
        let b = 3.0 * u * u * t;
        let c = 3.0 * u * t * t;
        let d = t * t * t;
        Vec2::new(
            b * self.p1.x + c * self.p2.x + d,
            b * self.p1.y + c * self.p2.y + d,
        )
    }

    /// 时间 `t` 处的 `y` 值。
    #[must_use]
    pub fn ease(&self, t: f32) -> f32 {
        self.sample(t).y
    }

    /// 开始拖拽手柄。
    pub fn begin(&mut self, handle: CurveHandle) {
        self.active = Some(handle);
    }

    /// 将当前手柄拖拽至归一化坐标点。
    ///
    /// `x` 坐标限制在 `0..=1` 内以保证三次贝塞尔缓动仍是时间的单值函数；
    /// `y` 坐标不受限制以支持过冲效果。
    pub fn drag_to(&mut self, point: Vec2) -> bool {
        let Some(handle) = self.active else {
            return false;
        };
        let point = Vec2::new(point.x.clamp(0.0, 1.0), point.y);
        let target = match handle {
            CurveHandle::P1 => &mut self.p1,
            CurveHandle::P2 => &mut self.p2,
        };
        let changed = *target != point;
        *target = point;
        changed
    }

    /// 结束手势拖拽。
    pub fn end(&mut self) {
        self.active = None;
    }

    /// CSS `cubic-bezier(...)` 字符串表示。
    #[must_use]
    pub fn css(&self) -> String {
        format!(
            "cubic-bezier({:.2}, {:.2}, {:.2}, {:.2})",
            self.p1.x, self.p1.y, self.p2.x, self.p2.y
        )
    }
}

impl Default for CurveEditorState {
    fn default() -> Self {
        Self::new()
    }
}

/// 单个渐变色标。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// 位于 `0..=1` 的位置。
    pub offset: f32,
    /// 该位置的颜色。
    pub color: Color,
}

/// 渐变色带的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct GradientEditorState {
    stops: Vec<GradientStop>,
    active: Option<usize>,
}

impl GradientEditorState {
    /// 创建黑到白的基础渐变色带。
    #[must_use]
    pub fn new() -> Self {
        Self {
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: Color::from_srgb8(0, 0, 0, 255),
                },
                GradientStop {
                    offset: 1.0,
                    color: Color::from_srgb8(255, 255, 255, 255),
                },
            ],
            active: None,
        }
    }

    /// 按偏移量排序的色标列表。
    #[must_use]
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops
    }

    /// 当前正在拖拽的色标索引。
    #[must_use]
    pub fn active(&self) -> Option<usize> {
        self.active
    }

    /// 添加色标，返回其在排序列表中的索引。
    pub fn add_stop(&mut self, stop: GradientStop) -> usize {
        let offset = stop.offset.clamp(0.0, 1.0);
        let index = self
            .stops
            .iter()
            .position(|existing| existing.offset > offset)
            .unwrap_or(self.stops.len());
        self.stops.insert(index, GradientStop { offset, ..stop });
        index
    }

    /// 移除色标。色带至少保留两个色标。
    pub fn remove_stop(&mut self, index: usize) -> bool {
        if self.stops.len() <= 2 || index >= self.stops.len() {
            return false;
        }
        self.stops.remove(index);
        self.active = None;
        true
    }

    /// 开始拖拽指定色标。
    pub fn begin(&mut self, index: usize) -> bool {
        if index >= self.stops.len() {
            return false;
        }
        self.active = Some(index);
        true
    }

    /// 将当前色标拖拽至 `offset`。
    ///
    /// 移动范围限制在相邻色标之间以维持列表有序，
    /// 从而使 [`Self::color_at`] 只需执行简单的线性扫描。
    pub fn drag_to(&mut self, offset: f32) -> bool {
        let Some(index) = self.active else {
            return false;
        };
        let low = if index == 0 {
            0.0
        } else {
            self.stops[index - 1].offset
        };
        let high = if index + 1 >= self.stops.len() {
            1.0
        } else {
            self.stops[index + 1].offset
        };
        let offset = offset.clamp(low, high);
        let changed = (self.stops[index].offset - offset).abs() > f32::EPSILON;
        self.stops[index].offset = offset;
        changed
    }

    /// 结束手势拖拽。
    pub fn end(&mut self) {
        self.active = None;
    }

    /// 在 Oklab 空间中插值计算 `offset` 处的颜色。
    #[must_use]
    pub fn color_at(&self, offset: f32) -> Color {
        let offset = offset.clamp(0.0, 1.0);
        let mut left = &self.stops[0];
        let mut right = self.stops.last().unwrap();
        for pair in self.stops.windows(2) {
            if offset >= pair[0].offset && offset <= pair[1].offset {
                left = &pair[0];
                right = &pair[1];
                break;
            }
        }
        if right.offset <= left.offset {
            return left.color;
        }
        let t = (offset - left.offset) / (right.offset - left.offset);
        left.color
            .to_oklab()
            .lerp(right.color.to_oklab(), t)
            .to_linear()
    }
}

impl Default for GradientEditorState {
    fn default() -> Self {
        Self::new()
    }
}

/// 发射曲线编辑器。
#[must_use]
pub fn emit_curve_editor(
    node: u64,
    desc: &CurveEditorDesc,
    state: &CurveEditorState,
    control: ControlState,
) -> WidgetOutput {
    let rect = desc.rect;
    let frame = desc.frame();
    let mut primitives = Vec::with_capacity(CURVE_DOTS + 20);

    primitives.push(surface(rect, 8.0, Token::SURFACE_SUNKEN, desc.z));
    for index in 1..4 {
        let t = index as f32 / 4.0;
        primitives.push(instance(
            Rect::new(
                frame.origin.x,
                frame.origin.y + frame.size.y * t,
                frame.size.x,
                1.0,
            ),
            [0.0; 4],
            Token::GRID,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Background,
            desc.z,
        ));
        primitives.push(instance(
            Rect::new(
                frame.origin.x + frame.size.x * t,
                frame.origin.y,
                1.0,
                frame.size.y,
            ),
            [0.0; 4],
            Token::GRID,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Background,
            desc.z,
        ));
    }

    for step in 0..=CURVE_DOTS {
        let point = desc.to_screen(state.sample(step as f32 / CURVE_DOTS as f32));
        primitives.push(surface(
            Rect::new(point.x - 1.25, point.y - 1.25, 2.5, 2.5),
            1.25,
            Token::PRIMARY,
            desc.z,
        ));
    }

    // 两条切线辅助杆以点状绘制，因此无需变换矩阵。
    for (from, to) in [(Vec2::ZERO, state.p1), (Vec2::new(1.0, 1.0), state.p2)] {
        for step in 0..=8 {
            let t = step as f32 / 8.0;
            let point = desc.to_screen(Vec2::new(
                from.x + (to.x - from.x) * t,
                from.y + (to.y - from.y) * t,
            ));
            primitives.push(surface(
                Rect::new(point.x - 0.75, point.y - 0.75, 1.5, 1.5),
                0.75,
                Token::OUTLINE_STRONG,
                desc.z,
            ));
        }
    }

    for (handle, point) in [(CurveHandle::P1, state.p1), (CurveHandle::P2, state.p2)] {
        let screen = desc.to_screen(point);
        let active = state.active == Some(handle);
        primitives.push(outlined(
            Rect::new(
                screen.x - CURVE_HANDLE,
                screen.y - CURVE_HANDLE,
                CURVE_HANDLE * 2.0,
                CURVE_HANDLE * 2.0,
            ),
            CURVE_HANDLE,
            if active {
                Token::PRIMARY
            } else {
                Token::SURFACE_RAISED
            },
            Token::PRIMARY,
            1.5,
            0.0,
            desc.z,
        ));
    }

    primitives.push(focus_ring(
        rect.inflate(3.0),
        11.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [8.0; 4] })
            .with_cursor(CursorKind::Crosshair)
    });
    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.css();

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 发射渐变色带编辑器。
#[must_use]
pub fn emit_gradient_editor(
    node: u64,
    rect: Rect,
    state: &GradientEditorState,
    control: ControlState,
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8 + state.stops.len() * 2);
    let track = Rect::new(
        rect.origin.x,
        rect.origin.y + rect.size.y * 0.5 - 6.0,
        rect.size.x,
        12.0,
    );
    primitives.push(surface(rect, 6.0, Token::SURFACE_SUNKEN, z));
    // 滑轨下方的棋盘格底纹，确保调用方叠加真实渐变后透明度依然清晰可辨。
    for index in 0..8u16 {
        let cell = track.size.x / 8.0;
        let x = track.origin.x + f32::from(index) * cell;
        primitives.push(surface(
            Rect::new(x, track.origin.y, cell, track.size.y),
            0.0,
            if index % 2 == 0 {
                Token::SURFACE
            } else {
                Token::SURFACE_SUNKEN
            },
            z,
        ));
    }
    for (index, stop) in state.stops.iter().enumerate() {
        let x = track.origin.x + stop.offset * track.size.x;
        let square = Rect::new(
            x - STOP_SIZE * 0.5,
            track.center().y - STOP_SIZE * 0.5,
            STOP_SIZE,
            STOP_SIZE,
        );
        primitives.push(surface(square, 3.0, Token::SURFACE_RAISED, z));
        primitives.push(outlined(
            square,
            3.0,
            Token::NONE,
            if state.active == Some(index) {
                Token::PRIMARY
            } else {
                Token::OUTLINE_STRONG
            },
            if state.active == Some(index) {
                2.5
            } else {
                1.0
            },
            0.0,
            z,
        ));
    }

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii: [6.0; 4] })
            .with_cursor(CursorKind::Crosshair)
    });
    let mut a11y = A11yNode::new(Role::Group, "gradient".to_owned(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state
        .stops
        .iter()
        .map(|stop| format!("{:.2}", stop.offset))
        .collect::<Vec<_>>()
        .join(", ");

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CurveEditorDesc {
        CurveEditorDesc::new(Rect::new(0.0, 0.0, 240.0, 200.0))
    }

    #[test]
    fn the_curve_hits_both_endpoints() {
        let state = CurveEditorState::new();
        assert_eq!(state.sample(0.0), Vec2::ZERO);
        assert_eq!(state.sample(1.0), Vec2::new(1.0, 1.0));
    }

    #[test]
    fn the_default_curve_is_monotonic() {
        let state = CurveEditorState::new();
        let mut previous = 0.0;
        for step in 0..=20 {
            let y = state.ease(step as f32 / 20.0);
            assert!(y >= previous - 1e-6);
            previous = y;
        }
    }

    #[test]
    fn dragging_moves_only_the_active_handle() {
        let mut state = CurveEditorState::new();
        assert!(!state.drag_to(Vec2::new(0.5, 0.5)), "no active handle");
        state.begin(CurveHandle::P1);
        assert!(state.drag_to(Vec2::new(0.4, 0.2)));
        assert_eq!(state.p1(), Vec2::new(0.4, 0.2));
        assert_eq!(state.p2(), Vec2::new(0.25, 1.0));
        state.end();
        assert_eq!(state.active(), None);
    }

    #[test]
    fn dragging_clamps_time_but_allows_overshoot() {
        let mut state = CurveEditorState::new();
        state.begin(CurveHandle::P2);
        assert!(state.drag_to(Vec2::new(2.0, 1.6)));
        assert_eq!(state.p2(), Vec2::new(1.0, 1.6));
        state.drag_to(Vec2::new(-1.0, -0.4));
        assert_eq!(state.p2(), Vec2::new(0.0, -0.4));
    }

    #[test]
    fn screen_mapping_round_trips_the_frame() {
        let desc = desc();
        for point in [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.25),
            Vec2::new(1.0, 1.0),
        ] {
            let back = desc.from_screen(desc.to_screen(point));
            assert!((back.x - point.x).abs() < 1e-5);
            assert!((back.y - point.y).abs() < 1e-5);
        }
    }

    #[test]
    fn gradient_stops_stay_sorted() {
        let mut state = GradientEditorState::new();
        let index = state.add_stop(GradientStop {
            offset: 0.5,
            color: Color::from_srgb8(255, 0, 0, 255),
        });
        assert_eq!(index, 1);
        assert_eq!(state.stops().len(), 3);
        assert!(state
            .stops()
            .windows(2)
            .all(|pair| pair[0].offset <= pair[1].offset));
    }

    #[test]
    fn the_last_two_stops_cannot_be_removed() {
        let mut state = GradientEditorState::new();
        assert!(!state.remove_stop(0));
        assert!(!state.remove_stop(5));
        state.add_stop(GradientStop {
            offset: 0.5,
            color: Color::from_srgb8(0, 255, 0, 255),
        });
        assert!(state.remove_stop(1));
        assert_eq!(state.stops().len(), 2);
    }

    #[test]
    fn a_stop_is_clamped_between_its_neighbours() {
        let mut state = GradientEditorState::new();
        state.add_stop(GradientStop {
            offset: 0.5,
            color: Color::from_srgb8(255, 0, 0, 255),
        });
        assert!(state.begin(1));
        assert!(state.drag_to(2.0));
        assert!((state.stops()[1].offset - 1.0).abs() < 1e-6);
        state.drag_to(-0.5);
        assert!((state.stops()[1].offset - 0.0).abs() < 1e-6);
        state.end();
        assert_eq!(state.active(), None);
    }

    #[test]
    fn the_ramp_interpolates_in_oklab() {
        let mut state = GradientEditorState::new();
        state.add_stop(GradientStop {
            offset: 0.5,
            color: Color::from_srgb8(255, 0, 0, 255),
        });
        let mid = state.color_at(0.5);
        assert!(mid.r > 0.5, "halfway between black and red is red-ish");

        let exact = state.color_at(0.0);
        assert!(exact.r < 1e-4 && exact.g < 1e-4 && exact.b < 1e-4);
    }

    #[test]
    fn the_curve_editor_reports_its_css_and_cursor() {
        let state = CurveEditorState::with_points(Vec2::new(0.42, 0.0), Vec2::new(0.58, 1.0));
        let output = emit_curve_editor(31, &desc(), &state, ControlState::default());
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.value, "cubic-bezier(0.42, 0.00, 0.58, 1.00)");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Crosshair))
        );
        assert_eq!(
            output.primitives.len(),
            1 + 6 + (CURVE_DOTS + 1) + 18 + 2 + 1
        );
    }

    #[test]
    fn the_gradient_editor_paints_one_square_per_stop() {
        let state = GradientEditorState::new();
        let output = emit_gradient_editor(
            32,
            Rect::new(0.0, 0.0, 200.0, 32.0),
            &state,
            ControlState::default(),
            10,
        );
        assert_eq!(output.primitives.len(), 1 + 8 + state.stops().len() * 2);
        assert_eq!(output.a11y.expect("reachable").value, "0.00, 1.00");
    }
}

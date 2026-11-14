//! 双层命中测试（`docs` §6.1）。
//!
//! 管线刻意分为两阶段，与架构文档完全一致：
//!
//! 1. **粗筛阶段（broad phase）**——针对区域包围盒及所有生效裁剪框的 AABB 测试；
//!    
//! 2. **精测阶段（narrow phase）**——真实几何求交：圆角矩形 SDF（支持四角独立半径，
//!    因此 GPU 的双槽位圆角掩码是其严格子集）、奇偶规则多边形测试，
//!    或二次贝塞尔曲线解析距离。
//!
//! 命中区域是纯值类型而非 trait 对象：调用方可从 Taffy 树、画布四叉树
//! 或 Portal 浮层构建它们，而本 crate 无需感知这些上层结构。排序键为
//! `(layer, z, node)`，使拾取结果与插入顺序无关——这一特性保证了热重载后
//! 输入行为的确定性。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Rect, Vec2};
use unbound_vector::{hit_quad, QuadSegment};

use crate::cursor::CursorKind;

/// 首个浮层层级。处于该层级或之上的元素在光标优先级上胜过下方的 UI 层
/// （`docs` §6.2）。
pub const PORTAL_LAYER: u8 = 2;

/// 圆角矩形裁剪框，`GpuClipNode` 的 CPU 侧镜像。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClipRect {
    pub rect: Rect,
    /// 四角圆角半径，顺序为 `TL, TR, BR, BL`（左上、右上、右下、左下）。
    pub radii: [f32; 4],
}

impl ClipRect {
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            radii: [0.0; 4],
        }
    }

    #[must_use]
    pub fn rounded(rect: Rect, radius: f32) -> Self {
        Self {
            rect,
            radii: [radius; 4],
        }
    }

    /// 当 `point` 通过该裁剪测试时返回 `true`。
    #[must_use]
    pub fn contains(&self, point: Vec2) -> bool {
        if !self.rect.contains_point(point) {
            return false;
        }
        if self.radii.iter().all(|r| *r <= 0.0) {
            return true;
        }
        rounded_rect_sdf(self.rect, self.radii, point) <= 0.0
    }
}

/// 命中区域的精测几何形状。
///
/// 所有变体均在与**区域包围盒相同的空间**（父节点坐标系）中求值，
/// 因此多边形顶点、贝塞尔控制点与待测点可直接在同一空间内比较。
///
///
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// 普通矩形。
    Rect,
    /// 带四角独立圆角半径（`TL, TR, BR, BL`）的矩形。
    Rounded { radii: [f32; 4] },
    /// 闭合多边形，采用奇偶填充规则。
    ///
    /// 顶点坐标与区域的 `bounds` 处于同一坐标系。
    Polygon(Vec<Vec2>),
    /// 二次贝塞尔描边：当距离小于线宽一半时判定命中。
    ///
    /// 控制点与区域的 `bounds` 处于同一坐标系。
    Bezier { quad: QuadSegment, half_width: f32 },
}

impl Shape {
    #[must_use]
    pub fn contains(&self, bounds: Rect, point: Vec2) -> bool {
        match self {
            Self::Rect => bounds.contains_point(point),
            Self::Rounded { radii } => rounded_rect_sdf(bounds, *radii, point) <= 0.0,
            Self::Polygon(points) => point_in_polygon(points, point),
            Self::Bezier { quad, half_width } => {
                bounds.inflate(*half_width).contains_point(point)
                    && hit_quad(quad, point, *half_width)
            }
        }
    }
}

/// 单个可拾取表面区域。
#[derive(Clone, Debug, PartialEq)]
pub struct HitRegion {
    /// 节点唯一标识，用作最终排序的平局决胜键。
    pub node: u64,
    /// 覆盖层级：`0` 画布、`1` UI、`2..=254` Portal、`255` 调试层。
    pub layer: u8,
    /// 层内拓扑顺序；数值越大越靠上。
    pub z: u32,
    pub bounds: Rect,
    pub shape: Shape,
    /// 从祖先继承的裁剪框，由调用方预先求交。
    pub clip: Option<ClipRect>,
    /// 为 `false` 时该区域正常绘制但对输入透明（`pointer-events: none`），
    /// 从而允许按钮内部的文本标签实现点击穿透。
    pub receive_events: bool,
    /// 指针悬停在该区域上时请求的光标类型（若有）。
    pub cursor: Option<CursorKind>,
}

impl HitRegion {
    /// 接收事件的矩形区域。
    #[must_use]
    pub fn new(node: u64, layer: u8, z: u32, bounds: Rect) -> Self {
        Self {
            node,
            layer,
            z,
            bounds,
            shape: Shape::Rect,
            clip: None,
            receive_events: true,
            cursor: None,
        }
    }

    /// 设置指针悬停在该区域上时请求的光标。
    #[must_use]
    pub fn with_cursor(mut self, cursor: CursorKind) -> Self {
        self.cursor = Some(cursor);
        self
    }

    #[must_use]
    pub fn with_shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }

    #[must_use]
    pub fn with_clip(mut self, clip: ClipRect) -> Self {
        self.clip = Some(clip);
        self
    }

    #[must_use]
    pub fn pass_through(mut self) -> Self {
        self.receive_events = false;
        self
    }

    /// 对单个点执行粗筛 + 精测。
    #[must_use]
    pub fn hit(&self, point: Vec2) -> bool {
        if !self.receive_events {
            return false;
        }
        if !self.bounds.contains_point(point) {
            // 贝塞尔曲线情形会在 `Shape` 内部按半线宽向外膨胀；
            // 其余形状均可在此直接剔除。
            if !matches!(self.shape, Shape::Bezier { .. }) {
                return false;
            }
        }
        if let Some(clip) = &self.clip {
            if !clip.contains(point) {
                return false;
            }
        }
        self.shape.contains(self.bounds, point)
    }

    /// 排序键：数值越大越优先。节点 ID 确保全序唯一。
    fn rank(&self) -> (u8, u32, u64) {
        (self.layer, self.z, self.node)
    }
}

/// 到圆角矩形的有向距离；内部为负值。
///
/// 根据采样点所在象限选取对应圆角半径，这与超级着色器（uber-shader）
/// 双槽位圆角掩码所用的近似一致：等半径时精确，不等半径时保守
/// （绝不大于真实形状）。
///
#[must_use]
pub fn rounded_rect_sdf(bounds: Rect, radii: [f32; 4], point: Vec2) -> f32 {
    let half = Vec2::new(bounds.size.x * 0.5, bounds.size.y * 0.5);
    if half.x < 0.0 || half.y < 0.0 || !bounds.is_finite() {
        return f32::INFINITY;
    }
    let local = point - bounds.center();
    let radius = if local.x >= 0.0 {
        if local.y >= 0.0 {
            radii[2]
        } else {
            radii[1]
        }
    } else if local.y >= 0.0 {
        radii[3]
    } else {
        radii[0]
    };
    let radius = radius.max(0.0).min(half.x.min(half.y));
    let q = Vec2::new(local.x.abs(), local.y.abs()) - (half - Vec2::splat(radius));
    let outside = Vec2::new(q.x.max(0.0), q.y.max(0.0)).length();
    let inside = q.x.max(q.y).min(0.0);
    outside + inside - radius
}

/// 奇偶规则点在多边形内判定。
#[must_use]
pub fn point_in_polygon(points: &[Vec2], point: Vec2) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[j];
        // 向 +X 方向发射射线并统计交点数；半开区间比较可避免顶点被重复计数。
        //
        if (a.y > point.y) != (b.y > point.y) {
            let t = (point.y - a.y) / (b.y - a.y);
            if point.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// 返回 `point` 下方最顶层的命中区域，若无则返回 `None`。
#[must_use]
pub fn pick(regions: &[HitRegion], point: Vec2) -> Option<&HitRegion> {
    regions
        .iter()
        .filter(|region| region.hit(point))
        .max_by_key(|region| region.rank())
}

/// 返回 `point` 下方最顶层命中区域的索引。
#[must_use]
pub fn pick_index(regions: &[HitRegion], point: Vec2) -> Option<usize> {
    regions
        .iter()
        .enumerate()
        .filter(|(_, region)| region.hit(point))
        .max_by_key(|(_, region)| region.rank())
        .map(|(index, _)| index)
}

/// 返回 `point` 下方的所有命中区域，按从顶层到底层排序。
///
/// 即光标栈消费的“拾取路径”：首项决定点击目标，其余项决定当首项为
/// 穿透区域或缩放手柄时指针应呈现的外观。
///
#[must_use]
pub fn pick_stack(regions: &[HitRegion], point: Vec2) -> Vec<&HitRegion> {
    let mut hits: Vec<&HitRegion> = regions.iter().filter(|r| r.hit(point)).collect();
    hits.sort_by_key(|r| core::cmp::Reverse(r.rank()));
    hits
}

/// 触发原生窗口缩放拖拽的窗口边缘或角点。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    /// 上边缘。
    North,
    /// 下边缘。
    South,
    /// 左边缘。
    West,
    /// 右边缘。
    East,
    /// 左上角。
    NorthWest,
    /// 右上角。
    NorthEast,
    /// 左下角。
    SouthWest,
    /// 右下角。
    SouthEast,
}

impl ResizeEdge {
    /// 指示该边缘方向的光标类型。
    #[must_use]
    pub fn cursor(self) -> CursorKind {
        match self {
            Self::North | Self::South => CursorKind::ResizeNorthSouth,
            Self::West | Self::East => CursorKind::ResizeEastWest,
            Self::NorthWest | Self::SouthEast => CursorKind::ResizeNorthWestSouthEast,
            Self::NorthEast | Self::SouthWest => CursorKind::ResizeNorthEastSouthWest,
        }
    }
}

/// 返回 `point` 下方的缩放边界带（若有）。
///
/// `border` 为直边带宽，`corner` 为四个对角手柄的正方形触发范围；
/// 先检测角点，使同时落入两处边界带的点获得双向对角光标。
///
#[must_use]
pub fn resize_edge(point: Vec2, size: Vec2, border: f32, corner: f32) -> Option<ResizeEdge> {
    if size.x <= 0.0
        || size.y <= 0.0
        || point.x < 0.0
        || point.y < 0.0
        || point.x > size.x
        || point.y > size.y
    {
        return None;
    }
    let border = border.max(0.0).min(size.x * 0.5).min(size.y * 0.5);
    let corner = corner.max(border).min(size.x * 0.5).min(size.y * 0.5);
    let left = point.x <= border;
    let right = point.x >= size.x - border;
    let top = point.y <= border;
    let bottom = point.y >= size.y - border;
    let corner_left = point.x <= corner;
    let corner_right = point.x >= size.x - corner;
    let corner_top = point.y <= corner;
    let corner_bottom = point.y >= size.y - corner;
    if corner_left && corner_top {
        return Some(ResizeEdge::NorthWest);
    }
    if corner_right && corner_top {
        return Some(ResizeEdge::NorthEast);
    }
    if corner_left && corner_bottom {
        return Some(ResizeEdge::SouthWest);
    }
    if corner_right && corner_bottom {
        return Some(ResizeEdge::SouthEast);
    }
    if top {
        return Some(ResizeEdge::North);
    }
    if bottom {
        return Some(ResizeEdge::South);
    }
    if left {
        return Some(ResizeEdge::West);
    }
    if right {
        return Some(ResizeEdge::East);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_plain_rect_hits_inside_only() {
        let region = HitRegion::new(1, 1, 0, rect(0.0, 0.0, 100.0, 50.0));
        assert!(region.hit(Vec2::new(50.0, 25.0)));
        assert!(region.hit(Vec2::new(0.0, 0.0)));
        assert!(!region.hit(Vec2::new(-1.0, 25.0)));
        assert!(!region.hit(Vec2::new(50.0, 51.0)));
    }

    #[test]
    fn rounded_corners_reject_the_corner_pixel() {
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let region =
            HitRegion::new(1, 1, 0, bounds).with_shape(Shape::Rounded { radii: [20.0; 4] });
        assert!(region.hit(Vec2::new(50.0, 50.0)));
        assert!(!region.hit(Vec2::new(1.0, 1.0)), "corner must be cut away");
        assert!(region.hit(Vec2::new(20.0, 20.0)), "inside the arc");
        assert!(rounded_rect_sdf(bounds, [20.0; 4], Vec2::new(50.0, 50.0)) < 0.0);
        assert!(rounded_rect_sdf(bounds, [20.0; 4], Vec2::new(0.0, 0.0)) > 0.0);
    }

    #[test]
    fn an_asymmetric_radius_cuts_only_its_own_corner() {
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        // 仅左上角为圆角。
        let radii = [30.0, 0.0, 0.0, 0.0];
        assert!(rounded_rect_sdf(bounds, radii, Vec2::new(2.0, 2.0)) > 0.0);
        assert!(rounded_rect_sdf(bounds, radii, Vec2::new(98.0, 2.0)) < 0.0);
    }

    #[test]
    fn a_clip_rejects_points_outside_it() {
        let region = HitRegion::new(1, 1, 0, rect(0.0, 0.0, 100.0, 100.0))
            .with_clip(ClipRect::new(rect(0.0, 0.0, 50.0, 100.0)));
        assert!(region.hit(Vec2::new(25.0, 50.0)));
        assert!(!region.hit(Vec2::new(75.0, 50.0)));
    }

    #[test]
    fn a_rounded_clip_respects_its_arc() {
        let clip = ClipRect::rounded(rect(0.0, 0.0, 100.0, 100.0), 40.0);
        assert!(clip.contains(Vec2::new(50.0, 50.0)));
        assert!(!clip.contains(Vec2::new(3.0, 3.0)));
        assert!(!clip.contains(Vec2::new(-5.0, 50.0)));
    }

    #[test]
    fn higher_layers_win_regardless_of_z() {
        let regions = vec![
            HitRegion::new(1, 1, 999, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 2, 0, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(pick(&regions, Vec2::new(50.0, 50.0)).unwrap().node, 2);
    }

    #[test]
    fn z_orders_within_a_layer() {
        let regions = vec![
            HitRegion::new(1, 1, 1, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 1, 2, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(pick(&regions, Vec2::new(50.0, 50.0)).unwrap().node, 2);
    }

    #[test]
    fn pick_is_independent_of_the_input_order() {
        let a = HitRegion::new(7, 1, 5, rect(0.0, 0.0, 10.0, 10.0));
        let b = HitRegion::new(9, 1, 5, rect(0.0, 0.0, 10.0, 10.0));
        let forward = vec![a.clone(), b.clone()];
        let backward = vec![b, a];
        assert_eq!(
            pick(&forward, Vec2::new(1.0, 1.0)).unwrap().node,
            pick(&backward, Vec2::new(1.0, 1.0)).unwrap().node
        );
    }

    #[test]
    fn pass_through_regions_are_skipped() {
        let regions = vec![
            HitRegion::new(1, 1, 9, rect(0.0, 0.0, 100.0, 100.0)).pass_through(),
            HitRegion::new(2, 1, 1, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(pick(&regions, Vec2::new(50.0, 50.0)).unwrap().node, 2);
    }

    #[test]
    fn window_edges_start_the_matching_resize() {
        let size = Vec2::new(1000.0, 700.0);
        assert_eq!(
            resize_edge(Vec2::new(2.0, 350.0), size, 6.0, 14.0),
            Some(ResizeEdge::West)
        );
        assert_eq!(
            resize_edge(Vec2::new(500.0, 698.0), size, 6.0, 14.0),
            Some(ResizeEdge::South)
        );
        assert_eq!(
            resize_edge(Vec2::new(997.0, 3.0), size, 6.0, 14.0),
            Some(ResizeEdge::NorthEast)
        );
        assert_eq!(
            resize_edge(Vec2::new(3.0, 697.0), size, 6.0, 14.0),
            Some(ResizeEdge::SouthWest)
        );
    }

    #[test]
    fn the_window_middle_is_not_a_resize() {
        let size = Vec2::new(1000.0, 700.0);
        assert_eq!(resize_edge(Vec2::new(500.0, 350.0), size, 6.0, 14.0), None);
        assert_eq!(resize_edge(Vec2::new(-1.0, 350.0), size, 6.0, 14.0), None);
    }

    #[test]
    fn polygons_use_even_odd_fill() {
        let tri = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(0.0, 100.0),
        ];
        assert!(point_in_polygon(&tri, Vec2::new(10.0, 10.0)));
        assert!(!point_in_polygon(&tri, Vec2::new(90.0, 90.0)));
        assert!(!point_in_polygon(&tri, Vec2::new(-1.0, 10.0)));
    }

    #[test]
    fn a_bezier_stroke_hits_near_the_curve_only() {
        let quad = QuadSegment {
            p0: Vec2::new(0.0, 0.0),
            p1: Vec2::new(50.0, 100.0),
            p2: Vec2::new(100.0, 0.0),
        };
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let region = HitRegion::new(1, 0, 0, bounds).with_shape(Shape::Bezier {
            quad,
            half_width: 4.0,
        });
        // 曲线中点位于 (50, 50)。
        assert!(region.hit(Vec2::new(50.0, 50.0)));
        assert!(!region.hit(Vec2::new(10.0, 90.0)));
    }

    #[test]
    fn the_pick_stack_is_ordered_topmost_first() {
        let regions = vec![
            HitRegion::new(1, 1, 1, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 1, 2, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(3, 3, 0, rect(0.0, 0.0, 100.0, 100.0)),
        ];
        let stack = pick_stack(&regions, Vec2::new(50.0, 50.0));
        assert_eq!(
            stack.iter().map(|r| r.node).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }

    #[test]
    fn a_degenerate_rect_is_never_hit() {
        let region = HitRegion::new(1, 1, 0, rect(0.0, 0.0, f32::NAN, 10.0));
        assert!(!region.hit(Vec2::new(0.0, 0.0)));
        assert_eq!(
            rounded_rect_sdf(rect(0.0, 0.0, f32::NAN, 1.0), [0.0; 4], Vec2::ZERO),
            f32::INFINITY
        );
    }
}

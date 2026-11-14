//! 轴对齐矩形：布局框、裁剪框、四叉树边界与脏矩形。

use bytemuck::{Pod, Zeroable};

use crate::vec2::Vec2;

/// 以原点 + 尺寸存储的轴对齐矩形。
///
/// 引擎全局不变量：布局矩形的 `size >= 0`；空交集由任一轴上的
/// `size <= 0` 表示，顶点着色器会将其折叠为退化四边形（参见 `docs` §3.2）。
///
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Rect {
    pub origin: Vec2,
    pub size: Vec2,
}

impl Rect {
    pub const ZERO: Self = Self {
        origin: Vec2::ZERO,
        size: Vec2::ZERO,
    };

    #[inline]
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            origin: Vec2::new(x, y),
            size: Vec2::new(w, h),
        }
    }

    #[inline]
    #[must_use]
    pub const fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self {
            origin: min,
            size: Vec2::new(max.x - min.x, max.y - min.y),
        }
    }

    #[inline]
    #[must_use]
    pub fn from_points(a: Vec2, b: Vec2) -> Self {
        Self::from_min_max(a.min(b), a.max(b))
    }

    #[inline]
    #[must_use]
    pub fn min(&self) -> Vec2 {
        self.origin
    }

    #[inline]
    #[must_use]
    pub fn max(&self) -> Vec2 {
        self.origin + self.size
    }

    #[inline]
    #[must_use]
    pub fn center(&self) -> Vec2 {
        self.origin + self.size * 0.5
    }

    #[inline]
    #[must_use]
    pub fn width(&self) -> f32 {
        self.size.x
    }

    #[inline]
    #[must_use]
    pub fn height(&self) -> f32 {
        self.size.y
    }

    #[inline]
    #[must_use]
    pub fn area(&self) -> f32 {
        self.size.x.max(0.0) * self.size.y.max(0.0)
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.x <= 0.0 || self.size.y <= 0.0
    }

    #[inline]
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.origin.is_finite() && self.size.is_finite()
    }

    #[inline]
    #[must_use]
    pub fn contains_point(&self, p: Vec2) -> bool {
        p.x >= self.origin.x
            && p.y >= self.origin.y
            && p.x <= self.origin.x + self.size.x
            && p.y <= self.origin.y + self.size.y
    }

    /// 当 `other` 完全位于 `self` 内部时返回 `true`。
    ///
    /// 包含边界的判断与 [`Self::contains_point`] 一致，后者将边界上的点
    /// 视为包含在内。
    #[inline]
    #[must_use]
    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.min().x >= self.min().x
            && other.min().y >= self.min().y
            && other.max().x <= self.max().x
            && other.max().y <= self.max().y
    }

    #[inline]
    #[must_use]
    pub fn intersects(&self, other: &Rect) -> bool {
        let a_max = self.max();
        let b_max = other.max();
        self.origin.x < b_max.x
            && other.origin.x < a_max.x
            && self.origin.y < b_max.y
            && other.origin.y < a_max.y
    }

    /// 当两个矩形重叠**或相触**时返回 `true`。
    ///
    /// [`Self::intersects`] 为严格相交：仅共享一条边的两个矩形不算相交。
    /// 这对于保守的粗筛阶段是正确的约定，但与 [`Self::contains_point`] 不同，
    /// 后者将边界点视为包含在内。精确框选测试需要包含边界的规则：
    /// 恰好落在图元边缘的选框也应选中该图元。
    ///
    ///
    #[inline]
    #[must_use]
    pub fn touches(&self, other: &Rect) -> bool {
        let a_max = self.max();
        let b_max = other.max();
        self.origin.x <= b_max.x
            && other.origin.x <= a_max.x
            && self.origin.y <= b_max.y
            && other.origin.y <= a_max.y
    }

    /// 求交集；当两矩形不相交时返回空矩形（尺寸为零或负）。
    /// 对于有限输入永不返回 `NaN`。
    #[inline]
    #[must_use]
    pub fn intersect(&self, other: &Rect) -> Rect {
        let min = self.min().max(other.min());
        let max = self.max().min(other.max());
        if max.x <= min.x || max.y <= min.y {
            Rect {
                origin: min,
                size: Vec2::ZERO,
            }
        } else {
            Rect::from_min_max(min, max)
        }
    }

    #[inline]
    #[must_use]
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::from_min_max(self.min().min(other.min()), self.max().max(other.max()))
    }

    #[inline]
    #[must_use]
    pub fn inflate(&self, by: f32) -> Rect {
        Rect::new(
            self.origin.x - by,
            self.origin.y - by,
            self.size.x + by * 2.0,
            self.size.y + by * 2.0,
        )
    }

    #[inline]
    #[must_use]
    pub fn inset(&self, by: f32) -> Rect {
        self.inflate(-by)
    }

    #[inline]
    #[must_use]
    pub fn translate(&self, by: Vec2) -> Rect {
        Rect {
            origin: self.origin + by,
            size: self.size,
        }
    }

    /// 绕矩形原点缩放——用于逻辑像素到物理像素的转换。
    #[inline]
    #[must_use]
    pub fn scale(&self, s: f32) -> Rect {
        Rect::new(
            self.origin.x * s,
            self.origin.y * s,
            self.size.x * s,
            self.size.y * s,
        )
    }

    /// 拆分为 `GpuClipNode` 所需的 `(min_x, min_y, max_x, max_y)`。
    #[inline]
    #[must_use]
    pub fn to_min_max_array(&self) -> [f32; 4] {
        let max = self.max();
        [self.origin.x, self.origin.y, max.x, max.y]
    }

    /// 从 `(min_x, min_y, max_x, max_y)` 重建矩形。
    #[inline]
    #[must_use]
    pub fn from_min_max_array(a: [f32; 4]) -> Rect {
        Rect {
            origin: Vec2::new(a[0], a[1]),
            size: Vec2::new(a[2] - a[0], a[3] - a[1]),
        }
    }

    /// 中心 + 半长形式，供圆角裁剪 SDF 槽位使用。
    #[inline]
    #[must_use]
    pub fn to_center_half(&self) -> [f32; 4] {
        let c = self.center();
        [c.x, c.y, self.size.x * 0.5, self.size.y * 0.5]
    }
}

/// 将**外层**圆角裁剪的圆角弧投影到**内层**矩形上。
///
/// 着色器仅提供两个圆角裁剪槽位，因此第三层及更深的嵌套容器必须在
/// CPU 端折叠。将外层圆角矩形与内层矩形求交，会把外层圆角弧转换为
/// 对应内角处更小的有效半径——即 CSS 用于嵌套圆角容器的
/// “内半径 = 外半径 − 内边距”规则。该结果仅在两矩形轴对齐时有效，
/// 这正是折叠路径所处理的情况。
///
///
/// `radii` 按 `[左上, 右上, 右下, 左下]` 排序，与图元圆角半径顺序
/// 及 `sd_rounded_box` 保持一致。
#[must_use]
pub fn project_rounded_radii(
    inner: Rect,
    inner_radii: [f32; 4],
    outer: Rect,
    outer_radii: [f32; 4],
) -> [f32; 4] {
    let mut out = inner_radii;
    let inner_min = inner.min();
    let outer_min = outer.min();
    let outer_max = outer.max();
    for (corner, radius) in outer_radii.iter().enumerate() {
        let r = *radius;
        if r <= 0.0 {
            continue;
        }
        // 按轴计算内角到外角的距离。
        let (dx, dy) = match corner {
            0 => (inner_min.x - outer_min.x, inner_min.y - outer_min.y),
            1 => (outer_max.x - inner.max().x, inner_min.y - outer_min.y),
            2 => (outer_max.x - inner.max().x, outer_max.y - inner.max().y),
            _ => (inner_min.x - outer_min.x, outer_max.y - inner.max().y),
        };
        // 内角至少需要保留外半径减去所在内边距；对于非均匀内边距，
        // 取两轴中的较大值作为保守选择。
        //
        let projected = (r - dx.max(dy)).max(0.0);
        out[corner] = out[corner].max(projected);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_of_overlapping_rects() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
    }

    #[test]
    fn disjoint_intersection_is_empty_not_nan() {
        let a = Rect::new(0.0, 0.0, 4.0, 4.0);
        let b = Rect::new(10.0, 10.0, 1.0, 1.0);
        let i = a.intersect(&b);
        assert!(i.is_empty());
        assert!(i.is_finite());
        assert_eq!(i.size, Vec2::ZERO);
    }

    #[test]
    fn union_expands_to_cover_both() {
        let a = Rect::new(0.0, 0.0, 4.0, 4.0);
        let b = Rect::new(-2.0, 6.0, 2.0, 2.0);
        let u = a.union(&b);
        assert!(u.contains_rect(&a));
        assert!(u.contains_rect(&b));
        assert_eq!(u, Rect::new(-2.0, 0.0, 6.0, 8.0));
    }

    #[test]
    fn center_half_round_trip() {
        let r = Rect::new(2.0, 4.0, 8.0, 6.0);
        let ch = r.to_center_half();
        assert_eq!(ch, [6.0, 7.0, 4.0, 3.0]);
    }

    #[test]
    fn min_max_array_round_trip() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(Rect::from_min_max_array(r.to_min_max_array()), r);
    }

    /// `intersects` 为严格判断，`touches` 为包含边界的判断。
    ///
    /// 两个矩形仅共享一条边：`intersects` 返回 false（保守粗筛的约定），
    /// `touches` 返回 true（精确框选测试的约定，与 `contains_point` 一致）。
    ///
    ///
    #[test]
    fn touching_rects_are_distinguished_from_intersecting_ones() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let touching = Rect::new(10.0, 0.0, 10.0, 10.0);
        let overlapping = Rect::new(5.0, 0.0, 10.0, 10.0);
        let apart = Rect::new(11.0, 0.0, 10.0, 10.0);

        assert!(!a.intersects(&touching));
        assert!(a.touches(&touching));
        assert!(a.intersects(&overlapping));
        assert!(a.touches(&overlapping));
        assert!(!a.touches(&apart));
    }

    #[test]
    fn a_nested_corner_radius_is_projected_onto_the_inner_rect() {
        let outer = Rect::new(0.0, 0.0, 100.0, 100.0);
        let inner = Rect::new(10.0, 10.0, 80.0, 80.0);
        let projected = project_rounded_radii(inner, [0.0; 4], outer, [20.0; 4]);
        // 每个内角位于外角内侧 10px，因此外层 20px 圆弧在内角处投影为
        // 20 - 10 = 10px。
        for r in projected {
            assert!((r - 10.0).abs() < 1e-3, "{projected:?}");
        }

        // 共享同一角点的容器保留完整的圆角半径。
        let same = project_rounded_radii(outer, [0.0; 4], outer, [20.0; 4]);
        assert!(same.iter().all(|r| (r - 20.0).abs() < 1e-4));

        // 远离外层圆弧的内层矩形不需要任何圆角半径。
        let far = project_rounded_radii(inner, [4.0; 4], outer, [8.0; 4]);
        assert_eq!(far, [4.0; 4]);
    }
}

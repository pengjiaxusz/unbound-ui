//! 选择集、框选（Marquee）与变换控制柄（`docs` §5.3、§6.1）。
//!
//! 画布从不向*组件*询问其位置：选择模型完全基于世界空间矩形工作，
//! 这些矩形由调用方从空间索引中解析得到。这使得 20 px 图标、40 000 px 画框与多选分组
//! 共用完全一致的手势代码，并保证每次指针交互都是 `(start bounds, pointer, modifiers)` 的纯函数。
//!
//!
//!
//! 此处所有状态均为瞬态：除非调用方在撤销栈上开启事务，否则不会发生任何 [`Selection`] 变更，
//! 因此取消拖拽不会留下任何痕迹。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::{Rect, Vec2};

use crate::quadtree::LooseQuadtree;

/// 改变手势语义的键盘修饰键状态。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        alt: false,
        ctrl: false,
    };
}

/// 八个缩放控制柄加上位于顶边上方的旋转控制柄。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    Rotate,
}

impl Handle {
    /// 按绘制顺序排列的所有控制柄。
    pub const ALL: [Self; 9] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
        Self::Rotate,
    ];

    /// 当控制柄同时缩放双轴时返回 true。
    #[must_use]
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Self::NorthWest | Self::NorthEast | Self::SouthEast | Self::SouthWest
        )
    }

    /// 控制柄在 `bounds` 上对应的世界空间位置。
    #[must_use]
    pub fn position(self, bounds: Rect, rotate_offset: f32) -> Vec2 {
        let min = bounds.min();
        let max = bounds.max();
        let mid = bounds.center();
        match self {
            Self::NorthWest => Vec2::new(min.x, min.y),
            Self::North => Vec2::new(mid.x, min.y),
            Self::NorthEast => Vec2::new(max.x, min.y),
            Self::East => Vec2::new(max.x, mid.y),
            Self::SouthEast => Vec2::new(max.x, max.y),
            Self::South => Vec2::new(mid.x, max.y),
            Self::SouthWest => Vec2::new(min.x, max.y),
            Self::West => Vec2::new(min.x, mid.y),
            Self::Rotate => Vec2::new(mid.x, min.y - rotate_offset),
        }
    }

    /// 查找 `point` 命中的控制柄（若有）。`radius` 单位为世界单位。
    #[must_use]
    pub fn hit(bounds: Rect, point: Vec2, radius: f32, rotate_offset: f32) -> Option<Self> {
        let mut best: Option<(f32, Self)> = None;
        for handle in Self::ALL {
            let delta = Self::position(handle, bounds, rotate_offset) - point;
            let distance = delta.length_squared();
            if distance <= radius * radius && best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, handle));
            }
        }
        best.map(|(_, handle)| handle)
    }

    /// 仅沿单轴移动的控制柄返回 true。
    #[must_use]
    pub fn axis_locked(self) -> bool {
        matches!(self, Self::North | Self::South | Self::East | Self::West)
    }
}

/// 与顺序无关的已选实体 ID 集合。
///
/// 此处有序 `Vec` 优于哈希集合：选择集通常较小，迭代顺序必须对归约器 `Action` 日志保持确定性，
/// 且成员测试只是连续内存上的二分查找。
///
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<u32>,
}

impl Selection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    #[must_use]
    pub fn contains(&self, id: u32) -> bool {
        self.ids.binary_search(&id).is_ok()
    }

    /// 按升序排列的已选 ID 切片。
    #[must_use]
    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// 若不存在则添加。集合发生变更时返回 `true`。
    pub fn insert(&mut self, id: u32) -> bool {
        match self.ids.binary_search(&id) {
            Ok(_) => false,
            Err(pos) => {
                self.ids.insert(pos, id);
                true
            }
        }
    }

    /// 若存在则移除。集合发生变更时返回 `true`。
    pub fn remove(&mut self, id: u32) -> bool {
        match self.ids.binary_search(&id) {
            Ok(pos) => {
                self.ids.remove(pos);
                true
            }
            Err(_) => false,
        }
    }

    /// 翻转选中状态。若最终为选中状态则返回 `true`。
    pub fn toggle(&mut self, id: u32) -> bool {
        if self.remove(id) {
            false
        } else {
            self.insert(id);
            true
        }
    }

    /// 替换整个选择集。
    pub fn set(&mut self, ids: &[u32]) {
        self.ids.clear();
        self.ids.extend_from_slice(ids);
        self.ids.sort_unstable();
        self.ids.dedup();
    }

    /// 通过 `lookup` 解析并计算所有已选实体的包围盒并集。
    #[must_use]
    pub fn bounds(&self, mut lookup: impl FnMut(u32) -> Option<Rect>) -> Option<Rect> {
        let mut acc: Option<Rect> = None;
        for id in &self.ids {
            let Some(rect) = lookup(*id) else { continue };
            acc = Some(match acc {
                None => rect,
                Some(current) => current.union(&rect),
            });
        }
        acc
    }
}

/// 框选拖拽对现有选择集的作用模式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Replace,
    Add,
    Subtract,
    Toggle,
}

impl SelectionMode {
    /// 根据用户按住的修饰键推导框选模式。
    #[must_use]
    pub fn from(modifiers: Modifiers) -> Self {
        if modifiers.shift && modifiers.ctrl {
            Self::Toggle
        } else if modifiers.shift {
            Self::Add
        } else if modifiers.ctrl {
            Self::Subtract
        } else {
            Self::Replace
        }
    }
}

/// 橡皮筋框选拖拽状态。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Marquee {
    pub origin: Vec2,
    pub current: Vec2,
}

impl Marquee {
    #[must_use]
    pub fn new(origin: Vec2) -> Self {
        Self {
            origin,
            current: origin,
        }
    }

    /// 更新自由角点坐标。
    pub fn drag_to(&mut self, point: Vec2) {
        self.current = point;
    }

    /// 归一化矩形，保证宽高永不为负。
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::from_points(self.origin, self.current)
    }

    /// 当拖拽距离大到足以构成框选而非点击时返回 true。
    #[must_use]
    pub fn is_active(&self, threshold: f32) -> bool {
        let r = self.rect();
        r.size.x.max(r.size.y) >= threshold
    }

    /// 收集与框选带相交的实体，并按 `mode` 更新 `selection`。
    ///
    /// 候选集来自四叉树，因此在百万实体画布上也只需测试与框选带实际重叠的对象。
    ///
    pub fn apply(&self, tree: &mut LooseQuadtree, mode: SelectionMode, selection: &mut Selection) {
        let band = self.rect();
        let mut hits = Vec::new();
        tree.query_exact(band, &mut hits);
        match mode {
            SelectionMode::Replace => selection.set(&hits),
            SelectionMode::Add => {
                for id in hits {
                    selection.insert(id);
                }
            }
            SelectionMode::Subtract => {
                for id in hits {
                    selection.remove(id);
                }
            }
            SelectionMode::Toggle => {
                for id in hits {
                    selection.toggle(id);
                }
            }
        }
    }
}

/// 控制柄拖拽产生的仿射变换修正量。
///
/// 该变换始终作用于被拖拽元素的*起始*矩形，因此 10 帧拖拽绝不会累积浮点误差：
/// 每一帧都直接基于初始快照重新计算。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Transform2 {
    pub anchor: Vec2,
    pub scale: Vec2,
    pub translate: Vec2,
    pub rotation: f32,
}

impl Default for Transform2 {
    fn default() -> Self {
        Self {
            anchor: Vec2::ZERO,
            scale: Vec2::ONE,
            translate: Vec2::ZERO,
            rotation: 0.0,
        }
    }
}

impl Transform2 {
    /// 恒等变换，用于无移动的点击。
    #[must_use]
    pub fn identity() -> Self {
        Self::default()
    }

    /// 当应用此变换不会产生任何改变时返回 true。
    #[must_use]
    pub fn is_identity(&self) -> bool {
        (self.scale.x - 1.0).abs() < 1e-6
            && (self.scale.y - 1.0).abs() < 1e-6
            && self.translate == Vec2::ZERO
            && self.rotation.abs() < 1e-6
    }

    /// 通过变换矩阵映射一个点。
    #[must_use]
    pub fn apply_point(&self, point: Vec2) -> Vec2 {
        let local = point - self.anchor;
        let rotated = local.rotate(self.rotation.cos(), self.rotation.sin());
        Vec2::new(
            self.anchor.x + rotated.x * self.scale.x + self.translate.x,
            self.anchor.y + rotated.y * self.scale.y + self.translate.y,
        )
    }

    /// 映射一个矩形，取四个变换后角点的包围盒。
    #[must_use]
    pub fn apply_rect(&self, rect: Rect) -> Rect {
        let min = rect.min();
        let max = rect.max();
        let corners = [
            self.apply_point(min),
            self.apply_point(Vec2::new(max.x, min.y)),
            self.apply_point(max),
            self.apply_point(Vec2::new(min.x, max.y)),
        ];
        let mut out_min = corners[0];
        let mut out_max = corners[0];
        for corner in &corners[1..] {
            out_min = out_min.min(*corner);
            out_max = out_max.max(*corner);
        }
        Rect::new(
            out_min.x,
            out_min.y,
            out_max.x - out_min.x,
            out_max.y - out_min.y,
        )
    }
}

/// 进行中的缩放或旋转拖拽会话。
#[derive(Copy, Clone, Debug)]
pub struct HandleDrag {
    handle: Handle,
    start_bounds: Rect,
    start_pointer: Vec2,
    /// 允许的最小尺寸（世界单位；文档中的 24 px 目标适用于命中区域，
    /// 此处是防止矩形翻转的几何下限）。
    min_size: f32,
    /// 旋转控制柄位于顶边上方的距离（世界单位）。
    rotate_offset: f32,
}

impl HandleDrag {
    /// 在 `pointer` 处开始拖拽 `handle`。
    #[must_use]
    pub fn new(handle: Handle, start_bounds: Rect, pointer: Vec2, rotate_offset: f32) -> Self {
        Self {
            handle,
            start_bounds,
            start_pointer: pointer,
            min_size: 1.0,
            rotate_offset,
        }
    }

    #[must_use]
    pub fn handle(&self) -> Handle {
        self.handle
    }

    /// 针对当前指针位置求解变换。
    ///
    /// 缩放比例为当前锚点到指针向量与拖拽起始时向量之比。
    /// 这单个表达式同时覆盖常规情况（锚点在对边）与 Alt 情况（锚点在中心），无需特殊分支，
    /// 且因始终基于起始快照重算而绝不累积误差。
    ///
    ///
    pub fn update(&mut self, pointer: Vec2, modifiers: Modifiers) -> Transform2 {
        if self.handle == Handle::Rotate {
            return self.rotate(pointer, modifiers);
        }
        let bounds = self.start_bounds;
        let min = bounds.min();
        let max = bounds.max();
        let centre = bounds.center();

        // 固定锚点：对边/对角，或按住 Alt 时的中心点。
        let anchor = if modifiers.alt {
            centre
        } else {
            match self.handle {
                Handle::East => Vec2::new(min.x, centre.y),
                Handle::West => Vec2::new(max.x, centre.y),
                Handle::North => Vec2::new(centre.x, max.y),
                Handle::South => Vec2::new(centre.x, min.y),
                Handle::NorthWest => max,
                Handle::NorthEast => Vec2::new(min.x, max.y),
                Handle::SouthEast => min,
                Handle::SouthWest => Vec2::new(max.x, min.y),
                Handle::Rotate => centre,
            }
        };

        let start_vec = self.start_pointer - anchor;
        let now_vec = pointer - anchor;
        let mut scale = Vec2::ONE;
        // 零跨度轴（细线）保持其缩放：没有可缩放的长度，除以零会污染变换。
        //
        if start_vec.x.abs() > 1e-6 {
            scale.x = now_vec.x / start_vec.x;
        }
        if start_vec.y.abs() > 1e-6 {
            scale.y = now_vec.y / start_vec.y;
        }

        // 锁定单轴的控制柄不得将缩放泄漏到另一轴。
        match self.handle {
            Handle::East | Handle::West => scale.y = 1.0,
            Handle::North | Handle::South => scale.x = 1.0,
            _ => {}
        }

        // 拖过锚点时必须停在几何下限处，而不能翻转。
        let floor_x = self.min_size / bounds.size.x.max(self.min_size);
        let floor_y = self.min_size / bounds.size.y.max(self.min_size);
        scale.x = scale.x.max(floor_x);
        scale.y = scale.y.max(floor_y);

        if modifiers.shift && self.handle.is_corner() {
            // 等比缩放：取指针移动幅度更大的轴为主导。
            let uniform = scale.x.max(scale.y);
            scale = Vec2::splat(uniform);
        }

        Transform2 {
            anchor,
            scale,
            translate: Vec2::ZERO,
            rotation: 0.0,
        }
    }

    fn rotate(&self, pointer: Vec2, modifiers: Modifiers) -> Transform2 {
        let centre = self.start_bounds.center();
        let start = (self.start_pointer - centre).normalized();
        let now = (pointer - centre).normalized();
        let mut angle = now.y.atan2(now.x) - start.y.atan2(start.x);
        // 折叠到 (-π, π] 区间。
        while angle > core::f32::consts::PI {
            angle -= core::f32::consts::TAU;
        }
        while angle <= -core::f32::consts::PI {
            angle += core::f32::consts::TAU;
        }
        if modifiers.shift {
            // 以 15° 为步进吸附。
            let step = core::f32::consts::PI / 12.0;
            angle = (angle / step).round() * step;
        }
        Transform2 {
            anchor: centre,
            scale: Vec2::ONE,
            translate: Vec2::ZERO,
            rotation: angle,
        }
    }

    /// 当前指针位置对应的投影包围盒，用于绘制预览轮廓。
    #[must_use]
    pub fn preview(&self, pointer: Vec2, modifiers: Modifiers) -> Rect {
        let mut copy = *self;
        copy.update(pointer, modifiers)
            .apply_rect(self.start_bounds)
    }

    /// 拖拽开始时记录的初始包围盒。
    #[must_use]
    pub fn start_bounds(&self) -> Rect {
        self.start_bounds
    }

    /// 旋转控制柄位于顶边上方的距离，确保调用方用同一数值绘制与命中测试。
    ///
    #[must_use]
    pub fn rotate_offset(&self) -> f32 {
        self.rotate_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn selection_is_ordered_and_deduplicated() {
        let mut s = Selection::new();
        s.set(&[5, 1, 5, 3]);
        assert_eq!(s.ids(), &[1, 3, 5]);
        assert!(s.contains(3));
        assert!(!s.contains(4));
        assert!(!s.insert(3), "insert of an existing id is a no-op");
        assert!(s.insert(2));
        assert_eq!(s.ids(), &[1, 2, 3, 5]);
        assert!(s.remove(2));
        assert!(!s.remove(2));
    }

    #[test]
    fn toggle_reports_the_new_state() {
        let mut s = Selection::new();
        assert!(s.toggle(9));
        assert!(!s.toggle(9));
        assert!(s.is_empty());
    }

    #[test]
    fn bounds_union_skips_missing_ids() {
        let mut s = Selection::new();
        s.set(&[1, 2, 3]);
        let bounds = s
            .bounds(|id| match id {
                1 => Some(rect(0.0, 0.0, 10.0, 10.0)),
                2 => Some(rect(20.0, 5.0, 10.0, 10.0)),
                _ => None,
            })
            .unwrap();
        assert_eq!(bounds, rect(0.0, 0.0, 30.0, 15.0));
    }

    #[test]
    fn marquee_selects_only_intersecting_entities() {
        let mut tree = LooseQuadtree::new(4096.0);
        tree.insert(1, rect(0.0, 0.0, 10.0, 10.0), 0);
        tree.insert(2, rect(100.0, 100.0, 10.0, 10.0), 0);
        tree.insert(3, rect(500.0, 500.0, 10.0, 10.0), 0);
        let mut selection = Selection::new();
        let mut marquee = Marquee::new(Vec2::new(-5.0, -5.0));
        marquee.drag_to(Vec2::new(50.0, 50.0));
        marquee.apply(&mut tree, SelectionMode::Replace, &mut selection);
        assert_eq!(selection.ids(), &[1]);
    }

    #[test]
    fn marquee_modes_compose_with_the_existing_selection() {
        let mut tree = LooseQuadtree::new(4096.0);
        tree.insert(1, rect(0.0, 0.0, 10.0, 10.0), 0);
        tree.insert(2, rect(100.0, 100.0, 10.0, 10.0), 0);
        let mut selection = Selection::new();
        selection.set(&[2]);
        let mut marquee = Marquee::new(Vec2::new(-5.0, -5.0));
        marquee.drag_to(Vec2::new(50.0, 50.0));

        marquee.apply(&mut tree, SelectionMode::Add, &mut selection);
        assert_eq!(selection.ids(), &[1, 2]);

        marquee.apply(&mut tree, SelectionMode::Subtract, &mut selection);
        assert_eq!(selection.ids(), &[2]);

        marquee.apply(&mut tree, SelectionMode::Toggle, &mut selection);
        assert_eq!(selection.ids(), &[1, 2]);
    }

    #[test]
    fn marquee_rect_is_normalised_when_dragged_up_left() {
        let mut m = Marquee::new(Vec2::new(100.0, 100.0));
        m.drag_to(Vec2::new(20.0, 30.0));
        assert_eq!(m.rect(), rect(20.0, 30.0, 80.0, 70.0));
        assert!(m.is_active(2.0));
        assert!(!m.is_active(200.0));
    }

    #[test]
    fn handles_sit_on_the_bounds_and_rotate_grip_above() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        assert_eq!(
            Handle::NorthWest.position(bounds, 20.0),
            Vec2::new(0.0, 0.0)
        );
        assert_eq!(
            Handle::SouthEast.position(bounds, 20.0),
            Vec2::new(100.0, 50.0)
        );
        assert_eq!(Handle::East.position(bounds, 20.0), Vec2::new(100.0, 25.0));
        assert_eq!(
            Handle::Rotate.position(bounds, 20.0),
            Vec2::new(50.0, -20.0)
        );
    }

    #[test]
    fn handle_hit_test_picks_the_nearest_grip() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        // 距离东侧控制柄 2 px，距离东南角控制柄 30 px。
        let hit = Handle::hit(bounds, Vec2::new(98.0, 26.0), 6.0, 20.0);
        assert_eq!(hit, Some(Handle::East));
        assert_eq!(Handle::hit(bounds, Vec2::new(50.0, 25.0), 6.0, 20.0), None);
    }

    #[test]
    fn east_drag_only_changes_width() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        let mut drag = HandleDrag::new(Handle::East, bounds, Vec2::new(100.0, 25.0), 20.0);
        let t = drag.update(Vec2::new(150.0, 25.0), Modifiers::NONE);
        let out = t.apply_rect(bounds);
        assert!((out.size.x - 150.0).abs() < 1e-3, "{out:?}");
        assert!((out.size.y - 50.0).abs() < 1e-3, "{out:?}");
        assert!((out.origin.x - 0.0).abs() < 1e-3, "{out:?}");
    }

    #[test]
    fn west_drag_pins_the_right_edge() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        let mut drag = HandleDrag::new(Handle::West, bounds, Vec2::new(0.0, 25.0), 20.0);
        let t = drag.update(Vec2::new(-50.0, 25.0), Modifiers::NONE);
        let out = t.apply_rect(bounds);
        assert!((out.max().x - 100.0).abs() < 1e-3, "{out:?}");
        assert!((out.min().x + 50.0).abs() < 1e-3, "{out:?}");
    }

    #[test]
    fn north_west_drag_pins_the_opposite_corner() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        let mut drag = HandleDrag::new(Handle::NorthWest, bounds, Vec2::new(0.0, 0.0), 20.0);
        let t = drag.update(Vec2::new(-20.0, -10.0), Modifiers::NONE);
        let out = t.apply_rect(bounds);
        assert!((out.max().x - 100.0).abs() < 1e-3, "{out:?}");
        assert!((out.max().y - 50.0).abs() < 1e-3, "{out:?}");
        assert!((out.size.x - 120.0).abs() < 1e-3, "{out:?}");
        assert!((out.size.y - 60.0).abs() < 1e-3, "{out:?}");
    }

    #[test]
    fn shift_keeps_the_aspect_ratio() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        let mut drag = HandleDrag::new(Handle::SouthEast, bounds, Vec2::new(100.0, 50.0), 20.0);
        let t = drag.update(
            Vec2::new(200.0, 50.0),
            Modifiers {
                shift: true,
                ..Modifiers::NONE
            },
        );
        assert!((t.scale.x - t.scale.y).abs() < 1e-6, "{:?}", t.scale);
        assert!((t.scale.x - 2.0).abs() < 1e-3, "{:?}", t.scale);
    }

    #[test]
    fn alt_resizes_about_the_centre() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        let mut drag = HandleDrag::new(Handle::East, bounds, Vec2::new(100.0, 25.0), 20.0);
        let t = drag.update(
            Vec2::new(125.0, 25.0),
            Modifiers {
                alt: true,
                ..Modifiers::NONE
            },
        );
        let out = t.apply_rect(bounds);
        assert!((out.center().x - 50.0).abs() < 1e-3, "{out:?}");
        assert!((out.size.x - 150.0).abs() < 1e-3, "{out:?}");
    }

    #[test]
    fn a_drag_never_inverts_the_rectangle() {
        let bounds = rect(0.0, 0.0, 100.0, 50.0);
        let mut drag = HandleDrag::new(Handle::East, bounds, Vec2::new(100.0, 25.0), 20.0);
        let t = drag.update(Vec2::new(-500.0, 25.0), Modifiers::NONE);
        let out = t.apply_rect(bounds);
        assert!(out.size.x >= 0.0, "{out:?}");
        assert!(out.size.y >= 0.0, "{out:?}");
    }

    #[test]
    fn rotation_wraps_and_snaps_with_shift() {
        let bounds = rect(0.0, 0.0, 100.0, 100.0);
        let centre = bounds.center();
        let mut drag = HandleDrag::new(
            Handle::Rotate,
            bounds,
            Vec2::new(centre.x, centre.y - 60.0),
            20.0,
        );
        // 顺时针旋转四分之一圈（90°）。
        let t = drag.update(Vec2::new(centre.x + 60.0, centre.y), Modifiers::NONE);
        assert!(
            (t.rotation - core::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "{:?}",
            t.rotation
        );
        let snapped = drag.update(
            Vec2::new(centre.x + 60.0, centre.y),
            Modifiers {
                shift: true,
                ..Modifiers::NONE
            },
        );
        assert!((snapped.rotation - core::f32::consts::FRAC_PI_2).abs() < 1e-3);
    }

    #[test]
    fn rotation_preserves_the_centre() {
        let bounds = rect(10.0, 10.0, 100.0, 40.0);
        let centre = bounds.center();
        let t = Transform2 {
            anchor: centre,
            scale: Vec2::ONE,
            translate: Vec2::ZERO,
            rotation: core::f32::consts::FRAC_PI_2,
        };
        let out = t.apply_rect(bounds);
        assert!((out.center().x - centre.x).abs() < 1e-3, "{out:?}");
        assert!((out.center().y - centre.y).abs() < 1e-3, "{out:?}");
        assert!((out.size.x - 40.0).abs() < 1e-3, "{out:?}");
        assert!((out.size.y - 100.0).abs() < 1e-3, "{out:?}");
    }

    #[test]
    fn identity_transform_is_cheap_and_exact() {
        let t = Transform2::identity();
        assert!(t.is_identity());
        let r = rect(3.0, 4.0, 5.0, 6.0);
        assert_eq!(t.apply_rect(r), r);
    }

    #[test]
    fn selection_mode_follows_modifiers() {
        assert_eq!(SelectionMode::from(Modifiers::NONE), SelectionMode::Replace);
        assert_eq!(
            SelectionMode::from(Modifiers {
                shift: true,
                ..Modifiers::NONE
            }),
            SelectionMode::Add
        );
        assert_eq!(
            SelectionMode::from(Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            }),
            SelectionMode::Subtract
        );
        assert_eq!(
            SelectionMode::from(Modifiers {
                shift: true,
                ctrl: true,
                ..Modifiers::NONE
            }),
            SelectionMode::Toggle
        );
    }
}

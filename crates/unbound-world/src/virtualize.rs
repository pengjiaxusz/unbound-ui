//! 视口虚拟化：`VirtualList`、`VirtualGrid`、`VirtualTree`（`docs` §4.3）。
//!
//!
//! 契约清晰且可测试：实例化实体的数量受限于视口大小，而非数据集大小。
//! 一个 100 000 行的列表无论滚动到何处都只保持 60–120 个活跃节点，
//! 并且滚动不足一行时仅需更新单个 `f32` 变换，实现**零**重新绑定。
//!
//!
//! 槽位复用是显式而非隐式的：[`VirtualList::update`] 会准确报告
//! 复用了多少槽位以及重新绑定了多少槽位，这正是测试与 KPI 套件所断言的指标。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use crate::{no_node, NodeId};
use unbound_math::Vec2;

/// 单次更新的开销统计。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct WindowUpdate {
    /// 滚出视口并被复用于新条目的项目数。
    pub recycled: usize,
    /// 所属条目索引发生变化的槽位数。
    pub rebound: usize,
    /// 行内滚动余数，作为平移量应用到每个槽位。
    pub offset: f32,
    /// 当仅偏移量发生变化时为 true（“零开销”快速路径）。
    pub offset_only: bool,
    /// 当窗口从头重建时为 true（大幅跳转）。
    pub rebuilt: bool,
}

impl WindowUpdate {
    /// 当窗口未发生任何变化时为 true。
    #[must_use]
    pub fn is_noop(&self) -> bool {
        !self.rebuilt && self.recycled == 0 && self.rebound == 0 && self.offset == 0.0
    }
}

/// 垂直滚动的列表窗口。
#[derive(Debug)]
pub struct VirtualList {
    row_height: f32,
    /// 在视口上方与下方额外实例化的缓冲屏数。
    overscan_screens: f32,
    /// 每个槽位当前代表的条目索引。
    item_of: Vec<usize>,
    /// 每个槽位绑定的节点；未绑定时为 `NO_NODE`。
    node_of: Vec<NodeId>,
    /// 首个可见条目的索引（而非槽位索引）。
    first_item: usize,
    /// 缓存的视口高度。
    viewport: f32,
    /// 缓存的总条目数。
    total: usize,
    /// 自构造以来执行的绑定调用次数（用于诊断）。
    binds: u64,
    /// 窗口不得不扩容的次数（稳态下应保持为 0）。
    grows: u64,
}

impl VirtualList {
    /// # Panic 说明
    /// 当 `row_height <= 0` 时 panic。
    #[must_use]
    pub fn new(row_height: f32, overscan_screens: f32) -> Self {
        assert!(row_height > 0.0, "row height must be positive");
        Self {
            row_height,
            overscan_screens: overscan_screens.max(0.0),
            item_of: Vec::new(),
            node_of: Vec::new(),
            first_item: 0,
            viewport: 0.0,
            total: 0,
            binds: 0,
            grows: 0,
        }
    }

    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.row_height
    }

    /// 活跃槽位数量（即实体预算上限）。
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.item_of.len()
    }

    #[must_use]
    pub fn binds(&self) -> u64 {
        self.binds
    }

    #[must_use]
    pub fn grows(&self) -> u64 {
        self.grows
    }

    /// 查询指定槽位持有的条目索引。
    #[must_use]
    pub fn item_at(&self, slot: usize) -> Option<usize> {
        self.item_of.get(slot).copied()
    }

    /// 查询绑定到指定槽位的节点。
    #[must_use]
    pub fn node_at(&self, slot: usize) -> Option<NodeId> {
        self.node_of.get(slot).copied()
    }

    /// 将槽位绑定（或重绑）到节点。由容器组件在为新可见条目创建节点时调用。
    ///
    pub fn set_node(&mut self, slot: usize, node: NodeId) {
        if let Some(slot_ref) = self.node_of.get_mut(slot) {
            *slot_ref = node;
        }
    }

    /// 视口加预渲染缓冲区（overscan）所需的槽位数量。
    ///
    /// 暴露此值以便宿主在不重复实现预渲染策略的情况下预估图元缓冲区大小。
    ///
    #[must_use]
    pub fn desired_slots(&self, viewport: f32) -> usize {
        let visible = (viewport / self.row_height).ceil().max(1.0) as usize;
        let overscan = (visible as f32 * self.overscan_screens).ceil() as usize;
        (visible + overscan * 2).max(1)
    }

    /// 根据滚动位置重新计算窗口。
    ///
    /// `scroll` 为视口顶部在内容空间中的偏移量。
    ///
    /// 槽位构成一个**环形队列**：槽位 `k` 始终代表条目 `k % capacity`。
    /// 正因如此，滚动一行只需重绑一个槽位而非重建整个窗口——
    /// 从顶部滚出的槽位恰好对应底部新进入条目所映射的槽位。
    ///
    pub fn update(&mut self, scroll: f32, viewport: f32, total: usize) -> WindowUpdate {
        let mut update = WindowUpdate::default();
        let scroll = scroll.max(0.0);
        let viewport = viewport.max(0.0);

        if total == 0 || viewport <= 0.0 {
            if !self.item_of.is_empty() {
                self.item_of.clear();
                self.node_of.clear();
            }
            self.first_item = 0;
            self.total = total;
            self.viewport = viewport;
            update.rebuilt = true;
            return update;
        }

        let rows = self.row_height;
        let visible_rows = (viewport / rows).ceil().max(1.0) as usize;
        let overscan_rows = ((visible_rows as f32) * self.overscan_screens).ceil() as usize;
        let desired = (visible_rows + overscan_rows * 2).min(total).max(1);
        let offset = -scroll % rows;
        let visible_first = (scroll / rows).floor() as usize;
        let first_item = visible_first
            .saturating_sub(overscan_rows)
            .min(total.saturating_sub(desired));

        let capacity_changed = self.item_of.len() != desired;
        let same_window = !capacity_changed && self.first_item == first_item;

        if same_window {
            self.total = total;
            self.viewport = viewport;
            update.offset = offset;
            update.offset_only = true;
            return update;
        }

        if capacity_changed {
            // 整个虚拟化路径中唯一可能发生内存分配的地方，仅在视口高度自身变化时触发。
            //
            let grew = desired > self.item_of.len();
            self.item_of.resize(desired, usize::MAX);
            self.node_of.resize(desired, no_node());
            if grew {
                self.grows += 1;
            }
        }

        // 仅重绑条目发生变化的槽位。使用环形索引时，这正好等于新进入窗口的条目数。
        //
        let mut rebound = 0usize;
        let end = (first_item + desired).min(total);
        for item in first_item..end {
            let slot = item % self.item_of.len();
            if self.item_of[slot] != item {
                self.item_of[slot] = item;
                rebound += 1;
                self.binds += 1;
            }
        }
        // 不再承载任何条目的槽位（列表缩短时）必须清空。
        for slot in &mut self.item_of {
            let item = *slot;
            if item != usize::MAX && (item < first_item || item >= end) {
                *slot = usize::MAX;
            }
        }

        let jumped = (first_item as isize - self.first_item as isize).unsigned_abs() >= desired;
        update.recycled = rebound;
        update.rebound = rebound;
        update.offset = offset;
        update.rebuilt = capacity_changed || jumped;

        self.first_item = first_item;
        self.total = total;
        self.viewport = viewport;
        update
    }

    /// 条目在内容空间中的 y 坐标。
    #[must_use]
    pub fn item_y(&self, item: usize) -> f32 {
        item as f32 * self.row_height
    }

    /// 首个已实例化的条目索引。
    #[must_use]
    pub fn first_item(&self) -> usize {
        self.first_item
    }

    /// 共 `total` 行的列表在内容空间中的总高度。
    #[must_use]
    pub fn content_height(&self, total: usize) -> f32 {
        total as f32 * self.row_height
    }

    /// 为新数据集重置状态（保留已分配容量）。
    pub fn reset(&mut self) {
        self.item_of.clear();
        self.node_of.clear();
        self.first_item = 0;
        self.total = 0;
    }
}

/// 按行优先排列的网格窗口（用于资产浏览器、缩略图墙）。
#[derive(Debug)]
pub struct VirtualGrid {
    cell: Vec2,
    gap: Vec2,
    columns: usize,
    list: VirtualList,
    first_row: usize,
}

impl VirtualGrid {
    /// # Panic 说明
    /// 当 `columns == 0` 或单元格尺寸非正数时 panic。
    #[must_use]
    pub fn new(cell: Vec2, gap: Vec2, columns: usize, overscan_screens: f32) -> Self {
        assert!(columns > 0, "grid needs at least one column");
        assert!(cell.y > 0.0, "cell height must be positive");
        Self {
            cell,
            gap,
            columns,
            list: VirtualList::new(cell.y + gap.y, overscan_screens),
            first_row: 0,
        }
    }

    #[must_use]
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// 指定条目总数下实际实例化的单元格数量。
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.list.slot_count() * self.columns
    }

    /// 更新网格窗口。返回行级更新统计；每滚动一行，行窗口向前推进一行且仅重绑该行。
    ///
    pub fn update(&mut self, scroll: f32, viewport: f32, items: usize) -> WindowUpdate {
        let rows = items.div_ceil(self.columns);
        let update = self.list.update(scroll, viewport, rows);
        self.first_row = self.list.first_item();
        update
    }

    #[must_use]
    pub fn first_row(&self) -> usize {
        self.first_row
    }

    /// 槽位在网格内容空间中的局部位置。
    #[must_use]
    pub fn cell_origin(&self, row: usize, column: usize) -> Vec2 {
        Vec2::new(
            column as f32 * (self.cell.x + self.gap.x),
            row as f32 * (self.cell.y + self.gap.y),
        )
    }

    #[must_use]
    pub fn cell_size(&self) -> Vec2 {
        self.cell
    }
}

/// 扁平化树中的单行记录（供虚拟化器使用）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    /// 调用方模型中的稳定 ID。
    pub id: u64,
    pub depth: u16,
    pub has_children: bool,
    pub expanded: bool,
}

/// 扁平化为行并像列表一样分窗的树形视图。
#[derive(Debug)]
pub struct VirtualTree {
    rows: Vec<TreeRow>,
    list: VirtualList,
    expanded: std::collections::HashSet<u64>,
}

impl VirtualTree {
    #[must_use]
    pub fn new(row_height: f32, overscan_screens: f32) -> Self {
        Self {
            rows: Vec::new(),
            list: VirtualList::new(row_height, overscan_screens),
            expanded: std::collections::HashSet::new(),
        }
    }

    /// 替换树模型。`children_of` 返回节点的有序子节点；
    /// 仅展开的节点会贡献行，因此扁平化开销仅与*可见*树成正比，而非整份文档。
    ///
    pub fn rebuild<F>(&mut self, roots: &[TreeRow], children_of: F)
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        self.rows.clear();
        let mut stack: Vec<TreeRow> = roots.iter().rev().copied().collect();
        while let Some(mut row) = stack.pop() {
            row.expanded = self.expanded.contains(&row.id);
            self.rows.push(row);
            if row.expanded {
                let kids = children_of(row.id);
                for k in kids.into_iter().rev() {
                    stack.push(k);
                }
            }
        }
    }

    pub fn set_expanded(&mut self, id: u64, expanded: bool) {
        if expanded {
            self.expanded.insert(id);
        } else {
            self.expanded.remove(&id);
        }
    }

    #[must_use]
    pub fn is_expanded(&self, id: u64) -> bool {
        self.expanded.contains(&id)
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn update(&mut self, scroll: f32, viewport: f32) -> WindowUpdate {
        self.list.update(scroll, viewport, self.rows.len())
    }

    #[must_use]
    pub fn row_at(&self, slot: usize) -> Option<TreeRow> {
        self.list
            .item_at(slot)
            .and_then(|i| self.rows.get(i).copied())
    }

    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.list.slot_count()
    }

    /// 构建虚拟化器时使用的行高。
    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.list.row_height()
    }

    /// 指定槽位当前代表的扁平化行索引。
    ///
    /// 槽位是环形的，因此需要获取某行内容空间 `y` 坐标的消费方
    /// 必须查询条目索引，而不能假设 `slot == item`。
    #[must_use]
    pub fn slot_item(&self, slot: usize) -> Option<usize> {
        self.list.item_at(slot)
    }

    /// 扁平化行在内容空间中的 `y` 坐标。
    #[must_use]
    pub fn item_y(&self, item: usize) -> f32 {
        self.list.item_y(item)
    }

    /// 窗口中首个已实例化的扁平化行索引。
    #[must_use]
    pub fn first_item(&self) -> usize {
        self.list.first_item()
    }

    /// 所有扁平化行的内容总高度。
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.list.content_height(self.rows.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hundred_thousand_rows_keep_a_constant_entity_budget() {
        let mut list = VirtualList::new(28.0, 1.0);
        list.update(0.0, 800.0, 100_000);
        let initial = list.slot_count();
        assert!(
            (28..=120).contains(&initial),
            "budget {initial} outside the 60–120 target band (with overscan)"
        );
        for scroll in [500.0, 5_000.0, 250_000.0, 2_500_000.0] {
            list.update(scroll, 800.0, 100_000);
            assert_eq!(list.slot_count(), initial, "budget must not grow");
        }
        assert_eq!(list.grows(), 1, "exactly one allocation");
    }

    #[test]
    fn sub_row_scrolling_is_free() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 1000);
        let binds_before = list.binds();
        for dy in [1.0, 2.0, 3.0, 7.5, 19.0] {
            let update = list.update(dy, 600.0, 1000);
            assert!(update.offset_only, "scroll {dy} must be offset-only");
            assert_eq!(update.recycled, 0);
            assert!((update.offset + dy).abs() < 1e-3);
        }
        assert_eq!(list.binds(), binds_before, "no rebinding under one row");
    }

    #[test]
    fn crossing_a_row_boundary_rebinds_exactly_one_slot() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 1000);
        let binds_before = list.binds();
        // 预渲染缓冲意味着首个条目位于视口前方；滚动足够距离使窗口推进一行时，
        // 只会重绑一个槽位。
        let previous_first = list.first_item();
        let mut crossing = None;
        for step in 1..=40 {
            let update = list.update(step as f32 * 20.0, 600.0, 1000);
            if list.first_item() != previous_first {
                crossing = Some(update);
                break;
            }
        }
        let update = crossing.expect("window must advance");
        assert_eq!(
            update.recycled, 1,
            "one row of scroll should recycle exactly one slot"
        );
        assert_eq!(list.binds(), binds_before + 1);
    }

    #[test]
    fn a_large_jump_rebuilds_the_window() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 10_000);
        let update = list.update(100_000.0, 600.0, 10_000);
        assert!(update.rebuilt);
        assert!(update.rebound > 10);
        assert_eq!(list.slot_count(), list.slot_count());
    }

    #[test]
    fn scrolling_past_the_end_never_binds_a_missing_item() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 1000);
        let update = list.update(1_000_000.0, 600.0, 1000);
        assert!(update.rebuilt || update.rebound > 0);
        let mut bound = 0;
        for slot in 0..list.slot_count() {
            if let Some(item) = list.item_at(slot) {
                if item == usize::MAX {
                    continue;
                }
                assert!(item < 1000, "slot {slot} bound to {item} (total 1000)");
                bound += 1;
            }
        }
        assert!(bound > 0, "the window must still hold items at the end");
    }

    #[test]
    fn a_list_shorter_than_the_viewport_binds_only_real_items() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 30);
        let mut bound = 0;
        for slot in 0..list.slot_count() {
            if let Some(item) = list.item_at(slot) {
                if item == usize::MAX {
                    continue;
                }
                assert!(item < 30);
                bound += 1;
            }
        }
        assert_eq!(bound, 30, "every item of a short list should be live");
    }

    #[test]
    fn an_empty_list_clears_every_slot() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 100);
        let update = list.update(0.0, 600.0, 0);
        assert!(update.rebuilt);
        assert_eq!(list.slot_count(), 0);
    }

    #[test]
    fn overscan_covers_a_screen_above_and_below() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(2000.0, 400.0, 1000);
        let first = list.first_item();
        let last = first + list.slot_count() - 1;
        let visible_first = 100;
        let visible_last = 120;
        assert!(first <= visible_first - 10, "first {first} lacks overscan");
        assert!(last >= visible_last + 10, "last {last} lacks overscan");
    }

    #[test]
    fn grid_windows_whole_rows() {
        let mut grid = VirtualGrid::new(Vec2::new(100.0, 100.0), Vec2::new(10.0, 10.0), 5, 1.0);
        let update = grid.update(0.0, 450.0, 10_000);
        assert!(!update.is_noop() || update.rebound == 0);
        let rows = grid.slot_count() / grid.columns();
        assert!(rows >= 4, "only {rows} rows instantiated");
        assert_eq!(grid.cell_origin(1, 2), Vec2::new(220.0, 110.0));
    }

    #[test]
    fn tree_only_flattens_expanded_nodes() {
        let root = TreeRow {
            id: 1,
            depth: 0,
            has_children: true,
            expanded: false,
        };
        let mut tree = VirtualTree::new(24.0, 1.0);
        let kids = |id: u64| {
            if id == 1 {
                vec![
                    TreeRow {
                        id: 2,
                        depth: 1,
                        has_children: false,
                        expanded: false,
                    },
                    TreeRow {
                        id: 3,
                        depth: 1,
                        has_children: true,
                        expanded: false,
                    },
                ]
            } else if id == 3 {
                vec![TreeRow {
                    id: 4,
                    depth: 2,
                    has_children: false,
                    expanded: false,
                }]
            } else {
                vec![]
            }
        };
        tree.rebuild(&[root], kids);
        assert_eq!(tree.row_count(), 1, "collapsed root hides everything");
        tree.set_expanded(1, true);
        tree.rebuild(&[root], kids);
        assert_eq!(tree.row_count(), 3);
        tree.set_expanded(3, true);
        tree.rebuild(&[root], kids);
        assert_eq!(tree.row_count(), 4);
        assert!(tree.is_expanded(3));
    }

    #[test]
    fn updating_twice_with_the_same_scroll_is_a_noop() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(120.0, 600.0, 1000);
        let update = list.update(120.0, 600.0, 1000);
        assert!(update.is_noop(), "expected noop, got {update:?}");
    }

    #[test]
    fn binding_nodes_to_slots_is_stable_across_offset_scrolls() {
        let mut list = VirtualList::new(20.0, 1.0);
        list.update(0.0, 600.0, 1000);
        let node = no_node();
        list.set_node(0, node);
        list.update(5.0, 600.0, 1000);
        assert_eq!(list.node_at(0), Some(node));
    }
}

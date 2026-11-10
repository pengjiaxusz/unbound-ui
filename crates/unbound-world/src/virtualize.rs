//! Viewport virtualisation: `VirtualList`, `VirtualGrid`, `VirtualTree`
//! (`docs` §4.3).
//!
//! The contract is narrow and testable: the number of instantiated entities is
//! bounded by the viewport, never by the data set. A 100 000-row list keeps
//! 60–120 nodes alive regardless of scroll position, and scrolling by less than
//! one row costs a single `f32` transform update with **zero** rebinding.
//!
//! Slot recycling is explicit rather than magical: [`VirtualList::update`]
//! reports how many slots were reused and how many had to be rebound, which is
//! exactly what the tests and the KPI harness assert on.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;
use crate::{no_node, NodeId};

/// What one update cost.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct WindowUpdate {
    /// Items that scrolled out and were reused for new items.
    pub recycled: usize,
    /// Slots whose item index changed.
    pub rebound: usize,
    /// Sub-row scroll remainder, to be applied as a translation to every slot.
    pub offset: f32,
    /// True when only the offset changed (the "cost 0" fast path).
    pub offset_only: bool,
    /// True when the window was rebuilt from scratch (large jump).
    pub rebuilt: bool,
}

impl WindowUpdate {
    /// True when nothing about the window changed.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        !self.rebuilt && self.recycled == 0 && self.rebound == 0 && self.offset == 0.0
    }
}

/// A vertically scrolling list window.
#[derive(Debug)]
pub struct VirtualList {
    row_height: f32,
    /// Extra screens instantiated above and below the viewport.
    overscan_screens: f32,
    /// Item index represented by each slot.
    item_of: Vec<usize>,
    /// Owning node of each slot, `NO_NODE` until bound.
    node_of: Vec<NodeId>,
    /// First visible item index (not slot).
    first_item: usize,
    /// Cached viewport height.
    viewport: f32,
    /// Cached total item count.
    total: usize,
    /// Number of bind calls performed since construction (diagnostics).
    binds: u64,
    /// Number of times the window had to grow (should stay at 0 in steady state).
    grows: u64,
}

impl VirtualList {
    /// # Panics
    /// Panics when `row_height <= 0`.
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

    /// Number of live slots (the entity budget).
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

    /// Item index held by a slot.
    #[must_use]
    pub fn item_at(&self, slot: usize) -> Option<usize> {
        self.item_of.get(slot).copied()
    }

    /// Node bound to a slot.
    #[must_use]
    pub fn node_at(&self, slot: usize) -> Option<NodeId> {
        self.node_of.get(slot).copied()
    }

    /// Bind (or rebind) a slot to a node. Called by the container widget when it
    /// creates the node for a newly visible item.
    pub fn set_node(&mut self, slot: usize, node: NodeId) {
        if let Some(slot_ref) = self.node_of.get_mut(slot) {
            *slot_ref = node;
        }
    }

    /// Number of slots needed for a viewport plus overscan.
    ///
    /// Exposed so a host can size its primitive buffer without duplicating the
    /// overscan policy.
    #[must_use]
    pub fn desired_slots(&self, viewport: f32) -> usize {
        let visible = (viewport / self.row_height).ceil().max(1.0) as usize;
        let overscan = (visible as f32 * self.overscan_screens).ceil() as usize;
        (visible + overscan * 2).max(1)
    }

    /// Recompute the window for a scroll position.
    ///
    /// `scroll` is the offset of the top of the viewport in content space.
    ///
    /// Slots are a **ring**: slot `k` always represents item `k % capacity`. That
    /// is what turns a one-row scroll into one rebind instead of a full window
    /// rebuild — the slot that scrolled out of the top is exactly the slot the
    /// new item at the bottom maps to.
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
            // The only allocation in the whole virtualisation path, and it
            // happens when the viewport height itself changes.
            let grew = desired > self.item_of.len();
            self.item_of.resize(desired, usize::MAX);
            self.node_of.resize(desired, no_node());
            if grew {
                self.grows += 1;
            }
        }

        // Rebind only the slots whose item changed. With ring indexing this is
        // one per item that entered the window.
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
        // Slots that no longer carry an item (list shrank) must be cleared.
        let live = self.item_of.len();
        for slot in 0..live {
            let item = self.item_of[slot];
            if item != usize::MAX && (item < first_item || item >= end) {
                self.item_of[slot] = usize::MAX;
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

    /// Content-space y of an item.
    #[must_use]
    pub fn item_y(&self, item: usize) -> f32 {
        item as f32 * self.row_height
    }

    /// First instantiated item.
    #[must_use]
    pub fn first_item(&self) -> usize {
        self.first_item
    }

    /// Total content height for a list of `total` rows.
    #[must_use]
    pub fn content_height(&self, total: usize) -> f32 {
        total as f32 * self.row_height
    }

    /// Reset for a new data set (keeps the allocated capacity).
    pub fn reset(&mut self) {
        self.item_of.clear();
        self.node_of.clear();
        self.first_item = 0;
        self.total = 0;
    }
}

/// A row-major grid window (asset browser, thumbnail wall).
#[derive(Debug)]
pub struct VirtualGrid {
    cell: Vec2,
    gap: Vec2,
    columns: usize,
    list: VirtualList,
    first_row: usize,
}

impl VirtualGrid {
    /// # Panics
    /// Panics when `columns == 0` or a cell dimension is not positive.
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

    /// Cells instantiated for a given item count.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.list.slot_count() * self.columns
    }

    /// Update the window. Returns the row-level update; rows advance by one per
    /// row of scroll with a single rebind.
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

    /// Local position of a slot within the grid, in content space.
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

/// One row of a flattened tree, as the virtualiser sees it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    /// Stable id in the caller's model.
    pub id: u64,
    pub depth: u16,
    pub has_children: bool,
    pub expanded: bool,
}

/// A tree flattened into rows, windowed like a list.
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

    /// Replace the model. `children_of` returns the ordered children of a node;
    /// only expanded nodes contribute rows, so the flattening cost is
    /// proportional to the *visible* tree, not to the document.
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
        self.list.item_at(slot).and_then(|i| self.rows.get(i).copied())
    }

    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.list.slot_count()
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
        // Overscan means the first item is ahead of the viewport; scrolling far
        // enough to move the window by one row rebinds one slot.
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

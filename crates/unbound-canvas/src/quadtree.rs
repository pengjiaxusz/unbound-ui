//! Loose quadtree over flat SoA arrays with intrusive linked lists
//! (`docs` §5.1).
//!
//! Why not a fixed-size spatial hash: a canvas mixes 8 px icons with 40 000 px
//! group frames, and any fixed cell size either degenerates into one bucket per
//! cell or one giant bucket. Why not a pointer quadtree: `Vec<Box<Node>>` costs a
//! cache miss per child and allocates on every split.
//!
//! What this is:
//!
//! * a **linear** quadtree — a node is addressed by `(depth, morton_index)`, so
//!   there are no child pointers at all, only `u32` indices into level arrays,
//! * **loose**: every cell is inflated by a factor of 2, so an element whose
//!   extent is at most the standard cell size always fits entirely inside one
//!   node without being split. Depth is therefore `floor(log2(world / extent))`
//!   — an O(1) decision equivalent to the doc's `leading_zeros`, no descent,
//! * **intrusive**: elements hang off a per-node singly-linked list stored in a
//!   parallel `Vec<u32>`, so moving an element is a handful of integer writes
//!   and never allocates.
//!
//! Two extra structures keep queries cheap:
//!
//! * `level_occupied` lists only the nodes that actually hold elements, so a
//!   query costs `O(occupied nodes)` rather than `O(4^MAX_DEPTH)` — with depth 8
//!   the dense scan would visit 87 381 nodes per query,
//! * `level_slot` maps a node index back to its position in that list so
//!   inserting the first element and removing the last are both O(1).
//!
//! The tree covers a fixed world square (`world_size`) centred on the origin.
//! Anything that cannot be found by a query over the root's loose region is
//! counted in [`LooseQuadtree::overflowed`] instead of being silently dropped.

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::FxHashMap;
use unbound_math::{Rect, Vec2};

/// Maximum subdivision depth.
///
/// A *linear* quadtree stores one head per node, so depth `d` costs `4^d` words.
/// Depth 8 is the largest value whose dense level arrays still fit in a few
/// hundred kilobytes (≈ 87 000 `u32`), and it already gives 16 px cells for a
/// 4096 px world and 3.9 kpx cells for a 10⁶ px world. Going deeper would trade
/// megabytes of sparse table for a pruning win the exact test already provides.
pub const MAX_DEPTH: u8 = 8;

/// Sentinel for "no element" / "no slot".
pub const NO_ELEMENT: u32 = u32::MAX;

/// One element in the tree.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// Caller's entity index (a canvas item id).
    pub entity: u32,
    /// Bounds used for both placement and exact re-testing at query time.
    pub aabb: Rect,
    /// Layer key used to order query results before the exact test.
    pub layer: u16,
    /// Next element in the same node's list.
    pub next: u32,
    /// Previous element in the same node's list.
    ///
    /// The list is doubly-linked so that removing an element from the middle is
    /// O(1). A singly-linked list would need a walk to find the predecessor,
    /// and the naive version (overwriting the head with the removed element's
    /// successor) silently drops every element that followed it.
    pub prev: u32,
    /// Node this element currently hangs off.
    pub node: u32,
}

/// One non-empty node: its index and its loose bounds.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Occupied {
    index: u32,
    bounds: Rect,
}

/// Query statistics, used by the KPI tests.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// Nodes whose loose bounds were tested against the query.
    pub nodes_visited: u32,
    /// Elements offered as candidates (all of them failed the broad phase).
    pub elements_tested: u32,
    /// Elements that survived the exact test.
    pub elements_returned: u32,
}

impl QueryStats {
    /// Fraction of tested elements that survived the exact test.
    #[must_use]
    pub fn hit_rate(&self) -> f32 {
        if self.elements_tested == 0 {
            0.0
        } else {
            self.elements_returned as f32 / self.elements_tested as f32
        }
    }
}

/// A loose linear quadtree.
pub struct LooseQuadtree {
    /// Edge length of the world square covered by the tree, centred on origin.
    world_size: f32,
    /// `level_heads[d]` has `4^d` entries: the head element of each node.
    level_heads: Vec<Vec<u32>>,
    /// Dense list of non-empty nodes per level, each with its loose bounds
    /// precomputed at insertion time.
    ///
    /// Caching the bounds turns a query into a linear scan of contiguous
    /// rectangles instead of replaying the Morton digits for every node: on a
    /// 1 M entity canvas that is the difference between 4 500 random 8-step walks
    /// and one 200 KiB sequential sweep.
    level_occupied: Vec<Vec<Occupied>>,
    /// `level_slot[d][node]` is the position of `node` in `level_occupied[d]`,
    /// or [`NO_ELEMENT`] when the node is empty.
    level_slot: Vec<Vec<u32>>,
    /// Element storage, indexed by element handle.
    elements: Vec<Element>,
    free_elements: Vec<u32>,
    /// Handle for each entity, for O(1) removal.
    entity_of: FxHashMap<u32, u32>,
    stats: QueryStats,
    overflowed: u64,
}

impl core::fmt::Debug for LooseQuadtree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LooseQuadtree")
            .field("world_size", &self.world_size)
            .field("elements", &self.entity_of.len())
            .field("levels", &self.level_heads.len())
            .field("occupied_nodes", &self.occupied_nodes())
            .field("overflowed", &self.overflowed)
            .finish_non_exhaustive()
    }
}

impl LooseQuadtree {
    /// Create a tree covering `world_size × world_size` centred on the origin.
    ///
    /// # Panics
    /// Panics when `world_size <= 0`.
    #[must_use]
    pub fn new(world_size: f32) -> Self {
        assert!(world_size > 0.0, "world size must be positive");
        let mut tree = Self {
            world_size,
            level_heads: Vec::with_capacity(MAX_DEPTH as usize + 1),
            level_occupied: Vec::with_capacity(MAX_DEPTH as usize + 1),
            level_slot: Vec::with_capacity(MAX_DEPTH as usize + 1),
            elements: Vec::new(),
            free_elements: Vec::new(),
            entity_of: FxHashMap::default(),
            stats: QueryStats::default(),
            overflowed: 0,
        };
        // Every level is allocated up front: a query walks all of them, and a
        // lazily grown level would change the loop bound mid-frame.
        tree.ensure_level(MAX_DEPTH);
        tree
    }

    /// Number of live entities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entity_of.len()
    }

    /// True when the tree holds no entities.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entity_of.is_empty()
    }

    /// Number of elements that could not be placed (their bounds fall outside
    /// the root's loose region). Surfaced instead of silently dropping them.
    #[must_use]
    pub fn overflowed(&self) -> u64 {
        self.overflowed
    }

    /// Statistics of the last [`Self::query`] / [`Self::query_exact`] call.
    #[must_use]
    pub fn stats(&self) -> QueryStats {
        self.stats
    }

    /// Number of non-empty nodes across all levels.
    #[must_use]
    pub fn occupied_nodes(&self) -> usize {
        self.level_occupied.iter().map(Vec::len).sum()
    }

    /// Bytes of level storage, reported by the KPI harness.
    ///
    /// The three levels have different element types: the head and the slot
    /// tables hold a `u32` each, but an occupied node is a `u32` index plus a
    /// `Rect`. Counting them all as four bytes under-reported the memory by
    /// five times.
    #[must_use]
    pub fn level_bytes(&self) -> usize {
        let heads: usize = self.level_heads.iter().map(|l| l.len() * 4).sum();
        let occ: usize = self
            .level_occupied
            .iter()
            .map(|l| l.len() * core::mem::size_of::<Occupied>())
            .sum();
        let slots: usize = self.level_slot.iter().map(|l| l.len() * 4).sum();
        heads + occ + slots
    }

    fn ensure_level(&mut self, depth: u8) {
        while self.level_heads.len() <= depth as usize {
            let d = self.level_heads.len() as u8;
            let count = 1usize << (2 * d);
            self.level_heads.push(vec![NO_ELEMENT; count]);
            self.level_occupied.push(Vec::new());
            self.level_slot.push(vec![NO_ELEMENT; count]);
        }
    }

    /// Half-extent of a *standard* (non-loose) cell at `depth`.
    fn cell_half(&self, depth: u8) -> f32 {
        self.world_size * 0.5 / (1u32 << depth) as f32
    }

    /// Deepest level whose cell still swallows the element whole.
    ///
    /// The loose cell at depth `d` has width `2 · world / 2^d`, i.e. it extends
    /// half a standard cell beyond the standard cell on every side. An element
    /// centred anywhere inside the standard cell therefore fits when
    /// `extent ≤ world / 2^d`, which is `d ≤ log2(world / extent)`: the doc's
    /// `leading_zeros` computation. The loop below is the exact float form of
    /// that expression and runs at most `MAX_DEPTH + 1` times.
    fn depth_for(&self, aabb: &Rect) -> Option<u8> {
        if !aabb.is_finite() || !aabb.origin.is_finite() {
            return None;
        }
        let extent = aabb.size.x.max(aabb.size.y);
        if extent >= self.world_size {
            // Larger than the world: only the root's loose region can hold it,
            // and `insert` re-checks containment before accepting.
            return Some(0);
        }
        for d in (0..=MAX_DEPTH).rev() {
            if self.cell_half(d) * 2.0 >= extent {
                return Some(d);
            }
        }
        Some(0)
    }

    /// Morton-style index of the cell at `depth` containing `point`.
    ///
    /// Quadratic (not Morton-interleaved) order: bits are appended two at a
    /// time, quadrant `qy * 2 + qx` last-shifted. That keeps the index a plain
    /// base-4 number, which is what `node_bounds` replays.
    fn node_index(&self, point: Vec2, depth: u8) -> u32 {
        let mut index = 0u32;
        let mut half = self.world_size * 0.5;
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        for _ in 0..depth {
            let qx = u32::from(point.x >= cx);
            let qy = u32::from(point.y >= cy);
            index = index * 4 + qy * 2 + qx;
            half *= 0.5;
            if qx == 1 {
                cx += half;
            } else {
                cx -= half;
            }
            if qy == 1 {
                cy += half;
            } else {
                cy -= half;
            }
        }
        index
    }

    /// Bounds of the *loose* cell at `(depth, index)`.
    ///
    /// The loose region is twice the standard cell in each direction (the doc's
    /// `k = 0.5`), which is exactly the margin that lets an element sit across a
    /// cell boundary while still being found by a query on the neighbouring cell.
    fn node_bounds(&self, index: u32, depth: u8) -> Rect {
        let half = self.cell_half(depth);
        // Replay the quadrant digits most-significant first, mirroring
        // `node_index`, which appends the current quadrant at the low end.
        let mut h = self.world_size * 0.5;
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        for d in 0..depth {
            let shift = 2 * (depth - 1 - d);
            let bit = (index >> shift) & 0b11;
            let qx = bit & 1;
            let qy = (bit >> 1) & 1;
            h *= 0.5;
            cx += if qx == 1 { h } else { -h };
            cy += if qy == 1 { h } else { -h };
        }
        Rect::new(cx - half * 2.0, cy - half * 2.0, half * 4.0, half * 4.0)
    }

    /// Insert or move an entity.
    ///
    /// Inserting an entity that is already present moves it, which keeps the
    /// updater free of "does it exist?" branches.
    pub fn insert(&mut self, entity: u32, aabb: Rect, layer: u16) {
        self.remove(entity);
        let Some(depth) = self.depth_for(&aabb) else {
            self.overflowed += 1;
            return;
        };
        let index = self.node_index(aabb.center(), depth);
        let node_bounds = self.node_bounds(index, depth);
        if !node_bounds.touches(&aabb) {
            // Outside the root's loose region (or higher levels' coverage):
            // unreachable by any query, so report rather than misplace.
            self.overflowed += 1;
            return;
        }
        let handle = match self.free_elements.pop() {
            Some(h) => {
                let e = &mut self.elements[h as usize];
                e.entity = entity;
                e.aabb = aabb;
                e.layer = layer;
                h
            }
            None => {
                self.elements.push(Element {
                    entity,
                    aabb,
                    layer,
                    next: NO_ELEMENT,
                    prev: NO_ELEMENT,
                    node: 0,
                });
                (self.elements.len() - 1) as u32
            }
        };
        self.attach(depth, index, handle);
        self.entity_of.insert(entity, handle);
    }

    /// Push `handle` onto the node's list, registering the node if it was empty.
    fn attach(&mut self, depth: u8, index: u32, handle: u32) {
        let head_slot = &mut self.level_heads[depth as usize][index as usize];
        let head = *head_slot;
        {
            let e = &mut self.elements[handle as usize];
            e.node = Self::pack_node(depth, index);
            e.next = head;
            e.prev = NO_ELEMENT;
        }
        if head == NO_ELEMENT {
            // Compute the bounds before taking the mutable borrow of the list.
            let bounds = self.node_bounds(index, depth);
            let slot = self.level_occupied[depth as usize].len() as u32;
            self.level_occupied[depth as usize].push(Occupied { index, bounds });
            self.level_slot[depth as usize][index as usize] = slot;
        } else {
            // The old head now has a predecessor.
            self.elements[head as usize].prev = handle;
        }
        self.level_heads[depth as usize][index as usize] = handle;
    }

    /// Unlink `handle` from its node, unregistering the node when it empties.
    fn detach(&mut self, handle: u32) {
        let (depth, index) = Self::unpack_node(self.elements[handle as usize].node);
        let next = self.elements[handle as usize].next;
        let prev = self.elements[handle as usize].prev;

        if prev == NO_ELEMENT {
            // The element was the head: the list now starts at its successor.
            self.level_heads[depth as usize][index as usize] = next;
        } else {
            // The element was in the middle: close the gap in the chain so the
            // elements behind it stay reachable.
            self.elements[prev as usize].next = next;
        }
        if next != NO_ELEMENT {
            self.elements[next as usize].prev = prev;
        }

        // The node is empty when the removed element was its only member.
        if prev == NO_ELEMENT && next == NO_ELEMENT {
            let level = depth as usize;
            let slot = self.level_slot[level][index as usize];
            debug_assert!(slot != NO_ELEMENT, "occupied list out of sync");
            let last = self.level_occupied[level].pop().expect("occupied node");
            if last.index != index {
                self.level_occupied[level][slot as usize] = last;
                self.level_slot[level][last.index as usize] = slot;
            }
            self.level_slot[level][index as usize] = NO_ELEMENT;
        }
    }

    fn pack_node(depth: u8, index: u32) -> u32 {
        // 8 bits of depth + 24 bits of index: enough for depth 12 with a dense
        // index, and depths beyond that pack into the same 24-bit space because
        // 4^12 == 2^24.
        ((depth as u32) << 24) | (index & 0x00ff_ffff)
    }

    fn unpack_node(packed: u32) -> (u8, u32) {
        ((packed >> 24) as u8, packed & 0x00ff_ffff)
    }

    /// Remove an entity. Returns `false` when it was not present.
    pub fn remove(&mut self, entity: u32) -> bool {
        let Some(handle) = self.entity_of.remove(&entity) else {
            return false;
        };
        self.detach(handle);
        {
            let e = &mut self.elements[handle as usize];
            e.next = NO_ELEMENT;
            e.prev = NO_ELEMENT;
            e.aabb = Rect::ZERO;
        }
        self.free_elements.push(handle);
        true
    }

    /// Move an entity, preserving its layer.
    pub fn update(&mut self, entity: u32, aabb: Rect) {
        let layer = self.layer_of(entity);
        self.insert(entity, aabb, layer);
    }

    /// Layer key recorded for `entity`, or `0` when absent.
    #[must_use]
    pub fn layer_of(&self, entity: u32) -> u16 {
        self.entity_of
            .get(&entity)
            .map(|h| self.elements[*h as usize].layer)
            .unwrap_or(0)
    }

    /// Bounds recorded for `entity`.
    #[must_use]
    pub fn bounds_of(&self, entity: u32) -> Option<Rect> {
        self.entity_of
            .get(&entity)
            .map(|h| self.elements[*h as usize].aabb)
    }

    /// Collect every entity whose node's loose region intersects `query`.
    ///
    /// The returned list is *candidate* geometry: it is conservative and must be
    /// re-tested with an exact intersection, which is what
    /// [`Self::query_exact`] does.
    pub fn query(&mut self, query: Rect, out: &mut Vec<u32>) {
        out.clear();
        self.stats = QueryStats::default();
        for depth in 0..self.level_heads.len() as u8 {
            let occupied = self.level_occupied[depth as usize].len();
            for slot in 0..occupied {
                let node = self.level_occupied[depth as usize][slot];
                if !node.bounds.intersects(&query) {
                    continue;
                }
                self.stats.nodes_visited += 1;
                let mut current = self.level_heads[depth as usize][node.index as usize];
                while current != NO_ELEMENT {
                    self.stats.elements_tested += 1;
                    out.push(self.elements[current as usize].entity);
                    current = self.elements[current as usize].next;
                }
            }
        }
    }

    /// Query with the exact AABB test applied, ordered by `(layer, entity)`.
    ///
    /// The sort is what makes picking deterministic: results never depend on
    /// insertion order or on which node happened to be visited first.
    pub fn query_exact(&mut self, query: Rect, out: &mut Vec<u32>) {
        let mut candidates = Vec::new();
        self.query(query, &mut candidates);
        out.clear();
        for entity in candidates {
            let Some(bounds) = self.bounds_of(entity) else {
                continue;
            };
            // The exact test is inclusive: a marquee whose edge lands exactly
            // on an entity's edge still selects it, matching the way
            // `Rect::contains_point` counts a point on the edge as inside.
            if bounds.touches(&query) {
                out.push(entity);
            }
        }
        out.sort_unstable_by_key(|e| (self.layer_of(*e), *e));
        out.dedup();
        self.stats.elements_returned = out.len() as u32;
    }

    /// True when any entity (other than `ignore`) intersects `query`.
    pub fn any_intersecting(&mut self, query: Rect, ignore: Option<u32>) -> bool {
        let mut hits = Vec::new();
        self.query_exact(query, &mut hits);
        hits.iter().any(|e| Some(*e) != ignore)
    }

    /// Clear the tree but keep the allocated levels.
    pub fn clear(&mut self) {
        for heads in &mut self.level_heads {
            heads.fill(NO_ELEMENT);
        }
        for occupied in &mut self.level_occupied {
            occupied.clear();
        }
        for slots in &mut self.level_slot {
            slots.fill(NO_ELEMENT);
        }
        self.elements.clear();
        self.free_elements.clear();
        self.entity_of.clear();
        self.overflowed = 0;
        self.stats = QueryStats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_single_element_is_found_by_its_own_bounds() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(7, rect(100.0, 100.0, 50.0, 50.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(90.0, 90.0, 100.0, 100.0), &mut out);
        assert_eq!(out, vec![7]);
    }

    #[test]
    fn a_far_element_is_not_returned() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(-3000.0, -3000.0, 10.0, 10.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(90.0, 90.0, 40.0, 40.0), &mut out);
        assert_eq!(out, vec![1]);
    }

    #[test]
    fn removal_is_exact_and_idempotent() {
        let mut t = LooseQuadtree::new(4096.0);
        t.insert(5, rect(0.0, 0.0, 10.0, 10.0), 0);
        assert!(t.remove(5));
        assert!(!t.remove(5), "second removal must be a no-op");
        assert!(t.is_empty());
        let mut out = Vec::new();
        t.query_exact(rect(-10.0, -10.0, 50.0, 50.0), &mut out);
        assert!(out.is_empty());
        assert_eq!(t.occupied_nodes(), 0, "emptied node must be unregistered");
    }

    #[test]
    fn moving_an_element_updates_its_placement() {
        let mut t = LooseQuadtree::new(4096.0);
        t.insert(3, rect(0.0, 0.0, 10.0, 10.0), 0);
        t.update(3, rect(1000.0, 1000.0, 10.0, 10.0));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 20.0, 20.0), &mut out);
        assert!(out.is_empty(), "old position must be empty");
        t.query_exact(rect(990.0, 990.0, 40.0, 40.0), &mut out);
        assert_eq!(out, vec![3]);
    }

    #[test]
    fn elements_of_many_scales_coexist() {
        let mut t = LooseQuadtree::new(1_000_000.0);
        // A tiny icon next to a huge group frame.
        t.insert(1, rect(10.0, 10.0, 4.0, 4.0), 0);
        t.insert(2, rect(-5000.0, -5000.0, 100_000.0, 100_000.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 100.0, 100.0), &mut out);
        assert_eq!(out, vec![1, 2], "both must be found");
    }

    #[test]
    fn every_element_is_returned_exactly_once() {
        let mut t = LooseQuadtree::new(16384.0);
        for i in 0..500u32 {
            let x = (i % 25) as f32 * 100.0 - 1200.0;
            let y = (i / 25) as f32 * 100.0 - 1200.0;
            t.insert(i, rect(x, y, 40.0, 40.0), 0);
        }
        let mut out = Vec::new();
        t.query_exact(rect(-5000.0, -5000.0, 10_000.0, 10_000.0), &mut out);
        out.sort_unstable();
        out.dedup();
        assert_eq!(out.len(), 500);
    }

    #[test]
    fn query_prunes_nodes_far_away() {
        let mut t = LooseQuadtree::new(65536.0);
        for i in 0..2000u32 {
            let x = ((i * 37) % 4000) as f32;
            let y = ((i * 71) % 4000) as f32;
            t.insert(i, rect(x, y, 8.0, 8.0), 0);
        }
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 200.0, 200.0), &mut out);
        let stats = t.stats();
        assert!(
            stats.elements_tested < 500,
            "culling failed: tested {} of 2000",
            stats.elements_tested
        );
        assert!(
            stats.nodes_visited < 200,
            "pruning failed: visited {} nodes",
            stats.nodes_visited
        );
        assert!(out.len() < 2000);
    }

    #[test]
    fn elements_outside_the_world_are_counted_not_lost() {
        let mut t = LooseQuadtree::new(1000.0);
        t.insert(1, rect(5000.0, 5000.0, 10.0, 10.0), 0);
        assert_eq!(t.overflowed(), 1);
        assert!(t.is_empty());
    }

    #[test]
    fn non_finite_input_is_reported_not_inserted() {
        let mut t = LooseQuadtree::new(1000.0);
        t.insert(1, rect(f32::NAN, 0.0, 10.0, 10.0), 0);
        t.insert(2, rect(0.0, 0.0, f32::INFINITY, 10.0), 0);
        assert_eq!(t.overflowed(), 2);
        assert!(t.is_empty());
    }

    #[test]
    fn handles_are_recycled_after_removal() {
        let mut t = LooseQuadtree::new(4096.0);
        for i in 0..100u32 {
            t.insert(i, rect(i as f32, 0.0, 4.0, 4.0), 0);
        }
        let capacity = t.elements.capacity();
        for i in 0..100u32 {
            t.remove(i);
        }
        for i in 0..100u32 {
            t.insert(i, rect(i as f32, 0.0, 4.0, 4.0), 0);
        }
        assert_eq!(t.elements.capacity(), capacity, "storage must be reused");
    }

    #[test]
    fn node_bounds_round_trip_for_every_depth() {
        let t = LooseQuadtree::new(1024.0);
        for depth in 0..=6u8 {
            let count = 1usize << (2 * depth);
            for index in [0usize, count / 2, count - 1] {
                let bounds = t.node_bounds(index as u32, depth);
                let centre = bounds.center();
                // The recorded centre must map back to the same node index.
                assert_eq!(
                    t.node_index(centre, depth) as usize,
                    index,
                    "depth {depth} index {index} -> {bounds:?}"
                );
            }
        }
    }

    #[test]
    fn loose_cells_overlap_their_neighbours() {
        let t = LooseQuadtree::new(1024.0);
        let depth = 2u8;
        let a = t.node_bounds(0, depth);
        let b = t.node_bounds(1, depth);
        // Neighbours must overlap by exactly half a standard cell on each side;
        // that overlap is the whole point of `k = 0.5`.
        let overlap = a.intersect(&b);
        assert!(overlap.size.x > 0.0 && overlap.size.y > 0.0);
    }

    #[test]
    fn placement_uses_the_deepest_fitting_level() {
        let t = LooseQuadtree::new(65536.0);
        // A 8 px icon in a 65 536 px world belongs at the deepest level.
        assert_eq!(t.depth_for(&rect(0.0, 0.0, 8.0, 8.0)), Some(MAX_DEPTH));
        // A 100 000 px frame cannot fit any level below the root.
        assert_eq!(t.depth_for(&rect(0.0, 0.0, 100_000.0, 100_000.0)), Some(0));
    }

    #[test]
    fn a_hundred_thousand_inserts_stay_fast_and_correct() {
        let mut t = LooseQuadtree::new(1_000_000.0);
        for i in 0..100_000u32 {
            // Widening arithmetic: the multipliers would overflow `u32`.
            let x = ((u64::from(i) * 7919) % 100_000) as f32 - 50_000.0;
            let y = ((u64::from(i) * 104_729) % 100_000) as f32 - 50_000.0;
            t.insert(i, rect(x, y, 16.0, 16.0), 0);
        }
        assert_eq!(t.len(), 100_000);
        let mut out = Vec::new();
        t.query_exact(rect(-500.0, -500.0, 1000.0, 1000.0), &mut out);
        // Only a small fraction of the 100k elements can be in a 1000x1000 box.
        assert!(out.len() < 2000, "returned {}", out.len());
        assert!(!out.is_empty());
        assert!(
            t.stats().elements_returned < t.stats().elements_tested,
            "broad phase must reject something"
        );
    }

    #[test]
    fn clear_resets_everything() {
        let mut t = LooseQuadtree::new(2048.0);
        t.insert(1, rect(0.0, 0.0, 10.0, 10.0), 0);
        t.clear();
        assert!(t.is_empty());
        assert_eq!(t.overflowed(), 0);
        assert_eq!(t.occupied_nodes(), 0);
        t.insert(2, rect(0.0, 0.0, 10.0, 10.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(-10.0, -10.0, 40.0, 40.0), &mut out);
        assert_eq!(out, vec![2]);
    }

    #[test]
    fn layer_orders_results() {
        let mut t = LooseQuadtree::new(4096.0);
        t.insert(1, rect(0.0, 0.0, 10.0, 10.0), 5);
        t.insert(2, rect(5.0, 5.0, 10.0, 10.0), 1);
        t.insert(3, rect(2.0, 2.0, 10.0, 10.0), 3);
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 20.0, 20.0), &mut out);
        assert_eq!(out, vec![2, 3, 1]);
    }

    /// Removing an element must not take its neighbours with it.
    ///
    /// The node list is head-inserted, so the element being removed is usually
    /// *not* the head. The old `detach` overwrote the head with the removed
    /// element's successor, which silently dropped every element that had
    /// been inserted before it.
    #[test]
    fn removing_one_entity_keeps_the_others() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(110.0, 100.0, 10.0, 10.0), 0);
        t.insert(3, rect(120.0, 100.0, 10.0, 10.0), 0);
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out, vec![1, 2, 3]);

        // Remove the oldest element: it sits at the tail of the list.
        assert!(t.remove(1));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out, vec![2, 3], "entities 2 and 3 must survive");

        // Remove the head next.
        assert!(t.remove(3));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out, vec![2]);
    }

    /// Moving an entity (which removes and re-inserts it) must not disturb the
    /// entities that share its quadtree node.
    #[test]
    fn moving_an_entity_keeps_its_neighbours() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(110.0, 100.0, 10.0, 10.0), 0);
        t.insert(3, rect(120.0, 100.0, 10.0, 10.0), 0);
        // Move the oldest entity: `update` removes it and re-inserts it.
        t.update(1, rect(130.0, 100.0, 10.0, 10.0));
        let mut out = Vec::new();
        t.query_exact(rect(0.0, 0.0, 8192.0, 8192.0), &mut out);
        assert_eq!(out.len(), 3, "all three entities must survive the move");
        assert!(out.contains(&1) && out.contains(&2) && out.contains(&3));
    }

    /// Removing the last element of a node must unregister the node.
    #[test]
    fn emptying_a_node_unregisters_it() {
        let mut t = LooseQuadtree::new(8192.0);
        t.insert(1, rect(100.0, 100.0, 10.0, 10.0), 0);
        t.insert(2, rect(110.0, 100.0, 10.0, 10.0), 0);
        let occupied = t.occupied_nodes();
        assert!(occupied > 0);
        t.remove(2);
        t.remove(1);
        assert_eq!(t.occupied_nodes(), 0, "the node must be gone");
        assert!(t.is_empty());
    }
}

//! The DOD UI world: flat entities, intrusive sibling links, dirty triage.
//!
//! `docs` §4.1 forbids pointer trees and `Rc<RefCell<..>>` graphs. Everything
//! here is a flat array indexed by a generational [`NodeId`]:
//!
//! ```text
//! nodes:        [ N0 | N1 | N2 | N3 | ... ]   SlotMap<u32, NodeData>
//! first_child:  [ 1  | 2  | -  | -  | ... ]   intrusive, no Vec<Vec<T>>
//! next_sibling: [ -  | 3  | -  | -  | ... ]
//! ```
//!
//! Consequences that matter:
//!
//! * adding/removing a child is four integer writes, never an allocation,
//! * walking children is a pointer chase through a contiguous array,
//! * a removed node's slot is reused with a bumped generation, so stale handles
//!   are detected instead of silently addressing another node.
//!
//! Dirty state is split into the three levels the architecture defines:
//! paint-only (opacity, colour, transform), layout (size/position inside a
//! relayout boundary) and topology (tree shape or viewport). Only the last one
//! forces a full re-solve.

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::{FxHashMap, FxHashSet};
use slotmap::{new_key_type, Key, SlotMap};
use unbound_abi::{bitflags_like, NodeFlags, NodeTag, RawAction, RawNodeDesc};
use unbound_math::{Rect, Vec2};

pub mod virtualize;

pub use virtualize::{TreeRow, VirtualGrid, VirtualList, VirtualTree, WindowUpdate};

new_key_type! {
    /// Stable, generational handle to a node.
    pub struct NodeId;
}

impl NodeId {
    /// True for the sentinel "no node" handle.
    #[inline]
    #[must_use]
    pub fn is_none(self) -> bool {
        Key::is_null(&self)
    }
}

/// Sentinel for "no node". A function rather than a `const` because
/// `Key::null()` is not a `const fn` in `slotmap` 1.x.
#[inline]
#[must_use]
pub fn no_node() -> NodeId {
    NodeId::null()
}

/// How expensive a change is to apply.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirtyLevel {
    /// Nothing changed.
    #[default]
    Clean = 0,
    /// Colour / opacity / transform only: no layout, no re-upload of geometry.
    Paint = 1,
    /// Geometry changed inside a `RelayoutBoundary`.
    Layout = 2,
    /// Tree shape or viewport size changed: everything is suspect.
    Topology = 3,
}

impl DirtyLevel {
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        if other as u8 > self as u8 {
            other
        } else {
            self
        }
    }
}

bitflags_like! {
    /// Per-node state bits, kept in one `u32` so a whole node's flags fit in a
    /// cache line together with its geometry.
    pub struct NodeState: u32 {
        /// Fixed width and height plus a non-visible overflow: children cannot
        /// change the parent's size, so layout dirt stops here.
        const RELAYOUT_BOUNDARY = 1 << 0;
        /// The node clips its children.
        const CLIPS_CHILDREN    = 1 << 1;
        /// The node is a scroll container.
        const SCROLLABLE        = 1 << 2;
        /// The node is currently animating (bypasses pixel snapping).
        const ANIMATING         = 1 << 3;
        /// The node is hidden together with its subtree.
        const HIDDEN            = 1 << 4;
        /// The node escaped its parent's clip via `Portal`.
        const PORTAL            = 1 << 5;
        /// Layout has been computed at least once.
        const LAID_OUT          = 1 << 6;
        /// The node is part of a virtualisation window.
        const VIRTUALIZED       = 1 << 7;
        /// The node has an action bound to it.
        const INTERACTIVE       = 1 << 8;
    }
}

/// Everything the engine needs to know about one node.
#[derive(Clone, Debug)]
pub struct NodeData {
    /// The descriptor the guest/declarative builder emitted.
    pub desc: RawNodeDesc,
    pub parent: NodeId,
    pub first_child: NodeId,
    pub last_child: NodeId,
    pub next_sibling: NodeId,
    pub prev_sibling: NodeId,
    /// Depth from the root, used by hit testing and accessibility.
    pub depth: u16,
    pub state: NodeState,
    /// Final layout rectangle in the parent's coordinate space (after offsets).
    pub rect: Rect,
    /// `rect` translated into absolute world space of the UI layer.
    pub world_rect: Rect,
    /// Folded hard clip in world space; `None` means "no clip".
    pub clip: Option<Rect>,
    /// Up to two rounded parent clips, innermost first.
    pub rounded_clips: [Option<RoundedClip>; 2],
    /// Pre-measured intrinsic size (text, images). Layout never calls into the
    /// text engine, which keeps shaping off the layout critical path.
    pub measure: Option<Vec2>,
    /// Monotonic generation of the state that produced this node's view. A child
    /// whose generation did not change is skipped by the view builder.
    pub state_generation: u64,
    /// Source location captured by `#[track_caller]`, for sanitizer reports.
    pub source: &'static core::panic::Location<'static>,
    /// Action bound to activation, if any.
    pub action: Option<RawAction>,
}

impl NodeData {
    #[must_use]
    pub fn new(desc: RawNodeDesc, source: &'static core::panic::Location<'static>) -> Self {
        let interactive = desc.has(NodeFlags::INTERACTIVE);
        let mut state = NodeState::EMPTY;
        if interactive {
            state |= NodeState::INTERACTIVE;
        }
        Self {
            desc,
            parent: no_node(),
            first_child: no_node(),
            last_child: no_node(),
            next_sibling: no_node(),
            prev_sibling: no_node(),
            depth: 0,
            state,
            rect: Rect::ZERO,
            world_rect: Rect::ZERO,
            clip: None,
            rounded_clips: [None, None],
            measure: None,
            state_generation: 0,
            source,
            action: None,
        }
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        !self.state.contains(NodeState::HIDDEN) && self.desc.opacity > 0.001
    }

    #[must_use]
    pub fn is_relayout_boundary(&self) -> bool {
        self.state.contains(NodeState::RELAYOUT_BOUNDARY)
    }

    #[must_use]
    pub fn tag(&self) -> NodeTag {
        self.desc.kind()
    }
}

/// A rounded clip inherited from an ancestor.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RoundedClip {
    pub rect: Rect,
    pub radii: [f32; 4],
}

/// A flattened child iterator that never allocates.
#[derive(Copy, Clone, Debug)]
pub struct ChildIter<'a> {
    world: &'a UiWorld,
    next: NodeId,
}

impl<'a> Iterator for ChildIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next.is_null() {
            return None;
        }
        let current = self.next;
        self.next = self.world.nodes[current].next_sibling;
        Some(current)
    }
}

/// Ancestor iterator, used by clip folding and hit testing.
#[derive(Copy, Clone, Debug)]
pub struct AncestorIter<'a> {
    world: &'a UiWorld,
    next: NodeId,
}

impl<'a> Iterator for AncestorIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next.is_null() {
            return None;
        }
        let current = self.next;
        let parent = self.world.nodes[current].parent;
        self.next = parent;
        Some(current)
    }
}

/// Statistics for one rebuild, consumed by the debug overlay and the tests.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub nodes: u32,
    pub paint_dirty: u32,
    pub layout_dirty: u32,
    pub topology_dirty: bool,
    pub relayout_boundaries: u32,
    pub nodes_visited: u32,
}

impl WorldStats {
    /// True when the last frame needed no work at all.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.paint_dirty == 0 && self.layout_dirty == 0 && !self.topology_dirty
    }
}

/// The UI world.
pub struct UiWorld {
    nodes: SlotMap<NodeId, NodeData>,
    texts: FxHashMap<NodeId, String>,
    paint_dirty: FxHashSet<NodeId>,
    layout_dirty: FxHashSet<NodeId>,
    topology_dirty: bool,
    root: NodeId,
    /// Scratch buffer for boundary collection, reused every frame.
    boundary_scratch: Vec<NodeId>,
    /// Nodes visited during the last dirty propagation (diagnostics).
    visited: u32,
}

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for UiWorld {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UiWorld")
            .field("nodes", &self.nodes.len())
            .field("paint_dirty", &self.paint_dirty.len())
            .field("layout_dirty", &self.layout_dirty.len())
            .field("topology_dirty", &self.topology_dirty)
            .finish_non_exhaustive()
    }
}

impl UiWorld {
    /// Create a world containing a single root node.
    #[must_use]
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let mut root_data = NodeData::new(
            RawNodeDesc::container()
                .size(0.0, 0.0)
                .flags(NodeFlags::CLIP),
            core::panic::Location::caller(),
        );
        root_data.state |= NodeState::RELAYOUT_BOUNDARY | NodeState::CLIPS_CHILDREN;
        let root = nodes.insert(root_data);
        Self {
            nodes,
            texts: FxHashMap::default(),
            paint_dirty: FxHashSet::default(),
            layout_dirty: FxHashSet::default(),
            topology_dirty: true,
            root,
            boundary_scratch: Vec::new(),
            visited: 0,
        }
    }

    #[must_use]
    pub fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&NodeData> {
        self.nodes.get(id)
    }

    #[must_use]
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut NodeData> {
        self.nodes.get_mut(id)
    }

    /// # Panics
    /// Panics when `id` is not a live node; use [`Self::contains`] to check.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id]
    }

    /// Insert a node as the last child of `parent`.
    ///
    /// `source` should be `core::panic::Location::caller()` — the view builder
    /// passes it automatically thanks to `#[track_caller]`, which is what lets a
    /// sanitizer report point at the exact construction site.
    pub fn insert(
        &mut self,
        parent: NodeId,
        desc: RawNodeDesc,
        source: &'static core::panic::Location<'static>,
    ) -> NodeId {
        let depth = if parent.is_null() {
            0
        } else {
            self.nodes.get(parent).map_or(0, |p| p.depth + 1)
        };
        let mut data = NodeData::new(desc, source);
        data.parent = parent;
        data.depth = depth;
        // Boundary heuristic (`docs` §4.2): a fixed-size, non-visible-overflow
        // container cannot be resized by its children, so layout dirt stops here.
        let fixed = matches!(desc.width.kind(), unbound_abi::DimUnit::Px)
            && matches!(desc.height.kind(), unbound_abi::DimUnit::Px);
        if fixed && desc.is_clipping() {
            data.state |= NodeState::RELAYOUT_BOUNDARY;
        }
        if desc.is_clipping() {
            data.state |= NodeState::CLIPS_CHILDREN;
        }
        if desc.has(NodeFlags::SCROLL_Y) || desc.has(NodeFlags::SCROLL_X) {
            data.state |= NodeState::SCROLLABLE;
        }
        if desc.kind() == NodeTag::Portal {
            data.state |= NodeState::PORTAL;
        }
        if desc.has(NodeFlags::HIDDEN) {
            data.state |= NodeState::HIDDEN;
        }
        if desc.has(NodeFlags::ANIMATED) {
            data.state |= NodeState::ANIMATING;
        }

        let id = self.nodes.insert(data);
        if !parent.is_null() {
            self.link_last(parent, id);
        } else {
            self.root = id;
        }
        if let Some(p) = self.nodes.get_mut(parent) {
            p.state.remove(NodeState::LAID_OUT);
        }
        self.topology_dirty = true;
        id
    }

    fn link_last(&mut self, parent: NodeId, child: NodeId) {
        let last = self.nodes[parent].last_child;
        {
            let p = &mut self.nodes[parent];
            p.last_child = child;
            if p.first_child.is_null() {
                p.first_child = child;
            }
        }
        {
            let c = &mut self.nodes[child];
            c.parent = parent;
            c.prev_sibling = last;
            c.next_sibling = no_node();
        }
        if !last.is_null() {
            self.nodes[last].next_sibling = child;
        }
    }

    fn unlink(&mut self, id: NodeId) {
        let (parent, prev, next) = {
            let n = &self.nodes[id];
            (n.parent, n.prev_sibling, n.next_sibling)
        };
        if !prev.is_null() {
            self.nodes[prev].next_sibling = next;
        } else if !parent.is_null() {
            self.nodes[parent].first_child = next;
        }
        if !next.is_null() {
            self.nodes[next].prev_sibling = prev;
        } else if !parent.is_null() {
            self.nodes[parent].last_child = prev;
        }
        {
            let n = &mut self.nodes[id];
            n.prev_sibling = no_node();
            n.next_sibling = no_node();
            n.parent = no_node();
        }
    }

    /// Remove a node and its whole subtree.
    pub fn remove(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) || id == self.root {
            return;
        }
        self.unlink(id);
        // Depth-first removal without recursion (a deep tree would otherwise
        // overflow the stack on a 1M-node document).
        let mut stack = vec![id];
        let mut doomed = Vec::new();
        while let Some(current) = stack.pop() {
            doomed.push(current);
            let mut child = self.nodes[current].first_child;
            while !child.is_null() {
                stack.push(child);
                child = self.nodes[child].next_sibling;
            }
        }
        for n in doomed {
            self.texts.remove(&n);
            self.paint_dirty.remove(&n);
            self.layout_dirty.remove(&n);
            self.nodes.remove(n);
        }
        self.topology_dirty = true;
    }

    /// Detach a node from its parent without destroying it (Portal adoption,
    /// virtualised recycling).
    pub fn detach(&mut self, id: NodeId) {
        if !self.nodes.contains_key(id) {
            return;
        }
        self.unlink(id);
        self.topology_dirty = true;
    }

    /// Move `id` under `new_parent` as the last child.
    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId) {
        if id == new_parent || !self.nodes.contains_key(id) || !self.nodes.contains_key(new_parent)
        {
            return;
        }
        // Refuse cycles: walking up from the new parent must not reach `id`.
        if self.ancestors(new_parent).any(|a| a == id) {
            return;
        }
        self.unlink(id);
        self.link_last(new_parent, id);
        self.refresh_depth(id);
        self.topology_dirty = true;
    }

    fn refresh_depth(&mut self, id: NodeId) {
        let depth = self.nodes[id].parent;
        let base = if depth.is_null() {
            0
        } else {
            self.nodes[depth].depth + 1
        };
        let mut stack = vec![(id, base)];
        while let Some((current, d)) = stack.pop() {
            self.nodes[current].depth = d;
            let mut child = self.nodes[current].first_child;
            while !child.is_null() {
                stack.push((child, d + 1));
                child = self.nodes[child].next_sibling;
            }
        }
    }

    /// Iterate a node's direct children without allocating.
    #[must_use]
    pub fn children(&self, id: NodeId) -> ChildIter<'_> {
        ChildIter {
            world: self,
            next: if self.nodes.contains_key(id) {
                self.nodes[id].first_child
            } else {
                no_node()
            },
        }
    }

    /// Iterate `id` and then its ancestors up to the root.
    #[must_use]
    pub fn ancestors(&self, id: NodeId) -> AncestorIter<'_> {
        AncestorIter {
            world: self,
            next: id,
        }
    }

    /// Depth-first pre-order traversal.
    ///
    /// Siblings come out in insertion order (left to right), and a whole
    /// subtree comes out before the next sibling.
    pub fn walk(&self, root: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack = vec![root];
        core::iter::from_fn(move || {
            let id = stack.pop()?;
            if !self.nodes.contains_key(id) {
                return Some(id);
            }
            let mut kids = Vec::new();
            let mut child = self.nodes[id].first_child;
            while !child.is_null() {
                kids.push(child);
                child = self.nodes[child].next_sibling;
            }
            // A stack pops last-in-first-out, so the children have to be
            // pushed in reverse to come out left-to-right.
            for kid in kids.into_iter().rev() {
                stack.push(kid);
            }
            Some(id)
        })
    }

    /// Number of children.
    #[must_use]
    pub fn child_count(&self, id: NodeId) -> usize {
        self.children(id).count()
    }

    /// Attach text to a node (replacing any previous text).
    pub fn set_text(&mut self, id: NodeId, text: impl Into<String>) {
        let text = text.into();
        if self.texts.get(&id) == Some(&text) {
            return;
        }
        self.texts.insert(id, text);
        self.mark_layout_dirty(id);
    }

    #[must_use]
    pub fn text(&self, id: NodeId) -> Option<&str> {
        self.texts.get(&id).map(String::as_str)
    }

    /// Bind an action to a node.
    pub fn bind_action(&mut self, id: NodeId, action: RawAction) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.action = Some(action);
            n.state |= NodeState::INTERACTIVE;
        }
        self.mark_paint_dirty(id);
    }

    /// Mark a node as paint-dirty and stop there: no ancestor cares about a
    /// colour change, so nothing propagates.
    pub fn mark_paint_dirty(&mut self, id: NodeId) {
        if self.nodes.contains_key(id) {
            self.paint_dirty.insert(id);
        }
    }

    /// Mark a node as layout-dirty and propagate **up to the nearest relayout
    /// boundary** (`docs` §4.2). This is the core of the incremental cost model:
    /// a text change inside a fixed-size card costs one card, not the tree.
    pub fn mark_layout_dirty(&mut self, id: NodeId) {
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        self.layout_dirty.insert(id);
        // A boundary is its own stopping point.
        if node.is_relayout_boundary() {
            return;
        }
        let mut current = node.parent;
        self.visited = 0;
        while !current.is_null() {
            self.visited += 1;
            let Some(parent) = self.nodes.get(current) else {
                break;
            };
            self.layout_dirty.insert(current);
            if parent.is_relayout_boundary() {
                break;
            }
            current = parent.parent;
        }
        // A child growing can change any ancestor's measured size, but only up
        // to the boundary; beyond it the size is fixed by definition.
    }

    /// Force a full re-layout (viewport resize, root style change).
    pub fn mark_topology_dirty(&mut self) {
        self.topology_dirty = true;
    }

    #[must_use]
    pub fn topology_dirty(&self) -> bool {
        self.topology_dirty
    }

    /// Consume the paint-dirty set (the paint pass owns it from here).
    pub fn take_paint_dirty(&mut self) -> FxHashSet<NodeId> {
        core::mem::take(&mut self.paint_dirty)
    }

    /// Dirty nodes that need a layout solve, sorted so that ancestors come first.
    #[must_use]
    pub fn layout_dirty_sorted(&mut self) -> Vec<NodeId> {
        let mut list: Vec<NodeId> = self.layout_dirty.drain().collect();
        list.sort_by_key(|id| self.nodes.get(*id).map_or(0, |n| n.depth));
        list
    }

    /// Non-draining view of the layout-dirty set, ordered by depth.
    #[must_use]
    pub fn layout_dirty_snapshot(&self) -> Vec<NodeId> {
        let mut list: Vec<NodeId> = self.layout_dirty.iter().copied().collect();
        list.sort_by_key(|id| self.nodes.get(*id).map_or(0, |n| n.depth));
        list
    }

    /// Collect the relayout boundaries touched by the pending layout dirt.
    ///
    /// These subtrees are independent, so the layout engine can solve them in
    /// parallel.
    #[must_use]
    pub fn dirty_boundaries(&mut self) -> Vec<NodeId> {
        self.boundary_scratch.clear();
        let dirty: Vec<NodeId> = self.layout_dirty.drain().collect();
        let mut boundaries = Vec::new();
        for id in dirty {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if node.is_relayout_boundary() {
                boundaries.push(id);
                continue;
            }
            // Find the closest boundary ancestor; if there is none the root
            // handles it.
            let mut current = node.parent;
            let mut found = no_node();
            while !current.is_null() {
                let Some(p) = self.nodes.get(current) else {
                    break;
                };
                if p.is_relayout_boundary() {
                    found = current;
                    break;
                }
                current = p.parent;
            }
            boundaries.push(if found.is_null() { self.root } else { found });
        }
        // Sort by depth so ancestors are solved before their descendants, then
        // remove the duplicates. `dedup` only drops *adjacent* equal elements,
        // so the sort has to group equal boundaries together: sorting by
        // depth alone leaves two boundaries of the same depth with a third
        // one in between, and the duplicate survived — the layout engine
        // would then solve the same boundary twice. The id breaks the tie so
        // the order is total and the duplicates end up next to each other.
        boundaries.sort_by_key(|id| (self.nodes.get(*id).map_or(0, |n| n.depth), *id));
        boundaries.dedup();
        boundaries
    }

    /// Count relayout boundaries (diagnostics + tests).
    #[must_use]
    pub fn relayout_boundary_count(&self) -> usize {
        self.nodes
            .values()
            .filter(|n| n.is_relayout_boundary())
            .count()
    }

    /// Statistics for the last propagation.
    #[must_use]
    pub fn stats(&self) -> WorldStats {
        WorldStats {
            nodes: self.nodes.len() as u32,
            paint_dirty: self.paint_dirty.len() as u32,
            layout_dirty: self.layout_dirty.len() as u32,
            topology_dirty: self.topology_dirty,
            relayout_boundaries: self.relayout_boundary_count() as u32,
            nodes_visited: self.visited,
        }
    }

    /// Clear every dirty flag (after a full frame).
    pub fn clear_dirty(&mut self) {
        self.paint_dirty.clear();
        self.layout_dirty.clear();
        self.topology_dirty = false;
        self.visited = 0;
    }

    /// Write the computed layout back and fold the clip chain.
    ///
    /// Clip folding happens **once, on the CPU**, so the shader only ever sees a
    /// single AABB plus two rounded slots (`docs` §3.2).
    pub fn apply_rects(&mut self, rects: &FxHashMap<NodeId, Rect>) {
        for (id, rect) in rects {
            if let Some(node) = self.nodes.get_mut(*id) {
                node.rect = *rect;
                node.state |= NodeState::LAID_OUT;
            }
        }
        let root = self.root;
        self.fold_clips(root);
    }

    /// Apply rectangles for a subset of nodes and refold from `start`.
    ///
    /// An incremental solve touches a handful of nodes; walking the whole tree to
    /// refresh `world_rect`/`clip` would make a 9 node re-layout cost as much as
    /// a full one.
    pub fn apply_rects_from(
        &mut self,
        rects: &FxHashMap<NodeId, Rect>,
        changed: &[NodeId],
        start: NodeId,
    ) {
        for id in changed {
            if let Some(rect) = rects.get(id) {
                if let Some(node) = self.nodes.get_mut(*id) {
                    node.rect = *rect;
                    node.state |= NodeState::LAID_OUT;
                }
            }
        }
        self.fold_clips(start);
    }

    fn fold_clips(&mut self, start: NodeId) {
        // An incremental refold starts in the *middle* of the tree, so the
        // clip and the rounded slots that `start` inherits from its ancestors
        // cannot be derived locally — take them from the parent, which
        // already stores exactly what it passes down to its children.
        //
        // Starting from an empty clip made the subtree forget its ancestors:
        // content that belongs inside a scroll container or a rounded panel
        // escaped it after an incremental re-layout, because the fold only
        // saw the boundary's own bounds.
        let (inherited_clip, inherited_rounded) = match self.nodes.get(start) {
            Some(node) if !node.parent.is_null() => match self.nodes.get(node.parent) {
                Some(parent) => (parent.clip, parent.rounded_clips),
                None => (None, [None; 2]),
            },
            _ => (None, [None; 2]),
        };
        let mut stack = vec![(start, inherited_clip, inherited_rounded)];
        while let Some((id, inherited_clip, inherited_rounded)) = stack.pop() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let world = if node.parent.is_null() {
                node.rect
            } else {
                let parent = self
                    .nodes
                    .get(node.parent)
                    .map_or(Rect::ZERO, |p| p.world_rect);
                node.rect.translate(parent.origin)
            };
            let mut clip = inherited_clip;
            let mut rounded = inherited_rounded;

            // A portal escapes every ancestor clip (`docs` §6.3): it paints in
            // the overlay layer, so inheriting a parent's scissor would be wrong.
            if node.state.contains(NodeState::PORTAL) {
                clip = None;
                rounded = [None; 2];
            }

            if node.state.contains(NodeState::CLIPS_CHILDREN)
                && !node.state.contains(NodeState::PORTAL)
            {
                let own = world;
                clip = Some(match clip {
                    Some(c) => c.intersect(&own),
                    None => own,
                });
                // Two rounded slots is what the shader can test; the innermost
                // two are the ones the user actually sees. Use `world`, not
                // `node.world_rect`, which is only written below.
                if node.desc.radius.iter().any(|r| *r > 0.0) {
                    rounded = [
                        Some(RoundedClip {
                            rect: world,
                            radii: node.desc.radius,
                        }),
                        rounded[0],
                    ];
                }
            }

            let node = self.nodes.get_mut(id).expect("checked above");
            node.world_rect = world;
            node.clip = clip;
            node.rounded_clips = rounded;

            let mut child = node.first_child;
            let mut kids = Vec::new();
            while !child.is_null() {
                kids.push(child);
                child = self.nodes[child].next_sibling;
            }
            // Reverse push so the pop order is left-to-right.
            for k in kids.into_iter().rev() {
                stack.push((k, clip, rounded));
            }
        }
    }

    /// Total nodes that would be visited by a naive full re-layout — the
    /// baseline the incremental path is compared against in tests.
    #[must_use]
    pub fn full_layout_cost(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_abi::{Dim, Edges};

    fn loc() -> &'static core::panic::Location<'static> {
        core::panic::Location::caller()
    }

    fn world() -> (UiWorld, NodeId) {
        let w = UiWorld::new();
        let root = w.root();
        (w, root)
    }

    #[test]
    fn children_are_linked_in_insertion_order() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let c = w.insert(root, RawNodeDesc::container(), loc());
        assert_eq!(w.children(root).collect::<Vec<_>>(), vec![a, b, c]);
        assert_eq!(w.node(a).next_sibling, b);
        assert_eq!(w.node(b).prev_sibling, a);
        assert_eq!(w.node(c).prev_sibling, b);
        assert_eq!(w.node(root).first_child, a);
        assert_eq!(w.node(root).last_child, c);
        assert_eq!(w.node(a).depth, 1);
        assert_eq!(w.node(c).depth, 1);
    }

    #[test]
    fn removing_a_middle_child_repairs_the_links() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let c = w.insert(root, RawNodeDesc::container(), loc());
        w.remove(b);
        assert_eq!(w.children(root).collect::<Vec<_>>(), vec![a, c]);
        assert_eq!(w.node(a).next_sibling, c);
        assert_eq!(w.node(c).prev_sibling, a);
        assert_eq!(w.len(), 3, "root + a + c");
        // The stale handle must be rejected, not silently reused.
        assert!(!w.contains(b));
    }

    #[test]
    fn removing_a_subtree_frees_every_descendant() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        let a2 = w.insert(a, RawNodeDesc::container(), loc());
        let a21 = w.insert(a2, RawNodeDesc::container(), loc());
        assert_eq!(w.len(), 5);
        w.remove(a);
        assert_eq!(w.len(), 1);
        for id in [a, a1, a2, a21] {
            assert!(!w.contains(id));
        }
    }

    #[test]
    fn generations_are_bumped_on_slot_reuse() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        w.remove(a);
        let b = w.insert(root, RawNodeDesc::container(), loc());
        assert_ne!(a, b, "a reused slot must get a new generation");
        assert!(!w.contains(a));
        assert!(w.contains(b));
    }

    #[test]
    fn reparenting_preserves_order_and_updates_depth() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        assert_eq!(w.node(a1).depth, 2);
        w.reparent(a1, b);
        assert_eq!(w.children(root).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(w.children(b).collect::<Vec<_>>(), vec![a1]);
        assert_eq!(w.node(a1).parent, b);
        assert_eq!(w.node(a1).depth, 2);
    }

    #[test]
    fn reparenting_refuses_to_create_a_cycle() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(a, RawNodeDesc::container(), loc());
        w.reparent(a, b);
        assert_eq!(w.node(a).parent, root, "cycle must be refused");
        assert_eq!(w.node(b).parent, a);
    }

    #[test]
    fn a_fixed_clipping_container_becomes_a_relayout_boundary() {
        let (mut w, root) = world();
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(320.0, 200.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        assert!(w.node(card).is_relayout_boundary());
        let free = w.insert(root, RawNodeDesc::container(), loc());
        assert!(!w.node(free).is_relayout_boundary());
    }

    #[test]
    fn layout_dirt_stops_at_the_nearest_boundary() {
        let (mut w, root) = world();
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(320.0, 200.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let inner = w.insert(card, RawNodeDesc::container(), loc());
        let label = w.insert(inner, RawNodeDesc::text(), loc());
        w.clear_dirty();

        w.mark_layout_dirty(label);
        let dirty = w.layout_dirty_snapshot();
        assert!(dirty.contains(&label));
        assert!(dirty.contains(&inner));
        assert!(dirty.contains(&card));
        assert!(!dirty.contains(&root), "root must not be dirtied");
        assert!(w.stats().nodes_visited <= 4);

        // `dirty_boundaries` drains the set, so it runs after the snapshot.
        let boundaries = w.dirty_boundaries();
        assert_eq!(boundaries, vec![card]);
        assert!(w.layout_dirty_snapshot().is_empty());
    }

    #[test]
    fn paint_dirt_never_propagates() {
        let (mut w, root) = world();
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let label = w.insert(card, RawNodeDesc::text(), loc());
        w.clear_dirty();
        w.mark_paint_dirty(label);
        let stats = w.stats();
        assert_eq!(stats.paint_dirty, 1);
        assert_eq!(stats.layout_dirty, 0);
        assert!(!stats.topology_dirty);
    }

    #[test]
    fn clips_fold_through_the_ancestor_chain() {
        let (mut w, root) = world();
        let outer = w.insert(
            root,
            RawNodeDesc::container()
                .size(400.0, 400.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let inner = w.insert(
            outer,
            RawNodeDesc::container()
                .size(200.0, 200.0)
                .radius(12.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(10.0, 10.0, 400.0, 400.0));
        rects.insert(inner, Rect::new(20.0, 20.0, 200.0, 200.0));
        rects.insert(leaf, Rect::new(5.0, 5.0, 50.0, 50.0));
        w.apply_rects(&rects);

        let leaf_node = w.node(leaf);
        // world = root.origin(0) + outer.origin(10) + inner.origin(20) + leaf(5)
        assert_eq!(leaf_node.world_rect, Rect::new(35.0, 35.0, 50.0, 50.0));
        // Clip is the intersection of both ancestors, in world space.
        let clip = leaf_node.clip.expect("clipped");
        assert_eq!(clip, Rect::new(30.0, 30.0, 200.0, 200.0));
        // The innermost rounded container is slot 0.
        let rounded = leaf_node.rounded_clips[0].expect("rounded clip");
        assert_eq!(rounded.radii, [12.0; 4]);
        assert_eq!(rounded.rect, Rect::new(30.0, 30.0, 200.0, 200.0));
    }

    #[test]
    fn portals_escape_ancestor_clips() {
        let (mut w, root) = world();
        let panel = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let portal = w.insert(panel, RawNodeDesc::container().tag(NodeTag::Portal), loc());
        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(panel, Rect::new(0.0, 0.0, 100.0, 100.0));
        rects.insert(portal, Rect::new(500.0, 500.0, 80.0, 80.0));
        w.apply_rects(&rects);
        assert!(
            w.node(portal).clip.is_none(),
            "a portal must not be clipped"
        );
    }

    #[test]
    fn text_is_stored_once_and_dedups() {
        let (mut w, root) = world();
        let label = w.insert(root, RawNodeDesc::text(), loc());
        w.set_text(label, "hello");
        w.clear_dirty();
        w.set_text(label, "hello");
        let stats = w.stats();
        // The second set must not dirty anything: compare-and-set on the string.
        assert_eq!(stats.layout_dirty, 0);
        assert_eq!(w.text(label), Some("hello"));
    }

    #[test]
    fn walk_visits_every_node_once_in_pre_order() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let visited: Vec<NodeId> = w.walk(root).collect();
        assert_eq!(visited.len(), 4);
        assert_eq!(visited[0], root);
        assert!(visited.contains(&a1));
        assert!(visited.contains(&b));
    }

    /// Siblings have to come out in insertion order.
    ///
    /// The traversal pushes the children onto a stack, and a stack pops
    /// last-in-first-out — so without reversing, every sibling group comes out
    /// **right-to-left**. The comment in the function promised left-to-right,
    /// and the two other tree walks in this codebase (`fold_clips` here,
    /// `preorder` in `unbound-layout`) do reverse. A caller that walks the
    /// tree to paint or to hit-test would see the children of every node in
    /// the wrong order.
    #[test]
    fn walk_yields_siblings_in_insertion_order() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let c = w.insert(root, RawNodeDesc::container(), loc());
        let visited: Vec<NodeId> = w.walk(root).collect();
        assert_eq!(
            visited,
            vec![root, a, b, c],
            "siblings must be visited left-to-right"
        );
    }

    /// The same order guarantee for a deeper tree: a whole subtree has to
    /// come out before the next sibling.
    #[test]
    fn walk_yields_a_whole_subtree_before_the_next_sibling() {
        let (mut w, root) = world();
        let a = w.insert(root, RawNodeDesc::container(), loc());
        let a1 = w.insert(a, RawNodeDesc::container(), loc());
        let a2 = w.insert(a, RawNodeDesc::container(), loc());
        let b = w.insert(root, RawNodeDesc::container(), loc());
        let visited: Vec<NodeId> = w.walk(root).collect();
        assert_eq!(visited, vec![root, a, a1, a2, b]);
    }

    /// An incremental solve must not drop the clip chain above the boundary.
    ///
    /// `apply_rects_from` refolds only the boundary's subtree, but the
    /// subtree's *effective* clip is the intersection of every clipping
    /// ancestor. Starting the fold with an empty inherited clip made the
    /// subtree forget its ancestors: content that belongs inside a scroll
    /// container or a rounded panel would escape it after an incremental
    /// re-layout.
    #[test]
    fn an_incremental_refold_keeps_the_ancestor_clip_chain() {
        let (mut w, root) = world();
        let outer = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        // The inner container is *wider* than the outer one, so the
        // difference between "clipped by the ancestor" and "clipped by
        // itself" is visible.
        let inner = w.insert(
            outer,
            RawNodeDesc::container()
                .size(400.0, 400.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(10.0, 10.0, 100.0, 100.0));
        rects.insert(inner, Rect::new(0.0, 0.0, 400.0, 400.0));
        rects.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects(&rects);

        // The full fold clips the leaf to the outer container.
        let full = w.node(leaf).clip.expect("clipped");
        assert_eq!(full, Rect::new(10.0, 10.0, 100.0, 100.0));

        // Now refold only the inner subtree, the way an incremental layout
        // solve does.
        let mut changed = FxHashMap::default();
        changed.insert(inner, Rect::new(0.0, 0.0, 400.0, 400.0));
        changed.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects_from(&changed, &[inner, leaf], inner);

        let incremental = w.node(leaf).clip.expect("still clipped");
        assert_eq!(
            incremental, full,
            "the incremental refold dropped the ancestor clip chain"
        );
    }

    /// The same guarantee for the rounded slots: an incremental refold must
    /// not forget the rounded ancestor above the boundary.
    #[test]
    fn an_incremental_refold_keeps_the_ancestor_rounded_clips() {
        let (mut w, root) = world();
        let outer = w.insert(
            root,
            RawNodeDesc::container()
                .size(400.0, 400.0)
                .radius(12.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let inner = w.insert(
            outer,
            RawNodeDesc::container()
                .size(200.0, 200.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let leaf = w.insert(inner, RawNodeDesc::container(), loc());

        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(outer, Rect::new(10.0, 10.0, 400.0, 400.0));
        rects.insert(inner, Rect::new(0.0, 0.0, 200.0, 200.0));
        rects.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects(&rects);
        assert_eq!(
            w.node(leaf).rounded_clips[0].expect("rounded").radii,
            [12.0; 4]
        );

        let mut changed = FxHashMap::default();
        changed.insert(inner, Rect::new(0.0, 0.0, 200.0, 200.0));
        changed.insert(leaf, Rect::new(0.0, 0.0, 50.0, 50.0));
        w.apply_rects_from(&changed, &[inner, leaf], inner);

        assert_eq!(
            w.node(leaf).rounded_clips[0].expect("rounded").radii,
            [12.0; 4],
            "the incremental refold dropped the ancestor rounded clip"
        );
    }

    /// The boundary list must not name the same boundary twice.
    ///
    /// The boundaries were sorted by depth and then `dedup`ed — but `dedup`
    /// only removes *adjacent* duplicates. Two boundaries at the same depth
    /// with a third one in between left the duplicate in place, and the
    /// layout engine then solved that boundary twice.
    #[test]
    fn dirty_boundaries_never_repeat_a_boundary() {
        let (mut w, root) = world();
        // Two boundaries at the same depth.
        let a = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let b = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let a_leaf = w.insert(a, RawNodeDesc::text(), loc());
        let a_leaf2 = w.insert(a, RawNodeDesc::text(), loc());
        let b_leaf = w.insert(b, RawNodeDesc::text(), loc());

        // Dirty nodes inside `a` and inside `b`. `a` is named twice — once
        // per dirty node inside it.
        w.mark_layout_dirty(a_leaf);
        w.mark_layout_dirty(a_leaf2);
        w.mark_layout_dirty(b_leaf);

        let boundaries = w.dirty_boundaries();
        let mut unique = boundaries.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            boundaries.len(),
            unique.len(),
            "the same boundary was listed twice: {boundaries:?}"
        );
        assert!(boundaries.contains(&a));
        assert!(boundaries.contains(&b));
    }

    #[test]
    fn dirty_levels_are_ordered() {
        assert!(DirtyLevel::Topology > DirtyLevel::Layout);
        assert!(DirtyLevel::Layout > DirtyLevel::Paint);
        assert_eq!(
            DirtyLevel::Clean.max(DirtyLevel::Layout),
            DirtyLevel::Layout
        );
        assert_eq!(
            DirtyLevel::Topology.max(DirtyLevel::Paint),
            DirtyLevel::Topology
        );
    }

    #[test]
    fn edge_values_do_not_leak_into_the_tree() {
        let (mut w, root) = world();
        let a = w.insert(
            root,
            RawNodeDesc::container()
                .padding(Edges::all(4.0))
                .width(Dim::percent(50.0)),
            loc(),
        );
        assert_eq!(w.children(root).count(), 1);
        assert!(w.contains(a));
        assert_eq!(w.node(a).desc.padding.left, 4.0);
    }
}

//! Taffy integration with relayout boundaries (`docs` §4.2).
//!
//! Two ideas carry the performance story:
//!
//! 1. **Isolation.** A `RelayoutBoundary` has a fixed size and clips its
//!    children, so nothing inside it can change anything outside it. Layout dirt
//!    stops there (see `unbound-world`), and this engine only re-solves the
//!    boundary subtree.
//! 2. **Parallelism.** Boundaries are disjoint, so when several are dirty they
//!    are solved concurrently with Rayon. Each worker builds a private small
//!    `TaffyTree` for its subtree, which avoids requiring `TaffyTree: Sync` and
//!    keeps workers fully independent.
//!
//! Text measurement never enters this module: the view builder pre-measures
//! labels and stores the result on the node, so layout sees only numbers. That
//! keeps shaping and layout independently testable and means a layout solve can
//! never accidentally block on the font system.

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::FxHashMap;
use taffy::prelude::*;
use taffy::{AvailableSpace, Layout, LayoutInput, LayoutOutput, TaffyTree};
use unbound_abi::{Align, Dim, DimUnit, Display, FlexDirection, Justify, NodeFlags, Position};
use unbound_math::{Rect, Vec2};
use unbound_world::{NodeId, UiWorld};

/// Result of one solve, for the KPI harness and the debug overlay.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    pub nodes_visited: u32,
    pub boundaries_solved: u32,
    /// Microseconds spent inside Taffy (excludes tree synchronisation).
    pub taffy_us: u64,
    /// Microseconds spent copying styles in and out.
    pub sync_us: u64,
}

impl LayoutStats {
    #[must_use]
    pub fn total_us(&self) -> u64 {
        self.taffy_us + self.sync_us
    }
}

// ---------------------------------------------------------------------------
// Descriptor -> Taffy style
// ---------------------------------------------------------------------------

fn dim(d: Dim) -> Dimension {
    match d.kind() {
        DimUnit::Px => Dimension::length(d.value),
        DimUnit::Percent => Dimension::percent((d.value / 100.0).max(0.0)),
        // `fr` is a grid concept; as a flex basis the literal value is the
        // closest sensible reading.
        DimUnit::Fr => Dimension::length(d.value),
        DimUnit::Auto => Dimension::auto(),
    }
}

fn inset(d: Dim) -> LengthPercentageAuto {
    match d.kind() {
        DimUnit::Px => LengthPercentageAuto::length(d.value),
        DimUnit::Percent => LengthPercentageAuto::percent((d.value / 100.0).max(0.0)),
        _ => LengthPercentageAuto::auto(),
    }
}

fn fixed(v: f32) -> LengthPercentage {
    LengthPercentage::length(v)
}

fn align_items(a: Align) -> taffy::AlignItems {
    match a {
        Align::FlexStart | Align::Baseline => taffy::AlignItems::FLEX_START,
        Align::FlexEnd => taffy::AlignItems::FLEX_END,
        Align::Center => taffy::AlignItems::CENTER,
        Align::Stretch => taffy::AlignItems::STRETCH,
    }
}

/// `align_self` returns `None` for the descriptor's default so that the value
/// is *inherited* from the parent's `align_items`, exactly like CSS
/// `align-self: auto`. Mapping the default to `STRETCH` would make every node
/// ignore its parent's alignment, which is the single most common flex surprise.
fn align_self(a: Align) -> Option<taffy::AlignSelf> {
    match a {
        Align::FlexStart | Align::Baseline => Some(taffy::AlignSelf::FLEX_START),
        Align::FlexEnd => Some(taffy::AlignSelf::FLEX_END),
        Align::Center => Some(taffy::AlignSelf::CENTER),
        Align::Stretch => None,
    }
}

fn justify(j: Justify) -> taffy::JustifyContent {
    match j {
        Justify::FlexStart => taffy::JustifyContent::START,
        Justify::FlexEnd => taffy::JustifyContent::END,
        Justify::Center => taffy::JustifyContent::CENTER,
        Justify::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
        Justify::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
        Justify::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
    }
}

fn display(d: Display) -> taffy::Display {
    match d {
        Display::Flex => taffy::Display::Flex,
        Display::Grid => taffy::Display::Grid,
        Display::Block => taffy::Display::Block,
        Display::None => taffy::Display::None,
    }
}

fn direction(d: FlexDirection) -> taffy::FlexDirection {
    match d {
        FlexDirection::Row => taffy::FlexDirection::Row,
        FlexDirection::Column => taffy::FlexDirection::Column,
        FlexDirection::RowReverse => taffy::FlexDirection::RowReverse,
        FlexDirection::ColumnReverse => taffy::FlexDirection::ColumnReverse,
    }
}

/// Translate one node's descriptor into a Taffy style.
#[must_use]
pub fn style_for(world: &UiWorld, id: NodeId) -> Style {
    let Some(node) = world.get(id) else {
        return Style::default();
    };
    let d = &node.desc;
    let clips_x = d.is_clipping() || d.has(NodeFlags::SCROLL_X);
    let clips_y = d.is_clipping() || d.has(NodeFlags::SCROLL_Y);
    let mut style = Style {
        display: display(d.display_kind()),
        flex_direction: direction(d.direction_kind()),
        align_items: align_items(d.align_items_kind()),
        align_self: align_self(d.align_self_kind()),
        justify_content: justify(d.justify_kind()),
        flex_wrap: if d.has(NodeFlags::WRAP) {
            taffy::FlexWrap::Wrap
        } else {
            taffy::FlexWrap::NoWrap
        },
        size: taffy::Size {
            width: dim(d.width),
            height: dim(d.height),
        },
        min_size: taffy::Size {
            width: inset(d.min_width),
            height: inset(d.min_height),
        },
        max_size: taffy::Size {
            width: inset(d.max_width),
            height: inset(d.max_height),
        },
        padding: taffy::Rect {
            left: fixed(d.padding.left),
            top: fixed(d.padding.top),
            right: fixed(d.padding.right),
            bottom: fixed(d.padding.bottom),
        },
        margin: taffy::Rect {
            left: inset(Dim::px(d.margin.left)),
            top: inset(Dim::px(d.margin.top)),
            right: inset(Dim::px(d.margin.right)),
            bottom: inset(Dim::px(d.margin.bottom)),
        },
        border: taffy::Rect {
            left: fixed(d.border_width),
            top: fixed(d.border_width),
            right: fixed(d.border_width),
            bottom: fixed(d.border_width),
        },
        gap: taffy::Size {
            width: LengthPercentage::length(d.gap_column),
            height: LengthPercentage::length(d.gap_row),
        },
        flex_grow: d.flex_grow.max(0.0),
        flex_shrink: d.flex_shrink.max(0.0),
        flex_basis: dim(d.flex_basis),
        position: match d.position_kind() {
            Position::Relative => taffy::Position::Relative,
            Position::Absolute => taffy::Position::Absolute,
        },
        inset: taffy::Rect {
            left: inset(Dim::px(d.inset.left)),
            top: inset(Dim::px(d.inset.top)),
            right: inset(Dim::px(d.inset.right)),
            bottom: inset(Dim::px(d.inset.bottom)),
        },
        overflow: taffy::Point {
            x: if clips_x {
                taffy::Overflow::Hidden
            } else {
                taffy::Overflow::Visible
            },
            y: if clips_y {
                taffy::Overflow::Hidden
            } else {
                taffy::Overflow::Visible
            },
        },
        ..Default::default()
    };
    if d.aspect_ratio > 0.0 {
        style.aspect_ratio = Some(d.aspect_ratio);
    }
    style
}

/// Resolve a leaf's size from taffy's known dimensions, falling back to the
/// pre-measured intrinsic size (text, images) and finally to zero.
fn measure_leaf(input: &LayoutInput, id: Option<NodeId>, world: &UiWorld) -> LayoutOutput {
    let intrinsic = id
        .and_then(|id| world.get(id))
        .and_then(|n| n.measure)
        .unwrap_or(Vec2::ZERO);
    let width = input.known_dimensions.width.unwrap_or(intrinsic.x).max(0.0);
    let height = input
        .known_dimensions
        .height
        .unwrap_or(intrinsic.y)
        .max(0.0);
    LayoutOutput::from_outer_size(taffy::Size { width, height })
}

fn available(viewport: Vec2) -> taffy::Size<AvailableSpace> {
    taffy::Size {
        width: AvailableSpace::Definite(viewport.x.max(1.0)),
        height: AvailableSpace::Definite(viewport.y.max(1.0)),
    }
}

/// The size a node's own style fixes, per axis.
///
/// A definite size is the node's own business; an automatic one is decided
/// by its content.
fn definite_size(world: &UiWorld, id: NodeId) -> (Option<f32>, Option<f32>) {
    let Some(node) = world.get(id) else {
        return (None, None);
    };
    let px = |d: Dim| {
        if d.unit == DimUnit::Px as u8 {
            Some(d.value)
        } else {
            None
        }
    };
    (px(node.desc.width), px(node.desc.height))
}

fn rect_of(layout: &Layout) -> Rect {
    Rect::new(
        layout.location.x,
        layout.location.y,
        layout.size.width,
        layout.size.height,
    )
}

/// Pre-order walk that yields (node, children) without recursion.
fn preorder(world: &UiWorld, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id);
        let mut kids = Vec::new();
        let mut child = world.get(id).map(|n| n.first_child);
        while let Some(c) = child {
            if c.is_none() {
                break;
            }
            kids.push(c);
            child = world.get(c).map(|n| n.next_sibling);
        }
        for c in kids.into_iter().rev() {
            stack.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Persistent layout engine over the UI world.
pub struct LayoutEngine {
    taffy: TaffyTree<NodeId>,
    map: FxHashMap<NodeId, taffy::NodeId>,
    rects: FxHashMap<NodeId, Rect>,
    /// Nodes touched by the last `collect`, reused across solves.
    changed: Vec<NodeId>,
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for LayoutEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LayoutEngine")
            .field("nodes", &self.map.len())
            .finish_non_exhaustive()
    }
}

impl LayoutEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            map: FxHashMap::default(),
            rects: FxHashMap::default(),
            changed: Vec::new(),
        }
    }

    /// Drop every cached node (world rebuild or viewport reset).
    pub fn clear(&mut self) {
        self.taffy = TaffyTree::new();
        self.map.clear();
        self.rects.clear();
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.map.len()
    }

    /// Create missing Taffy nodes and refresh every style.
    ///
    /// Style writes are unconditional because a style diff would cost as much as
    /// the write; the dirty tracking that matters happens *above* this layer, by
    /// deciding whether to call it at all.
    pub fn sync(&mut self, world: &UiWorld, root: NodeId) {
        let order = preorder(world, root);
        // Pass 1: ensure every node exists and carries the right style.
        for id in &order {
            let style = style_for(world, *id);
            match self.map.get(id) {
                Some(t) => {
                    let _ = self.taffy.set_style(*t, style);
                }
                None => {
                    let t = self
                        .taffy
                        .new_leaf_with_context(style, *id)
                        .expect("taffy node allocation");
                    self.map.insert(*id, t);
                }
            }
        }
        // Pass 2: attach children now that every node exists.
        for id in &order {
            let Some(t) = self.map.get(id).copied() else {
                continue;
            };
            let kids: Vec<taffy::NodeId> = world
                .children(*id)
                .filter_map(|c| self.map.get(&c).copied())
                .collect();
            let _ = self.taffy.set_children(t, &kids);
        }
        // Pass 3: drop the nodes that left the world.
        //
        // `UiWorld::remove` frees the slot, but the engine kept the node's
        // Taffy node, its map entry and its cached rectangle forever: a long
        // session that creates and drops widgets leaked one of each per
        // removal.
        self.map.retain(|id, taffy_id| {
            let alive = world.contains(*id);
            if !alive {
                let _ = self.taffy.remove(*taffy_id);
                self.rects.remove(id);
            }
            alive
        });
    }

    /// Full re-solve under `viewport` (topology changes, resize).
    pub fn solve(&mut self, world: &mut UiWorld, root: NodeId, viewport: Vec2) -> LayoutStats {
        let sync_start = std::time::Instant::now();
        self.sync(world, root);
        let sync_us = sync_start.elapsed().as_micros() as u64;

        let Some(taffy_root) = self.map.get(&root).copied() else {
            return LayoutStats {
                sync_us,
                ..Default::default()
            };
        };

        let start = std::time::Instant::now();
        {
            // Split the borrows: Taffy is mutated, the world is only read.
            let world_ref: &UiWorld = world;
            let taffy = &mut self.taffy;
            let _ = taffy.compute_layout_with_measure(
                taffy_root,
                available(viewport),
                |input: LayoutInput,
                 _node: taffy::NodeId,
                 ctx: Option<&mut NodeId>,
                 _style: &Style| {
                    // Taffy may call the measure function for any node it
                    // created with a context, including plain containers. The
                    // known dimensions it passes in are authoritative: ignoring
                    // them would silently collapse every fixed-size leaf to 0.
                    measure_leaf(&input, ctx.map(|c| *c), world_ref)
                },
            );
        }
        let taffy_us = start.elapsed().as_micros() as u64;

        let visited = self.collect(world, root);
        world.apply_rects(&self.rects);
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: 1,
            taffy_us,
            sync_us,
        }
    }

    /// Re-solve only the given boundary subtrees.
    ///
    /// One boundary uses the persistent tree (no allocation at all). Several are
    /// solved in parallel, each in a private tree, because they are disjoint by
    /// construction.
    pub fn solve_boundaries(
        &mut self,
        world: &mut UiWorld,
        boundaries: &[NodeId],
        viewport: Vec2,
    ) -> LayoutStats {
        if boundaries.is_empty() {
            return LayoutStats::default();
        }
        if boundaries.len() == 1 {
            return self.solve_subtree(world, boundaries[0], viewport);
        }

        let start = std::time::Instant::now();
        let nested: Vec<Vec<(NodeId, Rect)>> = {
            use rayon::prelude::*;
            boundaries
                .par_iter()
                .map(|b| {
                    // The boundary is solved as a root, so it is given the
                    // space it had: a child with `width: 100 %` resolves
                    // against the containing block.
                    let size = world.node(*b).rect.size;
                    let space = Vec2::new(size.x.max(viewport.x), size.y.max(viewport.y));
                    solve_isolated(world, *b, space)
                })
                .collect()
        };
        let taffy_us = start.elapsed().as_micros() as u64;

        let mut visited = 0u32;
        for (chunk, boundary) in nested.into_iter().zip(boundaries.iter().copied()) {
            let placement = world.node(boundary).rect;
            let changed: Vec<NodeId> = chunk.iter().map(|(id, _)| *id).collect();
            visited += chunk.len() as u32;
            for (id, r) in chunk {
                self.rects.insert(id, r);
            }
            // The subtree was solved with the boundary as its root, so the
            // boundary's own rectangle comes back at the origin — but in the
            // tree it sits wherever its parent put it. Keep the parent's
            // placement and take only the size from the new solve.
            let (fixed_width, fixed_height) = definite_size(world, boundary);
            if let Some(rect) = self.rects.get_mut(&boundary) {
                rect.origin = placement.origin;
                // A size the style fixes belongs to the node itself: Taffy
                // sizes a root from its content, so the fixed size has to be
                // restored.
                if let Some(width) = fixed_width {
                    rect.size.x = width;
                }
                if let Some(height) = fixed_height {
                    rect.size.y = height;
                }
            }
            world.apply_rects_from(&self.rects, &changed, boundary);
        }
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: boundaries.len() as u32,
            taffy_us,
            sync_us: 0,
        }
    }

    /// Re-solve one subtree on the persistent tree.
    pub fn solve_subtree(
        &mut self,
        world: &mut UiWorld,
        boundary: NodeId,
        viewport: Vec2,
    ) -> LayoutStats {
        if !self.map.contains_key(&boundary) {
            // Unknown subtree: a full solve is the only correct answer.
            let root = world.root();
            return self.solve(world, root, viewport);
        }
        let sync_start = std::time::Instant::now();
        self.sync(world, boundary);
        let sync_us = sync_start.elapsed().as_micros() as u64;

        let taffy_root = self.map[&boundary];
        // Taffy sizes a *root* node from its children, so the subtree has to
        // be solved against the space the boundary had: a child with
        // `width: 100 %` resolves against the containing block, and without
        // the old size it would resolve against nothing and collapse.
        let space = self.rects.get(&boundary).map_or(viewport, |r| r.size);
        let start = std::time::Instant::now();
        {
            let world_ref: &UiWorld = world;
            let taffy = &mut self.taffy;
            let _ = taffy.compute_layout_with_measure(
                taffy_root,
                available(space),
                |input: LayoutInput,
                 _node: taffy::NodeId,
                 ctx: Option<&mut NodeId>,
                 _style: &Style| {
                    // Taffy may call the measure function for any node it
                    // created with a context, including plain containers. The
                    // known dimensions it passes in are authoritative: ignoring
                    // them would silently collapse every fixed-size leaf to 0.
                    measure_leaf(&input, ctx.map(|c| *c), world_ref)
                },
            );
        }
        let taffy_us = start.elapsed().as_micros() as u64;

        let visited = self.collect(world, boundary);
        let changed = core::mem::take(&mut self.changed);
        // The subtree was solved with the boundary as its root, so the
        // boundary's own rectangle comes back at the origin — but in the
        // tree it sits wherever its parent put it. Keep the parent's
        // placement and take only the size from the new solve.
        let placement = world.node(boundary).rect;
        let (fixed_width, fixed_height) = definite_size(world, boundary);
        if let Some(rect) = self.rects.get_mut(&boundary) {
            rect.origin = placement.origin;
            // A size the style fixes belongs to the node itself: Taffy sizes
            // a root from its content, so the fixed size has to be restored.
            if let Some(width) = fixed_width {
                rect.size.x = width;
            }
            if let Some(height) = fixed_height {
                rect.size.y = height;
            }
        }
        world.apply_rects_from(&self.rects, &changed, boundary);
        self.changed = changed;
        LayoutStats {
            nodes_visited: visited,
            boundaries_solved: 1,
            taffy_us,
            sync_us,
        }
    }

    /// Read layouts for `root`'s subtree into `self.rects`.
    ///
    /// The map is *not* cleared: an incremental solve only refreshes the nodes it
    /// touched, and throwing away the untouched geometry would leave
    /// [`Self::rect`] unable to answer for the rest of the tree — which is
    /// exactly what a caller painting the frame needs.
    fn collect(&mut self, world: &UiWorld, root: NodeId) -> u32 {
        self.changed.clear();
        let mut visited = 0u32;
        for id in preorder(world, root) {
            if let Some(t) = self.map.get(&id) {
                if let Ok(layout) = self.taffy.layout(*t) {
                    self.rects.insert(id, rect_of(layout));
                    self.changed.push(id);
                    visited += 1;
                }
            }
        }
        visited
    }

    /// Last computed rectangle for a node, in its parent's coordinate space.
    ///
    /// Taffy reports a node's position inside its parent, so the rectangle
    /// is relative — the same space [`unbound_world::UiWorld::apply_rects`]
    /// writes back into the world. Use [`Self::world_rect`] when absolute
    /// coordinates are what the caller needs.
    #[must_use]
    pub fn rect(&self, id: NodeId) -> Option<Rect> {
        self.rects.get(&id).copied()
    }

    /// Last computed rectangle for a node in absolute window coordinates.
    ///
    /// The world folds every ancestor's offset into `world_rect` when it
    /// applies a solve, so this is the rectangle a painter wants.
    #[must_use]
    pub fn world_rect(&self, world: &UiWorld, id: NodeId) -> Option<Rect> {
        world.get(id).map(|node| node.world_rect)
    }
}

/// Solve `root`'s subtree in a private Taffy tree.
///
/// Returns the subtree's rectangles; the caller merges them into the world. This
/// is the unit of parallelism: no shared state, no `Sync` bound on `TaffyTree`.
fn solve_isolated(world: &UiWorld, root: NodeId, viewport: Vec2) -> Vec<(NodeId, Rect)> {
    let order = preorder(world, root);
    if order.is_empty() {
        return Vec::new();
    }
    let mut tree: TaffyTree<NodeId> = TaffyTree::new();
    let mut created: FxHashMap<NodeId, taffy::NodeId> =
        FxHashMap::with_capacity_and_hasher(order.len(), Default::default());
    for id in &order {
        let t = tree
            .new_leaf_with_context(style_for(world, *id), *id)
            .expect("leaf");
        created.insert(*id, t);
    }
    for id in &order {
        let kids: Vec<taffy::NodeId> = world
            .children(*id)
            .filter_map(|c| created.get(&c).copied())
            .collect();
        let _ = tree.set_children(created[id], &kids);
    }
    let root_t = created[&root];
    let _ = tree.compute_layout_with_measure(
        root_t,
        available(viewport),
        |input: LayoutInput, _node: taffy::NodeId, ctx: Option<&mut NodeId>, _style: &Style| {
            measure_leaf(&input, ctx.map(|c| *c), world)
        },
    );
    let mut out = Vec::with_capacity(order.len());
    for id in order {
        if let Ok(l) = tree.layout(created[&id]) {
            out.push((id, rect_of(l)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_abi::{Edges, RawNodeDesc};

    fn loc() -> &'static core::panic::Location<'static> {
        core::panic::Location::caller()
    }

    /// Builds: root (400x300) with a fixed 200x100 card containing a label.
    fn fixture() -> (UiWorld, NodeId, NodeId, NodeId) {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(400.0);
        w.node_mut(root).desc.height = Dim::px(300.0);
        let card = w.insert(
            root,
            RawNodeDesc::container()
                .size(200.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let label = w.insert(
            card,
            RawNodeDesc::text().flags(NodeFlags::MEASURE_TEXT),
            loc(),
        );
        w.node_mut(label).measure = Some(Vec2::new(80.0, 16.0));
        (w, root, card, label)
    }

    /// A subtree that is re-solved with the boundary as its root comes back
    /// with the boundary's own rectangle at the origin — but in the tree the
    /// boundary sits wherever its parent put it.
    ///
    /// Losing that placement is what made a re-solved card jump to the top
    /// left corner and land on top of its siblings.
    #[test]
    fn incremental_boundary_solve_preserves_the_boundary_origin() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(400.0);
        w.node_mut(root).desc.height = Dim::px(300.0);
        w.node_mut(root).desc.gap_row = 8.0;
        let first = w.insert(root, RawNodeDesc::container().size(200.0, 50.0), loc());
        let second = w.insert(root, RawNodeDesc::container().size(200.0, 50.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let placement = engine.rect(second).expect("rect").origin;
        assert!(placement.y > 0.0, "the second row is not at the top");

        // Re-solve the second row on its own.
        engine.solve_subtree(&mut w, second, Vec2::new(400.0, 300.0));
        let after = engine.rect(second).expect("rect");
        assert_eq!(
            after.origin, placement,
            "the boundary must keep the position its parent gave it"
        );
        assert_eq!(
            after.size,
            Vec2::new(200.0, 50.0),
            "the size is still taken"
        );
        assert_ne!(first, second);
    }

    /// A node that left the world must not stay in the engine's tables.
    #[test]
    fn sync_removes_deleted_world_nodes_from_taffy() {
        let (mut w, root, card, _label) = fixture();
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let before = engine.node_count();
        assert!(before >= 3);

        w.remove(card);
        engine.sync(&w, root);
        assert!(
            engine.node_count() < before,
            "the removed node must not stay in the engine: {} vs {before}",
            engine.node_count()
        );
        assert!(engine.rect(card).is_none());
    }

    #[test]
    fn a_fixed_card_gets_its_exact_size() {
        let (mut w, root, card, _label) = fixture();
        let mut engine = LayoutEngine::new();
        let stats = engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        assert!(stats.nodes_visited >= 3);
        let rect = engine.rect(card).expect("card rect");
        assert_eq!(rect.size, Vec2::new(200.0, 100.0));
        assert_eq!(rect.origin, Vec2::ZERO);
        assert_eq!(w.node(card).rect.size, Vec2::new(200.0, 100.0));
    }

    #[test]
    fn column_flow_stacks_children_with_a_gap() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(200.0);
        w.node_mut(root).desc.gap_row = 8.0;
        let a = w.insert(root, RawNodeDesc::container().size(50.0, 20.0), loc());
        let b = w.insert(root, RawNodeDesc::container().size(50.0, 20.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 200.0));
        assert_eq!(engine.rect(a).unwrap().origin, Vec2::new(0.0, 0.0));
        assert_eq!(engine.rect(b).unwrap().origin, Vec2::new(0.0, 28.0));
    }

    #[test]
    fn row_flow_uses_the_column_gap() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(40.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        w.node_mut(root).desc.gap_column = 6.0;
        let a = w.insert(root, RawNodeDesc::container().size(30.0, 10.0), loc());
        let b = w.insert(root, RawNodeDesc::container().size(30.0, 10.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 40.0));
        assert_eq!(engine.rect(a).unwrap().origin, Vec2::new(0.0, 0.0));
        assert_eq!(engine.rect(b).unwrap().origin, Vec2::new(36.0, 0.0));
    }

    #[test]
    fn flex_grow_distributes_the_remaining_space() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(300.0);
        w.node_mut(root).desc.height = Dim::px(50.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let mut left = RawNodeDesc::container();
        left.width = Dim::px(100.0);
        left.height = Dim::px(10.0);
        let a = w.insert(root, left, loc());
        let mut right = RawNodeDesc::container();
        right.width = Dim::px(0.0);
        right.height = Dim::px(10.0);
        right.flex_grow = 1.0;
        let b = w.insert(root, right, loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(300.0, 50.0));
        assert_eq!(engine.rect(a).unwrap().size.x, 100.0);
        assert_eq!(engine.rect(b).unwrap().size.x, 200.0);
    }

    #[test]
    fn padding_shifts_the_content_box() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(100.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        w.node_mut(root).desc.padding = Edges::all(10.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let child = w.insert(root, RawNodeDesc::container().size(20.0, 20.0), loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(100.0, 100.0));
        assert_eq!(engine.rect(child).unwrap().origin, Vec2::new(10.0, 10.0));
    }

    #[test]
    fn percentage_sizes_resolve_against_the_parent() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let half = w.insert(
            root,
            RawNodeDesc::container()
                .width(Dim::percent(50.0))
                .height(Dim::px(10.0)),
            loc(),
        );
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 100.0));
        assert_eq!(engine.rect(half).unwrap().size.x, 100.0);
    }

    #[test]
    fn measured_text_sets_the_leaf_size() {
        let (mut w, root, card, label) = fixture();
        // A stretching column would widen the label to the card's content box
        // (correct flex behaviour); pin it to the intrinsic width instead.
        w.node_mut(card).desc.align_items = Align::FlexStart as u8;
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let rect = engine.rect(label).expect("label rect");
        assert_eq!(rect.size, Vec2::new(80.0, 16.0));
    }

    #[test]
    fn absolute_positioning_uses_insets() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(200.0);
        w.node_mut(root).desc.height = Dim::px(200.0);
        let mut abs = RawNodeDesc::container().size(40.0, 40.0).absolute();
        abs.inset = Edges::trbl(10.0, 20.0, 0.0, 0.0);
        let child = w.insert(root, abs, loc());
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(200.0, 200.0));
        assert_eq!(engine.rect(child).unwrap().origin, Vec2::new(10.0, 20.0));
    }

    #[test]
    fn display_none_removes_the_node_from_flow() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(100.0);
        w.node_mut(root).desc.height = Dim::px(100.0);
        let a = w.insert(root, RawNodeDesc::container().size(10.0, 10.0), loc());
        let hidden = w.insert(
            root,
            RawNodeDesc::container()
                .size(10.0, 10.0)
                .display(unbound_abi::Display::None),
            loc(),
        );
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(100.0, 100.0));
        assert_eq!(engine.rect(hidden).unwrap().size, Vec2::ZERO);
        assert_eq!(engine.rect(a).unwrap().size, Vec2::new(10.0, 10.0));
    }

    #[test]
    fn incremental_solve_touches_only_the_boundary() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(600.0);
        w.node_mut(root).desc.height = Dim::px(400.0);
        let cards: Vec<NodeId> = (0..4)
            .map(|_| {
                w.insert(
                    root,
                    RawNodeDesc::container()
                        .size(100.0, 100.0)
                        .flags(NodeFlags::CLIP),
                    loc(),
                )
            })
            .collect();
        let labels: Vec<NodeId> = cards
            .iter()
            .map(|c| {
                let l = w.insert(*c, RawNodeDesc::text(), loc());
                w.node_mut(l).measure = Some(Vec2::new(40.0, 12.0));
                l
            })
            .collect();

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(600.0, 400.0));
        w.clear_dirty();

        // Change one label and solve only its card.
        w.mark_layout_dirty(labels[1]);
        let boundaries = w.dirty_boundaries();
        assert_eq!(boundaries, vec![cards[1]]);
        let stats = engine.solve_boundaries(&mut w, &boundaries, Vec2::new(600.0, 400.0));
        assert!(
            stats.nodes_visited <= 2,
            "only the card and its label should be visited, got {}",
            stats.nodes_visited
        );
        // The other cards keep their geometry (read through the world, because
        // `engine.rect` only reports the most recent solve).
        assert_eq!(w.node(cards[0]).rect.size, Vec2::new(100.0, 100.0));
    }

    #[test]
    fn multiple_boundaries_solve_in_parallel_with_identical_results() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(800.0);
        w.node_mut(root).desc.height = Dim::px(600.0);
        let cards: Vec<NodeId> = (0..6)
            .map(|_| {
                w.insert(
                    root,
                    RawNodeDesc::container()
                        .size(120.0, 80.0)
                        .flags(NodeFlags::CLIP),
                    loc(),
                )
            })
            .collect();
        for c in &cards {
            let l = w.insert(*c, RawNodeDesc::text(), loc());
            w.node_mut(l).measure = Some(Vec2::new(60.0, 14.0));
        }

        let mut full = LayoutEngine::new();
        full.solve(&mut w, root, Vec2::new(800.0, 600.0));
        let expected: Vec<Rect> = cards.iter().map(|c| full.rect(*c).unwrap()).collect();

        let mut parallel = LayoutEngine::new();
        parallel.sync(&w, root);
        let stats = parallel.solve_boundaries(&mut w, &cards, Vec2::new(800.0, 600.0));
        assert_eq!(stats.boundaries_solved, cards.len() as u32);
        for (card, want) in cards.iter().zip(expected) {
            let got = parallel.rect(*card).unwrap();
            assert_eq!(got.size, want.size, "size mismatch for {card:?}");
        }
    }

    #[test]
    fn solving_twice_is_idempotent() {
        let (mut w, root, card, _) = fixture();
        let mut engine = LayoutEngine::new();
        let first = engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        let r1 = engine.rect(card).unwrap();
        let second = engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        assert_eq!(engine.rect(card).unwrap(), r1);
        assert_eq!(first.nodes_visited, second.nodes_visited);
    }

    #[test]
    fn clear_resets_the_engine() {
        let (mut w, root, _, _) = fixture();
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        assert!(engine.node_count() > 0);
        engine.clear();
        assert_eq!(engine.node_count(), 0);
        assert!(engine.rect(root).is_none());
    }

    #[test]
    fn text_node_inside_a_card_does_not_resize_the_card() {
        let (mut w, root, card, label) = fixture();
        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(400.0, 300.0));
        // Make the label enormous; the fixed-size card must not grow.
        w.node_mut(label).measure = Some(Vec2::new(900.0, 400.0));
        w.mark_layout_dirty(label);
        let boundaries = w.dirty_boundaries();
        engine.solve_boundaries(&mut w, &boundaries, Vec2::new(400.0, 300.0));
        assert_eq!(engine.rect(card).unwrap().size, Vec2::new(200.0, 100.0));
    }

    #[test]
    fn tag_specific_style_is_stable() {
        let mut w = UiWorld::new();
        let root = w.root();
        let t = w.insert(
            root,
            RawNodeDesc::text().flags(NodeFlags::MEASURE_TEXT),
            loc(),
        );
        let style = style_for(&w, t);
        assert_eq!(style.display, taffy::Display::Flex);
        assert_eq!(style.size.width, Dimension::auto());
    }

    /// An incremental solve must not move the boundary it solves.
    ///
    /// The subtree is re-solved with the boundary as the *root* of that
    /// computation, so the boundary's own rectangle comes back at the origin
    /// — while in the tree it sits wherever its parent put it. Writing that
    /// rectangle back moved the card to the top-left of its parent, and the
    /// clip folding that follows propagated the wrong position.
    #[test]
    fn an_incremental_solve_keeps_the_boundary_where_it_was() {
        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(600.0);
        w.node_mut(root).desc.height = Dim::px(400.0);
        w.node_mut(root).desc.direction = unbound_abi::FlexDirection::Row as u8;
        let first = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let second = w.insert(
            root,
            RawNodeDesc::container()
                .size(100.0, 100.0)
                .flags(NodeFlags::CLIP),
            loc(),
        );
        let label = w.insert(second, RawNodeDesc::text(), loc());
        w.node_mut(label).measure = Some(Vec2::new(40.0, 12.0));

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(600.0, 400.0));
        let before = engine.rect(second).expect("second card");
        assert_eq!(
            before.origin,
            Vec2::new(100.0, 0.0),
            "the card is not at the origin"
        );

        // Change the label and re-solve only the second card's boundary.
        w.node_mut(label).measure = Some(Vec2::new(80.0, 12.0));
        w.mark_layout_dirty(label);
        let boundaries = w.dirty_boundaries();
        assert_eq!(boundaries, vec![second]);
        engine.solve_boundaries(&mut w, &boundaries, Vec2::new(600.0, 400.0));

        let after = engine.rect(second).expect("second card");
        assert_eq!(
            after.origin, before.origin,
            "the incremental solve moved the boundary to the origin"
        );
        assert_eq!(w.node(second).rect.origin, before.origin);
        assert_eq!(w.node(first).rect.origin, Vec2::new(0.0, 0.0));
    }

    /// The same defect, seen the way a long list sees it.
    ///
    /// The gallery lays out 200 rows and re-solves the one in the middle.
    /// Before the fix that row was teleported to the top of the list — it
    /// landed *inside the viewport*, so the spatial index reported one row
    /// too many: a row was drawn on top of the first one, and the one that
    /// belonged there had vanished.
    #[test]
    fn a_row_re_solved_inside_a_list_stays_at_its_index() {
        const ROWS: usize = 200;
        const ROW_HEIGHT: f32 = 32.0;

        let mut w = UiWorld::new();
        let root = w.root();
        w.node_mut(root).desc.width = Dim::px(1280.0);
        w.node_mut(root).desc.height = Dim::px(720.0);
        let list = w.insert(
            root,
            RawNodeDesc::text().size(1280.0, 600.0).column(),
            loc(),
        );
        let mut rows = Vec::with_capacity(ROWS);
        for _ in 0..ROWS {
            let mut desc = RawNodeDesc::text().size(1280.0, ROW_HEIGHT).row();
            desc.flex_shrink = 0.0;
            rows.push(w.insert(list, desc, loc()));
        }

        let mut engine = LayoutEngine::new();
        engine.solve(&mut w, root, Vec2::new(1280.0, 720.0));
        let middle = rows[ROWS / 2];
        let expected = w.node(middle).rect.origin;
        assert_eq!(
            expected,
            Vec2::new(0.0, (ROWS / 2) as f32 * ROW_HEIGHT),
            "the row must start at its own index"
        );

        // Re-solve the middle row's boundary, the way an edit would. The
        // gallery hands the row straight to the engine.
        engine.solve_boundaries(&mut w, &[middle], Vec2::new(1280.0, 720.0));

        assert_eq!(
            w.node(middle).rect.origin,
            expected,
            "the re-solved row was teleported to the top of the list"
        );

        // The row that belongs at the top must still be the one there.
        assert_eq!(
            w.node(rows[0]).rect.origin,
            Vec2::new(0.0, 0.0),
            "the first row must not have been displaced"
        );
    }
}

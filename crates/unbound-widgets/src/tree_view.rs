//! The virtualised tree: only the rows inside the viewport exist.
//!
//! The flattening and slot recycling live in
//! [`unbound_world::virtualize::VirtualTree`]; this module adds the control
//! semantics — selection, focus, inline rename and row painting. A 100 000
//! node document therefore keeps a constant number of rows alive, and a
//! collapsed subtree costs nothing because `children_of` is only called for
//! expanded nodes.

use accesskit::Role;
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_world::virtualize::{TreeRow, VirtualTree, WindowUpdate};

use crate::primitive::{focus_ring, glyphs, instance, outlined};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::switch::CONTROL_LAYER;
use crate::text_input::TextEditState;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default row height.
pub const TREE_ROW: f32 = 26.0;

/// The horizontal indent per depth level.
pub const TREE_INDENT: f32 = 16.0;

/// The number of rows kept alive around a viewport.
pub const TREE_OVERSCAN: f32 = 1.0;

/// One instantiated row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeViewRow {
    /// The flattened row.
    pub row: TreeRow,
    /// The content-space index of the row.
    pub item: usize,
    /// The screen rectangle of the row.
    pub rect: Rect,
}

/// The pure descriptor of a tree view.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeViewDesc {
    /// The viewport rectangle.
    pub rect: Rect,
    /// The row height.
    pub row_height: f32,
    /// The indent per depth level.
    pub indent: f32,
    /// The draw order key.
    pub z: u16,
}

impl TreeViewDesc {
    /// A tree viewport with the default metrics.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            row_height: TREE_ROW,
            indent: TREE_INDENT,
            z: 450,
        }
    }

    /// The scroll descriptor for the flattened row count.
    #[must_use]
    pub fn scroll_desc(&self, rows: usize) -> ScrollViewDesc {
        ScrollViewDesc::new(
            self.rect,
            unbound_math::Vec2::new(self.rect.size.x, rows as f32 * self.row_height),
        )
    }

    /// How many rows fit in the viewport.
    #[must_use]
    pub fn visible_capacity(&self) -> usize {
        (self.rect.size.y / self.row_height).ceil().max(1.0) as usize
    }
}

/// The live state of a tree view.
#[derive(Debug)]
pub struct TreeViewState {
    /// The virtualisation window.
    pub tree: VirtualTree,
    /// The scroll spring.
    pub scroll: ScrollState,
    /// The selected rows.
    pub selected: FxHashSet<u64>,
    /// The row with keyboard focus.
    pub focused: Option<u64>,
    /// The row currently being renamed and its editor.
    pub renaming: Option<(u64, TextEditState)>,
    /// Every flattened row, for keyboard navigation.
    order: Vec<TreeRow>,
    /// Parent of every flattened row.
    parent: FxHashMap<u64, u64>,
    /// Last window update, for the caller's dirty flag.
    pub last_window: WindowUpdate,
    /// Viewport height the last [`TreeViewState::rows`] call used.
    viewport_height: f32,
}

impl TreeViewState {
    /// An empty tree.
    #[must_use]
    pub fn new(row_height: f32) -> Self {
        Self {
            tree: VirtualTree::new(row_height, TREE_OVERSCAN),
            scroll: ScrollState::new(),
            selected: FxHashSet::default(),
            focused: None,
            renaming: None,
            order: Vec::new(),
            parent: FxHashMap::default(),
            last_window: WindowUpdate::default(),
            viewport_height: 0.0,
        }
    }

    /// Replace the model.
    ///
    /// `children_of` is called for expanded nodes only, so the cost tracks
    /// the open part of the tree rather than the whole document.
    pub fn rebuild<F>(&mut self, roots: &[TreeRow], children_of: F)
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        self.order.clear();
        self.parent.clear();
        // Flatten once to capture parent links for ArrowLeft; the virtual
        // tree performs the same walk with the same expansion set.
        let mut stack: Vec<(TreeRow, Option<u64>)> =
            roots.iter().rev().map(|row| (*row, None)).collect();
        while let Some((mut row, parent)) = stack.pop() {
            row.expanded = self.tree.is_expanded(row.id);
            self.order.push(row);
            if let Some(parent) = parent {
                self.parent.insert(row.id, parent);
            }
            if row.expanded {
                for child in children_of(row.id).into_iter().rev() {
                    stack.push((child, Some(row.id)));
                }
            }
        }
        self.tree.rebuild(roots, children_of);
    }

    /// The number of flattened rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.order.len()
    }

    /// The flattened row with a given identity.
    #[must_use]
    pub fn row(&self, id: u64) -> Option<TreeRow> {
        self.order.iter().copied().find(|row| row.id == id)
    }

    /// The parent of a row, when it is currently visible.
    #[must_use]
    pub fn parent_of(&self, id: u64) -> Option<u64> {
        self.parent.get(&id).copied()
    }

    /// The current scroll offset, including the in-flight spring.
    #[must_use]
    pub fn scroll_offset(&self) -> f32 {
        self.scroll.offset().y
    }

    /// Expand or collapse a row, then rebuild.
    pub fn set_expanded<F>(
        &mut self,
        id: u64,
        expanded: bool,
        roots: &[TreeRow],
        children_of: F,
    ) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        if self.tree.is_expanded(id) == expanded {
            return false;
        }
        self.tree.set_expanded(id, expanded);
        self.rebuild(roots, children_of);
        true
    }

    /// Toggle a row's expansion, then rebuild.
    pub fn toggle_expanded<F>(&mut self, id: u64, roots: &[TreeRow], children_of: F) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        let expanded = self.tree.is_expanded(id);
        self.set_expanded(id, !expanded, roots, children_of)
    }

    /// Select a row. Without `additive` the previous selection is replaced.
    pub fn select(&mut self, id: u64, additive: bool) -> bool {
        let changed = if additive {
            if self.selected.contains(&id) {
                self.selected.remove(&id);
                true
            } else {
                self.selected.insert(id);
                true
            }
        } else {
            let changed = self.selected.len() != 1 || !self.selected.contains(&id);
            self.selected.clear();
            self.selected.insert(id);
            changed
        };
        self.focused = Some(id);
        changed
    }

    /// Clear the selection.
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Move keyboard focus by a number of rows.
    pub fn focus_step(&mut self, delta: isize) -> bool {
        if self.order.is_empty() {
            return false;
        }
        let current = self
            .focused
            .and_then(|id| self.order.iter().position(|row| row.id == id))
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.order.len() as isize - 1) as usize;
        let id = self.order[next].id;
        self.focused = Some(id);
        self.selected.clear();
        self.selected.insert(id);
        true
    }

    /// Move focus to the parent row, collapsing the focused row first when it
    /// is open. This is the `ArrowLeft` contract.
    pub fn focus_parent_or_collapse<F>(&mut self, roots: &[TreeRow], children_of: F) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        let Some(id) = self.focused else {
            return false;
        };
        if self.tree.is_expanded(id) {
            return self.set_expanded(id, false, roots, children_of);
        }
        let Some(parent) = self.parent_of(id) else {
            return false;
        };
        self.select(parent, false);
        let position = self
            .order
            .iter()
            .position(|row| row.id == parent)
            .unwrap_or(0);
        self.reveal(position);
        true
    }

    /// Expand the focused row, or focus its first child when already open.
    pub fn expand_or_first_child<F>(&mut self, roots: &[TreeRow], children_of: F) -> bool
    where
        F: Fn(u64) -> Vec<TreeRow>,
    {
        let Some(id) = self.focused else {
            return false;
        };
        let Some(row) = self.row(id) else {
            return false;
        };
        if !row.has_children {
            return false;
        }
        if !self.tree.is_expanded(id) {
            return self.set_expanded(id, true, roots, children_of);
        }
        let position = self.order.iter().position(|row| row.id == id).unwrap_or(0);
        if let Some(child) = self.order.get(position + 1).copied() {
            self.select(child.id, false);
            return true;
        }
        false
    }

    /// Start an inline rename.
    pub fn begin_rename(&mut self, id: u64, current: impl Into<String>) -> bool {
        if self.row(id).is_none() {
            return false;
        }
        let mut editor = TextEditState::new();
        editor.set_text(current);
        self.renaming = Some((id, editor));
        self.focused = Some(id);
        true
    }

    /// The rename editor, when one is open.
    #[must_use]
    pub fn rename_editor(&self) -> Option<&TextEditState> {
        self.renaming.as_ref().map(|(_, editor)| editor)
    }

    /// Finish the rename and return the new title.
    pub fn finish_rename(&mut self) -> Option<(u64, String)> {
        self.renaming
            .take()
            .map(|(id, editor)| (id, editor.text().to_owned()))
    }

    /// Cancel the rename.
    pub fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    /// Scroll so a flattened row is visible.
    pub fn reveal(&mut self, item: usize) {
        let rows = self.order.len();
        let desc = ScrollViewDesc::new(
            Rect::new(0.0, 0.0, 1.0, self.viewport_height),
            unbound_math::Vec2::new(1.0, rows as f32 * self.row_height()),
        );
        let y = item as f32 * self.row_height();
        if y < self.scroll_offset() {
            self.scroll.jump_to(&desc, unbound_math::Vec2::new(0.0, y));
        } else if y + self.row_height() > self.scroll_offset() + self.viewport_height {
            self.scroll.jump_to(
                &desc,
                unbound_math::Vec2::new(0.0, y + self.row_height() - self.viewport_height),
            );
        }
    }

    /// The row height the virtualiser uses.
    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.tree.row_height()
    }

    /// Viewport height the last [`Self::rows`] call used.
    pub fn set_viewport_height(&mut self, height: f32) {
        self.viewport_height = height.max(0.0);
    }

    /// The instantiated rows for one frame.
    ///
    /// Only slots inside (and just around) the viewport are returned, which
    /// is the constant-entity guarantee of the virtualiser.
    pub fn rows(&mut self, desc: &TreeViewDesc) -> Vec<TreeViewRow> {
        self.set_viewport_height(desc.rect.size.y);
        self.last_window = self.tree.update(self.scroll_offset(), desc.rect.size.y);
        let mut out = Vec::with_capacity(self.tree.slot_count());
        for slot in 0..self.tree.slot_count() {
            let (Some(row), Some(item)) = (self.tree.row_at(slot), self.tree.slot_item(slot))
            else {
                continue;
            };
            let y = desc.rect.origin.y + self.tree.item_y(item) - self.scroll_offset();
            let rect = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, desc.row_height);
            if rect.max().y < desc.rect.origin.y || rect.origin.y > desc.rect.max().y {
                continue;
            }
            out.push(TreeViewRow { row, item, rect });
        }
        out
    }

    /// The total content height of the flattened rows.
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.order.len() as f32 * self.row_height()
    }

    /// Scroll by a wheel delta.
    pub fn scroll_by(&mut self, desc: &TreeViewDesc, delta: f32) -> bool {
        let scroll_desc = desc.scroll_desc(self.order.len());
        self.scroll
            .scroll_by(&scroll_desc, unbound_math::Vec2::new(0.0, delta));
        self.scroll.is_animating()
    }

    /// Step the scroll spring. Returns whether it still moves.
    pub fn advance(&mut self, desc: &TreeViewDesc, dt: f32) -> bool {
        let scroll_desc = desc.scroll_desc(self.order.len());
        self.scroll.step(&scroll_desc, dt)
    }
}

/// The colour slots a tree view paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeViewTokens {
    /// The viewport background.
    pub background: Token,
    /// An idle row.
    pub row: Token,
    /// A hovered row.
    pub row_hover: Token,
    /// A selected row.
    pub row_selected: Token,
    /// The text of an idle row.
    pub text: Token,
    /// The text of a selected row.
    pub text_selected: Token,
    /// The indent guides.
    pub guide: Token,
    /// The expand arrow.
    pub arrow: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl TreeViewTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            background: Token::SURFACE,
            row: Token::NONE,
            row_hover: Token::CONTROL_HOVER,
            row_selected: Token::PRIMARY_SOFT,
            text: Token::TEXT_PRIMARY,
            text_selected: Token::PRIMARY,
            guide: Token::DIVIDER,
            arrow: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// Emit one tree row: indent guides, expansion arrow, label and selection.
#[must_use]
pub fn emit_tree_row(
    desc: &TreeViewDesc,
    row: &TreeViewRow,
    selected: bool,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TreeViewTokens,
) -> WidgetOutput {
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(6 + label.len() + row.row.depth as usize);
    let mut payloads = Vec::with_capacity(label.len());
    let rect = row.rect;

    let fill = if selected {
        tokens.row_selected
    } else if control.phase == PointerPhase::Hovered {
        tokens.row_hover
    } else {
        tokens.row
    };
    primitives.push(outlined(rect, 4.0, fill, Token::NONE, 0.0, 0.0, desc.z));

    // One guide per depth level, stopping short of the arrow column.
    let indent = desc.indent;
    for level in 0..row.row.depth as usize {
        let x = rect.origin.x + level as f32 * indent + indent * 0.5;
        let guide = Rect::new(x, rect.origin.y, 1.0, rect.size.y);
        primitives.push(instance(
            guide,
            [0.0; 4],
            tokens.guide,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }

    let arrow_x = rect.origin.x + row.row.depth as f32 * indent + 6.0;
    let centre = [arrow_x + 4.0, rect.origin.y + rect.size.y * 0.5];
    if row.row.has_children {
        // A right-pointing chevron; the expanded state rotates it down.
        let arm = 4.0;
        let (sx, sy, ex, ey) = if row.row.expanded {
            (
                centre[0] - arm,
                centre[1] - arm * 0.5,
                centre[0] + arm,
                centre[1] - arm * 0.5,
            )
        } else {
            (
                centre[0] - arm * 0.5,
                centre[1] - arm,
                centre[0] - arm * 0.5,
                centre[1] + arm,
            )
        };
        primitives.push(line([sx, sy], [ex, ey], 1.5, tokens.arrow, desc.z));
        if row.row.expanded {
            primitives.push(line(
                [centre[0] - arm, centre[1] + arm * 0.5],
                [centre[0] + arm, centre[1] + arm * 0.5],
                1.5,
                tokens.arrow,
                desc.z,
            ));
        }
    }

    glyphs(
        label,
        if selected {
            tokens.text_selected
        } else {
            tokens.text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        5.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(row.row.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::TreeItem, "", rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = selected;
    a11y.level = u32::from(row.row.depth) + 1;
    if row.row.has_children {
        a11y.expanded = Some(row.row.expanded);
    }

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    };
    if row.row.has_children && !disabled {
        let toggle_key = tree_toggle_node(row.row.id);
        let toggle_r = Rect::new(arrow_x - 2.0, rect.origin.y, 16.0, rect.size.y);
        out.with_sub_control(
            toggle_key,
            HitRegion::new(toggle_key, CONTROL_LAYER, u32::from(desc.z) + 1, toggle_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Toggle branch", toggle_r).clickable(),
            false,
        );
    }
    out
}

/// Deterministic child node key for the expand/collapse chevron of a tree row.
#[must_use]
pub const fn tree_toggle_node(row_id: u64) -> u64 {
    row_id ^ 0x5452_4545_0001
}

/// Emit the viewport background.
#[must_use]
pub fn emit_tree_background(desc: &TreeViewDesc, tokens: TreeViewTokens) -> WidgetOutput {
    WidgetOutput {
        primitives: vec![outlined(
            desc.rect,
            0.0,
            tokens.background,
            Token::NONE,
            0.0,
            0.0,
            desc.z,
        )],
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// A thin stroke between two points.
fn line(
    start: [f32; 2],
    end: [f32; 2],
    thickness: f32,
    color: Token,
    z: u16,
) -> unbound_gpu::data::GpuPrimitiveInstance {
    instance(
        Rect::new(
            start[0].min(end[0]),
            start[1].min(end[1]),
            (start[0] - end[0]).abs().max(thickness),
            (start[1] - end[1]).abs().max(thickness),
        ),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Content,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(id: u64) -> TreeRow {
        TreeRow {
            id,
            depth: 0,
            has_children: true,
            expanded: false,
        }
    }

    fn leaf(id: u64, depth: u16) -> TreeRow {
        TreeRow {
            id,
            depth,
            has_children: false,
            expanded: false,
        }
    }

    fn children(id: u64) -> Vec<TreeRow> {
        match id {
            1 => vec![leaf(11, 1), leaf(12, 1)],
            2 => vec![leaf(21, 1)],
            _ => Vec::new(),
        }
    }

    fn state() -> TreeViewState {
        let mut state = TreeViewState::new(TREE_ROW);
        state.rebuild(&[root(1), root(2)], children);
        state
    }

    #[test]
    fn a_collapsed_tree_flattens_to_its_roots() {
        let state = state();
        assert_eq!(state.row_count(), 2);
        assert_eq!(state.row(1).expect("root visible").depth, 0);
    }

    #[test]
    fn expanding_a_root_adds_its_children() {
        let mut state = state();
        assert!(state.set_expanded(1, true, &[root(1), root(2)], children));
        assert_eq!(state.row_count(), 4);
        assert_eq!(state.parent_of(11), Some(1));
        assert!(!state.set_expanded(1, true, &[root(1), root(2)], children));
    }

    #[test]
    fn only_visible_rows_are_instantiated() {
        let mut state = TreeViewState::new(TREE_ROW);
        // 10 000 leaves under one root.
        let kids: Vec<TreeRow> = (100..10_100).map(|id| leaf(id, 1)).collect();
        state.rebuild(
            &[root(1)],
            move |id| if id == 1 { kids.clone() } else { Vec::new() },
        );
        state.set_expanded(1, true, &[root(1)], move |id| {
            if id == 1 {
                (100..10_100).map(|id| leaf(id, 1)).collect()
            } else {
                Vec::new()
            }
        });
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = state.rows(&desc);
        assert!(
            rows.len() <= 40,
            "a 10k-row tree must instantiate a viewport budget, got {}",
            rows.len()
        );
        assert!(rows.len() >= desc.visible_capacity());
    }

    #[test]
    fn scrolling_moves_the_window_without_growing_it() {
        let mut state = TreeViewState::new(TREE_ROW);
        let kids: Vec<TreeRow> = (100..10_100).map(|id| leaf(id, 1)).collect();
        let roots = [root(1)];
        state.rebuild(&roots, |_| kids.clone());
        state.set_expanded(1, true, &roots, |_| kids.clone());
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let first = state.rows(&desc).len();
        state.scroll_by(&desc, 2600.0);
        state.scroll.jump_to(
            &desc.scroll_desc(state.row_count()),
            unbound_math::Vec2::new(0.0, 2600.0),
        );
        let moved = state.rows(&desc);
        assert!(
            moved.len() <= 40,
            "scrolling must not grow the live set, got {}",
            moved.len()
        );
        assert!(
            (moved.len() as isize - first as isize).abs() <= 1,
            "the window keeps roughly the same size"
        );
        assert!(moved[0].item > 0);
    }

    #[test]
    fn selection_replaces_or_extends() {
        let mut state = state();
        assert!(state.select(1, false));
        assert!(state.select(2, true));
        assert_eq!(state.selected.len(), 2);
        assert!(state.select(2, true));
        assert_eq!(state.selected.len(), 1);
        state.clear_selection();
        assert!(state.selected.is_empty());
    }

    #[test]
    fn focus_steps_clamp_at_the_ends() {
        let mut state = state();
        assert!(state.focus_step(1));
        assert_eq!(state.focused, Some(2));
        assert!(state.focus_step(10));
        assert_eq!(state.focused, Some(2));
    }

    #[test]
    fn arrow_left_collapses_then_jumps_to_the_parent() {
        let roots = [root(1), root(2)];
        let mut state = TreeViewState::new(TREE_ROW);
        state.rebuild(&roots, children);
        state.set_expanded(1, true, &roots, children);
        state.select(11, false);
        assert!(state.focus_parent_or_collapse(&roots, children));
        assert_eq!(state.focused, Some(1));
        assert!(state.focus_parent_or_collapse(&roots, children));
        assert!(!state.tree.is_expanded(1));
        assert!(!state.focus_parent_or_collapse(&roots, children));
    }

    #[test]
    fn arrow_right_expands_then_enters_the_first_child() {
        let mut state = state();
        let roots = [root(1), root(2)];
        state.select(1, false);
        assert!(state.expand_or_first_child(&roots, children));
        assert!(state.tree.is_expanded(1));
        assert!(state.expand_or_first_child(&roots, children));
        assert_eq!(state.focused, Some(11));
        assert!(!state.expand_or_first_child(&roots, children));
    }

    #[test]
    fn rename_opens_with_the_current_title_and_commits() {
        let mut state = state();
        assert!(state.begin_rename(1, "Project"));
        assert_eq!(state.rename_editor().expect("editor").text(), "Project");
        let (id, title) = state.finish_rename().expect("committed");
        assert_eq!(id, 1);
        assert_eq!(title, "Project");
        assert!(state.rename_editor().is_none());
        assert!(!state.begin_rename(999, "missing"));
    }

    #[test]
    fn a_row_is_pickable_and_reports_its_depth() {
        let mut state = state();
        let roots = [root(1), root(2)];
        state.set_expanded(1, true, &roots, children);
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = state.rows(&desc);
        let child = rows
            .iter()
            .find(|row| row.row.id == 11)
            .expect("child visible");
        let output = emit_tree_row(
            &desc,
            child,
            false,
            ControlState::default(),
            &[],
            TreeViewTokens::standard(),
        );
        let hit = output.hit.expect("a row is pickable");
        assert_eq!(hit.node, 11);
        let a11y = output.a11y.expect("a row is reachable");
        assert_eq!(a11y.role, Role::TreeItem);
        assert_eq!(a11y.level, 2);
    }

    #[test]
    fn an_expanded_row_reports_expansion() {
        let mut state = state();
        let roots = [root(1), root(2)];
        state.set_expanded(1, true, &roots, children);
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = state.rows(&desc);
        let parent = rows.iter().find(|row| row.row.id == 1).expect("root");
        let output = emit_tree_row(
            &desc,
            parent,
            true,
            ControlState::default(),
            &[],
            TreeViewTokens::standard(),
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.expanded, Some(true));
        assert!(a11y.selected);
    }

    #[test]
    fn a_selected_row_uses_the_selected_token() {
        let state = state();
        let desc = TreeViewDesc::new(Rect::new(0.0, 0.0, 300.0, 260.0));
        let rows = {
            let mut state = state;
            state.rows(&desc)
        };
        let tokens = TreeViewTokens::standard();
        let output = emit_tree_row(&desc, &rows[0], true, ControlState::default(), &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.row_selected.raw() as u32
        );
    }
}

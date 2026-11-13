//! The dock area: a tree of split and tabbed panels that can be rearranged.
//!
//! The tree is a flat arena — [`DockNode::Split`] holds two child indices and
//! [`DockNode::Tabs`] holds a tab list — so a drag never has to move a
//! recursive structure through the event loop. Splitting a panel clones its
//! node into a fresh arena slot and overwrites the old slot with the new
//! split, which keeps every existing child index valid.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::scroll::ScrollAxis;
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The height of a tab strip, in logical pixels.
pub const DOCK_TAB_BAR: f32 = 32.0;

/// The minimum share of a split either side may take while dragging.
const SPLIT_LIMIT: f32 = 0.1;

/// One dockable tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockTab {
    /// Stable identity, also used as the tab's semantic and hit node.
    pub id: u64,
    /// The visible title.
    pub title: String,
    /// Whether the tab can be closed.
    pub closable: bool,
}

impl DockTab {
    /// A closable tab.
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            closable: true,
        }
    }

    /// The same tab without a close affordance.
    #[must_use]
    pub fn pinned(mut self) -> Self {
        self.closable = false;
        self
    }
}

/// One node of the dock arena.
#[derive(Clone, Debug, PartialEq)]
pub enum DockNode {
    /// Two children separated by a draggable gutter.
    Split {
        /// `Horizontal` puts the children side by side.
        axis: ScrollAxis,
        /// The share owned by `first`.
        ratio: f32,
        /// The left/top child.
        first: usize,
        /// The right/bottom child.
        second: usize,
    },
    /// A group of tabs sharing one panel.
    Tabs {
        /// The tabs, left to right.
        tabs: Vec<DockTab>,
        /// The visible tab.
        active: usize,
    },
}

impl DockNode {
    /// The tab list when this is a `Tabs` node.
    #[must_use]
    pub fn tabs(&self) -> Option<&[DockTab]> {
        match self {
            Self::Tabs { tabs, .. } => Some(tabs),
            Self::Split { .. } => None,
        }
    }

    /// The active tab index when this is a `Tabs` node.
    #[must_use]
    pub fn active(&self) -> Option<usize> {
        match self {
            Self::Tabs { active, .. } => Some(*active),
            Self::Split { .. } => None,
        }
    }
}

/// A flat tree of dock panels.
#[derive(Clone, Debug, PartialEq)]
pub struct DockTree {
    /// Every node; child indices point into this vector.
    pub nodes: Vec<DockNode>,
    /// The root node index.
    pub root: usize,
    /// The gutter thickness of every split.
    pub handle: f32,
}

impl DockTree {
    /// A tree with a single tab group.
    #[must_use]
    pub fn new(tabs: Vec<DockTab>) -> Self {
        Self {
            nodes: vec![DockNode::Tabs { tabs, active: 0 }],
            root: 0,
            handle: 8.0,
        }
    }

    /// The node at `index`, if it exists.
    #[must_use]
    pub fn node(&self, index: usize) -> Option<&DockNode> {
        self.nodes.get(index)
    }

    /// The node at `index`, mutably.
    pub fn node_mut(&mut self, index: usize) -> Option<&mut DockNode> {
        self.nodes.get_mut(index)
    }

    /// Every tab group, in arena order.
    #[must_use]
    pub fn tab_groups(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| matches!(node, DockNode::Tabs { .. }).then_some(index))
            .collect()
    }

    /// Locate a tab by identity.
    #[must_use]
    pub fn find_tab(&self, id: u64) -> Option<(usize, usize)> {
        for (index, node) in self.nodes.iter().enumerate() {
            if let DockNode::Tabs { tabs, .. } = node {
                if let Some(position) = tabs.iter().position(|tab| tab.id == id) {
                    return Some((index, position));
                }
            }
        }
        None
    }

    /// The panel rectangle of every tab group.
    #[must_use]
    pub fn layout(&self, rect: Rect) -> Vec<DockPane> {
        let mut out = Vec::new();
        self.layout_into(self.root, rect, &mut out);
        out
    }

    fn layout_into(&self, index: usize, rect: Rect, out: &mut Vec<DockPane>) {
        match self.nodes.get(index) {
            Some(DockNode::Tabs { tabs, active }) => out.push(DockPane {
                node: index,
                rect,
                tabs: tabs.clone(),
                active: *active,
            }),
            Some(DockNode::Split {
                axis,
                ratio,
                first,
                second,
            }) => {
                let horizontal = !matches!(axis, ScrollAxis::Vertical);
                let length = if horizontal { rect.size.x } else { rect.size.y };
                let available = (length - self.handle).max(0.0);
                let ratio = ratio.clamp(SPLIT_LIMIT, 1.0 - SPLIT_LIMIT);
                let first_len = available * ratio;
                let (first_rect, second_rect) = if horizontal {
                    (
                        Rect::new(rect.origin.x, rect.origin.y, first_len, rect.size.y),
                        Rect::new(
                            rect.origin.x + first_len + self.handle,
                            rect.origin.y,
                            (available - first_len).max(0.0),
                            rect.size.y,
                        ),
                    )
                } else {
                    (
                        Rect::new(rect.origin.x, rect.origin.y, rect.size.x, first_len),
                        Rect::new(
                            rect.origin.x,
                            rect.origin.y + first_len + self.handle,
                            rect.size.x,
                            (available - first_len).max(0.0),
                        ),
                    )
                };
                self.layout_into(*first, first_rect, out);
                self.layout_into(*second, second_rect, out);
            }
            _ => {}
        }
    }

    /// Split the target panel and put `tabs` into the new half.
    ///
    /// `Center` merges the tabs into the target group instead. Returns the
    /// index of the new tab group, or `None` when the target is not a panel.
    pub fn insert(&mut self, target: usize, zone: DockZone, tabs: Vec<DockTab>) -> Option<usize> {
        let node = self.nodes.get(target)?.clone();
        if !matches!(node, DockNode::Tabs { .. }) {
            return None;
        }
        if zone == DockZone::Center {
            if let Some(DockNode::Tabs {
                tabs: existing,
                active,
            }) = self.nodes.get_mut(target)
            {
                if !tabs.is_empty() {
                    *active = existing.len();
                }
                existing.extend(tabs);
            }
            return Some(target);
        }
        let existing = self.nodes.len();
        self.nodes.push(node);
        let fresh = self.nodes.len();
        self.nodes.push(DockNode::Tabs { tabs, active: 0 });
        let (axis, first, second) = match zone {
            DockZone::Left | DockZone::Top => (zone.axis(), fresh, existing),
            DockZone::Right | DockZone::Bottom => (zone.axis(), existing, fresh),
            DockZone::Center => unreachable!("centre merges above"),
        };
        self.nodes[target] = DockNode::Split {
            axis,
            ratio: 0.5,
            first,
            second,
        };
        Some(fresh)
    }

    /// Move a tab to a target panel and zone.
    ///
    /// A tab dropped on its own group in `Center` stays put; a tab moved to a
    /// new group is removed from its old one. Empty groups are left in place:
    /// a panel that vanishes under the cursor is worse than an empty one that
    /// can receive another tab.
    pub fn move_tab(&mut self, id: u64, target: usize, zone: DockZone) -> bool {
        let Some((source, position)) = self.find_tab(id) else {
            return false;
        };
        if source == target && zone == DockZone::Center {
            return false;
        }
        let Some(DockNode::Tabs { tabs, active }) = self.nodes.get_mut(source) else {
            return false;
        };
        if position >= tabs.len() {
            return false;
        }
        let tab = tabs.remove(position);
        if *active >= tabs.len() && !tabs.is_empty() {
            *active = tabs.len() - 1;
        }
        self.insert(target, zone, vec![tab]).is_some()
    }

    /// Remove a tab. Returns whether the tab existed.
    pub fn close_tab(&mut self, id: u64) -> bool {
        let Some((group, position)) = self.find_tab(id) else {
            return false;
        };
        let Some(DockNode::Tabs { tabs, active }) = self.nodes.get_mut(group) else {
            return false;
        };
        tabs.remove(position);
        if *active >= tabs.len() {
            *active = tabs.len().saturating_sub(1);
        }
        true
    }

    /// Apply a drag ratio to a split node.
    pub fn set_split_ratio(&mut self, index: usize, ratio: f32) -> bool {
        let Some(DockNode::Split { ratio: slot, .. }) = self.nodes.get_mut(index) else {
            return false;
        };
        *slot = ratio.clamp(SPLIT_LIMIT, 1.0 - SPLIT_LIMIT);
        true
    }
}

/// One laid-out panel.
#[derive(Clone, Debug, PartialEq)]
pub struct DockPane {
    /// The arena index of the tab group.
    pub node: usize,
    /// The panel rectangle.
    pub rect: Rect,
    /// The tabs in the group.
    pub tabs: Vec<DockTab>,
    /// The visible tab.
    pub active: usize,
}

impl DockPane {
    /// The tab strip inside the panel.
    #[must_use]
    pub fn tab_bar(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            DOCK_TAB_BAR.min(self.rect.size.y),
        )
    }

    /// The content area under the tab strip.
    #[must_use]
    pub fn content(&self) -> Rect {
        let bar = self.tab_bar();
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + bar.size.y,
            self.rect.size.x,
            (self.rect.size.y - bar.size.y).max(0.0),
        )
    }

    /// The rectangle of one tab, clipped to the strip.
    #[must_use]
    pub fn tab_rect(&self, index: usize, width: f32) -> Rect {
        let bar = self.tab_bar();
        Rect::new(
            bar.origin.x + index as f32 * width,
            bar.origin.y,
            width.min((bar.size.x - index as f32 * width).max(0.0)),
            bar.size.y,
        )
    }

    /// The close affordance of one tab.
    #[must_use]
    pub fn close_rect(&self, index: usize, width: f32) -> Rect {
        let tab = self.tab_rect(index, width);
        let size = (tab.size.y * 0.45).max(10.0);
        Rect::new(
            tab.max().x - size - 8.0,
            tab.origin.y + (tab.size.y - size) * 0.5,
            size,
            size,
        )
    }

    /// The visible tab, if the group has one.
    #[must_use]
    pub fn active_tab(&self) -> Option<&DockTab> {
        self.tabs.get(self.active)
    }
}

/// Where a dragged tab would land inside a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockZone {
    /// Split the panel and place the tab above.
    Top,
    /// Split the panel and place the tab below.
    Bottom,
    /// Split the panel and place the tab on the left.
    Left,
    /// Split the panel and place the tab on the right.
    Right,
    /// Merge into the panel's tab group.
    Center,
}

impl DockZone {
    /// The split axis this zone produces.
    #[must_use]
    pub fn axis(self) -> ScrollAxis {
        match self {
            Self::Left | Self::Right => ScrollAxis::Horizontal,
            Self::Top | Self::Bottom => ScrollAxis::Vertical,
            Self::Center => ScrollAxis::Both,
        }
    }

    /// Classify a pointer position inside a panel.
    ///
    /// The outer quarter of each side is a split zone; the middle is the
    /// merge zone. The closest edge wins, so a pointer near a corner still
    /// picks the more deliberate of the two directions.
    #[must_use]
    pub fn from_pointer(rect: Rect, pointer: Vec2) -> Self {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 || !rect.contains_point(pointer) {
            return Self::Center;
        }
        let nx = (pointer.x - rect.origin.x) / rect.size.x;
        let ny = (pointer.y - rect.origin.y) / rect.size.y;
        let left = nx;
        let right = 1.0 - nx;
        let top = ny;
        let bottom = 1.0 - ny;
        let mut best = Self::Center;
        let mut distance = 0.25;
        if left < distance {
            distance = left;
            best = Self::Left;
        }
        if right < distance {
            distance = right;
            best = Self::Right;
        }
        if top < distance {
            distance = top;
            best = Self::Top;
        }
        if bottom < distance {
            best = Self::Bottom;
        }
        best
    }

    /// The translucent preview this zone paints.
    #[must_use]
    pub fn preview(self, rect: Rect) -> Rect {
        match self {
            Self::Left => Rect::new(rect.origin.x, rect.origin.y, rect.size.x * 0.5, rect.size.y),
            Self::Right => Rect::new(
                rect.origin.x + rect.size.x * 0.5,
                rect.origin.y,
                rect.size.x * 0.5,
                rect.size.y,
            ),
            Self::Top => Rect::new(rect.origin.x, rect.origin.y, rect.size.x, rect.size.y * 0.5),
            Self::Bottom => Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.y * 0.5,
                rect.size.x,
                rect.size.y * 0.5,
            ),
            Self::Center => rect,
        }
    }
}

/// The in-flight tab drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockDrag {
    /// The dragged tab.
    pub tab: u64,
    /// The group the drag started from.
    pub from: usize,
    /// The current pointer position.
    pub pointer: Vec2,
    /// Whether the pointer has left every dock panel, which a caller may turn
    /// into a floating window.
    pub detached: bool,
}

/// The preview produced while a drag hovers a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockHover {
    /// The target group.
    pub target: usize,
    /// The zone inside it.
    pub zone: DockZone,
    /// The rectangle the preview paints.
    pub preview: Rect,
}

impl DockHover {
    /// Classify a pointer over the tree.
    #[must_use]
    pub fn resolve(tree: &DockTree, root: Rect, pointer: Vec2) -> Option<Self> {
        let pane = tree
            .layout(root)
            .into_iter()
            .find(|pane| pane.rect.contains_point(pointer))?;
        let zone = DockZone::from_pointer(pane.rect, pointer);
        Some(Self {
            target: pane.node,
            zone,
            preview: zone.preview(pane.rect),
        })
    }
}

/// The live state of a dock area.
#[derive(Clone, Debug, PartialEq)]
pub struct DockState {
    /// The panel tree.
    pub tree: DockTree,
    /// The active drag, if any.
    pub drag: Option<DockDrag>,
    /// The preview under the pointer, if any.
    pub hover: Option<DockHover>,
}

impl DockState {
    /// A state around the given tree.
    #[must_use]
    pub fn new(tree: DockTree) -> Self {
        Self {
            tree,
            drag: None,
            hover: None,
        }
    }

    /// Start dragging a tab.
    pub fn begin_drag(&mut self, tab: u64, from: usize, pointer: Vec2) {
        self.drag = Some(DockDrag {
            tab,
            from,
            pointer,
            detached: false,
        });
        self.hover = None;
    }

    /// Move the drag and recompute its preview.
    pub fn drag_to(&mut self, root: Rect, pointer: Vec2) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        drag.pointer = pointer;
        self.hover = DockHover::resolve(&self.tree, root, pointer);
        drag.detached = self.hover.is_none();
    }

    /// Finish the drag: move the tab to the previewed zone.
    ///
    /// Returns whether the tree changed.
    pub fn drop_drag(&mut self) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        let Some(hover) = self.hover.take() else {
            return false;
        };
        self.tree.move_tab(drag.tab, hover.target, hover.zone)
    }

    /// Abandon the drag without moving anything.
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.hover = None;
    }

    /// Whether a tab is being dragged.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Activate a tab inside a group.
    pub fn activate(&mut self, group: usize, index: usize) -> bool {
        let Some(DockNode::Tabs { tabs, active }) = self.tree.node_mut(group) else {
            return false;
        };
        if index >= tabs.len() || *active == index {
            return false;
        }
        *active = index;
        true
    }
}

/// The colour slots a dock area paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DockTokens {
    /// The panel background.
    pub panel: Token,
    /// The panel border.
    pub border: Token,
    /// The tab strip background.
    pub tab_bar: Token,
    /// An idle tab.
    pub tab: Token,
    /// A hovered tab.
    pub tab_hover: Token,
    /// The active tab.
    pub tab_active: Token,
    /// The text of an active tab.
    pub tab_text_active: Token,
    /// The text of an idle tab.
    pub tab_text: Token,
    /// The drop preview fill.
    pub preview: Token,
    /// The drop preview border.
    pub preview_border: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl DockTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            panel: Token::SURFACE,
            border: Token::OUTLINE,
            tab_bar: Token::HEADER_BG,
            tab: Token::NONE,
            tab_hover: Token::CONTROL_HOVER,
            tab_active: Token::CARD_BG,
            tab_text_active: Token::TEXT_PRIMARY,
            tab_text: Token::TEXT_SECONDARY,
            preview: Token::PRIMARY_SOFT,
            preview_border: Token::PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// Emit a panel frame: its surface, border and tab strip.
#[must_use]
pub fn emit_dock_pane(pane: &DockPane, tokens: DockTokens, z: u16) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3);
    primitives.push(outlined(
        pane.rect,
        0.0,
        tokens.panel,
        tokens.border,
        1.0,
        0.0,
        z,
    ));
    primitives.push(surface(pane.tab_bar(), 0.0, tokens.tab_bar, z));
    if pane.active_tab().is_some() {
        // The active tab is underlined in the accent colour, which is the
        // dock's only permanent state cue.
        let width = DOCK_TAB_BAR * 4.0;
        let rect = pane.tab_rect(pane.active, width);
        let line = Rect::new(rect.origin.x, rect.max().y - 2.0, rect.size.x, 2.0);
        primitives.push(instance(
            line,
            [0.0; 4],
            tokens.preview_border,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// Emit one dock tab.
///
/// The tab's identity is its own node: the tab strip is a list of controls,
/// not one control with sub-regions, so assistive technology can enumerate
/// and activate each tab independently.
#[must_use]
pub fn emit_dock_tab(
    desc: &DockPane,
    index: usize,
    width: f32,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: DockTokens,
    z: u16,
) -> WidgetOutput {
    let Some(tab) = desc.tabs.get(index) else {
        return WidgetOutput::default();
    };
    let rect = desc.tab_rect(index, width);
    let active = index == desc.active;
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    let fill = if active {
        tokens.tab_active
    } else if control.phase == PointerPhase::Hovered {
        tokens.tab_hover
    } else {
        tokens.tab
    };
    primitives.push(outlined(rect, 0.0, fill, Token::NONE, 0.0, 0.0, z));
    glyphs(
        label,
        if active {
            tokens.tab_text_active
        } else {
            tokens.tab_text
        },
        z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        2.0,
        tokens.focus_ring,
        2.0,
        z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(tab.id, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii: [0.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Tab, tab.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = active;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// Emit the translucent drop preview.
#[must_use]
pub fn emit_dock_preview(hover: DockHover, tokens: DockTokens, z: u16) -> WidgetOutput {
    let rect = hover.preview;
    WidgetOutput {
        primitives: vec![outlined(
            rect,
            4.0,
            tokens.preview,
            tokens.preview_border,
            2.0,
            0.0,
            z,
        )],
        ..WidgetOutput::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> DockTree {
        DockTree::new(vec![
            DockTab::new(1, "Explorer"),
            DockTab::new(2, "Search"),
            DockTab::new(3, "Source"),
        ])
    }

    #[test]
    fn a_single_group_fills_the_root() {
        let tree = tree();
        let panes = tree.layout(Rect::new(0.0, 0.0, 400.0, 300.0));
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].rect, Rect::new(0.0, 0.0, 400.0, 300.0));
        assert_eq!(panes[0].content().origin.y, DOCK_TAB_BAR);
    }

    #[test]
    fn splitting_a_panel_produces_two_panels_with_a_gutter() {
        let mut tree = tree();
        let new = tree
            .insert(0, DockZone::Right, vec![DockTab::new(9, "Preview")])
            .expect("split a tab group");
        assert_eq!(new, 2);
        let panes = tree.layout(Rect::new(0.0, 0.0, 408.0, 300.0));
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].rect.size.x, 200.0);
        assert_eq!(panes[1].rect.origin.x, 208.0);
        assert_eq!(panes[1].rect.size.x, 200.0);
        assert_eq!(panes[0].tabs.len(), 3);
        assert_eq!(panes[1].tabs[0].id, 9);
    }

    #[test]
    fn centre_drop_merges_into_the_target_group() {
        let mut tree = tree();
        let preview = tree
            .insert(0, DockZone::Bottom, vec![DockTab::new(9, "Preview")])
            .expect("split");
        assert!(tree.move_tab(3, preview, DockZone::Center));
        assert_eq!(
            tree.node(preview)
                .expect("group")
                .tabs()
                .expect("tabs")
                .len(),
            2
        );
        assert_eq!(tree.find_tab(3).expect("still present").0, preview);
    }

    #[test]
    fn moving_a_tab_to_another_panel_removes_it_from_the_source() {
        let mut tree = tree();
        let preview = tree
            .insert(0, DockZone::Bottom, vec![DockTab::new(9, "Preview")])
            .expect("split");
        // The split clones the original group into a fresh slot, so the source
        // group is no longer at index 0.
        let source = tree
            .tab_groups()
            .into_iter()
            .find(|index| *index != preview)
            .expect("source group");
        assert!(tree.move_tab(2, preview, DockZone::Center));
        let source_node = tree.node(source).expect("source");
        assert_eq!(source_node.tabs().expect("tabs").len(), 2);
        let target = tree.node(preview).expect("target");
        assert_eq!(target.tabs().expect("tabs").len(), 2);
    }

    #[test]
    fn a_tab_dropped_on_its_own_group_stays_put() {
        let mut tree = tree();
        assert!(!tree.move_tab(1, 0, DockZone::Center));
        assert_eq!(tree.nodes[0].tabs().expect("tabs").len(), 3);
    }

    #[test]
    fn closing_a_tab_keeps_the_active_index_valid() {
        let mut tree = tree();
        tree.nodes[0] = DockNode::Tabs {
            tabs: vec![DockTab::new(1, "A"), DockTab::new(2, "B")],
            active: 1,
        };
        assert!(tree.close_tab(2));
        assert_eq!(tree.node(0).expect("group").active(), Some(0));
        assert!(!tree.close_tab(99));
    }

    #[test]
    fn the_zone_detection_picks_the_nearest_edge() {
        let rect = Rect::new(0.0, 0.0, 400.0, 400.0);
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(10.0, 200.0)),
            DockZone::Left
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(390.0, 200.0)),
            DockZone::Right
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(200.0, 10.0)),
            DockZone::Top
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(200.0, 390.0)),
            DockZone::Bottom
        );
        assert_eq!(
            DockZone::from_pointer(rect, Vec2::new(200.0, 200.0)),
            DockZone::Center
        );
    }

    #[test]
    fn the_preview_covers_the_half_the_tab_would_occupy() {
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert_eq!(
            DockZone::Left.preview(rect),
            Rect::new(0.0, 0.0, 200.0, 300.0)
        );
        assert_eq!(
            DockZone::Bottom.preview(rect),
            Rect::new(0.0, 150.0, 400.0, 150.0)
        );
        assert_eq!(DockZone::Center.preview(rect), rect);
    }

    #[test]
    fn a_drag_recomputes_the_preview_and_drops_the_tab() {
        let mut state = DockState::new(tree());
        let root = Rect::new(0.0, 0.0, 400.0, 300.0);
        state.begin_drag(2, 0, Vec2::new(60.0, 10.0));
        state.drag_to(root, Vec2::new(390.0, 150.0));
        let hover = state.hover.expect("a preview inside the panel");
        assert_eq!(hover.zone, DockZone::Right);
        assert!(state.drop_drag());
        assert!(!state.is_dragging());
        let panes = state.tree.layout(root);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[1].tabs[0].id, 2);
    }

    #[test]
    fn dragging_outside_every_panel_marks_the_drag_detached() {
        let mut state = DockState::new(tree());
        let root = Rect::new(0.0, 0.0, 400.0, 300.0);
        state.begin_drag(1, 0, Vec2::new(10.0, 10.0));
        state.drag_to(root, Vec2::new(900.0, 900.0));
        assert!(state.drag.expect("drag").detached);
        assert!(!state.drop_drag(), "a detached drop moves nothing");
    }

    #[test]
    fn a_tab_is_pickable_and_exposes_the_tab_role() {
        let tree = tree();
        let pane = tree.layout(Rect::new(0.0, 0.0, 400.0, 300.0)).remove(0);
        let tokens = DockTokens::standard();
        let output = emit_dock_tab(
            &pane,
            1,
            DOCK_TAB_BAR * 4.0,
            ControlState::default(),
            &[],
            tokens,
            10,
        );
        let hit = output.hit.expect("a tab is pickable");
        assert_eq!(hit.node, 2);
        let a11y = output.a11y.expect("a tab is reachable");
        assert_eq!(a11y.role, Role::Tab);
        assert_eq!(a11y.label, "Search");
        assert!(!a11y.selected);
    }

    #[test]
    fn the_active_tab_is_selected() {
        let tree = tree();
        let pane = tree.layout(Rect::new(0.0, 0.0, 400.0, 300.0)).remove(0);
        let output = emit_dock_tab(
            &pane,
            0,
            DOCK_TAB_BAR * 4.0,
            ControlState::default(),
            &[],
            DockTokens::standard(),
            10,
        );
        assert!(output.a11y.expect("reachable").selected);
    }

    #[test]
    fn the_split_ratio_is_clamped() {
        let mut tree = tree();
        tree.insert(0, DockZone::Right, vec![DockTab::new(9, "P")]);
        assert!(tree.set_split_ratio(0, 0.0));
        let DockNode::Split { ratio, .. } = tree.node(0).expect("split") else {
            panic!("root became a split");
        };
        assert_eq!(*ratio, SPLIT_LIMIT);
    }
}

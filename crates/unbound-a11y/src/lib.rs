//! Incremental accessibility semantics (`docs` §6.4).
//!
//! AccessKit is the *only* semantic output of the engine. There is no second
//! bespoke AST for agents to learn: the same tree that Windows UI Automation
//! consumes is the tree an agent introspects, which is exactly the "双模赋能"
//! the doc asks for.
//!
//! The interesting constraint is *incrementality*. A 100 000 node canvas cannot
//! rebuild a `TreeUpdate` per frame; AccessKit's contract is that an update only
//! carries nodes that are new or changed. So this module is a dirty-set:
//!
//! * writing a node marks it dirty,
//! * changing a child list marks the *parent* dirty (AccessKit requires the
//!   parent to be re-sent with the new `children` vector),
//! * removing a subtree records the parent so the removal is visible,
//! * focus is echoed on every update because the protocol demands it.
//!
//! The first update after [`Semantics::new`] is a full snapshot; everything
//! after that is a delta.

#![forbid(unsafe_op_in_unsafe_fn)]

use accesskit::{Action, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_math::Rect;

/// Actions a semantic node can expose.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum A11yAction {
    Click,
    Focus,
    Blur,
    SetValue,
    Increment,
    Decrement,
    Expand,
    Collapse,
    ScrollIntoView,
    ShowContextMenu,
}

impl A11yAction {
    /// The AccessKit equivalent.
    #[must_use]
    pub fn to_accesskit(self) -> Action {
        match self {
            Self::Click => Action::Click,
            Self::Focus => Action::Focus,
            Self::Blur => Action::Blur,
            Self::SetValue => Action::SetValue,
            Self::Increment => Action::Increment,
            Self::Decrement => Action::Decrement,
            Self::Expand => Action::Expand,
            Self::Collapse => Action::Collapse,
            Self::ScrollIntoView => Action::ScrollIntoView,
            Self::ShowContextMenu => Action::ShowContextMenu,
        }
    }

    /// Short name, used by the structured error output of the headless harness.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Focus => "focus",
            Self::Blur => "blur",
            Self::SetValue => "set-value",
            Self::Increment => "increment",
            Self::Decrement => "decrement",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::ScrollIntoView => "scroll-into-view",
            Self::ShowContextMenu => "show-context-menu",
        }
    }
}

/// A compact set of [`A11yAction`]s.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct A11yActions(u16);

impl A11yActions {
    pub const NONE: Self = Self(0);

    #[must_use]
    pub fn single(action: A11yAction) -> Self {
        let mut set = Self::NONE;
        set.insert(action);
        set
    }

    pub fn insert(&mut self, action: A11yAction) {
        self.0 |= 1 << (action as u16);
    }

    pub fn remove(&mut self, action: A11yAction) {
        self.0 &= !(1 << (action as u16));
    }

    #[must_use]
    pub fn contains(self, action: A11yAction) -> bool {
        self.0 & (1 << (action as u16)) != 0
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Iterate the actions in declaration order (deterministic).
    pub fn iter(self) -> impl Iterator<Item = A11yAction> {
        const ALL: [A11yAction; 10] = [
            A11yAction::Click,
            A11yAction::Focus,
            A11yAction::Blur,
            A11yAction::SetValue,
            A11yAction::Increment,
            A11yAction::Decrement,
            A11yAction::Expand,
            A11yAction::Collapse,
            A11yAction::ScrollIntoView,
            A11yAction::ShowContextMenu,
        ];
        ALL.into_iter().filter(move |a| self.contains(*a))
    }
}

impl FromIterator<A11yAction> for A11yActions {
    fn from_iter<T: IntoIterator<Item = A11yAction>>(iter: T) -> Self {
        let mut set = Self::NONE;
        for action in iter {
            set.insert(action);
        }
        set
    }
}

/// One semantic node. A plain value: the caller (a reducer) owns it.
#[derive(Clone, Debug, PartialEq)]
pub struct A11yNode {
    pub role: Role,
    pub label: String,
    pub description: String,
    pub value: String,
    /// Screen-space bounds, in physical pixels.
    pub bounds: Rect,
    pub disabled: bool,
    pub selected: bool,
    /// `Some` only for expandable things.
    pub expanded: Option<bool>,
    pub focusable: bool,
    /// Heading level / tree depth; `0` when not applicable.
    pub level: u32,
    pub actions: A11yActions,
}

impl Default for A11yNode {
    fn default() -> Self {
        Self {
            role: Role::Unknown,
            label: String::new(),
            description: String::new(),
            value: String::new(),
            bounds: Rect::ZERO,
            disabled: false,
            selected: false,
            expanded: None,
            focusable: false,
            level: 0,
            actions: A11yActions::NONE,
        }
    }
}

impl A11yNode {
    #[must_use]
    pub fn new(role: Role, label: impl Into<String>, bounds: Rect) -> Self {
        Self {
            role,
            label: label.into(),
            bounds,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_actions(mut self, actions: A11yActions) -> Self {
        self.actions = actions;
        self
    }

    #[must_use]
    pub fn clickable(mut self) -> Self {
        self.actions.insert(A11yAction::Click);
        self.actions.insert(A11yAction::Focus);
        self.focusable = true;
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Convert to the AccessKit representation, filling in `children`.
    #[must_use]
    pub fn to_accesskit(&self, children: &[u64]) -> Node {
        let mut node = Node::new(self.role);
        node.set_bounds(accesskit::Rect::from_origin_size(
            accesskit::Point::new(
                f64::from(self.bounds.origin.x),
                f64::from(self.bounds.origin.y),
            ),
            accesskit::Size::new(
                f64::from(self.bounds.size.x.max(0.0)),
                f64::from(self.bounds.size.y.max(0.0)),
            ),
        ));
        if !self.label.is_empty() {
            node.set_label(self.label.clone());
        }
        if !self.description.is_empty() {
            node.set_description(self.description.clone());
        }
        if !self.value.is_empty() {
            node.set_value(self.value.clone());
        }
        if self.disabled {
            node.set_disabled();
        }
        if self.selected {
            node.set_selected(true);
        }
        if let Some(expanded) = self.expanded {
            node.set_expanded(expanded);
        }
        if self.focusable {
            // AccessKit 0.24 has no explicit `focusable` flag: a node is
            // focusable when it advertises the `Focus` action.
            node.add_action(Action::Focus);
        }
        if self.level > 0 {
            node.set_level(self.level as usize);
        }
        if !children.is_empty() {
            let ids: Vec<NodeId> = children.iter().map(|id| NodeId(*id)).collect();
            node.set_children(ids);
        }
        for action in self.actions.iter() {
            node.add_action(action.to_accesskit());
        }
        node
    }
}

/// The incremental semantic tree.
///
/// Cloning is a full snapshot: an application that rebuilds its semantic tree
/// from scratch every frame hands a copy to the scene collector while keeping
/// the original inspectable.
#[derive(Clone, Debug)]
pub struct Semantics {
    nodes: FxHashMap<u64, A11yNode>,
    children: FxHashMap<u64, Vec<u64>>,
    parents: FxHashMap<u64, u64>,
    root: u64,
    focus: u64,
    dirty: FxHashSet<u64>,
    removed: FxHashSet<u64>,
    structural: FxHashSet<u64>,
    initialized: bool,
    updates: u64,
}

impl Default for Semantics {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Semantics {
    /// Create a tree with a root node id. The root always exists and is the
    /// window node.
    #[must_use]
    pub fn new(root: u64) -> Self {
        Self {
            nodes: FxHashMap::default(),
            children: FxHashMap::default(),
            parents: FxHashMap::default(),
            root,
            focus: root,
            dirty: FxHashSet::default(),
            removed: FxHashSet::default(),
            structural: FxHashSet::default(),
            initialized: false,
            updates: 0,
        }
    }

    #[must_use]
    pub fn root(&self) -> u64 {
        self.root
    }

    #[must_use]
    pub fn focus(&self) -> u64 {
        self.focus
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
    pub fn get(&self, id: u64) -> Option<&A11yNode> {
        self.nodes.get(&id)
    }

    #[must_use]
    pub fn children_of(&self, id: u64) -> &[u64] {
        self.children.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Insert or update a node. Marks it dirty; returns `true` when this is new.
    pub fn upsert(&mut self, id: u64, node: A11yNode) -> bool {
        let is_new = self.nodes.insert(id, node).is_none();
        self.dirty.insert(id);
        if is_new && id != self.root {
            // A new node without a parent would violate the protocol; attach it
            // to the root so the tree stays well formed.
            self.children.entry(self.root).or_default().push(id);
            self.parents.insert(id, self.root);
            self.structural.insert(self.root);
        }
        is_new
    }

    /// Reset the tree for a new frame, keeping every allocation.
    ///
    /// The root stays the same; the next update is a full snapshot again.
    /// Reusing the maps is what keeps a steady-state frame from allocating a
    /// new semantic tree per frame.
    pub fn reset(&mut self, root: u64) {
        self.nodes.clear();
        self.children.clear();
        self.parents.clear();
        self.dirty.clear();
        self.removed.clear();
        self.structural.clear();
        self.root = root;
        self.focus = root;
        self.initialized = false;
    }

    /// Replace the child list of `parent`, reparenting the children as needed.
    pub fn set_children(&mut self, parent: u64, children: &[u64]) {
        let previous = self.children.get(&parent).cloned().unwrap_or_default();
        for old in &previous {
            if !children.contains(old) {
                self.parents.remove(old);
            }
        }
        for child in children {
            // Moving a node between parents must remove it from the old list:
            // AccessKit readers would otherwise see the child twice.
            if let Some(old) = self.parents.get(child).copied() {
                if old != parent {
                    if let Some(list) = self.children.get_mut(&old) {
                        list.retain(|c| c != child);
                    }
                    self.dirty.insert(old);
                    self.structural.insert(old);
                }
            }
            self.parents.insert(*child, parent);
        }
        self.children.insert(parent, children.to_vec());
        self.dirty.insert(parent);
        self.structural.insert(parent);
    }

    /// Move focus. Always echoed in the next update.
    pub fn set_focus(&mut self, id: u64) {
        if self.focus == id {
            return;
        }
        let previous = self.focus;
        self.focus = id;
        self.dirty.insert(previous);
        self.dirty.insert(id);
    }

    /// Remove a node and its whole subtree.
    pub fn remove(&mut self, id: u64) {
        let parent = self.parent_of(id);
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if let Some(children) = self.children.remove(&current) {
                stack.extend(children);
            }
            self.nodes.remove(&current);
            self.dirty.remove(&current);
            self.structural.remove(&current);
            self.removed.insert(current);
            self.parents.remove(&current);
        }
        if let Some(parent) = parent {
            if let Some(children) = self.children.get_mut(&parent) {
                children.retain(|child| *child != id);
            }
            self.structural.insert(parent);
        }
        if self.focus == id {
            self.focus = self.root;
        }
    }

    #[must_use]
    pub fn parent_of(&self, id: u64) -> Option<u64> {
        self.parents.get(&id).copied()
    }

    /// Number of nodes the next update will carry. Zero means "nothing to send".
    #[must_use]
    pub fn pending(&self) -> usize {
        if !self.initialized {
            return self.nodes.len();
        }
        let mut count = 0usize;
        for id in &self.dirty {
            if self.nodes.contains_key(id) {
                count += 1;
            }
        }
        for id in &self.structural {
            if self.nodes.contains_key(id) && !self.dirty.contains(id) {
                count += 1;
            }
        }
        count
    }

    /// Produce the next `TreeUpdate`.
    ///
    /// Returns `None` when nothing changed, which lets the caller skip the
    /// platform call entirely — the "静止态 0% CPU" requirement starts here.
    pub fn take_update(&mut self) -> Option<TreeUpdate> {
        if self.initialized && self.dirty.is_empty() && self.structural.is_empty() {
            return None;
        }
        self.updates += 1;
        let first = !self.initialized;
        self.initialized = true;

        let mut ids: Vec<u64> = if first {
            self.nodes.keys().copied().collect()
        } else {
            let mut ids: Vec<u64> = self.dirty.iter().copied().collect();
            ids.extend(self.structural.iter().copied());
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        // A removed child must not be re-sent, even if a stale dirty mark
        // survived: AccessKit rejects updates for nodes that no longer exist.
        ids.retain(|id| self.nodes.contains_key(id));
        // Root first when present: readers build the tree from the top down.
        ids.sort_unstable_by_key(|id| (*id != self.root, *id));

        let mut nodes = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(node) = self.nodes.get(&id) {
                let children = self.children_of(id);
                nodes.push((NodeId(id), node.to_accesskit(children)));
            }
        }
        self.dirty.clear();
        self.structural.clear();
        self.removed.clear();

        let tree = if first {
            let mut tree = Tree::new(NodeId(self.root));
            tree.toolkit_name = Some("Unbound UI".to_owned());
            tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
            Some(tree)
        } else {
            None
        };

        Some(TreeUpdate {
            nodes,
            tree,
            tree_id: TreeId::ROOT,
            focus: NodeId(self.focus),
        })
    }

    /// Semantic path of a node, e.g. `Window/Panel[2]/Button "Save"`.
    ///
    /// The headless harness prints this when a layout sanitizer fires, so an
    /// agent can locate the offending node without a second introspection API.
    #[must_use]
    pub fn path(&self, id: u64) -> String {
        let mut segments = Vec::new();
        let mut current = Some(id);
        let mut guard = 0;
        while let Some(node_id) = current {
            guard += 1;
            if guard > 64 {
                segments.push("...".to_owned());
                break;
            }
            let segment = match self.nodes.get(&node_id) {
                Some(node) => {
                    let role = format!("{:?}", node.role);
                    if node.label.is_empty() {
                        role
                    } else {
                        format!("{role} \"{}\"", node.label)
                    }
                }
                None => format!("#{node_id}"),
            };
            segments.push(segment);
            current = self.parent_of(node_id);
        }
        segments.reverse();
        segments.join("/")
    }

    /// Number of updates produced so far, for the KPI harness.
    #[must_use]
    pub fn updates(&self) -> u64 {
        self.updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn tree() -> Semantics {
        let mut s = Semantics::new(1);
        s.upsert(
            1,
            A11yNode::new(Role::Window, "Demo", rect(0.0, 0.0, 800.0, 600.0)),
        );
        s
    }

    #[test]
    fn the_first_update_is_a_full_snapshot() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(10.0, 10.0, 80.0, 30.0)).clickable(),
        );
        s.set_children(1, &[2]);
        let update = s.take_update().unwrap();
        assert_eq!(update.nodes.len(), 2);
        assert!(
            update.tree.is_some(),
            "the first update must carry the tree"
        );
        assert_eq!(update.focus, NodeId(1));
        let button = update.nodes.iter().find(|(id, _)| id.0 == 2).unwrap();
        assert_eq!(button.1.role(), Role::Button);
        assert!(button.1.supports_action(Action::Click));
    }

    #[test]
    fn a_second_update_only_carries_the_delta() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2]);
        s.take_update();

        s.upsert(
            3,
            A11yNode::new(Role::Button, "Cancel", rect(20.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2, 3]);
        let update = s.take_update().unwrap();
        let mut ids: Vec<u64> = update.nodes.iter().map(|(id, _)| id.0).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 3], "only the parent and the new node");
        assert!(update.tree.is_none());
    }

    #[test]
    fn focus_is_echoed_even_when_nothing_else_changed() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2]);
        s.take_update();
        s.set_focus(2);
        let update = s.take_update().unwrap();
        assert_eq!(update.focus, NodeId(2));
        assert!(update.nodes.iter().any(|(id, _)| id.0 == 2));
    }

    #[test]
    fn no_change_means_no_update() {
        let mut s = tree();
        s.take_update();
        assert!(s.take_update().is_none());
        assert_eq!(s.updates(), 1);
    }

    #[test]
    fn a_removed_child_updates_its_parent() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2]);
        s.take_update();
        s.remove(2);
        let update = s.take_update().unwrap();
        assert_eq!(update.nodes.len(), 1, "only the parent is re-sent");
        assert_eq!(update.nodes[0].0, NodeId(1));
        assert!(update.nodes[0].1.children().is_empty());
    }

    #[test]
    fn removing_a_subtree_takes_the_descendants_with_it() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Group, "Panel", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.upsert(
            3,
            A11yNode::new(Role::Button, "Deep", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2]);
        s.set_children(2, &[3]);
        s.take_update();
        s.remove(2);
        assert!(s.get(2).is_none());
        assert!(s.get(3).is_none(), "descendant must be gone too");
        let update = s.take_update().unwrap();
        assert_eq!(update.nodes.len(), 1);
    }

    #[test]
    fn focus_falls_back_to_the_root_when_the_focused_node_dies() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2]);
        s.set_focus(2);
        s.remove(2);
        assert_eq!(s.focus(), 1);
    }

    #[test]
    fn actions_round_trip_through_accesskit() {
        let set: A11yActions = [
            A11yAction::Click,
            A11yAction::SetValue,
            A11yAction::ScrollIntoView,
        ]
        .into_iter()
        .collect();
        assert!(set.contains(A11yAction::Click));
        assert!(!set.contains(A11yAction::Expand));
        let names: Vec<&str> = set.iter().map(A11yAction::name).collect();
        assert_eq!(names, vec!["click", "set-value", "scroll-into-view"]);
        assert_eq!(A11yAction::Click.to_accesskit(), Action::Click);
        assert_eq!(A11yAction::Expand.to_accesskit(), Action::Expand);
    }

    #[test]
    fn node_properties_reach_the_platform_node() {
        let mut node = A11yNode::new(Role::CheckBox, "Remember me", rect(2.0, 3.0, 4.0, 5.0));
        node.description = "keeps you signed in".to_owned();
        node.value = "on".to_owned();
        node.selected = true;
        node.expanded = Some(false);
        node.level = 2;
        node.focusable = true;
        node.disabled = true;
        let out = node.to_accesskit(&[]);
        assert_eq!(out.label().unwrap(), "Remember me");
        assert_eq!(out.description().unwrap(), "keeps you signed in");
        assert_eq!(out.value().unwrap(), "on");
        assert_eq!(out.is_selected(), Some(true));
        assert!(out.is_disabled());
        assert_eq!(out.is_expanded(), Some(false));
        assert!(out.supports_action(Action::Focus));
        assert_eq!(out.level(), Some(2));
        assert!((out.bounds().unwrap().width() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn the_semantic_path_reads_root_first() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(
                Role::GenericContainer,
                "Toolbar",
                rect(0.0, 0.0, 10.0, 10.0),
            ),
        );
        s.upsert(
            3,
            A11yNode::new(Role::Button, "Save", rect(0.0, 0.0, 10.0, 10.0)),
        );
        s.set_children(1, &[2]);
        s.set_children(2, &[3]);
        assert_eq!(
            s.path(3),
            "Window \"Demo\"/GenericContainer \"Toolbar\"/Button \"Save\""
        );
    }

    #[test]
    fn an_orphan_node_is_attached_to_the_root() {
        let mut s = tree();
        s.upsert(
            9,
            A11yNode::new(Role::Button, "Loose", rect(0.0, 0.0, 1.0, 1.0)),
        );
        assert_eq!(s.parent_of(9), Some(1));
        assert_eq!(s.children_of(1), &[9]);
    }

    #[test]
    fn reparenting_keeps_the_parent_map_consistent() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::GenericContainer, "A", rect(0.0, 0.0, 1.0, 1.0)),
        );
        s.upsert(
            3,
            A11yNode::new(Role::GenericContainer, "B", rect(0.0, 0.0, 1.0, 1.0)),
        );
        s.upsert(
            4,
            A11yNode::new(Role::Button, "X", rect(0.0, 0.0, 1.0, 1.0)),
        );
        s.set_children(2, &[4]);
        assert_eq!(s.parent_of(4), Some(2));
        s.set_children(3, &[4]);
        assert_eq!(s.parent_of(4), Some(3));
        assert!(s.children_of(2).is_empty());
    }

    /// A thousand-node canvas where one label changes must send one node.
    #[test]
    fn a_single_label_change_costs_one_node() {
        let mut s = tree();
        let ids: Vec<u64> = (10..1010).collect();
        for id in &ids {
            s.upsert(
                *id,
                A11yNode::new(Role::Button, "row", rect(0.0, 0.0, 10.0, 10.0)),
            );
        }
        s.set_children(1, &ids);
        s.take_update();
        let mut node = A11yNode::new(Role::Button, "row 500", rect(0.0, 0.0, 10.0, 10.0));
        node.value = "500".to_owned();
        s.upsert(500, node);
        let update = s.take_update().unwrap();
        assert_eq!(update.nodes.len(), 1);
        assert_eq!(update.nodes[0].0, NodeId(500));
    }
}

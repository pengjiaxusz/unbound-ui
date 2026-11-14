//! 增量无障碍语义树（`docs` §6.4）。
//!
//! AccessKit 是引擎**唯一**的语义输出，不存在第二套供 Agent 学习的
//! 定制 AST：Windows UI Automation 消费的树与 Agent 自省的树完全一致，
//! 这正是文档所要求的“双模赋能”。
//!
//! 核心约束在于**增量性**。拥有 10 万个节点的画布无法每帧重建一次
//! `TreeUpdate`；AccessKit 契约规定每次更新仅携带新增或变更的节点。
//! 因此本模块本质上是一个脏集合（dirty-set）：
//!
//! * 写入节点会将其标记为脏；
//! * 修改子节点列表会将其**父节点**标记为脏（AccessKit 要求连同新的 `children`
//!   向量重新发送父节点）；
//! * 移除子树会记录其父节点，使移除操作对外可见；
//! * 每次更新都会回显焦点（focus），因为协议对此有明确要求。
//!
//!
//! [`Semantics::new`] 之后的首次更新为完整快照，此后均为增量更新。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use accesskit::{Action, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_math::Rect;

/// 语义节点可对外暴露的交互动作。
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
    /// 对应的 AccessKit 动作枚举。
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

    /// 简短名称，用于无头测试框架（headless harness）的结构化错误输出。
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

/// [`A11yAction`] 的紧凑集合。
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

    /// 按声明顺序迭代动作（顺序确定）。
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

/// 单个语义节点。纯值类型，由调用方（reducer）持有所有权。
#[derive(Clone, Debug, PartialEq)]
pub struct A11yNode {
    pub role: Role,
    pub label: String,
    pub description: String,
    pub value: String,
    /// 屏幕空间包围盒，单位为物理像素。
    pub bounds: Rect,
    pub disabled: bool,
    pub selected: bool,
    /// 仅对可展开元素为 `Some`。
    pub expanded: Option<bool>,
    pub focusable: bool,
    /// 标题层级或树深度；不适用时为 `0`。
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

    /// 转换为 AccessKit 表示形式，并填充 `children`。
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
            // AccessKit 0.24 没有显式的 `focusable` 标志：当节点声明了 `Focus`
            // 动作时即视为可聚焦。
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

/// 增量语义树。
///
/// 克隆操作会生成完整快照：每帧从头重建语义树的应用可将副本交给
/// 场景收集器，同时保留原树用于自省检查。
///
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
    /// 使用指定的根节点 ID 创建语义树。根节点始终存在，即窗口节点。
    ///
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

    /// 插入或更新节点。将其标记为脏；若为新建节点则返回 `true`。
    pub fn upsert(&mut self, id: u64, node: A11yNode) -> bool {
        let is_new = self.nodes.insert(id, node).is_none();
        self.dirty.insert(id);
        if is_new && id != self.root {
            // 没有父节点的新节点会违反协议；将其挂载到根节点下，以保持树结构合法。
            //
            self.children.entry(self.root).or_default().push(id);
            self.parents.insert(id, self.root);
            self.structural.insert(self.root);
        }
        is_new
    }

    /// 重置语义树以开始新一帧，同时保留所有已分配内存。
    ///
    /// 根节点保持不变；下一次更新将再次为完整快照。
    /// 复用内部映射表可确保稳态帧不会每帧分配新的语义树。
    ///
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

    /// 替换 `parent` 的子节点列表，并按需重新挂载子节点。
    pub fn set_children(&mut self, parent: u64, children: &[u64]) {
        let previous = self.children.get(&parent).cloned().unwrap_or_default();
        for old in &previous {
            if !children.contains(old) {
                self.parents.remove(old);
            }
        }
        for child in children {
            // 在父节点间移动节点时必须将其从旧列表中移除：
            // 否则 AccessKit 读取方会看到重复的子节点。
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

    /// 将单个子节点移动到新的父节点下。
    pub fn reparent(&mut self, child: u64, parent: u64) {
        if let Some(old) = self.parents.get(&child).copied() {
            if old != parent {
                if let Some(list) = self.children.get_mut(&old) {
                    list.retain(|c| *c != child);
                }
                self.dirty.insert(old);
                self.structural.insert(old);
            }
        }
        self.parents.insert(child, parent);
        let list = self.children.entry(parent).or_default();
        if !list.contains(&child) {
            list.push(child);
        }
        self.dirty.insert(parent);
        self.structural.insert(parent);
    }

    /// 移动焦点。在下一次更新中始终回显。
    pub fn set_focus(&mut self, id: u64) {
        if self.focus == id {
            return;
        }
        let previous = self.focus;
        self.focus = id;
        self.dirty.insert(previous);
        self.dirty.insert(id);
    }

    /// 移除节点及其整棵子树。
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

    /// 下一次更新将携带的节点数量。零表示“无需发送”。
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

    /// 生成下一次 `TreeUpdate`。
    ///
    /// 若无任何变更则返回 `None`，使调用方可完全跳过平台调用——
    /// “静止态 0% CPU”要求即由此起步。
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
        // 已移除的子节点绝不能再次发送，即使残留了旧的脏标记：
        // AccessKit 会拒绝针对已不存在节点的更新。
        ids.retain(|id| self.nodes.contains_key(id));
        // 若存在根节点则优先输出：读取方自顶向下构建树。
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

    /// 节点的语义路径，例如 `Window/Panel[2]/Button "Save"`。
    ///
    /// 当布局合法性检查器触发时，无头测试框架会打印该路径，使 Agent
    /// 无需第二套自省 API 即可定位问题节点。
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

    /// 迄今为止生成的更新总次数，供 KPI 基准测试使用。
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

    /// 拥有上千个节点的画布中，仅修改一个标签时必须只发送一个节点。
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

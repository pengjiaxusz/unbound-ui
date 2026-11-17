//! 增量无障碍语义树。
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

use accesskit::{Action, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
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

    /// 构造只读文本节点。
    ///
    /// 静态文本在语义树里**不是可有可无的装饰**：屏幕阅读器需要它朗读界面内容，
    /// Agent 需要它读取界面上的数值与状态。一个缺失文本的界面，在这两者眼里
    /// 都是空的——这正是「界面存在但 Agent 什么都看不到」的根因。
    ///
    /// 因此文本节点同样进入语义树：AccessKit 的 `Label` 角色即静态文本。
    #[must_use]
    pub fn static_text(text: impl Into<String>, bounds: Rect) -> Self {
        Self::new(Role::Label, text, bounds)
    }

    /// 构造「名称 + 当前值」的只读数据节点。
    ///
    /// 例如显存占用这类读数：`label` 是「显存占用」，`value` 是「12.4 GB」。
    /// 二者分开后，Agent 既能按名称定位，也能直接取到数值而不必解析文本。
    #[must_use]
    pub fn data(label: impl Into<String>, value: impl Into<String>, bounds: Rect) -> Self {
        let mut node = Self::new(Role::Label, label, bounds);
        node.value = value.into();
        node
    }

    /// 设置当前值（如开关的开关态、滑块的数值）。
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    /// 设置补充说明（如占位提示、快捷键、度量单位）。
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// 设置选中态（如列表项、页签、单选按钮）。
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 设置展开态（可折叠节点）。
    #[must_use]
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// 设置层级（标题层级或树深度）。
    #[must_use]
    pub fn with_level(mut self, level: u32) -> Self {
        self.level = level;
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

/// 语义标注意图。
///
/// 应用在构建视图时声明「这个控件在语义上是什么」，框架在布局求解**之后**
/// 把它写成语义节点。意图与节点分离的原因很简单：角色、名称、状态在构建
/// 视图时就已确定，而矩形要等布局求解完才知道。
///
/// 只有显式给出的字段会被写入（`None` 表示不覆盖），因此意图可以叠加在
/// 控件自身产出的语义之上——例如给一个按钮补上当前值，而不改写它的角色。
#[derive(Clone, Debug, Default)]
pub struct SemanticIntent {
    /// 语义角色。
    pub role: Option<Role>,
    /// 可读名称。
    pub label: Option<String>,
    /// 补充说明（占位提示、快捷键、度量单位等）。
    pub description: Option<String>,
    /// 当前值。
    pub value: Option<String>,
    /// 选中态。
    pub selected: Option<bool>,
    /// 展开态。
    pub expanded: Option<bool>,
    /// 禁用态。
    pub disabled: Option<bool>,
    /// 层级（标题层级或树深度）。
    pub level: Option<u32>,
    /// 可用动作。
    pub actions: Option<A11yActions>,
}

impl SemanticIntent {
    /// 构造一个带角色与名称的意图。
    #[must_use]
    pub fn new(role: Role, label: impl Into<String>) -> Self {
        Self {
            role: Some(role),
            label: Some(label.into()),
            ..Self::default()
        }
    }

    /// 设置当前值。
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// 设置补充说明。
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// 设置选中态。
    #[must_use]
    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// 设置展开态。
    #[must_use]
    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// 设置禁用态。
    #[must_use]
    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(disabled);
        self
    }

    /// 设置可用动作。
    #[must_use]
    pub fn with_actions(mut self, actions: A11yActions) -> Self {
        self.actions = Some(actions);
        self
    }

    /// 把意图写入节点。只有显式给出的字段会被覆盖。
    pub fn apply(&self, node: &mut A11yNode) {
        if let Some(role) = self.role {
            node.role = role;
        }
        if let Some(label) = &self.label {
            node.label.clone_from(label);
        }
        if let Some(description) = &self.description {
            node.description.clone_from(description);
        }
        if let Some(value) = &self.value {
            node.value.clone_from(value);
        }
        if let Some(selected) = self.selected {
            node.selected = selected;
        }
        if let Some(expanded) = self.expanded {
            node.expanded = Some(expanded);
        }
        if let Some(disabled) = self.disabled {
            node.disabled = disabled;
        }
        if let Some(level) = self.level {
            node.level = level;
        }
        if let Some(actions) = self.actions {
            node.actions = actions;
        }
    }
}

/// 增量语义树。
///
/// 克隆操作会生成完整快照：每帧从头重建语义树的应用可将副本交给
/// 场景收集器，同时保留原树用于自省检查。
///
/// 语义树的最大合法深度。
///
/// 超过此深度说明父链已损坏（成环或数据错乱）。所有沿父链的遍历都以它为界，
/// 保证任何畸形输入都只会被截断，不会把进程拖进无限循环。
pub const MAX_TREE_DEPTH: usize = 256;

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
    ///
    /// 结构性不变式：父节点不得是自己的子节点，也不得是自己的后代。
    /// 违反其一都会让树变成环，任何遍历都只会在环上打转——对一个后台运行的
    /// Agent 框架而言，那是把整个宿主拖死的缺陷。因此这里直接拒绝成环的挂载。
    pub fn set_children(&mut self, parent: u64, children: &[u64]) {
        let mut accepted: Vec<u64> = Vec::with_capacity(children.len());
        for child in children {
            if accepted.contains(child) {
                continue;
            }
            if self.would_create_cycle(*child, parent) {
                continue;
            }
            accepted.push(*child);
        }
        let children = &accepted[..];
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
    ///
    /// 与 [`Self::set_children`] 同样拒绝成环的挂载。
    pub fn reparent(&mut self, child: u64, parent: u64) {
        if self.would_create_cycle(child, parent) {
            return;
        }
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

    /// 判断把 `child` 挂到 `parent` 下是否会形成环。
    ///
    /// 成环的条件有二：`child` 就是 `parent`；或 `child` 是 `parent` 的祖先
    /// （此时 `parent → … → child → parent`）。父链本身若已损坏，则一律拒绝。
    fn would_create_cycle(&self, child: u64, parent: u64) -> bool {
        if child == parent {
            return true;
        }
        let mut current = self.parents.get(&parent).copied();
        let mut guard = 0usize;
        while let Some(node_id) = current {
            guard += 1;
            if guard > MAX_TREE_DEPTH {
                // 父链已不可信：保守地拒绝这次挂载。
                return true;
            }
            if node_id == child {
                return true;
            }
            current = self.parents.get(&node_id).copied();
        }
        false
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
        let mut seen = FxHashSet::default();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                // 子树里出现环：停止展开，避免无限增长。
                continue;
            }
            if current == self.focus {
                self.focus = self.root;
            }
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
    }

    #[must_use]
    pub fn parent_of(&self, id: u64) -> Option<u64> {
        self.parents.get(&id).copied()
    }

    /// 迭代语义树中的全部节点。
    ///
    /// 这是**只读**遍历：Agent 侧据此构建查询视图，而不必复制整棵树。
    /// 迭代顺序不保证稳定，调用方若需要稳定输出应自行排序。
    pub fn iter(&self) -> impl Iterator<Item = (u64, &A11yNode)> {
        self.nodes.iter().map(|(id, node)| (*id, node))
    }

    /// 收集以 `id` 为根的整棵子树（含自身），按深度优先前序。
    ///
    /// 子树含大量节点时优先用 [`Self::iter`] 自行过滤；本方法为便利封装。
    ///
    /// 遍历带**已访问集合**：语义树一旦因外部数据错乱而成环，这里的输出长度
    /// 依然有界。Agent 框架跑在宿主进程里，一个无界的收集足以把整机拖死。
    #[must_use]
    pub fn descendants(&self, id: u64) -> Vec<u64> {
        let mut out = Vec::new();
        let mut seen = FxHashSet::default();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if !self.nodes.contains_key(&current) {
                continue;
            }
            if !seen.insert(current) {
                // 已访问过：说明出现了环，停止深入。
                continue;
            }
            out.push(current);
            // 逆序压栈，保证弹出顺序与子节点顺序一致。
            for child in self.children_of(current).iter().rev() {
                stack.push(*child);
            }
        }
        out
    }

    /// 判断树中是否存在文本包含 `needle` 的节点。
    ///
    /// 用于「某段文字是否已出现」这类条件等待：比对 `label` 与 `value`，
    /// 因此只读数据的当前值同样能被匹配到。
    #[must_use]
    pub fn contains_text(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return false;
        }
        self.nodes
            .values()
            .any(|node| node.label.contains(needle) || node.value.contains(needle))
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
            let mut tree = TreeInfo::new(NodeId(self.root));
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

    /// 语义树一旦成环，任何无界的遍历都会把宿主进程拖死。
    ///
    /// 这里绕过结构不变式，直接把父指针扭成环——模拟外部数据错乱的最坏情况——
    /// 确认收集仍然有界。
    #[test]
    fn a_cyclic_tree_still_yields_a_bounded_subtree() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(10.0, 10.0, 80.0, 30.0)),
        );
        s.upsert(
            3,
            A11yNode::new(Role::Button, "Cancel", rect(10.0, 40.0, 80.0, 30.0)),
        );
        // 手工制造环：2 的父是 3，3 的父是 2。
        s.parents.insert(2, 3);
        s.parents.insert(3, 2);
        s.children.insert(2, vec![3]);
        s.children.insert(3, vec![2]);

        let subtree = s.descendants(1);
        assert!(
            subtree.len() <= 3,
            "a cyclic tree must not yield an unbounded subtree, got {}",
            subtree.len()
        );
    }

    /// 结构不变式：父节点不得成为自己的子节点，也不得是自己的后代。
    #[test]
    fn cyclic_reparenting_is_refused() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(Role::Button, "Save", rect(10.0, 10.0, 80.0, 30.0)),
        );
        s.set_children(1, &[1]);
        assert!(!s.children_of(1).contains(&1), "self parenting is refused");

        s.set_children(1, &[2]);
        s.set_children(2, &[1]);
        assert!(
            !s.children_of(2).contains(&1),
            "reparenting an ancestor under a descendant is refused"
        );
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

    #[test]
    fn removing_an_ancestor_of_the_focused_node_resets_focus_to_root() {
        let mut s = tree();
        s.upsert(
            2,
            A11yNode::new(
                Role::GenericContainer,
                "Dialog",
                rect(0.0, 0.0, 100.0, 100.0),
            ),
        );
        s.upsert(
            3,
            A11yNode::new(Role::Button, "Confirm", rect(10.0, 10.0, 40.0, 20.0)).clickable(),
        );
        s.set_children(1, &[2]);
        s.set_children(2, &[3]);
        s.set_focus(3);
        let _ = s.take_update();

        s.remove(2);
        assert_eq!(s.focus(), 1);
        let update = s
            .take_update()
            .expect("removing focused subtree must emit an update");
        assert_eq!(update.focus, NodeId(1));
    }
}

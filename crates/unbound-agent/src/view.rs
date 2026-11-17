//! Agent 看到的界面：语义树的只读投影。

use accesskit::Role;
use rustc_hash::FxHashSet;
use unbound_a11y::{A11yNode, Semantics};
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};

use crate::query::NodeQuery;

/// 单个语义节点的快照。
///
/// 这是 Agent 实际消费的数据形状：纯值、可序列化、不含引擎内部类型。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSnapshot {
    /// 节点标识。
    pub id: u64,
    /// 语义角色（`Button`、`ComboBox`、`Label` …）。
    pub role: String,
    /// 可读名称。
    pub label: String,
    /// 补充说明。
    pub description: String,
    /// 当前值。
    pub value: String,
    /// 屏幕矩形 `[左, 上, 右, 下]`。
    pub bounds: [f32; 4],
    /// 是否禁用。
    pub disabled: bool,
    /// 是否选中。
    pub selected: bool,
    /// 展开态。
    pub expanded: Option<bool>,
    /// 是否可聚焦。
    pub focusable: bool,
    /// 层级。
    pub level: u32,
    /// 可用动作名。
    pub actions: Vec<String>,
    /// 语义路径（自根节点起）。
    pub path: String,
    /// 直接子节点标识。
    pub children: Vec<u64>,
}

/// 语义树快照。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeSnapshot {
    /// 根节点标识。
    pub root: u64,
    /// 节点列表，按视觉顺序（深度优先前序）。
    pub nodes: Vec<NodeSnapshot>,
}

impl TreeSnapshot {
    /// 按标识查找节点。
    #[must_use]
    pub fn node(&self, id: u64) -> Option<&NodeSnapshot> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// 节点总数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Agent 看到的界面。
///
/// 它是语义树与命中区域的**只读投影**，不持有所有权，也不做任何转换——
/// 看到的就是屏幕阅读器看到的那棵树。
pub struct AgentView<'a> {
    semantics: &'a Semantics,
    regions: &'a [HitRegion],
    focus: u64,
    viewport: Vec2,
}

impl<'a> AgentView<'a> {
    /// 构造视图。
    #[must_use]
    pub fn new(
        semantics: &'a Semantics,
        regions: &'a [HitRegion],
        focus: u64,
        viewport: Vec2,
    ) -> Self {
        Self {
            semantics,
            regions,
            focus,
            viewport,
        }
    }

    /// 底层语义树。
    #[must_use]
    pub fn semantics(&self) -> &'a Semantics {
        self.semantics
    }

    /// 命中区域。
    #[must_use]
    pub fn regions(&self) -> &'a [HitRegion] {
        self.regions
    }

    /// 当前焦点节点。
    #[must_use]
    pub fn focus(&self) -> u64 {
        self.focus
    }

    /// 视口尺寸（逻辑像素）。
    #[must_use]
    pub fn viewport(&self) -> Vec2 {
        self.viewport
    }

    /// 节点详情。
    #[must_use]
    pub fn node(&self, id: u64) -> Option<&'a A11yNode> {
        self.semantics.get(id)
    }

    /// 节点的语义路径（`Window "…"/Group "…"/Button "…"`）。
    #[must_use]
    pub fn path(&self, id: u64) -> String {
        self.semantics.path(id)
    }

    /// 节点的屏幕矩形。
    #[must_use]
    pub fn bounds(&self, id: u64) -> Option<Rect> {
        self.semantics.get(id).map(|node| node.bounds)
    }

    /// 节点中心点。
    ///
    /// 这是**唯一**允许出现坐标的地方：坐标由节点矩形算出，用于注入合成事件。
    /// 对外接口一律以节点标识寻址。
    #[must_use]
    pub fn center(&self, id: u64) -> Option<Vec2> {
        self.bounds(id).map(|rect| rect.center())
    }

    /// 按视觉顺序（深度优先前序）列出全部节点标识。
    ///
    /// 遍历带**已访问集合**：语义树若因外部数据错乱而成环，输出长度依然有界。
    /// 这条保证是刻意保留的最后一道防线——Agent 框架与宿主同进程，一次无界的
    /// 收集就足以把整机拖死。
    #[must_use]
    pub fn tree_order(&self) -> Vec<u64> {
        let mut out = Vec::with_capacity(self.semantics.len());
        let mut seen = FxHashSet::default();
        let mut stack = vec![self.semantics.root()];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                // 已访问过：说明出现环，停止深入。
                continue;
            }
            out.push(current);
            let children = self.semantics.children_of(current);
            // 逆序压栈，保证弹出顺序与子节点顺序一致。
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
        out
    }

    /// 按条件查找节点，返回**全部**匹配项。
    ///
    /// 返回全部匹配而非第一个，是刻意的设计：Agent 拿到候选列表与各自的
    /// 语义路径后自行消歧，比框架替它猜一个更可靠。
    #[must_use]
    pub fn query(&self, query: &NodeQuery) -> Vec<u64> {
        let candidates: Vec<u64> = match query.within {
            Some(root) => self.semantics.descendants(root),
            None => self.tree_order(),
        };
        let mut out = Vec::new();
        for id in candidates {
            let Some(node) = self.semantics.get(id) else {
                continue;
            };
            if !query.matches(node, id == self.focus) {
                continue;
            }
            if let Some((anchor, relation)) = query.relative_to {
                let Some(anchor_node) = self.semantics.get(anchor) else {
                    continue;
                };
                if !relation.holds(anchor_node.bounds, node.bounds) {
                    continue;
                }
            }
            out.push(id);
            if query.limit > 0 && out.len() >= query.limit {
                break;
            }
        }
        out
    }

    /// 查找唯一匹配的节点。
    ///
    /// 命中零个或多个都返回 `None`——歧义必须由调用方显式消解，
    /// 而不是让框架替它挑一个。
    #[must_use]
    pub fn find_unique(&self, query: &NodeQuery) -> Option<u64> {
        let mut hits = self.query(query);
        if hits.len() == 1 {
            hits.pop()
        } else {
            None
        }
    }

    /// 导出整棵语义树快照。
    #[must_use]
    pub fn to_tree(&self) -> TreeSnapshot {
        self.snapshot(self.semantics.root())
    }

    /// 导出以 `id` 为根的子树快照。
    #[must_use]
    pub fn subtree(&self, id: u64) -> TreeSnapshot {
        self.snapshot(id)
    }

    /// 生成快照。
    ///
    /// 遍历带已访问集合，成环的语义树不会让快照无限膨胀。
    fn snapshot(&self, root: u64) -> TreeSnapshot {
        let mut nodes = Vec::new();
        let mut seen = FxHashSet::default();
        let mut stack = vec![root];
        while let Some(current) = stack.pop() {
            let Some(node) = self.semantics.get(current) else {
                continue;
            };
            if !seen.insert(current) {
                continue;
            }
            let children = self.semantics.children_of(current).to_vec();
            nodes.push(NodeSnapshot {
                id: current,
                role: format!("{:?}", node.role),
                label: node.label.clone(),
                description: node.description.clone(),
                value: node.value.clone(),
                bounds: [
                    node.bounds.min().x,
                    node.bounds.min().y,
                    node.bounds.max().x,
                    node.bounds.max().y,
                ],
                disabled: node.disabled,
                selected: node.selected,
                expanded: node.expanded,
                focusable: node.focusable,
                level: node.level,
                actions: node
                    .actions
                    .iter()
                    .map(|action| action.name().to_owned())
                    .collect(),
                path: self.semantics.path(current),
                children: children.clone(),
            });
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
        TreeSnapshot { root, nodes }
    }

    /// 列出树中所有文本内容。
    ///
    /// Agent 常需要「界面上都写了什么」这一整体印象——逐个节点翻找既慢又容易
    /// 漏掉嵌套在容器里的说明文字。
    ///
    /// 节点的**名称与当前值都会列出**：只给其一，Agent 就得靠猜。例如读数类
    /// 数据的名称是「显存读数」、值是「12.4 GB」，两者都是界面上真实存在的信息。
    #[must_use]
    pub fn all_text(&self) -> Vec<(u64, String)> {
        let mut out = Vec::new();
        for id in self.tree_order() {
            let Some(node) = self.semantics.get(id) else {
                continue;
            };
            if !node.label.is_empty() {
                out.push((id, node.label.clone()));
            }
            if !node.value.is_empty() && node.value != node.label {
                out.push((id, node.value.clone()));
            }
        }
        out
    }

    /// 判断某个角色是否出现在树中。
    #[must_use]
    pub fn has_role(&self, role: Role) -> bool {
        !self.query(&NodeQuery::any().role(role)).is_empty()
    }
}

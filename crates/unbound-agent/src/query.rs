//! 查询条件：按角色、名称、状态与空间关系定位界面元素。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_math::Rect;

/// 节点状态谓词。
///
/// 状态用「谓词」而非「枚举」表达，是因为有些节点同时满足多个状态
/// （一个被选中的可聚焦按钮），用枚举会强迫调用方二选一。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodePredicate {
    /// 已禁用。
    Disabled,
    /// 可用（未禁用）。
    Enabled,
    /// 已选中。
    Selected,
    /// 未选中。
    Unselected,
    /// 已展开。
    Expanded,
    /// 已收起。
    Collapsed,
    /// 可聚焦。
    Focusable,
    /// 当前已获得焦点。
    Focused,
}

impl NodePredicate {
    /// 判断节点是否满足该谓词。
    #[must_use]
    pub fn matches(&self, node: &A11yNode, focused: bool) -> bool {
        match self {
            Self::Disabled => node.disabled,
            Self::Enabled => !node.disabled,
            Self::Selected => node.selected,
            Self::Unselected => !node.selected,
            Self::Expanded => node.expanded == Some(true),
            Self::Collapsed => node.expanded == Some(false),
            Self::Focusable => node.focusable,
            Self::Focused => focused,
        }
    }

    /// 谓词的简短名称，用于结构化诊断输出。
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Selected => "selected",
            Self::Unselected => "unselected",
            Self::Expanded => "expanded",
            Self::Collapsed => "collapsed",
            Self::Focusable => "focusable",
            Self::Focused => "focused",
        }
    }
}

/// 空间关系。
///
/// 有些界面元素**没有语义名称**，只能靠空间关系定位——例如表格某一行右侧
/// 的操作按钮。此时唯一的线索就是「在某个节点右边」。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpatialRelation {
    /// 位于目标右侧（大致同行）。
    RightOf,
    /// 位于目标左侧（大致同行）。
    LeftOf,
    /// 位于目标下方。
    Below,
    /// 位于目标上方。
    Above,
    /// 位于目标附近（默认 48 逻辑像素内）。
    Near,
    /// 与目标重叠。
    Overlapping,
}

impl SpatialRelation {
    /// 判断 `candidate` 是否与 `anchor` 满足该空间关系。
    #[must_use]
    pub fn holds(self, anchor: Rect, candidate: Rect) -> bool {
        let a = anchor.center();
        let c = candidate.center();
        match self {
            Self::RightOf => c.x > a.x && (c.y - a.y).abs() <= anchor.height().max(1.0),
            Self::LeftOf => c.x < a.x && (c.y - a.y).abs() <= anchor.height().max(1.0),
            Self::Below => c.y > a.y && (c.x - a.x).abs() <= anchor.width().max(1.0),
            Self::Above => c.y < a.y && (c.x - a.x).abs() <= anchor.width().max(1.0),
            Self::Near => {
                let dx = c.x - a.x;
                let dy = c.y - a.y;
                (dx * dx + dy * dy).sqrt() <= 48.0
            }
            Self::Overlapping => {
                let min_x = anchor.min().x.max(candidate.min().x);
                let min_y = anchor.min().y.max(candidate.min().y);
                let max_x = anchor.max().x.min(candidate.max().x);
                let max_y = anchor.max().y.min(candidate.max().y);
                max_x > min_x && max_y > min_y
            }
        }
    }
}

/// 下拉选项可能使用的角色。
///
/// ARIA 把 `<option>` 映射为 [`Role::ListBoxOption`]；菜单式下拉也可能用
/// [`Role::MenuListOption`] 或 [`Role::MenuItem`]。查询时三者都接受，
/// 免得因实现风格不同而找不到选项。
pub const OPTION_ROLES: [Role; 3] = [
    Role::ListBoxOption,
    Role::MenuListOption,
    Role::MenuItem,
];

/// 节点查询条件。
///
/// 查询**返回全部匹配项**而非「第一个」：Agent 拿到候选列表与各自的语义路径，
/// 自己结合上下文消歧，比框架替它猜一个更可靠。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeQuery {
    /// 角色过滤。
    pub role: Option<Role>,
    /// 名称精确匹配（区分大小写）。
    pub name: Option<String>,
    /// 名称包含（区分大小写）。
    pub name_contains: Option<String>,
    /// 当前值精确匹配。
    pub value: Option<String>,
    /// 限定在某祖先子树内。
    pub within: Option<u64>,
    /// 状态谓词（全部满足才算命中）。
    pub predicates: Vec<NodePredicate>,
    /// 空间关系：与某个节点的关系。
    pub relative_to: Option<(u64, SpatialRelation)>,
    /// 命中数量上限；`0` 表示不限。
    pub limit: usize,
}

impl NodeQuery {
    /// 空查询：匹配任意节点。
    #[must_use]
    pub fn any() -> Self {
        Self::default()
    }

    /// 按角色过滤。
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// 按名称精确匹配。
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 按名称模糊匹配。
    #[must_use]
    pub fn name_contains(mut self, name: impl Into<String>) -> Self {
        self.name_contains = Some(name.into());
        self
    }

    /// 按当前值精确匹配。
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// 限定在某祖先子树内。
    #[must_use]
    pub fn within(mut self, ancestor: u64) -> Self {
        self.within = Some(ancestor);
        self
    }

    /// 追加状态谓词。
    #[must_use]
    pub fn predicate(mut self, predicate: NodePredicate) -> Self {
        self.predicates.push(predicate);
        self
    }

    /// 按空间关系过滤。
    #[must_use]
    pub fn relative_to(mut self, anchor: u64, relation: SpatialRelation) -> Self {
        self.relative_to = Some((anchor, relation));
        self
    }

    /// 限制命中数量。
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// 判断节点是否满足全部条件。
    #[must_use]
    pub fn matches(&self, node: &A11yNode, focused: bool) -> bool {
        if let Some(role) = self.role {
            if node.role != role {
                return false;
            }
        }
        if let Some(name) = &self.name {
            if node.label != *name {
                return false;
            }
        }
        if let Some(contains) = &self.name_contains {
            if !node.label.contains(contains.as_str()) {
                return false;
            }
        }
        if let Some(value) = &self.value {
            if node.value != *value {
                return false;
            }
        }
        self.predicates
            .iter()
            .all(|predicate| predicate.matches(node, focused))
    }
}

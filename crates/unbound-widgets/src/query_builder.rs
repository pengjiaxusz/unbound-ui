//! 可视化嵌套条件规则构建器（`QueryBuilder`）与多维筛选器状态（`FilterPopupState`）。
//!
//! 提供规则 CRUD、嵌套 `AND`/`OR` 分组（[`QueryGroup`] / [`QueryBuilderState`]）、
//! 通用字段求值匹配器（[`evaluate_rule`]）与无损状态序列化/反序列化。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 逻辑组合算子。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum QueryCombinator {
    #[default]
    And,
    Or,
}

impl QueryCombinator {
    /// 切换逻辑组合算子（`AND <-> OR`）。
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::And => Self::Or,
            Self::Or => Self::And,
        }
    }

    /// 序列化标签。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
        }
    }

    /// 从字符串解析逻辑算子。
    #[must_use]
    pub fn from_label(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "AND" => Some(Self::And),
            "OR" => Some(Self::Or),
            _ => None,
        }
    }
}

/// 单条查询过滤规则。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRule {
    pub field: String,
    pub operator: String,
    pub value: String,
}

impl QueryRule {
    #[must_use]
    pub fn new(
        field: impl Into<String>,
        operator: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            operator: operator.into(),
            value: value.into(),
        }
    }
}

/// 对单条规则执行纯 Rust 通用匹配求值。
///
/// 支持的运算符：`=`/`==`、`!=`、`contains`、`starts_with`、`ends_with`、`>`、`>=`、`<`、`<=`。
/// 比较类运算符优先尝试按 `f64` 数值比较，解析失败时退回字典序字符串比较。
#[must_use]
pub fn evaluate_rule<'a, F>(rule: &QueryRule, lookup: &F) -> bool
where
    F: Fn(&str) -> Option<&'a str>,
{
    let Some(actual) = lookup(&rule.field) else {
        return false;
    };
    let expected = rule.value.as_str();
    match rule.operator.trim().to_ascii_lowercase().as_str() {
        "=" | "==" | "eq" => actual == expected,
        "!=" | "ne" => actual != expected,
        "contains" => actual
            .to_ascii_lowercase()
            .contains(&expected.to_ascii_lowercase()),
        "starts_with" => actual
            .to_ascii_lowercase()
            .starts_with(&expected.to_ascii_lowercase()),
        "ends_with" => actual
            .to_ascii_lowercase()
            .ends_with(&expected.to_ascii_lowercase()),
        ">" | "gt" => compare_values(actual, expected).is_gt(),
        ">=" | "gte" => compare_values(actual, expected).is_ge(),
        "<" | "lt" => compare_values(actual, expected).is_lt(),
        "<=" | "lte" => compare_values(actual, expected).is_le(),
        _ => false,
    }
}

fn compare_values(actual: &str, expected: &str) -> std::cmp::Ordering {
    if let (Ok(a), Ok(b)) = (actual.trim().parse::<f64>(), expected.trim().parse::<f64>()) {
        a.total_cmp(&b)
    } else {
        actual.cmp(expected)
    }
}

/// 支持嵌套 `AND`/`OR` 组合的规则分组。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryGroup {
    pub combinator: QueryCombinator,
    pub rules: Vec<QueryRule>,
    pub groups: Vec<QueryGroup>,
}

impl QueryGroup {
    /// 创建空规则分组。
    #[must_use]
    pub const fn new(combinator: QueryCombinator) -> Self {
        Self {
            combinator,
            rules: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// 统计当前分组及所有嵌套子分组中的规则总数。
    #[must_use]
    pub fn total_rule_count(&self) -> usize {
        self.rules.len()
            + self
                .groups
                .iter()
                .map(Self::total_rule_count)
                .sum::<usize>()
    }

    /// 递归求值当前分组（空分组默认返回 `true`，即不施加过滤限制）。
    #[must_use]
    pub fn evaluate<'a, F>(&self, lookup: &F) -> bool
    where
        F: Fn(&str) -> Option<&'a str>,
    {
        if self.rules.is_empty() && self.groups.is_empty() {
            return true;
        }
        match self.combinator {
            QueryCombinator::And => {
                self.rules.iter().all(|r| evaluate_rule(r, lookup))
                    && self.groups.iter().all(|g| g.evaluate(lookup))
            }
            QueryCombinator::Or => {
                self.rules.iter().any(|r| evaluate_rule(r, lookup))
                    || self.groups.iter().any(|g| g.evaluate(lookup))
            }
        }
    }

    fn write_serialized(&self, out: &mut String, depth: usize) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str("GROUP ");
        out.push_str(self.combinator.as_str());
        out.push('\n');
        for rule in &self.rules {
            for _ in 0..=depth {
                out.push_str("  ");
            }
            out.push_str("RULE ");
            out.push_str(&escape_token(&rule.field));
            out.push('\t');
            out.push_str(&escape_token(&rule.operator));
            out.push('\t');
            out.push_str(&escape_token(&rule.value));
            out.push('\n');
        }
        for sub in &self.groups {
            sub.write_serialized(out, depth + 1);
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str("END\n");
    }
}

fn escape_token(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn unescape_token(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 可视化查询构建器交互状态机（管理规则 CRUD、嵌套分组、求值与持久化序列化）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryBuilderState {
    /// 根条件分组。
    pub root: QueryGroup,
}

impl QueryBuilderState {
    /// 创建默认 `AND` 组合的空查询状态。
    #[must_use]
    pub fn new(combinator: QueryCombinator) -> Self {
        Self {
            root: QueryGroup::new(combinator),
        }
    }

    /// 在根分组追加一条规则并返回其索引。
    pub fn add_rule(&mut self, rule: QueryRule) -> usize {
        self.root.rules.push(rule);
        self.root.rules.len() - 1
    }

    /// 删除根分组指定索引的规则。
    pub fn remove_rule(&mut self, index: usize) -> Option<QueryRule> {
        if index < self.root.rules.len() {
            Some(self.root.rules.remove(index))
        } else {
            None
        }
    }

    /// 更新根分组指定规则的完整内容。
    pub fn update_rule(&mut self, index: usize, rule: QueryRule) -> bool {
        if let Some(slot) = self.root.rules.get_mut(index) {
            *slot = rule;
            true
        } else {
            false
        }
    }

    /// 修改指定规则的字段名。
    pub fn set_rule_field(&mut self, index: usize, field: impl Into<String>) -> bool {
        if let Some(slot) = self.root.rules.get_mut(index) {
            slot.field = field.into();
            true
        } else {
            false
        }
    }

    /// 修改指定规则的操作符。
    pub fn set_rule_operator(&mut self, index: usize, operator: impl Into<String>) -> bool {
        if let Some(slot) = self.root.rules.get_mut(index) {
            slot.operator = operator.into();
            true
        } else {
            false
        }
    }

    /// 修改指定规则的比较值。
    pub fn set_rule_value(&mut self, index: usize, value: impl Into<String>) -> bool {
        if let Some(slot) = self.root.rules.get_mut(index) {
            slot.value = value.into();
            true
        } else {
            false
        }
    }

    /// 切换根分组的 `AND` / `OR` 逻辑组合。
    pub fn toggle_combinator(&mut self) -> QueryCombinator {
        self.root.combinator = self.root.combinator.toggle();
        self.root.combinator
    }

    /// 添加嵌套子分组并返回其索引。
    pub fn add_group(&mut self, group: QueryGroup) -> usize {
        self.root.groups.push(group);
        self.root.groups.len() - 1
    }

    /// 移除指定嵌套子分组。
    pub fn remove_group(&mut self, index: usize) -> Option<QueryGroup> {
        if index < self.root.groups.len() {
            Some(self.root.groups.remove(index))
        } else {
            None
        }
    }

    /// 清空全部规则与嵌套子分组。
    pub fn clear_all(&mut self) {
        self.root.rules.clear();
        self.root.groups.clear();
    }

    /// 统计全部有效规则数量（含嵌套分组）。
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.root.total_rule_count()
    }

    /// 对给定字段查询闭包执行完整嵌套条件求值。
    #[must_use]
    pub fn evaluate<'a, F>(&self, lookup: F) -> bool
    where
        F: Fn(&str) -> Option<&'a str>,
    {
        self.root.evaluate(&lookup)
    }

    /// 将当前嵌套查询条件无损序列化为可持久化文本格式。
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        self.root.write_serialized(&mut out, 0);
        out
    }

    /// 从 [`Self::serialize`] 生成的文本格式反序列化恢复状态。
    pub fn deserialize(input: &str) -> Result<Self, String> {
        let mut stack: Vec<QueryGroup> = Vec::new();
        let mut finished: Option<QueryGroup> = None;

        for (line_no, raw_line) in input.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("GROUP ") {
                let comb = QueryCombinator::from_label(rest).ok_or_else(|| {
                    format!("Invalid combinator at line {}: {rest}", line_no + 1)
                })?;
                stack.push(QueryGroup::new(comb));
            } else if let Some(rest) = line.strip_prefix("RULE ") {
                let parts: Vec<&str> = rest.split('\t').collect();
                if parts.len() != 3 {
                    return Err(format!("Malformed RULE at line {}", line_no + 1));
                }
                let top = stack
                    .last_mut()
                    .ok_or_else(|| format!("RULE outside GROUP at line {}", line_no + 1))?;
                top.rules.push(QueryRule::new(
                    unescape_token(parts[0]),
                    unescape_token(parts[1]),
                    unescape_token(parts[2]),
                ));
            } else if line == "END" {
                let completed = stack
                    .pop()
                    .ok_or_else(|| format!("Unexpected END at line {}", line_no + 1))?;
                if let Some(parent) = stack.last_mut() {
                    parent.groups.push(completed);
                } else {
                    finished = Some(completed);
                }
            } else {
                return Err(format!("Unknown token at line {}: {line}", line_no + 1));
            }
        }

        if !stack.is_empty() {
            return Err("Unclosed GROUP in serialized query".to_owned());
        }
        let root = finished.ok_or_else(|| "Empty serialized query".to_owned())?;
        Ok(Self { root })
    }

    /// 构造用于渲染的 [`QueryBuilderDesc`]。
    #[must_use]
    pub fn to_desc(&self, rect: Rect) -> QueryBuilderDesc {
        QueryBuilderDesc {
            rect,
            combinator: self.root.combinator,
            rules: self.root.rules.clone(),
            groups: self.root.groups.clone(),
            z: 15,
        }
    }
}

/// 可视化查询构建器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct QueryBuilderDesc {
    pub rect: Rect,
    pub combinator: QueryCombinator,
    pub rules: Vec<QueryRule>,
    /// 嵌套子条件分组列表。
    pub groups: Vec<QueryGroup>,
    pub z: u16,
}

impl QueryBuilderDesc {
    #[must_use]
    pub fn new(rect: Rect, combinator: QueryCombinator, rules: Vec<QueryRule>) -> Self {
        Self {
            rect,
            combinator,
            rules,
            groups: Vec::new(),
            z: 15,
        }
    }

    /// 统计含嵌套分组在内的总规则数。
    #[must_use]
    pub fn total_rule_count(&self) -> usize {
        self.rules.len() + self.groups.iter().map(QueryGroup::total_rule_count).sum::<usize>()
    }
}

/// 发射查询构建器（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_query_builder(
    node: u64,
    desc: &QueryBuilderDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + desc.rules.len() * 3 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let outer_border = if !control.disabled && control.phase == PointerPhase::Pressed {
        Token::PRIMARY
    } else if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::INPUT_BORDER_HOVER
    } else {
        Token::CARD_BORDER
    };
    // 容器是 WinUI 卡片/浮层：圆角 `OverlayCornerRadius = 8`、
    // 填充 `CardBackgroundFillColorDefault`、描边 `CardStrokeColorDefault`。
    primitives.push(outlined(r, 8.0, Token::CARD_BG, outer_border, 1.0, 0.0, z));

    // 顶部 AND/OR 切换胶囊
    let comb_r = Rect::new(r.origin.x + 12.0, r.origin.y + 10.0, 56.0, 24.0);
    let comb_fill = match desc.combinator {
        QueryCombinator::And => Token::PRIMARY_SOFT,
        QueryCombinator::Or => Token::SECONDARY_SOFT,
    };
    primitives.push(outlined(
        comb_r,
        // 控件圆角 `ControlCornerRadius = 4`。
        4.0,
        comb_fill,
        Token::PRIMARY,
        1.0,
        0.0,
        z + 1,
    ));

    for (idx, _rule) in desc.rules.iter().enumerate() {
        let y = r.origin.y + 42.0 + idx as f32 * 34.0;
        let row_r = Rect::new(r.origin.x + 12.0, y, (r.size.x - 24.0).max(40.0), 28.0);
        // 规则行是文本控件外观：`ControlFillColorDefault` + `ControlStrokeColorDefault`。
        primitives.push(outlined(
            row_r,
            4.0,
            Token::CONTROL_FILL,
            Token::CONTROL_STROKE,
            1.0,
            0.0,
            z + 1,
        ));
        let op_r = Rect::new(row_r.origin.x + 96.0, y + 4.0, 48.0, 20.0);
        primitives.push(surface(op_r, 4.0, Token::PRIMARY_SOFT, z + 2));
        let del_r = Rect::new(row_r.max().x - 24.0, y + 4.0, 20.0, 20.0);
        primitives.push(surface(del_r, 4.0, Token::ERROR_SOFT, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        10.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(
            A11yNode::new(Role::Group, "Query Builder", r)
                .with_value(format!("{} rules", desc.total_rule_count()))
                .disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    }
}

/// 多维资产筛选器状态（跟踪选中扩展名、标签、最低评分与大小区间）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterPopupState {
    pub selected_extensions: Vec<String>,
    pub selected_tags: Vec<String>,
    pub min_rating: u8,
    pub size_range_mb: (f32, f32),
}

impl FilterPopupState {
    /// 当前已激活的过滤条件总数（用于工具栏徽章显示）。
    #[must_use]
    pub fn active_filter_count(&self) -> usize {
        let mut count = self.selected_extensions.len() + self.selected_tags.len();
        if self.min_rating > 0 {
            count += 1;
        }
        if self.size_range_mb.0 > 0.0 || self.size_range_mb.1 > 0.0 {
            count += 1;
        }
        count
    }

    /// 一键清空所有过滤条件。
    pub fn clear_all(&mut self) {
        self.selected_extensions.clear();
        self.selected_tags.clear();
        self.min_rating = 0;
        self.size_range_mb = (0.0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn query_builder_crud_nested_evaluation_and_clear_all() {
        let mut qb = QueryBuilderState::new(QueryCombinator::And);
        // 空条件默认放行。
        assert!(qb.evaluate(|_| None));

        let i0 = qb.add_rule(QueryRule::new("vram_mb", ">=", "512"));
        let i1 = qb.add_rule(QueryRule::new("format", "contains", "RGBA"));
        assert_eq!(i0, 0);
        assert_eq!(i1, 1);

        // 添加嵌套 OR 子分组：(usage == "RenderTarget" OR usage == "Storage")
        let mut sub = QueryGroup::new(QueryCombinator::Or);
        sub.rules.push(QueryRule::new("usage", "=", "RenderTarget"));
        sub.rules.push(QueryRule::new("usage", "=", "Storage"));
        qb.add_group(sub);
        assert_eq!(qb.rule_count(), 4);

        let matching = |field: &str| match field {
            "vram_mb" => Some("1024"),
            "format" => Some("VK_FORMAT_R16G16B16A16_SFLOAT_RGBA"),
            "usage" => Some("Storage"),
            _ => None,
        };
        assert!(qb.evaluate(matching));

        // 边界值：vram_mb 刚好等于 512 匹配 `>=`，511 不匹配。
        let boundary_ok = |field: &str| match field {
            "vram_mb" => Some("512"),
            "format" => Some("RGBA8"),
            "usage" => Some("RenderTarget"),
            _ => None,
        };
        assert!(qb.evaluate(boundary_ok));

        let boundary_fail = |field: &str| match field {
            "vram_mb" => Some("511"),
            "format" => Some("RGBA8"),
            "usage" => Some("RenderTarget"),
            _ => None,
        };
        assert!(!qb.evaluate(boundary_fail));

        // 修改规则操作符与删除规则。
        assert!(qb.set_rule_operator(0, "<"));
        assert!(qb.evaluate(boundary_fail));
        assert!(qb.remove_rule(1).is_some());
        assert_eq!(qb.rule_count(), 3);

        // 清空所有规则与子分组。
        qb.clear_all();
        assert_eq!(qb.rule_count(), 0);
        assert!(qb.evaluate(|_| None));
    }

    #[test]
    fn query_builder_serialization_roundtrip_and_filter_popup() {
        let mut qb = QueryBuilderState::new(QueryCombinator::Or);
        qb.add_rule(QueryRule::new("name", "starts_with", "shadow_"));
        let mut nested = QueryGroup::new(QueryCombinator::And);
        nested.rules.push(QueryRule::new("gpu_ms", ">", "2.5"));
        nested.rules.push(QueryRule::new("pass", "!=", "UI"));
        qb.add_group(nested);

        let serialized = qb.serialize();
        let restored = QueryBuilderState::deserialize(&serialized)
            .expect("serialized query must deserialize cleanly");
        assert_eq!(qb, restored);

        let mut fp = FilterPopupState {
            selected_extensions: vec!["rdc".to_owned(), "png".to_owned()],
            selected_tags: vec!["hdr".to_owned()],
            min_rating: 4,
            size_range_mb: (10.0, 250.0),
        };
        assert_eq!(fp.active_filter_count(), 5);
        fp.clear_all();
        assert_eq!(fp.active_filter_count(), 0);

        // 无头几何健康检查。
        let rect = Rect::new(20.0, 20.0, 420.0, 180.0);
        let desc = qb.to_desc(rect);
        let out = emit_query_builder(1, &desc, ControlState::default(), &[]);
        assert!(!out.primitives.is_empty());

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "QueryBuilder",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

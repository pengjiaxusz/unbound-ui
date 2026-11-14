//! 可视化嵌套条件规则构建器（`QueryBuilder`）与多维筛选器状态（`FilterPopupState`）。

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

/// 可视化查询构建器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct QueryBuilderDesc {
    pub rect: Rect,
    pub combinator: QueryCombinator,
    pub rules: Vec<QueryRule>,
    pub z: u16,
}

impl QueryBuilderDesc {
    #[must_use]
    pub fn new(rect: Rect, combinator: QueryCombinator, rules: Vec<QueryRule>) -> Self {
        Self {
            rect,
            combinator,
            rules,
            z: 15,
        }
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
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        r,
        8.0,
        Token::SURFACE_RAISED,
        outer_border,
        1.0,
        0.0,
        z,
    ));

    // 顶部 AND/OR 切换胶囊
    let comb_r = Rect::new(r.origin.x + 12.0, r.origin.y + 10.0, 56.0, 24.0);
    let comb_fill = match desc.combinator {
        QueryCombinator::And => Token::PRIMARY_SOFT,
        QueryCombinator::Or => Token::SECONDARY_SOFT,
    };
    primitives.push(outlined(
        comb_r,
        5.0,
        comb_fill,
        Token::PRIMARY,
        1.0,
        0.0,
        z + 1,
    ));

    for (idx, _rule) in desc.rules.iter().enumerate() {
        let y = r.origin.y + 42.0 + idx as f32 * 34.0;
        let row_r = Rect::new(r.origin.x + 12.0, y, (r.size.x - 24.0).max(40.0), 28.0);
        primitives.push(outlined(
            row_r,
            5.0,
            Token::SURFACE_SUNKEN,
            Token::OUTLINE,
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
        a11y: Some(A11yNode::new(Role::Group, "Query Builder", r).disabled(control.disabled)),
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

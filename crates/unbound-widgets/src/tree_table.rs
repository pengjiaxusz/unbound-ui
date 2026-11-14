//! TreeTable：将层级折叠树与多列表格数据结合的树形表格。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{outlined, surface};
use crate::table::ColumnSpec;
use crate::{ControlState, WidgetOutput};

pub const TREE_TABLE_ROW_HEIGHT: f32 = 28.0;
pub const TREE_TABLE_HEADER_HEIGHT: f32 = 32.0;

/// 树形表格中带缩进深度的单行。
#[derive(Clone, Debug, PartialEq)]
pub struct TreeTableRow {
    pub id: u64,
    pub depth: usize,
    pub has_children: bool,
    pub cells: Vec<String>,
}

/// 树形表格描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TreeTableDesc {
    pub rect: Rect,
    pub columns: Vec<ColumnSpec>,
    pub rows: Vec<TreeTableRow>,
    pub radius: f32,
}

impl TreeTableDesc {
    #[must_use]
    pub fn new(rect: Rect, columns: Vec<ColumnSpec>) -> Self {
        Self {
            rect,
            columns,
            rows: Vec::new(),
            radius: 6.0,
        }
    }

    #[must_use]
    pub fn columns(columns: Vec<ColumnSpec>) -> Self {
        Self::new(Rect::ZERO, columns)
    }

    #[must_use]
    pub fn with_rows(mut self, rows: Vec<TreeTableRow>) -> Self {
        self.rows = rows;
        self
    }
}

/// 树形表格状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeTableState {
    pub expanded: Vec<u64>,
    pub selected: Option<u64>,
}

impl TreeTableState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle_expand(&mut self, id: u64) {
        if let Some(pos) = self.expanded.iter().position(|&x| x == id) {
            self.expanded.remove(pos);
        } else {
            self.expanded.push(id);
        }
    }

    #[must_use]
    pub fn is_expanded(&self, id: u64) -> bool {
        self.expanded.contains(&id)
    }
}

/// 输出树形表格的图元与命中区域。
#[must_use]
pub fn emit_tree_table(
    node: u64,
    desc: &TreeTableDesc,
    state: &TreeTableState,
    control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let z = 700u16;

    // 1. 表格背景表面
    out.primitives
        .push(surface(rect, desc.radius, Token::SURFACE, z));

    // 2. 表头背景
    let header_rect = Rect::new(
        rect.origin.x,
        rect.origin.y,
        rect.width(),
        TREE_TABLE_HEADER_HEIGHT.min(rect.height()),
    );
    out.primitives.push(surface(
        header_rect,
        desc.radius,
        Token::SURFACE_RAISED,
        z + 1,
    ));

    // 3. 表头底部边框线
    let header_line = Rect::new(
        rect.origin.x,
        rect.origin.y + header_rect.height() - 1.0,
        rect.width(),
        1.0,
    );
    out.primitives
        .push(surface(header_line, 0.0, Token::OUTLINE, z + 2));

    // 4. 数据行（最多 4 个可见行以保持结构稳定）
    for row_idx in 0..4 {
        let y = rect.origin.y + header_rect.height() + row_idx as f32 * TREE_TABLE_ROW_HEIGHT;
        let row_rect = Rect::new(rect.origin.x, y, rect.width(), TREE_TABLE_ROW_HEIGHT);
        let is_selected = state.selected == Some(row_idx as u64);
        let row_bg = if is_selected {
            Token::PRIMARY_SOFT
        } else if row_idx % 2 == 1 {
            Token::SURFACE_SUNKEN
        } else {
            Token::SURFACE
        };

        out.primitives.push(surface(row_rect, 0.0, row_bg, z + 3));
    }

    // 5. 外边框
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if control.focused {
            Token::PRIMARY
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        z + 4,
    ));

    out.hit = Some(
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer),
    );

    out.a11y = Some(A11yNode::new(Role::TreeGrid, "TreeTable", rect));
    out
}

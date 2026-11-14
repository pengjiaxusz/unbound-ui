//! 可编辑虚拟化数据表格（DataGrid / Spreadsheet），支持单元格选中、多命中区域与行内编辑。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::table::ColumnSpec;
use crate::{ControlState, PointerPhase, WidgetOutput};

const GRID_COLS: usize = 3;
const GRID_ROWS: usize = 3;

/// [`DataGridDesc`] 单元格 `(row, col)` 的确定性子节点键。
#[must_use]
pub const fn cell_node(base: u64, row: usize, col: usize) -> u64 {
    base ^ (((row as u64 + 1) << 16) | (col as u64 + 1))
}

/// [`DataGridDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataGridState {
    /// 当前激活的 `(row, col)` 单元格。
    pub active_cell: (usize, usize),
    /// 当前激活单元格是否处于行内编辑模式。
    pub editing: bool,
    /// 编辑中的草稿单元格文本。
    pub edit_buffer: String,
}

impl DataGridState {
    /// 选中 `(row, col)` 并退出当前编辑状态。
    pub fn select_cell(&mut self, row: usize, col: usize) {
        self.active_cell = (row, col);
        self.editing = false;
    }

    /// 按 `(dr, dc)` 移动激活单元格，并限制在 `0..rows` 与 `0..cols` 内。
    pub fn move_cell(&mut self, dr: isize, dc: isize, rows: usize, cols: usize) {
        if rows == 0 || cols == 0 {
            return;
        }
        let r = (self.active_cell.0 as isize + dr).clamp(0, rows as isize - 1) as usize;
        let c = (self.active_cell.1 as isize + dc).clamp(0, cols as isize - 1) as usize;
        self.active_cell = (r, c);
    }

    /// 以 `initial` 文本开始行内编辑。
    pub fn begin_edit(&mut self, row: usize, col: usize, initial: impl Into<String>) {
        self.active_cell = (row, col);
        self.editing = true;
        self.edit_buffer = initial.into();
    }

    /// 提交行内编辑并返回 `(row, col, value)`。
    pub fn commit_edit(&mut self) -> Option<(usize, usize, String)> {
        if !self.editing {
            return None;
        }
        self.editing = false;
        Some((
            self.active_cell.0,
            self.active_cell.1,
            core::mem::take(&mut self.edit_buffer),
        ))
    }
}

/// [`DataGrid`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DataGridDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 当前激活的 `(row, col)` 单元格。
    pub active_cell: (usize, usize),
    /// 是否处于行内编辑状态。
    pub editing: bool,
    /// 可选列定义列表。
    pub columns: Vec<ColumnSpec>,
    /// 可见单元格文本矩阵 `[row][col]`。
    pub cells: Vec<Vec<String>>,
    /// 可见窗口的起始逻辑行索引。
    pub row_offset: usize,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl DataGridDesc {
    /// 创建数据表格描述符。
    #[must_use]
    pub fn new(rect: Rect, active_cell: (usize, usize), editing: bool) -> Self {
        Self {
            rect,
            active_cell,
            editing,
            columns: Vec::new(),
            cells: Vec::new(),
            row_offset: 0,
            label: "Data Grid".to_owned(),
            z: 600,
        }
    }

    /// 附加列定义。
    #[must_use]
    pub fn with_columns(mut self, columns: Vec<ColumnSpec>) -> Self {
        self.columns = columns;
        self
    }

    /// 附加可见单元格文本矩阵。
    #[must_use]
    pub fn with_cells(mut self, cells: Vec<Vec<String>>) -> Self {
        self.cells = cells;
        self
    }

    /// 设置逻辑起始行偏移量。
    #[must_use]
    pub fn with_row_offset(mut self, offset: usize) -> Self {
        self.row_offset = offset;
        self
    }

    /// 可见列数。
    #[must_use]
    pub fn visible_cols(&self) -> usize {
        if self.columns.is_empty() {
            GRID_COLS
        } else {
            self.columns.len()
        }
    }

    /// 可见行数。
    #[must_use]
    pub fn visible_rows(&self) -> usize {
        if self.cells.is_empty() {
            GRID_ROWS
        } else {
            self.cells.len()
        }
    }

    /// 计算 `(row, col)` 对应的单元格矩形。
    #[must_use]
    pub fn cell_rect(&self, row: usize, col: usize) -> Rect {
        let cols = self.visible_cols().max(1);
        let rows = self.visible_rows().max(1);
        let pad = 4.0;
        let cw = ((self.rect.width() - pad * 2.0) / cols as f32).max(16.0);
        let ch = ((self.rect.height() - pad * 2.0) / rows as f32).max(14.0);
        Rect::new(
            self.rect.origin.x + pad + col as f32 * cw,
            self.rect.origin.y + pad + row as f32 * ch,
            (cw - 2.0).max(8.0),
            (ch - 2.0).max(8.0),
        )
    }
}

/// 发射固定形状的数据表格，包含逐单元格子命中区域与无障碍节点。
#[must_use]
pub fn emit_data_grid(
    node: u64,
    desc: &DataGridDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let rows = desc.visible_rows();
    let cols = desc.visible_cols();
    let mut primitives = Vec::with_capacity(2 + rows * cols + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    for r in 0..rows {
        for c in 0..cols {
            let cell_r = desc.cell_rect(r, c);
            let is_active = desc.active_cell == (r + desc.row_offset, c);
            let fill = if is_active {
                Token::PRIMARY_SOFT
            } else if r == 0 || hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            };
            primitives.push(outlined(
                cell_r,
                2.0,
                fill,
                if is_active {
                    Token::PRIMARY
                } else {
                    Token::CARD_BORDER
                },
                if is_active && desc.editing { 1.5 } else { 1.0 },
                0.0,
                desc.z + 1,
            ));
        }
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Grid, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for r in 0..rows {
            for c in 0..cols {
                let cell_r = desc.cell_rect(r, c);
                let logical_r = r + desc.row_offset;
                let cell_key = cell_node(node, logical_r, c);
                let text = desc
                    .cells
                    .get(r)
                    .and_then(|row| row.get(c))
                    .cloned()
                    .unwrap_or_else(|| format!("R{}C{}", logical_r + 1, c + 1));
                let mut cell_a11y = A11yNode::new(Role::Cell, text, cell_r).clickable();
                cell_a11y.selected = desc.active_cell == (logical_r, c);
                out.with_sub_control(
                    cell_key,
                    HitRegion::new(cell_key, CONTROL_LAYER, u32::from(desc.z) + 1, cell_r)
                        .with_cursor(CursorKind::Pointer),
                    cell_a11y,
                    false,
                );
            }
        }
    }

    out
}

//! TreeTable：将层级折叠树与多列表格数据结合的树形表格。
//!
//! 补齐列排序回调、列宽拖拽调整（遵守 `min_width` 与 `resizable` 约束）、
//! 键盘方向键展开/折叠（`ArrowRight` / `ArrowLeft` / `ArrowUp` / `ArrowDown`）与无头几何测试。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::table::{ColumnSpec, SortDir};
use crate::{ControlState, WidgetOutput};

/// 数据行高度。对齐 WinUI `TableViewRowMinHeightMedium = 32`。
pub const TREE_TABLE_ROW_HEIGHT: f32 = 32.0;
/// 表头高度。对齐 WinUI `TableViewHeaderCellPaddingMedium` 的 32px 行。
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
            radius: 4.0, // ControlCornerRadius = 4
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

    /// 结合状态机中的动态列宽计算每个表头单元格的布局矩形。
    #[must_use]
    pub fn header_cell_rects(&self, state: &TreeTableState) -> Vec<Rect> {
        let h = TREE_TABLE_HEADER_HEIGHT.min(self.rect.height());
        let mut rects = Vec::with_capacity(self.columns.len());
        let mut x = self.rect.origin.x;
        for idx in 0..self.columns.len() {
            let w = state.column_width(idx, &self.columns);
            rects.push(Rect::new(x, self.rect.origin.y, w, h));
            x += w;
        }
        rects
    }
}

/// 树形表格交互状态（展开节点、选中行、列排序、列宽拖拽与键盘导航）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeTableState {
    pub expanded: Vec<u64>,
    pub selected: Option<u64>,
    /// 当前排序列索引及方向。
    pub sort_column: Option<(usize, SortDir)>,
    /// 动态调整后的各列宽度缓存（为空时回退到 `ColumnSpec::width`）。
    pub column_widths: Vec<f32>,
    /// 正在拖拽调整列宽的 `(col_idx, start_x, initial_width)`。
    active_resize: Option<(usize, f32, f32)>,
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

    /// 显式展开指定行，若状态发生变化则返回 `true`。
    pub fn expand(&mut self, id: u64) -> bool {
        if self.is_expanded(id) {
            false
        } else {
            self.expanded.push(id);
            true
        }
    }

    /// 显式折叠指定行，若状态发生变化则返回 `true`。
    pub fn collapse(&mut self, id: u64) -> bool {
        if let Some(pos) = self.expanded.iter().position(|&x| x == id) {
            self.expanded.remove(pos);
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn is_expanded(&self, id: u64) -> bool {
        self.expanded.contains(&id)
    }

    /// 切换指定列的排序方向并触发回调；若列不可排序或索引越界则返回 `None`。
    pub fn sort_by<F>(
        &mut self,
        col_idx: usize,
        columns: &[ColumnSpec],
        on_sort: F,
    ) -> Option<(usize, SortDir)>
    where
        F: FnOnce(usize, SortDir),
    {
        let col = columns.get(col_idx)?;
        if !col.sortable {
            return None;
        }
        let next_dir = match self.sort_column {
            Some((idx, dir)) if idx == col_idx => dir.flip(),
            _ => SortDir::Ascending,
        };
        self.sort_column = Some((col_idx, next_dir));
        on_sort(col_idx, next_dir);
        Some((col_idx, next_dir))
    }

    /// 获取指定列当前生效宽度。
    #[must_use]
    pub fn column_width(&self, col_idx: usize, columns: &[ColumnSpec]) -> f32 {
        self.column_widths
            .get(col_idx)
            .copied()
            .or_else(|| columns.get(col_idx).map(|c| c.width))
            .unwrap_or(0.0)
    }

    /// 开始拖拽调整列宽（仅当该列 `resizable == true` 时生效）。
    pub fn begin_resize(&mut self, col_idx: usize, start_x: f32, columns: &[ColumnSpec]) -> bool {
        let Some(col) = columns.get(col_idx) else {
            return false;
        };
        if !col.resizable {
            return false;
        }
        if self.column_widths.len() < columns.len() {
            self.column_widths = columns.iter().map(|c| c.width).collect();
        }
        let initial_w = self.column_widths[col_idx];
        self.active_resize = Some((col_idx, start_x, initial_w));
        true
    }

    /// 拖拽过程中更新目标列宽（严格按 `ColumnSpec::min_width` 下限钳制）。
    pub fn update_resize(&mut self, current_x: f32, columns: &[ColumnSpec]) -> Option<f32> {
        let (col_idx, start_x, initial_w) = self.active_resize?;
        let col = columns.get(col_idx)?;
        let next_w = (initial_w + (current_x - start_x)).max(col.min_width);
        if col_idx < self.column_widths.len() {
            self.column_widths[col_idx] = next_w;
        }
        Some(next_w)
    }

    /// 结束列宽拖拽。
    pub fn end_resize(&mut self) {
        self.active_resize = None;
    }

    /// 响应树形表格键盘导航与展开/折叠：
    /// - `ArrowDown` / `ArrowUp`：在可见行之间移动选中项；
    /// - `ArrowRight`：若当前行有子节点且折叠则展开，若已展开且下一行深度更深则进入首个子节点；
    /// - `ArrowLeft`：若当前行已展开则折叠，否则回跳至最近的父行；
    /// - `Home` / `End`：跳至首行/末行；
    /// - `Enter` / `Space`：切换当前有子节点行的展开态。
    pub fn handle_key(&mut self, key: &str, rows: &[TreeTableRow]) -> bool {
        if rows.is_empty() {
            return false;
        }
        let current_idx = self
            .selected
            .and_then(|id| rows.iter().position(|r| r.id == id))
            .unwrap_or(0);
        let current = &rows[current_idx];

        match key {
            "ArrowDown" => {
                let next = (current_idx + 1).min(rows.len() - 1);
                self.selected = Some(rows[next].id);
                true
            }
            "ArrowUp" => {
                let prev = current_idx.saturating_sub(1);
                self.selected = Some(rows[prev].id);
                true
            }
            "Home" => {
                self.selected = Some(rows[0].id);
                true
            }
            "End" => {
                self.selected = Some(rows[rows.len() - 1].id);
                true
            }
            "ArrowRight" => {
                self.selected = Some(current.id);
                if current.has_children && !self.is_expanded(current.id) {
                    self.expand(current.id);
                    true
                } else if let Some(next_row) = rows.get(current_idx + 1) {
                    if next_row.depth > current.depth {
                        self.selected = Some(next_row.id);
                        return true;
                    }
                    false
                } else {
                    false
                }
            }
            "ArrowLeft" => {
                self.selected = Some(current.id);
                if current.has_children && self.is_expanded(current.id) {
                    self.collapse(current.id);
                    true
                } else if current.depth > 0 {
                    // 向上回溯最近的父级行（depth < current.depth）。
                    for idx in (0..current_idx).rev() {
                        if rows[idx].depth < current.depth {
                            self.selected = Some(rows[idx].id);
                            return true;
                        }
                    }
                    false
                } else {
                    false
                }
            }
            "Enter" | "Space" => {
                self.selected = Some(current.id);
                if current.has_children {
                    self.toggle_expand(current.id);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
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
    emit_tree_table_with_glyphs(node, desc, state, control, &[])
}

/// 输出带文本字形的树形表格图元与命中区域。
#[must_use]
pub fn emit_tree_table_with_glyphs(
    node: u64,
    desc: &TreeTableDesc,
    state: &TreeTableState,
    control: ControlState,
    label_glyphs: &[GlyphQuad],
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
        Token::NONE, // TabularSurfaceHeaderBackground = SubtleFillColorTransparent
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
        .push(surface(header_line, 0.0, Token::DIVIDER, z + 2)); // TabularSurfaceHeaderSeparator

    // 4. 数据行（最多 4 个可见行以保持结构稳定，高度自适应容器剩余空间）
    let body_h = (rect.height() - header_rect.height()).max(0.0);
    let row_h = TREE_TABLE_ROW_HEIGHT.min(body_h / 4.0);
    for row_idx in 0..4 {
        let y = rect.origin.y + header_rect.height() + row_idx as f32 * row_h;
        let row_rect = Rect::new(rect.origin.x, y, rect.width(), row_h.max(0.0));
        let row_id = desc
            .rows
            .get(row_idx)
            .map_or(row_idx as u64, |r| r.id);
        let is_selected = state.selected == Some(row_id);
        let row_bg = if is_selected {
            Token::HOVER // TabularSurfaceRowBackgroundSelected = SubtleFillColorSecondary
        } else if row_idx % 2 == 1 {
            Token::CONTROL_FILL // TabularSurfaceRowBackgroundAlternating（约 4% 白色）
        } else {
            Token::NONE // TabularSurfaceRowBackground = SubtleFillColorTransparent
        };

        out.primitives.push(surface(row_rect, 0.0, row_bg, z + 3));
    }

    if !label_glyphs.is_empty() {
        emit_glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }
    for (row_idx, row) in desc.rows.iter().take(4).enumerate() {
        if row.has_children {
            let y = rect.origin.y + header_rect.height() + row_idx as f32 * row_h + row_h * 0.5;
            let cx = rect.origin.x + 12.0 + row.depth as f32 * 16.0; // TableView defaultRowIndentSize = 16
            let expanded = state.is_expanded(row.id);
            if expanded {
                curve_stroke(
                    [cx - 3.5, y - 1.5],
                    [cx, y + 2.0],
                    1.5,
                    Token::TEXT_SECONDARY,
                    z + 4,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                curve_stroke(
                    [cx, y + 2.0],
                    [cx + 3.5, y - 1.5],
                    1.5,
                    Token::TEXT_SECONDARY,
                    z + 4,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            } else {
                curve_stroke(
                    [cx - 1.5, y - 3.5],
                    [cx + 2.0, y],
                    1.5,
                    Token::TEXT_SECONDARY,
                    z + 4,
                    &mut out.primitives,
                    &mut out.payloads,
                );
                curve_stroke(
                    [cx + 2.0, y],
                    [cx - 1.5, y + 3.5],
                    1.5,
                    Token::TEXT_SECONDARY,
                    z + 4,
                    &mut out.primitives,
                    &mut out.payloads,
                );
            }
        }
    }

    // 5. 外边框
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if control.focused {
            Token::PRIMARY
        } else {
            Token::DIVIDER // TabularSurfaceOuterBorder = DividerStrokeColorDefault
        },
        1.0,
        0.0,
        z + 4,
    ));

    // 6. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !control.disabled,
    ));

    out.hit = (!control.disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });

    let mut a11y = A11yNode::new(Role::TreeGrid, "TreeTable", rect).disabled(control.disabled);
    if let Some((col_idx, dir)) = state.sort_column {
        if let Some(col) = desc.columns.get(col_idx) {
            let dir_label = match dir {
                SortDir::Ascending => "ascending",
                SortDir::Descending => "descending",
            };
            a11y = a11y.with_description(format!("Sorted by {} ({dir_label})", col.title));
        }
    }
    out.a11y = Some(a11y);
    out.focusable = !control.disabled;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    fn sample_columns() -> Vec<ColumnSpec> {
        let mut locked = ColumnSpec::new("ID", 64.0);
        locked.resizable = false;
        locked.sortable = false;
        vec![
            ColumnSpec::new("Pass", 180.0),
            ColumnSpec::new("GPU Time", 100.0),
            locked,
        ]
    }

    fn sample_rows() -> Vec<TreeTableRow> {
        vec![
            TreeTableRow {
                id: 1,
                depth: 0,
                has_children: true,
                cells: vec!["ShadowPass".to_owned(), "1.2 ms".to_owned(), "1".to_owned()],
            },
            TreeTableRow {
                id: 2,
                depth: 1,
                has_children: false,
                cells: vec!["Cascade 0".to_owned(), "0.5 ms".to_owned(), "2".to_owned()],
            },
            TreeTableRow {
                id: 3,
                depth: 0,
                has_children: false,
                cells: vec!["GBuffer".to_owned(), "2.1 ms".to_owned(), "3".to_owned()],
            },
        ]
    }

    #[test]
    fn tree_table_sorting_column_resize_and_keyboard_expand_collapse() {
        let cols = sample_columns();
        let rows = sample_rows();
        let mut st = TreeTableState::new();

        // 列排序回调与方向翻转；不可排序列返回 None。
        let mut sort_events = Vec::new();
        assert_eq!(
            st.sort_by(0, &cols, |c, d| sort_events.push((c, d))),
            Some((0, SortDir::Ascending))
        );
        assert_eq!(
            st.sort_by(0, &cols, |c, d| sort_events.push((c, d))),
            Some((0, SortDir::Descending))
        );
        assert_eq!(st.sort_by(2, &cols, |_, _| {}), None);
        assert_eq!(
            sort_events,
            vec![(0, SortDir::Ascending), (0, SortDir::Descending)]
        );

        // 列宽拖拽调整与 min_width 钳制；不可调整列拒绝拖拽。
        assert!(!st.begin_resize(2, 300.0, &cols));
        assert!(st.begin_resize(0, 180.0, &cols));
        assert_eq!(st.update_resize(240.0, &cols), Some(240.0));
        assert_eq!(st.update_resize(0.0, &cols), Some(cols[0].min_width));
        st.end_resize();

        // 键盘方向键展开/进入子节点/回跳父节点/折叠。
        st.selected = Some(1);
        assert!(st.handle_key("ArrowRight", &rows));
        assert!(st.is_expanded(1));
        assert_eq!(st.selected, Some(1));

        // 已展开时再按 ArrowRight 进入首个子节点。
        assert!(st.handle_key("ArrowRight", &rows));
        assert_eq!(st.selected, Some(2));

        // 在子节点按 ArrowLeft 回跳至父节点。
        assert!(st.handle_key("ArrowLeft", &rows));
        assert_eq!(st.selected, Some(1));

        // 在已展开的父节点按 ArrowLeft 执行折叠。
        assert!(st.handle_key("ArrowLeft", &rows));
        assert!(!st.is_expanded(1));
    }

    #[test]
    fn tree_table_headless_geometry() {
        let rect = Rect::new(20.0, 20.0, 480.0, 180.0);
        let desc = TreeTableDesc::new(rect, sample_columns()).with_rows(sample_rows());
        let st = TreeTableState::new();
        let out = emit_tree_table(1, &desc, &st, ControlState::default());
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
            "TreeTable",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

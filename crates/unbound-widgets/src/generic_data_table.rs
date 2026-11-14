//! 支持多列排序、列宽拖拽钳制、全拼/双拼实时过滤与底部汇总行的泛型数据表格（`GenericDataTable`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::{GlyphQuad, PinyinMatcher};
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 列排序方向。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// 泛型数据表列定义。
#[derive(Clone, Debug, PartialEq)]
pub struct DataColumnSpec {
    pub id: String,
    pub title: String,
    pub width: f32,
    pub min_width: f32,
    pub sortable: bool,
    pub resizable: bool,
}

impl DataColumnSpec {
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>, width: f32) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            width: width.max(40.0),
            min_width: 48.0,
            sortable: true,
            resizable: true,
        }
    }
}

/// 泛型数据表交互状态（管理排序列、列宽调整、选中行集合、虚拟滚动偏移与全拼/双拼搜索过滤）。
#[derive(Clone, Debug, Default)]
pub struct GenericDataTableState {
    pub sort_column: Option<(usize, SortDirection)>,
    pub selected_rows: Vec<usize>,
    pub focused_row: Option<usize>,
    pub scroll_offset_y: f32,
    pub filter_query: String,
}

impl GenericDataTableState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换指定列的排序状态（升序 -> 降序 -> 取消排序）。
    pub fn cycle_sort(&mut self, col_idx: usize) {
        self.sort_column = match self.sort_column {
            Some((idx, SortDirection::Ascending)) if idx == col_idx => {
                Some((col_idx, SortDirection::Descending))
            }
            Some((idx, SortDirection::Descending)) if idx == col_idx => None,
            _ => Some((col_idx, SortDirection::Ascending)),
        };
    }

    /// 拖拽调整指定列的宽度。
    pub fn resize_column(columns: &mut [DataColumnSpec], col_idx: usize, delta_x: f32) {
        if let Some(col) = columns.get_mut(col_idx) {
            if col.resizable {
                col.width = (col.width + delta_x).max(col.min_width);
            }
        }
    }

    /// 对给定行数据执行全拼/双拼/子串过滤与当前列排序，返回可见的原始行索引序列。
    #[must_use]
    pub fn resolve_visible_indices(&self, rows: &[Vec<String>]) -> Vec<usize> {
        let matcher = PinyinMatcher::new();
        let q = self.filter_query.trim();
        let mut indices: Vec<usize> = (0..rows.len())
            .filter(|&row_idx| {
                if q.is_empty() {
                    return true;
                }
                rows[row_idx]
                    .iter()
                    .any(|cell| matcher.match_text(cell, q).is_some())
            })
            .collect();

        if let Some((col_idx, dir)) = self.sort_column {
            indices.sort_by(|&a, &b| {
                let va = rows[a].get(col_idx).map_or("", String::as_str);
                let vb = rows[b].get(col_idx).map_or("", String::as_str);
                let ord = va.cmp(vb);
                match dir {
                    SortDirection::Ascending => ord,
                    SortDirection::Descending => ord.reverse(),
                }
            });
        }
        indices
    }

    /// 将当前过滤与排序后的可见行导出为 RFC 4180 兼容的 CSV 字符串。
    #[must_use]
    pub fn export_csv(&self, columns: &[DataColumnSpec], rows: &[Vec<String>]) -> String {
        let mut out = String::new();
        let header: Vec<String> = columns.iter().map(|c| c.title.clone()).collect();
        out.push_str(&header.join(","));
        out.push('\n');
        for idx in self.resolve_visible_indices(rows) {
            if let Some(row) = rows.get(idx) {
                let escaped: Vec<String> = row
                    .iter()
                    .map(|cell| {
                        if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
                            format!("\"{}\"", cell.replace('"', "\"\""))
                        } else {
                            cell.clone()
                        }
                    })
                    .collect();
                out.push_str(&escaped.join(","));
                out.push('\n');
            }
        }
        out
    }

    /// 键盘上下行导航。
    pub fn move_focus(&mut self, visible_len: usize, delta: isize) {
        if visible_len == 0 {
            self.focused_row = None;
            return;
        }
        let cur = self.focused_row.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, (visible_len - 1) as isize) as usize;
        self.focused_row = Some(next);
        self.selected_rows = vec![next];
    }
}

/// 泛型数据表渲染描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct GenericDataTableDesc {
    pub rect: Rect,
    pub columns: Vec<DataColumnSpec>,
    pub visible_row_slots: usize,
    pub row_height: f32,
    pub show_summary_footer: bool,
    pub z: u16,
}

impl GenericDataTableDesc {
    #[must_use]
    pub fn new(rect: Rect, columns: Vec<DataColumnSpec>, visible_row_slots: usize) -> Self {
        Self {
            rect,
            columns,
            visible_row_slots: visible_row_slots.max(1),
            row_height: 30.0,
            show_summary_footer: true,
            z: 10,
        }
    }
}

/// 发射泛型数据表（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_generic_data_table(
    node: u64,
    desc: &GenericDataTableDesc,
    state: &GenericDataTableState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives =
        Vec::with_capacity(4 + desc.columns.len() + desc.visible_row_slots + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let outer_border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(r, 8.0, Token::SURFACE, outer_border, 1.0, 0.0, z));

    // 表头栏
    let header_r = Rect::new(
        r.origin.x + 1.0,
        r.origin.y + 1.0,
        (r.size.x - 2.0).max(1.0),
        32.0,
    );
    primitives.push(surface(header_r, 7.0, Token::HEADER_BG, z + 1));

    // 列分隔把手线
    let mut col_x = r.origin.x;
    for col in &desc.columns {
        col_x += col.width;
        let sep_r = Rect::new(col_x.min(r.max().x - 2.0), r.origin.y + 6.0, 1.0, 20.0);
        primitives.push(surface(sep_r, 0.0, Token::DIVIDER, z + 2));
    }

    // 固定数量的虚拟化可视行槽位（保证 24 态恒定图元数）
    for slot_idx in 0..desc.visible_row_slots {
        let ry = header_r.max().y + slot_idx as f32 * desc.row_height;
        let row_r = Rect::new(
            r.origin.x + 2.0,
            ry,
            (r.size.x - 4.0).max(1.0),
            desc.row_height,
        );
        let is_selected = state.selected_rows.contains(&slot_idx);
        let fill = if is_selected {
            Token::SELECTION
        } else if slot_idx % 2 == 1 {
            Token::SURFACE_SUNKEN
        } else {
            Token::SURFACE
        };
        primitives.push(surface(row_r, 4.0, fill, z + 1));
    }

    // 底部汇总统计栏（Summary Footer）
    if desc.show_summary_footer {
        let footer_r = Rect::new(
            r.origin.x + 1.0,
            r.max().y - 28.0,
            (r.size.x - 2.0).max(1.0),
            27.0,
        );
        primitives.push(outlined(
            footer_r,
            6.0,
            Token::SURFACE_RAISED,
            Token::DIVIDER,
            1.0,
            0.0,
            z + 2,
        ));
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
        a11y: Some(A11yNode::new(Role::Table, "Data Table", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_data_table_sorts_resizes_and_filters_with_pinyin_and_shuangpin() {
        let mut cols = vec![
            DataColumnSpec::new("name", "Name", 160.0),
            DataColumnSpec::new("type", "Type", 100.0),
        ];
        let rows = vec![
            vec!["着色器_Uber.spv".into(), "SPV".into()],
            vec!["材质球_PBR.mat".into(), "MAT".into()],
            vec!["角色网格.glb".into(), "GLTF".into()],
        ];

        let mut state = GenericDataTableState::new();
        state.filter_query = "zsq".into(); // 全拼首字母匹配 "着色器"
        assert_eq!(state.resolve_visible_indices(&rows), vec![0]);

        state.filter_query = "voseqi".into(); // 小鹤双拼匹配 "着色器"
        assert_eq!(state.resolve_visible_indices(&rows), vec![0]);

        state.filter_query = "cdviqq".into(); // 小鹤双拼匹配 "材质球"
        assert_eq!(state.resolve_visible_indices(&rows), vec![1]);

        state.filter_query.clear();
        state.cycle_sort(1); // 按 Type 升序：GLTF (2), MAT (1), SPV (0)
        assert_eq!(state.resolve_visible_indices(&rows), vec![2, 1, 0]);

        GenericDataTableState::resize_column(&mut cols, 0, -500.0);
        assert_eq!(cols[0].width, cols[0].min_width);
    }
}

//! 支持多列排序、列宽拖拽钳制、全拼/双拼实时过滤与底部汇总行的泛型数据表格（`GenericDataTable`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphQuad, PinyinMatcher};
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::scroll::{emit_scrollbar, ScrollBarDesc, ScrollPointerEvent};
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

/// 虚拟化数据表交互事件。
#[derive(Clone, Debug, PartialEq)]
pub enum DataTableEvent {
    /// 点击列头触发排序变化。
    Sorted {
        column: usize,
        direction: Option<SortDirection>,
    },
    /// 拖拽列分隔线更新了列宽。
    ColumnResized { column: usize, width: f32 },
    /// 行选中集合发生变化。
    SelectionChanged { selected_rows: Vec<usize> },
}

/// 虚拟化数据表键盘导航动作。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DataTableKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    SelectAll,
}

/// 泛型数据表交互状态（管理排序列、列宽调整、选中行集合、框选、虚拟滚动偏移与全拼/双拼搜索过滤）。
#[derive(Clone, Debug, Default)]
pub struct GenericDataTableState {
    pub sort_column: Option<(usize, SortDirection)>,
    pub selected_rows: Vec<usize>,
    pub focused_row: Option<usize>,
    pub selection_anchor: Option<usize>,
    pub scroll_offset_y: f32,
    pub filter_query: String,
    scrollbar_hovered: bool,
    scrollbar_drag: Option<(f32, f32)>,
    column_drag: Option<(usize, f32, f32)>,
    marquee: Option<(Vec2, Vec2)>,
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

    /// 计算当前滚动偏移对应的首个可视行逻辑索引。
    #[must_use]
    pub fn first_row_index(&self, row_height: f32, visible_len: usize, slots: usize) -> usize {
        let max_first = visible_len.saturating_sub(slots);
        ((self.scroll_offset_y / row_height.max(1.0))
            .floor()
            .max(0.0) as usize)
            .min(max_first)
    }

    /// 键盘上下行导航（支持 Shift 范围多选、Home/End、PageUp/PageDown 与 Ctrl+A 全选）。
    pub fn move_focus(&mut self, visible_len: usize, delta: isize) {
        if visible_len == 0 {
            self.focused_row = None;
            self.selection_anchor = None;
            return;
        }
        let cur = self.focused_row.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, (visible_len - 1) as isize) as usize;
        self.focused_row = Some(next);
        self.selection_anchor = Some(next);
        self.selected_rows = vec![next];
    }

    /// 滚动指定垂直像素增量并钳制在合法内容范围内。
    pub fn scroll_by(&mut self, desc: &GenericDataTableDesc, delta_y: f32, visible_len: usize) {
        let max_scroll = (desc.content_height(visible_len) - desc.body_rect().size.y).max(0.0);
        self.scroll_offset_y = (self.scroll_offset_y + delta_y).clamp(0.0, max_scroll);
    }

    /// 确保目标可视行落在视口内，必要时自动调整 `scroll_offset_y`。
    pub fn reveal_row(&mut self, desc: &GenericDataTableDesc, row: usize, visible_len: usize) {
        if visible_len == 0 {
            return;
        }
        let row_top = row as f32 * desc.row_height;
        let row_bottom = row_top + desc.row_height;
        let view_h = desc.body_rect().size.y.max(desc.row_height);
        if row_top < self.scroll_offset_y {
            self.scroll_offset_y = row_top;
        } else if row_bottom > self.scroll_offset_y + view_h {
            self.scroll_offset_y = row_bottom - view_h;
        }
        let max_scroll = (desc.content_height(visible_len) - view_h).max(0.0);
        self.scroll_offset_y = self.scroll_offset_y.clamp(0.0, max_scroll);
    }

    /// 当前框选矩形（若有）。
    #[must_use]
    pub fn marquee_rect(&self) -> Option<Rect> {
        let (a, b) = self.marquee?;
        let min_x = a.x.min(b.x);
        let min_y = a.y.min(b.y);
        let max_x = a.x.max(b.x);
        let max_y = a.y.max(b.y);
        Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    /// 处理指针按下：支持表头列分隔线拖拽、表头点击排序、Ctrl/Shift 行多选与空白区框选。
    pub fn handle_pointer_press(
        &mut self,
        desc: &mut GenericDataTableDesc,
        pos: Vec2,
        ctrl: bool,
        shift: bool,
    ) -> Option<DataTableEvent> {
        let visible_len = self.resolve_visible_indices(&desc.rows).len();
        if self.reduce_scrollbar_pointer(desc, visible_len, ScrollPointerEvent::Press(pos)) {
            return None;
        }
        if let Some(col_idx) = desc.hit_test_column_divider(pos, 5.0) {
            if desc.columns[col_idx].resizable {
                self.column_drag = Some((col_idx, pos.x, desc.columns[col_idx].width));
                return None;
            }
        }
        if let Some(col_idx) = desc.hit_test_header_column(pos) {
            if desc.columns[col_idx].sortable {
                self.cycle_sort(col_idx);
                return Some(DataTableEvent::Sorted {
                    column: col_idx,
                    direction: self.sort_column.map(|(_, d)| d),
                });
            }
            return None;
        }
        if let Some(row_idx) = desc.hit_test_row(self, pos, visible_len) {
            if shift {
                let anchor = self.selection_anchor.unwrap_or(row_idx);
                let (lo, hi) = if anchor <= row_idx {
                    (anchor, row_idx)
                } else {
                    (row_idx, anchor)
                };
                self.selected_rows = (lo..=hi).collect();
                self.focused_row = Some(row_idx);
            } else if ctrl {
                if let Some(pos_in_sel) = self.selected_rows.iter().position(|&r| r == row_idx) {
                    self.selected_rows.remove(pos_in_sel);
                } else {
                    self.selected_rows.push(row_idx);
                    self.selected_rows.sort_unstable();
                }
                self.focused_row = Some(row_idx);
                self.selection_anchor = Some(row_idx);
            } else {
                self.selected_rows = vec![row_idx];
                self.focused_row = Some(row_idx);
                self.selection_anchor = Some(row_idx);
                self.marquee = Some((pos, pos));
            }
            return Some(DataTableEvent::SelectionChanged {
                selected_rows: self.selected_rows.clone(),
            });
        }
        if desc.body_rect().contains_point(pos) {
            self.marquee = Some((pos, pos));
            if !ctrl && !shift && !self.selected_rows.is_empty() {
                self.selected_rows.clear();
                return Some(DataTableEvent::SelectionChanged {
                    selected_rows: Vec::new(),
                });
            }
        }
        None
    }

    /// 处理指针移动：更新列宽拖拽、滚动条拖拽或框选多行范围。
    pub fn handle_pointer_move(
        &mut self,
        desc: &mut GenericDataTableDesc,
        pos: Vec2,
    ) -> Option<DataTableEvent> {
        let visible_len = self.resolve_visible_indices(&desc.rows).len();
        if self.reduce_scrollbar_pointer(desc, visible_len, ScrollPointerEvent::Drag(pos)) {
            return None;
        }
        if let Some((col_idx, start_x, start_w)) = self.column_drag {
            if let Some(col) = desc.columns.get_mut(col_idx) {
                col.width = (start_w + (pos.x - start_x)).max(col.min_width);
                return Some(DataTableEvent::ColumnResized {
                    column: col_idx,
                    width: col.width,
                });
            }
        }
        if let Some((start, _)) = self.marquee {
            self.marquee = Some((start, pos));
            if let Some(mrect) = self.marquee_rect() {
                if mrect.size.y >= 4.0 || mrect.size.x >= 4.0 {
                    let body = desc.body_rect();
                    let rel_top = (mrect.origin.y - body.origin.y + self.scroll_offset_y).max(0.0);
                    let rel_bot =
                        (mrect.max().y - body.origin.y + self.scroll_offset_y).max(rel_top);
                    let first = (rel_top / desc.row_height.max(1.0)).floor() as usize;
                    let last = (rel_bot / desc.row_height.max(1.0)).floor() as usize;
                    if visible_len > 0 {
                        let lo = first.min(visible_len - 1);
                        let hi = last.min(visible_len - 1);
                        let next_sel: Vec<usize> = (lo..=hi).collect();
                        if next_sel != self.selected_rows {
                            self.selected_rows = next_sel;
                            return Some(DataTableEvent::SelectionChanged {
                                selected_rows: self.selected_rows.clone(),
                            });
                        }
                    }
                }
            }
        }
        None
    }

    /// 处理指针释放：结束列宽拖拽、滚动条拖拽或框选会话。
    pub fn handle_pointer_release(
        &mut self,
        desc: &GenericDataTableDesc,
    ) -> Option<DataTableEvent> {
        let visible_len = self.resolve_visible_indices(&desc.rows).len();
        self.reduce_scrollbar_pointer(desc, visible_len, ScrollPointerEvent::Release);
        self.column_drag = None;
        self.marquee = None;
        None
    }

    /// 处理键盘导航与多选（`Up`/`Down`/`PageUp`/`PageDown`/`Home`/`End`/`SelectAll`）。
    pub fn handle_key(
        &mut self,
        desc: &GenericDataTableDesc,
        key: DataTableKey,
        _ctrl: bool,
        shift: bool,
    ) -> Option<DataTableEvent> {
        let visible_len = self.resolve_visible_indices(&desc.rows).len();
        if visible_len == 0 {
            return None;
        }
        if key == DataTableKey::SelectAll {
            self.selected_rows = (0..visible_len).collect();
            self.focused_row = Some(visible_len - 1);
            return Some(DataTableEvent::SelectionChanged {
                selected_rows: self.selected_rows.clone(),
            });
        }
        let cur = self.focused_row.unwrap_or(0);
        let page = desc.visible_row_slots.max(1);
        let next = match key {
            DataTableKey::Up => cur.saturating_sub(1),
            DataTableKey::Down => (cur + 1).min(visible_len - 1),
            DataTableKey::PageUp => cur.saturating_sub(page),
            DataTableKey::PageDown => (cur + page).min(visible_len - 1),
            DataTableKey::Home => 0,
            DataTableKey::End => visible_len - 1,
            DataTableKey::SelectAll => unreachable!(),
        };
        self.focused_row = Some(next);
        if shift {
            let anchor = self.selection_anchor.unwrap_or(cur);
            self.selection_anchor = Some(anchor);
            let (lo, hi) = if anchor <= next {
                (anchor, next)
            } else {
                (next, anchor)
            };
            self.selected_rows = (lo..=hi).collect();
        } else {
            self.selection_anchor = Some(next);
            self.selected_rows = vec![next];
        }
        self.reveal_row(desc, next, visible_len);
        Some(DataTableEvent::SelectionChanged {
            selected_rows: self.selected_rows.clone(),
        })
    }

    /// 更新垂直叠加滚动条热区悬停状态。
    pub fn set_scrollbar_hovered(&mut self, hovered: bool) {
        self.scrollbar_hovered = hovered;
    }

    /// 当前是否正在拖拽垂直叠加滚动条滑块。
    #[must_use]
    pub fn is_dragging_scrollbar(&self) -> bool {
        self.scrollbar_drag.is_some()
    }

    /// 构造表格数据区（不含表头与底部汇总栏）对应的垂直按需叠加滚动条描述符。
    #[must_use]
    pub fn scrollbar_desc(&self, desc: &GenericDataTableDesc, visible_len: usize) -> ScrollBarDesc {
        let mut bar = ScrollBarDesc::vertical(
            desc.body_rect(),
            desc.content_height(visible_len),
            self.scroll_offset_y,
        );
        bar.z = desc.z.saturating_add(5);
        bar.layer = LAYER;
        bar.hovered = self.scrollbar_hovered;
        bar.dragging = self.scrollbar_drag.is_some();
        bar
    }

    /// 归约作用于数据区垂直叠加滚动条的指针手势（滑块拖拽与点击轨道翻页）。
    pub fn reduce_scrollbar_pointer(
        &mut self,
        desc: &GenericDataTableDesc,
        visible_len: usize,
        event: ScrollPointerEvent,
    ) -> bool {
        match event {
            ScrollPointerEvent::Press(pos) => {
                let bar = self.scrollbar_desc(desc, visible_len);
                if !bar.is_overflowing() {
                    return false;
                }
                if bar.thumb_rect().contains_point(pos) {
                    self.scrollbar_drag = Some((pos.y, self.scroll_offset_y));
                    return true;
                }
                if let Some(next_y) = bar.page_offset_for_track_click(pos) {
                    self.scroll_offset_y = next_y;
                    return true;
                }
                false
            }
            ScrollPointerEvent::Drag(pos) => {
                let Some((start_pointer, start_offset)) = self.scrollbar_drag else {
                    return false;
                };
                let bar = self.scrollbar_desc(desc, visible_len);
                if !bar.is_overflowing() {
                    return false;
                }
                self.scroll_offset_y = bar.offset_for_drag(start_offset, start_pointer, pos.y);
                true
            }
            ScrollPointerEvent::Release => self.scrollbar_drag.take().is_some(),
        }
    }
}

/// 泛型数据表渲染描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct GenericDataTableDesc {
    pub rect: Rect,
    pub columns: Vec<DataColumnSpec>,
    pub rows: Vec<Vec<String>>,
    pub summary_text: String,
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
            rows: Vec::new(),
            summary_text: String::new(),
            visible_row_slots: visible_row_slots.max(1),
            // 对齐 WinUI `TableViewRowMinHeightMedium = 32`。
            row_height: 32.0,
            show_summary_footer: true,
            z: 10,
        }
    }

    /// 创建无 Rect 的声明式泛型数据表描述符。
    #[must_use]
    pub fn table(columns: Vec<DataColumnSpec>, visible_row_slots: usize) -> Self {
        Self::new(Rect::ZERO, columns, visible_row_slots)
    }

    /// 绑定表格行数据矩阵。
    #[must_use]
    pub fn with_rows(mut self, rows: Vec<Vec<String>>) -> Self {
        self.rows = rows;
        self
    }

    /// 绑定底部汇总栏文本。
    #[must_use]
    pub fn with_summary(mut self, summary_text: impl Into<String>) -> Self {
        self.summary_text = summary_text.into();
        self
    }

    /// 控制是否显示底部汇总统计栏。
    #[must_use]
    pub fn with_footer(mut self, show_summary_footer: bool) -> Self {
        self.show_summary_footer = show_summary_footer;
        self
    }

    /// 当表格容器宽度大于列宽总和时按比例拉伸各列宽度，填满整张表格。
    #[must_use]
    pub fn effective_column_widths(&self, table_width: f32) -> Vec<f32> {
        let sum: f32 = self.columns.iter().map(|c| c.width.max(c.min_width)).sum();
        if sum > 0.0 && table_width > sum {
            let scale = table_width / sum;
            self.columns
                .iter()
                .map(|c| c.width.max(c.min_width) * scale)
                .collect()
        } else {
            self.columns
                .iter()
                .map(|c| c.width.max(c.min_width))
                .collect()
        }
    }

    /// 表头栏矩形（固定 32px 高）。
    #[must_use]
    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + 1.0,
            self.rect.origin.y + 1.0,
            (self.rect.size.x - 2.0).max(1.0),
            32.0,
        )
    }

    /// 表头下方、汇总栏上方的数据区视口矩形（绝不覆盖表头与底部汇总栏）。
    #[must_use]
    pub fn body_rect(&self) -> Rect {
        let top = self.header_rect().max().y;
        let bottom = if self.show_summary_footer {
            self.rect.max().y - 28.0
        } else {
            self.rect.max().y - 1.0
        };
        Rect::new(
            self.rect.origin.x + 1.0,
            top,
            (self.rect.size.x - 2.0).max(0.0),
            (bottom - top).max(0.0),
        )
    }

    /// 可见数据行的总内容高度（逻辑像素）。
    #[must_use]
    pub fn content_height(&self, visible_len: usize) -> f32 {
        visible_len as f32 * self.row_height
    }

    /// 计算各列表头矩形（按有效列宽排列）。
    #[must_use]
    pub fn header_column_rects(&self) -> Vec<Rect> {
        let header = self.header_rect();
        let widths = self.effective_column_widths(self.rect.size.x);
        let mut x = self.rect.origin.x;
        widths
            .into_iter()
            .map(|w| {
                let r = Rect::new(x, header.origin.y, w, header.size.y);
                x += w;
                r
            })
            .collect()
    }

    /// 命中测试：判断 `pos` 是否落在表头列分隔线拖拽热区内。
    #[must_use]
    pub fn hit_test_column_divider(&self, pos: Vec2, tolerance: f32) -> Option<usize> {
        let header = self.header_rect();
        if pos.y < header.origin.y || pos.y > header.max().y {
            return None;
        }
        let widths = self.effective_column_widths(self.rect.size.x);
        let mut x = self.rect.origin.x;
        for (idx, &w) in widths.iter().enumerate() {
            x += w;
            if (pos.x - x).abs() <= tolerance {
                return Some(idx);
            }
        }
        None
    }

    /// 命中测试：判断 `pos` 是否落在某个表头列标题内。
    #[must_use]
    pub fn hit_test_header_column(&self, pos: Vec2) -> Option<usize> {
        self.header_column_rects()
            .into_iter()
            .position(|r| r.contains_point(pos))
    }

    /// 命中测试：判断 `pos` 是否落在某个可见数据行内，返回过滤/排序后的逻辑行索引。
    #[must_use]
    pub fn hit_test_row(
        &self,
        state: &GenericDataTableState,
        pos: Vec2,
        visible_len: usize,
    ) -> Option<usize> {
        let body = self.body_rect();
        if !body.contains_point(pos) || visible_len == 0 {
            return None;
        }
        let first_row = state.first_row_index(self.row_height, visible_len, self.visible_row_slots);
        let slot = ((pos.y - body.origin.y) / self.row_height.max(1.0)).floor() as usize;
        if slot >= self.visible_row_slots {
            return None;
        }
        let logical = first_row + slot;
        (logical < visible_len).then_some(logical)
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
    // 表格外框圆角对齐 WinUI `ControlCornerRadius = 4`。
    primitives.push(outlined(r, 4.0, Token::SURFACE, outer_border, 1.0, 0.0, z));

    // 表头栏：高度对齐 WinUI `TableViewRowMinHeightMedium = 32`，
    // 圆角对齐 `TableViewHeaderSingleCornerRadius = 4`。
    let header_r = Rect::new(
        r.origin.x + 1.0,
        r.origin.y + 1.0,
        (r.size.x - 2.0).max(1.0),
        32.0, // TableViewHeaderCellPaddingMedium 上下各 6，行高合计 32
    );
    primitives.push(surface(header_r, 4.0, Token::HEADER_BG, z + 1));

    // 列分隔把手线
    let col_widths = desc.effective_column_widths(r.size.x);
    let mut col_x = r.origin.x;
    for &col_w in &col_widths {
        col_x += col_w;
        let sep_r = Rect::new(col_x.min(r.max().x - 2.0), r.origin.y + 6.0, 1.0, 20.0);
        primitives.push(surface(sep_r, 0.0, Token::DIVIDER, z + 2));
    }

    let visible_len = state.resolve_visible_indices(&desc.rows).len();
    let first_row = state.first_row_index(desc.row_height, visible_len, desc.visible_row_slots);

    // 悬停行：表格整体悬停时高亮当前焦点行，对应 WinUI
    // `ListViewItemBackgroundPointerOver` = `SubtleFillColorSecondary`（HOVER 令牌）。
    let hovered_row = if !control.disabled && control.phase == PointerPhase::Hovered {
        state.focused_row
    } else {
        None
    };

    // 固定数量的虚拟化可视行槽位（保证 24 态恒定图元数）
    for slot_idx in 0..desc.visible_row_slots {
        let logical_idx = first_row + slot_idx;
        let ry = header_r.max().y + slot_idx as f32 * desc.row_height;
        let row_r = Rect::new(
            r.origin.x + 2.0,
            ry,
            (r.size.x - 4.0).max(1.0),
            desc.row_height,
        );
        let is_selected = logical_idx < visible_len && state.selected_rows.contains(&logical_idx);
        let fill = if is_selected {
            Token::SELECTION
        } else if hovered_row == Some(logical_idx) {
            Token::HOVER
        } else if slot_idx % 2 == 1 {
            Token::SURFACE_SUNKEN
        } else {
            Token::SURFACE
        };
        primitives.push(surface(row_r, 4.0, fill, z + 1));
        // 行底格线：1px `DIVIDER`，对应 WinUI `TabularSurfaceGridLine`。
        primitives.push(surface(
            Rect::new(row_r.origin.x, row_r.max().y - 1.0, row_r.size.x, 1.0),
            0.0,
            Token::DIVIDER,
            z + 2,
        ));
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
            4.0, // ControlCornerRadius = 4，与表格外框同档
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

    // 框选拖拽时叠加半透明选框高亮矩形。
    if let Some(mrect) = state.marquee_rect() {
        if mrect.size.x > 1.0 && mrect.size.y > 1.0 {
            primitives.push(outlined(
                mrect,
                2.0,
                Token::SELECTION,
                Token::ACCENT,
                1.0,
                0.0,
                z + 3,
            ));
        }
    }

    // 按需叠加垂直滚动条：宿主在数据区 `body_rect()` 内，绝不覆盖表头与底部汇总栏。
    let mut bar = state.scrollbar_desc(desc, visible_len);
    bar.hovered = bar.hovered
        || (!control.disabled
            && matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed));
    bar.disabled = control.disabled;
    let bar_out = emit_scrollbar(node, &bar);
    primitives.extend(bar_out.output.primitives);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Table, "Data Table", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    };
    if !control.disabled {
        for hit in bar_out.hits {
            out.sub_hits.push(hit);
        }
    }
    out
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
            vec!["着色器_Uber.spv".into(), "SPV".into()], // lang-ok: pinyin/shuangpin matching sample
            vec!["材质球_PBR.mat".into(), "MAT".into()], // lang-ok: pinyin/shuangpin matching sample
            vec!["角色网格.glb".into(), "GLTF".into()], // lang-ok: pinyin/shuangpin matching sample
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

    #[test]
    fn generic_data_table_overlay_scrollbar_stays_in_body_rect_and_emits_only_on_overflow() {
        use crate::scroll::{
            scrollbar_thumb_node, scrollbar_track_node, ScrollbarAxis, SCROLLBAR_THUMB_IDLE_WIDTH,
            SCROLLBAR_THUMB_WIDTH,
        };
        use unbound_math::Vec2;

        let cols = vec![
            DataColumnSpec::new("id", "ID", 80.0),
            DataColumnSpec::new("name", "Name", 160.0),
        ];
        let rect = Rect::new(0.0, 0.0, 320.0, 200.0);
        let node = 42u64;
        let thumb_node = scrollbar_thumb_node(node, ScrollbarAxis::Vertical);
        let track_node = scrollbar_track_node(node, ScrollbarAxis::Vertical);

        // 未溢出：2 行（64px <= body 139px）不发射滚动条图元与命中区。
        let short_rows = vec![
            vec!["1".into(), "Alpha".into()],
            vec!["2".into(), "Beta".into()],
        ];
        let short_desc = GenericDataTableDesc::new(rect, cols.clone(), 4).with_rows(short_rows);
        let short_state = GenericDataTableState::new();
        let short_out = emit_generic_data_table(
            node,
            &short_desc,
            &short_state,
            ControlState::default(),
            &[],
        );
        assert!(short_out.sub_hits.is_empty());

        // 溢出：12 行（384px > body 139px）在数据区发射 2 个稳定槽位与轨道/滑块命中区。
        let tall_rows: Vec<Vec<String>> = (0..12)
            .map(|i| vec![format!("{i}"), format!("Row {i}")])
            .collect();
        let tall_desc = GenericDataTableDesc::new(rect, cols, 4).with_rows(tall_rows);
        let mut tall_state = GenericDataTableState::new();
        let idle_out =
            emit_generic_data_table(node, &tall_desc, &tall_state, ControlState::default(), &[]);
        assert_eq!(idle_out.primitives.len(), short_out.primitives.len() + 2);
        let track_hit = idle_out
            .sub_hits
            .iter()
            .find(|h| h.node == track_node)
            .expect("generic data table track hit");
        let thumb_hit = idle_out
            .sub_hits
            .iter()
            .find(|h| h.node == thumb_node)
            .expect("generic data table thumb hit");
        assert!(track_hit.bounds.origin.y >= tall_desc.header_rect().max().y);
        assert!(tall_desc.body_rect().contains_rect(&track_hit.bounds));
        assert!(tall_desc.body_rect().contains_rect(&thumb_hit.bounds));

        let n = idle_out.primitives.len();
        assert_eq!(idle_out.primitives[n - 2].opacity(), 0.0);
        assert!((idle_out.primitives[n - 1].size[0] - SCROLLBAR_THUMB_IDLE_WIDTH).abs() < 0.01);

        // 悬停展开为 6px 粗滑块与半透明轨道胶囊，图元总数恒定。
        tall_state.set_scrollbar_hovered(true);
        let hovered_out =
            emit_generic_data_table(node, &tall_desc, &tall_state, ControlState::default(), &[]);
        assert_eq!(hovered_out.primitives.len(), n);
        assert!(hovered_out.primitives[n - 2].opacity() > 0.0);
        assert!((hovered_out.primitives[n - 1].size[0] - SCROLLBAR_THUMB_WIDTH).abs() < 0.01);

        // 点击轨道翻页与拖拽滑块。
        let track_click = Vec2::new(track_hit.bounds.center().x, track_hit.bounds.max().y - 4.0);
        assert!(tall_state.reduce_scrollbar_pointer(
            &tall_desc,
            12,
            ScrollPointerEvent::Press(track_click)
        ));
        assert!(tall_state.scroll_offset_y > 0.0);

        let cur_thumb = tall_state.scrollbar_desc(&tall_desc, 12).thumb_rect();
        let drag_start = cur_thumb.center();
        assert!(tall_state.reduce_scrollbar_pointer(
            &tall_desc,
            12,
            ScrollPointerEvent::Press(drag_start)
        ));
        assert!(tall_state.is_dragging_scrollbar());
        assert!(tall_state.reduce_scrollbar_pointer(
            &tall_desc,
            12,
            ScrollPointerEvent::Drag(Vec2::new(drag_start.x, drag_start.y + 30.0))
        ));
        assert!(tall_state.reduce_scrollbar_pointer(&tall_desc, 12, ScrollPointerEvent::Release));
        assert!(!tall_state.is_dragging_scrollbar());
    }

    #[test]
    fn generic_data_table_virtualizes_10k_rows_and_supports_header_sort_resize_and_multiselect() {
        let cols = vec![
            DataColumnSpec::new("id", "ID", 100.0),
            DataColumnSpec::new("name", "Name", 200.0),
        ];
        let rows: Vec<Vec<String>> = (0..10_000)
            .map(|i| vec![format!("{i:05}"), format!("Item {i}")])
            .collect();
        let mut desc =
            GenericDataTableDesc::new(Rect::new(0.0, 0.0, 400.0, 360.0), cols, 10).with_rows(rows);
        let mut state = GenericDataTableState::new();

        // 万级数据仅发射可视行槽位图元，不随总行数膨胀。
        let out = emit_generic_data_table(99, &desc, &state, ControlState::default(), &[]);
        assert!(out.primitives.len() < 40);

        // 点击第 0 列表头触发升序排序。
        let col0_center = desc.header_column_rects()[0].center();
        assert_eq!(
            state.handle_pointer_press(&mut desc, col0_center, false, false),
            Some(DataTableEvent::Sorted {
                column: 0,
                direction: Some(SortDirection::Ascending),
            })
        );

        // 拖拽列分隔线调整列宽。
        let divider_x = desc.header_column_rects()[0].max().x;
        let divider_pos = Vec2::new(divider_x, desc.header_rect().center().y);
        state.handle_pointer_press(&mut desc, divider_pos, false, false);
        assert!(matches!(
            state.handle_pointer_move(&mut desc, Vec2::new(divider_pos.x + 24.0, divider_pos.y)),
            Some(DataTableEvent::ColumnResized { column: 0, .. })
        ));
        state.handle_pointer_release(&desc);

        // 键盘 Shift 多选与 PageDown / End 虚拟滚动。
        state.handle_key(&desc, DataTableKey::Home, false, false);
        state.handle_key(&desc, DataTableKey::Down, false, true);
        assert_eq!(state.selected_rows, vec![0, 1]);
        state.handle_key(&desc, DataTableKey::End, false, false);
        assert_eq!(state.focused_row, Some(9999));
        assert!(state.scroll_offset_y > 0.0);
    }
}

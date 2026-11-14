//! 虚拟化数据表格：描述符、行窗口、选区与输出器。
//!
//! 表格从不持有数据模型。[`TableDesc`] 描述列定义与固定行节奏；
//! [`TableState`] 持有滚动偏移、排序键与选中行集合。调用方将*可见*行连同塑形字形传给 [`emit_table`]，
//! 因此即使十万行数据集每帧也仅输出十几个行背景。
//!
//!
//!
//! 排序是一个*请求*而非原地变更：[`TableState::sort_by`] 翻转排序方向并返回它，
//! 由应用程序在下一帧前重排自身模型，从而避免行标识跟踪问题。
//!

use std::ops::Range;

use accesskit::Role;
use rustc_hash::FxHashSet;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, surface};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 单元格内容的水平对齐方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignX {
    /// 前导对齐（左对齐）。
    #[default]
    Start,
    /// 居中对齐。
    Center,
    /// 末尾对齐（右对齐）。
    End,
}

impl AlignX {
    /// `width` 宽单元格在 `span` 宽列内的 x 偏移。
    #[must_use]
    pub fn offset(self, span: f32, width: f32) -> f32 {
        match self {
            Self::Start => 0.0,
            Self::Center => ((span - width) * 0.5).max(0.0),
            Self::End => (span - width).max(0.0),
        }
    }
}

/// 单列的排序方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    /// 升序（最小优先）。
    Ascending,
    /// 降序（最大优先）。
    Descending,
}

impl SortDir {
    /// 相反的排序方向。
    #[must_use]
    pub fn flip(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// 单列的布局策略。
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSpec {
    /// 表头文本。
    pub title: String,
    /// 首选宽度。
    pub width: f32,
    /// 调整列宽时的最小允许宽度。
    pub min_width: f32,
    /// 用户是否可拖拽右边缘调整列宽。
    pub resizable: bool,
    /// 点击表头是否切换排序。
    pub sortable: bool,
    /// 列是否固定在左侧前导边缘。
    pub frozen: bool,
    /// 单元格对齐方式。
    pub align: AlignX,
}

impl ColumnSpec {
    /// 创建宽度为 `width`、可调整宽度且可排序的列。
    #[must_use]
    pub fn new(title: impl Into<String>, width: f32) -> Self {
        Self {
            title: title.into(),
            width: width.max(1.0),
            min_width: 48.0,
            resizable: true,
            sortable: true,
            frozen: false,
            align: AlignX::Start,
        }
    }

    /// 设置为右对齐（常用于数字）。
    #[must_use]
    pub fn right_aligned(mut self) -> Self {
        self.align = AlignX::End;
        self
    }

    /// 冻结在左侧前导边缘。
    #[must_use]
    pub fn frozen(mut self) -> Self {
        self.frozen = true;
        self
    }

    /// 设置自定义最小宽度。
    #[must_use]
    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width.max(1.0);
        self
    }
}

/// 表格的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TableDesc {
    /// 整个表格矩形（包含表头）。
    pub rect: Rect,
    /// 无障碍名称。
    pub label: String,
    /// 按视觉顺序排列的列列表。
    pub columns: Vec<ColumnSpec>,
    /// 表头高度。
    pub header_height: f32,
    /// 统一行高。
    pub row_height: f32,
    /// 是否启用斑马纹交替行背景。
    pub striped: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl TableDesc {
    /// 创建带 32 px 表头与 28 px 行高的表格。
    #[must_use]
    pub fn new(rect: Rect, columns: Vec<ColumnSpec>) -> Self {
        Self {
            rect,
            label: String::new(),
            columns,
            header_height: 32.0,
            row_height: 28.0,
            striped: true,
            z: 640,
        }
    }

    /// 设置无障碍名称。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 所有列的总宽度。
    #[must_use]
    pub fn total_width(&self) -> f32 {
        self.columns.iter().map(|column| column.width).sum()
    }

    /// 表头区域矩形。
    #[must_use]
    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            self.header_height,
        )
    }

    /// 表头下方的主体区域矩形。
    #[must_use]
    pub fn body_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + self.header_height,
            self.rect.size.x,
            (self.rect.size.y - self.header_height).max(0.0),
        )
    }

    /// 可用于显示行内容的可见高度。
    #[must_use]
    pub fn body_height(&self) -> f32 {
        self.body_rect().size.y
    }

    /// 主体区域可容纳的行数。
    #[must_use]
    pub fn visible_row_count(&self) -> usize {
        let rows = self.body_height() / self.row_height.max(1.0);
        rows.floor().max(0.0) as usize
    }

    /// 指定列相对于表格前导边缘的 x 偏移。
    #[must_use]
    pub fn column_x(&self, index: usize) -> f32 {
        self.columns
            .iter()
            .take(index)
            .map(|column| column.width)
            .sum()
    }

    /// 单个表头单元格矩形。
    #[must_use]
    pub fn column_rect(&self, index: usize) -> Option<Rect> {
        let column = self.columns.get(index)?;
        Some(Rect::new(
            self.rect.origin.x + self.column_x(index),
            self.rect.origin.y,
            column.width,
            self.header_height,
        ))
    }

    /// 用户拖拽以调整第 `index` 列宽度的分隔条矩形。
    #[must_use]
    pub fn resize_handle(&self, index: usize) -> Option<Rect> {
        let column = self.columns.get(index)?;
        if !column.resizable {
            return None;
        }
        Some(Rect::new(
            self.rect.origin.x + self.column_x(index) + column.width - 3.0,
            self.rect.origin.y,
            6.0,
            self.rect.size.y,
        ))
    }

    /// 最后一个冻结列右边缘的 x 坐标。
    #[must_use]
    pub fn frozen_edge(&self) -> Option<f32> {
        self.columns
            .iter()
            .position(|column| !column.frozen)
            .map(|first_scrolling| self.column_x(first_scrolling))
            .filter(|_| self.columns.first().is_some_and(|column| column.frozen))
    }

    /// 计入 `scroll` 像素偏移后，指定行在主体区域中的矩形。
    #[must_use]
    pub fn row_rect(&self, row: usize, scroll: f32) -> Rect {
        let body = self.body_rect();
        Rect::new(
            body.origin.x,
            body.origin.y + row as f32 * self.row_height - scroll,
            body.size.x,
            self.row_height,
        )
    }
}

/// 当前处于视口内的行索引范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowWindow {
    /// 首个可见行（包含）。
    pub first: usize,
    /// 末尾可见行（不包含）。
    pub last: usize,
}

impl RowWindow {
    /// 窗口内的行数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.last.saturating_sub(self.first)
    }

    /// 窗口是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 迭代行索引。
    pub fn iter(&self) -> Range<usize> {
        self.first..self.last
    }
}

/// 表格的可变状态。
#[derive(Clone, Debug, Default)]
pub struct TableState {
    scroll: ScrollState,
    sort: Option<(usize, SortDir)>,
    selected: FxHashSet<usize>,
    widths: Vec<f32>,
    resizing: Option<(usize, f32, f32)>,
}

impl TableState {
    /// 创建未排序、无选区的初始表格状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 滚动状态引用。
    #[must_use]
    pub fn scroll(&self) -> &ScrollState {
        &self.scroll
    }

    /// 垂直滚动偏移量。
    #[must_use]
    pub fn offset(&self) -> f32 {
        self.scroll.offset().y
    }

    /// 按 `delta` 像素滚动并夹紧至内容范围。
    pub fn scroll_by(&mut self, desc: &TableDesc, row_count: usize, delta: f32) -> bool {
        let content = row_count as f32 * desc.row_height;
        let view = ScrollViewDesc::new(desc.body_rect(), unbound_math::Vec2::new(0.0, content));
        let before = self.scroll.target();
        self.scroll
            .scroll_by(&view, unbound_math::Vec2::new(0.0, delta));
        self.scroll.target() != before
    }

    /// 推进滚动动画。
    pub fn step(&mut self, desc: &TableDesc, row_count: usize, dt: f32) -> bool {
        let content = row_count as f32 * desc.row_height;
        let view = ScrollViewDesc::new(desc.body_rect(), unbound_math::Vec2::new(0.0, content));
        self.scroll.step(&view, dt)
    }

    /// 当前处于视口内的行范围。
    #[must_use]
    pub fn window(&self, desc: &TableDesc, row_count: usize) -> RowWindow {
        if row_count == 0 || desc.body_height() <= 0.0 {
            return RowWindow { first: 0, last: 0 };
        }
        let row_height = desc.row_height.max(1.0);
        let offset = self.scroll.target().y.max(0.0);
        let first = (offset / row_height).floor() as usize;
        let first = first.min(row_count.saturating_sub(1));
        let rows = (desc.body_height() / row_height).ceil().max(1.0) as usize;
        RowWindow {
            first,
            last: (first + rows).min(row_count),
        }
    }

    /// 当前排序键。
    #[must_use]
    pub fn sort(&self) -> Option<(usize, SortDir)> {
        self.sort
    }

    /// 切换 `column` 列的排序方向并返回新方向。
    pub fn sort_by(&mut self, column: usize) -> Option<(usize, SortDir)> {
        self.sort = match self.sort {
            Some((active, direction)) if active == column => Some((column, direction.flip())),
            _ => Some((column, SortDir::Ascending)),
        };
        self.sort
    }

    /// 已选中的行集合。
    #[must_use]
    pub fn selected(&self) -> &FxHashSet<usize> {
        &self.selected
    }

    /// 查询 `row` 是否处于选中状态。
    #[must_use]
    pub fn is_selected(&self, row: usize) -> bool {
        self.selected.contains(&row)
    }

    /// 单选指定行并清空其余选区。
    pub fn select_row(&mut self, row: usize) {
        self.selected.clear();
        self.selected.insert(row);
    }

    /// 切换 `row` 的选中状态，不影响其余行。
    pub fn toggle_row(&mut self, row: usize) {
        if !self.selected.remove(&row) {
            self.selected.insert(row);
        }
    }

    /// 清空选区。
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// 计入用户调整后的第 `index` 列有效宽度。
    #[must_use]
    pub fn column_width(&self, desc: &TableDesc, index: usize) -> f32 {
        self.widths
            .get(index)
            .copied()
            .or_else(|| desc.columns.get(index).map(|column| column.width))
            .unwrap_or(0.0)
    }

    /// 开始拖拽第 `index` 列的右边缘。
    pub fn begin_resize(&mut self, index: usize, x: f32, width: f32) {
        self.resizing = Some((index, x, width));
    }

    /// 更新列宽拖拽；返回宽度是否发生变化。
    pub fn resize_to(&mut self, desc: &TableDesc, x: f32) -> bool {
        let Some((index, origin, width)) = self.resizing else {
            return false;
        };
        let min = desc
            .columns
            .get(index)
            .map_or(16.0, |column| column.min_width);
        let next = (width + x - origin).max(min);
        if self.widths.len() <= index {
            self.widths.resize(index + 1, 0.0);
            for (slot, column) in self.widths.iter_mut().zip(&desc.columns) {
                if *slot <= 0.0 {
                    *slot = column.width;
                }
            }
        }
        let changed = (self.widths[index] - next).abs() > f32::EPSILON;
        self.widths[index] = next;
        changed
    }

    /// 结束列宽拖拽。
    pub fn end_resize(&mut self) {
        self.resizing = None;
    }

    /// 当前是否正在拖拽调整列宽。
    #[must_use]
    pub fn is_resizing(&self) -> bool {
        self.resizing.is_some()
    }
}

/// 表格绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableTokens {
    /// 表头表面背景。
    pub header: Token,
    /// 表头文本。
    pub header_text: Token,
    /// 主体数据行。
    pub row: Token,
    /// 交替主体数据行。
    pub row_alt: Token,
    /// 已选中行。
    pub selected: Token,
    /// 行与列分隔线。
    pub rule: Token,
    /// 冻结列分隔线。
    pub frozen_rule: Token,
    /// 主体文本。
    pub text: Token,
}

impl TableTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            header: Token::HEADER_BG,
            header_text: Token::TEXT_SECONDARY,
            row: Token::SURFACE,
            row_alt: Token::SURFACE_SUNKEN,
            selected: Token::SELECTION,
            rule: Token::DIVIDER,
            frozen_rule: Token::OUTLINE_STRONG,
            text: Token::TEXT_PRIMARY,
        }
    }
}

/// 单个可见行及其已塑形的字形。
#[derive(Clone, Copy, Debug)]
pub struct TableRow<'a> {
    /// 模型中的行索引。
    pub index: usize,
    /// 调用方已定位好的单元格文本。
    pub glyphs: &'a [GlyphQuad],
}

/// 输出表头、可见行与无障碍网格。
#[must_use]
pub fn emit_table(
    node: u64,
    desc: &TableDesc,
    state: &TableState,
    rows: &[TableRow<'_>],
    header_glyphs: &[GlyphQuad],
    control: ControlState,
) -> WidgetOutput {
    let tokens = TableTokens::standard();
    let mut primitives = Vec::with_capacity(6 + rows.len() * 4 + header_glyphs.len());
    let mut payloads = Vec::with_capacity(rows.len() + header_glyphs.len());

    primitives.push(surface(desc.header_rect(), 0.0, tokens.header, desc.z));
    primitives.push(line(
        Rect::new(
            desc.rect.origin.x,
            desc.rect.origin.y + desc.header_height - 1.0,
            desc.rect.size.x,
            1.0,
        ),
        tokens.rule,
        desc.z + 1,
    ));
    glyphs(
        header_glyphs,
        tokens.header_text,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    for row in rows {
        let rect = desc.row_rect(row.index, state.offset());
        if rect.origin.y + rect.size.y <= desc.body_rect().origin.y
            || rect.origin.y >= desc.body_rect().max().y
        {
            continue;
        }
        let base = if desc.striped && row.index % 2 == 1 {
            tokens.row_alt
        } else {
            tokens.row
        };
        primitives.push(surface(rect, 0.0, base, desc.z));
        if state.is_selected(row.index) {
            primitives.push(surface(rect, 0.0, tokens.selected, desc.z + 1));
        }
        primitives.push(line(
            Rect::new(rect.origin.x, rect.max().y - 1.0, rect.size.x, 1.0),
            tokens.rule,
            desc.z + 2,
        ));
        glyphs(
            row.glyphs,
            tokens.text,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    if let Some(edge) = desc.frozen_edge() {
        primitives.push(line(
            Rect::new(
                desc.rect.origin.x + edge,
                desc.rect.origin.y,
                1.0,
                desc.rect.size.y,
            ),
            tokens.frozen_rule,
            desc.z + 4,
        ));
    }

    primitives.push(focus_ring(
        desc.rect,
        0.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 5,
        control.focused,
    ));

    let mut a11y = A11yNode::new(Role::Grid, desc.label.clone(), desc.rect).clickable();
    a11y.description = format!("{} 行 × {} 列", rows.len(), desc.columns.len());
    a11y.disabled = control.disabled;

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rect)
                .with_cursor(CursorKind::Default),
        ),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    };

    if !control.disabled {
        let mut col_x = desc.rect.origin.x;
        let header_y = desc.rect.origin.y;
        let header_h = desc.header_height;
        for (col_idx, col) in desc.columns.iter().enumerate() {
            let w = col.width;
            let header_r = Rect::new(col_x, header_y, (w - 4.0).max(4.0), header_h);
            let h_key = column_header_node(node, col_idx);
            out.with_sub_control(
                h_key,
                HitRegion::new(h_key, CONTROL_LAYER, u32::from(desc.z) + 2, header_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::ColumnHeader, col.title.clone(), header_r).clickable(),
                false,
            );
            let resize_r = Rect::new(col_x + w - 4.0, header_y, 8.0, header_h);
            let r_key = column_resize_node(node, col_idx);
            out.with_sub_control(
                r_key,
                HitRegion::new(r_key, CONTROL_LAYER, u32::from(desc.z) + 3, resize_r)
                    .with_cursor(CursorKind::ResizeEastWest),
                A11yNode::new(Role::Splitter, format!("Resize {}", col.title), resize_r),
                false,
            );
            col_x += w;
        }
        for row in rows {
            let rect = desc.row_rect(row.index, state.offset());
            if rect.origin.y + rect.size.y <= desc.body_rect().origin.y
                || rect.origin.y >= desc.body_rect().max().y
            {
                continue;
            }
            let row_key = table_row_node(node, row.index);
            let mut row_a11y =
                A11yNode::new(Role::Row, format!("Row {}", row.index + 1), rect).clickable();
            row_a11y.selected = state.is_selected(row.index);
            out.with_sub_control(
                row_key,
                HitRegion::new(row_key, CONTROL_LAYER, u32::from(desc.z) + 1, rect)
                    .with_cursor(CursorKind::Pointer),
                row_a11y,
                false,
            );
        }
    }

    out
}

/// 列 `col` 表头的确定性子节点键。
#[must_use]
pub const fn column_header_node(base: u64, col: usize) -> u64 {
    base ^ (0x5442_4800_u64 | (col as u64 + 1))
}

/// 列 `col` 调整手柄的确定性子节点键。
#[must_use]
pub const fn column_resize_node(base: u64, col: usize) -> u64 {
    base ^ (0x5442_5200_u64 | (col as u64 + 1))
}

/// 行 `index` 的确定性子节点键。
#[must_use]
pub const fn table_row_node(base: u64, index: usize) -> u64 {
    base ^ (0x5442_5700_u64 | (index as u64 + 1))
}

/// 实心线段图元。
fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
    instance(
        rect,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WidgetHarness;

    fn desc() -> TableDesc {
        TableDesc::new(
            Rect::new(0.0, 0.0, 320.0, 200.0),
            vec![
                ColumnSpec::new("Name", 160.0),
                ColumnSpec::new("Size", 80.0).right_aligned(),
                ColumnSpec::new("Date", 80.0),
            ],
        )
        .with_label("Files")
    }

    impl crate::WidgetDesc for TableDesc {
        fn rect(&self) -> Rect {
            self.rect
        }

        fn key(&self) -> u64 {
            7
        }

        fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput {
            emit_table(7, self, &TableState::new(), &[], glyphs, state)
        }
    }

    #[test]
    fn columns_are_laid_out_end_to_end() {
        let desc = desc();
        assert_eq!(desc.total_width(), 320.0);
        assert_eq!(desc.column_x(0), 0.0);
        assert_eq!(desc.column_x(1), 160.0);
        assert_eq!(desc.column_rect(2).unwrap().origin.x, 240.0);
    }

    #[test]
    fn the_window_follows_the_scroll_offset() {
        let desc = desc();
        let mut state = TableState::new();
        assert_eq!(state.window(&desc, 100), RowWindow { first: 0, last: 6 });
        state.scroll_by(&desc, 100, 10.0 * desc.row_height);
        let window = state.window(&desc, 100);
        assert_eq!(window.first, 10);
        assert_eq!(window.last, 16);
    }

    #[test]
    fn sorting_toggles_direction() {
        let mut state = TableState::new();
        assert_eq!(state.sort(), None);
        assert_eq!(state.sort_by(1), Some((1, SortDir::Ascending)));
        assert_eq!(state.sort_by(1), Some((1, SortDir::Descending)));
        assert_eq!(state.sort_by(2), Some((2, SortDir::Ascending)));
    }

    #[test]
    fn selection_is_exclusive_or_toggleable() {
        let mut state = TableState::new();
        state.select_row(3);
        assert!(state.is_selected(3));
        state.toggle_row(4);
        assert_eq!(state.selected().len(), 2);
        state.select_row(5);
        assert_eq!(state.selected().len(), 1);
        assert!(state.is_selected(5));
    }

    #[test]
    fn resizing_respects_the_minimum_width() {
        let desc = desc();
        let mut state = TableState::new();
        state.begin_resize(0, 160.0, 160.0);
        assert!(state.resize_to(&desc, 190.0));
        assert_eq!(state.column_width(&desc, 0), 190.0);
        state.end_resize();
        assert!(!state.is_resizing());
        state.begin_resize(0, 0.0, 190.0);
        state.resize_to(&desc, -500.0);
        assert_eq!(state.column_width(&desc, 0), 48.0);
    }

    #[test]
    fn the_frozen_edge_sits_after_the_last_frozen_column() {
        let desc = TableDesc::new(
            Rect::new(0.0, 0.0, 300.0, 100.0),
            vec![
                ColumnSpec::new("A", 100.0).frozen(),
                ColumnSpec::new("B", 100.0).frozen(),
                ColumnSpec::new("C", 100.0),
            ],
        );
        assert_eq!(desc.frozen_edge(), Some(200.0));
    }

    #[test]
    fn the_emitter_reports_a_grid_and_paints_rows() {
        let desc = desc();
        let state = TableState::new();
        let row = [TableRow {
            index: 0,
            glyphs: &[],
        }];
        let output = emit_table(7, &desc, &state, &row, &[], ControlState::default());
        let a11y = output.a11y.expect("grid");
        assert_eq!(a11y.role, Role::Grid);
        assert_eq!(a11y.description, "1 行 × 3 列");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Default))
        );
        let harness = WidgetHarness::new(desc.clone(), Vec::new());
        assert!(!harness.output.primitives.is_empty());
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let state = TableState::new();
        let row = [TableRow {
            index: 0,
            glyphs: &[],
        }];
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        let output = emit_table(7, &desc, &state, &row, &[], control);
                        counts.push(output.primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

//! Virtualised data table: descriptor, row window, selection and emitter.
//!
//! The table never owns the model. [`TableDesc`] describes the columns and the
//! fixed row rhythm; [`TableState`] owns the scroll offset, the sort key and
//! the selection set. The caller hands the *visible* rows to [`emit_table`]
//! together with their shaped glyphs, so a 100 000-row dataset still emits a
//! dozen row backgrounds per frame.
//!
//! Sorting is a *request*, not a mutation: [`TableState::sort_by`] flips the
//! direction and returns it, and the application reorders its model before the
//! next frame. That keeps the widget free of the row identity problem.

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

/// Horizontal alignment of a column's contents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignX {
    /// Leading edge.
    #[default]
    Start,
    /// Centred.
    Center,
    /// Trailing edge.
    End,
}

impl AlignX {
    /// The x offset of a `width`-wide cell inside a `span`-wide column.
    #[must_use]
    pub fn offset(self, span: f32, width: f32) -> f32 {
        match self {
            Self::Start => 0.0,
            Self::Center => ((span - width) * 0.5).max(0.0),
            Self::End => (span - width).max(0.0),
        }
    }
}

/// Sort direction of one column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

impl SortDir {
    /// The opposite direction.
    #[must_use]
    pub fn flip(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// One column's layout policy.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSpec {
    /// The header text.
    pub title: String,
    /// The preferred width.
    pub width: f32,
    /// The width a resize cannot go below.
    pub min_width: f32,
    /// The user may drag the trailing edge.
    pub resizable: bool,
    /// The header toggles the sort.
    pub sortable: bool,
    /// The column stays pinned to the leading edge.
    pub frozen: bool,
    /// Cell alignment.
    pub align: AlignX,
}

impl ColumnSpec {
    /// A resizable, sortable column of `width`.
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

    /// The same column, right-aligned (numbers).
    #[must_use]
    pub fn right_aligned(mut self) -> Self {
        self.align = AlignX::End;
        self
    }

    /// The same column, frozen at the leading edge.
    #[must_use]
    pub fn frozen(mut self) -> Self {
        self.frozen = true;
        self
    }

    /// The same column with its own minimum width.
    #[must_use]
    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width.max(1.0);
        self
    }
}

/// The pure descriptor of a table.
#[derive(Clone, Debug, PartialEq)]
pub struct TableDesc {
    /// The whole table rectangle, header included.
    pub rect: Rect,
    /// The accessible name.
    pub label: String,
    /// The columns, in visual order.
    pub columns: Vec<ColumnSpec>,
    /// The header height.
    pub header_height: f32,
    /// The uniform row height.
    pub row_height: f32,
    /// Alternate row backgrounds.
    pub striped: bool,
    /// The draw order key.
    pub z: u16,
}

impl TableDesc {
    /// A table with a 32 px header and 28 px rows.
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

    /// The same table with an accessible name.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The total width of every column.
    #[must_use]
    pub fn total_width(&self) -> f32 {
        self.columns.iter().map(|column| column.width).sum()
    }

    /// The header band.
    #[must_use]
    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            self.header_height,
        )
    }

    /// The body band under the header.
    #[must_use]
    pub fn body_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + self.header_height,
            self.rect.size.x,
            (self.rect.size.y - self.header_height).max(0.0),
        )
    }

    /// The visible height available to rows.
    #[must_use]
    pub fn body_height(&self) -> f32 {
        self.body_rect().size.y
    }

    /// How many rows fit in the body.
    #[must_use]
    pub fn visible_row_count(&self) -> usize {
        let rows = self.body_height() / self.row_height.max(1.0);
        rows.floor().max(0.0) as usize
    }

    /// The x offset of a column from the table's leading edge.
    #[must_use]
    pub fn column_x(&self, index: usize) -> f32 {
        self.columns
            .iter()
            .take(index)
            .map(|column| column.width)
            .sum()
    }

    /// One header cell.
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

    /// The divider the user drags to resize `index`.
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

    /// The x of the right edge of the last frozen column.
    #[must_use]
    pub fn frozen_edge(&self) -> Option<f32> {
        self.columns
            .iter()
            .position(|column| !column.frozen)
            .map(|first_scrolling| self.column_x(first_scrolling))
            .filter(|_| self.columns.first().is_some_and(|column| column.frozen))
    }

    /// The body rectangle of a row, accounting for `scroll` pixels.
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

/// Which row range is inside the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowWindow {
    /// First visible row (inclusive).
    pub first: usize,
    /// Last visible row (exclusive).
    pub last: usize,
}

impl RowWindow {
    /// Number of rows in the window.
    #[must_use]
    pub fn len(&self) -> usize {
        self.last.saturating_sub(self.first)
    }

    /// Whether the window is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate the row indices.
    pub fn iter(&self) -> Range<usize> {
        self.first..self.last
    }
}

/// The mutable state of a table.
#[derive(Clone, Debug, Default)]
pub struct TableState {
    scroll: ScrollState,
    sort: Option<(usize, SortDir)>,
    selected: FxHashSet<usize>,
    widths: Vec<f32>,
    resizing: Option<(usize, f32, f32)>,
}

impl TableState {
    /// An unsorted, unselected table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The scroll state.
    #[must_use]
    pub fn scroll(&self) -> &ScrollState {
        &self.scroll
    }

    /// The vertical scroll offset.
    #[must_use]
    pub fn offset(&self) -> f32 {
        self.scroll.offset().y
    }

    /// Scroll by `delta` pixels and clamp to the content.
    pub fn scroll_by(&mut self, desc: &TableDesc, row_count: usize, delta: f32) -> bool {
        let content = row_count as f32 * desc.row_height;
        let view = ScrollViewDesc::new(desc.body_rect(), unbound_math::Vec2::new(0.0, content));
        let before = self.scroll.target();
        self.scroll
            .scroll_by(&view, unbound_math::Vec2::new(0.0, delta));
        self.scroll.target() != before
    }

    /// Advance the scroll animation.
    pub fn step(&mut self, desc: &TableDesc, row_count: usize, dt: f32) -> bool {
        let content = row_count as f32 * desc.row_height;
        let view = ScrollViewDesc::new(desc.body_rect(), unbound_math::Vec2::new(0.0, content));
        self.scroll.step(&view, dt)
    }

    /// The rows inside the viewport.
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

    /// The current sort key.
    #[must_use]
    pub fn sort(&self) -> Option<(usize, SortDir)> {
        self.sort
    }

    /// Toggle the sort on `column` and return the new direction.
    pub fn sort_by(&mut self, column: usize) -> Option<(usize, SortDir)> {
        self.sort = match self.sort {
            Some((active, direction)) if active == column => Some((column, direction.flip())),
            _ => Some((column, SortDir::Ascending)),
        };
        self.sort
    }

    /// The selected rows.
    #[must_use]
    pub fn selected(&self) -> &FxHashSet<usize> {
        &self.selected
    }

    /// Whether `row` is selected.
    #[must_use]
    pub fn is_selected(&self, row: usize) -> bool {
        self.selected.contains(&row)
    }

    /// Select one row and clear the rest.
    pub fn select_row(&mut self, row: usize) {
        self.selected.clear();
        self.selected.insert(row);
    }

    /// Toggle `row` without touching the others.
    pub fn toggle_row(&mut self, row: usize) {
        if !self.selected.remove(&row) {
            self.selected.insert(row);
        }
    }

    /// Clear the selection.
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// The effective width of `index`, after any user resize.
    #[must_use]
    pub fn column_width(&self, desc: &TableDesc, index: usize) -> f32 {
        self.widths
            .get(index)
            .copied()
            .or_else(|| desc.columns.get(index).map(|column| column.width))
            .unwrap_or(0.0)
    }

    /// Begin dragging the trailing edge of `index`.
    pub fn begin_resize(&mut self, index: usize, x: f32, width: f32) {
        self.resizing = Some((index, x, width));
    }

    /// Update the drag; returns whether the width changed.
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

    /// Finish the drag.
    pub fn end_resize(&mut self) {
        self.resizing = None;
    }

    /// Whether a resize drag is live.
    #[must_use]
    pub fn is_resizing(&self) -> bool {
        self.resizing.is_some()
    }
}

/// The colour slots a table paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableTokens {
    /// The header surface.
    pub header: Token,
    /// The header text.
    pub header_text: Token,
    /// A body row.
    pub row: Token,
    /// An alternating body row.
    pub row_alt: Token,
    /// A selected row.
    pub selected: Token,
    /// The row and column rules.
    pub rule: Token,
    /// The frozen column divider.
    pub frozen_rule: Token,
    /// The body text.
    pub text: Token,
}

impl TableTokens {
    /// The standard set.
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

/// One visible row and its shaped glyphs.
#[derive(Clone, Copy, Debug)]
pub struct TableRow<'a> {
    /// The model row index.
    pub index: usize,
    /// The cell text, already positioned by the caller.
    pub glyphs: &'a [GlyphQuad],
}

/// Emit the header, the visible rows and the a11y grid.
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

    if control.focused {
        primitives.push(focus_ring(
            desc.rect,
            0.0,
            Token::FOCUS_RING,
            2.0,
            desc.z + 5,
            true,
        ));
    }

    let mut a11y = A11yNode::new(Role::Grid, desc.label.clone(), desc.rect).clickable();
    a11y.description = format!("{} 行 × {} 列", rows.len(), desc.columns.len());
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rect)
                .with_cursor(CursorKind::Default),
        ),
        a11y: Some(a11y),
        focusable: !control.disabled,
    }
}

/// A solid line primitive.
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
}

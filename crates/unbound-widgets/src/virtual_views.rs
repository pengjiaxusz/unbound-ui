//! Variable-height virtual list backed by a Fenwick tree.
//!
//! A uniform list answers "which rows are visible?" with one division. A list
//! whose rows grow with their content (log lines, wrapped comments, expanded
//! groups) cannot: the offset of row `i` is the sum of every height before it.
//! Recomputing that prefix per frame is O(n) and, at 100 000 rows, it shows up
//! as scroll jank.
//!
//! [`VariableVirtualList`] keeps the heights in a Fenwick tree. Prefix sums,
//! point updates and the "first row at or below this offset" search are all
//! O(log n), so a row that expands mid-scroll only touches a logarithmic
//! number of counters instead of re-summing the list.
//!
//! The tree is deliberately floats, not integers: the caller measures text in
//! logical pixels and the accumulated error over 100 000 rows is far below one
//! physical pixel.

use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs, surface};
use crate::scroll::ScrollViewDesc;
use crate::WidgetOutput;

/// A visible slice of a variable list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VariableWindow {
    /// First row to emit (inclusive).
    pub first: usize,
    /// Last row to emit (exclusive).
    pub last: usize,
    /// The scroll offset the window was computed for.
    pub offset: f32,
    /// The total content height.
    pub total: f32,
}

impl VariableWindow {
    /// Number of rows in the window.
    #[must_use]
    pub fn len(&self) -> usize {
        self.last.saturating_sub(self.first)
    }

    /// Whether the window has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A Fenwick tree of row heights.
#[derive(Clone, Debug, Default)]
pub struct VariableVirtualList {
    heights: Vec<f32>,
    tree: Vec<f32>,
    total: f32,
}

impl VariableVirtualList {
    /// A list of `rows` rows, each `height` tall.
    #[must_use]
    pub fn new(rows: usize, height: f32) -> Self {
        let height = height.max(1.0);
        let mut list = Self {
            heights: vec![height; rows],
            tree: vec![0.0; rows + 1],
            total: 0.0,
        };
        list.rebuild();
        list
    }

    /// A list with explicit heights.
    #[must_use]
    pub fn from_heights(heights: impl IntoIterator<Item = f32>) -> Self {
        let mut list = Self {
            heights: heights.into_iter().map(|height| height.max(1.0)).collect(),
            tree: Vec::new(),
            total: 0.0,
        };
        list.rebuild();
        list
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// The height of one row.
    #[must_use]
    pub fn height(&self, row: usize) -> f32 {
        self.heights.get(row).copied().unwrap_or(0.0)
    }

    /// The total content height.
    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.total
    }

    /// The y offset of a row's leading edge.
    #[must_use]
    pub fn offset_of(&self, row: usize) -> f32 {
        self.prefix(row.min(self.heights.len()))
    }

    /// Set one row's height, updating only O(log n) counters.
    pub fn set_height(&mut self, row: usize, height: f32) {
        if row >= self.heights.len() {
            return;
        }
        let height = height.max(1.0);
        let delta = height - self.heights[row];
        if delta.abs() <= f32::EPSILON {
            return;
        }
        self.heights[row] = height;
        self.total += delta;
        let mut index = row + 1;
        while index < self.tree.len() {
            self.tree[index] += delta;
            index += index & index.wrapping_neg();
        }
    }

    /// Grow or shrink the list, keeping the existing heights.
    pub fn resize(&mut self, rows: usize, height: f32) {
        let height = height.max(1.0);
        self.heights.resize(rows, height);
        self.rebuild();
    }

    /// The first row intersecting `offset`.
    #[must_use]
    pub fn first_visible(&self, offset: f32) -> usize {
        let offset = offset.max(0.0);
        if self.heights.is_empty() || offset >= self.total {
            return self.heights.len().saturating_sub(1);
        }
        // Binary lifting: the largest index whose prefix sum is <= offset.
        let mut index = 0usize;
        let mut remaining = offset;
        let mut step = self.highest_power_of_two();
        while step > 0 {
            let next = index + step;
            if next < self.tree.len() && self.tree[next] <= remaining {
                index = next;
                remaining -= self.tree[next];
            }
            step >>= 1;
        }
        index.min(self.heights.len().saturating_sub(1))
    }

    /// The window of rows intersecting `[offset, offset + viewport)`.
    #[must_use]
    pub fn window(&self, offset: f32, viewport: f32, overscan: usize) -> VariableWindow {
        if self.is_empty() || viewport <= 0.0 {
            return VariableWindow {
                first: 0,
                last: 0,
                offset,
                total: self.total,
            };
        }
        let first = self.first_visible(offset).saturating_sub(overscan);
        let bottom = offset.max(0.0) + viewport;
        let mut last = first;
        while last < self.heights.len() && self.offset_of(last) < bottom {
            last += 1;
        }
        let last = (last + overscan).min(self.heights.len());
        VariableWindow {
            first,
            last,
            offset,
            total: self.total,
        }
    }

    /// The rectangle of one row inside a viewport.
    #[must_use]
    pub fn row_rect(&self, row: usize, offset: f32, origin: Vec2, width: f32) -> Rect {
        Rect::new(
            origin.x,
            origin.y + self.offset_of(row) - offset,
            width,
            self.height(row),
        )
    }

    /// A scroll view descriptor for this list.
    #[must_use]
    pub fn scroll_view(&self, viewport: Rect) -> ScrollViewDesc {
        ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, self.total))
    }

    fn prefix(&self, count: usize) -> f32 {
        let mut sum = 0.0;
        let mut index = count;
        while index > 0 {
            sum += self.tree[index];
            index -= index & index.wrapping_neg();
        }
        sum
    }

    fn rebuild(&mut self) {
        self.tree = vec![0.0; self.heights.len() + 1];
        self.total = 0.0;
        for (index, height) in self.heights.iter().copied().enumerate() {
            self.total += height;
            let mut slot = index + 1;
            while slot < self.tree.len() {
                self.tree[slot] += height;
                slot += slot & slot.wrapping_neg();
            }
        }
    }

    fn highest_power_of_two(&self) -> usize {
        let mut step = 1usize;
        while step * 2 < self.tree.len() {
            step *= 2;
        }
        step
    }
}

/// The colour slots a virtual row paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualRowTokens {
    /// The row surface.
    pub row: Token,
    /// The hovered row surface.
    pub hover: Token,
    /// The selected row surface.
    pub selected: Token,
    /// The row text.
    pub text: Token,
    /// The row separator.
    pub rule: Token,
}

impl VirtualRowTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            row: Token::SURFACE,
            hover: Token::CONTROL_HOVER,
            selected: Token::SELECTION,
            text: Token::TEXT_PRIMARY,
            rule: Token::DIVIDER,
        }
    }
}

/// Emit one virtual row.
#[must_use]
pub fn emit_virtual_row(
    rect: Rect,
    row_glyphs: &[GlyphQuad],
    selected: bool,
    hovered: bool,
    tokens: VirtualRowTokens,
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + row_glyphs.len());
    let mut payloads = Vec::with_capacity(row_glyphs.len());
    let fill = if selected {
        tokens.selected
    } else if hovered {
        tokens.hover
    } else {
        tokens.row
    };
    primitives.push(surface(rect, 0.0, fill, z));
    primitives.push(line(
        Rect::new(rect.origin.x, rect.max().y - 1.0, rect.size.x, 1.0),
        tokens.rule,
        z + 1,
    ));
    glyphs(
        row_glyphs,
        tokens.text,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: None,
        focusable: false,
    }
}

fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
    crate::primitive::instance(
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

    #[test]
    fn prefix_sums_follow_the_heights() {
        let list = VariableVirtualList::from_heights([10.0, 20.0, 30.0, 40.0]);
        assert_eq!(list.total_height(), 100.0);
        assert_eq!(list.offset_of(0), 0.0);
        assert_eq!(list.offset_of(2), 30.0);
        assert_eq!(list.offset_of(4), 100.0);
    }

    #[test]
    fn updating_a_height_only_touches_the_suffix() {
        let mut list = VariableVirtualList::new(4, 10.0);
        list.set_height(1, 30.0);
        assert_eq!(list.total_height(), 60.0);
        assert_eq!(list.offset_of(2), 40.0);
        assert_eq!(list.offset_of(3), 50.0);
        list.set_height(1, 10.0);
        assert_eq!(list.total_height(), 40.0);
    }

    #[test]
    fn first_visible_finds_the_row_under_the_offset() {
        let list = VariableVirtualList::from_heights([10.0, 20.0, 30.0, 40.0]);
        assert_eq!(list.first_visible(0.0), 0);
        assert_eq!(list.first_visible(10.0), 1);
        assert_eq!(list.first_visible(29.9), 1);
        assert_eq!(list.first_visible(30.0), 2);
        assert_eq!(list.first_visible(1_000.0), 3);
    }

    #[test]
    fn the_window_spans_the_viewport_plus_overscan() {
        let list = VariableVirtualList::from_heights([10.0; 100]);
        let window = list.window(50.0, 100.0, 2);
        assert_eq!(window.first, 3, "overscan pulls two rows above");
        assert_eq!(window.last, 17);
        assert_eq!(window.len(), 14);
    }

    #[test]
    fn growing_the_list_keeps_the_existing_rows() {
        let mut list = VariableVirtualList::from_heights([10.0, 20.0]);
        list.resize(4, 5.0);
        assert_eq!(list.len(), 4);
        assert_eq!(list.height(1), 20.0);
        assert_eq!(list.height(3), 5.0);
        assert_eq!(list.total_height(), 40.0);
    }

    #[test]
    fn row_rects_offset_by_the_scroll_position() {
        let list = VariableVirtualList::from_heights([10.0, 20.0, 30.0]);
        let rect = list.row_rect(2, 15.0, Vec2::ZERO, 200.0);
        assert_eq!(rect.origin.y, 15.0);
        assert_eq!(rect.size.y, 30.0);
        assert_eq!(rect.size.x, 200.0);
    }

    #[test]
    fn an_empty_list_has_an_empty_window() {
        let list = VariableVirtualList::new(0, 10.0);
        assert!(list.is_empty());
        let window = list.window(0.0, 100.0, 0);
        assert!(window.is_empty());
    }

    #[test]
    fn the_row_emitter_paints_a_surface_and_glyphs() {
        let output = emit_virtual_row(
            Rect::new(0.0, 0.0, 100.0, 20.0),
            &[],
            true,
            false,
            VirtualRowTokens::standard(),
            100,
        );
        assert_eq!(output.primitives.len(), 2);
        assert!(output.a11y.is_none());
    }
}

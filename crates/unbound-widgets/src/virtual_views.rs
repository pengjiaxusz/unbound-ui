//! 基于树状数组（Fenwick Tree）的可变高度虚拟列表。
//!
//! 等高列表通过一次除法即可回答“哪些行可见？”，但行高随内容增长的列表
//! （如日志行、自动折行评论、展开分组）无法做到：第 `i` 行的偏移是其之前所有行高之和。
//! 每帧重算该前缀和是 O(n) 复杂度，在 10 万行规模下会导致明显滚动卡顿。
//!
//!
//!
//! [`VariableVirtualList`] 将行高保存在树状数组中。前缀和、单点更新以及
//! “首个位于或低于该偏移的行”查找均为 O(log n)，因此滚动中途某行展开时
//! 仅触及对数个计数器，而无需重新对整张列表求和。
//!
//!
//! 树状数组故意使用浮点数而非整数：调用方以逻辑像素测量文本，
//! 十万行累积的浮点误差也远低于一个物理像素。
//!

use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs, surface};
use crate::scroll::ScrollViewDesc;
use crate::WidgetOutput;

/// 可变高度列表的可见切片。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VariableWindow {
    /// 首个待输出行（包含）。
    pub first: usize,
    /// 末尾待输出行（不包含）。
    pub last: usize,
    /// 计算该窗口时所用的滚动偏移量。
    pub offset: f32,
    /// 内容总高度。
    pub total: f32,
}

impl VariableWindow {
    /// 窗口内的行数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.last.saturating_sub(self.first)
    }

    /// 窗口是否为空（无可见行）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 维护行高的树状数组。
#[derive(Clone, Debug, Default)]
pub struct VariableVirtualList {
    heights: Vec<f32>,
    tree: Vec<f32>,
    total: f32,
}

impl VariableVirtualList {
    /// 创建包含 `rows` 行、每行高度为 `height` 的列表。
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

    /// 使用显式行高列表创建。
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

    /// 行数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    /// 列表是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// 单行高度。
    #[must_use]
    pub fn height(&self, row: usize) -> f32 {
        self.heights.get(row).copied().unwrap_or(0.0)
    }

    /// 内容总高度。
    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.total
    }

    /// 指定行前导边缘的 y 偏移量。
    #[must_use]
    pub fn offset_of(&self, row: usize) -> f32 {
        self.prefix(row.min(self.heights.len()))
    }

    /// 设置单行高度，仅更新 O(log n) 个计数器。
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

    /// 扩充或缩减列表长度，保留已有行高。
    pub fn resize(&mut self, rows: usize, height: f32) {
        let height = height.max(1.0);
        self.heights.resize(rows, height);
        self.rebuild();
    }

    /// 与 `offset` 相交的首个行索引。
    #[must_use]
    pub fn first_visible(&self, offset: f32) -> usize {
        let offset = offset.max(0.0);
        if self.heights.is_empty() || offset >= self.total {
            return self.heights.len().saturating_sub(1);
        }
        // 倍增查找：前缀和 <= offset 的最大索引。
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

    /// 与 `[offset, offset + viewport)` 相交的行窗口。
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

    /// 单行在视口内的矩形。
    #[must_use]
    pub fn row_rect(&self, row: usize, offset: f32, origin: Vec2, width: f32) -> Rect {
        Rect::new(
            origin.x,
            origin.y + self.offset_of(row) - offset,
            width,
            self.height(row),
        )
    }

    /// 该列表对应的滚动视图描述符。
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

/// 虚拟行绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualRowTokens {
    /// 行表面背景。
    pub row: Token,
    /// 悬停行表面背景。
    pub hover: Token,
    /// 选中行表面背景。
    pub selected: Token,
    /// 行文本。
    pub text: Token,
    /// 行分隔线。
    pub rule: Token,
}

impl VirtualRowTokens {
    /// 标准配色集。
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

/// 输出单个虚拟行。
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
        ..Default::default()
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

//! 二维等格虚拟化网格容器（`VirtualGrid`）。
//!
//! 补齐 `virtual_views` 体系中的固定行高二维等格虚拟化能力：根据容器宽度、
//! 固定/最小列宽、行高、行列间距与滚动偏移，以 O(1) 复杂度计算可见行区间与单元格矩形，
//! 仅发射视口 + `overscan` 范围内的可见单元格，支撑万级缩略图墙的 O(1) 帧开销，
//! 并在容器宽度变化触发列数重排时保持锚点/焦点项的相对滚动位置稳定。

use std::ops::Range;

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuClipNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, SceneCollector, WidgetOutput};

/// 虚拟网格单个可见单元格的确定性子节点 ID。
#[must_use]
pub const fn virtual_grid_cell_node(node: u64, item_index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA500 ^ (item_index as u64)
}

/// 容器圆角半径（对齐 WinUI `ControlCornerRadius = 4`）。
const GRID_RADIUS: f32 = 4.0;

/// 列宽计算策略。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum GridColumnSizing {
    /// 固定列宽，根据容器可用宽度向下取整计算列数。
    Fixed(f32),
    /// 最小列宽，自动填满容器宽度并在不小于最小列宽的前提下尽可能多排布列。
    MinWidth(f32),
}

impl Default for GridColumnSizing {
    fn default() -> Self {
        Self::MinWidth(140.0)
    }
}

/// 单帧可见窗口计算结果（O(1) 计算）。
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualGridWindow {
    /// 当前生效列数（至少为 1）。
    pub columns: usize,
    /// 总行数。
    pub total_rows: usize,
    /// 实际单列宽度（像素）。
    pub cell_width: f32,
    /// 可见行区间（含 `overscan`，左闭右开）。
    pub row_range: Range<usize>,
    /// 可见条目索引区间（左闭右开，已按 `item_count` 截断）。
    pub item_range: Range<usize>,
    /// 整个虚拟网格的内容总高度（像素）。
    pub total_height: f32,
}

impl VirtualGridWindow {
    /// 当前窗口内实际需要发射的可见单元格数量。
    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.item_range.len()
    }
}

/// 二维等格虚拟化网格描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualGridDesc {
    /// 视口容器矩形。
    pub rect: Rect,
    /// 总条目数（支持万级/十万级规模）。
    pub item_count: usize,
    /// 列宽策略（固定列宽或自适应最小列宽）。
    pub column_sizing: GridColumnSizing,
    /// 单个单元格固定高度（像素）。
    pub row_height: f32,
    /// 列间距（像素）。
    pub column_gap: f32,
    /// 行间距（像素）。
    pub row_gap: f32,
    /// 容器内边距（像素）。
    pub padding: f32,
    /// 视口上下额外预渲染的缓冲行数（默认 `1`）。
    pub overscan_rows: usize,
    /// 无障碍标签。
    pub label: String,
    /// 绘制 z-index。
    pub z: u16,
}

impl VirtualGridDesc {
    /// 创建默认二维等格虚拟化网格描述符。
    #[must_use]
    pub fn new(rect: Rect, item_count: usize) -> Self {
        Self {
            rect,
            item_count,
            column_sizing: GridColumnSizing::default(),
            row_height: 120.0,
            column_gap: 8.0,
            row_gap: 8.0,
            padding: 8.0,
            overscan_rows: 1,
            label: "Virtual Grid".to_owned(),
            z: 20,
        }
    }

    /// 设置列宽计算策略。
    #[must_use]
    pub const fn with_column_sizing(mut self, sizing: GridColumnSizing) -> Self {
        self.column_sizing = sizing;
        self
    }

    /// 设置行高。
    #[must_use]
    pub fn with_row_height(mut self, row_height: f32) -> Self {
        self.row_height = row_height.max(16.0);
        self
    }

    /// 设置行列间距。
    #[must_use]
    pub fn with_gaps(mut self, column_gap: f32, row_gap: f32) -> Self {
        self.column_gap = column_gap.max(0.0);
        self.row_gap = row_gap.max(0.0);
        self
    }

    /// 设置预渲染缓冲行数。
    #[must_use]
    pub const fn with_overscan_rows(mut self, overscan_rows: usize) -> Self {
        self.overscan_rows = overscan_rows;
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 根据当前容器宽度计算列数与实际单元格宽度（O(1)）。
    #[must_use]
    pub fn layout_columns(&self) -> (usize, f32) {
        let inner_w = (self.rect.width() - self.padding * 2.0).max(16.0);
        match self.column_sizing {
            GridColumnSizing::Fixed(col_w) => {
                let w = col_w.max(16.0);
                let stride = w + self.column_gap;
                let cols = (((inner_w + self.column_gap) / stride).floor() as usize).max(1);
                (cols, w.min(inner_w))
            }
            GridColumnSizing::MinWidth(min_w) => {
                let mw = min_w.max(16.0);
                let stride = mw + self.column_gap;
                let cols = (((inner_w + self.column_gap) / stride).floor() as usize).max(1);
                let actual_w = ((inner_w - self.column_gap * (cols.saturating_sub(1) as f32))
                    / cols as f32)
                    .max(16.0);
                (cols, actual_w)
            }
        }
    }

    /// 单行步距（行高 + 行间距）。
    #[must_use]
    pub fn row_stride(&self) -> f32 {
        (self.row_height + self.row_gap).max(1.0)
    }

    /// 总行数。
    #[must_use]
    pub fn total_rows(&self) -> usize {
        if self.item_count == 0 {
            return 0;
        }
        let (cols, _) = self.layout_columns();
        self.item_count.div_ceil(cols)
    }

    /// 虚拟内容总高度（含上下内边距）。
    #[must_use]
    pub fn total_height(&self) -> f32 {
        let rows = self.total_rows();
        if rows == 0 {
            return self.padding * 2.0;
        }
        self.padding * 2.0
            + rows as f32 * self.row_height
            + rows.saturating_sub(1) as f32 * self.row_gap
    }

    /// 最大合法垂直滚动偏移。
    #[must_use]
    pub fn max_scroll_y(&self) -> f32 {
        (self.total_height() - self.rect.height()).max(0.0)
    }

    /// 以 O(1) 复杂度计算当前垂直滚动位置对应的可见窗口。
    #[must_use]
    pub fn window(&self, scroll_y: f32) -> VirtualGridWindow {
        let (columns, cell_width) = self.layout_columns();
        let total_rows = if self.item_count == 0 {
            0
        } else {
            self.item_count.div_ceil(columns)
        };
        let total_height = self.total_height();
        if total_rows == 0 {
            return VirtualGridWindow {
                columns,
                total_rows: 0,
                cell_width,
                row_range: 0..0,
                item_range: 0..0,
                total_height,
            };
        }

        let clamped_scroll = scroll_y.clamp(0.0, self.max_scroll_y());
        let stride = self.row_stride();
        let first_visible = ((clamped_scroll - self.padding).max(0.0) / stride).floor() as usize;
        let last_visible =
            (((clamped_scroll + self.rect.height() - self.padding).max(0.0) / stride).ceil()
                as usize)
                .min(total_rows);

        let start_row = first_visible
            .saturating_sub(self.overscan_rows)
            .min(total_rows);
        let end_row = last_visible
            .saturating_add(self.overscan_rows)
            .min(total_rows);
        let start_item = (start_row * columns).min(self.item_count);
        let end_item = (end_row * columns).min(self.item_count);

        VirtualGridWindow {
            columns,
            total_rows,
            cell_width,
            row_range: start_row..end_row,
            item_range: start_item..end_item,
            total_height,
        }
    }

    /// 计算第 `index` 个单元格在视口坐标系中的绘制矩形（已扣除 `scroll_y`）。
    #[must_use]
    pub fn cell_rect(&self, index: usize, scroll_y: f32) -> Option<Rect> {
        if index >= self.item_count {
            return None;
        }
        let (columns, cell_width) = self.layout_columns();
        let row = index / columns;
        let col = index % columns;
        let x = self.rect.origin.x + self.padding + col as f32 * (cell_width + self.column_gap);
        let y = self.rect.origin.y + self.padding + row as f32 * self.row_stride()
            - scroll_y.clamp(0.0, self.max_scroll_y());
        Some(Rect::new(x, y, cell_width, self.row_height))
    }

    /// 生成对齐容器视口矩形的 GPU 裁剪节点（供 [`SceneCollector::push_clip`] 使用）。
    #[must_use]
    pub fn clip_node(&self) -> GpuClipNode {
        GpuClipNode::from_aabb(self.rect.to_min_max_array())
    }
}

/// 二维虚拟网格交互与滚动状态机。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VirtualGridState {
    /// 当前垂直滚动偏移（像素）。
    pub scroll_y: f32,
    /// 当前键盘焦点所在条目索引。
    pub focused_index: Option<usize>,
    /// 当前选中条目索引。
    pub selected_index: Option<usize>,
    /// 最近一次布局计算得到的列数（用于检测容器缩放导致的列数重排）。
    last_columns: usize,
}

impl VirtualGridState {
    /// 创建默认虚拟网格状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 滚动指定增量（正值向下，负值向上），自动钳制至合法区间。
    pub fn scroll_by(&mut self, delta_y: f32, desc: &VirtualGridDesc) {
        self.scroll_y = (self.scroll_y + delta_y).clamp(0.0, desc.max_scroll_y());
    }

    /// 直接设置滚动偏移（自动钳制）。
    pub fn set_scroll_y(&mut self, scroll_y: f32, desc: &VirtualGridDesc) {
        self.scroll_y = scroll_y.clamp(0.0, desc.max_scroll_y());
    }

    /// 选中并聚焦指定条目，并确保其滚动入镜。
    pub fn select(&mut self, index: usize, desc: &VirtualGridDesc) -> bool {
        if index >= desc.item_count {
            return false;
        }
        self.selected_index = Some(index);
        self.focused_index = Some(index);
        self.ensure_visible(index, desc);
        true
    }

    /// 当容器宽度变化导致列数重排时，保持焦点/选中项或视口顶部锚点项的相对滚动位置稳定，
    /// 避免跳屏与定位漂移。
    pub fn sync_layout(&mut self, desc: &VirtualGridDesc) {
        let (new_cols, _) = desc.layout_columns();
        let stride = desc.row_stride();

        if self.last_columns > 0 && self.last_columns != new_cols && desc.item_count > 0 {
            let old_cols = self.last_columns;
            let anchor_index = self
                .focused_index
                .or(self.selected_index)
                .unwrap_or_else(|| {
                    let top_row =
                        ((self.scroll_y - desc.padding).max(0.0) / stride).floor() as usize;
                    (top_row * old_cols).min(desc.item_count - 1)
                })
                .min(desc.item_count - 1);

            let old_row = anchor_index / old_cols;
            let old_top = desc.padding + old_row as f32 * stride;
            let relative_offset = old_top - self.scroll_y;

            let new_row = anchor_index / new_cols;
            let new_top = desc.padding + new_row as f32 * stride;
            self.scroll_y = (new_top - relative_offset).clamp(0.0, desc.max_scroll_y());
        } else {
            self.scroll_y = self.scroll_y.clamp(0.0, desc.max_scroll_y());
        }

        if desc.item_count == 0 {
            self.focused_index = None;
            self.selected_index = None;
        } else {
            if let Some(f) = self.focused_index {
                if f >= desc.item_count {
                    self.focused_index = Some(desc.item_count - 1);
                }
            }
            if let Some(s) = self.selected_index {
                if s >= desc.item_count {
                    self.selected_index = Some(desc.item_count - 1);
                }
            }
        }
        self.last_columns = new_cols;
    }

    /// 将指定条目滚动到视口可见区域内。
    pub fn ensure_visible(&mut self, index: usize, desc: &VirtualGridDesc) {
        if index >= desc.item_count {
            return;
        }
        let (cols, _) = desc.layout_columns();
        let row = index / cols;
        let stride = desc.row_stride();
        let item_top = desc.padding + row as f32 * stride;
        let item_bottom = item_top + desc.row_height;
        let view_top = self.scroll_y;
        let view_bottom = self.scroll_y + desc.rect.height();

        if item_top < view_top {
            self.scroll_y = item_top.clamp(0.0, desc.max_scroll_y());
        } else if item_bottom > view_bottom {
            self.scroll_y = (item_bottom - desc.rect.height()).clamp(0.0, desc.max_scroll_y());
        }
    }

    /// 响应二维网格键盘导航（`ArrowLeft/Right/Up/Down`、`Home/End`、`PageUp/PageDown`）并自动滚动入镜。
    pub fn handle_named_key(&mut self, key: &str, desc: &VirtualGridDesc) -> bool {
        if desc.item_count == 0 {
            return false;
        }
        let (cols, _) = desc.layout_columns();
        let last = desc.item_count - 1;
        let current = self
            .focused_index
            .or(self.selected_index)
            .unwrap_or(0)
            .min(last);
        let page_rows = ((desc.rect.height() / desc.row_stride()).floor() as usize).max(1);
        let page_items = page_rows.saturating_mul(cols);

        let next = match key {
            "ArrowLeft" => current.saturating_sub(1),
            "ArrowRight" => (current + 1).min(last),
            "ArrowUp" => current.saturating_sub(cols),
            "ArrowDown" => (current + cols).min(last),
            "Home" => 0,
            "End" => last,
            "PageUp" => current.saturating_sub(page_items),
            "PageDown" => (current + page_items).min(last),
            "Enter" | "Space" => {
                self.selected_index = Some(current);
                self.focused_index = Some(current);
                return true;
            }
            _ => return false,
        };

        self.focused_index = Some(next);
        self.selected_index = Some(next);
        self.ensure_visible(next, desc);
        true
    }
}

/// 发射二维虚拟化网格容器（仅发射视口可见单元格，24 态下同视口图元数恒定）。
#[must_use]
pub fn emit_virtual_grid(
    node: u64,
    desc: &VirtualGridDesc,
    state: &VirtualGridState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let window = desc.window(state.scroll_y);
    let visible = window.visible_count();
    let r = desc.rect;
    let z = desc.z;

    let mut primitives = Vec::with_capacity(2 + visible + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let outer_border = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        r,
        GRID_RADIUS,
        Token::SURFACE,
        outer_border,
        1.0,
        0.0,
        z,
    ));

    // 仅发射与容器视口相交的 O(visible) 个单元格：
    // 先跳过完全落在容器 `desc.rect` 之外的 overscan 单元格，再将半截露出的边缘单元格裁剪至容器内，
    // 防止越界图元覆盖下方或上方相邻控件。
    let mut visible_cells: Vec<(usize, Rect, bool)> = Vec::with_capacity(visible);
    for idx in window.item_range.clone() {
        if let Some(cell_r) = desc.cell_rect(idx, state.scroll_y) {
            if !cell_r.intersects(&r) {
                continue;
            }
            let clipped_r = cell_r.intersect(&r);
            if clipped_r.is_empty() {
                continue;
            }
            let is_selected = state.selected_index == Some(idx);
            let is_focused = state.focused_index == Some(idx);
            let (fill, border) = if control.disabled {
                (Token::CONTROL_DISABLED_BG, Token::CONTROL_DISABLED_BORDER)
            } else if is_selected {
                (Token::PRIMARY_SOFT, Token::PRIMARY)
            } else if is_focused {
                (Token::HOVER, Token::FOCUS_RING)
            } else {
                (Token::CARD_BG, Token::CARD_BORDER)
            };
            primitives.push(outlined(
                clipped_r,
                GRID_RADIUS,
                fill,
                border,
                1.0,
                0.0,
                z + 1,
            ));
            visible_cells.push((idx, clipped_r, is_selected));
        }
    }

    if !glyphs.is_empty() {
        let text_tok = if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };
        // 跳过完全落在容器矩形之外的字形，避免 overscan 文本越界发射。
        let visible_glyphs: Vec<GlyphQuad> = glyphs
            .iter()
            .copied()
            .filter(|q| q.rect.intersects(&r))
            .collect();
        if !visible_glyphs.is_empty() {
            emit_glyphs(
                &visible_glyphs,
                text_tok,
                z + 2,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    // 与 `tree_table` 保持一致使用贴边焦点环，避免在 `emit_virtual_grid_scoped` 的容器裁剪作用域内被裁平。
    primitives.push(focus_ring(
        r,
        GRID_RADIUS,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), r)
            .with_shape(Shape::Rounded {
                radii: [GRID_RADIUS; 4],
            })
            .with_cursor(CursorKind::Default)
    });

    let a11y = A11yNode::new(Role::Grid, desc.label.clone(), r)
        .with_value(format!("{} items", desc.item_count))
        .disabled(control.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    };

    // 仅为可见单元格注册子控件命中区域与无障碍 Cell 节点，严格维持 O(visible) 零节点泄漏。
    if !control.disabled {
        for (idx, cell_r, is_selected) in visible_cells {
            let cell_id = virtual_grid_cell_node(node, idx);
            out.with_sub_control(
                cell_id,
                HitRegion::new(cell_id, CONTROL_LAYER, u32::from(z) + 1, cell_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Cell, format!("Grid Item {idx}"), cell_r)
                    .selected(is_selected)
                    .clickable(),
                true,
            );
        }
    }

    out
}

/// 在容器 `desc.rect` 的 `push_clip` / `pop_clip` 裁剪作用域内向 [`SceneCollector`] 发射二维虚拟网格。
///
/// 通过对网格绘制区域施加容器 AABB 裁剪，防止底部或顶部半截露出的单元格与文字越界覆盖下方控件。
pub fn emit_virtual_grid_scoped(
    collector: &mut SceneCollector,
    node: u64,
    desc: &VirtualGridDesc,
    state: &VirtualGridState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) {
    let clip = desc.clip_node();
    collector.push_clip(clip, desc.rect);
    collector.push(node, emit_virtual_grid(node, desc, state, control, glyphs));
    collector.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn virtual_grid_ten_thousand_items_constant_window_and_zero_leak() {
        let rect = Rect::new(0.0, 0.0, 800.0, 600.0);
        let small = VirtualGridDesc::new(rect, 200)
            .with_column_sizing(GridColumnSizing::MinWidth(150.0))
            .with_row_height(120.0);
        let huge = VirtualGridDesc::new(rect, 100_000)
            .with_column_sizing(GridColumnSizing::MinWidth(150.0))
            .with_row_height(120.0);

        let mut st_small = VirtualGridState::new();
        let mut st_huge = VirtualGridState::new();
        st_small.sync_layout(&small);
        st_huge.sync_layout(&huge);

        let out_small = emit_virtual_grid(1, &small, &st_small, ControlState::default(), &[]);
        let out_huge = emit_virtual_grid(2, &huge, &st_huge, ControlState::default(), &[]);

        // 万级条目发射的图元数与 200 条目完全相同（严格 O(1) 视口裁剪，零节点泄漏）。
        assert_eq!(out_small.primitives.len(), out_huge.primitives.len());
        assert!(out_huge.primitives.len() < 64);

        // 滚动到中部与末尾后图元数依然保持 O(1) 上界。
        st_huge.set_scroll_y(huge.max_scroll_y() * 0.5, &huge);
        let out_mid = emit_virtual_grid(2, &huge, &st_huge, ControlState::default(), &[]);
        assert!(out_mid.primitives.len() < 64);
    }

    #[test]
    fn virtual_grid_dynamic_column_reflow_preserves_anchor() {
        let wide = VirtualGridDesc::new(Rect::new(0.0, 0.0, 960.0, 480.0), 10_000)
            .with_column_sizing(GridColumnSizing::Fixed(180.0))
            .with_row_height(100.0);
        let narrow = VirtualGridDesc::new(Rect::new(0.0, 0.0, 420.0, 480.0), 10_000)
            .with_column_sizing(GridColumnSizing::Fixed(180.0))
            .with_row_height(100.0);

        let mut st = VirtualGridState::new();
        st.sync_layout(&wide);
        st.select(120, &wide);
        let before_rect = wide.cell_rect(120, st.scroll_y).unwrap();

        // 容器缩窄触发列数重排，锚点项相对视口 Y 坐标应保持一致。
        st.sync_layout(&narrow);
        let after_rect = narrow.cell_rect(120, st.scroll_y).unwrap();
        assert!(
            (before_rect.origin.y - after_rect.origin.y).abs() < 1.0,
            "anchor cell must stay visually stable across column reflow"
        );
    }

    #[test]
    fn virtual_grid_2d_keyboard_navigation_and_headless_geometry() {
        let rect = Rect::new(20.0, 20.0, 640.0, 400.0);
        let desc = VirtualGridDesc::new(rect, 500)
            .with_column_sizing(GridColumnSizing::Fixed(140.0))
            .with_row_height(100.0);
        let (cols, _) = desc.layout_columns();
        assert!(cols >= 3);

        let mut st = VirtualGridState::new();
        st.sync_layout(&desc);
        assert!(st.handle_named_key("ArrowRight", &desc));
        assert_eq!(st.focused_index, Some(1));
        assert!(st.handle_named_key("ArrowDown", &desc));
        assert_eq!(st.focused_index, Some(1 + cols));
        assert!(st.handle_named_key("End", &desc));
        assert_eq!(st.focused_index, Some(499));
        assert!(st.scroll_y > 0.0);
        assert!(st.handle_named_key("Home", &desc));
        assert_eq!(st.focused_index, Some(0));
        assert!(st.scroll_y <= desc.padding + 1.0);

        // 24 态图元数恒定。
        let base = emit_virtual_grid(1, &desc, &st, ControlState::default(), &[])
            .primitives
            .len();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let c = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        assert_eq!(
                            emit_virtual_grid(1, &desc, &st, c, &[]).primitives.len(),
                            base
                        );
                    }
                }
            }
        }

        // 无头几何健康检查。
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "VirtualGrid",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }

    #[test]
    fn virtual_grid_skips_out_of_bounds_overscan_cells_and_clips_to_container() {
        let rect = Rect::new(10.0, 20.0, 320.0, 150.0);
        let desc = VirtualGridDesc::new(rect, 40)
            .with_column_sizing(GridColumnSizing::Fixed(140.0))
            .with_row_height(100.0)
            .with_gaps(8.0, 8.0)
            .with_overscan_rows(2);
        let mut st = VirtualGridState::new();
        st.sync_layout(&desc);

        // 窗口含 2 行 overscan，共 4 行 8 个单元格，但仅有前 2 行（4 个单元格）与容器相交。
        let window = desc.window(st.scroll_y);
        assert_eq!(window.visible_count(), 8);

        let inside_glyph = GlyphQuad {
            rect: Rect::new(24.0, 36.0, 10.0, 12.0),
            uv: [0.0; 4],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        };
        let outside_glyph = GlyphQuad {
            rect: Rect::new(24.0, 260.0, 10.0, 12.0),
            uv: [0.0; 4],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        };

        let out = emit_virtual_grid(
            1,
            &desc,
            &st,
            ControlState::default(),
            &[inside_glyph, outside_glyph],
        );
        // 完全落在容器外的 overscan 单元格与字形均不会被发射。
        assert_eq!(out.sub_hits.len(), 4);
        assert_eq!(out.payloads.len(), 1);
        for hit in &out.sub_hits {
            assert!(
                rect.contains_rect(&hit.bounds),
                "cell hit bounds {:?} must stay within container {:?}",
                hit.bounds,
                rect
            );
        }
        // 容器外框（index 0）之后、字形与焦点环之前的 4 个单元格图元均应裁剪至容器高度内。
        for prim in &out.primitives[1..5] {
            assert!(prim.local_pos[1] >= rect.origin.y);
            assert!(prim.local_pos[1] + prim.size[1] <= rect.max().y);
        }

        // 通过 `emit_virtual_grid_scoped` 推入时应施加并正确弹出容器裁剪作用域。
        let mut collector = SceneCollector::default();
        emit_virtual_grid_scoped(
            &mut collector,
            1,
            &desc,
            &st,
            ControlState::default(),
            &[inside_glyph, outside_glyph],
        );
        assert_eq!(collector.clips.len(), 1);
        assert_eq!(collector.clips[0].aabb_clip, rect.to_min_max_array());
        assert!(collector.primitives.iter().all(|prim| prim.clip_idx == 0));

        // 确认裁剪作用域已在返回前弹出，后续控件不再受该裁剪节点约束。
        let trailing_count = collector.primitives.len();
        collector.push(
            2,
            WidgetOutput {
                primitives: vec![outlined(
                    Rect::new(0.0, 200.0, 100.0, 32.0),
                    4.0,
                    Token::SURFACE,
                    Token::CARD_BORDER,
                    1.0,
                    0.0,
                    20,
                )],
                ..Default::default()
            },
        );
        assert_eq!(
            collector.primitives[trailing_count].clip_idx,
            unbound_gpu::data::CLIP_NONE
        );
    }
}

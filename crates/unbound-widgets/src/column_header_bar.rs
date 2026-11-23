//! 通用可拖拽调宽与列尾过滤表头控件（`ColumnHeaderBar`）。
//!
//! 原生支持：
//! - 列标题左/中/右对齐；
//! - 1px 列间竖分隔线与底部分隔线；
//! - 独立矢量排序箭头（零字体依赖，基于 [`ShellIcon::ChevronUp`] / [`ShellIcon::ChevronDown`]）；
//! - 列右边界 8px 调宽热区（`ResizeEastWest` 光标、`PointerDown/Drag/Up` 实时调宽、双击自适应事件）；
//! - 每列末尾悬停/点击显示 `⌄` 下拉过滤子热区并派发带锚点矩形的列过滤请求事件；
//! - 与下方数据列表共享横向滚动偏移 `scroll_x`。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, PointerButton, PointerEvent};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs as emit_glyphs, instance, surface};
use crate::shell::{emit_shell_icon_primitives, ShellIcon};
use crate::table::SortDir;
use crate::{ControlState, WidgetOutput};

const LAYER: u8 = 1;

/// 表头控件默认高度（`32px`）。
pub const COLUMN_HEADER_BAR_HEIGHT: f32 = 32.0;

/// 列右边界拖拽调宽热区宽度（`8px`，以列右边界为中心左右各延展 `4px`）。
pub const COLUMN_RESIZE_HANDLE_WIDTH: f32 = 8.0;

/// 列尾 `⌄` 下拉过滤按钮热区宽度（`20px`）。
pub const COLUMN_FILTER_BUTTON_WIDTH: f32 = 20.0;

/// 默认最小列宽（`48px`）。
pub const COLUMN_DEFAULT_MIN_WIDTH: f32 = 48.0;

/// 默认最大列宽（`1200px`）。
pub const COLUMN_DEFAULT_MAX_WIDTH: f32 = 1200.0;

/// 列间竖分隔线宽度（`1px`）。
pub const COLUMN_DIVIDER_WIDTH: f32 = 1.0;

/// 表头控件默认绘制深度键。
const DEFAULT_COLUMN_HEADER_Z: u16 = 460;

/// 标题左右水平内边距（`8px`）。
const CELL_HORIZONTAL_PADDING: f32 = 8.0;

/// 独立排序矢量箭头占位边长（`12px`）。
const SORT_ARROW_SIZE: f32 = 12.0;

/// 表头列标题的水平对齐方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColumnHeaderAlign {
    /// 左对齐（默认）。
    #[default]
    Left,
    /// 居中对齐。
    Center,
    /// 右对齐（常用于数值、大小等列）。
    Right,
}

/// 表头单列描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnHeaderSpec {
    /// 列标题文本。
    pub title: String,
    /// 当前列宽（逻辑像素）。
    pub width: f32,
    /// 拖拽调宽时的最小允许宽度（逻辑像素）。
    pub min_width: f32,
    /// 拖拽调宽时的最大允许宽度（逻辑像素）。
    pub max_width: f32,
    /// 列标题水平对齐方式。
    pub align: ColumnHeaderAlign,
    /// 是否允许点击列头主体触发排序。
    pub sortable: bool,
    /// 是否允许拖拽右边界调宽及双击自适应。
    pub resizable: bool,
    /// 是否启用列尾 `⌄` 下拉过滤子热区。
    pub filterable: bool,
    /// 当前列是否已激活过滤条件（激活时列尾 `⌄` 常驻高亮显示）。
    pub filter_active: bool,
}

impl ColumnHeaderSpec {
    /// 创建默认左对齐、可排序、可调宽且支持列尾过滤的单列描述符。
    #[must_use]
    pub fn new(title: impl Into<String>, width: f32) -> Self {
        Self {
            title: title.into(),
            width: width.max(1.0),
            min_width: COLUMN_DEFAULT_MIN_WIDTH,
            max_width: COLUMN_DEFAULT_MAX_WIDTH,
            align: ColumnHeaderAlign::Left,
            sortable: true,
            resizable: true,
            filterable: true,
            filter_active: false,
        }
    }

    /// 设置列标题水平对齐方式。
    #[must_use]
    pub fn with_align(mut self, align: ColumnHeaderAlign) -> Self {
        self.align = align;
        self
    }

    /// 快捷设置为右对齐。
    #[must_use]
    pub fn right_aligned(mut self) -> Self {
        self.align = ColumnHeaderAlign::Right;
        self
    }

    /// 设置最小允许列宽。
    #[must_use]
    pub fn with_min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width.max(1.0);
        if self.max_width < self.min_width {
            self.max_width = self.min_width;
        }
        self
    }

    /// 设置最大允许列宽。
    #[must_use]
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width.max(self.min_width);
        self
    }

    /// 控制是否允许点击排序。
    #[must_use]
    pub fn with_sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }

    /// 控制是否允许边界拖拽调宽与双击自适应。
    #[must_use]
    pub fn with_resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// 控制是否启用列尾 `⌄` 下拉过滤子热区。
    #[must_use]
    pub fn with_filterable(mut self, filterable: bool) -> Self {
        self.filterable = filterable;
        self
    }

    /// 设置当前列是否处于过滤激活态。
    #[must_use]
    pub fn with_filter_active(mut self, active: bool) -> Self {
        self.filter_active = active;
        self
    }
}

/// 通用表头控件纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnHeaderBarDesc {
    /// 表头条外框矩形。
    pub rect: Rect,
    /// 按视觉顺序排列的列定义列表。
    pub columns: Vec<ColumnHeaderSpec>,
    /// 当前排序列索引与方向（为 `Some` 时绘制独立排序矢量箭头）。
    pub sort: Option<(usize, SortDir)>,
    /// 与下方列表共享的横向滚动偏移（逻辑像素，`>= 0.0`）。
    pub scroll_x: f32,
    /// 左侧预留缩进（例如列表首列包含复选框时的前置占位，逻辑像素）。
    pub leading_inset: f32,
    /// 列标题字号（默认 `12.0`）。
    pub font_size: f32,
    /// 是否绘制 1px 列间竖分隔线（默认 `true`）。
    pub show_dividers: bool,
    /// 绘制顺序深度键。
    pub z: u16,
}

impl ColumnHeaderBarDesc {
    /// 创建默认配置的表头描述符。
    #[must_use]
    pub fn new(rect: Rect, columns: Vec<ColumnHeaderSpec>) -> Self {
        Self {
            rect,
            columns,
            sort: None,
            scroll_x: 0.0,
            leading_inset: 0.0,
            font_size: 12.0,
            show_dividers: true,
            z: DEFAULT_COLUMN_HEADER_Z,
        }
    }

    /// 设置当前排序列与排序方向。
    #[must_use]
    pub fn with_sort(mut self, sort: Option<(usize, SortDir)>) -> Self {
        self.sort = sort;
        self
    }

    /// 设置与下方列表共享的横向滚动偏移。
    #[must_use]
    pub fn with_scroll_x(mut self, scroll_x: f32) -> Self {
        self.scroll_x = scroll_x.max(0.0);
        self
    }

    /// 设置左侧预留缩进宽度。
    #[must_use]
    pub fn with_leading_inset(mut self, leading_inset: f32) -> Self {
        self.leading_inset = leading_inset.max(0.0);
        self
    }

    /// 设置列标题字号。
    #[must_use]
    pub fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size.max(8.0);
        self
    }

    /// 控制是否显示 1px 列间竖分隔线。
    #[must_use]
    pub fn with_dividers(mut self, show_dividers: bool) -> Self {
        self.show_dividers = show_dividers;
        self
    }

    /// 设置绘制深度键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 所有列与前导缩进合计的总内容宽度。
    #[must_use]
    pub fn total_width(&self) -> f32 {
        self.leading_inset.max(0.0)
            + self
                .columns
                .iter()
                .map(|col| col.width.max(1.0))
                .sum::<f32>()
    }
}

/// 表头控件子热区命中类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnHeaderHitKind {
    /// 第 `index` 列的列头主体（悬停高亮与点击排序）。
    Column(usize),
    /// 第 `index` 列末尾的 `⌄` 下拉过滤子热区。
    FilterButton(usize),
    /// 第 `index` 列右边界 8px 调宽/双击自适应热区。
    ResizeHandle(usize),
}

/// 为表头子热区生成确定性子节点键。
#[must_use]
pub const fn column_header_sub_node(node: u64, kind: ColumnHeaderHitKind) -> u64 {
    let salt: u64 = match kind {
        ColumnHeaderHitKind::Column(idx) => 0x4348_1000 + idx as u64,
        ColumnHeaderHitKind::FilterButton(idx) => 0x4348_2000 + idx as u64,
        ColumnHeaderHitKind::ResizeHandle(idx) => 0x4348_3000 + idx as u64,
    };
    node ^ salt
}

/// 表头控件离散交互事件。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnHeaderEvent {
    /// 点击可排序列头主体，请求切换排序列或翻转排序方向。
    ColumnClicked { column_index: usize },
    /// 拖拽列边界过程中实时派发的新列宽（已夹紧至 `[min_width, max_width]`）。
    ColumnResized { column_index: usize, width: f32 },
    /// 拖拽列边界释放（`PointerUp`）提交最终列宽。
    ColumnResizeCommitted { column_index: usize, width: f32 },
    /// 双击列右边界 8px 热区，请求按内容自适应列宽。
    ColumnAutoFitRequested { column_index: usize },
    /// 点击列尾 `⌄` 下拉过滤子热区，请求在指定锚点矩形弹出列过滤菜单。
    ColumnFilterRequested {
        column_index: usize,
        anchor_rect: [i32; 4],
    },
}

/// 单列表头单元格的确定性几何布局（已计入 `scroll_x` 偏移）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnHeaderCellLayout {
    /// 列索引。
    pub index: usize,
    /// 列头单元格完整矩形。
    pub cell_rect: Rect,
    /// 标题文本可用区域矩形（已扣除左右内边距、独立排序箭头与过滤按钮占位）。
    pub label_rect: Rect,
    /// 独立排序矢量箭头矩形（仅当前排序列为 `Some`）。
    pub sort_arrow_rect: Option<Rect>,
    /// 列尾 `⌄` 下拉过滤按钮矩形（仅 `filterable == true` 且列宽足够时为 `Some`）。
    pub filter_rect: Option<Rect>,
    /// 右边界 1px 竖分隔线矩形。
    pub divider_rect: Rect,
    /// 右边界 8px 拖拽调宽热区矩形（仅 `resizable == true` 时为 `Some`）。
    pub resize_rect: Option<Rect>,
}

/// 表头控件整体确定性几何布局（供绘制器、命中测试与 `ViewBuilder` 文本排版共用）。
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnHeaderBarLayout {
    /// 表头条外框矩形。
    pub bar_rect: Rect,
    /// 所有列与前导缩进的总内容宽度。
    pub content_width: f32,
    /// 本次布局生效的横向滚动偏移。
    pub scroll_x: f32,
    /// 按列顺序排列的单元格布局列表。
    pub cells: Vec<ColumnHeaderCellLayout>,
}

/// 计算表头控件各列及子热区的确定性几何布局。
#[must_use]
pub fn compute_column_header_layout(desc: &ColumnHeaderBarDesc) -> ColumnHeaderBarLayout {
    let scroll_x = desc.scroll_x.max(0.0);
    let leading_inset = desc.leading_inset.max(0.0);
    let bar_y = desc.rect.origin.y;
    let bar_h = desc.rect.size.y.max(0.0);
    let mut cursor_x = desc.rect.origin.x + leading_inset - scroll_x;
    let mut cells = Vec::with_capacity(desc.columns.len());
    let mut content_width = leading_inset;

    for (idx, col) in desc.columns.iter().enumerate() {
        let w = col.width.max(1.0);
        content_width += w;

        let cell_rect = Rect::new(cursor_x, bar_y, w, bar_h);
        let divider_v_inset = (bar_h * 0.18).clamp(3.0, 6.0);
        let divider_rect = Rect::new(
            (cursor_x + w - COLUMN_DIVIDER_WIDTH).round(),
            bar_y + divider_v_inset,
            COLUMN_DIVIDER_WIDTH,
            (bar_h - divider_v_inset * 2.0).max(0.0),
        );

        let resize_rect = col.resizable.then(|| {
            Rect::new(
                cursor_x + w - COLUMN_RESIZE_HANDLE_WIDTH * 0.5,
                bar_y,
                COLUMN_RESIZE_HANDLE_WIDTH,
                bar_h,
            )
        });

        let filter_rect = (col.filterable && w >= COLUMN_FILTER_BUTTON_WIDTH + 16.0).then(|| {
            let fx = cursor_x + w - COLUMN_FILTER_BUTTON_WIDTH - 2.0;
            Rect::new(
                fx,
                bar_y + 2.0,
                COLUMN_FILTER_BUTTON_WIDTH,
                (bar_h - 4.0).max(0.0),
            )
        });

        let mut right_limit = cursor_x + w - CELL_HORIZONTAL_PADDING;
        if let Some(fr) = filter_rect {
            right_limit = fr.origin.x - 2.0;
        }

        let is_sorted = desc.sort.is_some_and(|(sort_idx, _)| sort_idx == idx);
        let sort_arrow_rect = is_sorted.then(|| {
            let arrow_x = (right_limit - SORT_ARROW_SIZE).max(cursor_x + 4.0);
            let arrow_y = bar_y + (bar_h - SORT_ARROW_SIZE) * 0.5;
            right_limit = arrow_x - 4.0;
            Rect::new(arrow_x, arrow_y, SORT_ARROW_SIZE, SORT_ARROW_SIZE)
        });

        let left_limit = cursor_x + CELL_HORIZONTAL_PADDING;
        let label_rect = Rect::new(
            left_limit,
            bar_y,
            (right_limit - left_limit).max(0.0),
            bar_h,
        );

        cells.push(ColumnHeaderCellLayout {
            index: idx,
            cell_rect,
            label_rect,
            sort_arrow_rect,
            filter_rect,
            divider_rect,
            resize_rect,
        });

        cursor_x += w;
    }

    ColumnHeaderBarLayout {
        bar_rect: desc.rect,
        content_width,
        scroll_x,
        cells,
    }
}

/// 正在进行的列宽拖拽会话快照。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnResizeDrag {
    /// 被拖拽调整宽度的列索引。
    pub column_index: usize,
    /// 按下时的指针水平坐标。
    pub start_x: f32,
    /// 拖拽开始时的列宽。
    pub initial_width: f32,
    /// 拖拽过程中的当前实时列宽。
    pub current_width: f32,
    /// 本次拖拽会话中是否实际产生过列宽变化。
    pub moved: bool,
}

/// 表头控件交互状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnHeaderBarState {
    /// 当前悬停的子热区（可由运行时按子节点键反解或由调用方直接维护）。
    pub hovered_hit: Option<ColumnHeaderHitKind>,
    /// 当前按压的子热区。
    pub pressed_hit: Option<ColumnHeaderHitKind>,
    /// 当前正在进行的列宽拖拽会话。
    pub active_resize: Option<ColumnResizeDrag>,
    /// 与下方列表共享的横向滚动偏移（逻辑像素）。
    pub scroll_x: f32,
    /// 待业务层消费的离散交互事件队列。
    events: Vec<ColumnHeaderEvent>,
}

impl ColumnHeaderBarState {
    /// 创建默认空闲状态的表头交互状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出并清空当前累积的离散交互事件。
    pub fn drain_events(&mut self) -> Vec<ColumnHeaderEvent> {
        std::mem::take(&mut self.events)
    }

    /// 当前是否正处于列宽拖拽中。
    #[must_use]
    pub const fn is_resizing(&self) -> bool {
        self.active_resize.is_some()
    }

    /// 若正在拖拽调宽，返回对应的列索引。
    #[must_use]
    pub fn resizing_column(&self) -> Option<usize> {
        self.active_resize.map(|drag| drag.column_index)
    }

    /// 同步共享横向滚动偏移（自动夹紧至非负值）。
    pub fn set_scroll_x(&mut self, scroll_x: f32) {
        self.scroll_x = scroll_x.max(0.0);
    }

    /// 读取当前共享横向滚动偏移。
    #[must_use]
    pub const fn scroll_x(&self) -> f32 {
        self.scroll_x
    }

    /// 按优先级执行精确子热区命中测试：`ResizeHandle > FilterButton > Column`。
    #[must_use]
    pub fn hit_test(&self, desc: &ColumnHeaderBarDesc, point: Vec2) -> Option<ColumnHeaderHitKind> {
        if !desc.rect.contains_point(point) {
            return None;
        }
        let layout = compute_column_header_layout(desc);

        // 1. 列边界 8px 调宽热区优先级最高，防止拖拽边界时误触发相邻列排序或过滤。
        for cell in &layout.cells {
            if let Some(rr) = cell.resize_rect {
                let clipped = rr.intersect(&desc.rect);
                if !clipped.is_empty() && clipped.contains_point(point) {
                    return Some(ColumnHeaderHitKind::ResizeHandle(cell.index));
                }
            }
        }

        // 2. 列尾 `⌄` 下拉过滤子热区次之。
        for cell in &layout.cells {
            if let Some(fr) = cell.filter_rect {
                let clipped = fr.intersect(&desc.rect);
                if !clipped.is_empty() && clipped.contains_point(point) {
                    return Some(ColumnHeaderHitKind::FilterButton(cell.index));
                }
            }
        }

        // 3. 最后命中列头主体。
        for cell in &layout.cells {
            let clipped = cell.cell_rect.intersect(&desc.rect);
            if !clipped.is_empty() && clipped.contains_point(point) {
                return Some(ColumnHeaderHitKind::Column(cell.index));
            }
        }

        None
    }

    /// 查询指针位于 `point` 时表头应呈现的光标形态。
    #[must_use]
    pub fn cursor_at(&self, desc: &ColumnHeaderBarDesc, point: Vec2) -> CursorKind {
        if self.active_resize.is_some() {
            return CursorKind::ResizeEastWest;
        }
        match self.hit_test(desc, point) {
            Some(ColumnHeaderHitKind::ResizeHandle(_)) => CursorKind::ResizeEastWest,
            Some(ColumnHeaderHitKind::FilterButton(_)) => CursorKind::Pointer,
            Some(ColumnHeaderHitKind::Column(idx)) => {
                if desc.columns.get(idx).is_some_and(|c| c.sortable) {
                    CursorKind::Pointer
                } else {
                    CursorKind::Default
                }
            }
            None => CursorKind::Default,
        }
    }

    /// 统一处理指针事件（`Down/Move/DragStart/Drag/Up/DoubleClick/CaptureLost`）。
    ///
    /// 返回 `true` 表示事件被表头控件消费或引起了交互状态变化。
    pub fn handle_pointer_event(
        &mut self,
        desc: &ColumnHeaderBarDesc,
        event: PointerEvent,
    ) -> bool {
        match event {
            PointerEvent::Move { position, .. } => {
                if self.active_resize.is_some() {
                    return self.update_resize_drag(desc, position.x);
                }
                let next_hover = self.hit_test(desc, position);
                let changed = next_hover != self.hovered_hit;
                self.hovered_hit = next_hover;
                changed
            }
            PointerEvent::Down {
                position,
                button: PointerButton::Left,
                click_count,
                ..
            } => {
                let hit = self.hit_test(desc, position);
                self.hovered_hit = hit;
                self.pressed_hit = hit;
                if let Some(ColumnHeaderHitKind::ResizeHandle(idx)) = hit {
                    if click_count < 2 {
                        if let Some(col) = desc.columns.get(idx) {
                            self.active_resize = Some(ColumnResizeDrag {
                                column_index: idx,
                                start_x: position.x,
                                initial_width: col.width,
                                current_width: col.width,
                                moved: false,
                            });
                        }
                    } else {
                        self.active_resize = None;
                    }
                    return true;
                }
                hit.is_some()
            }
            PointerEvent::DragStart { position, .. } => {
                if self.active_resize.is_none() {
                    if let Some(ColumnHeaderHitKind::ResizeHandle(idx)) = self.pressed_hit {
                        if let Some(col) = desc.columns.get(idx) {
                            self.active_resize = Some(ColumnResizeDrag {
                                column_index: idx,
                                start_x: position.x,
                                initial_width: col.width,
                                current_width: col.width,
                                moved: false,
                            });
                        }
                    }
                }
                self.active_resize.is_some()
            }
            PointerEvent::Drag { position, .. } => self.update_resize_drag(desc, position.x),
            PointerEvent::Up {
                position,
                button: PointerButton::Left,
                ..
            } => {
                self.hovered_hit = self.hit_test(desc, position);
                if let Some(drag) = self.active_resize.take() {
                    self.pressed_hit = None;
                    if drag.moved {
                        self.events.push(ColumnHeaderEvent::ColumnResizeCommitted {
                            column_index: drag.column_index,
                            width: drag.current_width,
                        });
                    }
                    return true;
                }
                let Some(pressed) = self.pressed_hit.take() else {
                    return false;
                };
                let released = self.hit_test(desc, position);
                if released != Some(pressed) {
                    return false;
                }
                match pressed {
                    ColumnHeaderHitKind::FilterButton(idx) => {
                        let layout = compute_column_header_layout(desc);
                        let anchor = layout
                            .cells
                            .get(idx)
                            .map(|cell| {
                                let clipped = cell.cell_rect.intersect(&desc.rect);
                                if clipped.is_empty() {
                                    cell.cell_rect
                                } else {
                                    clipped
                                }
                            })
                            .unwrap_or(desc.rect);
                        self.events.push(ColumnHeaderEvent::ColumnFilterRequested {
                            column_index: idx,
                            anchor_rect: rect_to_i32(anchor),
                        });
                        true
                    }
                    ColumnHeaderHitKind::Column(idx) => {
                        if desc.columns.get(idx).is_some_and(|c| c.sortable) {
                            self.events
                                .push(ColumnHeaderEvent::ColumnClicked { column_index: idx });
                            true
                        } else {
                            false
                        }
                    }
                    ColumnHeaderHitKind::ResizeHandle(_) => false,
                }
            }
            PointerEvent::DoubleClick { position, .. } => {
                if let Some(ColumnHeaderHitKind::ResizeHandle(idx)) = self.hit_test(desc, position)
                {
                    self.active_resize = None;
                    self.pressed_hit = None;
                    self.events
                        .push(ColumnHeaderEvent::ColumnAutoFitRequested { column_index: idx });
                    return true;
                }
                false
            }
            PointerEvent::CaptureLost { .. } => {
                let had_active = self.active_resize.take().is_some();
                self.pressed_hit = None;
                had_active
            }
            _ => false,
        }
    }

    fn update_resize_drag(&mut self, desc: &ColumnHeaderBarDesc, pointer_x: f32) -> bool {
        let Some(drag) = self.active_resize.as_mut() else {
            return false;
        };
        let Some(col) = desc.columns.get(drag.column_index) else {
            self.active_resize = None;
            return false;
        };
        let delta = pointer_x - drag.start_x;
        let next_width = (drag.initial_width + delta).clamp(col.min_width, col.max_width);
        if (next_width - drag.current_width).abs() > 0.01 {
            drag.current_width = next_width;
            drag.moved = true;
            self.events.push(ColumnHeaderEvent::ColumnResized {
                column_index: drag.column_index,
                width: next_width,
            });
        }
        true
    }
}

/// 发射表头控件的外框、列悬停高亮、1px 竖分隔线、独立排序矢量箭头、列尾 `⌄` 过滤按钮与子热区。
///
/// 不包含列标题字形，供 `unbound-app` 在独立 GPU 裁剪作用域内分别合成背景与文本层。
#[must_use]
pub fn emit_column_header_bar_chrome(
    node: u64,
    desc: &ColumnHeaderBarDesc,
    state: &ColumnHeaderBarState,
    control: ControlState,
) -> WidgetOutput {
    let layout = compute_column_header_layout(desc);
    let z = desc.z;
    let mut primitives = Vec::with_capacity(desc.columns.len() * 6 + 2);
    let mut payloads = Vec::new();

    let active_resize_col = state.resizing_column();

    for cell in &layout.cells {
        let col = &desc.columns[cell.index];
        let cell_hovered = matches!(
            state.hovered_hit,
            Some(ColumnHeaderHitKind::Column(i) | ColumnHeaderHitKind::FilterButton(i))
                if i == cell.index
        );
        let cell_pressed = matches!(
            state.pressed_hit,
            Some(ColumnHeaderHitKind::Column(i)) if i == cell.index
        );
        let filter_hovered = matches!(
            state.hovered_hit,
            Some(ColumnHeaderHitKind::FilterButton(i)) if i == cell.index
        );
        let filter_pressed = matches!(
            state.pressed_hit,
            Some(ColumnHeaderHitKind::FilterButton(i)) if i == cell.index
        );
        let resize_hovered = matches!(
            state.hovered_hit,
            Some(ColumnHeaderHitKind::ResizeHandle(i)) if i == cell.index
        );
        let resizing_this = active_resize_col == Some(cell.index);

        // 1. 列单元格悬停/按压底板
        let cell_bg = if control.disabled {
            Token::NONE
        } else if cell_pressed {
            Token::PRESSED
        } else if cell_hovered {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        if cell_bg != Token::NONE {
            primitives.push(surface(cell.cell_rect, 4.0, cell_bg, z));
        }

        // 2. 独立排序矢量箭头（零字体依赖）
        if let Some(arrow_rect) = cell.sort_arrow_rect {
            if let Some((_, dir)) = desc.sort {
                let icon = match dir {
                    SortDir::Ascending => ShellIcon::ChevronUp,
                    SortDir::Descending => ShellIcon::ChevronDown,
                };
                let arrow_color = if control.disabled {
                    Token::TEXT_DISABLED
                } else {
                    Token::TEXT_PRIMARY
                };
                emit_shell_icon_primitives(
                    icon,
                    arrow_rect,
                    arrow_color,
                    Token::NONE,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
        }

        // 3. 列尾 `⌄` 下拉过滤子热区（悬停该列、点击按压或已激活过滤时显示）
        if let Some(fr) = cell.filter_rect {
            let show_filter = col.filter_active
                || cell_hovered
                || cell_pressed
                || filter_hovered
                || filter_pressed;
            if show_filter {
                let btn_bg = if control.disabled {
                    Token::NONE
                } else if filter_pressed {
                    Token::PRESSED
                } else if filter_hovered {
                    Token::CONTROL_HOVER
                } else {
                    Token::NONE
                };
                if btn_bg != Token::NONE {
                    primitives.push(surface(fr, 3.0, btn_bg, z + 1));
                }
                let chev_color = if control.disabled {
                    Token::TEXT_DISABLED
                } else if col.filter_active {
                    Token::PRIMARY
                } else if filter_hovered || filter_pressed {
                    Token::TEXT_PRIMARY
                } else {
                    Token::TEXT_SECONDARY
                };
                emit_shell_icon_primitives(
                    ShellIcon::ChevronDown,
                    fr,
                    chev_color,
                    btn_bg,
                    z + 2,
                    &mut primitives,
                    &mut payloads,
                );
            }
        }

        // 4. 列间 1px 竖分隔线（拖拽中或悬停边界时强调高亮）
        if desc.show_dividers {
            let divider_color = if control.disabled {
                Token::DIVIDER
            } else if resizing_this {
                Token::PRIMARY
            } else if resize_hovered {
                Token::OUTLINE_STRONG
            } else {
                Token::DIVIDER
            };
            primitives.push(instance(
                cell.divider_rect,
                [0.0; 4],
                divider_color,
                Token::NONE,
                0.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Border,
                z + 1,
            ));
        }
    }

    // 5. 表头底部 1px 横向分隔线
    if desc.rect.size.x > 0.0 && desc.rect.size.y > 0.0 {
        let bottom_line = Rect::new(
            desc.rect.origin.x,
            (desc.rect.max().y - 1.0).max(desc.rect.origin.y),
            desc.rect.size.x,
            1.0,
        );
        primitives.push(instance(
            bottom_line,
            [0.0; 4],
            Token::DIVIDER,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            z + 1,
        ));
    }

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| HitRegion::new(node, LAYER, u32::from(z), desc.rect)),
        a11y: Some(
            A11yNode::new(Role::Row, "Column Header Bar", desc.rect).disabled(control.disabled),
        ),
        focusable: false,
        ..Default::default()
    };

    if !control.disabled {
        for cell in &layout.cells {
            let col = &desc.columns[cell.index];

            // 列头主体子热区
            let clipped_cell = cell.cell_rect.intersect(&desc.rect);
            if !clipped_cell.is_empty() {
                let col_key = column_header_sub_node(node, ColumnHeaderHitKind::Column(cell.index));
                let col_cursor = if col.sortable {
                    CursorKind::Pointer
                } else {
                    CursorKind::Default
                };
                out.with_sub_control(
                    col_key,
                    HitRegion::new(col_key, LAYER, u32::from(z) + 1, clipped_cell)
                        .with_cursor(col_cursor),
                    A11yNode::new(Role::ColumnHeader, col.title.clone(), clipped_cell).clickable(),
                    col.sortable,
                );
            }

            // 列尾 `⌄` 下拉过滤子热区
            if let Some(fr) = cell.filter_rect {
                let clipped_filter = fr.intersect(&desc.rect);
                if !clipped_filter.is_empty() {
                    let filter_key =
                        column_header_sub_node(node, ColumnHeaderHitKind::FilterButton(cell.index));
                    out.with_sub_control(
                        filter_key,
                        HitRegion::new(filter_key, LAYER, u32::from(z) + 2, clipped_filter)
                            .with_cursor(CursorKind::Pointer),
                        A11yNode::new(
                            Role::Button,
                            format!("Filter {}", col.title),
                            clipped_filter,
                        )
                        .clickable(),
                        false,
                    );
                }
            }

            // 列右边界 8px 调宽热区（最高层级 z+3 与 ResizeEastWest 光标）
            if let Some(rr) = cell.resize_rect {
                let clipped_resize = rr.intersect(&desc.rect);
                if !clipped_resize.is_empty() {
                    let resize_key =
                        column_header_sub_node(node, ColumnHeaderHitKind::ResizeHandle(cell.index));
                    out.with_sub_control(
                        resize_key,
                        HitRegion::new(resize_key, LAYER, u32::from(z) + 3, clipped_resize)
                            .with_cursor(CursorKind::ResizeEastWest),
                        A11yNode::new(
                            Role::Splitter,
                            format!("Resize {}", col.title),
                            clipped_resize,
                        ),
                        false,
                    );
                }
            }
        }
    }

    out
}

/// 发射表头列标题字形层。
#[must_use]
pub fn emit_column_header_bar_text_layer(
    glyphs: &[GlyphQuad],
    disabled: bool,
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let color = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    emit_glyphs(glyphs, color, z + 2, &mut primitives, &mut payloads);
    WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    }
}

/// 一次性发射完整表头控件（外框、分隔线、矢量箭头、子热区与标题字形）。
#[must_use]
pub fn emit_column_header_bar(
    node: u64,
    desc: &ColumnHeaderBarDesc,
    state: &ColumnHeaderBarState,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut out = emit_column_header_bar_chrome(node, desc, state, control);
    let text_out = emit_column_header_bar_text_layer(glyphs, control.disabled, desc.z);
    out.primitives.extend(text_out.primitives);
    out.payloads.extend(text_out.payloads);
    out
}

fn rect_to_i32(r: Rect) -> [i32; 4] {
    [
        r.origin.x.round() as i32,
        r.origin.y.round() as i32,
        r.size.x.round() as i32,
        r.size.y.round() as i32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_desc() -> ColumnHeaderBarDesc {
        ColumnHeaderBarDesc::new(
            Rect::new(0.0, 0.0, 480.0, COLUMN_HEADER_BAR_HEIGHT),
            vec![
                ColumnHeaderSpec::new("Name", 200.0).with_min_width(80.0),
                ColumnHeaderSpec::new("Date Modified", 150.0).with_min_width(90.0),
                ColumnHeaderSpec::new("Size", 100.0)
                    .right_aligned()
                    .with_min_width(60.0),
            ],
        )
        .with_sort(Some((0, SortDir::Ascending)))
    }

    #[test]
    fn layout_applies_scroll_x_dividers_sort_arrow_and_8px_resize_handle() {
        let desc = sample_desc().with_scroll_x(24.0);
        let layout = compute_column_header_layout(&desc);

        assert_eq!(layout.cells.len(), 3);
        assert!((layout.content_width - 450.0).abs() < 0.01);
        assert!((layout.scroll_x - 24.0).abs() < 0.01);

        // 第 0 列受 scroll_x=24 平移
        let c0 = layout.cells[0];
        assert!((c0.cell_rect.origin.x - (-24.0)).abs() < 0.01);
        assert!((c0.cell_rect.size.x - 200.0).abs() < 0.01);
        // 1px 竖分隔线位于列右边缘
        assert!((c0.divider_rect.size.x - 1.0).abs() < 0.01);
        assert!((c0.divider_rect.origin.x - 175.0).abs() < 0.01);
        // 8px 调宽热区以列右边缘 176.0 为中心
        let r0 = c0.resize_rect.expect("column 0 resize handle");
        assert!((r0.size.x - 8.0).abs() < 0.01);
        assert!((r0.origin.x - 172.0).abs() < 0.01);
        // 排序列具备独立排序箭头与列尾过滤按钮
        assert!(c0.sort_arrow_rect.is_some());
        assert!(c0.filter_rect.is_some());
        // 非排序列不生成排序箭头
        assert!(layout.cells[1].sort_arrow_rect.is_none());
        assert_eq!(desc.columns[2].align, ColumnHeaderAlign::Right);
    }

    #[test]
    fn hit_test_prioritizes_resize_handle_over_filter_and_column() {
        let desc = sample_desc();
        let state = ColumnHeaderBarState::new();

        // 列 0 右边界 x=200，8px 热区覆盖 [196, 204]
        assert_eq!(
            state.hit_test(&desc, Vec2::new(199.0, 16.0)),
            Some(ColumnHeaderHitKind::ResizeHandle(0))
        );
        assert_eq!(
            state.cursor_at(&desc, Vec2::new(199.0, 16.0)),
            CursorKind::ResizeEastWest
        );

        // 列 0 过滤按钮位于 [178, 198]，避开 resize 热区取 185.0
        assert_eq!(
            state.hit_test(&desc, Vec2::new(185.0, 16.0)),
            Some(ColumnHeaderHitKind::FilterButton(0))
        );
        assert_eq!(
            state.cursor_at(&desc, Vec2::new(185.0, 16.0)),
            CursorKind::Pointer
        );

        // 列 0 主体
        assert_eq!(
            state.hit_test(&desc, Vec2::new(60.0, 16.0)),
            Some(ColumnHeaderHitKind::Column(0))
        );

        // 超出表头矩形返回 None
        assert_eq!(state.hit_test(&desc, Vec2::new(60.0, 40.0)), None);
    }

    #[test]
    fn pointer_drag_resizes_column_with_clamp_and_commits_on_up() {
        let desc = sample_desc();
        let mut state = ColumnHeaderBarState::new();

        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Down {
                target: Some(1),
                position: Vec2::new(200.0, 16.0),
                button: PointerButton::Left,
                click_count: 1,
            },
        ));
        assert!(state.is_resizing());
        assert_eq!(state.resizing_column(), Some(0));

        // 向右拖拽 +40px -> 240.0
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Drag {
                target: 1,
                position: Vec2::new(240.0, 16.0),
                delta: Vec2::new(40.0, 0.0),
            },
        ));
        // 向左极限拖拽 -300px -> 夹紧到 min_width=80.0
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Drag {
                target: 1,
                position: Vec2::new(-100.0, 16.0),
                delta: Vec2::new(-340.0, 0.0),
            },
        ));
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Up {
                target: Some(1),
                position: Vec2::new(-100.0, 16.0),
                button: PointerButton::Left,
            },
        ));
        assert!(!state.is_resizing());

        let events = state.drain_events();
        assert_eq!(
            events,
            vec![
                ColumnHeaderEvent::ColumnResized {
                    column_index: 0,
                    width: 240.0,
                },
                ColumnHeaderEvent::ColumnResized {
                    column_index: 0,
                    width: 80.0,
                },
                ColumnHeaderEvent::ColumnResizeCommitted {
                    column_index: 0,
                    width: 80.0,
                },
            ]
        );
    }

    #[test]
    fn double_click_on_resize_handle_emits_autofit_request() {
        let desc = sample_desc();
        let mut state = ColumnHeaderBarState::new();

        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::DoubleClick {
                target: 1,
                position: Vec2::new(200.0, 16.0),
            },
        ));
        assert_eq!(
            state.drain_events(),
            vec![ColumnHeaderEvent::ColumnAutoFitRequested { column_index: 0 }]
        );
    }

    #[test]
    fn filter_button_click_emits_filter_requested_and_column_click_emits_sort() {
        let desc = sample_desc();
        let mut state = ColumnHeaderBarState::new();

        // 点击列 1 末尾过滤子热区（列 1 占 [200, 350]，过滤按钮 [328, 348]）
        let filter_pt = Vec2::new(335.0, 16.0);
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Down {
                target: Some(1),
                position: filter_pt,
                button: PointerButton::Left,
                click_count: 1,
            },
        ));
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Up {
                target: Some(1),
                position: filter_pt,
                button: PointerButton::Left,
            },
        ));
        let events = state.drain_events();
        assert_eq!(
            events,
            vec![ColumnHeaderEvent::ColumnFilterRequested {
                column_index: 1,
                anchor_rect: [200, 0, 150, 32],
            }]
        );

        // 点击列 2 主体触发排序事件
        let col_pt = Vec2::new(380.0, 16.0);
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Down {
                target: Some(1),
                position: col_pt,
                button: PointerButton::Left,
                click_count: 1,
            },
        ));
        assert!(state.handle_pointer_event(
            &desc,
            PointerEvent::Up {
                target: Some(1),
                position: col_pt,
                button: PointerButton::Left,
            },
        ));
        assert_eq!(
            state.drain_events(),
            vec![ColumnHeaderEvent::ColumnClicked { column_index: 2 }]
        );
    }

    #[test]
    fn emit_column_header_bar_outputs_dividers_sort_vector_filter_chevron_and_resize_cursor() {
        let desc = sample_desc();
        let mut state = ColumnHeaderBarState::new();
        state.hovered_hit = Some(ColumnHeaderHitKind::Column(1));

        let out = emit_column_header_bar(10, &desc, &state, ControlState::default(), &[]);

        // 应包含 3 个列主热区 + 3 个过滤子热区 + 3 个调宽热区 = 9 个 sub_hits
        assert_eq!(out.sub_hits.len(), 9);
        assert!(out.sub_hits.iter().any(|h| h.node
            == column_header_sub_node(10, ColumnHeaderHitKind::ResizeHandle(0))
            && h.cursor == Some(CursorKind::ResizeEastWest)));
        assert!(out.sub_hits.iter().any(|h| h.node
            == column_header_sub_node(10, ColumnHeaderHitKind::FilterButton(1))
            && h.cursor == Some(CursorKind::Pointer)));

        // 独立排序箭头（2 条曲线段）+ 悬停列过滤 `⌄`（2 条曲线段）均应作为矢量图元发射
        let curve_count = out
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(curve_count, 4);

        // 3 条 1px 竖分隔线 + 1 条底部分隔线 = 4 个 Border 子元素
        let border_count = out
            .primitives
            .iter()
            .filter(|p| {
                unbound_gpu::data::GpuPrimitiveInstance::unpack_type(p.prim_type_and_flags).2
                    == SubElement::Border as u16
            })
            .count();
        assert_eq!(border_count, 4);
    }
}

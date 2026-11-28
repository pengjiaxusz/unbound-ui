//! 标签页栏：水平滚动的标签页行，带动画选中指示器与 Win11 壳层卡片融合形态。
//!
//! 标签页栏仅持有标识与几何信息，从不持有内容。每个标签页由单独调用输出，
//! 以便关闭按钮拥有独立的命中区域，且调用方可在标签页内放置任意标签字形四边形。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_anim::Spring;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs, instance, outlined, surface};
use crate::shell::{emit_widget_icon_primitives, ShellIcon, WidgetIcon};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 单个标签页的默认宽度（逻辑像素）。
///
/// 落在 WinUI `TabViewItemMinWidth = 100` 与 `TabViewItemMaxWidth = 240` 之间。
pub const TAB_WIDTH: f32 = 140.0;

/// 自适应模式下单个标签页的最小宽度（WinUI `TabViewItemMinWidth = 100`）。
pub const TAB_MIN_WIDTH: f32 = 100.0;

/// 自适应模式下单个标签页的最大宽度（WinUI `TabViewItemMaxWidth = 240`）。
pub const TAB_MAX_WIDTH: f32 = 240.0;

/// 标签页栏默认高度（对齐 Win11 融合标题栏 `48px`：`38px` 标签卡片 + `10px` 顶部留白）。
pub const TAB_STRIP_HEIGHT: f32 = 48.0;

/// 标签页标准圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
const TAB_RADIUS: f32 = 4.0;

/// 壳层融合态选中标签顶部圆角半径（对齐 WinUI `OverlayCornerRadius = 8`）。
pub const TAB_SHELL_CORNER_RADIUS: f32 = 8.0;

/// 壳层融合态标签卡片高度（对齐 Win11 原生 `36–38px` 尺度，取 `38px`）。
pub const TAB_SHELL_ITEM_HEIGHT: f32 = 38.0;

/// 壳层融合态顶部留白（`48px` 融合标题栏减去 `38px` 标签卡片高度 = `10px`）。
pub const TAB_SHELL_TOP_INSET: f32 = 10.0;

/// 关闭按钮图标宽度（对齐 Win11 标签页 `12px` 关闭叉号尺度）。
pub const TAB_CLOSE: f32 = 12.0;

/// 壳层融合态关闭按钮独立命中盒边长（24×24，内部居中 12×12 关闭叉号图标）。
pub const TAB_CLOSE_HIT_SIZE: f32 = 24.0;

/// 壳层融合态“+”新增按钮宽度（32×28，圆角 4px）。
pub const TAB_ADD_WIDTH: f32 = 32.0;

/// 壳层融合态“+”新增按钮高度。
pub const TAB_ADD_HEIGHT: f32 = 28.0;

/// 壳层融合态“+”新增按钮与末标签之间的水平间距。
pub const TAB_ADD_GAP: f32 = 4.0;

/// 派生单个标签页主体的确定性子热区键。
#[must_use]
pub const fn tab_item_node(tabs_key: u64, index: usize) -> u64 {
    tabs_key ^ 0x5441_4249_0000_0000u64 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15u64) ^ 1
}

/// 派生单个标签页关闭按钮（`×`）的确定性子热区键。
#[must_use]
pub const fn tab_close_node(tabs_key: u64, index: usize) -> u64 {
    tabs_key ^ 0x5441_4243_0000_0000u64 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15u64) ^ 2
}

/// 派生标签栏末尾新建按钮（`+`）的确定性子热区键。
#[must_use]
pub const fn tab_add_node(tabs_key: u64) -> u64 {
    tabs_key ^ 0x5441_4241_0000_0000u64 ^ 0x5441_42FFu64
}

/// `TabView` 视觉风格变体。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabViewStyle {
    /// 标准底部指示条风格（向后兼容既有调用与默认差分）。
    #[default]
    Standard,
    /// Win11 壳层卡片融合风格：顶部圆角卡片与下方表面无缝衔接，无底部蓝条。
    ShellChrome,
}

/// 标签宽度分配策略。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TabWidthMode {
    /// 固定宽度（兼容既有 `tab_width`）。
    #[default]
    Fixed,
    /// Win11 自适应宽：在 `[min_width, max_width]` 之间随容器宽度弹性缩放。
    Adaptive {
        /// 单个标签压缩下限（触底后开启水平溢出滚动）。
        min_width: f32,
        /// 单个标签展开上限。
        max_width: f32,
    },
}

impl TabWidthMode {
    /// Win11 标准自适应宽度范围（`100..=240px`）。
    #[must_use]
    pub const fn adaptive_default() -> Self {
        Self::Adaptive {
            min_width: TAB_MIN_WIDTH,
            max_width: TAB_MAX_WIDTH,
        }
    }
}

/// 标签页拖出主窗边界触发独立副窗撕裂的默认逻辑像素阈值。
pub const TAB_TEAR_OFF_THRESHOLD: f32 = 36.0;

/// `TabView` 交互事件。
#[derive(Clone, Debug, PartialEq)]
pub enum TabViewEvent {
    /// 选中了某个标签页。
    Selected {
        /// 标签页稳定标识。
        id: u64,
        /// 标签页索引。
        index: usize,
    },
    /// 点击关闭按钮请求关闭某个标签页。
    CloseRequested {
        /// 标签页稳定标识。
        id: u64,
        /// 标签页索引。
        index: usize,
    },
    /// 鼠标中键点击某个标签页（通常用于快捷关闭）。
    MiddleClicked {
        /// 标签页稳定标识。
        id: u64,
        /// 标签页索引。
        index: usize,
    },
    /// 点击“+”按钮请求新建标签页。
    AddClicked,
    /// 在某个标签页上请求右键上下文菜单。
    ContextMenuRequested {
        /// 标签页稳定标识。
        id: u64,
        /// 标签页索引。
        index: usize,
        /// 触发菜单的屏幕逻辑坐标。
        screen_pos: [i32; 2],
    },
    /// 在标签栏内拖拽重排标签页顺序。
    Reordered {
        /// 标签页稳定标识。
        id: u64,
        /// 拖拽前索引。
        from_index: usize,
        /// 拖拽后索引。
        to_index: usize,
    },
    /// 标签页拖出主窗边界或撕裂阈值并释放，请求创建独立原生副窗。
    TearOffRequested {
        /// 标签页稳定标识。
        id: u64,
        /// 标签页索引。
        index: usize,
        /// 释放时的屏幕物理坐标（已按 DPI 换算）。
        screen_pos: [i32; 2],
    },
}

/// 单个标签页拖拽与离窗撕裂会话状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDragSession {
    /// 被拖拽标签页的稳定标识。
    pub id: u64,
    /// 按下时的原始索引。
    pub origin_index: usize,
    /// 当前重排目标索引。
    pub current_index: usize,
    /// 按下时的窗口局部逻辑坐标。
    pub press_pos: Vec2,
    /// 最近一次移动时的窗口局部逻辑坐标。
    pub current_pos: Vec2,
    /// 当前是否已超出标签栏离窗阈值（处于可撕裂状态）。
    pub detached: bool,
}

/// 单个标签页。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabItem {
    /// 稳定标识，同时用作该标签页的节点 ID。
    pub id: u64,
    /// 可见标题。
    pub title: String,
    /// 可选前置矢量图标。
    pub icon: Option<ShellIcon>,
    /// 统一前置图标槽（支持内置矢量与外部彩色纹理，优先于 `icon`）。
    pub widget_icon: WidgetIcon,
    /// 该标签页是否带有关闭按钮。
    pub closable: bool,
    /// 是否为固定标签页（固定标签不展示关闭按钮）。
    pub pinned: bool,
    /// 可选悬停提示文案（常用于展示完整路径或长标题）。
    pub tooltip: Option<String>,
}

impl TabItem {
    /// 创建可关闭的标签页。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            icon: None,
            widget_icon: WidgetIcon::None,
            closable: true,
            pinned: false,
            tooltip: None,
        }
    }

    /// 设为不带关闭按钮。
    #[must_use]
    pub fn fixed(mut self) -> Self {
        self.closable = false;
        self
    }

    /// 设置标签页前置图标。
    #[must_use]
    pub fn with_icon(mut self, icon: ShellIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 设置标签页统一前置图标（支持内置矢量或外部彩色纹理）。
    #[must_use]
    pub fn with_widget_icon(mut self, icon: WidgetIcon) -> Self {
        self.widget_icon = icon;
        self
    }

    /// 解析标签页实际生效的前置图标：优先返回 `widget_icon`，其次回退至 `icon`。
    #[must_use]
    pub const fn effective_icon(&self) -> WidgetIcon {
        match self.widget_icon {
            WidgetIcon::None => match self.icon {
                Some(shell_icon) => WidgetIcon::Vector(shell_icon),
                None => WidgetIcon::None,
            },
            other => other,
        }
    }

    /// 标记为固定标签页（同时隐藏关闭按钮）。
    #[must_use]
    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self.closable = false;
        self
    }

    /// 设置悬停提示文案。
    #[must_use]
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

/// 标签页栏的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TabsDesc {
    /// 标签页栏矩形。
    pub rect: Rect,
    /// 从左到右排列的标签页列表。
    pub items: Vec<TabItem>,
    /// 是否为每个标签页显示关闭按钮。
    pub closable: bool,
    /// 是否显示末尾的“+”按钮。
    pub addable: bool,
    /// 单个标签页的固定宽度（在 [`TabWidthMode::Fixed`] 下生效）。
    pub tab_width: f32,
    /// 视觉风格变体（默认 [`TabViewStyle::Standard`]）。
    pub style: TabViewStyle,
    /// 标签宽度分配策略（默认 [`TabWidthMode::Fixed`]）。
    pub width_mode: TabWidthMode,
    /// 在 [`TabViewStyle::ShellChrome`] 模式下是否为激活标签页绘制 1px 外描边（默认 `false`，对齐 Win11 无描边平滑表面衔接）。
    pub shell_active_border: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl TabsDesc {
    /// 使用指定标题列表创建标签页栏。
    #[must_use]
    pub fn new(rect: Rect, titles: &[&str]) -> Self {
        Self {
            rect,
            items: titles
                .iter()
                .enumerate()
                .map(|(index, title)| TabItem::new(index as u64 + 1, *title))
                .collect(),
            closable: true,
            addable: false,
            tab_width: TAB_WIDTH,
            style: TabViewStyle::Standard,
            width_mode: TabWidthMode::Fixed,
            shell_active_border: false,
            z: 620,
        }
    }

    /// 使用显式标签页标识与标题。
    #[must_use]
    pub fn with_items(mut self, items: Vec<TabItem>) -> Self {
        self.items = items;
        self
    }

    /// 显示或隐藏末尾的“+”按钮。
    #[must_use]
    pub fn with_addable(mut self, addable: bool) -> Self {
        self.addable = addable;
        self
    }

    /// 覆盖标签页固定宽度。
    #[must_use]
    pub fn with_tab_width(mut self, width: f32) -> Self {
        self.tab_width = width.max(24.0);
        self
    }

    /// 设置视觉风格变体。
    #[must_use]
    pub fn with_style(mut self, style: TabViewStyle) -> Self {
        self.style = style;
        self
    }

    /// 设置标签宽度分配策略。
    #[must_use]
    pub fn with_width_mode(mut self, width_mode: TabWidthMode) -> Self {
        self.width_mode = width_mode;
        self
    }

    /// 控制 [`TabViewStyle::ShellChrome`] 激活标签页是否绘制 1px 外描边（默认 `false`，无描边无缝融合）。
    #[must_use]
    pub fn with_shell_active_border(mut self, enabled: bool) -> Self {
        self.shell_active_border = enabled;
        self
    }

    /// 标签页数量。
    #[must_use]
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// 不含新增按钮的标签页可用宽度。
    #[must_use]
    pub fn strip_width(&self) -> f32 {
        let add = if self.addable {
            match self.style {
                TabViewStyle::Standard => TAB_STRIP_HEIGHT,
                TabViewStyle::ShellChrome => TAB_ADD_GAP + TAB_ADD_WIDTH,
            }
        } else {
            0.0
        };
        (self.rect.size.x - add).max(0.0)
    }

    /// 当前宽度模式下实际生效的单标签宽度。
    #[must_use]
    pub fn effective_tab_width(&self) -> f32 {
        match self.width_mode {
            TabWidthMode::Fixed => self.tab_width.max(24.0),
            TabWidthMode::Adaptive {
                min_width,
                max_width,
            } => {
                let min_w = min_width.max(24.0);
                let max_w = max_width.max(min_w);
                let count = self.count();
                if count == 0 {
                    max_w
                } else {
                    (self.strip_width() / count as f32).clamp(min_w, max_w)
                }
            }
        }
    }

    /// 最后一个标签页右边缘齐平前可滚动的最大距离。
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.count() as f32 * self.effective_tab_width() - self.strip_width()).max(0.0)
    }

    /// 当前标签流是否超出可视宽度并进入水平溢出滚动。
    #[must_use]
    pub fn is_overflowing(&self) -> bool {
        self.max_scroll() > 0.5
    }

    /// 壳层融合态顶部留白；标准态恒为 0。
    fn shell_top_inset(&self) -> f32 {
        match self.style {
            TabViewStyle::Standard => 0.0,
            TabViewStyle::ShellChrome => {
                (self.rect.size.y - TAB_SHELL_ITEM_HEIGHT).clamp(0.0, TAB_SHELL_TOP_INSET)
            }
        }
    }

    /// 给定滚动偏移下单个标签页的未裁剪矩形。
    #[must_use]
    pub fn raw_tab_rect(&self, index: usize, scroll: f32) -> Rect {
        let w = self.effective_tab_width();
        let top_inset = self.shell_top_inset();
        Rect::new(
            self.rect.origin.x + index as f32 * w - scroll,
            self.rect.origin.y + top_inset,
            w,
            (self.rect.size.y - top_inset).max(0.0),
        )
    }

    /// 裁剪至标签页栏范围内的单个标签页可见矩形。
    #[must_use]
    pub fn tab_rect(&self, index: usize, scroll: f32) -> Rect {
        let raw = self.raw_tab_rect(index, scroll);
        let top_inset = self.shell_top_inset();
        let strip = Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + top_inset,
            self.strip_width(),
            (self.rect.size.y - top_inset).max(0.0),
        );
        raw.intersect(&strip)
    }

    /// 单个标签页的前置 16×16 图标槽矩形（仅当该标签配置了图标且宽度足够时返回）。
    #[must_use]
    pub fn icon_rect(&self, index: usize, scroll: f32) -> Option<Rect> {
        let tab = self.items.get(index)?;
        if tab.effective_icon() == WidgetIcon::None {
            return None;
        }
        let visible = self.tab_rect(index, scroll);
        if visible.size.x < 36.0 || visible.size.y < 16.0 {
            return None;
        }
        Some(Rect::new(
            visible.origin.x + 12.0,
            visible.origin.y + (visible.size.y - 16.0) * 0.5,
            16.0,
            16.0,
        ))
    }

    /// 单个标签页标题文本的排版矩形（已避让前置图标与右侧关闭按钮）。
    #[must_use]
    pub fn label_rect(&self, index: usize, scroll: f32) -> Rect {
        let visible = self.tab_rect(index, scroll);
        if visible.size.x <= 0.0 {
            return Rect::ZERO;
        }
        let left_pad = if self.icon_rect(index, scroll).is_some() {
            12.0 + 16.0 + 8.0
        } else {
            12.0
        };
        let right_edge = if let Some(close) = self.close_rect(index, scroll) {
            close.origin.x - 4.0
        } else {
            visible.max().x - 8.0
        };
        let x = visible.origin.x + left_pad;
        let w = (right_edge - x).max(0.0);
        Rect::new(x, visible.origin.y, w, visible.size.y)
    }

    /// 单个标签页的关闭按钮矩形（若有）。
    #[must_use]
    pub fn close_rect(&self, index: usize, scroll: f32) -> Option<Rect> {
        let tab = self.items.get(index)?;
        if !(self.closable && tab.closable) || tab.pinned {
            return None;
        }
        let visible = self.tab_rect(index, scroll);
        match self.style {
            TabViewStyle::Standard => {
                if visible.size.x < TAB_CLOSE * 2.0 {
                    return None;
                }
                Some(Rect::new(
                    visible.max().x - TAB_CLOSE - 6.0,
                    visible.origin.y + (visible.size.y - TAB_CLOSE) * 0.5,
                    TAB_CLOSE,
                    TAB_CLOSE,
                ))
            }
            TabViewStyle::ShellChrome => {
                if visible.size.x < TAB_CLOSE_HIT_SIZE + 20.0 {
                    return None;
                }
                let hit_size = TAB_CLOSE_HIT_SIZE.min(visible.size.y);
                Some(Rect::new(
                    visible.max().x - hit_size - 4.0,
                    visible.origin.y + (visible.size.y - hit_size) * 0.5,
                    hit_size,
                    hit_size,
                ))
            }
        }
    }

    /// 末尾的“+”按钮矩形（未滚动偏移）。
    #[must_use]
    pub fn add_rect(&self) -> Option<Rect> {
        self.add_rect_scrolled(0.0)
    }

    /// 考虑滚动偏移后的“+”按钮矩形。
    ///
    /// 在 [`TabViewStyle::ShellChrome`] 下紧随最后一个标签之后 `4px` 间距，
    /// 溢出时自动停靠于可视区右沿，不遮挡右侧窗口控制按钮。
    #[must_use]
    pub fn add_rect_scrolled(&self, scroll: f32) -> Option<Rect> {
        self.addable.then(|| match self.style {
            TabViewStyle::Standard => Rect::new(
                self.rect.max().x - TAB_STRIP_HEIGHT,
                self.rect.origin.y,
                TAB_STRIP_HEIGHT,
                self.rect.size.y,
            ),
            TabViewStyle::ShellChrome => {
                let top_inset = self.shell_top_inset();
                let card_h = (self.rect.size.y - top_inset).max(0.0);
                let btn_h = TAB_ADD_HEIGHT.min(card_h);
                let flow_x = self.rect.origin.x + self.count() as f32 * self.effective_tab_width()
                    - scroll
                    + TAB_ADD_GAP;
                let max_x = (self.rect.max().x - TAB_ADD_WIDTH).max(self.rect.origin.x);
                let x = flow_x.min(max_x).max(self.rect.origin.x);
                let y = self.rect.origin.y + top_inset + (card_h - btn_h) * 0.5;
                Rect::new(x, y, TAB_ADD_WIDTH, btn_h)
            }
        })
    }

    /// 命中测试：判断 `pos` 是否落在某个标签页的关闭按钮内。
    #[must_use]
    pub fn hit_test_close(&self, scroll: f32, pos: Vec2) -> Option<usize> {
        for index in 0..self.count() {
            if let Some(close) = self.close_rect(index, scroll) {
                if close.contains_point(pos) {
                    return Some(index);
                }
            }
        }
        None
    }

    /// 命中测试：判断 `pos` 是否落在“+”新增按钮内。
    #[must_use]
    pub fn hit_test_add(&self, scroll: f32, pos: Vec2) -> bool {
        self.add_rect_scrolled(scroll)
            .is_some_and(|rect| rect.contains_point(pos))
    }

    /// 命中测试：判断 `pos` 是否落在某个可见标签页内。
    #[must_use]
    pub fn hit_test_tab(&self, scroll: f32, pos: Vec2) -> Option<usize> {
        for index in 0..self.count() {
            let rect = self.tab_rect(index, scroll);
            if rect.size.x > 0.0 && rect.size.y > 0.0 && rect.contains_point(pos) {
                return Some(index);
            }
        }
        None
    }

    /// 处理左键点击：优先检测独立关闭按钮与“+”按钮，避免误触标签选中。
    pub fn handle_left_click(&self, state: &mut TabsState, pos: Vec2) -> Option<TabViewEvent> {
        if let Some(index) = self.hit_test_close(state.scroll_x, pos) {
            let id = self.items[index].id;
            return Some(TabViewEvent::CloseRequested { id, index });
        }
        if self.hit_test_add(state.scroll_x, pos) {
            return Some(TabViewEvent::AddClicked);
        }
        if let Some(index) = self.hit_test_tab(state.scroll_x, pos) {
            let id = self.items[index].id;
            state.activate(self, index);
            return Some(TabViewEvent::Selected { id, index });
        }
        None
    }

    /// 处理鼠标中键点击：命中标签时发射 [`TabViewEvent::MiddleClicked`]。
    #[must_use]
    pub fn handle_middle_click(&self, state: &TabsState, pos: Vec2) -> Option<TabViewEvent> {
        let index = self.hit_test_tab(state.scroll_x, pos)?;
        let id = self.items[index].id;
        Some(TabViewEvent::MiddleClicked { id, index })
    }

    /// 处理鼠标右键点击：命中标签时发射 [`TabViewEvent::ContextMenuRequested`]。
    #[must_use]
    pub fn handle_right_click(&self, state: &TabsState, pos: Vec2) -> Option<TabViewEvent> {
        let index = self.hit_test_tab(state.scroll_x, pos)?;
        let id = self.items[index].id;
        Some(TabViewEvent::ContextMenuRequested {
            id,
            index,
            screen_pos: [pos.x.round() as i32, pos.y.round() as i32],
        })
    }

    /// 判断给定局部逻辑坐标是否满足标签离窗撕裂条件。
    ///
    /// 当指针离开标签栏垂直/水平阈值 [`TabsState::tear_off_threshold`]，
    /// 或超出宿主窗口逻辑边界 `window_bounds_logical` 时返回 `true`。
    #[must_use]
    pub fn should_tear_off(
        &self,
        pos_logical: Vec2,
        window_bounds_logical: Option<Rect>,
        threshold: f32,
    ) -> bool {
        let t = threshold.max(8.0);
        let outside_strip = pos_logical.y < self.rect.origin.y - t
            || pos_logical.y > self.rect.max().y + t
            || pos_logical.x < self.rect.origin.x - t
            || pos_logical.x > self.rect.max().x + t;
        let outside_window =
            window_bounds_logical.is_some_and(|wb| !wb.contains_point(pos_logical));
        outside_strip || outside_window
    }

    /// 处理左键按下：若命中标签主体则启动拖拽/撕裂状态机并激活该标签。
    pub fn handle_pointer_press(&self, state: &mut TabsState, pos: Vec2) -> Option<TabViewEvent> {
        if let Some(index) = self.hit_test_close(state.scroll_x, pos) {
            let id = self.items[index].id;
            return Some(TabViewEvent::CloseRequested { id, index });
        }
        if self.hit_test_add(state.scroll_x, pos) {
            return Some(TabViewEvent::AddClicked);
        }
        if let Some(index) = self.hit_test_tab(state.scroll_x, pos) {
            let id = self.items[index].id;
            state.activate(self, index);
            state.dragging = Some(TabDragSession {
                id,
                origin_index: index,
                current_index: index,
                press_pos: pos,
                current_pos: pos,
                detached: false,
            });
            return Some(TabViewEvent::Selected { id, index });
        }
        None
    }

    /// 处理拖拽移动：更新离窗判定标志，并在标签栏内按水平中心线触发重排事件。
    pub fn handle_pointer_move(
        &self,
        state: &mut TabsState,
        pos: Vec2,
        window_bounds_logical: Option<Rect>,
    ) -> Option<TabViewEvent> {
        let mut session = state.dragging?;
        session.current_pos = pos;
        session.detached =
            self.should_tear_off(pos, window_bounds_logical, state.tear_off_threshold);
        if !session.detached && (pos.x - session.press_pos.x).abs() > 4.0 && self.count() > 1 {
            let tab_w = self.effective_tab_width().max(1.0);
            let rel_x = (pos.x - self.rect.origin.x + state.scroll_x).max(0.0);
            let target = (rel_x / tab_w).floor() as usize;
            let clamped = target.min(self.count() - 1);
            if clamped != session.current_index {
                let from_index = session.current_index;
                session.current_index = clamped;
                state.active = clamped;
                state.dragging = Some(session);
                return Some(TabViewEvent::Reordered {
                    id: session.id,
                    from_index,
                    to_index: clamped,
                });
            }
        }
        state.dragging = Some(session);
        None
    }

    /// 处理左键释放：若超出离窗阈值或窗口边界，按多屏 DPI 换算屏幕物理坐标并发射 [`TabViewEvent::TearOffRequested`]。
    pub fn handle_pointer_release(
        &self,
        state: &mut TabsState,
        pos_logical: Vec2,
        window_origin_physical: (i32, i32),
        window_size_physical: Option<(u32, u32)>,
        scale_factor: f32,
    ) -> Option<TabViewEvent> {
        let session = state.dragging.take()?;
        let scale = scale_factor.max(0.5);
        let window_bounds_logical = window_size_physical
            .map(|(w, h)| Rect::new(0.0, 0.0, w as f32 / scale, h as f32 / scale));
        let tear_off = session.detached
            || self.should_tear_off(pos_logical, window_bounds_logical, state.tear_off_threshold);
        if tear_off {
            let screen_x = window_origin_physical.0 + (pos_logical.x * scale).round() as i32;
            let screen_y = window_origin_physical.1 + (pos_logical.y * scale).round() as i32;
            return Some(TabViewEvent::TearOffRequested {
                id: session.id,
                index: session.current_index,
                screen_pos: [screen_x, screen_y],
            });
        }
        None
    }
}

/// 标签页栏的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct TabsState {
    /// 当前选中的标签页索引。
    pub active: usize,
    /// 指示器左边缘的动画位置。
    pub indicator_x: Spring,
    /// 指示器宽度的动画值。
    pub indicator_w: Spring,
    /// 标签页栏的水平滚动偏移。
    pub scroll_x: f32,
    /// 正在进行的标签页拖拽与离窗撕裂会话。
    pub dragging: Option<TabDragSession>,
    /// 触发离窗撕裂的逻辑像素阈值（默认 [`TAB_TEAR_OFF_THRESHOLD`]）。
    pub tear_off_threshold: f32,
    /// 弹簧是否已定位到静止目标。
    initialised: bool,
}

impl Default for TabsState {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TabsState {
    /// 创建选中指定标签页的状态。
    #[must_use]
    pub fn new(active: usize) -> Self {
        Self {
            active,
            indicator_x: Spring::new(unbound_anim::SpringPreset::Precise, 0.0),
            indicator_w: Spring::new(unbound_anim::SpringPreset::Precise, 0.0),
            scroll_x: 0.0,
            dragging: None,
            tear_off_threshold: TAB_TEAR_OFF_THRESHOLD,
            initialised: false,
        }
    }

    /// 夹紧到当前标签页数量的激活索引。
    #[must_use]
    pub fn active_index(&self, desc: &TabsDesc) -> usize {
        self.active.min(desc.count().saturating_sub(1))
    }

    /// 激活标签页对应的静止指示器几何信息。
    #[must_use]
    pub fn indicator_target(&self, desc: &TabsDesc) -> (f32, f32) {
        let index = self.active_index(desc);
        let rect = desc.tab_rect(index, self.scroll_x);
        (rect.origin.x, rect.size.x)
    }

    /// 无动画直接将指示器放置到静止位置。
    pub fn snap(&mut self, desc: &TabsDesc) {
        let (x, width) = self.indicator_target(desc);
        self.indicator_x.snap_to(x);
        self.indicator_w.snap_to(width);
        self.initialised = true;
    }

    /// 将弹簧目标对准当前激活标签页。
    ///
    /// 每帧在 [`Self::advance`] 之前调用一次；调用方读取
    /// [`Self::is_animating`] 以决定是否需要下一帧。
    pub fn sync(&mut self, desc: &TabsDesc) {
        if !self.initialised {
            self.snap(desc);
            return;
        }
        let (x, width) = self.indicator_target(desc);
        self.indicator_x.set_target(x);
        self.indicator_w.set_target(width);
    }

    /// 推进弹簧物理步进 `dt` 秒，返回是否仍在运动。
    pub fn advance(&mut self, dt: f32) -> bool {
        let moving_x = self.indicator_x.step(dt);
        let moving_w = self.indicator_w.step(dt);
        moving_x || moving_w
    }

    /// 指示器是否仍在运动。
    #[must_use]
    pub fn is_animating(&self) -> bool {
        !self.indicator_x.is_settled() || !self.indicator_w.is_settled()
    }

    /// 动画中的指示器矩形。
    #[must_use]
    pub fn indicator_rect(&self, desc: &TabsDesc) -> Rect {
        Rect::new(
            self.indicator_x.value,
            desc.rect.max().y - 2.0,
            self.indicator_w.value.max(0.0),
            2.0,
        )
    }

    /// 激活标签页并滚动使其可见，返回选中项是否变化。
    pub fn activate(&mut self, desc: &TabsDesc, index: usize) -> bool {
        if index >= desc.count() || index == self.active_index(desc) {
            return false;
        }
        self.active = index;
        self.reveal(desc, index);
        true
    }

    /// 滚动使指定标签页完全可见。
    pub fn reveal(&mut self, desc: &TabsDesc, index: usize) {
        let tab_w = desc.effective_tab_width();
        let left = index as f32 * tab_w;
        let right = left + tab_w;
        let width = desc.strip_width();
        if left < self.scroll_x {
            self.scroll_x = left;
        } else if right > self.scroll_x + width {
            self.scroll_x = right - width;
        }
        self.scroll_x = self.scroll_x.clamp(0.0, desc.max_scroll());
    }

    /// 按滚轮增量滚动。
    pub fn scroll_by(&mut self, desc: &TabsDesc, delta: f32) -> bool {
        let next = (self.scroll_x + delta).clamp(0.0, desc.max_scroll());
        let changed = (next - self.scroll_x).abs() > 1e-4;
        self.scroll_x = next;
        changed
    }
}

/// 聚焦后的标签页栏可识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsKey {
    /// 选择上一个标签页（循环）。
    Left,
    /// 选择下一个标签页（循环）。
    Right,
    /// 选择第一个标签页。
    Home,
    /// 选择最后一个标签页。
    End,
}

/// 将单次按键归约为下一个激活索引。
#[must_use]
pub fn reduce_tabs_key(desc: &TabsDesc, active: usize, key: TabsKey) -> usize {
    let count = desc.count();
    if count == 0 {
        return 0;
    }
    match key {
        TabsKey::Left => (active + count - 1) % count,
        TabsKey::Right => (active + 1) % count,
        TabsKey::Home => 0,
        TabsKey::End => count - 1,
    }
}

/// 标签页栏绘制所用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabsTokens {
    /// 标签页栏背景。
    pub strip: Token,
    /// 标签页栏细分隔线。
    pub border: Token,
    /// 空闲标签页。
    pub tab: Token,
    /// 悬停标签页。
    pub tab_hover: Token,
    /// 激活标签页。
    pub tab_active: Token,
    /// 空闲标签页文本。
    pub tab_text: Token,
    /// 激活标签页文本。
    pub tab_text_active: Token,
    /// 选中指示器。
    pub indicator: Token,
    /// 关闭图标。
    pub close: Token,
    /// 悬停状态的关闭图标。
    pub close_hover: Token,
    /// 键盘焦点环。
    pub focus_ring: Token,
}

impl TabsTokens {
    /// 标准配色集。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            // `TabViewBackground` 透明，这里用页面底色 `SolidBackgroundFillColorBase` 承接。
            strip: Token::SURFACE,
            // `TabViewBorderBrush` = CardStrokeColorDefault。
            border: Token::CARD_BORDER,
            tab: Token::NONE,
            // `TabViewItemHeaderBackgroundPointerOver` = LayerOnMicaBaseAltFillColorSecondary。
            tab_hover: Token::NAV_HOVER,
            // `TabViewItemHeaderBackgroundSelected` = SolidBackgroundFillColorTertiary。
            tab_active: Token::SURFACE_RAISED,
            tab_text: Token::TEXT_SECONDARY,
            tab_text_active: Token::TEXT_PRIMARY,
            indicator: Token::PRIMARY,
            // `TabViewItemHeaderCloseButtonForeground` = TextFillColorPrimary。
            close: Token::TEXT_PRIMARY,
            close_hover: Token::TEXT_PRIMARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 输出标签页栏背景、细分隔线与动画指示器。
#[must_use]
pub fn emit_tabs_strip(desc: &TabsDesc, state: &TabsState, tokens: TabsTokens) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3);
    match desc.style {
        TabViewStyle::Standard => {
            primitives.push(surface(desc.rect, 0.0, tokens.strip, desc.z));
            let line = Rect::new(
                desc.rect.origin.x,
                desc.rect.max().y - 1.0,
                desc.rect.size.x,
                1.0,
            );
            primitives.push(instance(
                line,
                [0.0; 4],
                tokens.border,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            ));
            if desc.count() > 0 {
                primitives.push(surface(
                    state.indicator_rect(desc),
                    1.0,
                    tokens.indicator,
                    desc.z,
                ));
            }
        }
        TabViewStyle::ShellChrome => {
            // 壳层融合态底板透明以透出 TitleBarChrome 的 HEADER_BG。
            primitives.push(surface(desc.rect, 0.0, Token::NONE, desc.z));
            // 底部 1px 分隔线绕过当前选中标签底边，使选中卡片与下方工具栏无缝融合。
            let y = desc.rect.max().y - 1.0;
            let (left_w, right_x, right_w) = if desc.count() > 0 {
                let active_r = desc.tab_rect(state.active_index(desc), state.scroll_x);
                let left_w = (active_r.origin.x - desc.rect.origin.x).max(0.0);
                let right_x = active_r.max().x.min(desc.rect.max().x);
                let right_w = (desc.rect.max().x - right_x).max(0.0);
                (left_w, right_x, right_w)
            } else {
                (desc.rect.size.x, desc.rect.max().x, 0.0)
            };
            let mut left_line = instance(
                Rect::new(desc.rect.origin.x, y, left_w, 1.0),
                [0.0; 4],
                tokens.border,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            );
            left_line.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(desc.z, if left_w > 0.0 { 1.0 } else { 0.0 });
            primitives.push(left_line);

            let mut right_line = instance(
                Rect::new(right_x, y, right_w, 1.0),
                [0.0; 4],
                tokens.border,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            );
            right_line.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(desc.z, if right_w > 0.0 { 1.0 } else { 0.0 });
            primitives.push(right_line);
        }
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 输出单个标签页。
#[must_use]
pub fn emit_tab(
    desc: &TabsDesc,
    state: &TabsState,
    index: usize,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: TabsTokens,
) -> WidgetOutput {
    let Some(item) = desc.items.get(index) else {
        return WidgetOutput::default();
    };
    let rect = desc.tab_rect(index, state.scroll_x);
    if rect.size.x <= 0.0 {
        return WidgetOutput::default();
    }
    let active = index == state.active_index(desc);
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(8 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    let text_token = if disabled {
        Token::TEXT_DISABLED
    } else if active {
        tokens.tab_text_active
    } else {
        tokens.tab_text
    };

    let hit_radii = match desc.style {
        TabViewStyle::Standard => {
            let fill = if active {
                tokens.tab_active
            } else if control.phase == PointerPhase::Hovered {
                tokens.tab_hover
            } else {
                tokens.tab
            };
            primitives.push(outlined(
                rect,
                TAB_RADIUS,
                fill,
                Token::NONE,
                0.0,
                0.0,
                desc.z,
            ));
            [TAB_RADIUS; 4]
        }
        TabViewStyle::ShellChrome => {
            let radii = if active {
                [TAB_SHELL_CORNER_RADIUS, TAB_SHELL_CORNER_RADIUS, 0.0, 0.0]
            } else {
                [TAB_RADIUS; 4]
            };
            let fill = if active {
                tokens.tab_active
            } else if control.phase == PointerPhase::Pressed {
                Token::PRESSED
            } else if control.phase == PointerPhase::Hovered {
                tokens.tab_hover
            } else {
                tokens.tab
            };
            // 1. 标签卡片底板（选中时为顶圆角 8px + 底直角 0px，填充 SURFACE_RAISED）。
            primitives.push(instance(
                rect,
                radii,
                fill,
                Token::NONE,
                0.0,
                0.0,
                PrimType::RoundedRect,
                SubElement::Background,
                desc.z,
            ));
            // 2~4. 选中态顶/左/右 1px CARD_BORDER 描边（无底边），非选中态透明度 0 保持图元数恒定。
            let border_opacity = if active && desc.shell_active_border {
                1.0
            } else {
                0.0
            };
            let top_inset = TAB_SHELL_CORNER_RADIUS.min(rect.size.x * 0.5);
            let mut top_border = instance(
                Rect::new(
                    rect.origin.x + top_inset * 0.5,
                    rect.origin.y,
                    (rect.size.x - top_inset).max(0.0),
                    1.0,
                ),
                [0.0; 4],
                tokens.border,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            );
            top_border.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(desc.z, border_opacity);
            primitives.push(top_border);

            let side_h = (rect.size.y - top_inset * 0.5).max(0.0);
            let mut left_border = instance(
                Rect::new(rect.origin.x, rect.origin.y + top_inset * 0.5, 1.0, side_h),
                [0.0; 4],
                tokens.border,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            );
            left_border.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(desc.z, border_opacity);
            primitives.push(left_border);

            let mut right_border = instance(
                Rect::new(
                    (rect.max().x - 1.0).max(rect.origin.x),
                    rect.origin.y + top_inset * 0.5,
                    1.0,
                    side_h,
                ),
                [0.0; 4],
                tokens.border,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            );
            right_border.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(desc.z, border_opacity);
            primitives.push(right_border);

            // 5. 相邻非选中标签之间的 1px × 16px DIVIDER 竖线（紧邻选中或悬停标签时隐藏）。
            let active_idx = state.active_index(desc);
            let show_divider = !active
                && control.phase == PointerPhase::Idle
                && index + 1 < desc.count()
                && index + 1 != active_idx;
            let div_h = 16.0f32.min(rect.size.y);
            let mut divider = instance(
                Rect::new(
                    (rect.max().x - 1.0).max(rect.origin.x),
                    rect.origin.y + (rect.size.y - div_h) * 0.5,
                    1.0,
                    div_h,
                ),
                [0.0; 4],
                Token::DIVIDER,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Background,
                desc.z,
            );
            divider.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(desc.z, if show_divider { 1.0 } else { 0.0 });
            primitives.push(divider);

            radii
        }
    };

    let effective_icon = item.effective_icon();
    if let Some(icon_r) = desc.icon_rect(index, state.scroll_x) {
        emit_widget_icon_primitives(
            effective_icon,
            icon_r,
            text_token,
            if active {
                tokens.tab_active
            } else {
                Token::HEADER_BG
            },
            disabled,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
    }

    glyphs(label, text_token, desc.z, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect.inflate(1.0),
        2.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(item.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: hit_radii })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Tab, item.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = active;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 输出单个标签页的关闭按钮。
///
/// 关闭按钮拥有独立节点，使点击“×”不会被误判为点击标签页，
/// 且辅助技术可以单独播报。
#[must_use]
pub fn emit_tab_close(
    node: u64,
    desc: &TabsDesc,
    state: &TabsState,
    index: usize,
    control: ControlState,
    tokens: TabsTokens,
) -> WidgetOutput {
    let Some(rect) = desc.close_rect(index, state.scroll_x) else {
        return WidgetOutput::default();
    };
    let disabled = control.disabled;
    let color = if disabled {
        Token::TEXT_DISABLED
    } else if control.phase == PointerPhase::Hovered {
        tokens.close_hover
    } else {
        tokens.close
    };
    let mut primitives = Vec::with_capacity(4);
    let mut payloads = Vec::with_capacity(2);
    let close_z = match desc.style {
        TabViewStyle::Standard => desc.z,
        TabViewStyle::ShellChrome => desc.z.saturating_add(1),
    };

    if desc.style == TabViewStyle::ShellChrome {
        let bg = if disabled {
            Token::NONE
        } else if control.phase == PointerPhase::Pressed {
            Token::PRESSED
        } else if control.phase == PointerPhase::Hovered {
            Token::HOVER
        } else {
            Token::NONE
        };
        primitives.push(outlined(
            rect,
            TAB_RADIUS,
            bg,
            Token::NONE,
            0.0,
            0.0,
            close_z,
        ));
    }

    let glyph_rect = match desc.style {
        TabViewStyle::Standard => rect,
        TabViewStyle::ShellChrome => {
            let icon_size = TAB_CLOSE.min(rect.size.x).min(rect.size.y);
            Rect::new(
                rect.origin.x + (rect.size.x - icon_size) * 0.5,
                rect.origin.y + (rect.size.y - icon_size) * 0.5,
                icon_size,
                icon_size,
            )
        }
    };
    // Win11 标准 `12×12` 关闭叉号图标盒内对角线跨度取 `8.0px`（落在 `8.0..=9.0px` 区间），
    // 兼顾 100% 与高 DPI 下的锐利度。
    let span = 8.0f32.min(glyph_rect.size.x - 2.0).max(4.0);
    let inset = (glyph_rect.size.x - span) * 0.5;
    let thickness = 1.4;
    for (start, end) in [
        (
            [glyph_rect.origin.x + inset, glyph_rect.origin.y + inset],
            [glyph_rect.max().x - inset, glyph_rect.max().y - inset],
        ),
        (
            [glyph_rect.max().x - inset, glyph_rect.origin.y + inset],
            [glyph_rect.origin.x + inset, glyph_rect.max().y - inset],
        ),
    ] {
        curve_stroke(
            start,
            end,
            thickness,
            color,
            close_z,
            &mut primitives,
            &mut payloads,
        );
    }
    primitives.push(focus_ring(
        rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        1.5,
        close_z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(close_z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let title = desc
        .items
        .get(index)
        .map(|item| format!("Close {}", item.title))
        .unwrap_or_else(|| "Close".to_owned());
    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(
            A11yNode::new(Role::Button, title, rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 输出末尾的“+”按钮。
#[must_use]
pub fn emit_tabs_add(
    node: u64,
    desc: &TabsDesc,
    control: ControlState,
    tokens: TabsTokens,
) -> WidgetOutput {
    let Some(rect) = desc.add_rect() else {
        return WidgetOutput::default();
    };
    let disabled = control.disabled;
    let (fill, radius) = match desc.style {
        TabViewStyle::Standard => (
            if control.phase == PointerPhase::Hovered {
                tokens.tab_hover
            } else {
                tokens.tab
            },
            0.0,
        ),
        TabViewStyle::ShellChrome => (
            if disabled {
                Token::NONE
            } else if control.phase == PointerPhase::Pressed {
                Token::PRESSED
            } else if control.phase == PointerPhase::Hovered {
                Token::HOVER
            } else {
                Token::NONE
            },
            TAB_RADIUS,
        ),
    };
    let color = if disabled {
        Token::TEXT_DISABLED
    } else {
        tokens.tab_text
    };
    let mut primitives = Vec::with_capacity(4);
    primitives.push(outlined(rect, radius, fill, Token::NONE, 0.0, 0.0, desc.z));
    let centre = rect.size.x * 0.5;
    let arm = match desc.style {
        TabViewStyle::Standard => rect.size.x * 0.18,
        TabViewStyle::ShellChrome => 5.0,
    };
    let plus_thickness = match desc.style {
        TabViewStyle::Standard => 1.5,
        TabViewStyle::ShellChrome => 1.0,
    };
    let cx = rect.origin.x + centre;
    let cy = rect.origin.y + rect.size.y * 0.5;
    primitives.push(stroke_between(
        [cx - arm, cy],
        [cx + arm, cy],
        plus_thickness,
        color,
        desc.z,
    ));
    primitives.push(stroke_between(
        [cx, cy - arm],
        [cx, cy + arm],
        plus_thickness,
        color,
        desc.z,
    ));
    primitives.push(focus_ring(
        rect.inflate(1.0),
        4.0,
        tokens.focus_ring,
        1.5,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit,
        a11y: Some(
            A11yNode::new(Role::Button, "New tab", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

/// 使用调用方提供的独立子部件 [`ControlState`] 解析函数发射完整标签页栏。
///
/// 合并各标签页、关闭按钮与新建按钮的输出时统一通过 [`WidgetOutput::append_child_primitives`]
/// 重定位 `extra_payload_idx`，并为每个可见标签页、关闭按钮（更高 `z` 序）与新建按钮注册独立
/// `sub_hits` / `sub_a11y` / `sub_focus_order`。
#[must_use]
pub fn emit_tabs_with_control_states(
    node: u64,
    desc: &TabsDesc,
    state: &TabsState,
    text_ctx: &mut crate::ergonomics::TextCtx<'_>,
    mut resolve_tab_control: impl FnMut(usize) -> ControlState,
    mut resolve_close_control: impl FnMut(usize) -> ControlState,
    add_control: ControlState,
) -> WidgetOutput {
    let mut st = state.clone();
    st.snap(desc);
    let tokens = TabsTokens::standard();
    let mut out = emit_tabs_strip(desc, &st, tokens);
    let align_x = match desc.style {
        TabViewStyle::Standard => crate::ergonomics::AlignX::Center,
        TabViewStyle::ShellChrome => crate::ergonomics::AlignX::Left,
    };
    let active_idx = st.active_index(desc);
    let base_z = u32::from(desc.z);
    let close_hit_z = base_z.saturating_add(1);

    for idx in 0..desc.count() {
        let tab_r = desc.tab_rect(idx, st.scroll_x);
        if tab_r.width() <= 0.0 {
            continue;
        }
        let label_r = desc.label_rect(idx, st.scroll_x);
        let title = &desc.items[idx].title;
        let glyphs = if label_r.width() > 0.0 {
            text_ctx.shape_line(
                title,
                label_r,
                unbound_theme::typography::font_size::BODY,
                if idx == active_idx { 600 } else { 400 },
                align_x,
                crate::ergonomics::AlignY::Middle,
            )
        } else {
            Vec::new()
        };
        let item_key = tab_item_node(node, idx);
        let tab_ctrl = resolve_tab_control(idx);
        let tab_out = emit_tab(desc, &st, idx, tab_ctrl, &glyphs, tokens);
        if let (Some(mut hit), Some(a11y)) = (tab_out.hit.clone(), tab_out.a11y.clone()) {
            hit.node = item_key;
            hit.z = base_z;
            out.with_sub_control(item_key, hit, a11y, tab_out.focusable);
        }
        out.append_child_primitives(tab_out);

        if desc.close_rect(idx, st.scroll_x).is_some() {
            let close_key = tab_close_node(node, idx);
            let close_ctrl = resolve_close_control(idx);
            let close_out = emit_tab_close(close_key, desc, &st, idx, close_ctrl, tokens);
            if let (Some(mut close_hit), Some(close_a11y)) =
                (close_out.hit.clone(), close_out.a11y.clone())
            {
                close_hit.z = close_hit_z;
                out.with_sub_control(close_key, close_hit, close_a11y, close_out.focusable);
            }
            out.append_child_primitives(close_out);
        }
    }

    if desc.addable {
        let add_key = tab_add_node(node);
        let add_out = emit_tabs_add(add_key, desc, add_control, tokens);
        if let (Some(mut add_hit), Some(add_a11y)) = (add_out.hit.clone(), add_out.a11y.clone()) {
            add_hit.z = base_z;
            out.with_sub_control(add_key, add_hit, add_a11y, add_out.focusable);
        }
        out.append_child_primitives(add_out);
    }

    // 顶层标签栏兜底热区置于子热区之下（`z - 1`），避免抢占子标签、关闭按钮或新建按钮的命中。
    out.hit = Some(
        HitRegion::new(node, CONTROL_LAYER, base_z.saturating_sub(1), desc.rect)
            .with_cursor(CursorKind::Pointer),
    );
    out.a11y = Some(A11yNode::new(Role::TabList, "Tabs", desc.rect));
    out
}

/// 两点之间的细描边，打包为线段图元。
fn stroke_between(
    start: [f32; 2],
    end: [f32; 2],
    thickness: f32,
    color: Token,
    z: u16,
) -> GpuPrimitiveInstance {
    let min_x = start[0].min(end[0]);
    let min_y = start[1].min(end[1]);
    let width = (start[0] - end[0]).abs().max(thickness);
    let height = (start[1] - end[1]).abs().max(thickness);
    instance(
        Rect::new(min_x, min_y, width, height),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Content,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TabsDesc {
        TabsDesc::new(Rect::new(0.0, 0.0, 300.0, 36.0), &["One", "Two", "Three"])
    }

    fn shell_desc(width: f32, count: usize) -> TabsDesc {
        let items: Vec<TabItem> = (0..count)
            .map(|i| {
                TabItem::new(i as u64 + 1, format!("Tab {}", i + 1))
                    .with_icon(ShellIcon::Folder)
                    .with_tooltip(format!("Path {}", i + 1))
            })
            .collect();
        TabsDesc::new(Rect::new(0.0, 0.0, width, TAB_STRIP_HEIGHT), &[])
            .with_items(items)
            .with_addable(true)
            .with_style(TabViewStyle::ShellChrome)
            .with_width_mode(TabWidthMode::adaptive_default())
    }

    #[test]
    fn shell_chrome_active_tab_is_borderless_by_default_and_add_plus_is_1px() {
        let desc = shell_desc(720.0, 2);
        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();

        // 默认 ShellChrome 激活标签页无外描边（三段边线透明度为 0）
        let active_out = emit_tab(&desc, &state, 0, ControlState::default(), &[], tokens);
        for border_prim in &active_out.primitives[1..=3] {
            assert_eq!(
                border_prim.opacity(),
                0.0,
                "active ShellChrome tab must be borderless by default"
            );
        }

        // 显式启用 shell_active_border(true) 时恢复 1px 外描边，且图元数恒定
        let bordered_desc = desc.clone().with_shell_active_border(true);
        let bordered_out = emit_tab(
            &bordered_desc,
            &state,
            0,
            ControlState::default(),
            &[],
            tokens,
        );
        assert_eq!(active_out.primitives.len(), bordered_out.primitives.len());
        for border_prim in &bordered_out.primitives[1..=3] {
            assert!(
                (border_prim.opacity() - 1.0).abs() < 1e-4,
                "bordered ShellChrome tab must render 1px border when opted in"
            );
        }

        // ShellChrome 新建 `+` 按钮采用 1px 纤细线条风格
        let add_out = emit_tabs_add(99, &desc, ControlState::default(), tokens);
        let h_stroke = add_out.primitives[1];
        let v_stroke = add_out.primitives[2];
        assert!(
            (h_stroke.size[1] - 1.0).abs() < 1e-4,
            "horizontal plus stroke must be 1px thick, got {}",
            h_stroke.size[1]
        );
        assert!(
            (v_stroke.size[0] - 1.0).abs() < 1e-4,
            "vertical plus stroke must be 1px thick, got {}",
            v_stroke.size[0]
        );
    }

    #[test]
    fn tabs_are_laid_out_left_to_right() {
        let desc = desc();
        assert_eq!(desc.tab_rect(0, 0.0).origin.x, 0.0);
        assert_eq!(desc.tab_rect(1, 0.0).origin.x, TAB_WIDTH);
        assert_eq!(desc.tab_rect(2, 0.0).origin.x, TAB_WIDTH * 2.0);
    }

    #[test]
    fn a_tab_past_the_strip_is_clipped_away() {
        let desc = desc();
        // 300 px 宽，140 px 标签页：标签页 2 从 280 开始，可见 20 px。
        let third = desc.tab_rect(2, 0.0);
        assert_eq!(third.origin.x, 280.0);
        assert_eq!(third.size.x, 20.0);
    }

    #[test]
    fn scrolling_reveals_the_last_tab() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        assert_eq!(desc.max_scroll(), 120.0);
        assert!(state.scroll_by(&desc, 500.0));
        assert_eq!(state.scroll_x, 120.0);
        let third = desc.tab_rect(2, state.scroll_x);
        assert_eq!(third.origin.x, 160.0);
        assert_eq!(third.size.x, 140.0);
    }

    #[test]
    fn activating_a_tab_reveals_it() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        assert!(state.activate(&desc, 2));
        assert_eq!(state.scroll_x, 120.0);
        assert!(!state.activate(&desc, 2));
    }

    #[test]
    fn the_indicator_snaps_and_then_animates() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        let rest = state.indicator_rect(&desc);
        assert_eq!(rest.origin.x, 0.0);
        assert_eq!(rest.size.x, TAB_WIDTH);

        state.activate(&desc, 1);
        state.sync(&desc);
        assert!(state.is_animating());
        for _ in 0..120 {
            state.advance(1.0 / 60.0);
        }
        let moved = state.indicator_rect(&desc);
        assert!((moved.origin.x - TAB_WIDTH).abs() < 1.0);
        assert!((moved.size.x - TAB_WIDTH).abs() < 1.0);
    }

    #[test]
    fn the_indicator_clamps_when_the_active_tab_leaves_the_strip() {
        let desc = desc();
        let mut state = TabsState::new(2);
        state.snap(&desc);
        let indicator = state.indicator_rect(&desc);
        assert_eq!(indicator.origin.x, 280.0);
        assert_eq!(indicator.size.x, 20.0);
    }

    #[test]
    fn the_close_affordance_only_exists_when_the_tab_is_wide_enough() {
        let desc = desc();
        assert!(desc.close_rect(0, 0.0).is_some());
        assert!(desc.close_rect(2, 0.0).is_none(), "20 px is too narrow");
        let wide = desc.clone().with_tab_width(80.0);
        assert!(wide.close_rect(2, 0.0).is_some());
    }

    #[test]
    fn the_add_button_sits_at_the_trailing_edge() {
        let desc = desc().with_addable(true);
        let add = desc.add_rect().expect("addable");
        assert_eq!(add.origin.x, 300.0 - TAB_STRIP_HEIGHT);
        assert_eq!(desc.strip_width(), 300.0 - TAB_STRIP_HEIGHT);
    }

    #[test]
    fn arrow_keys_wrap_and_home_end_jump() {
        let desc = desc();
        assert_eq!(reduce_tabs_key(&desc, 0, TabsKey::Left), 2);
        assert_eq!(reduce_tabs_key(&desc, 2, TabsKey::Right), 0);
        assert_eq!(reduce_tabs_key(&desc, 1, TabsKey::Home), 0);
        assert_eq!(reduce_tabs_key(&desc, 1, TabsKey::End), 2);
    }

    #[test]
    fn a_tab_is_pickable_and_the_active_tab_is_selected() {
        let desc = desc();
        let state = TabsState::new(1);
        let tokens = TabsTokens::standard();
        let output = emit_tab(&desc, &state, 1, ControlState::default(), &[], tokens);
        let hit = output.hit.expect("a tab is pickable");
        assert_eq!(hit.node, 2);
        let a11y = output.a11y.expect("a tab is reachable");
        assert_eq!(a11y.role, Role::Tab);
        assert!(a11y.selected);
        assert_eq!(a11y.label, "Two");
    }

    #[test]
    fn a_hovered_tab_paints_the_hover_fill() {
        let desc = desc();
        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();
        let output = emit_tab(
            &desc,
            &state,
            1,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.tab_hover.raw() as u32
        );
    }

    #[test]
    fn the_close_button_is_its_own_control() {
        let desc = desc();
        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();
        let output = emit_tab_close(77, &desc, &state, 0, ControlState::default(), tokens);
        let hit = output.hit.expect("the close button is pickable");
        assert_eq!(hit.node, 77);
        assert_eq!(
            output.payloads.len(),
            2,
            "the close icon's two curve strokes must retain their GPU payloads"
        );
        let a11y = output.a11y.expect("the close button is reachable");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(a11y.label, "Close One");
    }

    #[test]
    fn the_strip_paints_the_indicator() {
        let desc = desc();
        let mut state = TabsState::new(0);
        state.snap(&desc);
        let output = emit_tabs_strip(&desc, &state, TabsTokens::standard());
        assert_eq!(output.primitives.len(), 3);
    }

    #[test]
    fn adaptive_width_clamps_between_min_and_max_and_triggers_overflow() {
        // 宽容器：2 个标签上限夹紧至 240px，且 `+` 按钮紧随最后一个标签流末端。
        let wide = shell_desc(800.0, 2);
        assert_eq!(wide.effective_tab_width(), TAB_MAX_WIDTH);
        assert!(!wide.is_overflowing());
        let add = wide.add_rect().expect("addable");
        assert_eq!(add.origin.x, TAB_MAX_WIDTH * 2.0 + TAB_ADD_GAP);
        assert_eq!(add.size.x, TAB_ADD_WIDTH);
        assert_eq!(add.size.y, TAB_ADD_HEIGHT);

        // 中等容器：4 个标签按 strip_width 等分压缩至 (636 - 36) / 4 = 150px。
        let medium = shell_desc(636.0, 4);
        assert_eq!(medium.effective_tab_width(), 150.0);
        assert!(!medium.is_overflowing());

        // 窄容器：6 个标签触底 100px 并开启水平溢出滚动，`+` 按钮停靠可视区右沿。
        let narrow = shell_desc(436.0, 6);
        assert_eq!(narrow.effective_tab_width(), TAB_MIN_WIDTH);
        assert!(narrow.is_overflowing());
        assert_eq!(narrow.max_scroll(), 200.0);
        let docked_add = narrow.add_rect().expect("addable");
        assert_eq!(docked_add.origin.x, 436.0 - TAB_ADD_WIDTH);
    }

    #[test]
    fn shell_chrome_keeps_constant_primitive_count_across_24_states() {
        let desc = shell_desc(600.0, 3);
        let tokens = TabsTokens::standard();
        let mut counts = Vec::new();
        for active in [0usize, 1] {
            let state = TabsState::new(active);
            for phase in [
                PointerPhase::Idle,
                PointerPhase::Hovered,
                PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected: active == 0,
                        };
                        let out = emit_tab(&desc, &state, 0, control, &[], tokens);
                        counts.push(out.primitives.len());
                    }
                }
            }
        }
        assert_eq!(counts.len(), 24);
        assert!(counts.windows(2).all(|w| w[0] == w[1]), "{counts:?}");
    }

    #[test]
    fn shell_chrome_selected_card_has_top_and_side_borders_without_bottom_stroke() {
        let desc = shell_desc(600.0, 3).with_shell_active_border(true);
        let state = TabsState::new(1);
        let tokens = TabsTokens::standard();

        let strip = emit_tabs_strip(&desc, &state, tokens);
        assert_eq!(strip.primitives.len(), 3);
        // 底部线分为选中标签左段与右段，且左段终点等于选中标签左沿。
        let active_rect = desc.tab_rect(1, 0.0);
        assert_eq!(strip.primitives[1].size[0], active_rect.origin.x);
        assert_eq!(strip.primitives[2].local_pos[0], active_rect.max().x);

        let active_tab = emit_tab(&desc, &state, 1, ControlState::default(), &[], tokens);
        assert_eq!(
            active_tab.primitives[0].fill_token_or_uv,
            u32::from(Token::SURFACE_RAISED.raw())
        );
        // 顶/左/右描边透明度为 1.0，右侧非选中分隔线透明度为 0.0。
        assert_eq!(active_tab.primitives[1].opacity(), 1.0);
        assert_eq!(active_tab.primitives[2].opacity(), 1.0);
        assert_eq!(active_tab.primitives[3].opacity(), 1.0);
        assert_eq!(active_tab.primitives[4].opacity(), 0.0);

        // 非选中标签 0 紧邻选中标签 1，分隔线应自动隐藏。
        let adjacent_inactive = emit_tab(&desc, &state, 0, ControlState::default(), &[], tokens);
        assert_eq!(adjacent_inactive.primitives[1].opacity(), 0.0);
        assert_eq!(adjacent_inactive.primitives[4].opacity(), 0.0);

        // 两个连续非选中标签（active=2 时标签 0 与标签 1 之间）应显示 1px × 16px 分隔竖线。
        let state_last = TabsState::new(2);
        let non_adjacent = emit_tab(&desc, &state_last, 0, ControlState::default(), &[], tokens);
        assert_eq!(non_adjacent.primitives[4].opacity(), 1.0);
        assert_eq!(non_adjacent.primitives[4].size, [1.0, 16.0]);
    }

    #[test]
    fn close_hit_is_independent_and_middle_and_right_click_emit_events() {
        let mut desc = shell_desc(600.0, 3);
        desc.items[2] = desc.items[2].clone().pinned();
        assert!(
            desc.close_rect(2, 0.0).is_none(),
            "pinned tabs must not render a close button"
        );

        let mut state = TabsState::new(0);
        let close_rect = desc.close_rect(1, 0.0).expect("tab 1 is closable");
        assert_eq!(close_rect.size.x, TAB_CLOSE_HIT_SIZE);
        assert_eq!(close_rect.size.y, TAB_CLOSE_HIT_SIZE);

        // 点击标签 1 的关闭按钮：发射 CloseRequested 且不得误触选中标签 1。
        let close_evt = desc.handle_left_click(&mut state, close_rect.center());
        assert_eq!(
            close_evt,
            Some(TabViewEvent::CloseRequested { id: 2, index: 1 })
        );
        assert_eq!(state.active, 0);

        // 点击标签 1 标题区：切换激活至标签 1。
        let label_center = desc.label_rect(1, 0.0).center();
        let select_evt = desc.handle_left_click(&mut state, label_center);
        assert_eq!(select_evt, Some(TabViewEvent::Selected { id: 2, index: 1 }));
        assert_eq!(state.active, 1);

        // 点击 `+` 按钮：发射 AddClicked。
        let add_center = desc.add_rect().expect("addable").center();
        assert_eq!(
            desc.handle_left_click(&mut state, add_center),
            Some(TabViewEvent::AddClicked)
        );

        // 中键点击标签 0：发射 MiddleClicked。
        let tab0_center = desc.tab_rect(0, 0.0).center();
        assert_eq!(
            desc.handle_middle_click(&state, tab0_center),
            Some(TabViewEvent::MiddleClicked { id: 1, index: 0 })
        );

        // 右键点击标签 0：发射 ContextMenuRequested。
        let right_pos = Vec2::new(tab0_center.x + 2.4, tab0_center.y);
        assert_eq!(
            desc.handle_right_click(&state, right_pos),
            Some(TabViewEvent::ContextMenuRequested {
                id: 1,
                index: 0,
                screen_pos: [right_pos.x.round() as i32, right_pos.y.round() as i32],
            })
        );
    }

    #[test]
    fn tab_item_widget_icon_precedence_and_external_texture_emission() {
        let plain = TabItem::new(1, "Plain");
        assert_eq!(plain.effective_icon(), WidgetIcon::None);

        let vec_tab = TabItem::new(2, "Docs").with_icon(ShellIcon::Folder);
        assert_eq!(
            vec_tab.effective_icon(),
            WidgetIcon::Vector(ShellIcon::Folder)
        );

        let tex_tab = TabItem::new(3, "Preview")
            .with_icon(ShellIcon::Folder)
            .with_widget_icon(WidgetIcon::ExternalTexture(52));
        assert_eq!(tex_tab.effective_icon(), WidgetIcon::ExternalTexture(52));

        let desc = TabsDesc::new(Rect::new(0.0, 0.0, 480.0, TAB_STRIP_HEIGHT), &[])
            .with_items(vec![tex_tab]);
        assert!(desc.icon_rect(0, 0.0).is_some());

        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();
        let out = emit_tab(&desc, &state, 0, ControlState::default(), &[], tokens);
        let img = out
            .primitives
            .iter()
            .find(|p| p.prim_type() == PrimType::Image)
            .expect("tab external icon primitive");
        assert_eq!(img.fill_token_or_uv, u32::from(Token::NONE.raw()));
        assert_eq!(img.border_token_or_tex, 52);
        assert!((img.opacity() - 1.0).abs() < 0.01);
    }

    #[test]
    fn tab_sub_node_keys_are_deterministic_and_distinct() {
        let tabs_key = 0x1234_5678u64;
        let add = tab_add_node(tabs_key);
        assert_ne!(add, tabs_key);
        let mut keys = vec![tabs_key, add];
        for idx in 0..8 {
            let item = tab_item_node(tabs_key, idx);
            let close = tab_close_node(tabs_key, idx);
            assert_eq!(item, tab_item_node(tabs_key, idx));
            assert_eq!(close, tab_close_node(tabs_key, idx));
            assert!(!keys.contains(&item), "duplicate tab item key at {idx}");
            assert!(!keys.contains(&close), "duplicate tab close key at {idx}");
            assert_ne!(item, close);
            keys.push(item);
            keys.push(close);
        }
    }

    #[test]
    fn close_icon_curve_span_matches_win11_geometry() {
        let desc = shell_desc(600.0, 2);
        let state = TabsState::new(0);
        let tokens = TabsTokens::standard();
        let out = emit_tab_close(
            tab_close_node(99, 0),
            &desc,
            &state,
            0,
            ControlState::default(),
            tokens,
        );
        let curves: Vec<_> = out
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .collect();
        assert_eq!(
            curves.len(),
            2,
            "close icon must emit two diagonal curve strokes"
        );
        for curve in curves {
            let payload = out
                .payloads
                .get(curve.extra_payload_idx as usize)
                .expect("curve payload in bounds");
            let dx = (payload.p2[0] - payload.p0[0]).abs();
            let dy = (payload.p2[1] - payload.p0[1]).abs();
            assert!(
                (8.0..=9.0).contains(&dx) && (8.0..=9.0).contains(&dy),
                "expected 8..=9px close cross span, got dx={dx}, dy={dy}"
            );
        }
    }

    #[test]
    fn declarative_multi_tab_rebases_payloads_and_registers_sub_hits() {
        use crate::ergonomics::{TextCtx, UiWidget};

        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        };
        let desc = TabsDesc::new(Rect::new(0.0, 0.0, 720.0, TAB_STRIP_HEIGHT), &[])
            .with_items(vec![
                TabItem::new(1, "Alpha"),
                TabItem::new(2, "Beta").with_icon(ShellIcon::Folder),
                TabItem::new(3, "Gamma").with_widget_icon(WidgetIcon::ExternalTexture(52)),
            ])
            .with_addable(true)
            .with_style(TabViewStyle::ShellChrome)
            .with_width_mode(TabWidthMode::adaptive_default());
        let state = TabsState::new(1);
        let tabs_key = 4242u64;
        let out = desc.emit_laid_out(
            tabs_key,
            desc.rect,
            ControlState::default(),
            &state,
            &mut text_ctx,
        );
        let close_rects: Vec<Rect> = (0..3).filter_map(|i| desc.close_rect(i, 0.0)).collect();
        assert_eq!(close_rects.len(), 3);

        // 校验所有含载荷图元的索引均不越界且无重复覆盖（每个含载荷图元独占自己的载荷槽）。
        let mut seen_payload_indices = std::collections::HashSet::new();
        let mut close_curve_count = 0usize;
        let mut glyph_count = 0usize;
        let mut image_count = 0usize;
        for prim in &out.primitives {
            if WidgetOutput::primitive_uses_payload(prim) {
                let idx = prim.extra_payload_idx as usize;
                assert!(
                    idx < out.payloads.len(),
                    "payload index {idx} out of bounds ({})",
                    out.payloads.len()
                );
                assert!(
                    seen_payload_indices.insert(idx),
                    "duplicate extra_payload_idx {idx} detected across tabs"
                );
                if prim.prim_type() == PrimType::Curve {
                    let center = Vec2::new(
                        prim.local_pos[0] + prim.size[0] * 0.5,
                        prim.local_pos[1] + prim.size[1] * 0.5,
                    );
                    if close_rects.iter().any(|r| r.contains_point(center)) {
                        close_curve_count += 1;
                        let payload = &out.payloads[idx];
                        let dx = (payload.p2[0] - payload.p0[0]).abs();
                        let dy = (payload.p2[1] - payload.p0[1]).abs();
                        assert!(
                            (8.0..=9.0).contains(&dx) && (8.0..=9.0).contains(&dy),
                            "close curve payload {idx} collapsed or out of range: dx={dx}, dy={dy}"
                        );
                    }
                } else if matches!(prim.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                    glyph_count += 1;
                } else if prim.prim_type() == PrimType::Image {
                    image_count += 1;
                    assert_eq!(prim.fill_token_or_uv, u32::from(Token::NONE.raw()));
                    assert_eq!(prim.border_token_or_tex, 52);
                }
            }
        }
        // 3 个标签页各含 2 条关闭叉号斜线（共 6 条）、多标签字形与 1 个外部纹理图标。
        assert_eq!(
            close_curve_count, 6,
            "expected 6 close curves across 3 tabs"
        );
        assert!(
            glyph_count >= 3,
            "expected title glyphs across 3 tabs, got {glyph_count}"
        );
        assert_eq!(image_count, 1, "expected 1 external texture icon primitive");

        // 校验子热区注册：3 个标签主体 + 3 个关闭按钮（更高 z 序）+ 1 个新建按钮。
        let item_hit_0 = out
            .sub_hits
            .iter()
            .find(|h| h.node == tab_item_node(tabs_key, 0))
            .expect("tab 0 sub_hit");
        let close_hit_0 = out
            .sub_hits
            .iter()
            .find(|h| h.node == tab_close_node(tabs_key, 0))
            .expect("tab 0 close sub_hit");
        assert!(
            close_hit_0.z > item_hit_0.z,
            "close button z ({}) must be above tab item z ({})",
            close_hit_0.z,
            item_hit_0.z
        );
        assert!(
            out.sub_hits
                .iter()
                .any(|h| h.node == tab_add_node(tabs_key)),
            "add button sub_hit must be registered"
        );
    }

    #[test]
    fn fluent_shell_tab_metrics_match_win11_scale() {
        assert!((TAB_STRIP_HEIGHT - 48.0).abs() < f32::EPSILON);
        assert!((36.0..=38.0).contains(&TAB_SHELL_ITEM_HEIGHT));
        assert!((TAB_CLOSE - 12.0).abs() < f32::EPSILON);

        let desc = shell_desc(600.0, 2);
        let tab0 = desc.tab_rect(0, 0.0);
        assert!((tab0.origin.y - TAB_SHELL_TOP_INSET).abs() < f32::EPSILON);
        assert!((tab0.size.y - TAB_SHELL_ITEM_HEIGHT).abs() < f32::EPSILON);
    }

    #[test]
    fn tab_drag_out_emits_tear_off_with_dpi_conversion_and_reorders_inside() {
        let desc = TabsDesc::new(
            Rect::new(0.0, 0.0, 480.0, 40.0),
            &["Tab A", "Tab B", "Tab C"],
        );
        let mut state = TabsState::new(0);

        // 1) 在标签栏内部水平拖拽重排：Tab A -> index 1
        let tab0_center = desc.tab_rect(0, 0.0).center();
        let press_ev = desc.handle_pointer_press(&mut state, tab0_center);
        assert!(matches!(
            press_ev,
            Some(TabViewEvent::Selected { index: 0, .. })
        ));
        let tab1_center = desc.tab_rect(1, 0.0).center();
        let reorder_ev = desc.handle_pointer_move(
            &mut state,
            tab1_center,
            Some(Rect::new(0.0, 0.0, 800.0, 600.0)),
        );
        assert!(matches!(
            reorder_ev,
            Some(TabViewEvent::Reordered {
                from_index: 0,
                to_index: 1,
                ..
            })
        ));
        // 回到标签栏内释放不应触发撕裂。
        let drop_inside =
            desc.handle_pointer_release(&mut state, tab1_center, (100, 50), Some((1200, 900)), 1.5);
        assert!(drop_inside.is_none());

        // 2) 拖出窗口边界并在 1.5x DPI 下释放，稳定触发 TearOffRequested 并换算屏幕物理坐标。
        let press_ev2 = desc.handle_pointer_press(&mut state, tab0_center);
        assert!(press_ev2.is_some());
        let outside_pos = Vec2::new(220.0, 120.0);
        let move_out = desc.handle_pointer_move(
            &mut state,
            outside_pos,
            Some(Rect::new(0.0, 0.0, 800.0, 600.0)),
        );
        assert!(move_out.is_none());
        assert!(state.dragging.is_some_and(|d| d.detached));

        let tear_ev =
            desc.handle_pointer_release(&mut state, outside_pos, (100, 50), Some((1200, 900)), 1.5);
        assert_eq!(
            tear_ev,
            Some(TabViewEvent::TearOffRequested {
                id: desc.items[0].id,
                index: 0,
                screen_pos: [100 + 330, 50 + 180],
            })
        );
        assert!(state.dragging.is_none());
    }
}

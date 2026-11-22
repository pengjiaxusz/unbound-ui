//! 声明式构建器、[`TextCtx`] 自动文本排版与 [`UiWidget`] trait。
//!
//! 本模块将 Unbound UI 零分配扁平 POD 描述符（`*Desc`）桥接到 `unbound-app` 的双遍布局与文本排版管线：
//! 1. **无 Rect 构造函数与无前缀构建方法**（`ButtonDesc::label("Save").variant(...)`、
//!    `SliderDesc::range(0.0, 100.0, 42.0)`、`TextInputDesc::text("hello")` 等）。
//! 2. **[`TextCtx`]** 用于将单行或对齐的控件标签测量并排版为 [`GlyphQuad`]。
//! 3. **[`UiWidget`]** trait，为所有控件描述符与 [`WidgetCatalogId`] 统一实现。

use unbound_input::KeyName;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;

use crate::address_bar::{
    compute_text_edit_visual, emit_address_bar_with_visual, AddressBarDesc, AddressBarState,
    AddressBarTextVisual,
};
use crate::alert::{emit_alert, AlertDesc, Severity};
use crate::badge::{emit_badge, BadgeDesc};
use crate::button::{emit_button, ButtonDesc, ButtonSize, ButtonVariant};
use crate::callout::{emit_callout_with_glyphs, CalloutDesc, CalloutState};
use crate::catalog::{emit_widget_with_shaper, CatalogParams};
use crate::checkbox::{emit_checkbox, CheckState, CheckboxDesc, CheckboxTokens};
use crate::code_editor::{emit_code_editor, CodeEditorDesc, CodeEditorState};
use crate::containers::{emit_card, CardDesc, CardTokens};
use crate::data_grid::{emit_data_grid, DataGridDesc, DataGridState};
use crate::descriptions::{
    emit_descriptions_with_glyphs, DescriptionsDesc, DescriptionsState, DESCRIPTION_ROW_HEIGHT,
};
use crate::divider::{emit_divider, DividerDesc, DividerOrientation, DividerTokens};
use crate::generic_data_table::{
    emit_generic_data_table, GenericDataTableDesc, GenericDataTableState,
};
use crate::multi_split::{emit_multi_split, MultiSplitDesc, MultiSplitState};
use crate::pagination::{emit_pagination, PaginationDesc, PaginationState};
use crate::pipeline_view::{emit_pipeline_view, PipelineViewDesc};
use crate::popconfirm::{emit_popconfirm, PopconfirmDesc};
use crate::progress::{emit_progress, ProgressDesc, ProgressState};
use crate::query_builder::{emit_query_builder, QueryBuilderDesc};
use crate::schema_form::{
    emit_schema_form, SchemaFormDesc, SchemaFormEvent, SchemaFormState, LABEL_FONT_SIZE,
};
use crate::select::{emit_select_with_glyphs, SelectDesc, SelectState};
use crate::shortcut_bar::{emit_shortcut_bar, ShortcutBarDesc};
use crate::slider::{emit_slider, value_from_pointer, Orientation, SliderDesc, SliderTokens};
use crate::split_button::{
    emit_split_button_with_glyphs, SplitButtonDesc, SplitButtonState, DROPDOWN_WIDTH,
};
use crate::switch::{
    emit_switch, SwitchDesc, SwitchLayout, SwitchTokens, SWITCH_HEADER_GAP, SWITCH_HEADER_LINE,
    SWITCH_LABEL_GAP,
};
use crate::table::{emit_table, ColumnSpec, TableDesc, TableRow, TableState};
use crate::task_hud::{emit_task_hud, TaskHudDesc};
use crate::terminal_view::{
    emit_terminal_view_with_glyphs, TerminalViewDesc, TerminalViewState, TERMINAL_LINE_HEIGHT,
};
use crate::text_input::{
    emit_text_input, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    TextInputVisual,
};
use crate::timeline_track::{emit_timeline_track, TimelineTrackDesc, TimelineTrackState};
use crate::toggle_button::{emit_toggle_button_with_glyphs, ToggleButtonDesc, ToggleButtonState};
use crate::tooltip::{TooltipDesc, TooltipState};
use crate::tour::{emit_tour, TourDesc};
use crate::transfer::{emit_transfer, TransferDesc, TransferState};
use crate::tree_table::{
    emit_tree_table_with_glyphs, TreeTableDesc, TreeTableState, TREE_TABLE_HEADER_HEIGHT,
    TREE_TABLE_ROW_HEIGHT,
};
use crate::virtual_masonry::{emit_virtual_masonry, VirtualMasonryDesc, VirtualMasonryState};
use crate::{ControlState, WidgetCatalogId, WidgetOutput};

/// 边界矩形内的水平文本对齐方式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum AlignX {
    /// 靠前（左）边缘对齐。
    #[default]
    Left,
    /// 水平居中。
    Center,
    /// 靠后（右）边缘对齐。
    Right,
}

/// 边界矩形内的垂直文本对齐方式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum AlignY {
    /// 靠上边缘对齐。
    Top,
    /// 垂直居中。
    #[default]
    Middle,
    /// 靠下边缘对齐。
    Bottom,
}

/// 传递给 [`UiWidget`] 方法的文本排版与字形光栅化上下文。
pub struct TextCtx<'a> {
    /// 共享文本排版引擎。
    pub engine: &'a mut unbound_text::TextEngine,
    /// 共享字形光栅化与图集缓存。
    pub cache: &'a mut unbound_text::GlyphCache,
    /// 当前帧 / 图集时钟。
    pub atlas_clock: u64,
    /// 每逻辑像素对应的物理纹素光栅化缩放比（`os_scale * ui_scale`，默认 `1.0`）。
    ///
    /// 对齐 `Showcase::Labels` 与 `uber.vert` 的位图字形契约：
    /// 文本按 `size * raster_scale` 与物理起点 `origin * raster_scale` 整形及光栅化，
    /// 再将产出的 [`GlyphQuad`] 矩形除以 `raster_scale` 还原回逻辑坐标系，
    /// 使 GPU 顶点着色器乘回 `scale` 后严格实现「1 图集纹素 = 1 物理屏幕像素」。
    pub raster_scale: f32,
}

impl TextCtx<'_> {
    /// 返回钳制在安全正数区间 `[0.5, 4.0]` 内的物理光栅化缩放因子。
    #[must_use]
    pub fn effective_raster_scale(&self) -> f32 {
        if self.raster_scale.is_finite() && self.raster_scale > 0.0 {
            self.raster_scale.clamp(0.5, 4.0)
        } else {
            1.0
        }
    }

    /// 按 `size` 与 `weight` 测量单行文本（返回逻辑像素尺寸）。
    pub fn measure_line(&mut self, text: &str, size: f32, weight: u16) -> Vec2 {
        if text.is_empty() {
            return Vec2::new(0.0, size * 1.35);
        }
        let scale = self.effective_raster_scale();
        let mut style = unbound_text::TextStyle::sized(size * scale);
        style.weight = weight;
        let shaped = self.engine.shape(text, &style);
        Vec2::new(
            shaped.width / scale,
            (shaped.height / scale).max(size * 1.35),
        )
    }

    /// 在 `rect` 内对齐排版并按物理缩放因子光栅化单行文本（返回逻辑像素坐标系的 [`GlyphQuad`]）。
    pub fn shape_line(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        weight: u16,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Vec<GlyphQuad> {
        self.shape_line_styled(text, rect, size, weight, None, align_x, align_y)
    }

    /// 与 [`Self::shape_line`] 相同，但允许显式指定字体族（例如 WinUI 的
    /// 「Segoe UI Variable Text」光学尺寸）。
    #[allow(clippy::too_many_arguments)]
    pub fn shape_line_styled(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        weight: u16,
        family: Option<&str>,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Vec<GlyphQuad> {
        if text.is_empty() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Vec::new();
        }
        let scale = self.effective_raster_scale();
        let mut style = unbound_text::TextStyle::sized(size * scale);
        style.weight = weight;
        style.family = family.map(str::to_owned);
        let shaped = match self.engine.shape_laid_out(text, &style) {
            Some(shaped) => shaped,
            None => self.engine.shape(text, &style),
        };
        // XAML 的 `TextBlock` 在测量时把文本宽度向上取整到整像素，其
        // ContentPresenter 再按该期望宽度居中；直接用浮点宽度会让居中
        // 起点系统地偏右零点几像素。
        let logical_w = (shaped.width / scale).ceil();
        let logical_h = (shaped.height / scale).max(size);
        let x = match align_x {
            AlignX::Left => rect.origin.x,
            AlignX::Center => {
                let center_x = rect.origin.x + ((rect.width() - logical_w) * 0.5).max(0.0);
                (center_x * scale).round() / scale
            }
            AlignX::Right => (rect.max().x - logical_w).max(rect.origin.x),
        };
        let y = match align_y {
            AlignY::Top => rect.origin.y,
            // WinUI 的 ContentPresenter 以字体自然行高居中，而本引擎按
            // `font_size * 1.4` 的行盒居中；实测 14px 正文在 32px 控件里
            // 因此整体低 1px。所有 Middle 对齐统一上移 1px，使基线对齐
            // 参考截图的像素网格（否则字形亚像素相位与参考错位一整行）。
            AlignY::Middle => rect.origin.y + ((rect.height() - logical_h) * 0.5).max(0.0) - 1.0,
            AlignY::Bottom => (rect.max().y - logical_h).max(rect.origin.y),
        };
        let mut quads = self.cache.quads_for(
            self.engine,
            &shaped,
            Vec2::new(x * scale, y * scale),
            weight,
            false,
        );
        if (scale - 1.0).abs() > 1e-6 {
            for quad in &mut quads {
                quad.rect.origin.x /= scale;
                quad.rect.origin.y /= scale;
                quad.rect.size.x /= scale;
                quad.rect.size.y /= scale;
            }
        }
        quads
    }
}

/// 在冒泡到应用程序前提供给 [`UiWidget::reduce`] 的高层输入事件。
#[derive(Clone, Debug, PartialEq)]
pub enum WidgetInputEvent {
    /// 指针按键在屏幕坐标 `pos` 按下（附带 `click_count`）。
    PointerDown {
        /// 屏幕空间指针位置。
        pos: Vec2,
        /// 连击次数（`1`、`2`、`3`）。
        click_count: u8,
    },
    /// 指针拖拽至屏幕坐标 `pos`。
    PointerDrag {
        /// 屏幕空间指针位置。
        pos: Vec2,
    },
    /// 指针在屏幕坐标 `pos` 释放。
    PointerUp {
        /// 屏幕空间指针位置。
        pos: Vec2,
    },
    /// 鼠标滚轮增量。
    Wheel {
        /// 滚动增量。
        delta: Vec2,
    },
    /// 提交的文本 / 字符输入。
    TextInput(String),
    /// 输入法预编辑更新（替换组合区间）。
    ImePreedit {
        /// 组合文本。
        text: String,
        /// 组合文本内的光标字节偏移。
        cursor: Option<usize>,
    },
    /// 输入法提交。
    ImeCommit(String),
    /// 取消输入法组合。
    ImeCancel,
    /// 聚焦时按下的组合键。
    KeyDown(unbound_input::KeyChord),
}

/// 统一的声明式控件契约，涵盖自动布局测量、文本排版、状态归约与图元发射。
pub trait UiWidget: Clone + core::fmt::Debug + Send + Sync + 'static {
    /// 存储在 `WidgetStateStore` 中的持久化控件状态。
    type State: Default + Send + Sync + 'static;

    /// 首次挂载控件键时构造初始状态。
    fn initial_state(&self) -> Self::State {
        Self::State::default()
    }

    /// 当控件键已存在于状态池中时，将外部描述符的最新受控值同步进局部状态。
    fn sync_state(&self, _state: &mut Self::State) {}

    /// 使用 `text_ctx` 进行真实字体测量，计算控件的自然（固有）尺寸。
    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2;

    /// 在持久化状态上归约控件级输入事件，若状态发生变更则返回 `true`。
    fn reduce(&self, _state: &mut Self::State, _rect: Rect, _event: &WidgetInputEvent) -> bool {
        false
    }

    /// 将已布局的图元、载荷、命中区域与无障碍节点发射到 `rect` 中。
    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput;
}

// ============================================================================
// 无 Rect 构造函数与无前缀构建方法
// ============================================================================

impl ButtonDesc {
    /// 创建带 `label` 的无 Rect 按钮描述符（默认无重阴影、6px 圆角、13px 字号）。
    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, label)
            .with_shadow(0.0)
            .with_radius(6.0)
            .with_font_size(13.0)
    }

    /// 设置按钮视觉变体。
    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// 设置按钮标准尺寸。
    #[must_use]
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// 设置加载指示器状态。
    #[must_use]
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// 设置原生矢量图标（[`crate::shell::ShellIcon`]）。
    #[must_use]
    pub fn shell_icon(mut self, icon: crate::shell::ShellIcon) -> Self {
        self.shell_icon = Some(icon);
        self
    }
}

impl CheckboxDesc {
    /// 创建带 `label` 与 `checked` 状态的无 Rect 复选框。
    #[must_use]
    pub fn labeled(label: impl Into<String>, checked: bool) -> Self {
        Self::new(Rect::ZERO, label).with_checked(checked)
    }

    /// 设置选中状态。
    #[must_use]
    pub fn checked(self, checked: bool) -> Self {
        self.with_checked(checked)
    }

    /// 设置三态勾选位置。
    #[must_use]
    pub fn state(self, state: CheckState) -> Self {
        self.with_state(state)
    }
}

impl SwitchDesc {
    /// 创建 `on` 状态的无 Rect 开关。
    #[must_use]
    pub fn on(on: bool) -> Self {
        Self::new(Rect::ZERO, "Switch").with_on(on)
    }

    /// 创建带标签与 `on` 状态的无 Rect 开关。
    #[must_use]
    pub fn labeled(label: impl Into<String>, on: bool) -> Self {
        Self::new(Rect::ZERO, label).with_on(on)
    }
}

impl SliderDesc {
    /// 创建带 `min`、`max` 与初始 `value` 的无 Rect 滑块。
    #[must_use]
    pub fn range(min: f32, max: f32, value: f32) -> Self {
        Self::new(Rect::ZERO, "Slider", value, min, max)
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置量化步长。
    #[must_use]
    pub fn step(self, step: f32) -> Self {
        self.with_step(step)
    }

    /// 设置滑块方向。
    #[must_use]
    pub fn orientation(self, orientation: Orientation) -> Self {
        self.with_orientation(orientation)
    }
}

impl TextInputDesc {
    /// 创建以 `initial_text` 初始化的无 Rect 文本输入框。
    #[must_use]
    pub fn text(initial_text: impl Into<String>) -> Self {
        Self::new(Rect::ZERO).with_label(initial_text)
    }

    /// 设置占位文本。
    #[must_use]
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        self.with_placeholder(placeholder)
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn label(self, label: impl Into<String>) -> Self {
        self.with_label(label)
    }
}

impl SelectDesc {
    /// 创建带 `options` 的无 Rect 下拉选择框。
    #[must_use]
    pub fn options(options: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self::new(
            Rect::ZERO,
            options
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
        )
    }

    /// 设置占位文本。
    #[must_use]
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        self.with_placeholder(placeholder)
    }

    /// 预选指定索引的选项。
    #[must_use]
    pub fn selected(mut self, index: usize) -> Self {
        self.initial_selection = Some(index);
        self
    }
}

impl TableDesc {
    /// 创建带 `columns` 的无 Rect 表格。
    #[must_use]
    pub fn columns(columns: Vec<ColumnSpec>) -> Self {
        Self::new(Rect::ZERO, columns)
    }
}

impl BadgeDesc {
    /// 创建带 `text` 的无 Rect 胶囊徽标。
    #[must_use]
    pub fn label(text: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, text)
    }

    /// 切换为主强调色胶囊样式（`ACCENT` 底 + `ON_ACCENT` 文本）。
    #[must_use]
    pub fn primary(mut self) -> Self {
        self.variant = crate::badge::BadgeVariant::Primary;
        self
    }

    /// 切换为成功语义胶囊样式（`SUCCESS_SOFT` 底 + `SUCCESS` 文本）。
    #[must_use]
    pub fn success(mut self) -> Self {
        self.variant = crate::badge::BadgeVariant::Success;
        self
    }

    /// 切换为警告语义胶囊样式（`WARNING_SOFT` 底 + `WARNING` 文本）。
    #[must_use]
    pub fn warning(mut self) -> Self {
        self.variant = crate::badge::BadgeVariant::Warning;
        self
    }

    /// 切换为危险/错误语义胶囊样式（`ERROR_SOFT` 底 + `ERROR` 文本）。
    #[must_use]
    pub fn error(mut self) -> Self {
        self.variant = crate::badge::BadgeVariant::Error;
        self
    }

    /// 切换为中性语义胶囊样式（`SURFACE_SUNKEN` 底 + `TEXT_SECONDARY` 文本）。
    #[must_use]
    pub fn neutral(mut self) -> Self {
        self.variant = crate::badge::BadgeVariant::Neutral;
        self
    }

    /// 设置是否显示前置状态圆点。
    #[must_use]
    pub fn dot(mut self, enabled: bool) -> Self {
        self.dot = enabled;
        self
    }
}

impl AlertDesc {
    /// 创建带 `severity`、`title` 与 `message` 的无 Rect 行内提示横幅。
    #[must_use]
    pub fn message(
        severity: Severity,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Rect::ZERO, severity, title).with_message(message)
    }
}

impl ProgressDesc {
    /// 创建进度为 `value`（`0.0..=1.0`）的无 Rect 线性进度条。
    #[must_use]
    pub fn bar(value: f32) -> Self {
        Self::new(Rect::ZERO).value(value)
    }

    /// 创建进度为 `value`（`0.0..=1.0`）的无 Rect 环形进度条。
    #[must_use]
    pub fn ring(value: f32) -> Self {
        Self::circular(Rect::ZERO).value(value)
    }
}

impl DividerDesc {
    /// 创建无 Rect 水平分隔线。
    #[must_use]
    pub fn rule() -> Self {
        Self::horizontal(Rect::ZERO)
    }
}

// ============================================================================
// UiWidget 实现
// ============================================================================

impl UiWidget for ButtonDesc {
    type State = ();

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        // 按下按钮即吞掉事件：按钮是离散交互控件，按压绝不能再冒泡给祖先。
        // 例如标题栏里的按钮若不吞掉按压，就会冒泡到标题栏根容器，连带拖走整个窗口。
        matches!(event, WidgetInputEvent::PointerDown { .. })
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, self.font_size, 400);
        // WinUI `ButtonPadding = 11,5,11,6`：标准按钮的文本区两侧各让出 11px。
        let pad_x = if self.left_align {
            20.0
        } else if self.size == ButtonSize::Sm {
            18.0
        } else {
            crate::button::BUTTON_PADDING.0 + crate::button::BUTTON_PADDING.2
        };
        let min_w = if self.label.chars().count() <= 2 {
            26.0
        } else if self.size == ButtonSize::Sm {
            42.0
        } else {
            56.0
        };
        let has_widget_icon = self.effective_widget_icon() != crate::shell::WidgetIcon::None;
        let icon_extra = if has_widget_icon {
            if self.label.is_empty() {
                crate::button::BUTTON_SHELL_ICON_SIZE
            } else {
                crate::button::BUTTON_SHELL_ICON_SIZE + crate::button::BUTTON_CONTENT_GAP
            }
        } else {
            0.0
        };
        let chevron_extra = if self.dropdown_chevron {
            if self.label.is_empty() && !has_widget_icon && self.icon.is_none() {
                crate::button::BUTTON_DROPDOWN_CHEVRON_SIZE
            } else {
                crate::button::BUTTON_DROPDOWN_CHEVRON_SIZE
                    + crate::button::BUTTON_DROPDOWN_CHEVRON_GAP
            }
        } else {
            0.0
        };
        let w = (measured.x + icon_extra + chevron_extra + pad_x)
            .max(self.rect.width())
            .max(min_w);
        let h = self.size.height().max(self.rect.height());
        Vec2::new(w, h)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.centre_label =
            desc.effective_widget_icon() != crate::shell::WidgetIcon::None || desc.dropdown_chevron;
        let (text_rect, align_x) = if desc.left_align {
            let inset_left = if desc.active_selected { 11.0 } else { 9.0 };
            (
                Rect::new(
                    rect.origin.x + inset_left,
                    rect.origin.y,
                    (rect.width() - inset_left - 6.0).max(0.0),
                    rect.height(),
                ),
                AlignX::Left,
            )
        } else {
            (rect, AlignX::Center)
        };
        let mut glyphs = text_ctx.shape_line(
            &desc.label,
            text_rect,
            desc.font_size,
            400,
            align_x,
            AlignY::Middle,
        );
        emit_button(node, &desc, control, &mut glyphs)
    }
}

impl UiWidget for CheckboxDesc {
    type State = CheckState;

    fn initial_state(&self) -> Self::State {
        self.state
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, 14.0, 400);
        // WinUI：MinWidth 120、MinHeight 32、方框列 20 + 右边距 8。
        let w = (self.gap + measured.x + 8.0)
            .max(self.rect.width())
            .max(self.box_size)
            .max(crate::checkbox::CHECKBOX_MIN_WIDTH);
        let h = crate::checkbox::CHECKBOX_HEIGHT.max(self.rect.height());
        Vec2::new(w, h)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if matches!(event, WidgetInputEvent::PointerDown { .. }) {
            *state = match *state {
                CheckState::Checked => CheckState::Unchecked,
                CheckState::Unchecked | CheckState::Indeterminate => CheckState::Checked,
            };
            return true;
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        mut control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.state = *state;
        control.selected = desc.state.is_checked();
        let label_r = desc.label_rect();
        let mut glyphs = text_ctx.shape_line(
            &desc.label,
            label_r,
            14.0,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        // WinUI 复选框的标签行盒比标准 32px 行更高 1px：参考截图的墨迹顶端
        // 比按钮/输入框再高 1px，这里补齐剩余偏移。
        for quad in &mut glyphs {
            quad.rect.origin.y -= 1.0;
        }
        emit_checkbox(
            node,
            &desc,
            control,
            &[],
            &glyphs,
            CheckboxTokens::standard(),
        )
    }
}

impl UiWidget for SwitchDesc {
    type State = bool;

    fn initial_state(&self) -> Self::State {
        self.on
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, 14.0, 400);
        let label_w = if self.label.is_empty() {
            0.0
        } else {
            measured.x
        };
        let (w, h) = match self.layout {
            // Header 排版：标签独占一行，轨道在下一行左对齐。
            SwitchLayout::Header => (
                label_w
                    .max(self.track_size.0)
                    .max(crate::switch::SWITCH_MIN_WIDTH)
                    .max(self.rect.width()),
                (SWITCH_HEADER_LINE
                    + SWITCH_HEADER_GAP
                    + self.track_size.1
                    + crate::switch::SWITCH_HEADER_TAIL)
                    .max(self.rect.height()),
            ),
            // 标签在左、轨道在右，两者之间留 10px：
            // 旧算法只按「轨道宽 + 标签宽」估算总宽，却把标签画在轨道右侧 50px 处，
            // 于是标签必然压在轨道上——设置页里的开关会盖住自己的文字。
            SwitchLayout::Row => (
                (label_w + SWITCH_LABEL_GAP + self.track_size.0).max(self.rect.width()),
                self.track_size.1.max(26.0).max(self.rect.height()),
            ),
        };
        Vec2::new(w, h)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if matches!(event, WidgetInputEvent::PointerDown { .. }) {
            *state = !*state;
            return true;
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        mut control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.on = *state;
        control.selected = desc.on;
        // Win11 设置行是「文字在左、开关在右」；WinUI 模板的 Header 排版则是
        // 「标题在上、开关在左下」。两种排版各自决定标签字形的位置。
        let label_r = match desc.layout {
            SwitchLayout::Header => Rect::new(
                rect.origin.x,
                rect.origin.y,
                rect.width(),
                SWITCH_HEADER_LINE,
            ),
            SwitchLayout::Row => Rect::new(
                rect.origin.x,
                rect.origin.y,
                (rect.width() - desc.track_size.0 - SWITCH_LABEL_GAP).max(0.0),
                rect.height(),
            ),
        };
        let glyphs = text_ctx.shape_line(
            &desc.label,
            label_r,
            14.0,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        // Header 文本与标准正文共用同一基线：参考截图墨迹顶端 y=21，
        // 与本引擎直接居中后的位置一致，无需额外补偿。
        emit_switch(node, &desc, control, &glyphs, SwitchTokens::standard())
    }
}

impl UiWidget for SliderDesc {
    type State = f32;

    fn initial_state(&self) -> Self::State {
        self.value
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        // WinUI `Slider` 与按钮、输入框共享 32px 标准交互行高；旧实现的 28px
        // 会在自由维度固有尺寸门禁下被参考边界暴露为偏差。
        Vec2::new(
            self.rect.width().max(160.0),
            self.rect
                .height()
                .max(unbound_theme::typography::geometry::INPUT_HEIGHT),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { pos, .. } | WidgetInputEvent::PointerDrag { pos } => {
                let mut desc = self.clone();
                desc.rect = rect;
                *state = value_from_pointer(*pos, &desc);
                true
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.value = state.clamp(desc.min.min(desc.max), desc.max.max(desc.min));
        let glyphs = text_ctx.shape_line(&desc.label, rect, 12.0, 400, AlignX::Left, AlignY::Top);
        emit_slider(node, &desc, control, &glyphs, SliderTokens::standard())
    }
}

impl UiWidget for TextInputDesc {
    type State = TextEditState;

    fn initial_state(&self) -> Self::State {
        if self.label.is_empty() {
            TextEditState::new()
        } else {
            TextEditState::with_text(&self.label)
        }
    }

    fn sync_state(&self, state: &mut Self::State) {
        if !self.label.is_empty() && state.text() != self.label {
            state.set_text_preserving_cursor(&self.label);
        }
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let sample = if !self.label.is_empty() {
            self.label.as_str()
        } else {
            self.placeholder.as_str()
        };
        let measured = text_ctx.measure_line(sample, self.font_size, 400);
        Vec2::new(
            (measured.x + 28.0).max(self.rect.width()).max(160.0),
            // WinUI `TextBox` 与 `ComboBox` 同高：32px。旧实现的 34px 会被
            // 参考边界掩盖，只有在自由维度固有尺寸门禁开启后才会暴露。
            self.rect
                .height()
                .max(unbound_theme::typography::geometry::INPUT_HEIGHT),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if self.disabled {
            return false;
        }
        if self.read_only {
            // 只读输入框仍需吞掉按压：命中它即代表用户想在其中选择/复制文本，
            // 绝不能让事件继续冒泡到标题栏根容器触发窗口拖拽。
            return matches!(event, WidgetInputEvent::PointerDown { .. });
        }
        match event {
            // 按下输入框时：若命中可清除输入框右侧清除按钮则清空内容；
            // 否则按点击位置估算字符索引定位光标，并吞掉事件防止触发标题栏拖拽。
            WidgetInputEvent::PointerDown { pos, click_count } => {
                let mut desc = self.clone();
                desc.rect = rect;
                if self.clearable
                    && !state.text().is_empty()
                    && desc.clear_rect().contains_point(*pos)
                {
                    // 复用「全选 + 退格」既有编辑通路清空内容，使清除动作同样进入撤销历史。
                    state.select_all();
                    let outcome = reduce_text_input(&desc, state, &TextInputEvent::Backspace);
                    return outcome.changed;
                }
                if *click_count >= 2 {
                    state.select_all();
                } else {
                    let rel_x = (pos.x - (rect.origin.x + 11.0)).max(0.0);
                    let approx_char_w = (self.font_size * 0.56).max(6.0);
                    let target_chars = (rel_x / approx_char_w).round() as usize;
                    let byte_idx = state
                        .text()
                        .char_indices()
                        .nth(target_chars)
                        .map_or_else(|| state.text().len(), |(idx, _)| idx);
                    state.set_cursor(byte_idx, false);
                }
                true
            }
            WidgetInputEvent::TextInput(text) => {
                let outcome = reduce_text_input(self, state, &TextInputEvent::Insert(text.clone()));
                outcome.changed
            }
            WidgetInputEvent::ImePreedit { text, cursor } => {
                let outcome = reduce_text_input(
                    self,
                    state,
                    &TextInputEvent::ImePreedit {
                        text: text.clone(),
                        cursor: *cursor,
                    },
                );
                outcome.changed
            }
            WidgetInputEvent::ImeCommit(text) => {
                let outcome = reduce_text_input(
                    self,
                    state,
                    &TextInputEvent::ImeCommit { text: text.clone() },
                );
                outcome.changed
            }
            WidgetInputEvent::ImeCancel => {
                reduce_text_input(self, state, &TextInputEvent::ImeCancel).changed
            }
            WidgetInputEvent::KeyDown(chord) => {
                let ev = match chord.key {
                    KeyName::Named("Backspace") => {
                        if chord.ctrl {
                            Some(TextInputEvent::DeleteWordBack)
                        } else {
                            Some(TextInputEvent::Backspace)
                        }
                    }
                    KeyName::Named("Delete") => {
                        if chord.ctrl {
                            Some(TextInputEvent::DeleteWordForward)
                        } else {
                            Some(TextInputEvent::Delete)
                        }
                    }
                    KeyName::Named("ArrowLeft") => {
                        if chord.ctrl {
                            Some(TextInputEvent::MoveWordLeft {
                                extend: chord.shift,
                            })
                        } else {
                            Some(TextInputEvent::MoveLeft {
                                extend: chord.shift,
                            })
                        }
                    }
                    KeyName::Named("ArrowRight") => {
                        if chord.ctrl {
                            Some(TextInputEvent::MoveWordRight {
                                extend: chord.shift,
                            })
                        } else {
                            Some(TextInputEvent::MoveRight {
                                extend: chord.shift,
                            })
                        }
                    }
                    KeyName::Named("Home") => Some(TextInputEvent::Home {
                        extend: chord.shift,
                    }),
                    KeyName::Named("End") => Some(TextInputEvent::End {
                        extend: chord.shift,
                    }),
                    KeyName::Char('a') if chord.ctrl => Some(TextInputEvent::SelectAll),
                    KeyName::Char('z') if chord.ctrl && !chord.shift => Some(TextInputEvent::Undo),
                    KeyName::Char('z') if chord.ctrl && chord.shift => Some(TextInputEvent::Redo),
                    KeyName::Char('y') if chord.ctrl => Some(TextInputEvent::Redo),
                    _ => None,
                };
                if let Some(input_ev) = ev {
                    let outcome = reduce_text_input(self, state, &input_ev);
                    outcome.changed
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let display = if !state.text().is_empty() {
            desc.display_text(state.text())
        } else {
            desc.placeholder.clone()
        };
        // WinUI `TextControlThemePadding` 左内边距 11px：参考截图的字形墨迹
        // 从控件左沿 +11 开始（旧值 10 会让整行文字左移 1px）。
        let text_r = Rect::new(
            rect.origin.x + 11.0,
            rect.origin.y + 2.0,
            (rect.width() - 22.0).max(0.0),
            (rect.height() - 4.0).max(0.0),
        );
        let mut glyphs = text_ctx.shape_line(
            &display,
            text_r,
            desc.font_size,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        let caret_x = if state.text().is_empty() || state.cursor() == 0 {
            text_r.origin.x
        } else {
            let cursor_byte = state.cursor().min(state.text().len());
            let char_idx = state.text()[..cursor_byte].chars().count();
            if char_idx == 0 {
                text_r.origin.x
            } else if let Some(g) = glyphs.get(char_idx - 1) {
                g.rect.max().x.max(text_r.origin.x)
            } else {
                glyphs
                    .last()
                    .map_or(text_r.origin.x, |g| g.rect.max().x.max(text_r.origin.x))
            }
        };
        let visual = TextInputVisual {
            caret: Rect::new(
                caret_x,
                rect.origin.y + 6.0,
                1.5,
                (rect.height() - 12.0).max(1.0),
            ),
            caret_visible: control.focused && !self.disabled,
            ..TextInputVisual::default()
        };
        emit_text_input(node, &desc, control, state, &visual, &mut glyphs)
    }
}

impl UiWidget for SelectDesc {
    type State = SelectState;

    fn initial_state(&self) -> Self::State {
        SelectState {
            open: false,
            selected: self.initial_selection,
            highlighted: self.initial_selection,
        }
    }

    fn sync_state(&self, state: &mut Self::State) {
        // 受控控件：描述符里的 `initial_selection` 是选中项的唯一真源。
        // 应用侧改了值（例如下拉列表里选了另一项）之后，框架每帧把它同步回局部状态，
        // 收起态的输入框才会立刻显示新值，而不必等控件被重新挂载。
        if let Some(selected) = self.initial_selection {
            if state.selected != Some(selected) {
                state.selected = Some(selected);
            }
            if state.highlighted.is_none() {
                state.highlighted = state.selected;
            }
        }
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let longest = self
            .options
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(self.placeholder.as_str()))
            .max_by_key(|s| s.len())
            .unwrap_or("");
        let measured = text_ctx.measure_line(longest, 14.0, 400);
        Vec2::new(
            (measured.x + 40.0).max(self.rect.width()).max(140.0),
            // Win11 的下拉框与输入框同高（32px），不是 34px。
            self.rect
                .height()
                .max(unbound_theme::typography::geometry::INPUT_HEIGHT),
        )
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { .. } => {
                // 只负责开合：真正的选项切换发生在应用渲染的下拉列表里。
                // 旧实现在按下时顺手把选中项跳到下一项，于是「点一下输入框」
                // 会悄悄改掉用户的设置——这既不是 WinUI 的行为，也没法承载
                // 「打开列表 → 选中某一项 → 应用变更」这条标准链路。
                state.open = !state.open;
                true
            }
            WidgetInputEvent::KeyDown(chord) => match chord.key {
                KeyName::Named("ArrowDown") | KeyName::Named("ArrowRight") => {
                    self.clone().step_selection(state, true);
                    true
                }
                KeyName::Named("ArrowUp") | KeyName::Named("ArrowLeft") => {
                    self.clone().step_selection(state, false);
                    true
                }
                KeyName::Named("Enter") | KeyName::Named("Space") => {
                    state.open = !state.open;
                    true
                }
                KeyName::Named("Escape") => {
                    state.close();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let display = desc.display_text(state);
        // Win11 下拉框的文本左内边距是 12px（参考截图中框左沿 16、墨迹左沿 29，
        // 扣掉字形 1px 左侧承距后为 12）。
        let text_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y,
            (rect.width() - 34.0).max(0.0),
            rect.height(),
        );
        let glyphs = text_ctx.shape_line(display, text_r, 14.0, 400, AlignX::Left, AlignY::Middle);
        emit_select_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl SelectDesc {
    fn step_selection(&self, state: &mut SelectState, forward: bool) {
        if self.options.is_empty() {
            return;
        }
        state.highlighted = state.selected;
        state.step(self, forward);
        state.selected = state.highlighted;
    }
}

impl UiWidget for TableDesc {
    type State = TableState;

    fn initial_state(&self) -> Self::State {
        TableState::new()
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let w = self.total_width().max(self.rect.width()).max(240.0);
        let rows = 5.0_f32;
        let h = (self.header_height + rows * self.row_height)
            .max(self.rect.height())
            .max(120.0);
        Vec2::new(w, h)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let mut header_glyphs = Vec::new();
        for idx in 0..desc.columns.len() {
            if let Some(col_r) = desc.column_rect(idx) {
                let padded = Rect::new(
                    col_r.origin.x + 8.0,
                    col_r.origin.y,
                    (col_r.width() - 16.0).max(0.0),
                    col_r.height(),
                );
                let title = &desc.columns[idx].title;
                header_glyphs.extend(text_ctx.shape_line(
                    title,
                    padded,
                    13.0,
                    600,
                    AlignX::Left,
                    AlignY::Middle,
                ));
            }
        }
        let vis_count = desc.visible_row_count();
        let mut row_glyph_vecs: Vec<Vec<GlyphQuad>> = Vec::with_capacity(vis_count);
        for r in 0..vis_count {
            let row_r = desc.row_rect(r, state.offset());
            let mut row_glyphs = Vec::new();
            if desc.columns.is_empty() {
                let padded = Rect::new(
                    row_r.origin.x + 8.0,
                    row_r.origin.y,
                    (row_r.width() - 16.0).max(0.0),
                    row_r.height(),
                );
                row_glyphs.extend(text_ctx.shape_line(
                    &format!("Row {}", r + 1),
                    padded,
                    13.0,
                    400,
                    AlignX::Left,
                    AlignY::Middle,
                ));
            } else {
                let mut col_x = row_r.origin.x;
                for (c_idx, col) in desc.columns.iter().enumerate() {
                    let cell_r = Rect::new(
                        col_x + 8.0,
                        row_r.origin.y,
                        (col.width - 16.0).max(0.0),
                        row_r.height(),
                    );
                    let cell_text = match c_idx {
                        0 => format!("Item #{:02}", r + 1),
                        1 => format!("{} KB", (r + 1) * 12),
                        _ => format!("Col {}", c_idx + 1),
                    };
                    // `ColumnSpec.align` 是 table 模块的枚举，需映射到本模块文本排版用的 AlignX。
                    let align_x = match col.align {
                        crate::table::AlignX::End => AlignX::Right,
                        crate::table::AlignX::Center => AlignX::Center,
                        crate::table::AlignX::Start => AlignX::Left,
                    };
                    row_glyphs.extend(text_ctx.shape_line(
                        &cell_text,
                        cell_r,
                        13.0,
                        400,
                        align_x,
                        AlignY::Middle,
                    ));
                    col_x += col.width;
                }
            }
            row_glyph_vecs.push(row_glyphs);
        }
        let rows: Vec<TableRow<'_>> = row_glyph_vecs
            .iter()
            .enumerate()
            .map(|(index, g)| TableRow { index, glyphs: g })
            .collect();
        emit_table(node, &desc, state, &rows, &header_glyphs, control)
    }
}

impl UiWidget for BadgeDesc {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.text, 11.5, 500);
        let dot_extra = if self.dot { 12.0 } else { 0.0 };
        Vec2::new(
            (measured.x + 18.0 + dot_extra)
                .max(self.rect.width())
                .max(22.0),
            self.rect.height().max(22.0),
        )
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        if desc.radius <= 0.0 {
            desc.radius = rect.size.y * 0.5;
        }
        let text_r = if desc.dot {
            Rect::new(
                rect.origin.x + 10.0,
                rect.origin.y,
                (rect.width() - 12.0).max(0.0),
                rect.height(),
            )
        } else {
            rect
        };
        let glyphs = text_ctx.shape_line(
            &desc.text,
            text_r,
            11.5,
            500,
            AlignX::Center,
            AlignY::Middle,
        );
        emit_badge(&desc, &glyphs)
    }
}

impl UiWidget for DividerDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        match self.orientation {
            DividerOrientation::Horizontal => Vec2::new(
                self.rect.width().max(64.0),
                self.rect.height().max(self.thickness.max(1.0)),
            ),
            DividerOrientation::Vertical => Vec2::new(
                self.rect.width().max(self.thickness.max(1.0)),
                self.rect.height().max(24.0),
            ),
        }
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let (glyphs, w) = if let Some(lbl) = &desc.label {
            let m = text_ctx.measure_line(lbl, 12.0, 400);
            let g = text_ctx.shape_line(lbl, rect, 12.0, 400, AlignX::Center, AlignY::Middle);
            (g, m.x)
        } else {
            (Vec::new(), 0.0)
        };
        emit_divider(&desc, &glyphs, w, DividerTokens::standard())
    }
}

impl UiWidget for AlertDesc {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let t = text_ctx.measure_line(&self.title, 14.0, 600);
        let m = text_ctx.measure_line(&self.message, 13.0, 400);
        Vec2::new(
            (t.x.max(m.x) + 64.0).max(self.rect.width()).max(220.0),
            self.rect
                .height()
                .max(if self.message.is_empty() { 44.0 } else { 64.0 }),
        )
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let content_r = desc.content_rect();
        let mut glyphs = text_ctx.shape_line(
            &desc.title,
            Rect::new(
                content_r.origin.x,
                content_r.origin.y,
                content_r.width(),
                18.0,
            ),
            13.5,
            600,
            AlignX::Left,
            AlignY::Middle,
        );
        if !desc.message.is_empty() {
            let msg_r = Rect::new(
                content_r.origin.x,
                content_r.origin.y + 20.0,
                content_r.width(),
                (content_r.height() - 20.0).max(14.0),
            );
            glyphs.extend(text_ctx.shape_line(
                &desc.message,
                msg_r,
                12.0,
                400,
                AlignX::Left,
                AlignY::Top,
            ));
        }
        emit_alert(node, &desc, &glyphs, control)
    }
}

impl UiWidget for ProgressDesc {
    type State = ProgressState;

    fn initial_state(&self) -> Self::State {
        let mut st = ProgressState::new();
        if let Some(v) = self.value {
            st.jump_to(v);
        }
        st
    }

    fn sync_state(&self, state: &mut Self::State) {
        if let Some(v) = self.value {
            state.jump_to(v);
        }
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(160.0), self.rect.height().max(12.0))
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_progress(&desc, state)
    }
}

impl UiWidget for CardDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(160.0), self.rect.height().max(96.0))
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_card(&desc, CardTokens::standard())
    }
}

impl UiWidget for DataGridDesc {
    type State = DataGridState;

    fn initial_state(&self) -> Self::State {
        DataGridState {
            active_cell: self.active_cell,
            editing: self.editing,
            edit_buffer: String::new(),
        }
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(280.0), self.rect.height().max(160.0))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                let mut desc = self.clone();
                desc.rect = rect;
                for r in 0..desc.visible_rows() {
                    for c in 0..desc.visible_cols() {
                        if desc.cell_rect(r, c).contains_point(*pos) {
                            state.select_cell(r + desc.row_offset, c);
                            return true;
                        }
                    }
                }
                false
            }
            WidgetInputEvent::KeyDown(chord) => {
                let rows = self.visible_rows().max(1);
                let cols = self.visible_cols().max(1);
                match chord.key {
                    KeyName::Named("ArrowUp") => {
                        state.move_cell(-1, 0, rows, cols);
                        true
                    }
                    KeyName::Named("ArrowDown") => {
                        state.move_cell(1, 0, rows, cols);
                        true
                    }
                    KeyName::Named("ArrowLeft") => {
                        state.move_cell(0, -1, rows, cols);
                        true
                    }
                    KeyName::Named("ArrowRight") => {
                        state.move_cell(0, 1, rows, cols);
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.active_cell = state.active_cell;
        desc.editing = state.editing;
        let rows = desc.visible_rows();
        let cols = desc.visible_cols();
        let mut glyphs = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let cell_r = desc.cell_rect(r, c);
                let text = desc
                    .cells
                    .get(r)
                    .and_then(|row| row.get(c))
                    .cloned()
                    .unwrap_or_else(|| {
                        if r == 0 {
                            desc.columns
                                .get(c)
                                .map(|col| col.title.clone())
                                .unwrap_or_else(|| format!("Col {}", (b'A' + c as u8) as char))
                        } else {
                            format!("R{}C{}", r + desc.row_offset, c + 1)
                        }
                    });
                let weight = if r == 0 { 600 } else { 400 };
                glyphs.extend(text_ctx.shape_line(
                    &text,
                    cell_r,
                    12.5,
                    weight,
                    AlignX::Center,
                    AlignY::Middle,
                ));
            }
        }
        emit_data_grid(node, &desc, control, &glyphs)
    }
}

impl UiWidget for CodeEditorDesc {
    type State = CodeEditorState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(320.0), self.rect.height().max(180.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let gutter_w = 36.0_f32.min(rect.width() * 0.2);
        let line_h = 20.0_f32;
        let max_lines = ((rect.height() - 12.0) / line_h).floor().max(1.0) as usize;
        let mut glyphs = Vec::new();
        let source_lines: Vec<&str> = if !desc.lines.is_empty() {
            desc.lines.iter().map(String::as_str).collect()
        } else if !state.source.is_empty() {
            state.source.lines().collect()
        } else {
            vec![
                "fn render_frame(ctx: &mut FrameCtx) {",
                "    ctx.submit_indirect_draws();",
                "}",
            ]
        };
        for (i, line) in source_lines.into_iter().take(max_lines).enumerate() {
            let y = rect.origin.y + 6.0 + i as f32 * line_h;
            let num_r = Rect::new(rect.origin.x + 4.0, y, (gutter_w - 8.0).max(8.0), line_h);
            let code_r = Rect::new(
                rect.origin.x + gutter_w + 8.0,
                y,
                (rect.width() - gutter_w - 16.0).max(16.0),
                line_h,
            );
            glyphs.extend(text_ctx.shape_line(
                &format!("{}", i + 1),
                num_r,
                12.0,
                400,
                AlignX::Right,
                AlignY::Middle,
            ));
            glyphs.extend(text_ctx.shape_line(
                line,
                code_r,
                12.5,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_code_editor(node, &desc, control, &glyphs)
    }
}

impl UiWidget for PaginationDesc {
    type State = PaginationState;

    fn initial_state(&self) -> Self::State {
        PaginationState::new(self.current_page)
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(260.0), self.rect.height().max(36.0))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if let WidgetInputEvent::PointerDown { pos, .. } = event {
            let total_slots = 9usize;
            let gap = 4.0;
            let pad = 4.0;
            let slot_w = ((rect.width() - pad * 2.0 - gap * (total_slots - 1) as f32)
                / total_slots as f32)
                .max(18.0);
            let slot_h = (rect.height() - pad * 2.0).max(18.0);
            let mut desc = self.clone();
            desc.rect = rect;
            if state.page >= 1 {
                desc.current_page = state.page;
            }
            let slots = desc.visible_slots();
            for idx in 0..total_slots {
                let r = Rect::new(
                    rect.origin.x + pad + idx as f32 * (slot_w + gap),
                    rect.origin.y + pad,
                    slot_w,
                    slot_h,
                );
                if r.contains_point(*pos) {
                    if idx == 0 {
                        state.page = desc.current_page.saturating_sub(1).max(1);
                        return true;
                    } else if idx == total_slots - 1 {
                        state.page = (desc.current_page + 1).min(desc.total_pages.max(1));
                        return true;
                    } else if let Some(p) = slots[idx - 1] {
                        state.page = p;
                        return true;
                    }
                }
            }
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        if state.page >= 1 {
            desc.current_page = state.page;
        }
        let slots = desc.visible_slots();
        let total_slots = 9usize;
        let gap = 4.0;
        let pad = 4.0;
        let slot_w = ((rect.width() - pad * 2.0 - gap * (total_slots - 1) as f32)
            / total_slots as f32)
            .max(18.0);
        let slot_h = (rect.height() - pad * 2.0).max(18.0);
        let mut glyphs = Vec::new();
        for idx in 0..total_slots {
            let r = Rect::new(
                rect.origin.x + pad + idx as f32 * (slot_w + gap),
                rect.origin.y + pad,
                slot_w,
                slot_h,
            );
            let txt = if idx == 0 {
                "<".to_string()
            } else if idx == total_slots - 1 {
                ">".to_string()
            } else if let Some(p) = slots[idx - 1] {
                p.to_string()
            } else {
                "...".to_string()
            };
            glyphs.extend(text_ctx.shape_line(&txt, r, 12.0, 500, AlignX::Center, AlignY::Middle));
        }
        emit_pagination(node, &desc, control, &glyphs)
    }
}

impl UiWidget for TransferDesc {
    type State = TransferState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(320.0), self.rect.height().max(180.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let (right_btn, left_btn) = desc.action_button_rects();
        let mut glyphs =
            text_ctx.shape_line(">", right_btn, 12.0, 600, AlignX::Center, AlignY::Middle);
        glyphs.extend(text_ctx.shape_line(
            "<",
            left_btn,
            12.0,
            600,
            AlignX::Center,
            AlignY::Middle,
        ));
        let pad = 6.0;
        let mid_w = 28.0;
        let pane_w = ((rect.width() - pad * 4.0 - mid_w) * 0.5).max(24.0);
        let left_hdr = Rect::new(
            rect.origin.x + pad + 8.0,
            rect.origin.y + pad + 4.0,
            (pane_w - 16.0).max(16.0),
            20.0,
        );
        let right_hdr = Rect::new(
            rect.origin.x + pad * 3.0 + pane_w + mid_w + 8.0,
            rect.origin.y + pad + 4.0,
            (pane_w - 16.0).max(16.0),
            20.0,
        );
        glyphs.extend(text_ctx.shape_line(
            &format!("Source ({})", desc.source_count),
            left_hdr,
            12.5,
            500,
            AlignX::Left,
            AlignY::Middle,
        ));
        glyphs.extend(text_ctx.shape_line(
            &format!("Target ({})", desc.target_count),
            right_hdr,
            12.5,
            500,
            AlignX::Left,
            AlignY::Middle,
        ));
        emit_transfer(node, &desc, control, &glyphs)
    }
}

impl UiWidget for PopconfirmDesc {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(&self.title, 13.0, 500);
        Vec2::new(
            (m.x + 32.0).max(self.rect.width()).max(180.0),
            self.rect.height().max(84.0),
        )
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let title_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 10.0,
            (rect.width() - 24.0).max(0.0),
            22.0,
        );
        let mut glyphs = text_ctx.shape_line(
            &desc.title,
            title_r,
            13.0,
            500,
            AlignX::Left,
            AlignY::Middle,
        );
        let btn_w = 56.0_f32.min((rect.width() - 24.0) * 0.45);
        let btn_h = 22.0_f32.min(rect.height() * 0.4);
        let btn_y = rect.max().y - btn_h - 8.0;
        let confirm_r = Rect::new(rect.max().x - btn_w - 8.0, btn_y, btn_w, btn_h);
        let cancel_r = Rect::new(confirm_r.origin.x - btn_w - 6.0, btn_y, btn_w, btn_h);
        glyphs.extend(text_ctx.shape_line(
            &desc.cancel_label,
            cancel_r,
            11.5,
            500,
            AlignX::Center,
            AlignY::Middle,
        ));
        glyphs.extend(text_ctx.shape_line(
            &desc.confirm_label,
            confirm_r,
            11.5,
            500,
            AlignX::Center,
            AlignY::Middle,
        ));
        emit_popconfirm(node, &desc, control, &glyphs)
    }
}

impl UiWidget for TourDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(320.0), self.rect.height().max(180.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        if desc.target.is_empty() {
            desc.target = Rect::new(rect.origin.x + 16.0, rect.origin.y + 12.0, 96.0, 28.0);
        }
        let bubble_y = (desc.target.max().y + 6.0).min(rect.max().y - 54.0);
        let title_r = Rect::new(desc.target.origin.x + 10.0, bubble_y + 6.0, 160.0, 20.0);
        let glyphs = text_ctx.shape_line(
            &desc.title,
            title_r,
            12.5,
            600,
            AlignX::Left,
            AlignY::Middle,
        );
        emit_tour(node, &desc, control, &glyphs)
    }
}

impl UiWidget for TimelineTrackDesc {
    type State = TimelineTrackState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(300.0), self.rect.height().max(96.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let pad = 6.0;
        let lane_h = ((rect.height() - pad * 4.0) / 3.0).max(14.0);
        let mut glyphs = Vec::new();
        for (i, label) in ["Transform", "Opacity", "Camera"].iter().enumerate() {
            let lane_r = Rect::new(
                rect.origin.x + pad + 8.0,
                rect.origin.y + pad + i as f32 * (lane_h + pad),
                120.0,
                lane_h,
            );
            glyphs.extend(text_ctx.shape_line(
                label,
                lane_r,
                11.5,
                500,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_timeline_track(node, &desc, control, &glyphs)
    }
}

impl UiWidget for SplitButtonDesc {
    type State = SplitButtonState;

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(&self.label, 14.0, 500);
        Vec2::new((m.x + DROPDOWN_WIDTH + 32.0).max(120.0), 36.0)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if matches!(event, WidgetInputEvent::PointerDown { .. }) {
            state.menu_open = !state.menu_open;
            return true;
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let action_w = (rect.width() - DROPDOWN_WIDTH).max(0.0);
        let action_r = Rect::new(rect.origin.x, rect.origin.y, action_w, rect.height());
        let content_r = Rect::new(
            action_r.origin.x + 11.0,
            action_r.origin.y,
            (action_r.width() - 11.0).max(0.0),
            action_r.height(),
        );
        let mut glyphs = text_ctx.shape_line(
            &desc.label,
            content_r,
            14.0,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        glyphs.retain(|quad| quad.rect.origin.x < action_r.max().x - 13.0);
        emit_split_button_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for ToggleButtonDesc {
    type State = ToggleButtonState;

    fn initial_state(&self) -> Self::State {
        ToggleButtonState::new(self.checked)
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(&self.label, 14.0, 500);
        Vec2::new((m.x + 28.0).max(64.0), 36.0)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if matches!(event, WidgetInputEvent::PointerDown { .. }) {
            state.toggle();
            return true;
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let glyphs =
            text_ctx.shape_line(&desc.label, rect, 14.0, 500, AlignX::Center, AlignY::Middle);
        emit_toggle_button_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for TreeTableDesc {
    type State = TreeTableState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(400.0), self.rect.height().max(180.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let mut glyphs = Vec::new();
        let header_h = TREE_TABLE_HEADER_HEIGHT.min(rect.height());
        let mut col_x = rect.origin.x;
        for col in &desc.columns {
            let hdr_r = Rect::new(
                col_x + 12.0,
                rect.origin.y,
                (col.width - 16.0).max(0.0),
                header_h,
            );
            glyphs.extend(text_ctx.shape_line(
                &col.title,
                hdr_r,
                12.5,
                600,
                AlignX::Left,
                AlignY::Middle,
            ));
            col_x += col.width;
        }
        let body_h = (rect.height() - header_h).max(0.0);
        let row_h = TREE_TABLE_ROW_HEIGHT.min(body_h / 4.0);
        for (r_idx, row) in desc.rows.iter().take(4).enumerate() {
            let row_y = rect.origin.y + header_h + r_idx as f32 * row_h;
            let mut cx = rect.origin.x;
            for (c_idx, cell) in row.cells.iter().enumerate() {
                let col_w = desc.columns.get(c_idx).map_or(140.0, |c| c.width);
                let indent = if c_idx == 0 {
                    22.0 + row.depth as f32 * 14.0
                } else {
                    12.0
                };
                let cell_r = Rect::new(cx + indent, row_y, (col_w - indent - 8.0).max(0.0), row_h);
                glyphs.extend(text_ctx.shape_line(
                    cell,
                    cell_r,
                    12.5,
                    400,
                    AlignX::Left,
                    AlignY::Middle,
                ));
                cx += col_w;
            }
        }
        emit_tree_table_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for MultiSplitDesc {
    type State = MultiSplitState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(360.0), self.rect.height().max(180.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_multi_split(node, &desc, state, control)
    }
}

impl UiWidget for DescriptionsDesc {
    type State = DescriptionsState;

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(&self.title, 16.0, 600);
        Vec2::new(
            (m.x + 120.0).max(360.0),
            (32.0 * (self.items.len().max(2) as f32) + 40.0).max(120.0),
        )
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let row_h = DESCRIPTION_ROW_HEIGHT.min((rect.height() / 4.0).max(1.0));
        let title_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y,
            (rect.width() - 24.0).max(0.0),
            row_h,
        );
        let mut glyphs = text_ctx.shape_line(
            &desc.title,
            title_r,
            13.5,
            600,
            AlignX::Left,
            AlignY::Middle,
        );
        for (idx, item) in desc.items.iter().take(3).enumerate() {
            let y = rect.origin.y + (idx + 1) as f32 * row_h;
            let key_r = Rect::new(rect.origin.x + 12.0, y, rect.width() * 0.4, row_h);
            let val_r = Rect::new(
                rect.origin.x + rect.width() * 0.42,
                y,
                (rect.width() * 0.58 - 12.0).max(0.0),
                row_h,
            );
            glyphs.extend(text_ctx.shape_line(
                &item.label,
                key_r,
                12.5,
                500,
                AlignX::Left,
                AlignY::Middle,
            ));
            glyphs.extend(text_ctx.shape_line(
                &item.value,
                val_r,
                12.5,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_descriptions_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for CalloutDesc {
    type State = CalloutState;

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m1 = text_ctx.measure_line(&self.title, 14.0, 600);
        let m2 = text_ctx.measure_line(&self.message, 13.0, 400);
        Vec2::new(m1.x.max(m2.x).max(280.0) + 60.0, 64.0)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if matches!(event, WidgetInputEvent::PointerDown { .. }) {
            state.closed = !state.closed;
            return true;
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let title_r = Rect::new(
            rect.origin.x + 16.0,
            rect.origin.y + 8.0,
            (rect.width() - 48.0).max(0.0),
            20.0,
        );
        let mut glyphs = text_ctx.shape_line(
            &desc.title,
            title_r,
            13.5,
            600,
            AlignX::Left,
            AlignY::Middle,
        );
        if !desc.message.is_empty() {
            let msg_r = Rect::new(
                rect.origin.x + 16.0,
                rect.origin.y + 30.0,
                (rect.width() - 48.0).max(0.0),
                (rect.height() - 36.0).max(16.0),
            );
            glyphs.extend(text_ctx.shape_line(
                &desc.message,
                msg_r,
                12.0,
                400,
                AlignX::Left,
                AlignY::Top,
            ));
        }
        emit_callout_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for VirtualMasonryDesc {
    type State = VirtualMasonryState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(400.0), self.rect.height().max(240.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_virtual_masonry(node, &desc, state, control)
    }
}

impl UiWidget for TerminalViewDesc {
    type State = TerminalViewState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(400.0), self.rect.height().max(200.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let gutter_w = 40.0_f32.min(rect.width() * 0.25);
        let line_h = TERMINAL_LINE_HEIGHT.min((rect.height() - 8.0).max(4.0));
        let max_lines = ((rect.height() - 12.0) / line_h).floor().max(1.0) as usize;
        let lines: Vec<&str> = if desc.lines.is_empty() {
            vec![
                "$ cargo test --workspace",
                "test result: ok. 142 passed; 0 failed",
            ]
        } else {
            desc.lines.iter().map(String::as_str).collect()
        };
        let mut glyphs = Vec::new();
        for (idx, line) in lines.into_iter().take(max_lines).enumerate() {
            let y = rect.origin.y + 8.0 + idx as f32 * line_h;
            let num_r = Rect::new(rect.origin.x + 4.0, y, (gutter_w - 8.0).max(8.0), line_h);
            let txt_r = Rect::new(
                rect.origin.x + gutter_w + 8.0,
                y,
                (rect.width() - gutter_w - 16.0).max(16.0),
                line_h,
            );
            glyphs.extend(text_ctx.shape_line(
                &format!("{}", idx + 1),
                num_r,
                12.0,
                400,
                AlignX::Right,
                AlignY::Middle,
            ));
            glyphs.extend(text_ctx.shape_line(
                line,
                txt_r,
                12.5,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_terminal_view_with_glyphs(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for TooltipDesc {
    type State = TooltipState;

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        if self.label.is_empty() {
            return Vec2::new(self.size.x.max(64.0), self.size.y.max(28.0));
        }
        let m = text_ctx.measure_line(&self.label, 12.0, 400);
        Vec2::new(
            (m.x + self.padding.x * 2.0)
                .max(self.size.x)
                .min(self.max_width.max(64.0)),
            (m.y + self.padding.y * 2.0).max(self.size.y).max(28.0),
        )
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        if desc.anchor.is_empty() {
            desc.anchor = rect;
        }
        if !desc.label.is_empty() {
            let m = text_ctx.measure_line(&desc.label, 12.0, 400);
            desc.size = Vec2::new(
                (m.x + desc.padding.x * 2.0)
                    .max(desc.size.x)
                    .min(desc.max_width.max(64.0)),
                (m.y + desc.padding.y * 2.0).max(desc.size.y).max(28.0),
            );
        }
        let text_r = desc.content_rect();
        let glyphs = text_ctx.shape_line(
            &desc.label,
            text_r,
            12.0,
            400,
            AlignX::Center,
            AlignY::Middle,
        );
        desc.render_with_glyphs(node, 0, control, &glyphs).0
    }
}

impl UiWidget for AddressBarDesc {
    type State = AddressBarState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(
            self.rect.width().max(260.0),
            self.rect
                .height()
                .max(crate::address_bar::ADDRESS_BAR_HEIGHT),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let mut desc = self.clone();
        desc.rect = rect;
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => state.handle_click(&desc, *pos),
            WidgetInputEvent::KeyDown(chord) => state.handle_key(&desc, *chord),
            WidgetInputEvent::TextInput(text) => {
                state
                    .handle_text_event(&desc, &TextInputEvent::Insert(text.clone()))
                    .changed
            }
            WidgetInputEvent::ImePreedit { text, cursor } => {
                state
                    .handle_text_event(
                        &desc,
                        &TextInputEvent::ImePreedit {
                            text: text.clone(),
                            cursor: *cursor,
                        },
                    )
                    .changed
            }
            WidgetInputEvent::ImeCommit(text) => {
                state
                    .handle_text_event(&desc, &TextInputEvent::ImeCommit { text: text.clone() })
                    .changed
            }
            WidgetInputEvent::ImeCancel => {
                state
                    .handle_text_event(&desc, &TextInputEvent::ImeCancel)
                    .changed
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let layout = crate::address_bar::compute_layout(&desc);
        let mut primary_glyphs = Vec::new();
        let mut placeholder_glyphs = Vec::new();
        let mut visual = AddressBarTextVisual::default();
        if state.editing {
            let edit_text = if state.edit_core.text().is_empty() && !state.edit_buffer.is_empty() {
                state.edit_buffer.as_str()
            } else {
                state.edit_core.text()
            };
            let text_x = layout.root_icon_rect.max().x + 6.0;
            let edit_w = (layout.address_rect.max().x - text_x - 8.0).max(0.0);
            let edit_r = Rect::new(
                text_x,
                layout.address_rect.origin.y,
                edit_w,
                layout.address_rect.size.y.max(0.0),
            );
            // 活动编辑态保留未省略原文并按 `scroll_x` 平移起点，确保字形与光标/选区坐标严格一致。
            let scroll_x = state.edit_core.scroll_x();
            let scrolled_edit_r = Rect::new(
                edit_r.origin.x - scroll_x,
                edit_r.origin.y,
                edit_r.size.x + scroll_x.max(0.0),
                edit_r.size.y,
            );
            if edit_w > 0.0 && !edit_text.is_empty() {
                primary_glyphs.extend(text_ctx.shape_line(
                    edit_text,
                    scrolled_edit_r,
                    12.5,
                    400,
                    AlignX::Left,
                    AlignY::Middle,
                ));
            }
            visual.edit_visual = compute_text_edit_visual(
                &state.edit_core,
                edit_r,
                !control.disabled,
                |candidate| text_ctx.measure_line(candidate, 12.5, 400).x,
            );
        } else {
            for seg_layout in &layout.segments {
                if let Some(seg) = desc.segments.get(seg_layout.index) {
                    let avail_w = (seg_layout.label_rect.width() - 8.0).max(0.0);
                    if avail_w > 0.0 && !seg.label.is_empty() {
                        let elided = crate::elided_text::elide_text_with_measure(
                            &seg.label,
                            avail_w,
                            crate::elided_text::ElideMode::End,
                            |candidate| text_ctx.measure_line(candidate, 12.0, 400).x,
                        );
                        if !elided.display.is_empty() {
                            primary_glyphs.extend(text_ctx.shape_line(
                                &elided.display,
                                seg_layout.label_rect,
                                12.0,
                                400,
                                AlignX::Center,
                                AlignY::Middle,
                            ));
                        }
                    }
                }
            }
        }
        if let Some(sr) = layout.search_rect {
            if !layout.search_collapsed {
                let search_text = state.search_core.text();
                let is_placeholder = search_text.is_empty();
                let display = if is_placeholder {
                    desc.search_placeholder.as_str()
                } else {
                    search_text
                };
                let left_x = layout
                    .search_icon_rect
                    .map_or(sr.origin.x + 8.0, |ir| ir.max().x + 6.0);
                let show_clear = !control.disabled && !is_placeholder;
                let right_x = if show_clear {
                    layout
                        .search_clear_rect
                        .map_or(sr.max().x - 6.0, |cr| cr.origin.x - 6.0)
                } else {
                    sr.max().x - 6.0
                };
                let search_w = (right_x - left_x).max(0.0);
                let text_r = Rect::new(left_x, sr.origin.y, search_w, sr.size.y.max(0.0));
                if search_w > 0.0 && !display.is_empty() {
                    if state.search_focused && !is_placeholder {
                        // 搜索聚焦输入态保留未省略原文并按 `scroll_x` 平移，避免与光标/选区错位。
                        let scroll_x = state.search_core.scroll_x();
                        let scrolled_r = Rect::new(
                            text_r.origin.x - scroll_x,
                            text_r.origin.y,
                            text_r.size.x + scroll_x.max(0.0),
                            text_r.size.y,
                        );
                        primary_glyphs.extend(text_ctx.shape_line(
                            search_text,
                            scrolled_r,
                            12.0,
                            400,
                            AlignX::Left,
                            AlignY::Middle,
                        ));
                    } else {
                        let elided = crate::elided_text::elide_text_with_measure(
                            display,
                            search_w,
                            crate::elided_text::ElideMode::End,
                            |candidate| text_ctx.measure_line(candidate, 12.0, 400).x,
                        );
                        if !elided.display.is_empty() {
                            let shaped = text_ctx.shape_line(
                                &elided.display,
                                text_r,
                                12.0,
                                400,
                                AlignX::Left,
                                AlignY::Middle,
                            );
                            if is_placeholder {
                                placeholder_glyphs.extend(shaped);
                            } else {
                                primary_glyphs.extend(shaped);
                            }
                        }
                    }
                }
                if state.search_focused {
                    visual.search_visual = compute_text_edit_visual(
                        &state.search_core,
                        text_r,
                        !control.disabled,
                        |candidate| text_ctx.measure_line(candidate, 12.0, 400).x,
                    );
                }
            }
        }
        emit_address_bar_with_visual(
            node,
            &desc,
            state,
            control,
            &primary_glyphs,
            &placeholder_glyphs,
            &visual,
        )
    }
}

impl UiWidget for QueryBuilderDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let h = 48.0 + self.rules.len().max(1) as f32 * 34.0;
        Vec2::new(self.rect.width().max(280.0), self.rect.height().max(h))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let comb_r = Rect::new(rect.origin.x + 12.0, rect.origin.y + 10.0, 56.0, 24.0);
        let comb_txt = match desc.combinator {
            crate::query_builder::QueryCombinator::And => "AND",
            crate::query_builder::QueryCombinator::Or => "OR",
        };
        let mut glyphs =
            text_ctx.shape_line(comb_txt, comb_r, 11.5, 500, AlignX::Center, AlignY::Middle);
        for (idx, rule) in desc.rules.iter().enumerate() {
            let y = rect.origin.y + 42.0 + idx as f32 * 34.0;
            let row_r = Rect::new(
                rect.origin.x + 18.0,
                y,
                (rect.size.x - 36.0).max(40.0),
                28.0,
            );
            let txt = format!("{}  {}  {}", rule.field, rule.operator, rule.value);
            glyphs.extend(text_ctx.shape_line(
                &txt,
                row_r,
                12.0,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_query_builder(node, &desc, control, &glyphs)
    }
}

impl UiWidget for ShortcutBarDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(260.0), self.rect.height().max(32.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let mut glyphs = Vec::new();
        let mut x = rect.origin.x + 10.0;
        for entry in &desc.entries {
            let keycap_w = (entry.chord.len() as f32 * 7.0 + 12.0).max(24.0);
            let keycap_r = Rect::new(
                x,
                rect.origin.y + 4.0,
                keycap_w,
                (rect.size.y - 8.0).max(16.0),
            );
            glyphs.extend(text_ctx.shape_line(
                &entry.chord,
                keycap_r,
                11.0,
                500,
                AlignX::Center,
                AlignY::Middle,
            ));
            let lbl_r = Rect::new(
                x + keycap_w + 6.0,
                rect.origin.y + 4.0,
                96.0,
                (rect.size.y - 8.0).max(16.0),
            );
            glyphs.extend(text_ctx.shape_line(
                &entry.label,
                lbl_r,
                11.5,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
            x += keycap_w + (entry.label.len() as f32 * 7.0) + 20.0;
        }
        emit_shortcut_bar(node, &desc, control, &glyphs)
    }
}

impl UiWidget for PipelineViewDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(280.0), self.rect.height().max(72.0))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let count = desc.stages.len().max(1) as f32;
        let gap = 8.0;
        let stage_w = ((rect.size.x - 24.0 - (count - 1.0) * gap) / count).max(32.0);
        let mut glyphs = Vec::new();
        for (idx, stg) in desc.stages.iter().enumerate() {
            let sx = rect.origin.x + 12.0 + idx as f32 * (stage_w + gap);
            let label_r = Rect::new(
                sx + 6.0,
                rect.origin.y + 14.0,
                (stage_w - 12.0).max(16.0),
                20.0,
            );
            let txt = format!("{} {:.2}ms", stg.name, stg.duration_ms);
            glyphs.extend(text_ctx.shape_line(
                &txt,
                label_r,
                11.5,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_pipeline_view(node, &desc, control, &glyphs)
    }
}

impl UiWidget for TaskHudDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let h = 20.0 + self.tasks.len().max(1) as f32 * 38.0;
        Vec2::new(self.rect.width().max(260.0), self.rect.height().max(h))
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let mut glyphs = Vec::new();
        for (idx, task) in desc.tasks.iter().enumerate() {
            let y = rect.origin.y + 10.0 + idx as f32 * 38.0;
            let row_r = Rect::new(
                rect.origin.x + 12.0,
                y,
                (rect.size.x - 40.0).max(40.0),
                18.0,
            );
            let txt = format!("{} ({:.0}%)", task.title, task.progress * 100.0);
            glyphs.extend(text_ctx.shape_line(
                &txt,
                row_r,
                11.5,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        emit_task_hud(node, &desc, control, &glyphs)
    }
}

impl UiWidget for SchemaFormDesc {
    type State = SchemaFormState;

    fn initial_state(&self) -> Self::State {
        SchemaFormState::from_schema(&self.fields)
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let h = 20.0 + self.fields.len().max(1) as f32 * self.row_height;
        Vec2::new(self.rect.width().max(280.0), self.rect.height().max(h))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if let WidgetInputEvent::PointerDown { pos, .. } = event {
            let rows = self.layout_rows_in(rect);
            let Some(hit) = self.hit_test_in(rect, *pos) else {
                return false;
            };
            match hit {
                SchemaFormEvent::ResetDot(idx) => {
                    let Some(field) = self.fields.get(idx) else {
                        return false;
                    };
                    state.reset_field(field);
                    state.record_event(SchemaFormEvent::ResetDot(idx));
                    return true;
                }
                SchemaFormEvent::TrailingAction(idx) => {
                    let Some(field) = self.fields.get(idx) else {
                        return false;
                    };
                    if !state.is_field_enabled(field) {
                        return false;
                    }
                    state.record_event(SchemaFormEvent::TrailingAction(idx));
                    return true;
                }
                SchemaFormEvent::ControlClicked(idx) => {
                    let Some(field) = self.fields.get(idx) else {
                        return false;
                    };
                    if !state.is_field_enabled(field) {
                        return false;
                    }
                    let Some(row) = rows.get(idx) else {
                        return false;
                    };
                    let current = state
                        .get(&field.key)
                        .cloned()
                        .unwrap_or_else(|| field.default_value.clone());
                    match (&field.kind, current) {
                        (
                            crate::schema_form::SchemaFieldKind::Switch,
                            crate::schema_form::SchemaValue::Bool(b),
                        ) => {
                            state.set(field.key.clone(), crate::schema_form::SchemaValue::Bool(!b));
                            state.record_event(SchemaFormEvent::ControlClicked(idx));
                            return true;
                        }
                        (
                            crate::schema_form::SchemaFieldKind::Select { options },
                            crate::schema_form::SchemaValue::EnumIndex(cur_i),
                        ) => {
                            if !options.is_empty() {
                                let next = (cur_i + 1) % options.len();
                                state.set(
                                    field.key.clone(),
                                    crate::schema_form::SchemaValue::EnumIndex(next),
                                );
                                state.record_event(SchemaFormEvent::ControlClicked(idx));
                                return true;
                            }
                        }
                        (
                            crate::schema_form::SchemaFieldKind::Number { min, max, step, .. },
                            crate::schema_form::SchemaValue::Number(cur_v),
                        ) => {
                            let ctrl_mid = row.control_rect.center().x;
                            let delta = if pos.x >= ctrl_mid { *step } else { -*step };
                            let next = (cur_v + delta).clamp(*min, *max);
                            state.set(
                                field.key.clone(),
                                crate::schema_form::SchemaValue::Number(next),
                            );
                            state.record_event(SchemaFormEvent::ControlClicked(idx));
                            return true;
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let rows = desc.layout_rows();
        let mut glyphs = Vec::new();
        for (row, field) in rows.iter().zip(desc.fields.iter()) {
            let full_label = field.full_label_text();
            let elided = crate::elided_text::elide_text_with_measure(
                &full_label,
                row.label_rect.size.x,
                crate::elided_text::ElideMode::End,
                |sample| text_ctx.measure_line(sample, LABEL_FONT_SIZE, 400).x,
            );
            glyphs.extend(text_ctx.shape_line(
                &elided.display,
                row.label_rect,
                LABEL_FONT_SIZE,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));

            let val_rect = Rect::new(
                row.control_rect.origin.x + 8.0,
                row.control_rect.origin.y,
                (row.control_rect.size.x - 16.0).max(16.0),
                row.control_rect.size.y,
            );
            let val = state.get(&field.key).unwrap_or(&field.default_value);
            let val_txt = val.display_text(&field.kind);
            let elided_val = crate::elided_text::elide_text_with_measure(
                &val_txt,
                val_rect.size.x,
                crate::elided_text::ElideMode::End,
                |sample| text_ctx.measure_line(sample, LABEL_FONT_SIZE, 400).x,
            );
            glyphs.extend(text_ctx.shape_line(
                &elided_val.display,
                val_rect,
                LABEL_FONT_SIZE,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));

            if let (Some(action_r), Some(action_label)) = (
                row.trailing_action_rect,
                field.trailing_action_label.as_ref(),
            ) {
                let action_text_rect = Rect::new(
                    action_r.origin.x + 4.0,
                    action_r.origin.y,
                    (action_r.size.x - 8.0).max(16.0),
                    action_r.size.y,
                );
                let elided_action = crate::elided_text::elide_text_with_measure(
                    action_label,
                    action_text_rect.size.x,
                    crate::elided_text::ElideMode::End,
                    |sample| text_ctx.measure_line(sample, LABEL_FONT_SIZE, 500).x,
                );
                glyphs.extend(text_ctx.shape_line(
                    &elided_action.display,
                    action_text_rect,
                    LABEL_FONT_SIZE,
                    500,
                    AlignX::Center,
                    AlignY::Middle,
                ));
            }
        }
        emit_schema_form(node, &desc, state, control, &glyphs)
    }
}

impl UiWidget for GenericDataTableDesc {
    type State = GenericDataTableState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let h = 36.0
            + self.visible_row_slots as f32 * self.row_height
            + if self.show_summary_footer { 28.0 } else { 0.0 };
        Vec2::new(self.rect.width().max(300.0), self.rect.height().max(h))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let visible_len = state.resolve_visible_indices(&self.rows).len();
        let col_widths = self.effective_column_widths(rect.width());
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                let header_bottom = rect.origin.y + 33.0;
                if pos.y <= header_bottom {
                    let mut col_x = rect.origin.x;
                    for (idx, col) in self.columns.iter().enumerate() {
                        let col_w = col_widths.get(idx).copied().unwrap_or(col.width);
                        if pos.x >= col_x && pos.x < col_x + col_w {
                            if col.sortable {
                                state.cycle_sort(idx);
                                return true;
                            }
                            break;
                        }
                        col_x += col_w;
                    }
                    return false;
                }
                let body_y = pos.y - header_bottom;
                let slot_idx = (body_y / self.row_height.max(1.0)).floor() as usize;
                if slot_idx < self.visible_row_slots {
                    let first_row =
                        state.first_row_index(self.row_height, visible_len, self.visible_row_slots);
                    let logical_idx = first_row + slot_idx;
                    if logical_idx < visible_len {
                        state.focused_row = Some(logical_idx);
                        state.selected_rows = vec![logical_idx];
                        return true;
                    }
                }
                false
            }
            WidgetInputEvent::Wheel { delta } => {
                if visible_len <= self.visible_row_slots {
                    return false;
                }
                let max_scroll =
                    (visible_len.saturating_sub(self.visible_row_slots) as f32) * self.row_height;
                let next =
                    (state.scroll_offset_y - delta.y * self.row_height).clamp(0.0, max_scroll);
                if (next - state.scroll_offset_y).abs() > 0.01 {
                    state.scroll_offset_y = next;
                    return true;
                }
                false
            }
            WidgetInputEvent::KeyDown(chord) => match chord.key {
                KeyName::Named("ArrowUp") => {
                    state.move_focus(visible_len, -1);
                    if let Some(focused) = state.focused_row {
                        let first = state.first_row_index(
                            self.row_height,
                            visible_len,
                            self.visible_row_slots,
                        );
                        if focused < first {
                            state.scroll_offset_y = focused as f32 * self.row_height;
                        }
                    }
                    true
                }
                KeyName::Named("ArrowDown") => {
                    state.move_focus(visible_len, 1);
                    if let Some(focused) = state.focused_row {
                        let first = state.first_row_index(
                            self.row_height,
                            visible_len,
                            self.visible_row_slots,
                        );
                        if focused >= first + self.visible_row_slots {
                            state.scroll_offset_y =
                                (focused + 1 - self.visible_row_slots) as f32 * self.row_height;
                        }
                    }
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let col_widths = desc.effective_column_widths(rect.width());
        let mut glyphs = Vec::new();
        let mut hx = rect.origin.x + 10.0;
        for (col_idx, col) in desc.columns.iter().enumerate() {
            let col_w = col_widths.get(col_idx).copied().unwrap_or(col.width);
            let hr = Rect::new(hx, rect.origin.y + 2.0, (col_w - 12.0).max(24.0), 28.0);
            let title_txt = match state.sort_column {
                Some((idx, crate::generic_data_table::SortDirection::Ascending))
                    if idx == col_idx =>
                {
                    format!("{} ▲", col.title)
                }
                Some((idx, crate::generic_data_table::SortDirection::Descending))
                    if idx == col_idx =>
                {
                    format!("{} ▼", col.title)
                }
                _ => col.title.clone(),
            };
            glyphs.extend(text_ctx.shape_line(
                &title_txt,
                hr,
                12.0,
                600,
                AlignX::Left,
                AlignY::Middle,
            ));
            hx += col_w;
        }

        let visible_indices = state.resolve_visible_indices(&desc.rows);
        let first_row = state.first_row_index(
            desc.row_height,
            visible_indices.len(),
            desc.visible_row_slots,
        );
        let header_bottom = rect.origin.y + 33.0;
        for slot_idx in 0..desc.visible_row_slots {
            let Some(&row_idx) = visible_indices.get(first_row + slot_idx) else {
                break;
            };
            let Some(row_cells) = desc.rows.get(row_idx) else {
                continue;
            };
            let ry = header_bottom + slot_idx as f32 * desc.row_height;
            let mut cx = rect.origin.x + 10.0;
            for (c_idx, col) in desc.columns.iter().enumerate() {
                let col_w = col_widths.get(c_idx).copied().unwrap_or(col.width);
                if let Some(cell_text) = row_cells.get(c_idx) {
                    let cell_r = Rect::new(cx, ry, (col_w - 14.0).max(16.0), desc.row_height);
                    glyphs.extend(text_ctx.shape_line(
                        cell_text,
                        cell_r,
                        12.0,
                        400,
                        AlignX::Left,
                        AlignY::Middle,
                    ));
                }
                cx += col_w;
            }
        }

        if desc.show_summary_footer {
            let footer_r = Rect::new(
                rect.origin.x + 12.0,
                rect.max().y - 27.0,
                (rect.size.x - 24.0).max(24.0),
                25.0,
            );
            let summary = if !desc.summary_text.is_empty() {
                desc.summary_text.clone()
            } else {
                format!(
                    "Showing {} of {} records",
                    visible_indices.len().min(desc.visible_row_slots),
                    desc.rows.len()
                )
            };
            glyphs.extend(text_ctx.shape_line(
                &summary,
                footer_r,
                11.5,
                500,
                AlignX::Left,
                AlignY::Middle,
            ));
        }

        emit_generic_data_table(node, &desc, state, control, &glyphs)
    }
}

// ============================================================================
// 扩展声明式控件：HotkeyInput / ColorPicker / CodeBlock / NumberInput /
// Combobox / AutoComplete / Tabs / Chart / SplitPane
// ============================================================================

impl crate::hotkey_input::HotkeyInputDesc {
    /// 创建无 Rect 的快捷键录制器描述符。
    #[must_use]
    pub fn shortcut(shortcut_text: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, shortcut_text)
    }

    /// 设置录制状态。
    #[must_use]
    pub fn recording(mut self, recording: bool) -> Self {
        self.recording = recording;
        self
    }

    /// 设置冲突告警状态。
    #[must_use]
    pub fn conflict(mut self, conflict: bool) -> Self {
        self.conflict = conflict;
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

impl UiWidget for crate::hotkey_input::HotkeyInputDesc {
    type State = crate::hotkey_input::HotkeyInputState;

    fn initial_state(&self) -> Self::State {
        crate::hotkey_input::HotkeyInputState {
            chord: None,
            recording: self.recording,
            conflict: self.conflict,
        }
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(&self.shortcut_text, 12.5, 500);
        Vec2::new(
            (m.x + 48.0).max(self.rect.width()).max(148.0),
            self.rect.height().max(32.0),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                let clear_r = Rect::new(
                    rect.max().x - 20.0,
                    rect.origin.y + (rect.height() - 16.0) * 0.5,
                    16.0,
                    16.0,
                );
                if clear_r.contains_point(*pos) && state.chord.is_some() {
                    state.clear();
                    return true;
                }
                state.recording = !state.recording;
                true
            }
            WidgetInputEvent::KeyDown(chord) if state.recording => {
                if matches!(chord.key, KeyName::Named("Escape"))
                    && !chord.ctrl
                    && !chord.alt
                    && !chord.shift
                {
                    state.recording = false;
                    return true;
                }
                state.chord = Some(*chord);
                state.recording = false;
                true
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        if let Some(chord_text) = state.formatted_chord() {
            desc.shortcut_text = chord_text;
        }
        desc.recording = state.recording;
        desc.conflict = desc.conflict || state.conflict;
        let display = if desc.recording {
            "Press a shortcut...".to_owned()
        } else if desc.shortcut_text.is_empty() {
            "Not bound".to_owned()
        } else {
            desc.shortcut_text.clone()
        };
        let text_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 4.0,
            (rect.width() - 36.0).max(16.0),
            (rect.height() - 8.0).max(12.0),
        );
        let glyphs =
            text_ctx.shape_line(&display, text_r, 12.0, 500, AlignX::Center, AlignY::Middle);
        crate::hotkey_input::emit_hotkey_input(node, &desc, control, &glyphs)
    }
}

impl crate::color_swatch::ColorSwatchDesc {
    /// 创建无 Rect 的颜色色块描述符。
    #[must_use]
    pub fn swatch(fill: unbound_theme::Token, label: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, fill, label)
    }
}

impl UiWidget for crate::color_swatch::ColorSwatchDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(
            self.rect.width().max(crate::color_swatch::SWATCH_SIZE),
            self.rect.height().max(crate::color_swatch::SWATCH_SIZE),
        )
    }

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        matches!(event, WidgetInputEvent::PointerDown { .. })
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        crate::color_swatch::emit_color_swatch(node, &desc, control)
    }
}

impl crate::color_picker::ColorPickerDesc {
    /// 创建无 Rect 的颜色选择器描述符。
    #[must_use]
    pub fn picker() -> Self {
        Self::new(Rect::ZERO)
    }
    /// 设置颜色文本显示格式。
    #[must_use]
    pub fn with_format(mut self, format: crate::color_picker::ColorFormat) -> Self {
        self.format = format;
        self
    }
}

impl UiWidget for crate::color_picker::ColorPickerDesc {
    type State = crate::color_picker::ColorPickerState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(220.0), self.rect.height().max(180.0))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let mut desc = self.clone();
        desc.rect = rect;
        let layout = desc.layout();
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                if layout.sv.contains_point(*pos) {
                    state.begin(crate::color_picker::ColorPart::Sv);
                    state.set_from_pointer(
                        crate::color_picker::ColorPart::Sv,
                        pos.x,
                        pos.y,
                        layout.sv,
                    );
                    return true;
                }
                if layout.hue.contains_point(*pos) {
                    state.begin(crate::color_picker::ColorPart::Hue);
                    state.set_from_pointer(
                        crate::color_picker::ColorPart::Hue,
                        pos.x,
                        pos.y,
                        layout.hue,
                    );
                    return true;
                }
                if layout.alpha.contains_point(*pos) {
                    state.begin(crate::color_picker::ColorPart::Alpha);
                    state.set_from_pointer(
                        crate::color_picker::ColorPart::Alpha,
                        pos.x,
                        pos.y,
                        layout.alpha,
                    );
                    return true;
                }
                false
            }
            WidgetInputEvent::PointerDrag { pos } => {
                if let Some(part) = state.active() {
                    let target_r = match part {
                        crate::color_picker::ColorPart::Sv => layout.sv,
                        crate::color_picker::ColorPart::Hue => layout.hue,
                        crate::color_picker::ColorPart::Alpha => layout.alpha,
                    };
                    state.set_from_pointer(part, pos.x, pos.y, target_r);
                    return true;
                }
                false
            }
            WidgetInputEvent::PointerUp { .. } => {
                if state.active().is_some() {
                    state.end();
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let txt = desc.format.format(state.hsva());
        let label_r = Rect::new(
            rect.origin.x + 12.0,
            rect.max().y - 26.0,
            (rect.width() - 24.0).max(24.0),
            20.0,
        );
        let mut glyphs =
            text_ctx.shape_line(&txt, label_r, 11.5, 500, AlignX::Right, AlignY::Middle);
        crate::color_picker::emit_color_picker(node, &desc, state, control, &mut glyphs)
    }
}

impl crate::code_block::CodeBlockDesc {
    /// 创建无 Rect 的只读语法代码块描述符。
    #[must_use]
    pub fn code(text: impl Into<String>, language: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, text, language)
    }

    /// 设置高亮行号列表（1-based）。
    #[must_use]
    pub fn with_highlight_lines(mut self, lines: Vec<usize>) -> Self {
        self.highlight_lines = lines;
        self
    }

    /// 设置是否显示行号栏。
    #[must_use]
    pub fn with_line_numbers(mut self, show_numbers: bool) -> Self {
        self.show_numbers = show_numbers;
        self
    }
}

impl UiWidget for crate::code_block::CodeBlockDesc {
    type State = crate::code_block::CodeBlockState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let lines = self.line_count().clamp(2, 32) as f32;
        Vec2::new(
            self.rect.width().max(320.0),
            (lines * crate::code_block::CODE_LINE_HEIGHT + 12.0)
                .max(self.rect.height())
                .max(48.0),
        )
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let max_lines = (rect.height() / crate::code_block::CODE_LINE_HEIGHT)
            .floor()
            .max(1.0) as usize;
        let mut glyphs = Vec::new();
        let gutter_w = desc.gutter_width();
        for (idx, line) in desc.text.lines().take(max_lines).enumerate() {
            let line_r = desc.line_rect(idx);
            if desc.show_numbers {
                let num_r = Rect::new(
                    rect.origin.x + 4.0,
                    line_r.origin.y,
                    (gutter_w - 10.0).max(8.0),
                    line_r.height(),
                );
                glyphs.extend(text_ctx.shape_line(
                    &format!("{}", desc.start_line + idx),
                    num_r,
                    11.5,
                    400,
                    AlignX::Right,
                    AlignY::Middle,
                ));
            }
            let code_r = Rect::new(
                rect.origin.x + gutter_w + 10.0,
                line_r.origin.y,
                (rect.width() - gutter_w - 18.0).max(16.0),
                line_r.height(),
            );
            glyphs.extend(text_ctx.shape_line(
                line,
                code_r,
                12.0,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
        }
        crate::code_block::emit_code_block(node, &desc, state, &glyphs, control)
    }
}

impl crate::number_input::NumberInputDesc {
    /// 创建无 Rect 的数字输入框描述符。
    #[must_use]
    pub fn number(min: f64, max: f64) -> Self {
        Self::new(Rect::ZERO, min, max)
    }

    /// 设置微调步长。
    #[must_use]
    pub fn with_step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// 设置小数精度。
    #[must_use]
    pub fn with_precision(mut self, precision: u8) -> Self {
        self.precision = precision;
        self
    }

    /// 设置单位后缀。
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }
}

impl UiWidget for crate::number_input::NumberInputDesc {
    type State = crate::number_input::NumberInputState;

    fn initial_state(&self) -> Self::State {
        crate::number_input::NumberInputState::new(self.min.max(0.0).min(self.max))
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(120.0), self.rect.height().max(32.0))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let mut desc = self.clone();
        desc.rect = rect;
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                if crate::number_input::spin_up_rect(rect).contains_point(*pos) {
                    return state.step_by(1, &desc);
                }
                if crate::number_input::spin_down_rect(rect).contains_point(*pos) {
                    return state.step_by(-1, &desc);
                }
                state.begin_drag(pos.x);
                true
            }
            WidgetInputEvent::PointerDrag { pos } => state.drag_to(pos.x, false, false, &desc),
            WidgetInputEvent::PointerUp { .. } => {
                if state.is_dragging() {
                    state.end_drag();
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let text_r = crate::number_input::number_text_rect(rect);
        let display = state.display_text(&desc);
        let mut glyphs =
            text_ctx.shape_line(&display, text_r, 13.0, 500, AlignX::Left, AlignY::Middle);
        crate::number_input::emit_number_input(node, &desc, state, control, &mut glyphs)
    }
}

impl crate::combobox::ComboboxDesc {
    /// 创建无 Rect 的可搜索组合框描述符。
    #[must_use]
    pub fn options(items: Vec<crate::combobox::ComboItem>) -> Self {
        Self::new(Rect::ZERO, items)
    }

    /// 设置占位文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
}

impl UiWidget for crate::combobox::ComboboxDesc {
    type State = crate::combobox::ComboboxState;

    fn initial_state(&self) -> Self::State {
        crate::combobox::ComboboxState::new(self)
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(180.0), self.rect.height().max(34.0))
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { .. } => {
                if state.is_open() {
                    state.close();
                } else {
                    state.open(self);
                }
                true
            }
            WidgetInputEvent::KeyDown(chord) => match chord.key {
                KeyName::Named("ArrowDown") => {
                    state.key(self, crate::combobox::ComboboxKey::Down).handled
                }
                KeyName::Named("ArrowUp") => {
                    state.key(self, crate::combobox::ComboboxKey::Up).handled
                }
                KeyName::Named("Enter") => {
                    state.key(self, crate::combobox::ComboboxKey::Enter).handled
                }
                KeyName::Named("Escape") => {
                    state
                        .key(self, crate::combobox::ComboboxKey::Escape)
                        .handled
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let display = {
            let txt = state.display_text(&desc);
            if txt.is_empty() {
                desc.placeholder.clone()
            } else {
                txt
            }
        };
        let text_r = Rect::new(
            rect.origin.x + 10.0,
            rect.origin.y + 2.0,
            (rect.width() - 30.0).max(16.0),
            (rect.height() - 4.0).max(16.0),
        );
        let mut glyphs =
            text_ctx.shape_line(&display, text_r, 13.0, 400, AlignX::Left, AlignY::Middle);
        crate::combobox::emit_combobox_trigger(node, &desc, state, control, &mut glyphs)
    }
}

impl crate::autocomplete::AutoCompleteDesc {
    /// 创建无 Rect 的自动补全输入框描述符。
    #[must_use]
    pub fn input(value: impl Into<String>, suggestions: Vec<String>) -> Self {
        let count = suggestions.len();
        Self::new(Rect::ZERO, value, count).with_suggestions(suggestions, false)
    }
}

impl UiWidget for crate::autocomplete::AutoCompleteDesc {
    type State = crate::autocomplete::AutoCompleteState;

    fn initial_state(&self) -> Self::State {
        crate::autocomplete::AutoCompleteState {
            query: self.value.clone(),
            highlighted: 0,
            open: self.open,
        }
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        // AutoSuggestBox 与 NumberBox / TextBox 共用 32px 标准行高。
        Vec2::new(
            self.rect.width().max(180.0),
            self.rect
                .height()
                .max(unbound_theme::typography::geometry::INPUT_HEIGHT),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let filtered: Vec<String> = state
            .filter(&self.suggestions)
            .into_iter()
            .map(str::to_owned)
            .collect();
        let visible_count = filtered.len().min(6);
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                if state.open && visible_count > 0 {
                    let row_h = 26.0;
                    let popup_r = Rect::new(
                        rect.origin.x,
                        rect.max().y + 4.0,
                        rect.width(),
                        visible_count as f32 * row_h + 8.0,
                    );
                    if popup_r.contains_point(*pos) {
                        let idx =
                            ((pos.y - (popup_r.origin.y + 4.0)).max(0.0) / row_h).floor() as usize;
                        if let Some(chosen) = filtered.get(idx) {
                            state.query = chosen.clone();
                            state.highlighted = idx;
                            state.open = false;
                            return true;
                        }
                    }
                }
                state.open = !state.open;
                true
            }
            WidgetInputEvent::TextInput(txt) => {
                state.query.push_str(txt);
                state.highlighted = 0;
                state.open = true;
                true
            }
            WidgetInputEvent::KeyDown(chord) => match chord.key {
                KeyName::Named("Backspace") => {
                    state.query.pop();
                    state.highlighted = 0;
                    state.open = true;
                    true
                }
                KeyName::Named("ArrowDown") => {
                    if !state.open {
                        state.open = true;
                    } else if visible_count > 0 {
                        state.highlighted = (state.highlighted + 1) % visible_count;
                    }
                    true
                }
                KeyName::Named("ArrowUp") => {
                    if !state.open {
                        state.open = true;
                    } else if visible_count > 0 {
                        state.highlighted = (state.highlighted + visible_count - 1) % visible_count;
                    }
                    true
                }
                KeyName::Named("Enter") => {
                    if state.open {
                        if let Some(chosen) = filtered.get(state.highlighted) {
                            state.query = chosen.clone();
                        }
                        state.open = false;
                        return true;
                    }
                    false
                }
                KeyName::Named("Escape") => {
                    state.open = false;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        desc.value = state.query.clone();
        let filtered: Vec<String> = state
            .filter(&self.suggestions)
            .into_iter()
            .map(str::to_owned)
            .collect();
        desc.suggestion_count = filtered.len();
        desc.suggestions = filtered;
        desc.open = desc.open || state.open;
        let text_r = Rect::new(
            rect.origin.x + 10.0,
            rect.origin.y + 2.0,
            (rect.width() - 32.0).max(16.0),
            (rect.height() - 4.0).max(16.0),
        );
        let glyphs =
            text_ctx.shape_line(&desc.value, text_r, 13.0, 400, AlignX::Left, AlignY::Middle);
        let mut suggestion_glyphs = Vec::new();
        if desc.open && !control.disabled {
            let count = desc.suggestions.len().min(6);
            let row_h = 26.0;
            let popup_y = rect.max().y + 4.0;
            for (idx, item) in desc.suggestions.iter().take(count).enumerate() {
                let item_r = Rect::new(
                    rect.origin.x + 10.0,
                    popup_y + 4.0 + idx as f32 * row_h,
                    (rect.width() - 20.0).max(16.0),
                    row_h - 2.0,
                );
                suggestion_glyphs.extend(text_ctx.shape_line(
                    item,
                    item_r,
                    12.5,
                    400,
                    AlignX::Left,
                    AlignY::Middle,
                ));
            }
        }
        crate::autocomplete::emit_autocomplete_with_suggestions(
            node,
            &desc,
            control,
            Some(state.highlighted),
            &glyphs,
            &suggestion_glyphs,
        )
    }
}

impl crate::tabs::TabsDesc {
    /// 创建无 Rect 的标签页栏描述符。
    #[must_use]
    pub fn titles(titles: &[&str]) -> Self {
        Self::new(Rect::ZERO, titles)
    }
}

impl UiWidget for crate::tabs::TabsDesc {
    type State = crate::tabs::TabsState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let tab_w = self.effective_tab_width();
        let add_w = if self.addable {
            match self.style {
                crate::tabs::TabViewStyle::Standard => crate::tabs::TAB_STRIP_HEIGHT,
                crate::tabs::TabViewStyle::ShellChrome => {
                    crate::tabs::TAB_ADD_GAP + crate::tabs::TAB_ADD_WIDTH
                }
            }
        } else {
            0.0
        };
        let w = (self.count() as f32 * tab_w + add_w)
            .max(self.rect.width())
            .max(200.0);
        Vec2::new(w, self.rect.height().max(crate::tabs::TAB_STRIP_HEIGHT))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if let WidgetInputEvent::PointerDown { pos, .. } = event {
            let mut desc = self.clone();
            desc.rect = rect;
            if desc.hit_test_close(state.scroll_x, *pos).is_some()
                || desc.hit_test_add(state.scroll_x, *pos)
            {
                return true;
            }
            let rel_x = pos.x - rect.origin.x + state.scroll_x;
            let tab_w = desc.effective_tab_width().max(1.0);
            if rel_x >= 0.0 && pos.x <= rect.origin.x + desc.strip_width() {
                let idx = (rel_x / tab_w).floor() as usize;
                if idx < desc.count() {
                    state.activate(&desc, idx);
                    state.snap(&desc);
                    return true;
                }
            }
        }
        false
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let idle_sub = ControlState {
            disabled: control.disabled,
            ..ControlState::default()
        };
        crate::tabs::emit_tabs_with_control_states(
            node,
            &desc,
            state,
            text_ctx,
            |_idx| control,
            |_idx| idle_sub,
            idle_sub,
        )
    }
}

impl crate::chart::ChartDesc {
    /// 创建无 Rect 的折线/柱状/面积图表描述符。
    #[must_use]
    pub fn plot(series: Vec<crate::chart::ChartSeries>) -> Self {
        Self::new(Rect::ZERO).with_series(series)
    }

    /// 显式指定 X/Y 轴数据范围。
    #[must_use]
    pub fn with_ranges(mut self, x_range: (f32, f32), y_range: (f32, f32)) -> Self {
        self.x_range = x_range;
        self.y_range = y_range;
        self
    }
}

impl UiWidget for crate::chart::ChartDesc {
    type State = crate::chart::ChartState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(320.0), self.rect.height().max(180.0))
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { pos, .. } | WidgetInputEvent::PointerDrag { pos } => {
                state.hover(*pos);
                true
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let mut tooltip_glyphs = Vec::new();
        if let Some(ptr) = state.pointer {
            if let Some((s_idx, p_idx)) = desc.nearest(ptr) {
                let pt = desc.series[s_idx].points[p_idx];
                let mapped = desc.map_point(pt);
                let plot = desc.plot_rect();
                let bubble = Rect::new(
                    (mapped.x + 14.0)
                        .min(plot.max().x - crate::chart::TOOLTIP_SIZE.0)
                        .max(plot.origin.x),
                    (mapped.y - crate::chart::TOOLTIP_SIZE.1 - 8.0).max(plot.origin.y),
                    crate::chart::TOOLTIP_SIZE.0 - 8.0,
                    crate::chart::TOOLTIP_SIZE.1,
                );
                tooltip_glyphs = text_ctx.shape_line(
                    &format!("({:.2}, {:.2})", pt.x, pt.y),
                    bubble,
                    11.5,
                    500,
                    AlignX::Center,
                    AlignY::Middle,
                );
            }
        }
        crate::chart::emit_chart(node, &desc, state, &tooltip_glyphs, control).output
    }
}

impl crate::split_pane::SplitPaneDesc {
    /// 创建无 Rect 的左右分割面板描述符。
    #[must_use]
    pub fn horizontal_pane() -> Self {
        Self::horizontal(Rect::ZERO)
    }

    /// 创建无 Rect 的上下分割面板描述符。
    #[must_use]
    pub fn vertical_pane() -> Self {
        Self::vertical(Rect::ZERO)
    }
}

impl UiWidget for crate::split_pane::SplitPaneDesc {
    type State = crate::split_pane::SplitPaneState;

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(240.0), self.rect.height().max(160.0))
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        let mut desc = self.clone();
        desc.rect = rect;
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                state.begin_drag(&desc, *pos);
                true
            }
            WidgetInputEvent::PointerDrag { pos } => {
                if state.dragging {
                    state.drag_to(&desc, *pos);
                    return true;
                }
                false
            }
            WidgetInputEvent::PointerUp { .. } => {
                if state.dragging {
                    state.end_drag();
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        crate::split_pane::emit_split_pane(
            node,
            &desc,
            state,
            control,
            crate::split_pane::SplitPaneTokens::standard(),
        )
    }
}

impl UiWidget for WidgetCatalogId {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(self.module_name(), 13.0, 500);
        Vec2::new((m.x + 48.0).max(220.0), 64.0)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        emit_widget_with_shaper(
            *self,
            node,
            control,
            rect,
            CatalogParams::default(),
            |text, origin, size| {
                let line_r = Rect::new(origin.x, origin.y, 400.0, (size * 1.4).max(16.0));
                text_ctx.shape_line(text, line_r, size, 400, AlignX::Left, AlignY::Top)
            },
        )
    }
}

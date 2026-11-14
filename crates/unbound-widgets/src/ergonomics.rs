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

use crate::alert::{emit_alert, AlertDesc, Severity};
use crate::badge::{emit_badge, BadgeDesc};
use crate::button::{emit_button, ButtonDesc, ButtonSize, ButtonVariant};
use crate::callout::{emit_callout, CalloutDesc, CalloutState};
use crate::catalog::emit_widget;
use crate::checkbox::{emit_checkbox, CheckState, CheckboxDesc, CheckboxTokens};
use crate::code_editor::{emit_code_editor, CodeEditorDesc, CodeEditorState};
use crate::containers::{emit_card, CardDesc, CardTokens};
use crate::data_grid::{emit_data_grid, DataGridDesc, DataGridState};
use crate::descriptions::{emit_descriptions, DescriptionsDesc, DescriptionsState};
use crate::divider::{emit_divider, DividerDesc, DividerOrientation, DividerTokens};
use crate::multi_split::{emit_multi_split, MultiSplitDesc, MultiSplitState};
use crate::pagination::{emit_pagination, PaginationDesc, PaginationState};
use crate::popconfirm::{emit_popconfirm, PopconfirmDesc};
use crate::progress::{emit_progress, ProgressDesc, ProgressState};
use crate::select::{emit_select, SelectDesc, SelectState};
use crate::slider::{emit_slider, value_from_pointer, Orientation, SliderDesc, SliderTokens};
use crate::split_button::{emit_split_button, SplitButtonDesc, SplitButtonState, DROPDOWN_WIDTH};
use crate::switch::{emit_switch, SwitchDesc, SwitchTokens};
use crate::table::{emit_table, ColumnSpec, TableDesc, TableRow, TableState};
use crate::terminal_view::{emit_terminal_view, TerminalViewDesc, TerminalViewState};
use crate::text_input::{
    emit_text_input, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    TextInputVisual,
};
use crate::timeline_track::{emit_timeline_track, TimelineTrackDesc, TimelineTrackState};
use crate::toggle_button::{emit_toggle_button, ToggleButtonDesc, ToggleButtonState};
use crate::tour::{emit_tour, TourDesc};
use crate::transfer::{emit_transfer, TransferDesc, TransferState};
use crate::tree_table::{emit_tree_table, TreeTableDesc, TreeTableState};
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
}

impl TextCtx<'_> {
    /// 按 `size` 与 `weight` 测量单行文本。
    pub fn measure_line(&mut self, text: &str, size: f32, weight: u16) -> Vec2 {
        if text.is_empty() {
            return Vec2::new(0.0, size * 1.35);
        }
        let mut style = unbound_text::TextStyle::sized(size);
        style.weight = weight;
        let shaped = self.engine.shape(text, &style);
        Vec2::new(shaped.width, shaped.height.max(size * 1.35))
    }

    /// 在 `rect` 内对齐排版并光栅化单行文本。
    pub fn shape_line(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        weight: u16,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Vec<GlyphQuad> {
        if text.is_empty() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Vec::new();
        }
        let mut style = unbound_text::TextStyle::sized(size);
        style.weight = weight;
        let shaped = self.engine.shape(text, &style);
        let x = match align_x {
            AlignX::Left => rect.origin.x,
            AlignX::Center => rect.origin.x + ((rect.width() - shaped.width) * 0.5).max(0.0),
            AlignX::Right => (rect.max().x - shaped.width).max(rect.origin.x),
        };
        let line_h = shaped.height.max(size);
        let y = match align_y {
            AlignY::Top => rect.origin.y,
            AlignY::Middle => rect.origin.y + ((rect.height() - line_h) * 0.5).max(0.0),
            AlignY::Bottom => (rect.max().y - line_h).max(rect.origin.y),
        };
        self.cache
            .quads_for(self.engine, &shaped, Vec2::new(x, y), weight, false)
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
    /// 创建带 `label` 的无 Rect 按钮描述符。
    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, label)
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

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, self.font_size, 500);
        let w = (measured.x + 32.0).max(self.rect.width()).max(64.0);
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
        desc.centre_label = false;
        let mut glyphs = text_ctx.shape_line(
            &desc.label,
            rect,
            desc.font_size,
            500,
            AlignX::Center,
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
        let w = (self.gap + measured.x + 8.0)
            .max(self.rect.width())
            .max(self.box_size);
        let h = 24.0_f32.max(self.rect.height());
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
        let glyphs = text_ctx.shape_line(
            &desc.label,
            label_r,
            14.0,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
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
            measured.x + 12.0
        };
        let w = (self.track_size.0 + label_w).max(self.rect.width());
        let h = self.track_size.1.max(26.0).max(self.rect.height());
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
        let label_r = Rect::new(
            rect.origin.x,
            rect.origin.y,
            (rect.width() - desc.track_size.0 - 8.0).max(0.0),
            rect.height(),
        );
        let glyphs = text_ctx.shape_line(
            &desc.label,
            label_r,
            14.0,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        emit_switch(node, &desc, control, &glyphs, SwitchTokens::standard())
    }
}

impl UiWidget for SliderDesc {
    type State = f32;

    fn initial_state(&self) -> Self::State {
        self.value
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(160.0), self.rect.height().max(28.0))
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

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let sample = if !self.label.is_empty() {
            self.label.as_str()
        } else {
            self.placeholder.as_str()
        };
        let measured = text_ctx.measure_line(sample, self.font_size, 400);
        Vec2::new(
            (measured.x + 28.0).max(self.rect.width()).max(160.0),
            self.rect.height().max(34.0),
        )
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if self.disabled || self.read_only {
            return false;
        }
        match event {
            WidgetInputEvent::TextInput(text) => {
                let outcome = reduce_text_input(self, state, &TextInputEvent::Insert(text.clone()));
                outcome.changed
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
        let text_r = Rect::new(
            rect.origin.x + 10.0,
            rect.origin.y + 2.0,
            (rect.width() - 20.0).max(0.0),
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
        let caret_x = text_r.origin.x
            + glyphs
                .last()
                .map_or(0.0, |g| (g.rect.max().x - text_r.origin.x).max(0.0));
        let visual = TextInputVisual {
            caret: Rect::new(
                caret_x,
                rect.origin.y + 6.0,
                1.5,
                (rect.height() - 12.0).max(1.0),
            ),
            caret_visible: true,
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
            highlighted: None,
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
            self.rect.height().max(34.0),
        )
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        if matches!(event, WidgetInputEvent::PointerDown { .. }) {
            state.open = !state.open;
            if state.selected.is_none() && !self.options.is_empty() {
                state.selected = Some(0);
            }
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
        let display = desc.display_text(state);
        let text_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y,
            (rect.width() - 32.0).max(0.0),
            rect.height(),
        );
        let _glyphs = text_ctx.shape_line(display, text_r, 14.0, 400, AlignX::Left, AlignY::Middle);
        emit_select(node, &desc, state, control)
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
            let padded = Rect::new(
                row_r.origin.x + 8.0,
                row_r.origin.y,
                (row_r.width() - 16.0).max(0.0),
                row_r.height(),
            );
            row_glyph_vecs.push(text_ctx.shape_line(
                &format!("Row {}", r + 1),
                padded,
                13.0,
                400,
                AlignX::Left,
                AlignY::Middle,
            ));
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
        let measured = text_ctx.measure_line(&self.text, 12.0, 500);
        Vec2::new(
            (measured.x + 18.0).max(self.rect.width()).max(48.0),
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
        let glyphs =
            text_ctx.shape_line(&desc.text, rect, 12.0, 500, AlignX::Center, AlignY::Middle);
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
            self.rect.height().max(52.0),
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
        let text_r = Rect::new(
            rect.origin.x + 40.0,
            rect.origin.y + 8.0,
            (rect.width() - 64.0).max(0.0),
            (rect.height() - 16.0).max(0.0),
        );
        let glyphs = text_ctx.shape_line(&desc.title, text_r, 14.0, 600, AlignX::Left, AlignY::Top);
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

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(280.0), self.rect.height().max(160.0))
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
        let glyphs = text_ctx.shape_line(&desc.label, rect, 13.0, 500, AlignX::Left, AlignY::Top);
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
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let glyphs = text_ctx.shape_line(&desc.label, rect, 13.0, 400, AlignX::Left, AlignY::Top);
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
        let glyphs =
            text_ctx.shape_line(&desc.label, rect, 12.0, 500, AlignX::Center, AlignY::Middle);
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
        let glyphs = text_ctx.shape_line(&desc.label, rect, 13.0, 400, AlignX::Left, AlignY::Top);
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
        let glyphs = text_ctx.shape_line(&desc.title, rect, 13.0, 500, AlignX::Left, AlignY::Top);
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
        let glyphs = text_ctx.shape_line(&desc.title, rect, 13.0, 500, AlignX::Left, AlignY::Top);
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
        let glyphs = text_ctx.shape_line(&desc.label, rect, 12.0, 400, AlignX::Left, AlignY::Top);
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
        let _glyphs = text_ctx.shape_line(
            &desc.label,
            action_r,
            14.0,
            500,
            AlignX::Center,
            AlignY::Middle,
        );
        emit_split_button(node, &desc, state, control)
    }
}

impl UiWidget for ToggleButtonDesc {
    type State = ToggleButtonState;

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
        let _glyphs =
            text_ctx.shape_line(&desc.label, rect, 14.0, 500, AlignX::Center, AlignY::Middle);
        emit_toggle_button(node, &desc, state, control)
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
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_tree_table(node, &desc, state, control)
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
        let title_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 4.0,
            (rect.width() - 24.0).max(0.0),
            24.0,
        );
        let _glyphs = text_ctx.shape_line(
            &desc.title,
            title_r,
            15.0,
            600,
            AlignX::Left,
            AlignY::Middle,
        );
        emit_descriptions(node, &desc, state, control)
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
            state.dismiss();
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
        let text_r = Rect::new(
            rect.origin.x + 16.0,
            rect.origin.y + 8.0,
            (rect.width() - 50.0).max(0.0),
            20.0,
        );
        let _glyphs =
            text_ctx.shape_line(&desc.title, text_r, 14.0, 600, AlignX::Left, AlignY::Middle);
        emit_callout(node, &desc, state, control)
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
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_terminal_view(node, &desc, state, control)
    }
}

impl UiWidget for WidgetCatalogId {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(self.module_name(), 13.0, 500);
        Vec2::new((m.x + 48.0).max(180.0), 44.0)
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut glyphs = text_ctx.shape_line(
            self.module_name(),
            rect,
            13.0,
            500,
            AlignX::Left,
            AlignY::Middle,
        );
        emit_widget(*self, node, control, rect, &mut glyphs)
    }
}

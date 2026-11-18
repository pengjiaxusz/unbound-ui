//! DatePickerWheel：三列滚轮式日期选择（月 / 日 / 年）。
//!
//! 控件由 32px 高的触发器和其下方的滚轮面板组成：每一列上下各有一个
//! Chevron，中间一行为选中高亮。滚轮步进是纯逻辑，日期合法性
//! （每月天数、闰年）复用 [`crate::date_picker`] 的日历规则。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::date_picker::days_in_month;
use crate::primitive::{
    curve_stroke, focus_ring, glyphs as emit_glyphs, instance, outlined, surface,
};
use crate::switch::CONTROL_LAYER;
use crate::{
    AlignX, AlignY, ControlState, PointerPhase, TextCtx, UiWidget, WidgetInputEvent, WidgetOutput,
};

/// 英文月份全称清单。
pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// 触发器高度。对齐 WinUI `DatePicker` 的 32px 控件高度。
pub const DATE_PICKER_WHEEL_TRIGGER_HEIGHT: f32 = 32.0;
/// 滚轮每行的高度。
pub const DATE_PICKER_WHEEL_ROW_HEIGHT: f32 = 32.0;
/// 滚轮每列可见行数（奇数，保证中间行是选中行）。
pub const DATE_PICKER_WHEEL_ROWS: usize = 5;
/// 滚轮面板高度：可见行 + 上下内边距。
pub const DATE_PICKER_WHEEL_PANEL_HEIGHT: f32 =
    DATE_PICKER_WHEEL_ROW_HEIGHT * DATE_PICKER_WHEEL_ROWS as f32 + 16.0;
/// 面板圆角半径。对齐 WinUI `OverlayCornerRadius = 8`。
pub const DATE_PICKER_WHEEL_RADIUS: f32 = 8.0;
/// 面板阴影模糊半径。
pub const DATE_PICKER_WHEEL_SHADOW: f32 = 24.0;
/// 列数。
pub const DATE_PICKER_WHEEL_COLUMNS: usize = 3;
/// 年份下限。
pub const DATE_PICKER_WHEEL_MIN_YEAR: i32 = 1900;
/// 年份上限。
pub const DATE_PICKER_WHEEL_MAX_YEAR: i32 = 2100;

/// 滚轮的三列。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelColumn {
    /// 月列。
    Month,
    /// 日列。
    Day,
    /// 年列。
    Year,
}

impl WheelColumn {
    /// 三列按显示顺序排列。
    pub const ALL: [Self; DATE_PICKER_WHEEL_COLUMNS] = [Self::Month, Self::Day, Self::Year];

    /// 列索引（0 = 月、1 = 日、2 = 年）。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Month => 0,
            Self::Day => 1,
            Self::Year => 2,
        }
    }

    /// 列的英文标签。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Month => "Month",
            Self::Day => "Day",
            Self::Year => "Year",
        }
    }
}

/// 三列滚轮描述符。`rect` 是 32px 高的触发器，滚轮面板绘制在其正下方。
#[derive(Clone, Debug, PartialEq)]
pub struct DatePickerWheelDesc {
    /// 触发器矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl DatePickerWheelDesc {
    /// 使用默认度量创建滚轮日期选择器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            disabled: false,
            z: 710,
        }
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 触发器矩形：高度收敛到标准 32px。
    #[must_use]
    pub fn trigger_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            self.rect.size.y.min(DATE_PICKER_WHEEL_TRIGGER_HEIGHT),
        )
    }

    /// 触发栏中第 `column` 列的矩形。
    #[must_use]
    pub fn trigger_column_rect(&self, column: WheelColumn) -> Rect {
        let trigger = self.trigger_rect();
        let col_w = trigger.size.x / DATE_PICKER_WHEEL_COLUMNS as f32;
        Rect::new(
            trigger.origin.x + column.index() as f32 * col_w,
            trigger.origin.y,
            col_w,
            trigger.size.y,
        )
    }

    /// 触发栏中第 `index` 条列间分割线矩形（0 = 月日间，1 = 日年间）。
    #[must_use]
    pub fn trigger_divider_rect(&self, index: usize) -> Rect {
        let trigger = self.trigger_rect();
        let col_w = trigger.size.x / DATE_PICKER_WHEEL_COLUMNS as f32;
        let x = trigger.origin.x + (index + 1) as f32 * col_w;
        Rect::new(
            x - 0.5,
            trigger.origin.y + 1.0,
            1.0,
            (trigger.size.y - 2.0).max(0.0),
        )
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置绘制深度。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 滚轮面板矩形，位于触发器正下方。
    #[must_use]
    pub fn panel_rect(&self) -> Rect {
        let trigger = self.trigger_rect();
        Rect::new(
            trigger.origin.x,
            trigger.max().y,
            trigger.size.x,
            DATE_PICKER_WHEEL_PANEL_HEIGHT,
        )
    }

    /// 第 `column` 列的矩形（覆盖可见行区域）。
    #[must_use]
    pub fn column_rect(&self, column: WheelColumn) -> Rect {
        let panel = self.panel_rect();
        let width = (panel.size.x - 16.0).max(0.0) / DATE_PICKER_WHEEL_COLUMNS as f32;
        Rect::new(
            panel.origin.x + 8.0 + column.index() as f32 * width,
            panel.origin.y + 8.0,
            width,
            DATE_PICKER_WHEEL_ROW_HEIGHT * DATE_PICKER_WHEEL_ROWS as f32,
        )
    }
}

/// 三列滚轮状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatePickerWheelState {
    /// 年。
    pub year: i32,
    /// 月（1..=12）。
    pub month: u8,
    /// 日（1..=当月天数）。
    pub day: u8,
    /// 当前聚焦的列。
    pub active_column: WheelColumn,
    /// 滚轮面板是否展开。
    pub open: bool,
}

impl Default for DatePickerWheelState {
    fn default() -> Self {
        Self::new(2024, 1, 1)
    }
}

impl DatePickerWheelState {
    /// 创建日期状态，越界值会被夹紧到合法范围。
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        let mut state = Self {
            year: year.clamp(DATE_PICKER_WHEEL_MIN_YEAR, DATE_PICKER_WHEEL_MAX_YEAR),
            month: month.clamp(1, 12),
            day: day.max(1),
            active_column: WheelColumn::Month,
            open: false,
        };
        state.clamp_day();
        state
    }

    /// 设置展开状态。
    #[must_use]
    pub fn with_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// 月份英文全称。
    #[must_use]
    pub fn month_name(&self) -> &'static str {
        let idx = (self.month as usize).saturating_sub(1).min(11);
        MONTH_NAMES[idx]
    }

    /// 切换到上一列。
    pub fn select_prev_column(&mut self) {
        self.active_column = match self.active_column {
            WheelColumn::Month => WheelColumn::Year,
            WheelColumn::Day => WheelColumn::Month,
            WheelColumn::Year => WheelColumn::Day,
        };
    }

    /// 切换到下一列。
    pub fn select_next_column(&mut self) {
        self.active_column = match self.active_column {
            WheelColumn::Month => WheelColumn::Day,
            WheelColumn::Day => WheelColumn::Year,
            WheelColumn::Year => WheelColumn::Month,
        };
    }

    /// 步进当前激活列。
    pub fn advance_active(&mut self, delta: i32) {
        self.advance(self.active_column, delta);
    }

    /// 处理键盘按键输入，若状态发生改变则返回 `true`。
    pub fn handle_key(&mut self, chord: &KeyChord) -> bool {
        match chord.key {
            KeyName::Named("ArrowLeft") => {
                self.select_prev_column();
                true
            }
            KeyName::Named("ArrowRight") => {
                self.select_next_column();
                true
            }
            KeyName::Named("ArrowUp") => {
                self.advance_active(1);
                true
            }
            KeyName::Named("ArrowDown") => {
                self.advance_active(-1);
                true
            }
            KeyName::Named("PageUp") => {
                self.advance_active(5);
                true
            }
            KeyName::Named("PageDown") => {
                self.advance_active(-5);
                true
            }
            KeyName::Named("Home") => {
                self.active_column = WheelColumn::Month;
                true
            }
            KeyName::Named("End") => {
                self.active_column = WheelColumn::Year;
                true
            }
            KeyName::Named("Enter") | KeyName::Named("Space") => {
                self.open = !self.open;
                true
            }
            KeyName::Named("Escape") if self.open => {
                self.open = false;
                true
            }
            _ => false,
        }
    }

    /// 某一列当前选中的数值。
    #[must_use]
    pub fn selected(&self, column: WheelColumn) -> i32 {
        match column {
            WheelColumn::Month => i32::from(self.month),
            WheelColumn::Day => i32::from(self.day),
            WheelColumn::Year => self.year,
        }
    }

    /// 某一列选中值的文本表示。
    #[must_use]
    pub fn selected_text(&self, column: WheelColumn) -> String {
        self.selected(column).to_string()
    }

    /// ISO 日期字符串 `YYYY-MM-DD`。
    #[must_use]
    pub fn to_iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// 按列步进 `delta` 行。
    ///
    /// 月与日循环滚动，年份夹紧到 1900..=2100；步进后若当前日超出
    /// 当月天数（例如从 1 月 31 日滚到 2 月），日会被夹到月末。
    pub fn advance(&mut self, column: WheelColumn, delta: i32) {
        match column {
            WheelColumn::Month => {
                let total = (i32::from(self.month) - 1 + delta).rem_euclid(12);
                self.month = u8::try_from(total + 1).unwrap_or(1);
            }
            WheelColumn::Day => {
                let days = i32::from(days_in_month(self.year, self.month)).max(1);
                let total = (i32::from(self.day) - 1 + delta).rem_euclid(days);
                self.day = u8::try_from(total + 1).unwrap_or(1);
            }
            WheelColumn::Year => {
                self.year = self
                    .year
                    .saturating_add(delta)
                    .clamp(DATE_PICKER_WHEEL_MIN_YEAR, DATE_PICKER_WHEEL_MAX_YEAR);
            }
        }
        self.clamp_day();
    }

    fn clamp_day(&mut self) {
        let days = days_in_month(self.year, self.month).max(1);
        self.day = self.day.clamp(1, days);
    }
}

/// 输出滚轮日期选择器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_date_picker_wheel(
    node: u64,
    desc: &DatePickerWheelDesc,
    state: &DatePickerWheelState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let trigger = desc.trigger_rect();
    let panel = desc.panel_rect();
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;

    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else {
        match control.phase {
            PointerPhase::Pressed => Token::SURFACE_SUNKEN,
            PointerPhase::Hovered => Token::CONTROL_HOVER,
            PointerPhase::Idle => Token::CONTROL_FILL,
        }
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::CONTROL_STROKE
    };
    let row_fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else {
        match control.phase {
            PointerPhase::Pressed => Token::SURFACE_SUNKEN,
            PointerPhase::Hovered => Token::CONTROL_HOVER,
            PointerPhase::Idle => Token::PRIMARY_SOFT,
        }
    };
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let chevron_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };

    let mut primitives = Vec::with_capacity(14 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len() + 8);

    // 1. 触发器：表面、描边、列间分割线与下拉 Chevron。
    primitives.push(surface(trigger, 4.0, fill, z));
    primitives.push(outlined(trigger, 4.0, Token::NONE, border, 1.0, 0.0, z + 1));
    let col_w = trigger.size.x / DATE_PICKER_WHEEL_COLUMNS as f32;
    for i in 1..DATE_PICKER_WHEEL_COLUMNS {
        let div_x = trigger.origin.x + i as f32 * col_w;
        primitives.push(surface(
            Rect::new(
                div_x - 0.5,
                trigger.origin.y + 1.0,
                1.0,
                (trigger.size.y - 2.0).max(0.0),
            ),
            0.0,
            border,
            z + 1,
        ));
    }
    let tcx = trigger.max().x - 16.0;
    let tcy = trigger.center().y;
    curve_stroke(
        [tcx - 4.0, tcy - 2.0],
        [tcx, tcy + 2.0],
        1.5,
        chevron_ink,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [tcx, tcy + 2.0],
        [tcx + 4.0, tcy - 2.0],
        1.5,
        chevron_ink,
        z + 2,
        &mut primitives,
        &mut payloads,
    );

    // 2. 展开态绘制滚轮面板：阴影、表面、描边、选中行高亮与每列上下 Chevron。
    if state.open {
        primitives.push(instance(
            panel,
            [0.0; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            DATE_PICKER_WHEEL_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            z,
        ));
        primitives.push(surface(
            panel,
            DATE_PICKER_WHEEL_RADIUS,
            Token::OVERLAY_BG,
            z,
        ));
        primitives.push(outlined(
            panel,
            DATE_PICKER_WHEEL_RADIUS,
            Token::NONE,
            Token::CARD_BORDER,
            1.0,
            0.0,
            z,
        ));

        let highlight = Rect::new(
            panel.origin.x + 8.0,
            panel.origin.y + 8.0 + DATE_PICKER_WHEEL_ROW_HEIGHT * 2.0,
            (panel.size.x - 16.0).max(0.0),
            DATE_PICKER_WHEEL_ROW_HEIGHT,
        );
        primitives.push(surface(highlight, 4.0, row_fill, z));

        for column in WheelColumn::ALL {
            let column_rect = desc.column_rect(column);
            let cx = column_rect.center().x;
            let top = column_rect.origin.y - 6.0;
            let bottom = column_rect.max().y + 6.0;
            curve_stroke(
                [cx - 4.0, top + 2.0],
                [cx, top - 2.0],
                1.5,
                chevron_ink,
                z,
                &mut primitives,
                &mut payloads,
            );
            curve_stroke(
                [cx, top - 2.0],
                [cx + 4.0, top + 2.0],
                1.5,
                chevron_ink,
                z,
                &mut primitives,
                &mut payloads,
            );
            curve_stroke(
                [cx - 4.0, bottom - 2.0],
                [cx, bottom + 2.0],
                1.5,
                chevron_ink,
                z,
                &mut primitives,
                &mut payloads,
            );
            curve_stroke(
                [cx, bottom + 2.0],
                [cx + 4.0, bottom - 2.0],
                1.5,
                chevron_ink,
                z,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    emit_glyphs(glyphs, ink, z, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        trigger,
        4.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), trigger)
                .with_shape(Shape::Rounded { radii: [4.0; 4] })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(
                Role::ComboBox,
                if desc.label.is_empty() {
                    "Date".to_owned()
                } else {
                    desc.label.clone()
                },
                trigger,
            )
            .clickable()
            .disabled(disabled)
            .expanded(state.open)
            .with_value(state.to_iso()),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        for column in WheelColumn::ALL {
            let key = node ^ (0x57_0000 + column.index() as u64);
            let hit_rect = if state.open {
                desc.column_rect(column)
            } else {
                desc.trigger_column_rect(column)
            };
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(z) + 1, hit_rect)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::SpinButton, column.label(), hit_rect)
                    .with_value(state.selected_text(column))
                    .clickable(),
                true,
            );
        }
    }

    out
}

impl UiWidget for DatePickerWheelDesc {
    type State = DatePickerWheelState;

    fn initial_state(&self) -> Self::State {
        DatePickerWheelState::new(2024, 4, 15)
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(
            self.rect.width().max(240.0),
            DATE_PICKER_WHEEL_TRIGGER_HEIGHT,
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if self.disabled {
            return false;
        }
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                let mut desc = self.clone();
                desc.rect = rect;
                for col in WheelColumn::ALL {
                    let col_r = if state.open {
                        desc.column_rect(col)
                    } else {
                        desc.trigger_column_rect(col)
                    };
                    if col_r.contains_point(*pos) {
                        state.active_column = col;
                        state.open = !state.open;
                        return true;
                    }
                }
                if desc.trigger_rect().contains_point(*pos) {
                    state.open = !state.open;
                    true
                } else {
                    false
                }
            }
            WidgetInputEvent::KeyDown(chord) => state.handle_key(chord),
            WidgetInputEvent::Wheel { delta } => {
                let step = if delta.y > 0.0 { 1 } else { -1 };
                state.advance(state.active_column, step);
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
        let mut glyphs = Vec::new();

        if state.open {
            let row_h = DATE_PICKER_WHEEL_ROW_HEIGHT;
            for column in WheelColumn::ALL {
                let column_rect = desc.column_rect(column);
                for row in 0..DATE_PICKER_WHEEL_ROWS {
                    let value = state.selected(column) + row as i32 - 2;
                    let text = match column {
                        WheelColumn::Month => {
                            let m = (value - 1).rem_euclid(12) as usize;
                            MONTH_NAMES[m].to_owned()
                        }
                        WheelColumn::Day => {
                            let days = i32::from(days_in_month(state.year, state.month)).max(1);
                            let d = (value - 1).rem_euclid(days) + 1;
                            format!("{d:02}")
                        }
                        WheelColumn::Year => value.to_string(),
                    };
                    let row_rect = Rect::new(
                        column_rect.origin.x,
                        column_rect.origin.y + row as f32 * row_h,
                        column_rect.size.x,
                        row_h,
                    );
                    glyphs.extend(text_ctx.shape_line(
                        &text,
                        row_rect,
                        13.0,
                        if row == 2 { 600 } else { 400 },
                        AlignX::Center,
                        AlignY::Middle,
                    ));
                }
            }
        } else {
            for column in WheelColumn::ALL {
                let col_r = desc.trigger_column_rect(column);
                let text = match column {
                    WheelColumn::Month => state.month_name().to_owned(),
                    WheelColumn::Day => format!("{:02}", state.day),
                    WheelColumn::Year => state.year.to_string(),
                };
                glyphs.extend(text_ctx.shape_line(
                    &text,
                    col_r,
                    13.0,
                    400,
                    AlignX::Center,
                    AlignY::Middle,
                ));
            }
        }

        emit_date_picker_wheel(node, &desc, state, control, &mut glyphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> DatePickerWheelDesc {
        DatePickerWheelDesc::new(Rect::new(10.0, 20.0, 220.0, 32.0))
    }

    #[test]
    fn the_trigger_is_exactly_32_pixels_high() {
        let desc = DatePickerWheelDesc::new(Rect::new(0.0, 0.0, 220.0, 80.0));
        assert_eq!(desc.trigger_rect().size.y, DATE_PICKER_WHEEL_TRIGGER_HEIGHT);
        assert_eq!(desc.panel_rect().origin.y, DATE_PICKER_WHEEL_TRIGGER_HEIGHT);
        assert_eq!(desc.panel_rect().size.y, DATE_PICKER_WHEEL_PANEL_HEIGHT);
    }

    #[test]
    fn the_month_column_wraps_and_clamps_the_day() {
        let mut state = DatePickerWheelState::new(2024, 1, 31);
        state.advance(WheelColumn::Month, 1);
        assert_eq!(state.month, 2);
        assert_eq!(state.day, 29, "leap February caps the day at 29");

        state.advance(WheelColumn::Month, 11);
        assert_eq!(state.month, 1);
        assert_eq!(state.day, 29);
    }

    #[test]
    fn the_day_column_cycles_within_the_month() {
        let mut state = DatePickerWheelState::new(2023, 4, 30);
        state.advance(WheelColumn::Day, 1);
        assert_eq!(state.day, 1);
        state.advance(WheelColumn::Day, -1);
        assert_eq!(state.day, 30);
    }

    #[test]
    fn the_year_column_clamps_and_revalidates_the_day() {
        let mut state = DatePickerWheelState::new(2024, 2, 29);
        state.advance(WheelColumn::Year, 1);
        assert_eq!(state.year, 2025);
        assert_eq!(state.day, 28);

        state.advance(WheelColumn::Year, 10_000);
        assert_eq!(state.year, DATE_PICKER_WHEEL_MAX_YEAR);
        state.advance(WheelColumn::Year, -10_000);
        assert_eq!(state.year, DATE_PICKER_WHEEL_MIN_YEAR);
    }

    #[test]
    fn the_emitter_exposes_three_column_sub_controls() {
        let state = DatePickerWheelState::new(2024, 3, 15);
        let out = emit_date_picker_wheel(3, &desc(), &state, ControlState::default(), &mut []);
        assert_eq!(out.sub_hits.len(), DATE_PICKER_WHEEL_COLUMNS);
        assert_eq!(out.sub_a11y.len(), DATE_PICKER_WHEEL_COLUMNS);
        assert_eq!(out.a11y.expect("reachable").value, "2024-03-15");
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let state = DatePickerWheelState::new(2024, 3, 15);
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
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
                        counts.push(
                            emit_date_picker_wheel(1, &desc(), &state, control, &mut [])
                                .primitives
                                .len(),
                        );
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }

    #[test]
    fn keyboard_and_open_navigation_works() {
        let mut state = DatePickerWheelState::new(2024, 3, 15);
        assert_eq!(state.active_column, WheelColumn::Month);

        let right = KeyChord::named("ArrowRight");
        assert!(state.handle_key(&right));
        assert_eq!(state.active_column, WheelColumn::Day);

        let up = KeyChord::named("ArrowUp");
        assert!(state.handle_key(&up));
        assert_eq!(state.day, 16);

        let enter = KeyChord::named("Enter");
        assert!(state.handle_key(&enter));
        assert!(state.open);

        let esc = KeyChord::named("Escape");
        assert!(state.handle_key(&esc));
        assert!(!state.open);
    }
}

//! DatePickerWheel：三列滚轮式日期选择（月 / 日 / 年）。
//!
//! 控件由 32px 高的触发器和其下方的滚轮面板组成：每一列上下各有一个
//! Chevron，中间一行为选中高亮。滚轮步进是纯逻辑，日期合法性
//! （每月天数、闰年）复用 [`crate::date_picker`] 的日历规则。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::date_picker::days_in_month;
use crate::primitive::{
    curve_stroke, focus_ring, glyphs as emit_glyphs, instance, outlined, surface,
};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

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
}

impl DatePickerWheelState {
    /// 创建日期状态，越界值会被夹紧到合法范围。
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        let mut state = Self {
            year: year.clamp(DATE_PICKER_WHEEL_MIN_YEAR, DATE_PICKER_WHEEL_MAX_YEAR),
            month: month.clamp(1, 12),
            day: day.max(1),
        };
        state.clamp_day();
        state
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

    // 1. 触发器：表面、描边与下拉 Chevron。
    primitives.push(surface(trigger, 4.0, fill, z));
    primitives.push(outlined(
        trigger,
        4.0,
        Token::NONE,
        border,
        1.0,
        0.0,
        z + 1,
    ));
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

    // 2. 面板：阴影、表面与描边。
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

    // 3. 中间选中行高亮。
    let highlight = Rect::new(
        panel.origin.x + 8.0,
        panel.origin.y + 8.0 + DATE_PICKER_WHEEL_ROW_HEIGHT * 2.0,
        (panel.size.x - 16.0).max(0.0),
        DATE_PICKER_WHEEL_ROW_HEIGHT,
    );
    primitives.push(surface(highlight, 4.0, row_fill, z));

    // 4. 每列上下各一枚 Chevron。
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
            .with_value(state.to_iso()),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        for column in WheelColumn::ALL {
            let key = node ^ (0x57_0000 + column.index() as u64);
            let column_rect = desc.column_rect(column);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(z), column_rect)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::SpinButton, column.label(), column_rect)
                    .with_value(state.selected_text(column))
                    .clickable(),
                true,
            );
        }
    }

    out
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
        assert_eq!(
            desc.trigger_rect().size.y,
            DATE_PICKER_WHEEL_TRIGGER_HEIGHT
        );
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
        assert!(counts.windows(2).all(|pair| pair[0] == pair[1]), "{counts:?}");
    }
}

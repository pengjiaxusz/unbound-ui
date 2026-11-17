//! TimePicker 与 DateTimePicker 步进/滚轮时分秒选择控件。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::text_input::focus_underline;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 时 / 分 / 秒列的目标宽度。
///
/// WinUI 的三列等分内容区以 64px 为设计基准；官方 `TimePickerSelectorThemeMinWidth = 80`
/// 是浮层循环列表的最小宽度，内联控件按三列压缩到 64。
pub const TIME_PICKER_COLUMN_WIDTH: f32 = 64.0;
/// 选择器行高。
///
/// 对齐 WinUI 32px 的控件标准高度（官方 `TimePickerFlyoutPresenterItemHeight = 40` 是浮层列表项）。
pub const TIME_PICKER_ROW_HEIGHT: f32 = 32.0;
/// 触发器圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const TIME_PICKER_RADIUS: f32 = 4.0;

/// 时间值 `(时, 分, 秒)`。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TimeValue {
    /// 小时 `0..=23`。
    pub hour: u8,
    /// 分钟 `0..=59`。
    pub minute: u8,
    /// 秒 `0..=59`。
    pub second: u8,
}

impl TimeValue {
    /// 创建夹紧后的时间值。
    #[must_use]
    pub fn new(hour: u8, minute: u8, second: u8) -> Self {
        Self {
            hour: hour.min(23),
            minute: minute.min(59),
            second: second.min(59),
        }
    }

    /// 格式化为 `HH:MM:SS`。
    #[must_use]
    pub fn format_hms(self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }
}

/// [`TimePickerDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimePickerState {
    /// 当前时间值。
    pub value: TimeValue,
    /// 激活列（`0 = 时`，`1 = 分`，`2 = 秒`）。
    pub active_column: usize,
}

/// 时间选择器时/分/秒列的确定性子节点 ID（`0..3`）。
#[must_use]
pub const fn time_picker_col_node(node: u64, col: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x700 + col as u64)
}

impl TimePickerState {
    /// 选中激活列（`0 = 时`，`1 = 分`，`2 = 秒`）。
    pub fn select_column(&mut self, col: usize) {
        self.active_column = col.min(2);
    }

    /// 通过步进激活列处理鼠标滚轮增量。
    pub fn handle_wheel(&mut self, delta_y: f32) {
        if delta_y > 0.0 {
            self.step_active(1);
        } else if delta_y < 0.0 {
            self.step_active(-1);
        }
    }

    /// 按 `delta`（`+1` 或 `-1`，循环回绕）步进激活列。
    pub fn step_active(&mut self, delta: i32) {
        match self.active_column {
            0 => self.value.hour = (self.value.hour as i32 + delta).rem_euclid(24) as u8,
            1 => self.value.minute = (self.value.minute as i32 + delta).rem_euclid(60) as u8,
            _ => self.value.second = (self.value.second as i32 + delta).rem_euclid(60) as u8,
        }
    }
}

/// [`TimePicker`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TimePickerDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 已选时间。
    pub value: TimeValue,
    /// 激活列索引（`0..3`）。
    pub active_column: usize,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl TimePickerDesc {
    /// 创建时间选择器描述符。
    #[must_use]
    pub fn new(rect: Rect, value: TimeValue) -> Self {
        Self {
            rect,
            value,
            active_column: 0,
            label: "Time Picker".to_owned(),
            z: 600,
        }
    }
}

/// 输出结构稳定的时间选择器（1 个背景 + 3 个时/分/秒列 + 1 个焦点强调条 + 1 个焦点环 = 6 个图元）。
#[must_use]
pub fn emit_time_picker(
    node: u64,
    desc: &TimePickerDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        TIME_PICKER_RADIUS,
        if hovered {
            Token::CONTROL_HOVER
        } else {
            Token::CONTROL_FILL
        },
        if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            // WinUI `TimePickerButtonBorderBrush = ControlElevationBorderBrush`。
            Token::CONTROL_STROKE
        },
        1.0,
        0.0,
        desc.z,
    ));

    // 槽位沿用与调用方一致的等分算法，视觉列再居中收缩到 64×32：
    // 字形仍落在槽位中心，不会因列宽变化而偏移。
    let gap = 6.0;
    let pad = 4.0;
    let slot_w = ((desc.rect.width() - pad * 2.0 - gap * 2.0) / 3.0).max(16.0);
    let slot_h = (desc.rect.height() - pad * 2.0).max(16.0);
    let col_w = TIME_PICKER_COLUMN_WIDTH.min(slot_w);
    let col_h = TIME_PICKER_ROW_HEIGHT.min(slot_h);
    let column_rect = |col: usize| {
        Rect::new(
            desc.rect.origin.x + pad + col as f32 * (slot_w + gap) + (slot_w - col_w) * 0.5,
            desc.rect.origin.y + pad + (slot_h - col_h) * 0.5,
            col_w,
            col_h,
        )
    };
    for col in 0..3 {
        let r = column_rect(col);
        let active = col == desc.active_column.min(2);
        primitives.push(outlined(
            r,
            TIME_PICKER_RADIUS,
            if active {
                Token::PRIMARY_SOFT
            } else {
                Token::SURFACE_SUNKEN
            },
            if active {
                Token::PRIMARY
            } else {
                Token::CONTROL_STROKE
            },
            1.0,
            0.0,
            desc.z + 1,
        ));
    }

    // 聚焦态底部 2px 强调条（`TextControlBorderThemeThicknessFocused = 1,1,1,2`）。
    primitives.push(focus_underline(
        desc.rect,
        TIME_PICKER_RADIUS,
        desc.z + 2,
        if state.focused { 1.0 } else { 0.0 },
    ));

    primitives.push(focus_ring(
        desc.rect,
        TIME_PICKER_RADIUS,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y =
        A11yNode::new(Role::SpinButton, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = desc.value.format_hms();

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        const COL_NAMES: [&str; 3] = ["Hour", "Minute", "Second"];
        for (col, name) in COL_NAMES.iter().enumerate() {
            let r = column_rect(col);
            let key = time_picker_col_node(node, col);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::SpinButton, (*name).to_owned(), r).clickable(),
                true,
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_picker_steps_and_wraps() {
        let mut st = TimePickerState {
            value: TimeValue::new(23, 59, 0),
            active_column: 0,
        };
        st.step_active(1);
        assert_eq!(st.value.hour, 0);
        st.active_column = 1;
        st.step_active(2);
        assert_eq!(st.value.minute, 1);
    }
}

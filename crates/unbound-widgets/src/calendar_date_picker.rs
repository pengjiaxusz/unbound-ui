//! CalendarDatePicker：收起态像输入框、右侧带日历图标的日期选择器。
//!
//! 收起时只绘制文本框与右侧的日历 glyph 触发器；点击后语义上展开一个
//! 弹出面板（[`CalendarDatePickerState::open`]）。面板内容由调用方后续补充，
//! 这里先绘制浮层外壳，保证展开状态在场景中有可见反馈。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::date_picker::NaiveDate;
use crate::primitive::{
    curve_stroke, focus_ring, glyphs as emit_glyphs, instance, outlined, surface,
};
use crate::text_input::{focus_underline, TEXT_INPUT_LAYER};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 标准高度。对齐 WinUI `TextControlThemeMinHeight = 32`。
pub const CALENDAR_DATE_PICKER_HEIGHT: f32 = 32.0;
/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const CALENDAR_DATE_PICKER_RADIUS: f32 = 4.0;
/// 默认字号。
pub const CALENDAR_DATE_PICKER_FONT_SIZE: f32 = 14.0;
/// 右侧日历图标触发器的宽度。
pub const CALENDAR_DATE_PICKER_ICON_WIDTH: f32 = 32.0;
/// 弹出面板高度（占位外壳）。
pub const CALENDAR_DATE_PICKER_PANEL_HEIGHT: f32 = 220.0;
/// 弹出面板圆角半径。对齐 WinUI `OverlayCornerRadius = 8`。
pub const CALENDAR_DATE_PICKER_PANEL_RADIUS: f32 = 8.0;
/// 弹出面板阴影模糊半径。
pub const CALENDAR_DATE_PICKER_PANEL_SHADOW: f32 = 24.0;

/// 日历日期选择器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarDatePickerDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 未选择日期时显示的占位文本。
    pub placeholder: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
    /// 圆角半径。
    pub radius: f32,
    /// 字号（供调用方塑形时使用）。
    pub font_size: f32,
}

impl CalendarDatePickerDesc {
    /// 使用默认度量创建日历日期选择器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            z: 711,
            radius: CALENDAR_DATE_PICKER_RADIUS,
            font_size: CALENDAR_DATE_PICKER_FONT_SIZE,
        }
    }

    /// 设置占位文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 右侧日历图标触发器矩形。
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        let width = CALENDAR_DATE_PICKER_ICON_WIDTH.min(self.rect.size.x);
        Rect::new(
            self.rect.max().x - width,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }

    /// 弹出面板占位外壳矩形。
    #[must_use]
    pub fn panel_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.max().y + 4.0,
            self.rect.size.x,
            CALENDAR_DATE_PICKER_PANEL_HEIGHT,
        )
    }
}

/// 日历日期选择器状态：是否展开与已选日期。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalendarDatePickerState {
    /// 弹出面板是否展开。
    pub open: bool,
    /// 已选日期。
    pub date: Option<NaiveDate>,
}

impl CalendarDatePickerState {
    /// 创建收起且未选日期的状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建已选日期的收起状态。
    #[must_use]
    pub fn with_date(date: NaiveDate) -> Self {
        Self {
            open: false,
            date: Some(date),
        }
    }

    /// 切换展开状态。
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// 展开面板。
    pub fn expand(&mut self) {
        self.open = true;
    }

    /// 收起面板。
    pub fn collapse(&mut self) {
        self.open = false;
    }

    /// 已选日期的 ISO 文本；未选择时为空串。
    #[must_use]
    pub fn selected_text(&self) -> String {
        self.date.map(NaiveDate::to_iso).unwrap_or_default()
    }
}

/// 日历图标触发器的确定性子节点键。
#[must_use]
pub const fn calendar_icon_node(node: u64) -> u64 {
    node ^ 0x4341_4C45_4E44
}

/// 输出日历日期选择器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_calendar_date_picker(
    node: u64,
    desc: &CalendarDatePickerDesc,
    state: &CalendarDatePickerState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let radii = [desc.radius; 4];
    let disabled = control.disabled || desc.disabled;
    let icon = desc.icon_rect();

    // 收起态与 TextBox 同色；展开时叠加一档悬停色，提示面板已打开。
    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == PointerPhase::Hovered || state.open {
        Token::CONTROL_HOVER
    } else {
        Token::CONTROL_FILL
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::CONTROL_STROKE
    };
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else if state.date.is_some() {
        Token::TEXT_PRIMARY
    } else {
        Token::TEXT_SECONDARY
    };
    let icon_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };

    let mut primitives = Vec::with_capacity(9 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len() + 2);

    primitives.push(surface(rect, desc.radius, fill, z));
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z + 1,
    ));
    primitives.push(focus_underline(
        rect,
        desc.radius,
        z + 1,
        if control.focused && !disabled { 1.0 } else { 0.0 },
    ));

    // 右侧日历 glyph：14×14 方框 + 顶部横条 + 两枚装订环。
    let body = 14.0;
    let body_rect = Rect::new(
        icon.center().x - body * 0.5,
        icon.center().y - body * 0.5 + 1.0,
        body,
        body,
    );
    primitives.push(outlined(
        body_rect,
        2.0,
        Token::NONE,
        icon_ink,
        1.0,
        0.0,
        z + 2,
    ));
    primitives.push(surface(
        Rect::new(
            body_rect.origin.x + 1.0,
            body_rect.origin.y + 1.0,
            body_rect.size.x - 2.0,
            3.0,
        ),
        0.0,
        icon_ink,
        z + 2,
    ));
    curve_stroke(
        [body_rect.origin.x + 4.0, body_rect.origin.y - 2.0],
        [body_rect.origin.x + 4.0, body_rect.origin.y + 1.0],
        1.2,
        icon_ink,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [body_rect.max().x - 4.0, body_rect.origin.y - 2.0],
        [body_rect.max().x - 4.0, body_rect.origin.y + 1.0],
        1.2,
        icon_ink,
        z + 2,
        &mut primitives,
        &mut payloads,
    );

    emit_glyphs(glyphs, ink, z + 2, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    // 展开态绘制浮层占位外壳；收起态不产生额外图元。
    if state.open {
        let panel = desc.panel_rect();
        primitives.push(instance(
            panel,
            [0.0; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            CALENDAR_DATE_PICKER_PANEL_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            z,
        ));
        primitives.push(surface(
            panel,
            CALENDAR_DATE_PICKER_PANEL_RADIUS,
            Token::OVERLAY_BG,
            z,
        ));
        primitives.push(outlined(
            panel,
            CALENDAR_DATE_PICKER_PANEL_RADIUS,
            Token::NONE,
            Token::CARD_BORDER,
            1.0,
            0.0,
            z,
        ));
    }

    let hit = (!disabled).then(|| {
        HitRegion::new(node, TEXT_INPUT_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });

    let mut a11y = A11yNode::new(
        Role::ComboBox,
        if desc.label.is_empty() {
            desc.placeholder.clone()
        } else {
            desc.label.clone()
        },
        rect,
    )
    .clickable()
    .disabled(disabled)
    .expanded(state.open);
    a11y.value = state.selected_text();

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        let key = calendar_icon_node(node);
        out.with_sub_control(
            key,
            HitRegion::new(key, TEXT_INPUT_LAYER, u32::from(z) + 1, icon)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(
                Role::Button,
                if state.open {
                    "Close calendar"
                } else {
                    "Open calendar"
                },
                icon,
            )
            .clickable()
            .expanded(state.open),
            true,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CalendarDatePickerDesc {
        CalendarDatePickerDesc::new(Rect::new(0.0, 0.0, 220.0, CALENDAR_DATE_PICKER_HEIGHT))
            .with_placeholder("Select a date")
    }

    fn date() -> NaiveDate {
        NaiveDate::new(2024, 3, 15).expect("valid date")
    }

    #[test]
    fn the_picker_starts_collapsed() {
        let state = CalendarDatePickerState::new();
        assert!(!state.open);
        assert_eq!(state.selected_text(), "");
    }

    #[test]
    fn clicking_the_trigger_expands_the_panel_shell() {
        let mut state = CalendarDatePickerState::with_date(date());
        let collapsed = emit_calendar_date_picker(1, &desc(), &state, ControlState::default(), &mut []);
        state.toggle();
        let expanded = emit_calendar_date_picker(1, &desc(), &state, ControlState::default(), &mut []);
        assert!(
            expanded.primitives.len() > collapsed.primitives.len(),
            "the open panel must add visible shell primitives"
        );
        assert_eq!(
            expanded.a11y.expect("reachable").expanded,
            Some(true)
        );
        assert_eq!(expanded.sub_a11y[0].1.label, "Close calendar");
        state.collapse();
        assert!(!state.open);
    }

    #[test]
    fn the_a11y_value_reports_the_selected_iso_date() {
        let out = emit_calendar_date_picker(
            1,
            &desc(),
            &CalendarDatePickerState::with_date(date()),
            ControlState::default(),
            &mut [],
        );
        assert_eq!(out.a11y.expect("reachable").value, "2024-03-15");
        assert_eq!(out.sub_hits.len(), 1);
        assert_eq!(out.sub_hits[0].node, calendar_icon_node(1));
    }

    #[test]
    fn hover_changes_the_field() {
        let idle = emit_calendar_date_picker(
            1,
            &desc(),
            &CalendarDatePickerState::new(),
            ControlState::default(),
            &mut [],
        );
        let hovered = emit_calendar_date_picker(
            1,
            &desc(),
            &CalendarDatePickerState::new(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_ne!(idle.primitives, hovered.primitives);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let state = CalendarDatePickerState::new();
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
                            emit_calendar_date_picker(1, &desc(), &state, control, &mut [])
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

//! ToggleButton：选中后保持按下状态的双态切换按钮。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 切换按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ToggleButtonDesc {
    pub rect: Rect,
    pub label: String,
    pub checked: bool,
    pub radius: f32,
    pub disabled: bool,
}

impl ToggleButtonDesc {
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>, checked: bool) -> Self {
        Self {
            rect,
            label: label.into(),
            checked,
            // WinUI ToggleButton CornerRadius = ControlCornerRadius = 4。
            radius: 4.0,
            disabled: false,
        }
    }

    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, label, false)
    }

    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 切换按钮状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToggleButtonState {
    pub checked: bool,
}

impl ToggleButtonState {
    #[must_use]
    pub fn new(checked: bool) -> Self {
        Self { checked }
    }

    pub fn toggle(&mut self) {
        self.checked = !self.checked;
    }
}

/// 输出切换按钮的图元与命中区域。
#[must_use]
pub fn emit_toggle_button(
    node: u64,
    desc: &ToggleButtonDesc,
    state: &ToggleButtonState,
    control: ControlState,
) -> WidgetOutput {
    emit_toggle_button_with_glyphs(node, desc, state, control, &[])
}

/// 输出带文本字形的切换按钮图元与命中区域。
#[must_use]
pub fn emit_toggle_button_with_glyphs(
    node: u64,
    desc: &ToggleButtonDesc,
    state: &ToggleButtonState,
    control: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let is_checked = state.checked || desc.checked;
    let z = 700u16;

    let disabled = control.disabled || desc.disabled;
    let (bg_token, border_token) = if disabled {
        if is_checked {
            // WinUI ToggleButtonBackgroundCheckedDisabled = AccentFillColorDisabled；
            // ToggleButtonBorderBrushCheckedDisabled = ControlFillColorTransparent。
            (Token::PRIMARY_SOFT, Token::NONE)
        } else {
            // ToggleButtonBackgroundDisabled = ControlFillColorDisabled；
            // ToggleButtonBorderBrushDisabled = ControlStrokeColorDefault。
            (Token::CONTROL_DISABLED_BG, Token::CONTROL_STROKE)
        }
    } else if is_checked {
        // Checked = AccentFillColorDefault，CheckedPointerOver = AccentFillColorSecondary，
        // CheckedPressed = AccentFillColorTertiary；按下态边框为透明。
        if control.phase == PointerPhase::Pressed {
            (Token::PRIMARY_PRESSED, Token::NONE)
        } else if control.phase == PointerPhase::Hovered {
            (Token::PRIMARY_HOVER, Token::PRIMARY)
        } else {
            (Token::PRIMARY, Token::PRIMARY)
        }
    } else if control.phase == PointerPhase::Pressed {
        // ToggleButtonBackgroundPressed = ControlFillColorTertiary；
        // ToggleButtonBorderBrushPressed = ControlStrokeColorDefault。
        (Token::SURFACE_SUNKEN, Token::CONTROL_STROKE)
    } else if control.phase == PointerPhase::Hovered {
        // ToggleButtonBackgroundPointerOver = ControlFillColorSecondary。
        (Token::CONTROL_HOVER, Token::CONTROL_STROKE)
    } else {
        // ToggleButtonBackground = ControlFillColorDefault；
        // ToggleButtonBorderBrush = ControlElevationBorderBrush。
        (Token::CONTROL_FILL, Token::CONTROL_STROKE)
    };

    // 1. 表面背景
    out.primitives.push(surface(rect, desc.radius, bg_token, z));

    // 2. 边框轮廓：WinUI ToggleButtonBorderThemeThickness = 1，各状态宽度一致。
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border_token,
        1.0,
        0.0,
        z + 1,
    ));

    if !label_glyphs.is_empty() {
        let ink = if disabled {
            if is_checked {
                // ToggleButtonForegroundCheckedDisabled = TextOnAccentFillColorDisabled。
                Token::ON_DISABLED
            } else {
                // ToggleButtonForegroundDisabled = TextFillColorDisabled。
                Token::TEXT_DISABLED
            }
        } else if is_checked {
            // ToggleButtonForegroundChecked = TextOnAccentFillColorPrimary。
            Token::ON_PRIMARY
        } else if control.phase == PointerPhase::Pressed {
            // ToggleButtonForegroundPressed = TextFillColorSecondary。
            Token::TEXT_SECONDARY
        } else {
            // ToggleButtonForeground = TextFillColorPrimary。
            Token::TEXT_PRIMARY
        };
        emit_glyphs(
            label_glyphs,
            ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 3. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    out.hit = (!disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });

    let mut a11y = A11yNode::new(Role::CheckBox, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = is_checked;
    out.a11y = Some(a11y);
    out.focusable = !disabled;
    out
}

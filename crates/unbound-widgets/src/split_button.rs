//! SplitButton：带下拉菜单触发器的组合操作按钮。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::{ButtonVariant, BUTTON_LAYER};
use crate::primitive::{
    curve_stroke, focus_ring, glyphs as emit_glyphs, instance, outlined, surface,
};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 下拉触发器的固定宽度。对齐 WinUI `SplitButtonSecondaryButtonSize = 32`。
pub const DROPDOWN_WIDTH: f32 = 32.0;

/// 分裂按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SplitButtonDesc {
    pub rect: Rect,
    pub label: String,
    pub variant: ButtonVariant,
    pub radius: f32,
    pub disabled: bool,
}

impl SplitButtonDesc {
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            // WinUI DefaultSplitButtonStyle 的底色是 ControlFillColorDefault，
            // 不是强调色；Primary 变体保留给「已选中」的 SplitButton。
            variant: ButtonVariant::Secondary,
            // WinUI SplitButton CornerRadius = ControlCornerRadius = 4。
            radius: 4.0,
            disabled: false,
        }
    }

    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::ZERO, label)
    }

    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 分裂按钮状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SplitButtonState {
    pub menu_open: bool,
    pub main_pressed: bool,
    pub arrow_pressed: bool,
}

impl SplitButtonState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// 输出分裂按钮的图元与命中区域。
#[must_use]
pub fn emit_split_button(
    node: u64,
    desc: &SplitButtonDesc,
    state: &SplitButtonState,
    control: ControlState,
) -> WidgetOutput {
    emit_split_button_with_glyphs(node, desc, state, control, &[])
}

/// 输出带文本字形的分裂按钮图元与命中区域。
#[must_use]
pub fn emit_split_button_with_glyphs(
    node: u64,
    desc: &SplitButtonDesc,
    state: &SplitButtonState,
    control: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let arrow_w = DROPDOWN_WIDTH.min(rect.width() * 0.4);
    let main_w = (rect.width() - arrow_w).max(0.0);
    let z = 700u16;

    let main_rect = Rect::new(rect.origin.x, rect.origin.y, main_w, rect.height());
    let arrow_rect = Rect::new(
        rect.origin.x + main_w,
        rect.origin.y,
        arrow_w,
        rect.height(),
    );
    // 分隔线占满整列高度。WinUI 模板里它是独立的 1px 网格列。
    let divider_rect = Rect::new(
        rect.origin.x + main_w - 0.5,
        rect.origin.y,
        1.0,
        rect.height(),
    );

    let disabled = control.disabled || desc.disabled;
    // Primary 变体对应 WinUI 的 Checked 配色（强调色 + on-accent 文字），
    // 其余变体使用 DefaultSplitButtonStyle 的中性表面。
    let accent = desc.variant == ButtonVariant::Primary;
    let bg_token = if disabled {
        // SplitButtonBackgroundDisabled = ControlFillColorDisabled。
        Token::CONTROL_DISABLED_BG
    } else if accent {
        if control.phase == PointerPhase::Pressed || state.main_pressed {
            // SplitButtonBackgroundCheckedPressed = AccentFillColorTertiary。
            Token::PRIMARY_PRESSED
        } else if control.phase == PointerPhase::Hovered {
            // SplitButtonBackgroundCheckedPointerOver = AccentFillColorSecondary。
            Token::PRIMARY_HOVER
        } else {
            // SplitButtonBackgroundChecked = AccentFillColorDefault。
            Token::PRIMARY
        }
    } else if control.phase == PointerPhase::Pressed || state.main_pressed {
        // SplitButtonBackgroundPressed = ControlFillColorTertiary。
        Token::SURFACE_SUNKEN
    } else if control.phase == PointerPhase::Hovered {
        // SplitButtonBackgroundPointerOver = ControlFillColorSecondary。
        Token::CONTROL_HOVER
    } else {
        // SplitButtonBackground = ControlFillColorDefault。
        Token::CONTROL_FILL
    };
    let divider_token = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if accent {
        // SplitButtonBorderBrushCheckedDivider = ControlStrokeColorOnAccentTertiary。
        Token::PRIMARY_PRESSED
    } else {
        // SplitButtonBorderBrushDivider = ControlStrokeColorDefault。
        Token::CONTROL_STROKE
    };

    // 1. 主操作按钮背景表面（仅左侧圆角）
    out.primitives.push(instance(
        main_rect,
        [desc.radius, 0.0, 0.0, desc.radius],
        bg_token,
        Token::NONE,
        0.0,
        0.0,
        unbound_gpu::data::PrimType::RoundedRect,
        unbound_gpu::data::SubElement::Background,
        z,
    ));

    // 2. 下拉触发器背景表面（仅右侧圆角）
    let arrow_bg = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.menu_open || state.arrow_pressed {
        if accent {
            Token::PRIMARY_PRESSED
        } else {
            Token::SURFACE_SUNKEN
        }
    } else {
        bg_token
    };
    out.primitives.push(instance(
        arrow_rect,
        [0.0, desc.radius, desc.radius, 0.0],
        arrow_bg,
        Token::NONE,
        0.0,
        0.0,
        unbound_gpu::data::PrimType::RoundedRect,
        unbound_gpu::data::SubElement::Background,
        z + 1,
    ));

    // 3. 分隔线
    out.primitives
        .push(surface(divider_rect, 0.0, divider_token, z + 2));

    // 4. 外框轮廓边框（SplitButtonBorderThemeThickness = 1）
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if accent && !disabled {
            Token::PRIMARY
        } else {
            Token::CONTROL_STROKE
        },
        1.0,
        0.0,
        z + 3,
    ));

    // 5. 下拉箭头与标签字形
    //
    // 箭头取 `SplitButtonForegroundSecondary`（TextFillColorSecondary），
    // 主标签取 `SplitButtonForeground`（TextFillColorPrimary）。
    let (arrow_ink, ink) = if disabled {
        (Token::TEXT_DISABLED, Token::TEXT_DISABLED)
    } else if accent {
        (Token::ON_PRIMARY, Token::ON_PRIMARY)
    } else {
        (Token::TEXT_SECONDARY, Token::TEXT_PRIMARY)
    };
    // WinUI 的下拉图标是 12×12、距右沿 12px 的内容内边距，故中心在右沿内 18px。
    let acx = arrow_rect.max().x - 18.0;
    let acy = arrow_rect.center().y;
    curve_stroke(
        [acx - 4.0, acy - 2.0],
        [acx, acy + 2.0],
        1.5,
        arrow_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );
    curve_stroke(
        [acx, acy + 2.0],
        [acx + 4.0, acy - 2.0],
        1.5,
        arrow_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );
    if !label_glyphs.is_empty() {
        emit_glyphs(
            label_glyphs,
            ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 6. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    // 多命中区域：主操作区与下拉箭头区
    let action_node = node ^ 0xFE;
    let arrow_node = node ^ 0xFF;

    if !disabled {
        out.sub_hits.push(
            HitRegion::new(action_node, BUTTON_LAYER, u32::from(z + 4), main_rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer),
        );
        out.sub_hits.push(
            HitRegion::new(arrow_node, BUTTON_LAYER, u32::from(z + 4), arrow_rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer),
        );
        out.hit = Some(
            HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer),
        );
    }

    out.a11y = Some(
        A11yNode::new(Role::Button, desc.label.clone(), rect)
            .clickable()
            .disabled(disabled),
    );
    out.sub_a11y.push((
        action_node,
        A11yNode::new(Role::Button, desc.label.clone(), main_rect)
            .clickable()
            .disabled(disabled),
    ));
    out.sub_a11y.push((
        arrow_node,
        A11yNode::new(Role::Button, "Options", arrow_rect)
            .clickable()
            .disabled(disabled),
    ));
    out.focusable = !disabled;

    out
}

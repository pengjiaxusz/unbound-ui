//! SplitButton：带下拉菜单触发器的组合操作按钮。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::{ButtonVariant, BUTTON_LAYER};
use crate::primitive::{focus_ring, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

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
            variant: ButtonVariant::Primary,
            radius: 6.0,
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
    let divider_rect = Rect::new(
        rect.origin.x + main_w - 0.5,
        rect.origin.y + 4.0,
        1.0,
        (rect.height() - 8.0).max(0.0),
    );

    let disabled = control.disabled || desc.disabled;
    let bg_token = if disabled {
        Token::DISABLED
    } else if control.phase == PointerPhase::Pressed || state.main_pressed {
        Token::PRIMARY_PRESSED
    } else if control.phase == PointerPhase::Hovered {
        Token::PRIMARY_HOVER
    } else {
        Token::PRIMARY
    };

    // 1. 主操作按钮背景表面
    out.primitives
        .push(surface(main_rect, desc.radius, bg_token, z));

    // 2. 下拉触发器背景表面
    let arrow_bg = if disabled {
        Token::DISABLED
    } else if state.menu_open || state.arrow_pressed {
        Token::PRIMARY_PRESSED
    } else {
        bg_token
    };
    out.primitives
        .push(surface(arrow_rect, desc.radius, arrow_bg, z + 1));

    // 3. 分隔线
    out.primitives
        .push(surface(divider_rect, 0.0, Token::OUTLINE, z + 2));

    // 4. 外框轮廓边框
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        if disabled {
            Token::DISABLED
        } else {
            Token::OUTLINE_STRONG
        },
        1.0,
        0.0,
        z + 3,
    ));

    // 5. 焦点环
    out.primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !disabled,
    ));

    // 多命中区域：主操作区与下拉箭头区
    let action_node = node;
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

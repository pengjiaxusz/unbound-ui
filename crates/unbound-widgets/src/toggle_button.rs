//! ToggleButton: A two-state button that remains checked when selected.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// Descriptor for a toggle button.
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
            radius: 6.0,
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

/// State for the toggle button.
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

/// Emit primitives and hit regions for a toggle button.
#[must_use]
pub fn emit_toggle_button(
    node: u64,
    desc: &ToggleButtonDesc,
    state: &ToggleButtonState,
    control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = desc.rect;
    let is_checked = state.checked || desc.checked;
    let z = 700u16;

    let (bg_token, border_token) = if control.disabled || desc.disabled {
        (Token::DISABLED, Token::OUTLINE)
    } else if is_checked {
        if control.phase == PointerPhase::Pressed {
            (Token::PRIMARY_PRESSED, Token::PRIMARY)
        } else if control.phase == PointerPhase::Hovered {
            (Token::PRIMARY_HOVER, Token::PRIMARY)
        } else {
            (Token::PRIMARY, Token::PRIMARY)
        }
    } else if control.phase == PointerPhase::Pressed {
        (Token::PRIMARY_PRESSED, Token::OUTLINE_STRONG)
    } else if control.phase == PointerPhase::Hovered {
        (Token::PRIMARY_HOVER, Token::OUTLINE_STRONG)
    } else {
        (Token::SURFACE, Token::OUTLINE)
    };

    // 1. Surface background
    out.primitives.push(surface(rect, desc.radius, bg_token, z));

    // 2. Border outline
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border_token,
        if is_checked { 1.5 } else { 1.0 },
        0.0,
        z + 1,
    ));

    out.hit = Some(
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(if desc.disabled {
                CursorKind::NotAllowed
            } else {
                CursorKind::Pointer
            }),
    );

    let mut a11y = A11yNode::new(Role::CheckBox, desc.label.clone(), rect).clickable();
    a11y.selected = is_checked;
    out.a11y = Some(a11y);
    out
}

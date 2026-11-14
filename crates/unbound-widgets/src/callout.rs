//! Callout: In-page notification banner with title, message, severity, and dismiss action.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::alert::Severity;
use crate::button::BUTTON_LAYER;
use crate::primitive::{outlined, surface};
use crate::{ControlState, WidgetOutput};

/// Descriptor for a callout bar.
#[derive(Clone, Debug, PartialEq)]
pub struct CalloutDesc {
    pub rect: Rect,
    pub title: String,
    pub message: String,
    pub severity: Severity,
    pub closable: bool,
    pub radius: f32,
}

impl CalloutDesc {
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>, severity: Severity) -> Self {
        Self {
            rect,
            title: title.into(),
            message: String::new(),
            severity,
            closable: true,
            radius: 8.0,
        }
    }

    #[must_use]
    pub fn title(title: impl Into<String>, severity: Severity) -> Self {
        Self::new(Rect::ZERO, title, severity)
    }

    #[must_use]
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    #[must_use]
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

/// State for a callout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalloutState {
    pub closed: bool,
}

impl CalloutState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dismiss(&mut self) {
        self.closed = true;
    }
}

/// Emit primitives and hit regions for a callout banner.
#[must_use]
pub fn emit_callout(
    node: u64,
    desc: &CalloutDesc,
    state: &CalloutState,
    _control: ControlState,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let rect = if state.closed { Rect::ZERO } else { desc.rect };
    let z = 700u16;

    let (bg_token, border_token, stripe_token) = match desc.severity {
        Severity::Info => (Token::PRIMARY_SOFT, Token::PRIMARY, Token::PRIMARY),
        Severity::Success => (Token::SUCCESS_SOFT, Token::SUCCESS, Token::SUCCESS),
        Severity::Warning => (Token::WARNING_SOFT, Token::WARNING, Token::WARNING),
        Severity::Error => (Token::ERROR_SOFT, Token::ERROR, Token::ERROR),
    };

    // 1. Background surface
    out.primitives.push(surface(rect, desc.radius, bg_token, z));

    // 2. Left severity accent stripe
    let stripe_rect = Rect::new(rect.origin.x, rect.origin.y, 4.0, rect.height());
    out.primitives
        .push(surface(stripe_rect, 2.0, stripe_token, z + 1));

    // 3. Border outline
    out.primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border_token,
        1.0,
        0.0,
        z + 2,
    ));

    // 4. Close button hit region
    let close_node = node ^ 0xC105E;
    let close_rect = Rect::new(
        rect.origin.x + (rect.width() - 32.0).max(0.0),
        rect.origin.y + 6.0,
        24.0,
        24.0,
    );

    if desc.closable && !state.closed {
        out.sub_hits.push(
            HitRegion::new(close_node, BUTTON_LAYER, u32::from(z + 3), close_rect)
                .with_shape(Shape::Rounded { radii: [4.0; 4] })
                .with_cursor(CursorKind::Pointer),
        );
        out.sub_a11y.push((
            close_node,
            A11yNode::new(Role::Button, "Dismiss", close_rect).clickable(),
        ));
    }

    out.hit = Some(
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Pointer),
    );

    out.a11y = Some(A11yNode::new(Role::Alert, desc.title.clone(), rect));
    out
}

//! Badges and chips: compact status and filter affordances.
//!
//! Both are labels first: a badge is never the only carrier of meaning, and
//! its accessible label repeats the text. Colour is a secondary channel, which
//! is why every variant also has a distinct default label in the audit's
//! contrast matrix.
//!
//! A chip owns no state. [`ChipDesc::selected`] and `closable` are policy, and
//! the emitter reports the two hit regions (body, close) so the reducer can
//! route a click to either.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The semantic colour family of a badge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BadgeVariant {
    /// Neutral information.
    #[default]
    Neutral,
    /// The primary accent.
    Primary,
    /// A success state.
    Success,
    /// A warning.
    Warning,
    /// An error or destructive state.
    Error,
}

impl BadgeVariant {
    /// The low-saturation fill.
    #[must_use]
    pub fn fill(self) -> Token {
        match self {
            Self::Neutral => Token::SURFACE_SUNKEN,
            Self::Primary => Token::PRIMARY_SOFT,
            Self::Success => Token::SUCCESS_SOFT,
            Self::Warning => Token::WARNING_SOFT,
            Self::Error => Token::ERROR_SOFT,
        }
    }

    /// The saturated text colour.
    #[must_use]
    pub fn text(self) -> Token {
        match self {
            Self::Neutral => Token::TEXT_SECONDARY,
            Self::Primary => Token::PRIMARY,
            Self::Success => Token::SUCCESS,
            Self::Warning => Token::WARNING,
            Self::Error => Token::ERROR,
        }
    }
}

/// A pill-shaped status label.
#[derive(Clone, Debug, PartialEq)]
pub struct BadgeDesc {
    /// The badge rectangle.
    pub rect: Rect,
    /// The text.
    pub text: String,
    /// The colour family.
    pub variant: BadgeVariant,
    /// Draw a leading status dot.
    pub dot: bool,
    /// The corner radius; defaults to a full pill.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
}

impl BadgeDesc {
    /// A pill badge.
    #[must_use]
    pub fn new(rect: Rect, text: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            variant: BadgeVariant::Neutral,
            dot: false,
            radius: rect.size.y * 0.5,
            z: 560,
        }
    }

    /// The same badge in a colour family.
    #[must_use]
    pub fn variant(mut self, variant: BadgeVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The same badge with a leading dot.
    #[must_use]
    pub fn with_dot(mut self) -> Self {
        self.dot = true;
        self
    }

    /// A dot-only badge, for counters with no room for text.
    #[must_use]
    pub fn status_dot(rect: Rect, variant: BadgeVariant) -> Self {
        Self {
            rect,
            text: String::new(),
            variant,
            dot: true,
            radius: rect.size.x.min(rect.size.y) * 0.5,
            z: 560,
        }
    }

    /// The leading dot, when there is one.
    #[must_use]
    pub fn dot_rect(&self) -> Option<Rect> {
        if !self.dot {
            return None;
        }
        let size = (self.rect.size.y * 0.4).min(8.0);
        Some(Rect::new(
            self.rect.origin.x + 6.0,
            self.rect.origin.y + (self.rect.size.y - size) * 0.5,
            size,
            size,
        ))
    }
}

/// Emit a badge.
#[must_use]
pub fn emit_badge(desc: &BadgeDesc, badge_glyphs: &[GlyphQuad]) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + badge_glyphs.len());
    let mut payloads = Vec::with_capacity(badge_glyphs.len());
    primitives.push(surface(desc.rect, desc.radius, desc.variant.fill(), desc.z));
    if let Some(dot) = desc.dot_rect() {
        primitives.push(surface(
            dot,
            dot.size.x * 0.5,
            desc.variant.text(),
            desc.z + 1,
        ));
    }
    glyphs(
        badge_glyphs,
        desc.variant.text(),
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(A11yNode::new(Role::Label, desc.text.clone(), desc.rect)),
        focusable: false,
    }
}

/// A filter pill with an optional close affordance.
#[derive(Clone, Debug, PartialEq)]
pub struct ChipDesc {
    /// The chip rectangle.
    pub rect: Rect,
    /// The label.
    pub text: String,
    /// The chip is toggled on.
    pub selected: bool,
    /// The chip shows a close button.
    pub closable: bool,
    /// The chip refuses interaction.
    pub disabled: bool,
    /// The draw order key.
    pub z: u16,
}

impl ChipDesc {
    /// A deselected, unclosable chip.
    #[must_use]
    pub fn new(rect: Rect, text: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            selected: false,
            closable: false,
            disabled: false,
            z: 558,
        }
    }

    /// The same chip, selected.
    #[must_use]
    pub fn selected(mut self) -> Self {
        self.selected = true;
        self
    }

    /// The same chip, closable.
    #[must_use]
    pub fn closable(mut self) -> Self {
        self.closable = true;
        self
    }

    /// The same chip, disabled.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The close affordance; `None` when the chip is not closable.
    #[must_use]
    pub fn close_rect(&self) -> Option<Rect> {
        if !self.closable {
            return None;
        }
        let size = (self.rect.size.y * 0.36).min(12.0);
        Some(Rect::new(
            self.rect.max().x - size - 6.0,
            self.rect.origin.y + (self.rect.size.y - size) * 0.5,
            size,
            size,
        ))
    }

    /// The body hit region, excluding the close affordance.
    #[must_use]
    pub fn body_rect(&self) -> Rect {
        match self.close_rect() {
            Some(close) => Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                (close.origin.x - self.rect.origin.x - 2.0).max(0.0),
                self.rect.size.y,
            ),
            None => self.rect,
        }
    }
}

/// Emit a chip and the two hit regions.
#[must_use]
pub fn emit_chip(
    node: u64,
    desc: &ChipDesc,
    control: ControlState,
    chip_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let radius = desc.rect.size.y * 0.5;
    let disabled = control.disabled || desc.disabled;
    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if desc.selected {
        Token::PRIMARY_SOFT
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_SUNKEN
    };
    let border = if desc.selected {
        Token::PRIMARY
    } else if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::OUTLINE
    };
    let text = if disabled {
        Token::ON_DISABLED
    } else if desc.selected {
        Token::PRIMARY
    } else {
        Token::TEXT_PRIMARY
    };

    let mut primitives = Vec::with_capacity(3 + chip_glyphs.len());
    let mut payloads = Vec::with_capacity(chip_glyphs.len());
    primitives.push(outlined(desc.rect, radius, fill, border, 1.0, 0.0, desc.z));
    if let Some(close) = desc.close_rect() {
        primitives.push(surface(close, 1.0, text, desc.z + 1));
    }
    glyphs(
        chip_glyphs,
        text,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        desc.rect,
        radius,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        control.focused,
    ));

    let mut a11y = A11yNode::new(Role::Button, desc.text.clone(), desc.rect).clickable();
    a11y.selected = desc.selected;
    a11y.disabled = disabled;
    a11y.description = if desc.closable {
        "按 Delete 移除".to_owned()
    } else {
        String::new()
    };

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.body_rect())
                .with_shape(Shape::Rounded { radii: [radius; 4] })
                .with_cursor(if disabled {
                    CursorKind::NotAllowed
                } else {
                    CursorKind::Pointer
                }),
        ),
        a11y: Some(a11y),
        focusable: !disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn badge() -> BadgeDesc {
        BadgeDesc::new(Rect::new(0.0, 0.0, 60.0, 20.0), "New")
    }

    #[test]
    fn variants_have_distinct_fills_and_text() {
        let variants = [
            BadgeVariant::Neutral,
            BadgeVariant::Primary,
            BadgeVariant::Success,
            BadgeVariant::Warning,
            BadgeVariant::Error,
        ];
        let fills: Vec<Token> = variants.iter().map(|variant| variant.fill()).collect();
        let texts: Vec<Token> = variants.iter().map(|variant| variant.text()).collect();
        for (index, fill) in fills.iter().enumerate() {
            assert!(!fills[..index].contains(fill));
        }
        for (index, text) in texts.iter().enumerate() {
            assert!(!texts[..index].contains(text));
        }
    }

    #[test]
    fn a_pill_is_fully_rounded() {
        let badge = badge();
        assert_eq!(badge.radius, 10.0);
        assert!(badge.dot_rect().is_none());
    }

    #[test]
    fn a_dot_badge_places_the_dot_inside() {
        let badge = badge().with_dot();
        let dot = badge.dot_rect().expect("dot");
        assert!(badge.rect.contains_rect(&dot));
        assert_eq!(dot.size.x, dot.size.y);
    }

    #[test]
    fn the_badge_emitter_is_a_label() {
        let output = emit_badge(&badge(), &[]);
        assert_eq!(output.a11y.expect("label").role, Role::Label);
        assert_eq!(output.primitives.len(), 1);
        assert!(output.hit.is_none());
    }

    #[test]
    fn a_chip_without_a_close_uses_the_whole_rect() {
        let chip = ChipDesc::new(Rect::new(0.0, 0.0, 80.0, 24.0), "PNG");
        assert!(chip.close_rect().is_none());
        assert_eq!(chip.body_rect(), chip.rect);
    }

    #[test]
    fn a_closable_chip_reserves_room_for_the_close() {
        let chip = ChipDesc::new(Rect::new(0.0, 0.0, 80.0, 24.0), "PNG").closable();
        let close = chip.close_rect().expect("close");
        assert!(chip.rect.contains_rect(&close));
        assert!(chip.body_rect().max().x <= close.origin.x);
    }

    #[test]
    fn a_selected_chip_exposes_selected_and_primary_colours() {
        let chip = ChipDesc::new(Rect::new(0.0, 0.0, 80.0, 24.0), "PNG").selected();
        let output = emit_chip(3, &chip, ControlState::default(), &[]);
        let a11y = output.a11y.expect("button");
        assert_eq!(a11y.role, Role::Button);
        assert!(a11y.selected);
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            u32::from(Token::PRIMARY_SOFT.raw())
        );
    }

    #[test]
    fn a_disabled_chip_refuses_the_pointer() {
        let chip = ChipDesc::new(Rect::new(0.0, 0.0, 80.0, 24.0), "PNG").disabled(true);
        let output = emit_chip(3, &chip, ControlState::default(), &[]);
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::NotAllowed))
        );
        assert!(!output.focusable);
    }

    #[test]
    fn stable_shape_across_states() {
        let chip = ChipDesc::new(Rect::new(0.0, 0.0, 80.0, 24.0), "PNG").closable();
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
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
                        let output = emit_chip(3, &chip, control, &[]);
                        counts.push(output.primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

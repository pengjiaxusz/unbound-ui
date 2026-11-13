//! The divider: a hairline that separates two regions, optionally labelled.
//!
//! A divider is decoration, so it has no hit region and takes no focus. The
//! only rule it enforces is that its stroke lands on the physical pixel grid:
//! a one-pixel rule that straddles two pixels reads as a two-pixel blur.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::{snap_screen, Token};

use crate::primitive::{glyphs, instance};
use crate::WidgetOutput;

/// Which way a divider runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DividerOrientation {
    /// A horizontal rule spanning the width.
    #[default]
    Horizontal,
    /// A vertical rule spanning the height.
    Vertical,
}

/// The pure descriptor of a divider.
#[derive(Clone, Debug, PartialEq)]
pub struct DividerDesc {
    /// The region the divider lives in.
    pub rect: Rect,
    /// Whether the rule runs horizontally or vertically.
    pub orientation: DividerOrientation,
    /// The optional centred label.
    pub label: Option<String>,
    /// The leading and trailing inset of the rule.
    pub inset: f32,
    /// The gap between the rule and a label.
    pub label_gap: f32,
    /// The hairline thickness.
    pub thickness: f32,
    /// The draw order key.
    pub z: u16,
}

impl DividerDesc {
    /// A horizontal divider across the given rectangle.
    #[must_use]
    pub fn horizontal(rect: Rect) -> Self {
        Self {
            rect,
            orientation: DividerOrientation::Horizontal,
            label: None,
            inset: 0.0,
            label_gap: 12.0,
            thickness: 1.0,
            z: 300,
        }
    }

    /// A vertical divider down the given rectangle.
    #[must_use]
    pub fn vertical(rect: Rect) -> Self {
        Self {
            orientation: DividerOrientation::Vertical,
            ..Self::horizontal(rect)
        }
    }

    /// The same divider with a centred label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The same divider with a leading and trailing inset.
    #[must_use]
    pub fn with_inset(mut self, inset: f32) -> Self {
        self.inset = inset.max(0.0);
        self
    }

    /// The snapped stroke rectangle.
    #[must_use]
    pub fn rule_rect(&self) -> Rect {
        let thickness = self.thickness.max(1.0);
        match self.orientation {
            DividerOrientation::Horizontal => {
                let y = snap_screen(self.rect.origin.y + (self.rect.size.y - thickness) * 0.5);
                Rect::new(
                    snap_screen(self.rect.origin.x + self.inset),
                    y,
                    (self.rect.size.x - self.inset * 2.0).max(0.0),
                    snap_screen(thickness),
                )
            }
            DividerOrientation::Vertical => {
                let x = snap_screen(self.rect.origin.x + (self.rect.size.x - thickness) * 0.5);
                Rect::new(
                    x,
                    snap_screen(self.rect.origin.y + self.inset),
                    snap_screen(thickness),
                    (self.rect.size.y - self.inset * 2.0).max(0.0),
                )
            }
        }
    }

    /// The two half-rules that make room for a label.
    ///
    /// Returns the leading and trailing segments plus the label rectangle.
    #[must_use]
    pub fn labelled_segments(&self, label_width: f32) -> Option<(Rect, Rect, Rect)> {
        let rule = self.rule_rect();
        if label_width <= 0.0 {
            return None;
        }
        let horizontal = self.orientation == DividerOrientation::Horizontal;
        let span = if horizontal { rule.size.x } else { rule.size.y };
        if label_width + self.label_gap * 2.0 >= span {
            return None;
        }
        let start = if horizontal {
            rule.origin.x
        } else {
            rule.origin.y
        };
        let leading = (span - label_width) * 0.5 - self.label_gap;
        let trailing = span - leading - label_width - self.label_gap * 2.0;
        if horizontal {
            let band = Rect::new(
                rule.origin.x,
                rule.origin.y - self.rect.size.y * 0.25,
                label_width.max(1.0),
                (self.rect.size.y * 0.5).max(rule.size.y),
            );
            let first = Rect::new(start, rule.origin.y, leading.max(0.0), rule.size.y);
            let second = Rect::new(
                start + leading + self.label_gap * 2.0 + label_width,
                rule.origin.y,
                trailing.max(0.0),
                rule.size.y,
            );
            Some((first, second, band))
        } else {
            // A vertical divider with a label is unusual; the label is laid
            // out as a horizontal band crossing the rule.
            let band = Rect::new(
                rule.origin.x - self.rect.size.x * 0.25,
                rule.origin.y,
                (self.rect.size.x * 0.5).max(rule.size.x),
                label_width.max(1.0),
            );
            let first = Rect::new(rule.origin.x, rule.origin.y, rule.size.x, leading.max(0.0));
            let second = Rect::new(
                rule.origin.x,
                rule.origin.y + leading + self.label_gap * 2.0 + label_width,
                rule.size.x,
                trailing.max(0.0),
            );
            Some((first, second, band))
        }
    }
}

/// The colour slots a divider paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DividerTokens {
    /// The rule.
    pub rule: Token,
    /// The label text.
    pub label: Token,
}

impl DividerTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            rule: Token::DIVIDER,
            label: Token::TEXT_SECONDARY,
        }
    }
}

/// Emit the rule, and the label when the caller shaped one.
///
/// `label_width` is the measured width of `label`; pass `0.0` to draw an
/// unbroken rule even when [`DividerDesc::label`] is set.
#[must_use]
pub fn emit_divider(
    desc: &DividerDesc,
    label: &[GlyphQuad],
    label_width: f32,
    tokens: DividerTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    match desc.labelled_segments(label_width) {
        Some((first, second, _band)) => {
            primitives.push(line(first, tokens.rule, desc.z));
            primitives.push(line(second, tokens.rule, desc.z));
            glyphs(label, tokens.label, desc.z, &mut primitives, &mut payloads);
        }
        None => primitives.push(line(desc.rule_rect(), tokens.rule, desc.z)),
    }

    let a11y = desc
        .label
        .as_ref()
        .map(|text| A11yNode::new(Role::GenericContainer, text.clone(), desc.rect));

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y,
        focusable: false,
        ..Default::default()
    }
}

/// A solid line primitive.
fn line(rect: Rect, color: Token, z: u16) -> GpuPrimitiveInstance {
    instance(
        rect,
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_horizontal_rule_sits_on_the_pixel_grid() {
        let desc = DividerDesc::horizontal(Rect::new(10.4, 20.6, 200.0, 8.0));
        let rule = desc.rule_rect();
        assert_eq!(rule.origin.y, rule.origin.y.round());
        assert_eq!(rule.size.y, 1.0);
        assert_eq!(rule.origin.x, 10.0);
        assert_eq!(rule.size.x, 200.0);
    }

    #[test]
    fn the_inset_shortens_both_ends() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 100.0, 8.0)).with_inset(12.0);
        let rule = desc.rule_rect();
        assert_eq!(rule.origin.x, 12.0);
        assert_eq!(rule.size.x, 76.0);
    }

    #[test]
    fn a_vertical_rule_is_one_pixel_wide() {
        let desc = DividerDesc::vertical(Rect::new(0.0, 0.0, 8.0, 100.0));
        let rule = desc.rule_rect();
        assert_eq!(rule.size.x, 1.0);
        assert_eq!(rule.size.y, 100.0);
    }

    #[test]
    fn a_label_splits_the_rule_into_two_segments() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 20.0)).with_label("OR");
        let (first, second, band) = desc.labelled_segments(24.0).expect("room for the label");
        assert!(first.size.x > 0.0);
        assert!(second.size.x > 0.0);
        assert_eq!(band.size.x, 24.0);
        assert_eq!(band.size.y, 10.0);
        assert!(first.max().x < second.origin.x);
    }

    #[test]
    fn a_label_that_does_not_fit_leaves_the_rule_whole() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 20.0, 20.0)).with_label("OR");
        assert!(desc.labelled_segments(200.0).is_none());
    }

    #[test]
    fn the_divider_is_decorative_and_takes_no_input() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 8.0));
        let output = emit_divider(&desc, &[], 0.0, DividerTokens::standard());
        assert!(output.hit.is_none());
        assert!(output.a11y.is_none());
        assert!(!output.focusable);
        assert_eq!(output.primitives.len(), 1);
    }

    #[test]
    fn a_labelled_divider_exposes_its_text() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 20.0)).with_label("OR");
        let output = emit_divider(&desc, &[], 24.0, DividerTokens::standard());
        let a11y = output.a11y.expect("a label is reachable");
        assert_eq!(a11y.label, "OR");
        assert_eq!(output.primitives.len(), 2);
    }

    #[test]
    fn the_rule_uses_the_divider_token() {
        let desc = DividerDesc::horizontal(Rect::new(0.0, 0.0, 300.0, 8.0));
        let tokens = DividerTokens::standard();
        let output = emit_divider(&desc, &[], 0.0, tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.rule.raw() as u32
        );
        assert_eq!(output.primitives[0].prim_type(), PrimType::Line);
    }
}

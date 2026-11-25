//! Single-line rich text widget rendering colored [`TextSpan`] runs directly.
//!
//! Unlike [`crate::rich_text::RichTextDesc`] which parses multi-block Markdown and emits
//! monochrome glyph runs, [`RichTextLineDesc`] consumes pre-tokenized [`TextSpan`] slices
//! (for example syntax-highlighted code lines, log lines, or styled inline labels) and
//! preserves each span's semantic [`Token`] color without falling back to monochrome text.
//!
//! Supports optional monospace font resolution and optional per-span background/underline
//! decorations while maintaining a strictly constant primitive count across all
//! [`ControlState`] transitions (`Stable Shape` invariant).

use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::{GlyphQuad, TextSpan, TextStyle};
use unbound_theme::Token;

use crate::ergonomics::{AlignX, AlignY};
use crate::primitive::{glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// Default font size (in logical pixels) for a rich text line.
pub const RICH_TEXT_LINE_DEFAULT_SIZE: f32 = 14.0;

/// A single rasterized glyph quad paired with its resolved semantic foreground color token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColoredGlyphQuad {
    /// Positioned and atlas-mapped glyph quad in logical coordinates.
    pub quad: GlyphQuad,
    /// Semantic color token resolved from the owning [`TextSpan`] (or line default).
    pub color: Token,
}

/// Optional per-span decoration rectangle (inline background highlight or underline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpanDecoration {
    /// Decoration bounds in logical pixels.
    pub rect: Rect,
    /// Semantic color token used to fill the decoration surface.
    pub color: Token,
}

/// Pure data descriptor for a single colored [`TextSpan`] rich text line.
#[derive(Clone, Debug, PartialEq)]
pub struct RichTextLineDesc {
    /// Bounding rectangle for alignment, clipping, and accessibility.
    pub rect: Rect,
    /// Source UTF-8 line text referenced by `spans` byte ranges.
    pub text: String,
    /// Styled inline spans covering byte ranges within `text`.
    pub spans: Vec<TextSpan>,
    /// Base text style applied to gaps not covered by any span.
    pub base_style: TextStyle,
    /// Fallback semantic color token for text outside all spans.
    pub default_color: Token,
    /// When `true`, resolves unspecified font families to the system monospace font.
    pub monospace: bool,
    /// Horizontal alignment within `rect`.
    pub align_x: AlignX,
    /// Vertical alignment within `rect`.
    pub align_y: AlignY,
    /// Optional accessibility label override (defaults to `text`).
    pub label: Option<String>,
    /// Base draw-order Z key.
    pub z: u16,
}

impl RichTextLineDesc {
    /// Create a new rich text line descriptor with default typography and left-middle alignment.
    #[must_use]
    pub fn new(rect: Rect, text: impl Into<String>, spans: Vec<TextSpan>) -> Self {
        Self {
            rect,
            text: text.into(),
            spans,
            base_style: TextStyle::sized(RICH_TEXT_LINE_DEFAULT_SIZE),
            default_color: Token::TEXT_PRIMARY,
            monospace: false,
            align_x: AlignX::Left,
            align_y: AlignY::Middle,
            label: None,
            z: 500,
        }
    }

    /// Override the base text style.
    #[must_use]
    pub fn with_base_style(mut self, style: TextStyle) -> Self {
        self.base_style = style;
        self
    }

    /// Override the default fallback text color token.
    #[must_use]
    pub const fn with_default_color(mut self, color: Token) -> Self {
        self.default_color = color;
        self
    }

    /// Enable or disable monospace font resolution for unspecified families.
    #[must_use]
    pub const fn with_monospace(mut self, monospace: bool) -> Self {
        self.monospace = monospace;
        self
    }

    /// Set horizontal and vertical alignment within `rect`.
    #[must_use]
    pub const fn with_align(mut self, align_x: AlignX, align_y: AlignY) -> Self {
        self.align_x = align_x;
        self.align_y = align_y;
        self
    }

    /// Set an explicit accessibility label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the base draw-order Z key.
    #[must_use]
    pub const fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// Return the effective base style with monospace family applied when `monospace` is enabled.
    #[must_use]
    pub fn effective_base_style(&self) -> TextStyle {
        let mut style = self.base_style.clone();
        if self.monospace && style.family.is_none() {
            style.family = Some("monospace".to_owned());
        }
        style
    }

    /// Return effective spans with monospace family inherited when `monospace` is enabled.
    #[must_use]
    pub fn effective_spans(&self) -> Vec<TextSpan> {
        if !self.monospace {
            return self.spans.clone();
        }
        self.spans
            .iter()
            .cloned()
            .map(|mut span| {
                if span.style.family.is_none() {
                    span.style.family = Some("monospace".to_owned());
                }
                span
            })
            .collect()
    }

    /// Resolve the semantic color token at a given UTF-8 byte offset within `text`.
    #[must_use]
    pub fn color_at(&self, byte_offset: usize) -> Token {
        for span in &self.spans {
            if span.range.contains(&byte_offset) {
                return span.color;
            }
        }
        self.default_color
    }

    /// Pair clustered glyph quads (`(cluster_byte_offset, GlyphQuad)`) with their span colors.
    #[must_use]
    pub fn resolve_colored_quads(
        &self,
        clustered_quads: &[(u32, GlyphQuad)],
    ) -> Vec<ColoredGlyphQuad> {
        clustered_quads
            .iter()
            .map(|&(cluster, quad)| ColoredGlyphQuad {
                quad,
                color: self.color_at(cluster as usize),
            })
            .collect()
    }
}

/// Emit GPU primitives for a single colored [`TextSpan`] rich text line.
///
/// Every glyph quad is emitted with its resolved per-span [`Token`] color (or
/// [`Token::TEXT_DISABLED`] when disabled), avoiding any monochrome fallback.
/// The total primitive count is strictly `decorations.len() + colored_quads.len()`
/// across all [`ControlState`] values.
#[must_use]
pub fn emit_rich_text_line(
    node: u64,
    desc: &RichTextLineDesc,
    colored_quads: &[ColoredGlyphQuad],
    decorations: &[SpanDecoration],
    control: ControlState,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(decorations.len() + colored_quads.len());
    let mut payloads = Vec::with_capacity(colored_quads.len());

    for deco in decorations {
        let fill = if control.disabled {
            Token::SURFACE_SUNKEN
        } else {
            deco.color
        };
        primitives.push(surface(deco.rect, 2.0, fill, desc.z));
    }

    let glyph_z = desc.z.saturating_add(1);
    for colored in colored_quads {
        let color = if control.disabled {
            Token::TEXT_DISABLED
        } else {
            colored.color
        };
        glyphs(
            core::slice::from_ref(&colored.quad),
            color,
            glyph_z,
            &mut primitives,
            &mut payloads,
        );
    }

    let a11y_label = desc.label.clone().unwrap_or_else(|| desc.text.clone());
    let mut a11y = A11yNode::static_text(a11y_label, desc.rect);
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_shape(Shape::Rect)
                .with_cursor(CursorKind::Text),
        ),
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PointerPhase;

    fn sample_quad(x: f32) -> GlyphQuad {
        GlyphQuad {
            rect: Rect::new(x, 2.0, 8.0, 14.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        }
    }

    fn sample_spans() -> Vec<TextSpan> {
        let base = TextStyle::monospace(13.0);
        vec![
            TextSpan {
                range: 0..3,
                style: base.clone(),
                color: Token::PRIMARY,
                underline: false,
                background: None,
                link_id: None,
            },
            TextSpan {
                range: 4..8,
                style: base.clone(),
                color: Token::SUCCESS,
                underline: false,
                background: Some(Token::CODE_BG),
                link_id: None,
            },
            TextSpan {
                range: 11..13,
                style: base,
                color: Token::WARNING,
                underline: true,
                background: None,
                link_id: None,
            },
        ]
    }

    #[test]
    fn color_at_resolves_span_tokens_and_default_gaps() {
        let desc = RichTextLineDesc::new(
            Rect::new(0.0, 0.0, 240.0, 20.0),
            "let name = 42;",
            sample_spans(),
        )
        .with_monospace(true);

        assert_eq!(desc.color_at(0), Token::PRIMARY);
        assert_eq!(desc.color_at(2), Token::PRIMARY);
        assert_eq!(desc.color_at(3), Token::TEXT_PRIMARY);
        assert_eq!(desc.color_at(5), Token::SUCCESS);
        assert_eq!(desc.color_at(11), Token::WARNING);
        assert_eq!(desc.color_at(13), Token::TEXT_PRIMARY);
        assert_eq!(
            desc.effective_base_style().family.as_deref(),
            Some("monospace")
        );
    }

    #[test]
    fn emit_rich_text_line_preserves_per_span_colors_without_monochrome_fallback() {
        let desc = RichTextLineDesc::new(
            Rect::new(0.0, 0.0, 240.0, 20.0),
            "let name = 42;",
            sample_spans(),
        )
        .with_monospace(true);

        let clustered = vec![
            (0u32, sample_quad(0.0)),
            (4u32, sample_quad(32.0)),
            (11u32, sample_quad(88.0)),
        ];
        let colored = desc.resolve_colored_quads(&clustered);
        let decos = vec![SpanDecoration {
            rect: Rect::new(32.0, 0.0, 32.0, 18.0),
            color: Token::CODE_BG,
        }];

        let out = emit_rich_text_line(7, &desc, &colored, &decos, ControlState::default());
        assert_eq!(out.primitives.len(), 4);
        assert_eq!(out.payloads.len(), 3);
        assert_eq!(
            out.primitives[0].fill_token_or_uv,
            u32::from(Token::CODE_BG.raw())
        );
        assert_eq!(
            out.primitives[1].fill_token_or_uv,
            u32::from(Token::PRIMARY.raw())
        );
        assert_eq!(
            out.primitives[2].fill_token_or_uv,
            u32::from(Token::SUCCESS.raw())
        );
        assert_eq!(
            out.primitives[3].fill_token_or_uv,
            u32::from(Token::WARNING.raw())
        );

        // Stable shape across interactive & disabled states.
        let hovered = ControlState {
            phase: PointerPhase::Hovered,
            ..ControlState::default()
        };
        let disabled = ControlState {
            disabled: true,
            ..ControlState::default()
        };
        let out_hovered = emit_rich_text_line(7, &desc, &colored, &decos, hovered);
        let out_disabled = emit_rich_text_line(7, &desc, &colored, &decos, disabled);
        assert_eq!(out_hovered.primitives.len(), out.primitives.len());
        assert_eq!(out_disabled.primitives.len(), out.primitives.len());
        assert_eq!(
            out_disabled.primitives[1].fill_token_or_uv,
            u32::from(Token::TEXT_DISABLED.raw())
        );
    }
}

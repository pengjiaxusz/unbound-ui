//! The three structural containers: card, wrap row and stack.
//!
//! None of them owns state. A card is a rectangle with a header and footer
//! slot, a wrap row is a pure line-breaking function, and a stack is a list of
//! children sharing one rectangle with increasing depth keys. Keeping them
//! stateless means a container can be recomputed from layout without an event
//! loop round-trip.

use unbound_abi::desc::NodeFlags;
use unbound_gpu::data::{GpuPayload, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{instance, outlined, surface};
use crate::WidgetOutput;

/// The elevation steps a card may use.
pub const CARD_ELEVATIONS: [f32; 5] = [0.0, 2.0, 6.0, 12.0, 20.0];

/// The default corner radius of a card.
pub const CARD_RADIUS: f32 = 10.0;

/// The node flag that turns a row into a wrapping row.
pub const WRAP_FLAG: NodeFlags = NodeFlags::WRAP;

/// The pure descriptor of a card.
#[derive(Clone, Debug, PartialEq)]
pub struct CardDesc {
    /// The card rectangle.
    pub rect: Rect,
    /// The corner radius.
    pub radius: f32,
    /// The elevation step, `0..=4`.
    pub elevation: u8,
    /// The header height, when the card has one.
    pub header_height: Option<f32>,
    /// The footer height, when the card has one.
    pub footer_height: Option<f32>,
    /// The draw order key.
    pub z: u16,
}

impl CardDesc {
    /// A flat card.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            radius: CARD_RADIUS,
            elevation: 0,
            header_height: None,
            footer_height: None,
            z: 100,
        }
    }

    /// The same card with a shadow.
    #[must_use]
    pub fn elevated(mut self, elevation: u8) -> Self {
        self.elevation = elevation.min(CARD_ELEVATIONS.len() as u8 - 1);
        self
    }

    /// The same card with a header band.
    #[must_use]
    pub fn with_header(mut self, height: f32) -> Self {
        self.header_height = Some(height.max(0.0));
        self
    }

    /// The same card with a footer band.
    #[must_use]
    pub fn with_footer(mut self, height: f32) -> Self {
        self.footer_height = Some(height.max(0.0));
        self
    }

    /// The shadow blur for the current elevation.
    #[must_use]
    pub fn shadow(&self) -> f32 {
        CARD_ELEVATIONS[self.elevation.min(4) as usize]
    }

    /// The header band, if the card has one.
    #[must_use]
    pub fn header_rect(&self) -> Option<Rect> {
        self.header_height.map(|height| {
            Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                self.rect.size.x,
                height.min(self.rect.size.y),
            )
        })
    }

    /// The footer band, if the card has one.
    #[must_use]
    pub fn footer_rect(&self) -> Option<Rect> {
        self.footer_height.map(|height| {
            let height = height.min(self.rect.size.y);
            Rect::new(
                self.rect.origin.x,
                self.rect.max().y - height,
                self.rect.size.x,
                height,
            )
        })
    }

    /// The content rectangle between the header and footer.
    #[must_use]
    pub fn body_rect(&self) -> Rect {
        let top = self.header_height.unwrap_or(0.0).min(self.rect.size.y);
        let bottom = self.footer_height.unwrap_or(0.0);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + top,
            self.rect.size.x,
            (self.rect.size.y - top - bottom).max(0.0),
        )
    }
}

/// The colour slots a card paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardTokens {
    /// The card surface.
    pub surface: Token,
    /// The card border.
    pub border: Token,
    /// The header band.
    pub header: Token,
    /// The footer band.
    pub footer: Token,
    /// The hairline between the body and its bands.
    pub divider: Token,
}

impl CardTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            surface: Token::CARD_BG,
            border: Token::CARD_BORDER,
            header: Token::SURFACE_RAISED,
            footer: Token::SURFACE_RAISED,
            divider: Token::DIVIDER,
        }
    }
}

/// Emit a card's surface, border, bands and separators.
#[must_use]
pub fn emit_card(desc: &CardDesc, tokens: CardTokens) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5);
    primitives.push(outlined(
        desc.rect,
        desc.radius,
        tokens.surface,
        tokens.border,
        1.0,
        desc.shadow(),
        desc.z,
    ));
    if let Some(header) = desc.header_rect() {
        primitives.push(outlined(
            header,
            desc.radius,
            tokens.header,
            Token::NONE,
            0.0,
            0.0,
            desc.z,
        ));
        primitives.push(hairline(
            Rect::new(header.origin.x, header.max().y - 1.0, header.size.x, 1.0),
            tokens.divider,
            desc.z,
        ));
    }
    if let Some(footer) = desc.footer_rect() {
        primitives.push(outlined(
            footer,
            desc.radius,
            tokens.footer,
            Token::NONE,
            0.0,
            0.0,
            desc.z,
        ));
        primitives.push(hairline(
            Rect::new(footer.origin.x, footer.origin.y, footer.size.x, 1.0),
            tokens.divider,
            desc.z,
        ));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::<GpuPayload>::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// The pure descriptor of a wrapping row.
#[derive(Clone, Debug, PartialEq)]
pub struct WrapDesc {
    /// The row rectangle.
    pub rect: Rect,
    /// The horizontal gap between items.
    pub gap_x: f32,
    /// The vertical gap between lines.
    pub gap_y: f32,
    /// The height of one line.
    pub line_height: f32,
    /// The node flags a caller should apply; always [`WRAP_FLAG`].
    pub flags: NodeFlags,
}

impl WrapDesc {
    /// A wrap row with the default gaps.
    #[must_use]
    pub fn new(rect: Rect, line_height: f32) -> Self {
        Self {
            rect,
            gap_x: 8.0,
            gap_y: 8.0,
            line_height: line_height.max(1.0),
            flags: WRAP_FLAG,
        }
    }

    /// Override the gaps.
    #[must_use]
    pub fn with_gaps(mut self, x: f32, y: f32) -> Self {
        self.gap_x = x.max(0.0);
        self.gap_y = y.max(0.0);
        self
    }

    /// Break a list of item sizes into lines.
    ///
    /// An item wider than the row gets its own line and overflows to the
    /// right, which is the only choice that does not mutate the item's size
    /// behind the caller's back.
    #[must_use]
    pub fn layout(&self, sizes: &[Vec2]) -> Vec<Rect> {
        let mut out = Vec::with_capacity(sizes.len());
        let mut x = self.rect.origin.x;
        let mut y = self.rect.origin.y;
        let mut line_height = self.line_height;
        for size in sizes {
            if x > self.rect.origin.x && x + size.x > self.rect.max().x {
                x = self.rect.origin.x;
                y += line_height + self.gap_y;
                line_height = self.line_height;
            }
            out.push(Rect::new(x, y, size.x, size.y.max(self.line_height)));
            x += size.x + self.gap_x;
            line_height = line_height.max(size.y);
        }
        out
    }

    /// The number of lines the sizes produce.
    #[must_use]
    pub fn line_count(&self, sizes: &[Vec2]) -> usize {
        let mut lines = usize::from(!sizes.is_empty());
        let mut x = 0.0;
        for size in sizes {
            if x > 0.0 && x + size.x > self.rect.size.x {
                lines += 1;
                x = 0.0;
            }
            x += size.x + self.gap_x;
        }
        lines
    }

    /// The total height the wrapped lines need.
    #[must_use]
    pub fn content_height(&self, sizes: &[Vec2]) -> f32 {
        let lines = self.line_count(sizes);
        if lines == 0 {
            0.0
        } else {
            lines as f32 * self.line_height + (lines - 1) as f32 * self.gap_y
        }
    }
}

/// The pure descriptor of a stack.
#[derive(Clone, Debug, PartialEq)]
pub struct StackDesc {
    /// The shared rectangle every child fills.
    pub rect: Rect,
    /// The draw order key of the bottom child.
    pub z: u16,
    /// The depth increment per layer.
    pub z_step: u16,
}

impl StackDesc {
    /// A stack over the given rectangle.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            z: 100,
            z_step: 1,
        }
    }

    /// The rectangle of one layer: every layer shares the stack's box.
    #[must_use]
    pub fn layer_rect(&self, _index: usize) -> Rect {
        self.rect
    }

    /// The draw order key of one layer.
    #[must_use]
    pub fn layer_z(&self, index: usize) -> u16 {
        self.z.saturating_add(
            self.z_step
                .saturating_mul(index.min(u16::MAX as usize) as u16),
        )
    }

    /// Every layer rectangle, bottom first.
    #[must_use]
    pub fn layout(&self, count: usize) -> Vec<Rect> {
        (0..count).map(|index| self.layer_rect(index)).collect()
    }
}

/// Emit a stack's optional background.
#[must_use]
pub fn emit_stack(desc: &StackDesc, fill: Token) -> WidgetOutput {
    WidgetOutput {
        primitives: if fill == Token::NONE {
            Vec::new()
        } else {
            vec![surface(desc.rect, 0.0, fill, desc.z)]
        },
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// A one-pixel separator.
fn hairline(rect: Rect, color: Token, z: u16) -> unbound_gpu::data::GpuPrimitiveInstance {
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
    fn a_card_without_bands_fills_with_its_body() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 300.0, 200.0));
        assert_eq!(desc.body_rect(), desc.rect);
        assert!(desc.header_rect().is_none());
        assert!(desc.footer_rect().is_none());
    }

    #[test]
    fn the_body_sits_between_the_bands() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 300.0, 200.0))
            .with_header(40.0)
            .with_footer(30.0);
        assert_eq!(desc.header_rect().expect("header").size.y, 40.0);
        assert_eq!(desc.footer_rect().expect("footer").origin.y, 170.0);
        let body = desc.body_rect();
        assert_eq!(body.origin.y, 40.0);
        assert_eq!(body.size.y, 130.0);
    }

    #[test]
    fn elevation_maps_to_shadow_blur() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 10.0, 10.0)).elevated(2);
        assert_eq!(desc.shadow(), CARD_ELEVATIONS[2]);
        assert_eq!(desc.clone().elevated(9).shadow(), CARD_ELEVATIONS[4]);
    }

    #[test]
    fn a_card_emits_its_bands_and_separators() {
        let desc = CardDesc::new(Rect::new(0.0, 0.0, 300.0, 200.0))
            .with_header(40.0)
            .with_footer(30.0);
        let output = emit_card(&desc, CardTokens::standard());
        assert_eq!(output.primitives.len(), 5);
        assert!(output.hit.is_none());
    }

    #[test]
    fn wrap_breaks_a_row_at_the_right_edge() {
        let desc = WrapDesc::new(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0);
        let rects = desc.layout(&[
            Vec2::new(40.0, 20.0),
            Vec2::new(40.0, 20.0),
            Vec2::new(40.0, 20.0),
        ]);
        assert_eq!(rects[0].origin, Vec2::new(0.0, 0.0));
        assert_eq!(rects[1].origin, Vec2::new(48.0, 0.0));
        assert_eq!(rects[2].origin, Vec2::new(0.0, 28.0));
        assert_eq!(desc.line_count(&[Vec2::new(40.0, 20.0); 3]), 2);
        assert_eq!(desc.content_height(&[Vec2::new(40.0, 20.0); 3]), 48.0);
    }

    #[test]
    fn an_oversized_item_gets_its_own_line() {
        let desc = WrapDesc::new(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0);
        let rects = desc.layout(&[Vec2::new(200.0, 20.0), Vec2::new(30.0, 20.0)]);
        assert_eq!(rects[0].origin, Vec2::new(0.0, 0.0));
        assert_eq!(rects[1].origin, Vec2::new(0.0, 28.0));
    }

    #[test]
    fn wrap_carries_the_layout_flag() {
        let desc = WrapDesc::new(Rect::new(0.0, 0.0, 100.0, 40.0), 20.0);
        assert_eq!(desc.flags, NodeFlags::WRAP);
    }

    #[test]
    fn a_stack_shares_one_rectangle_and_raises_each_layer() {
        let desc = StackDesc::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let rects = desc.layout(3);
        assert_eq!(rects.len(), 3);
        assert!(rects.iter().all(|rect| *rect == desc.rect));
        assert_eq!(desc.layer_z(0), 100);
        assert_eq!(desc.layer_z(1), 101);
        assert_eq!(desc.layer_z(2), 102);
    }

    #[test]
    fn an_empty_stack_paints_nothing() {
        let desc = StackDesc::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        let output = emit_stack(&desc, Token::NONE);
        assert!(output.primitives.is_empty());
    }
}

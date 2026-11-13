//! The colour field: HSV state, hex / rgb / oklch text, and a picker surface.
//!
//! Colour is stored as [`Hsva`] because that is what a picker manipulates — a
//! hue ring, a saturation/value pad and an alpha track. Conversion to linear
//! [`Color`] happens at the edge, so the rest of the engine keeps its single
//! colour type.
//!
//! The emitter paints the pad, the two tracks and the preset swatches. It does
//! **not** paint the pad's spectrum: that needs a gradient fill (P3.2) or a
//! generated atlas. Until then the pad is a sunken surface plus the selection
//! ring, and the caller can layer a texture underneath without touching the
//! state machine.

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::color::linear_to_srgb;
use unbound_math::{Color, Rect, Srgba};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::TextEditState;
use crate::{ControlState, WidgetOutput};

/// The inset between the field border and its content.
pub const PICKER_PADDING: f32 = 10.0;
/// The height of the hue / alpha track.
pub const PICKER_TRACK: f32 = 10.0;
/// The gap between stacked parts.
pub const PICKER_GAP: f32 = 8.0;
/// The radius of the picker surface.
pub const PICKER_RADIUS: f32 = 8.0;
/// The preset swatch size.
pub const SWATCH_SIZE: f32 = 18.0;

/// Hue, saturation, value and alpha. Hue is in degrees `0..360`, the rest in
/// `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsva {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
}

impl Hsva {
    /// A colour, normalised into range.
    #[must_use]
    pub fn new(h: f32, s: f32, v: f32, a: f32) -> Self {
        Self {
            h: h.rem_euclid(360.0),
            s: s.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    /// The sRGB bytes, for hex formatting and tests.
    #[must_use]
    pub fn to_srgb8(self) -> Srgba {
        let c = self.v * self.s;
        let h = self.h / 60.0;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        let byte = |value: f32| ((value + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        Srgba::new(byte(r), byte(g), byte(b), (self.a * 255.0).round() as u8)
    }

    /// Convert to a linear [`Color`].
    #[must_use]
    pub fn to_color(self) -> Color {
        let srgb = self.to_srgb8();
        Color::from_srgb8(srgb.r, srgb.g, srgb.b, srgb.a)
    }

    /// Derive the HSV representation of a linear colour.
    #[must_use]
    pub fn from_color(color: Color) -> Self {
        let r = linear_to_srgb(color.r.clamp(0.0, 1.0));
        let g = linear_to_srgb(color.g.clamp(0.0, 1.0));
        let b = linear_to_srgb(color.b.clamp(0.0, 1.0));
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let h = if delta <= f32::EPSILON {
            0.0
        } else if max == r {
            60.0 * (((g - b) / delta).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        let s = if max <= f32::EPSILON {
            0.0
        } else {
            delta / max
        };
        Self::new(h, s, max, color.a)
    }

    /// `#rrggbb` or `#rrggbbaa` when the colour is translucent.
    #[must_use]
    pub fn hex(self) -> String {
        let srgb = self.to_srgb8();
        if srgb.a == 255 {
            format!("#{:02x}{:02x}{:02x}", srgb.r, srgb.g, srgb.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", srgb.r, srgb.g, srgb.b, srgb.a)
        }
    }
}

/// How the textual representation is written and parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorFormat {
    /// `#rrggbb` / `#rrggbbaa`.
    #[default]
    Hex,
    /// `rgba(r, g, b, a)` with byte channels and a `0..=1` alpha.
    Rgba,
    /// `oklch(l c h / a)`.
    Oklch,
}

impl ColorFormat {
    /// The next format in the cycle.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Hex => Self::Rgba,
            Self::Rgba => Self::Oklch,
            Self::Oklch => Self::Hex,
        }
    }

    /// Write `hsva` in this format.
    #[must_use]
    pub fn format(self, hsva: Hsva) -> String {
        match self {
            Self::Hex => hsva.hex(),
            Self::Rgba => {
                let c = hsva.to_srgb8();
                format!("rgba({}, {}, {}, {:.2})", c.r, c.g, c.b, hsva.a)
            }
            Self::Oklch => {
                let lab = hsva.to_color().to_oklab();
                let c = lab.a.hypot(lab.b);
                let h = lab.b.atan2(lab.a).to_degrees().rem_euclid(360.0);
                format!("oklch({:.3} {:.3} {:.1} / {:.2})", lab.l, c, h, lab.alpha)
            }
        }
    }

    /// Parse text in this format.
    #[must_use]
    pub fn parse(self, text: &str) -> Option<Hsva> {
        match self {
            Self::Hex => parse_hex(text),
            Self::Rgba => parse_rgba(text),
            Self::Oklch => parse_oklch(text),
        }
    }
}

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`, with or without the hash.
#[must_use]
pub fn parse_hex(text: &str) -> Option<Hsva> {
    let digits = text.trim().trim_start_matches('#');
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let pair = |i: usize| -> Option<u8> {
        let hi = nibble(*digits.as_bytes().get(i)?)?;
        let lo = nibble(*digits.as_bytes().get(i + 1)?)?;
        Some(hi << 4 | lo)
    };
    let bytes = digits.as_bytes();
    let (r, g, b, a) = match bytes.len() {
        3 => {
            let r = nibble(bytes[0])?;
            let g = nibble(bytes[1])?;
            let b = nibble(bytes[2])?;
            (r * 17, g * 17, b * 17, 255)
        }
        6 => (pair(0)?, pair(2)?, pair(4)?, 255),
        8 => (pair(0)?, pair(2)?, pair(4)?, pair(6)?),
        _ => return None,
    };
    Some(Hsva::from_color(Color::from_srgb8(r, g, b, a)))
}

/// Parse `rgb(r, g, b)` or `rgba(r, g, b, a)`; alpha is `0..=1` or `0..=255`.
#[must_use]
pub fn parse_rgba(text: &str) -> Option<Hsva> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close <= open {
        return None;
    }
    let mut parts = text[open + 1..close]
        .split([',', ' '])
        .filter(|part| !part.trim().is_empty());
    let mut channel = || -> Option<u8> {
        let value: f32 = parts.next()?.trim().trim_end_matches('%').parse().ok()?;
        Some(value.round().clamp(0.0, 255.0) as u8)
    };
    let r = channel()?;
    let g = channel()?;
    let b = channel()?;
    let a = match parts.next() {
        None => 255,
        Some(raw) => {
            let value: f32 = raw.trim().parse().ok()?;
            if value <= 1.0 {
                (value * 255.0).round() as u8
            } else {
                value.round().clamp(0.0, 255.0) as u8
            }
        }
    };
    Some(Hsva::from_color(Color::from_srgb8(r, g, b, a)))
}

/// Parse `oklch(l c h)` with an optional `/ alpha`.
#[must_use]
pub fn parse_oklch(text: &str) -> Option<Hsva> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close <= open {
        return None;
    }
    let body = &text[open + 1..close];
    let (channels, alpha) = match body.split_once('/') {
        Some((left, right)) => (left, Some(right.trim().parse::<f32>().ok()?)),
        None => (body, None),
    };
    let mut parts = channels.split_whitespace();
    let l_raw = parts.next()?;
    let l = if let Some(percent) = l_raw.strip_suffix('%') {
        percent.parse::<f32>().ok()? / 100.0
    } else {
        l_raw.parse::<f32>().ok()?
    };
    let c: f32 = parts.next()?.parse().ok()?;
    let h: f32 = parts.next()?.trim_end_matches("deg").parse().ok()?;
    let alpha = alpha.unwrap_or(1.0);
    let lab = unbound_math::Oklab {
        l: l.clamp(0.0, 1.0),
        a: c * h.to_radians().cos(),
        b: c * h.to_radians().sin(),
        alpha: alpha.clamp(0.0, 1.0),
    };
    Some(Hsva::from_color(lab.to_linear()))
}

/// The pure descriptor of a colour field.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorPickerDesc {
    /// The field rectangle.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The field refuses interaction.
    pub disabled: bool,
    /// The text format used by the inline editor.
    pub format: ColorFormat,
    /// Preset theme colours shown below the tracks.
    pub swatches: Vec<Token>,
    /// The draw order key.
    pub z: u16,
}

impl ColorPickerDesc {
    /// A field with the default metrics and six presets.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            disabled: false,
            format: ColorFormat::Hex,
            swatches: vec![
                Token::PRIMARY,
                Token::SECONDARY,
                Token::ACCENT,
                Token::SUCCESS,
                Token::WARNING,
                Token::ERROR,
            ],
            z: 735,
        }
    }
}

/// Which sub-control a pointer gesture targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorPart {
    /// The saturation / value pad.
    Sv,
    /// The hue track.
    Hue,
    /// The alpha track.
    Alpha,
}

/// The measured sub-rectangles of a picker.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorPickerLayout {
    /// The saturation / value pad.
    pub sv: Rect,
    /// The hue track.
    pub hue: Rect,
    /// The alpha track.
    pub alpha: Rect,
    /// The preset swatches.
    pub swatches: Vec<Rect>,
}

impl ColorPickerDesc {
    /// Compute the sub-rectangles.
    #[must_use]
    pub fn layout(&self) -> ColorPickerLayout {
        let content = self.rect.inflate(-PICKER_PADDING);
        let track = PICKER_TRACK;
        let swatch_row = if self.swatches.is_empty() {
            0.0
        } else {
            SWATCH_SIZE + PICKER_GAP
        };
        let sv_height = (content.size.y - track * 2.0 - PICKER_GAP * 2.0 - swatch_row).max(0.0);
        let sv = Rect::new(
            content.origin.x,
            content.origin.y,
            content.size.x,
            sv_height,
        );
        let hue = Rect::new(
            content.origin.x,
            sv.max().y + PICKER_GAP,
            content.size.x,
            track,
        );
        let alpha = Rect::new(
            content.origin.x,
            hue.max().y + PICKER_GAP,
            content.size.x,
            track,
        );
        let swatches = (0..self.swatches.len())
            .map(|index| {
                Rect::new(
                    content.origin.x + index as f32 * (SWATCH_SIZE + PICKER_GAP),
                    alpha.max().y + PICKER_GAP,
                    SWATCH_SIZE,
                    SWATCH_SIZE,
                )
            })
            .collect();
        ColorPickerLayout {
            sv,
            hue,
            alpha,
            swatches,
        }
    }
}

/// The live state of a colour field.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorPickerState {
    hsva: Hsva,
    format: ColorFormat,
    active: Option<ColorPart>,
    editor: TextEditState,
}

impl ColorPickerState {
    /// A picker holding `color`.
    #[must_use]
    pub fn new(color: Color) -> Self {
        Self {
            hsva: Hsva::from_color(color),
            format: ColorFormat::Hex,
            active: None,
            editor: TextEditState::new(),
        }
    }

    /// The current colour.
    #[must_use]
    pub fn hsva(&self) -> Hsva {
        self.hsva
    }

    /// The current colour as linear RGBA.
    #[must_use]
    pub fn color(&self) -> Color {
        self.hsva.to_color()
    }

    /// The active gesture target.
    #[must_use]
    pub fn active(&self) -> Option<ColorPart> {
        self.active
    }

    /// The text format.
    #[must_use]
    pub fn format(&self) -> ColorFormat {
        self.format
    }

    /// Replace the colour.
    pub fn set_hsva(&mut self, hsva: Hsva) {
        self.hsva = hsva;
    }

    /// Replace the colour from a linear value.
    pub fn set_color(&mut self, color: Color) {
        self.hsva = Hsva::from_color(color);
    }

    /// Cycle the text format.
    pub fn cycle_format(&mut self) {
        self.format = self.format.next();
    }

    /// Map a pointer position onto a sub-control.
    pub fn set_from_pointer(&mut self, part: ColorPart, x: f32, y: f32, rect: Rect) {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
            return;
        }
        let u = ((x - rect.origin.x) / rect.size.x).clamp(0.0, 1.0);
        match part {
            ColorPart::Sv => {
                let v = 1.0 - ((y - rect.origin.y) / rect.size.y).clamp(0.0, 1.0);
                self.hsva.s = u;
                self.hsva.v = v;
            }
            ColorPart::Hue => self.hsva.h = (u * 360.0).rem_euclid(360.0),
            ColorPart::Alpha => self.hsva.a = u,
        }
    }

    /// Begin dragging a sub-control.
    pub fn begin(&mut self, part: ColorPart) {
        self.active = Some(part);
    }

    /// End the gesture.
    pub fn end(&mut self) {
        self.active = None;
    }

    /// The inline text editor.
    #[must_use]
    pub fn editor(&self) -> &TextEditState {
        &self.editor
    }

    /// The text the caller should shape.
    #[must_use]
    pub fn text(&self) -> String {
        self.format.format(self.hsva)
    }

    /// Open the text editor with the current representation.
    pub fn begin_edit(&mut self) {
        self.editor = TextEditState::with_text(self.format.format(self.hsva));
    }

    /// Parse the editor's contents into the colour.
    pub fn commit_edit(&mut self) -> bool {
        let Some(hsva) = self.format.parse(self.editor.text()) else {
            return false;
        };
        self.hsva = hsva;
        true
    }
}

/// Emit the picker chrome, tracks, presets and semantic node.
#[must_use]
pub fn emit_color_picker(
    node: u64,
    desc: &ColorPickerDesc,
    state: &ColorPickerState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let layout = desc.layout();
    let z = desc.z;
    let mut primitives = Vec::with_capacity(24 + desc.swatches.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else {
        Token::CARD_BG
    };
    primitives.push(surface(rect, PICKER_RADIUS, fill, z));
    primitives.push(outlined(
        rect,
        PICKER_RADIUS,
        Token::NONE,
        if control.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        z,
    ));

    // The pad and tracks. The spectrum itself is layered by the caller until
    // the gradient fill lands, so these are deliberately flat surfaces.
    primitives.push(surface(layout.sv, 4.0, Token::SURFACE_SUNKEN, z));
    primitives.push(surface(
        layout.hue,
        PICKER_TRACK * 0.5,
        Token::CONTROL_TRACK,
        z,
    ));
    primitives.push(surface(
        layout.alpha,
        PICKER_TRACK * 0.5,
        Token::SURFACE_SUNKEN,
        z,
    ));
    // Alpha checkerboard: four alternating cells read as transparency even
    // without a texture.
    for index in 0..4u16 {
        let cell = layout.alpha.size.x / 4.0;
        let x = layout.alpha.origin.x + f32::from(index) * cell;
        if index % 2 == 0 {
            primitives.push(surface(
                Rect::new(x, layout.alpha.origin.y, cell, layout.alpha.size.y),
                0.0,
                Token::SURFACE,
                z,
            ));
        }
    }

    let mark = |center_x: f32, center_y: f32| {
        outlined(
            Rect::new(center_x - 5.0, center_y - 5.0, 10.0, 10.0),
            5.0,
            Token::NONE,
            Token::ON_PRIMARY,
            2.0,
            0.0,
            z,
        )
    };
    primitives.push(mark(
        layout.sv.origin.x + layout.sv.size.x * state.hsva.s,
        layout.sv.origin.y + layout.sv.size.y * (1.0 - state.hsva.v),
    ));
    primitives.push(mark(
        layout.hue.origin.x + layout.hue.size.x * (state.hsva.h / 360.0),
        layout.hue.center().y,
    ));
    primitives.push(mark(
        layout.alpha.origin.x + layout.alpha.size.x * state.hsva.a,
        layout.alpha.center().y,
    ));

    for (token, swatch) in desc.swatches.iter().zip(&layout.swatches) {
        primitives.push(surface(*swatch, 4.0, *token, z));
        primitives.push(outlined(
            *swatch,
            4.0,
            Token::NONE,
            Token::CARD_BORDER,
            1.0,
            0.0,
            z,
        ));
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_SECONDARY,
        z,
        &mut primitives,
        &mut payloads,
    );

    primitives.push(focus_ring(
        rect.inflate(3.0),
        PICKER_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [PICKER_RADIUS; 4],
            })
            .with_cursor(CursorKind::Crosshair)
    });
    let mut a11y = A11yNode::new(Role::ColorWell, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.hsva.hex();
    a11y.actions.insert(A11yAction::SetValue);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_primaries_convert_to_rgb() {
        assert_eq!(
            Hsva::new(0.0, 1.0, 1.0, 1.0).to_srgb8(),
            Srgba::new(255, 0, 0, 255)
        );
        assert_eq!(
            Hsva::new(120.0, 1.0, 1.0, 1.0).to_srgb8(),
            Srgba::new(0, 255, 0, 255)
        );
        assert_eq!(
            Hsva::new(240.0, 1.0, 1.0, 1.0).to_srgb8(),
            Srgba::new(0, 0, 255, 255)
        );
        assert_eq!(
            Hsva::new(0.0, 0.0, 0.5, 1.0).to_srgb8(),
            Srgba::new(128, 128, 128, 255)
        );
    }

    #[test]
    fn converting_from_a_color_round_trips() {
        let color = Color::from_srgb8(24, 128, 200, 255);
        let hsva = Hsva::from_color(color);
        assert_eq!(hsva.to_srgb8(), Srgba::new(24, 128, 200, 255));
        assert_eq!(hsva.hex(), "#1880c8");
    }

    #[test]
    fn hex_accepts_three_six_and_eight_digits() {
        assert_eq!(parse_hex("#fff").map(Hsva::hex), Some("#ffffff".into()));
        assert_eq!(parse_hex("1880c8").map(Hsva::hex), Some("#1880c8".into()));
        assert_eq!(
            parse_hex("#1880c880").map(Hsva::to_srgb8),
            Some(Srgba::new(24, 128, 200, 128))
        );
        assert!(parse_hex("#12").is_none());
        assert!(parse_hex("#gggggg").is_none());
    }

    #[test]
    fn rgba_and_oklch_parse_and_format() {
        let rgba = parse_rgba("rgba(255, 0, 0, 0.5)").expect("parses");
        assert_eq!(rgba.to_srgb8(), Srgba::new(255, 0, 0, 128));

        let picked = Hsva::new(200.0, 0.6, 0.7, 1.0);
        let text = ColorFormat::Oklch.format(picked);
        let parsed = ColorFormat::Oklch.parse(&text).expect("round trips");
        assert!((parsed.to_srgb8().r as i32 - picked.to_srgb8().r as i32).abs() <= 1);
        assert!((parsed.to_srgb8().b as i32 - picked.to_srgb8().b as i32).abs() <= 1);
        assert!((parsed.h - picked.h).abs() <= 2.0);
    }

    #[test]
    fn format_cycles_through_three_representations() {
        assert_eq!(ColorFormat::Hex.next(), ColorFormat::Rgba);
        assert_eq!(ColorFormat::Rgba.next(), ColorFormat::Oklch);
        assert_eq!(ColorFormat::Oklch.next(), ColorFormat::Hex);
    }

    #[test]
    fn the_pointer_maps_onto_the_pad_and_tracks() {
        let mut state = ColorPickerState::new(Color::from_srgb8(0, 0, 0, 255));
        let pad = Rect::new(0.0, 0.0, 100.0, 50.0);
        state.begin(ColorPart::Sv);
        state.set_from_pointer(ColorPart::Sv, 25.0, 10.0, pad);
        assert!((state.hsva().s - 0.25).abs() < 1e-6);
        assert!((state.hsva().v - 0.8).abs() < 1e-6);
        state.set_from_pointer(ColorPart::Hue, 75.0, 0.0, Rect::new(0.0, 0.0, 100.0, 10.0));
        assert!((state.hsva().h - 270.0).abs() < 1e-4);
        state.set_from_pointer(
            ColorPart::Alpha,
            40.0,
            0.0,
            Rect::new(0.0, 0.0, 100.0, 10.0),
        );
        assert!((state.hsva().a - 0.4).abs() < 1e-6);
        state.end();
        assert_eq!(state.active(), None);
    }

    #[test]
    fn the_editor_commits_only_valid_text() {
        let mut state = ColorPickerState::new(Color::from_srgb8(0, 0, 0, 255));
        state.begin_edit();
        assert_eq!(state.text(), "#000000");
        state.editor = TextEditState::with_text("#ff0000");
        assert!(state.commit_edit());
        assert_eq!(state.hsva().to_srgb8(), Srgba::new(255, 0, 0, 255));
        state.editor = TextEditState::with_text("nope");
        assert!(!state.commit_edit());
        assert_eq!(state.hsva().to_srgb8(), Srgba::new(255, 0, 0, 255));
    }

    #[test]
    fn the_layout_stacks_pad_tracks_and_swatches() {
        let desc = ColorPickerDesc::new(Rect::new(0.0, 0.0, 220.0, 200.0));
        let layout = desc.layout();
        assert!(layout.sv.size.y > 0.0);
        assert!(layout.sv.max().y < layout.hue.origin.y);
        assert!(layout.hue.max().y <= layout.alpha.origin.y);
        assert_eq!(layout.swatches.len(), desc.swatches.len());
        assert!(layout.swatches[0].origin.x >= PICKER_PADDING);
    }

    #[test]
    fn the_emitter_reports_a_color_well() {
        let state = ColorPickerState::new(Color::from_srgb8(0, 0, 0, 255));
        let output = emit_color_picker(
            7,
            &ColorPickerDesc::new(Rect::new(0.0, 0.0, 220.0, 200.0)),
            &state,
            ControlState::default(),
            &mut [],
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::ColorWell);
        assert_eq!(a11y.value, "#000000");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Crosshair))
        );
    }
}

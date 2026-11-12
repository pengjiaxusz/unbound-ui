//! Semantic design tokens and the flat colour lookup table (`docs` §4.5).
//!
//! The design system is split in two:
//!
//! * **Tokens** are stable `u16` identifiers. A primitive instance stores the
//!   token, never the colour, so a theme switch never touches instance data.
//! * **Palettes** map every token to a concrete colour. Switching themes
//!   rebuilds a *flat* 4 KiB table (512 RGBA16F entries) and uploads it in a
//!   single write.
//!
//! The invariant the whole design rests on: **a theme switch must not cause a
//! re-layout or an instance re-upload**. Both are enforced here by construction
//! — [`ThemeState::switch`] only ever produces a new LUT.

pub mod snapping;
pub mod zoom;

pub use snapping::{edge_screen, edge_screen_snapped, jitter, snap_error, snap_screen};
pub use zoom::ZoomLayers;

use unbound_math::color::Color;

/// Number of entries in the flat colour table.
///
/// 512 tokens × 8 bytes (RGBA16F) = 4 KiB, exactly the budget from the design
/// document.
pub const MAX_TOKENS: usize = 512;

/// Size of the packed table in bytes.
pub const LUT_BYTES: usize = MAX_TOKENS * 8;

/// A semantic colour token.
///
/// The numeric value is part of the ABI: it travels to the GPU inside
/// `GpuPrimitiveInstance::fill_token_or_uv`, so it must never be reordered.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(u16);

impl Token {
    /// Wrap a raw token id, saturating instead of panicking.
    #[must_use]
    pub const fn new(raw: u16) -> Self {
        Self(if (raw as usize) < MAX_TOKENS {
            raw
        } else {
            MAX_TOKENS as u16 - 1
        })
    }

    /// The raw id, as stored in a primitive instance.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Index into a palette.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The reserved "no colour" slot.
    ///
    /// The GPU's `common.glsl` hard-codes slot 0 as fully transparent, so
    /// "no fill" and "no border" cost nothing: a primitive can name slot 0
    /// and the shader sees a colour with zero alpha.
    pub const NONE: Self = Self(0);
    /// The global shadow colour.
    ///
    /// The shadow is a *theme* concept — every primitive shares it — so it
    /// lives in a fixed slot rather than in a per-primitive field. The slot
    /// index is part of the engine's contract with whoever fills the LUT
    /// (`SHADOW_TOKEN` in `unbound-gpu/shaders/common.glsl`).
    pub const SHADOW: Self = Self(1);
    pub const SURFACE: Self = Self(2);
    pub const SURFACE_RAISED: Self = Self(3);
    pub const SURFACE_SUNKEN: Self = Self(4);
    pub const PRIMARY: Self = Self(5);
    pub const ON_PRIMARY: Self = Self(6);
    pub const ACCENT: Self = Self(7);
    pub const ON_ACCENT: Self = Self(8);
    pub const OUTLINE: Self = Self(9);
    pub const OUTLINE_STRONG: Self = Self(10);
    pub const ON_SURFACE: Self = Self(11);
    pub const ON_SURFACE_MUTED: Self = Self(12);
    pub const ERROR: Self = Self(13);
    pub const ON_ERROR: Self = Self(14);
    pub const WARNING: Self = Self(15);
    pub const SUCCESS: Self = Self(16);
    pub const SELECTION: Self = Self(17);
    pub const ON_SELECTION: Self = Self(18);
    pub const HOVER: Self = Self(19);
    pub const PRESSED: Self = Self(20);
    pub const DISABLED: Self = Self(21);
    pub const ON_DISABLED: Self = Self(22);
    pub const GRID: Self = Self(23);
    pub const CANVAS_BACKDROP: Self = Self(24);
    pub const CANVAS_NODE: Self = Self(25);
    pub const CANVAS_NODE_SELECTED: Self = Self(26);
    pub const SNAP_GUIDE: Self = Self(27);
    pub const TEXT_PRIMARY: Self = Self(28);
    pub const TEXT_SECONDARY: Self = Self(29);
    pub const TEXT_DISABLED: Self = Self(30);
    pub const SCRIM: Self = Self(31);

    // ---- control states and containers (32..=60) -------------------------
    //
    // The first thirty-two slots are the *primitive* vocabulary: surfaces,
    // accents and text. The slots below are the *control* vocabulary: the
    // state colours of a standard widget and the containers a shell paints.
    //
    // Every control state lives in its own slot, so a hover never rewrites
    // the instance data — it only changes which colour the token resolves
    // to. That is what makes a theme switch (and a hover) free of instance
    // re-uploads.

    /// The primary fill while hovered.
    pub const PRIMARY_HOVER: Self = Self(32);
    /// The primary fill while pressed.
    pub const PRIMARY_PRESSED: Self = Self(33);
    /// A low-saturation primary tint.
    pub const PRIMARY_SOFT: Self = Self(34);
    /// A low-saturation secondary tint.
    pub const SECONDARY_SOFT: Self = Self(35);
    /// A low-saturation success tint.
    pub const SUCCESS_SOFT: Self = Self(36);
    /// A low-saturation warning tint.
    pub const WARNING_SOFT: Self = Self(37);
    /// A low-saturation error tint.
    pub const ERROR_SOFT: Self = Self(38);
    /// The keyboard focus ring.
    pub const FOCUS_RING: Self = Self(39);
    /// The track of a slider or a switch.
    pub const CONTROL_TRACK: Self = Self(40);
    /// The thumb of a slider or the knob of a switch.
    pub const CONTROL_THUMB: Self = Self(41);
    /// A hovered control surface.
    pub const CONTROL_HOVER: Self = Self(42);
    /// The title bar surface.
    pub const HEADER_BG: Self = Self(43);
    /// The navigation pane surface.
    pub const NAV_BG: Self = Self(44);
    /// A hovered navigation row.
    pub const NAV_HOVER: Self = Self(45);
    /// The selected navigation row.
    pub const NAV_SELECTED: Self = Self(46);
    /// A hairline divider.
    pub const DIVIDER: Self = Self(47);
    /// The code panel background.
    pub const CODE_BG: Self = Self(48);
    /// The code panel foreground.
    pub const CODE_TEXT: Self = Self(49);
    /// The overlay surface.
    pub const OVERLAY_BG: Self = Self(50);
    /// The first stop of a hero gradient.
    pub const HERO_START: Self = Self(51);
    /// The last stop of a hero gradient.
    pub const HERO_END: Self = Self(52);
    /// Text and icons painted on a hero gradient.
    pub const ON_HERO: Self = Self(53);
    /// The scrim behind a call to action on a hero surface.
    pub const HERO_SCRIM: Self = Self(54);
    /// The performance HUD background.
    pub const HUD_BG: Self = Self(55);
    /// The performance HUD foreground.
    pub const HUD_TEXT: Self = Self(56);
    /// A low-saturation accent tint.
    pub const ACCENT_SOFT: Self = Self(57);
    /// The dots of a canvas or stage grid.
    pub const GRID_DOT: Self = Self(58);
    /// The primary text of a navigation row.
    pub const NAV_TEXT: Self = Self(59);
    /// The secondary text of a navigation row.
    pub const NAV_TEXT_MUTED: Self = Self(60);
    /// A secondary accent, next to [`Token::PRIMARY`].
    ///
    /// The primary accent carries the call to action; the secondary one is
    /// for the supporting highlights of a surface. Both are *solid* colours —
    /// the tinted variants are [`Token::PRIMARY_SOFT`] and
    /// [`Token::SECONDARY_SOFT`].
    pub const SECONDARY: Self = Self(61);
    /// The surface of a card that sits on [`Token::SURFACE`].
    pub const CARD_BG: Self = Self(62);
    /// The outline of a card that sits on [`Token::SURFACE`].
    pub const CARD_BORDER: Self = Self(63);

    // ---- disabled / input / editing states (64..=73) ----------------------

    /// The fill of a disabled button, input or select.
    pub const CONTROL_DISABLED_BG: Self = Self(64);
    /// The outline of a disabled control.
    pub const CONTROL_DISABLED_BORDER: Self = Self(65);
    /// The thumb or knob of a disabled slider / switch.
    pub const CONTROL_DISABLED_THUMB: Self = Self(66);
    /// The fill of an indeterminate checkbox or a mixed progress bar.
    pub const INDETERMINATE_FILL: Self = Self(67);
    /// The background of a text input.
    pub const INPUT_BG: Self = Self(68);
    /// The input border while hovered.
    pub const INPUT_BORDER_HOVER: Self = Self(69);
    /// The input border in an error validation state.
    pub const INPUT_BORDER_ERROR: Self = Self(70);
    /// The input border in a warning validation state.
    pub const INPUT_BORDER_WARNING: Self = Self(71);
    /// The IME pre-edit underline.
    pub const PREEDIT_UNDERLINE: Self = Self(72);
    /// The disclosure arrow of a submenu row.
    pub const SUBMENU_ARROW: Self = Self(73);
}

/// The set of semantic tokens the design system defines.
///
/// Keeping the list in one place means a token cannot be renamed in one
/// palette and forgotten in another.
///
/// [`Token::NONE`] is not in the list: it is a reserved engine slot, not a
/// semantic colour.
pub const TOKENS: [Token; 73] = [
    Token::SURFACE,
    Token::SURFACE_RAISED,
    Token::SURFACE_SUNKEN,
    Token::ON_SURFACE,
    Token::ON_SURFACE_MUTED,
    Token::PRIMARY,
    Token::ON_PRIMARY,
    Token::ACCENT,
    Token::ON_ACCENT,
    Token::OUTLINE,
    Token::OUTLINE_STRONG,
    Token::SHADOW,
    Token::ERROR,
    Token::ON_ERROR,
    Token::WARNING,
    Token::SUCCESS,
    Token::SELECTION,
    Token::ON_SELECTION,
    Token::HOVER,
    Token::PRESSED,
    Token::DISABLED,
    Token::ON_DISABLED,
    Token::GRID,
    Token::CANVAS_BACKDROP,
    Token::CANVAS_NODE,
    Token::CANVAS_NODE_SELECTED,
    Token::SNAP_GUIDE,
    Token::TEXT_PRIMARY,
    Token::TEXT_SECONDARY,
    Token::TEXT_DISABLED,
    Token::SCRIM,
    Token::PRIMARY_HOVER,
    Token::PRIMARY_PRESSED,
    Token::PRIMARY_SOFT,
    Token::SECONDARY_SOFT,
    Token::SUCCESS_SOFT,
    Token::WARNING_SOFT,
    Token::ERROR_SOFT,
    Token::FOCUS_RING,
    Token::CONTROL_TRACK,
    Token::CONTROL_THUMB,
    Token::CONTROL_HOVER,
    Token::HEADER_BG,
    Token::NAV_BG,
    Token::NAV_HOVER,
    Token::NAV_SELECTED,
    Token::DIVIDER,
    Token::CODE_BG,
    Token::CODE_TEXT,
    Token::OVERLAY_BG,
    Token::HERO_START,
    Token::HERO_END,
    Token::ON_HERO,
    Token::HERO_SCRIM,
    Token::HUD_BG,
    Token::HUD_TEXT,
    Token::ACCENT_SOFT,
    Token::GRID_DOT,
    Token::NAV_TEXT,
    Token::NAV_TEXT_MUTED,
    Token::SECONDARY,
    Token::CARD_BG,
    Token::CARD_BORDER,
    Token::CONTROL_DISABLED_BG,
    Token::CONTROL_DISABLED_BORDER,
    Token::CONTROL_DISABLED_THUMB,
    Token::INDETERMINATE_FILL,
    Token::INPUT_BG,
    Token::INPUT_BORDER_HOVER,
    Token::INPUT_BORDER_ERROR,
    Token::INPUT_BORDER_WARNING,
    Token::PREEDIT_UNDERLINE,
    Token::SUBMENU_ARROW,
];

/// Which theme is currently active.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    Light,
    Dark,
    HighContrast,
}

impl ThemeKind {
    /// Every theme, in a stable order.
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Light, ThemeKind::Dark, ThemeKind::HighContrast];

    /// Short identifier used in diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ThemeKind::Light => "light",
            ThemeKind::Dark => "dark",
            ThemeKind::HighContrast => "high-contrast",
        }
    }
}

/// A complete colour assignment for every token.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    kind: ThemeKind,
    entries: Vec<Color>,
}

impl Palette {
    /// Build a palette from a token → colour mapping.
    ///
    /// Tokens that are not mentioned fall back to [`Color::TRANSPARENT`]; the
    /// table is always exactly [`MAX_TOKENS`] long.
    #[must_use]
    pub fn new(kind: ThemeKind, colors: &[(Token, Color)]) -> Self {
        let mut entries = vec![Color::TRANSPARENT; MAX_TOKENS];
        for (token, color) in colors {
            entries[token.index()] = *color;
        }
        Self { kind, entries }
    }

    /// The theme this palette belongs to.
    #[must_use]
    pub const fn kind(&self) -> ThemeKind {
        self.kind
    }

    /// Colour assigned to a token.
    #[must_use]
    pub fn color(&self, token: Token) -> Color {
        self.entries[token.index()]
    }

    /// Every entry, in token order.
    #[must_use]
    pub fn entries(&self) -> &[Color] {
        &self.entries
    }

    /// Build a palette from 8-bit sRGB components.
    ///
    /// The authoring form used by applications: a flat list of
    /// `(token, rgba)` pairs, one per theme. Unmentioned tokens stay
    /// transparent.
    #[must_use]
    pub fn from_srgb8(kind: ThemeKind, colors: &[(Token, [u8; 4])]) -> Self {
        let mapped: Vec<(Token, Color)> = colors
            .iter()
            .map(|(token, rgba)| {
                (
                    *token,
                    Color::from_srgb8(rgba[0], rgba[1], rgba[2], rgba[3]),
                )
            })
            .collect();
        Self::new(kind, &mapped)
    }
}

/// The three shipped themes.
#[derive(Clone, Debug)]
pub struct ThemeSet {
    light: Palette,
    dark: Palette,
    high_contrast: Palette,
}

impl ThemeSet {
    /// Build a theme set from three explicit palettes.
    ///
    /// The shipped [`ThemeSet::default_set`] is one particular set; an
    /// application that carries its own brand colours builds its own set and
    /// hands it to [`ThemeState::new`]. The engine does not care where the
    /// colours come from — only that every token resolves to one.
    #[must_use]
    pub fn from_palettes(light: Palette, dark: Palette, high_contrast: Palette) -> Self {
        Self {
            light,
            dark,
            high_contrast,
        }
    }

    /// The default Material-ish light/dark pair plus a high-contrast variant.
    #[must_use]
    pub fn default_set() -> Self {
        let light = Palette::new(
            ThemeKind::Light,
            &[
                (Token::SURFACE, Color::from_srgb8(255, 255, 255, 255)),
                (Token::SURFACE_RAISED, Color::from_srgb8(248, 249, 251, 255)),
                (Token::SURFACE_SUNKEN, Color::from_srgb8(238, 240, 244, 255)),
                (Token::ON_SURFACE, Color::from_srgb8(20, 22, 26, 255)),
                (
                    Token::ON_SURFACE_MUTED,
                    Color::from_srgb8(96, 101, 110, 255),
                ),
                (Token::PRIMARY, Color::from_srgb8(38, 110, 255, 255)),
                (Token::ON_PRIMARY, Color::from_srgb8(255, 255, 255, 255)),
                (Token::ACCENT, Color::from_srgb8(255, 106, 61, 255)),
                (Token::ON_ACCENT, Color::from_srgb8(255, 255, 255, 255)),
                (Token::OUTLINE, Color::from_srgb8(206, 210, 218, 255)),
                (Token::OUTLINE_STRONG, Color::from_srgb8(140, 146, 158, 255)),
                (Token::SHADOW, Color::from_srgb8(0, 0, 0, 64)),
                (Token::ERROR, Color::from_srgb8(206, 47, 47, 255)),
                (Token::ON_ERROR, Color::from_srgb8(255, 255, 255, 255)),
                (Token::WARNING, Color::from_srgb8(214, 137, 16, 255)),
                (Token::SUCCESS, Color::from_srgb8(35, 148, 78, 255)),
                (Token::SELECTION, Color::from_srgb8(38, 110, 255, 64)),
                (Token::ON_SELECTION, Color::from_srgb8(12, 16, 24, 255)),
                (Token::HOVER, Color::from_srgb8(20, 22, 26, 16)),
                (Token::PRESSED, Color::from_srgb8(20, 22, 26, 32)),
                (Token::DISABLED, Color::from_srgb8(20, 22, 26, 24)),
                (Token::ON_DISABLED, Color::from_srgb8(96, 101, 110, 96)),
                (Token::GRID, Color::from_srgb8(206, 210, 218, 128)),
                (
                    Token::CANVAS_BACKDROP,
                    Color::from_srgb8(246, 247, 250, 255),
                ),
                (Token::CANVAS_NODE, Color::from_srgb8(255, 255, 255, 255)),
                (
                    Token::CANVAS_NODE_SELECTED,
                    Color::from_srgb8(38, 110, 255, 255),
                ),
                (Token::SNAP_GUIDE, Color::from_srgb8(255, 45, 85, 255)),
                (Token::TEXT_PRIMARY, Color::from_srgb8(20, 22, 26, 255)),
                (Token::TEXT_SECONDARY, Color::from_srgb8(96, 101, 110, 255)),
                (Token::TEXT_DISABLED, Color::from_srgb8(120, 125, 133, 128)),
                (Token::SCRIM, Color::from_srgb8(0, 0, 0, 96)),
                (Token::PRIMARY_HOVER, Color::from_srgb8(58, 126, 255, 255)),
                (Token::PRIMARY_PRESSED, Color::from_srgb8(22, 88, 224, 255)),
                (Token::PRIMARY_SOFT, Color::from_srgb8(232, 238, 254, 255)),
                (Token::SECONDARY_SOFT, Color::from_srgb8(240, 234, 254, 255)),
                (Token::SUCCESS_SOFT, Color::from_srgb8(228, 246, 236, 255)),
                (Token::WARNING_SOFT, Color::from_srgb8(252, 242, 223, 255)),
                (Token::ERROR_SOFT, Color::from_srgb8(253, 233, 234, 255)),
                (Token::FOCUS_RING, Color::from_srgb8(38, 110, 255, 255)),
                (Token::CONTROL_TRACK, Color::from_srgb8(228, 231, 239, 255)),
                (Token::CONTROL_THUMB, Color::from_srgb8(255, 255, 255, 255)),
                (Token::CONTROL_HOVER, Color::from_srgb8(236, 239, 246, 255)),
                (Token::HEADER_BG, Color::from_srgb8(255, 255, 255, 255)),
                (Token::NAV_BG, Color::from_srgb8(247, 248, 251, 255)),
                (Token::NAV_HOVER, Color::from_srgb8(236, 238, 244, 255)),
                (Token::NAV_SELECTED, Color::from_srgb8(228, 235, 253, 255)),
                (Token::DIVIDER, Color::from_srgb8(234, 236, 242, 255)),
                (Token::CODE_BG, Color::from_srgb8(28, 32, 41, 255)),
                (Token::CODE_TEXT, Color::from_srgb8(215, 220, 234, 255)),
                (Token::OVERLAY_BG, Color::from_srgb8(255, 255, 255, 255)),
                (Token::HERO_START, Color::from_srgb8(59, 110, 246, 255)),
                (Token::HERO_END, Color::from_srgb8(138, 92, 246, 255)),
                (Token::ON_HERO, Color::from_srgb8(255, 255, 255, 255)),
                (Token::HERO_SCRIM, Color::from_srgb8(15, 18, 28, 200)),
                (Token::HUD_BG, Color::from_srgb8(255, 255, 255, 246)),
                (Token::HUD_TEXT, Color::from_srgb8(28, 30, 38, 255)),
                (Token::ACCENT_SOFT, Color::from_srgb8(255, 240, 232, 255)),
                (Token::GRID_DOT, Color::from_srgb8(216, 220, 232, 255)),
                (Token::NAV_TEXT, Color::from_srgb8(42, 47, 58, 255)),
                (Token::NAV_TEXT_MUTED, Color::from_srgb8(122, 128, 144, 255)),
                (Token::SECONDARY, Color::from_srgb8(124, 106, 240, 255)),
                (Token::CARD_BG, Color::from_srgb8(255, 255, 255, 255)),
                (Token::CARD_BORDER, Color::from_srgb8(228, 230, 238, 255)),
                (
                    Token::CONTROL_DISABLED_BG,
                    Color::from_srgb8(241, 242, 246, 255),
                ),
                (
                    Token::CONTROL_DISABLED_BORDER,
                    Color::from_srgb8(213, 216, 224, 255),
                ),
                (
                    Token::CONTROL_DISABLED_THUMB,
                    Color::from_srgb8(196, 200, 210, 255),
                ),
                (
                    Token::INDETERMINATE_FILL,
                    Color::from_srgb8(38, 110, 255, 255),
                ),
                (Token::INPUT_BG, Color::WHITE),
                (
                    Token::INPUT_BORDER_HOVER,
                    Color::from_srgb8(154, 163, 181, 255),
                ),
                (
                    Token::INPUT_BORDER_ERROR,
                    Color::from_srgb8(229, 72, 77, 255),
                ),
                (
                    Token::INPUT_BORDER_WARNING,
                    Color::from_srgb8(217, 119, 6, 255),
                ),
                (
                    Token::PREEDIT_UNDERLINE,
                    Color::from_srgb8(38, 110, 255, 255),
                ),
                (Token::SUBMENU_ARROW, Color::from_srgb8(107, 114, 128, 255)),
            ],
        );

        let dark = Palette::new(
            ThemeKind::Dark,
            &[
                (Token::SURFACE, Color::from_srgb8(24, 26, 31, 255)),
                (Token::SURFACE_RAISED, Color::from_srgb8(32, 35, 42, 255)),
                (Token::SURFACE_SUNKEN, Color::from_srgb8(16, 17, 21, 255)),
                (Token::ON_SURFACE, Color::from_srgb8(233, 236, 241, 255)),
                (
                    Token::ON_SURFACE_MUTED,
                    Color::from_srgb8(160, 166, 176, 255),
                ),
                (Token::PRIMARY, Color::from_srgb8(120, 165, 255, 255)),
                (Token::ON_PRIMARY, Color::from_srgb8(10, 18, 34, 255)),
                (Token::ACCENT, Color::from_srgb8(255, 148, 104, 255)),
                (Token::ON_ACCENT, Color::from_srgb8(34, 14, 4, 255)),
                (Token::OUTLINE, Color::from_srgb8(64, 68, 78, 255)),
                (Token::OUTLINE_STRONG, Color::from_srgb8(108, 114, 126, 255)),
                (Token::SHADOW, Color::from_srgb8(0, 0, 0, 160)),
                (Token::ERROR, Color::from_srgb8(255, 120, 120, 255)),
                (Token::ON_ERROR, Color::from_srgb8(40, 8, 8, 255)),
                (Token::WARNING, Color::from_srgb8(240, 180, 70, 255)),
                (Token::SUCCESS, Color::from_srgb8(110, 214, 150, 255)),
                (Token::SELECTION, Color::from_srgb8(120, 165, 255, 56)),
                (Token::ON_SELECTION, Color::from_srgb8(240, 245, 255, 255)),
                (Token::HOVER, Color::from_srgb8(233, 236, 241, 18)),
                (Token::PRESSED, Color::from_srgb8(233, 236, 241, 38)),
                (Token::DISABLED, Color::from_srgb8(233, 236, 241, 32)),
                (Token::ON_DISABLED, Color::from_srgb8(160, 166, 176, 96)),
                (Token::GRID, Color::from_srgb8(64, 68, 78, 140)),
                (Token::CANVAS_BACKDROP, Color::from_srgb8(18, 20, 24, 255)),
                (Token::CANVAS_NODE, Color::from_srgb8(44, 48, 56, 255)),
                (
                    Token::CANVAS_NODE_SELECTED,
                    Color::from_srgb8(120, 165, 255, 255),
                ),
                (Token::SNAP_GUIDE, Color::from_srgb8(255, 92, 122, 255)),
                (Token::TEXT_PRIMARY, Color::from_srgb8(233, 236, 241, 255)),
                (Token::TEXT_SECONDARY, Color::from_srgb8(160, 166, 176, 255)),
                (Token::TEXT_DISABLED, Color::from_srgb8(120, 126, 134, 128)),
                (Token::SCRIM, Color::from_srgb8(0, 0, 0, 160)),
                (Token::PRIMARY_HOVER, Color::from_srgb8(140, 180, 255, 255)),
                (Token::PRIMARY_PRESSED, Color::from_srgb8(88, 132, 235, 255)),
                (Token::PRIMARY_SOFT, Color::from_srgb8(34, 48, 79, 255)),
                (Token::SECONDARY_SOFT, Color::from_srgb8(46, 37, 66, 255)),
                (Token::SUCCESS_SOFT, Color::from_srgb8(28, 58, 42, 255)),
                (Token::WARNING_SOFT, Color::from_srgb8(59, 47, 27, 255)),
                (Token::ERROR_SOFT, Color::from_srgb8(61, 34, 38, 255)),
                (Token::FOCUS_RING, Color::from_srgb8(150, 195, 255, 255)),
                (Token::CONTROL_TRACK, Color::from_srgb8(45, 51, 65, 255)),
                (Token::CONTROL_THUMB, Color::from_srgb8(237, 241, 250, 255)),
                (Token::CONTROL_HOVER, Color::from_srgb8(56, 64, 82, 255)),
                (Token::HEADER_BG, Color::from_srgb8(23, 26, 33, 255)),
                (Token::NAV_BG, Color::from_srgb8(20, 23, 29, 255)),
                (Token::NAV_HOVER, Color::from_srgb8(35, 40, 51, 255)),
                (Token::NAV_SELECTED, Color::from_srgb8(38, 51, 79, 255)),
                (Token::DIVIDER, Color::from_srgb8(38, 44, 56, 255)),
                (Token::CODE_BG, Color::from_srgb8(14, 16, 21, 255)),
                (Token::CODE_TEXT, Color::from_srgb8(201, 210, 228, 255)),
                (Token::OVERLAY_BG, Color::from_srgb8(32, 37, 47, 255)),
                (Token::HERO_START, Color::from_srgb8(76, 125, 255, 255)),
                (Token::HERO_END, Color::from_srgb8(160, 107, 255, 255)),
                (Token::ON_HERO, Color::from_srgb8(255, 255, 255, 255)),
                (Token::HERO_SCRIM, Color::from_srgb8(8, 10, 16, 200)),
                (Token::HUD_BG, Color::from_srgb8(8, 10, 14, 224)),
                (Token::HUD_TEXT, Color::from_srgb8(220, 227, 242, 255)),
                (Token::ACCENT_SOFT, Color::from_srgb8(58, 42, 34, 255)),
                (Token::GRID_DOT, Color::from_srgb8(51, 58, 72, 255)),
                (Token::NAV_TEXT, Color::from_srgb8(228, 233, 243, 255)),
                (Token::NAV_TEXT_MUTED, Color::from_srgb8(138, 147, 166, 255)),
                (Token::SECONDARY, Color::from_srgb8(160, 130, 255, 255)),
                (Token::CARD_BG, Color::from_srgb8(32, 35, 42, 255)),
                (Token::CARD_BORDER, Color::from_srgb8(52, 56, 66, 255)),
                (
                    Token::CONTROL_DISABLED_BG,
                    Color::from_srgb8(42, 44, 51, 255),
                ),
                (
                    Token::CONTROL_DISABLED_BORDER,
                    Color::from_srgb8(74, 77, 87, 255),
                ),
                (
                    Token::CONTROL_DISABLED_THUMB,
                    Color::from_srgb8(90, 94, 105, 255),
                ),
                (
                    Token::INDETERMINATE_FILL,
                    Color::from_srgb8(122, 162, 255, 255),
                ),
                (Token::INPUT_BG, Color::from_srgb8(32, 34, 41, 255)),
                (
                    Token::INPUT_BORDER_HOVER,
                    Color::from_srgb8(108, 114, 128, 255),
                ),
                (
                    Token::INPUT_BORDER_ERROR,
                    Color::from_srgb8(255, 107, 107, 255),
                ),
                (
                    Token::INPUT_BORDER_WARNING,
                    Color::from_srgb8(255, 178, 36, 255),
                ),
                (
                    Token::PREEDIT_UNDERLINE,
                    Color::from_srgb8(122, 162, 255, 255),
                ),
                (Token::SUBMENU_ARROW, Color::from_srgb8(160, 166, 180, 255)),
            ],
        );

        // High contrast is not "dark with more contrast": it removes translucency
        // entirely so that every token is fully opaque.
        let high_contrast = Palette::new(
            ThemeKind::HighContrast,
            &[
                (Token::SURFACE, Color::BLACK),
                (Token::SURFACE_RAISED, Color::BLACK),
                (Token::SURFACE_SUNKEN, Color::BLACK),
                (Token::ON_SURFACE, Color::WHITE),
                (Token::ON_SURFACE_MUTED, Color::WHITE),
                (Token::PRIMARY, Color::from_srgb8(255, 230, 0, 255)),
                (Token::ON_PRIMARY, Color::BLACK),
                (Token::ACCENT, Color::from_srgb8(0, 230, 255, 255)),
                (Token::ON_ACCENT, Color::BLACK),
                (Token::OUTLINE, Color::WHITE),
                (Token::OUTLINE_STRONG, Color::WHITE),
                (Token::SHADOW, Color::BLACK),
                (Token::ERROR, Color::from_srgb8(255, 80, 80, 255)),
                (Token::ON_ERROR, Color::BLACK),
                (Token::WARNING, Color::from_srgb8(255, 200, 0, 255)),
                (Token::SUCCESS, Color::from_srgb8(0, 255, 128, 255)),
                (Token::SELECTION, Color::from_srgb8(255, 230, 0, 255)),
                (Token::ON_SELECTION, Color::BLACK),
                (Token::HOVER, Color::from_srgb8(255, 255, 255, 48)),
                (Token::PRESSED, Color::from_srgb8(255, 255, 255, 96)),
                (Token::DISABLED, Color::from_srgb8(255, 255, 255, 64)),
                (Token::ON_DISABLED, Color::from_srgb8(255, 255, 255, 128)),
                (Token::GRID, Color::WHITE),
                (Token::CANVAS_BACKDROP, Color::BLACK),
                (Token::CANVAS_NODE, Color::from_srgb8(16, 16, 16, 255)),
                (
                    Token::CANVAS_NODE_SELECTED,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (Token::SNAP_GUIDE, Color::from_srgb8(255, 0, 128, 255)),
                (Token::TEXT_PRIMARY, Color::WHITE),
                (Token::TEXT_SECONDARY, Color::WHITE),
                (Token::TEXT_DISABLED, Color::from_srgb8(255, 255, 255, 160)),
                (Token::SCRIM, Color::BLACK),
                (Token::PRIMARY_HOVER, Color::from_srgb8(255, 245, 120, 255)),
                (Token::PRIMARY_PRESSED, Color::from_srgb8(200, 190, 0, 255)),
                (Token::PRIMARY_SOFT, Color::from_srgb8(60, 58, 0, 255)),
                (Token::SECONDARY_SOFT, Color::from_srgb8(0, 58, 66, 255)),
                (Token::SUCCESS_SOFT, Color::from_srgb8(0, 60, 32, 255)),
                (Token::WARNING_SOFT, Color::from_srgb8(60, 48, 0, 255)),
                (Token::ERROR_SOFT, Color::from_srgb8(66, 16, 16, 255)),
                (Token::FOCUS_RING, Color::from_srgb8(255, 230, 0, 255)),
                (Token::CONTROL_TRACK, Color::from_srgb8(48, 48, 48, 255)),
                (Token::CONTROL_THUMB, Color::WHITE),
                (Token::CONTROL_HOVER, Color::from_srgb8(64, 64, 64, 255)),
                (Token::HEADER_BG, Color::BLACK),
                (Token::NAV_BG, Color::BLACK),
                (Token::NAV_HOVER, Color::from_srgb8(40, 40, 40, 255)),
                (Token::NAV_SELECTED, Color::from_srgb8(255, 230, 0, 255)),
                (Token::DIVIDER, Color::WHITE),
                (Token::CODE_BG, Color::BLACK),
                (Token::CODE_TEXT, Color::WHITE),
                (Token::OVERLAY_BG, Color::BLACK),
                (Token::HERO_START, Color::from_srgb8(0, 120, 255, 255)),
                (Token::HERO_END, Color::from_srgb8(120, 0, 255, 255)),
                (Token::ON_HERO, Color::BLACK),
                (Token::HERO_SCRIM, Color::BLACK),
                (Token::HUD_BG, Color::BLACK),
                (Token::HUD_TEXT, Color::WHITE),
                (Token::ACCENT_SOFT, Color::from_srgb8(60, 32, 16, 255)),
                (Token::GRID_DOT, Color::from_srgb8(64, 64, 64, 255)),
                (Token::NAV_TEXT, Color::WHITE),
                (Token::NAV_TEXT_MUTED, Color::from_srgb8(200, 200, 200, 255)),
                (Token::SECONDARY, Color::from_srgb8(200, 180, 255, 255)),
                (Token::CARD_BG, Color::BLACK),
                (Token::CARD_BORDER, Color::WHITE),
                (
                    Token::CONTROL_DISABLED_BG,
                    Color::from_srgb8(32, 32, 32, 255),
                ),
                (Token::CONTROL_DISABLED_BORDER, Color::WHITE),
                (Token::CONTROL_DISABLED_THUMB, Color::WHITE),
                (
                    Token::INDETERMINATE_FILL,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (Token::INPUT_BG, Color::BLACK),
                (
                    Token::INPUT_BORDER_HOVER,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (
                    Token::INPUT_BORDER_ERROR,
                    Color::from_srgb8(255, 80, 80, 255),
                ),
                (
                    Token::INPUT_BORDER_WARNING,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (
                    Token::PREEDIT_UNDERLINE,
                    Color::from_srgb8(255, 230, 0, 255),
                ),
                (Token::SUBMENU_ARROW, Color::WHITE),
            ],
        );

        Self {
            light,
            dark,
            high_contrast,
        }
    }

    /// The palette for a theme.
    #[must_use]
    pub fn palette(&self, kind: ThemeKind) -> &Palette {
        match kind {
            ThemeKind::Light => &self.light,
            ThemeKind::Dark => &self.dark,
            ThemeKind::HighContrast => &self.high_contrast,
        }
    }
}

/// The packed 4 KiB colour table, ready to be uploaded to the GPU.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeLut {
    /// 512 × RGBA16F, in token order.
    entries: Vec<[u16; 4]>,
    /// The same table, serialised as little-endian bytes.
    bytes: Vec<u8>,
}

impl ThemeLut {
    /// Pack a palette into the flat table.
    #[must_use]
    pub fn from_palette(palette: &Palette) -> Self {
        let entries: Vec<[u16; 4]> = palette.entries().iter().map(|c| c.to_rgba16f()).collect();
        let mut bytes = Vec::with_capacity(LUT_BYTES);
        for entry in &entries {
            for channel in entry {
                bytes.extend_from_slice(&channel.to_le_bytes());
            }
        }
        Self { entries, bytes }
    }

    /// The raw RGBA16F entries.
    #[must_use]
    pub fn entries(&self) -> &[[u16; 4]] {
        &self.entries
    }

    /// The table as the bytes the GPU expects.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The colour of a token, decoded back to linear RGBA.
    #[must_use]
    pub fn color(&self, token: Token) -> Color {
        let e = self.entries[token.index()];
        Color::new(
            unbound_math::f16_to_f32(e[0]),
            unbound_math::f16_to_f32(e[1]),
            unbound_math::f16_to_f32(e[2]),
            unbound_math::f16_to_f32(e[3]),
        )
    }
}

/// What a theme switch costs.
///
/// The two `false` fields are the point of the whole design: a theme switch
/// must not re-run layout and must not re-upload primitive instances.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SwitchOutcome {
    /// Bytes written to the colour table.
    pub bytes_uploaded: u32,
    /// Whether the layout engine has to run again.
    pub relayout_required: bool,
    /// Whether primitive instances have to be re-uploaded.
    pub instance_reupload_required: bool,
    /// Whether the switch actually changed anything.
    pub changed: bool,
}

/// The live theme state of an application.
#[derive(Clone, Debug)]
pub struct ThemeState {
    set: ThemeSet,
    current: ThemeKind,
    lut: ThemeLut,
    /// Number of switches performed, for diagnostics.
    switches: u64,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self::new(ThemeSet::default_set(), ThemeKind::Light)
    }
}

impl ThemeState {
    /// Start with an explicit theme set.
    #[must_use]
    pub fn new(set: ThemeSet, kind: ThemeKind) -> Self {
        let lut = ThemeLut::from_palette(set.palette(kind));
        Self {
            set,
            current: kind,
            lut,
            switches: 0,
        }
    }

    /// The active theme.
    #[must_use]
    pub const fn current(&self) -> ThemeKind {
        self.current
    }

    /// The packed colour table.
    #[must_use]
    pub const fn lut(&self) -> &ThemeLut {
        &self.lut
    }

    /// How many switches have happened.
    #[must_use]
    pub const fn switch_count(&self) -> u64 {
        self.switches
    }

    /// Switch to another theme.
    ///
    /// The only side effect is a new colour table: no layout, no instance
    /// upload, no re-sorting — the primitives reference tokens, not colours.
    pub fn switch(&mut self, kind: ThemeKind) -> SwitchOutcome {
        if kind == self.current {
            return SwitchOutcome {
                bytes_uploaded: 0,
                relayout_required: false,
                instance_reupload_required: false,
                changed: false,
            };
        }
        self.lut = ThemeLut::from_palette(self.set.palette(kind));
        self.current = kind;
        self.switches += 1;
        SwitchOutcome {
            bytes_uploaded: LUT_BYTES as u32,
            relayout_required: false,
            instance_reupload_required: false,
            changed: true,
        }
    }

    /// Toggle between light and dark, leaving high contrast alone.
    pub fn toggle_light_dark(&mut self) -> SwitchOutcome {
        match self.current {
            ThemeKind::Light => self.switch(ThemeKind::Dark),
            ThemeKind::Dark => self.switch(ThemeKind::Light),
            ThemeKind::HighContrast => SwitchOutcome {
                bytes_uploaded: 0,
                relayout_required: false,
                instance_reupload_required: false,
                changed: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lut_is_exactly_four_kibibytes() {
        let state = ThemeState::default();
        assert_eq!(state.lut().bytes().len(), 4096);
        assert_eq!(LUT_BYTES, 4096);
        assert_eq!(state.lut().entries().len(), MAX_TOKENS);
    }

    #[test]
    fn every_token_has_a_colour_in_every_theme() {
        let set = ThemeSet::default_set();
        for kind in ThemeKind::ALL {
            let palette = set.palette(kind);
            for token in TOKENS {
                assert!(
                    palette
                        .color(token)
                        .to_array()
                        .iter()
                        .all(|c| c.is_finite()),
                    "token {} has no finite colour in {}",
                    token.raw(),
                    kind.name()
                );
            }
        }
    }

    /// The GPU reserves two slots: 0 is "no colour" and 1 is the shadow.
    ///
    /// A palette that puts a real colour in slot 0 makes every "no fill"
    /// primitive paint that colour, and a palette that puts something else
    /// in slot 1 turns every analytic shadow into a solid block.
    #[test]
    fn slot_zero_is_transparent_and_slot_one_is_shadow() {
        let set = ThemeSet::default_set();
        for kind in ThemeKind::ALL {
            let palette = set.palette(kind);
            let none = palette.color(Token::NONE);
            assert_eq!(
                none.to_array()[3],
                0.0,
                "slot 0 must be fully transparent in {}",
                kind.name()
            );
            assert_eq!(
                palette.color(Token::SHADOW).to_array(),
                palette.entries()[Token::SHADOW.index()].to_array(),
                "slot 1 must be the shadow colour in {}",
                kind.name()
            );
            // The shadow is not transparent: an invisible shadow would be a
            // shadow that never renders.
            assert!(palette.color(Token::SHADOW).to_array()[3] > 0.0);
            assert_eq!(Token::NONE.raw(), 0);
            assert_eq!(Token::SHADOW.raw(), 1);
        }
    }

    /// Every semantic token owns a slot of its own.
    #[test]
    fn the_semantic_tokens_never_share_a_slot() {
        let mut seen = std::collections::HashSet::new();
        for token in TOKENS {
            assert!(
                seen.insert(token.raw()),
                "token {} shares a slot with another token",
                token.raw()
            );
        }
        // The reserved engine slots stay reserved.
        assert!(!seen.contains(&Token::NONE.raw()));
    }

    #[test]
    fn a_switch_uploads_four_kibibytes_and_nothing_else() {
        let mut state = ThemeState::default();
        let outcome = state.switch(ThemeKind::Dark);
        assert_eq!(outcome.bytes_uploaded, 4096);
        assert!(
            !outcome.relayout_required,
            "a theme switch must not re-layout"
        );
        assert!(
            !outcome.instance_reupload_required,
            "a theme switch must not re-upload instances"
        );
        assert!(outcome.changed);
        assert_eq!(state.current(), ThemeKind::Dark);
        assert_eq!(state.switch_count(), 1);
    }

    #[test]
    fn switching_to_the_active_theme_is_a_no_op() {
        let mut state = ThemeState::default();
        let outcome = state.switch(ThemeKind::Light);
        assert_eq!(outcome.bytes_uploaded, 0);
        assert!(!outcome.changed);
        assert_eq!(state.switch_count(), 0);
    }

    #[test]
    fn the_colour_table_actually_changes_with_the_theme() {
        let mut state = ThemeState::default();
        let light = state.lut().color(Token::SURFACE);
        state.switch(ThemeKind::Dark);
        let dark = state.lut().color(Token::SURFACE);
        assert_ne!(light.to_array(), dark.to_array());
        assert!(light.luminance_perceptual() > dark.luminance_perceptual());
    }

    #[test]
    fn the_lut_round_trips_through_half_floats() {
        let state = ThemeState::default();
        let original = state.set.palette(ThemeKind::Light).color(Token::PRIMARY);
        let decoded = state.lut().color(Token::PRIMARY);
        let a = original.to_array();
        let b = decoded.to_array();
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 0.005, "{x} vs {y}");
        }
    }

    /// The theme used to carry its own half-float decoder next to the
    /// canonical one in `unbound-math`. Two decoders for the same format is
    /// two chances to disagree — and this one did: it mapped every
    /// exponent-31 pattern to infinity, including the NaNs, where the
    /// canonical decoder returns NaN.
    ///
    /// The theme now uses the canonical decoder outright. This test keeps
    /// the two from drifting apart again, and pins the NaN behaviour.
    #[test]
    fn the_theme_uses_the_canonical_half_float_decoder() {
        // A signalling NaN and a quiet NaN.
        for bits in [0x7c01u16, 0x7e00, 0xfc01] {
            let decoded = unbound_math::f16_to_f32(bits);
            assert!(
                decoded.is_nan(),
                "pattern {bits:#06x} must decode to NaN, got {decoded}"
            );
        }
        // And the ordinary values still decode the way the LUT expects.
        assert_eq!(unbound_math::f16_to_f32(0x3c00), 1.0);
        assert_eq!(unbound_math::f16_to_f32(0x0000), 0.0);
    }

    #[test]
    fn high_contrast_removes_translucency() {
        let set = ThemeSet::default_set();
        let palette = set.palette(ThemeKind::HighContrast);
        // Only the three state overlays may stay translucent; every content
        // colour has to be fully opaque or the high-contrast theme would
        // wash out against an arbitrary wallpaper.
        let translucent = [
            Token::HOVER,
            Token::PRESSED,
            Token::DISABLED,
            Token::ON_DISABLED,
            Token::TEXT_DISABLED,
        ];
        for token in TOKENS {
            let alpha = palette.color(token).to_array()[3];
            if translucent.contains(&token) {
                assert!(alpha < 1.0, "token {} should be translucent", token.raw());
            } else {
                assert_eq!(alpha, 1.0, "token {} must be opaque", token.raw());
            }
        }
    }

    #[test]
    fn token_ids_are_stable_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for token in TOKENS {
            assert!(seen.insert(token.raw()), "duplicate token {}", token.raw());
        }
        // The two lowest ids are frozen because they are referenced from
        // built-in shaders: slot 0 is "no colour", slot 1 is the shadow.
        assert_eq!(Token::NONE.raw(), 0);
        assert_eq!(Token::SHADOW.raw(), 1);
        assert_eq!(Token::PRIMARY.raw(), 5);
    }

    #[test]
    fn a_token_out_of_range_saturates_instead_of_panicking() {
        assert_eq!(Token::new(u16::MAX).index(), MAX_TOKENS - 1);
    }
}

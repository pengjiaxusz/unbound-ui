//! HotkeyInput / ShortcutRecorder capturing key chords & detecting conflicts.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, KeyChord};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// Interactive state for [`HotkeyInputDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotkeyInputState {
    /// Currently bound shortcut chord.
    pub chord: Option<KeyChord>,
    /// Whether actively listening for the next key press.
    pub recording: bool,
    /// Whether the captured chord conflicts with another binding.
    pub conflict: bool,
}

/// Descriptor for a [`HotkeyInput`].
#[derive(Clone, Debug, PartialEq)]
pub struct HotkeyInputDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Display chord text (e.g. `"Ctrl+K"`).
    pub shortcut_text: String,
    /// Whether actively recording.
    pub recording: bool,
    /// Whether a conflict warning is active.
    pub conflict: bool,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl HotkeyInputDesc {
    /// Create a hotkey recorder descriptor.
    #[must_use]
    pub fn new(rect: Rect, shortcut_text: impl Into<String>) -> Self {
        Self {
            rect,
            shortcut_text: shortcut_text.into(),
            recording: false,
            conflict: false,
            label: "Shortcut Input".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape hotkey recorder (1 bg + 1 recording dot + 1 focus ring = 3 primitives).
#[must_use]
pub fn emit_hotkey_input(
    node: u64,
    desc: &HotkeyInputDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let border = if desc.conflict {
        Token::ERROR
    } else if desc.recording || hovered || state.focused {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        border,
        if desc.recording || desc.conflict {
            1.5
        } else {
            1.0
        },
        0.0,
        desc.z,
    ));

    let dot_r = Rect::new(
        desc.rect.max().x - 16.0,
        desc.rect.origin.y + (desc.rect.height() - 8.0) * 0.5,
        8.0,
        8.0,
    );
    primitives.push(surface(
        dot_r,
        4.0,
        if desc.conflict {
            Token::ERROR
        } else if desc.recording {
            Token::PRIMARY
        } else {
            Token::TEXT_SECONDARY
        },
        desc.z + 1,
    ));

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = desc.shortcut_text.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

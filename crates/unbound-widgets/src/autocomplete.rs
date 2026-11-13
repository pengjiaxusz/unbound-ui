//! AutoComplete / MentionInput with prefix filtering & suggestion popup indicator.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// Interactive state for [`AutoCompleteDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoCompleteState {
    /// Current query text.
    pub query: String,
    /// Highlighted suggestion index.
    pub highlighted: usize,
    /// Whether the suggestion list is open.
    pub open: bool,
}

impl AutoCompleteState {
    /// Filter `candidates` case-insensitively by `query`.
    #[must_use]
    pub fn filter<'a>(&self, candidates: &'a [String]) -> Vec<&'a str> {
        let q = self.query.to_ascii_lowercase();
        candidates
            .iter()
            .filter(|c| c.to_ascii_lowercase().contains(&q))
            .map(String::as_str)
            .collect()
    }
}

/// Descriptor for an [`AutoComplete`].
#[derive(Clone, Debug, PartialEq)]
pub struct AutoCompleteDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Current input value.
    pub value: String,
    /// Number of matching suggestions.
    pub suggestion_count: usize,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl AutoCompleteDesc {
    /// Create an autocomplete descriptor.
    #[must_use]
    pub fn new(rect: Rect, value: impl Into<String>, suggestion_count: usize) -> Self {
        Self {
            rect,
            value: value.into(),
            suggestion_count,
            label: "AutoComplete".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape autocomplete field (1 input + 1 suggestion badge + 1 focus ring = 3 primitives).
#[must_use]
pub fn emit_autocomplete(
    node: u64,
    desc: &AutoCompleteDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE
        },
        if state.disabled {
            Token::DISABLED
        } else if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let badge_r = Rect::new(
        desc.rect.max().x - 18.0,
        desc.rect.origin.y + (desc.rect.height() - 10.0) * 0.5,
        10.0,
        10.0,
    );
    let mut badge = surface(badge_r, 5.0, Token::PRIMARY, desc.z + 1);
    if desc.suggestion_count == 0 {
        badge.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(badge.depth_u16(), 0.0);
    }
    primitives.push(badge);

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
            .with_cursor(CursorKind::Text)
    });
    let mut a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = desc.value.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

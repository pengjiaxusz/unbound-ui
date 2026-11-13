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

/// Deterministic sub-node id for an autocomplete suggestion item.
#[must_use]
pub const fn autocomplete_option_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xB00 + index as u64)
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
    /// Optional suggestion labels rendered when `open` is true.
    pub suggestions: Vec<String>,
    /// Whether the suggestion popup is open.
    pub open: bool,
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
            suggestions: Vec::new(),
            open: false,
            label: "AutoComplete".to_owned(),
            z: 600,
        }
    }

    /// Attach suggestion labels and open state.
    #[must_use]
    pub fn with_suggestions(mut self, suggestions: Vec<String>, open: bool) -> Self {
        self.suggestion_count = suggestions.len();
        self.suggestions = suggestions;
        self.open = open;
        self
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

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if desc.open && !state.disabled {
        let count = desc.suggestions.len().min(6);
        let row_h = 26.0;
        if count > 0 {
            let popup_r = Rect::new(
                desc.rect.origin.x,
                desc.rect.max().y + 4.0,
                desc.rect.width(),
                count as f32 * row_h + 8.0,
            );
            out.primitives.push(outlined(
                popup_r,
                6.0,
                Token::SURFACE_RAISED,
                Token::CARD_BORDER,
                1.0,
                6.0,
                desc.z + 4,
            ));
            for (idx, label) in desc.suggestions.iter().take(count).enumerate() {
                let row_r = Rect::new(
                    popup_r.origin.x + 4.0,
                    popup_r.origin.y + 4.0 + idx as f32 * row_h,
                    (popup_r.width() - 8.0).max(16.0),
                    row_h - 2.0,
                );
                out.primitives.push(surface(
                    row_r,
                    4.0,
                    Token::SURFACE,
                    desc.z + 5,
                ));
                let key = autocomplete_option_node(node, idx);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 5, row_r)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::ListItem, label.clone(), row_r).clickable(),
                    true,
                );
            }
        }
    }

    out
}

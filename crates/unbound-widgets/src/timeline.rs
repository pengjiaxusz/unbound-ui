//! Timeline event stream with status nodes, connectors, and timestamps.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::alert::Severity;
use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_TIMELINE_NODES: usize = 4;

/// A single event on a [`TimelineDesc`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineItem {
    /// Event title.
    pub title: String,
    /// Timestamp string.
    pub timestamp: String,
    /// Status severity.
    pub severity: Severity,
}

/// Descriptor for a [`Timeline`].
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Events in chronological order.
    pub items: Vec<TimelineItem>,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl TimelineDesc {
    /// Create a timeline descriptor.
    #[must_use]
    pub fn new(rect: Rect, items: Vec<TimelineItem>) -> Self {
        Self {
            rect,
            items,
            label: "Timeline".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape vertical timeline (1 bg + 1 rail + 4 dots + 4 row cards + 1 focus ring = 11 primitives).
#[must_use]
pub fn emit_timeline(
    node: u64,
    desc: &TimelineDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(11 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let rail_r = Rect::new(
        desc.rect.origin.x + 14.0,
        desc.rect.origin.y + 10.0,
        2.0,
        (desc.rect.height() - 20.0).max(4.0),
    );
    primitives.push(surface(rail_r, 1.0, Token::CARD_BORDER, desc.z + 1));

    let row_h = ((desc.rect.height() - 16.0) / MAX_TIMELINE_NODES as f32).max(14.0);
    for i in 0..MAX_TIMELINE_NODES {
        let item = desc.items.get(i);
        let y = desc.rect.origin.y + 8.0 + i as f32 * row_h;
        let dot_tok = match item.map(|it| it.severity).unwrap_or(Severity::Info) {
            Severity::Info => Token::PRIMARY,
            Severity::Success => Token::SUCCESS,
            Severity::Warning => Token::WARNING,
            Severity::Error => Token::ERROR,
        };
        let mut dot = surface(
            Rect::new(desc.rect.origin.x + 10.0, y + 4.0, 10.0, 10.0),
            5.0,
            dot_tok,
            desc.z + 2,
        );
        if item.is_none() {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);

        let card_r = Rect::new(
            desc.rect.origin.x + 28.0,
            y,
            (desc.rect.width() - 36.0).max(16.0),
            (row_h - 4.0).max(10.0),
        );
        let mut card = outlined(
            card_r,
            4.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        );
        if item.is_none() {
            card.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(card.depth_u16(), 0.0);
        }
        primitives.push(card);
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::List, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

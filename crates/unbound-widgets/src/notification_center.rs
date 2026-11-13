//! NotificationCenter drawer aggregating toast history, severity filters & read state.

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

const MAX_ITEMS: usize = 4;

/// A single archived notification item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationItem {
    /// Stable ID.
    pub id: u64,
    /// Title text.
    pub title: String,
    /// Severity level.
    pub severity: Severity,
    /// Whether already read.
    pub read: bool,
}

/// Interactive state for [`NotificationCenterDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationCenterState {
    /// Archived notifications.
    pub items: Vec<NotificationItem>,
    /// Optional severity filter.
    pub filter: Option<Severity>,
}

impl NotificationCenterState {
    /// Mark all notifications as read.
    pub fn mark_all_read(&mut self) {
        for item in &mut self.items {
            item.read = true;
        }
    }

    /// Clear all notifications.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Count unread items.
    #[must_use]
    pub fn unread_count(&self) -> usize {
        self.items.iter().filter(|i| !i.read).count()
    }
}

/// Descriptor for a [`NotificationCenter`].
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationCenterDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Visible items.
    pub items: Vec<NotificationItem>,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl NotificationCenterDesc {
    /// Create a notification center descriptor.
    #[must_use]
    pub fn new(rect: Rect, items: Vec<NotificationItem>) -> Self {
        Self {
            rect,
            items,
            label: "Notification Center".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape notification center (1 bg + 4 item cards + 4 unread dots + 1 focus ring = 10 primitives).
#[must_use]
pub fn emit_notification_center(
    node: u64,
    desc: &NotificationCenterDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(10 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        Token::SURFACE,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pad = 6.0;
    let row_h = ((desc.rect.height() - pad * 5.0) / MAX_ITEMS as f32).max(12.0);
    for i in 0..MAX_ITEMS {
        let item = desc.items.get(i);
        let r = Rect::new(
            desc.rect.origin.x + pad,
            desc.rect.origin.y + pad + i as f32 * (row_h + pad),
            (desc.rect.width() - pad * 2.0).max(16.0),
            row_h,
        );
        let mut card = outlined(
            r,
            5.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE_SUNKEN
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

        let dot_r = Rect::new(r.max().x - 14.0, r.origin.y + (row_h - 6.0) * 0.5, 6.0, 6.0);
        let unread = item.is_some_and(|it| !it.read);
        let dot_tok = match item.map(|it| it.severity).unwrap_or(Severity::Info) {
            Severity::Error => Token::ERROR,
            Severity::Warning => Token::WARNING,
            Severity::Success => Token::SUCCESS,
            Severity::Info => Token::PRIMARY,
        };
        let mut dot = surface(dot_r, 3.0, dot_tok, desc.z + 2);
        if !unread {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
    }

    primitives.push(focus_ring(
        desc.rect,
        8.0,
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
    let a11y = A11yNode::new(Role::Region, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

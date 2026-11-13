//! Dual-column Transfer box with filtering and batch move between source & target.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// One item in a [`TransferDesc`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferItem {
    /// Stable item key.
    pub key: u64,
    /// Display label.
    pub label: String,
    /// Whether this item currently resides in the target (right) list.
    pub in_target: bool,
    /// Whether this item is checked for batch transfer.
    pub checked: bool,
}

/// Interactive state for [`TransferDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferState {
    /// Items across both lists.
    pub items: Vec<TransferItem>,
}

impl TransferState {
    /// Move all checked items into `to_target` (`true` = right, `false` = left) and uncheck them.
    pub fn move_checked(&mut self, to_target: bool) -> usize {
        let mut moved = 0;
        for item in &mut self.items {
            if item.checked && item.in_target != to_target {
                item.in_target = to_target;
                item.checked = false;
                moved += 1;
            }
        }
        moved
    }
}

/// Descriptor for a dual-list [`Transfer`].
#[derive(Clone, Debug, PartialEq)]
pub struct TransferDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Left list count.
    pub source_count: usize,
    /// Right list count.
    pub target_count: usize,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl TransferDesc {
    /// Create a transfer descriptor.
    #[must_use]
    pub fn new(rect: Rect, source_count: usize, target_count: usize) -> Self {
        Self {
            rect,
            source_count,
            target_count,
            label: "Transfer".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape dual transfer list (1 bg + left pane + right pane + 2 transfer buttons + 1 focus ring = 6 primitives).
#[must_use]
pub fn emit_transfer(
    node: u64,
    desc: &TransferDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(6 + label_glyphs.len());
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
    let mid_w = 28.0;
    let pane_w = ((desc.rect.width() - pad * 4.0 - mid_w) * 0.5).max(24.0);
    let pane_h = (desc.rect.height() - pad * 2.0).max(24.0);

    let left_r = Rect::new(
        desc.rect.origin.x + pad,
        desc.rect.origin.y + pad,
        pane_w,
        pane_h,
    );
    let right_r = Rect::new(
        left_r.max().x + pad * 2.0 + mid_w,
        desc.rect.origin.y + pad,
        pane_w,
        pane_h,
    );
    for pane in [left_r, right_r] {
        primitives.push(outlined(
            pane,
            6.0,
            Token::SURFACE_SUNKEN,
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        ));
    }

    let mid_x = left_r.max().x + pad;
    let cy = desc.rect.origin.y + desc.rect.height() * 0.5;
    for dy in [-16.0_f32, 4.0] {
        let btn_r = Rect::new(mid_x + 2.0, cy + dy, mid_w - 4.0, 20.0);
        primitives.push(outlined(
            btn_r,
            4.0,
            if hovered {
                Token::PRIMARY_SOFT
            } else {
                Token::SURFACE_RAISED
            },
            Token::PRIMARY,
            1.0,
            0.0,
            desc.z + 2,
        ));
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
    let a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

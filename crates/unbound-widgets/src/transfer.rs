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

/// Deterministic child node key for the "Move to target (`>`)" button.
#[must_use]
pub const fn move_right_node(base: u64) -> u64 {
    base ^ 0x5452_4652_0001
}

/// Deterministic child node key for the "Move to source (`<`)" button.
#[must_use]
pub const fn move_left_node(base: u64) -> u64 {
    base ^ 0x5452_4652_0002
}

/// Deterministic child node key for an individual [`TransferItem`].
#[must_use]
pub const fn transfer_item_node(base: u64, item_key: u64) -> u64 {
    base ^ (item_key.wrapping_mul(0x9E37_79B9_7F4A_7C15))
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
    /// Items across both panes.
    pub items: Vec<TransferItem>,
    /// Left search filter query.
    pub left_query: String,
    /// Right search filter query.
    pub right_query: String,
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
            items: Vec::new(),
            left_query: String::new(),
            right_query: String::new(),
            label: "Transfer".to_owned(),
            z: 600,
        }
    }

    /// Attach items to the transfer descriptor.
    #[must_use]
    pub fn with_items(mut self, items: Vec<TransferItem>) -> Self {
        self.source_count = items.iter().filter(|i| !i.in_target).count();
        self.target_count = items.iter().filter(|i| i.in_target).count();
        self.items = items;
        self
    }

    /// Attach left/right search queries.
    #[must_use]
    pub fn with_queries(mut self, left: impl Into<String>, right: impl Into<String>) -> Self {
        self.left_query = left.into();
        self.right_query = right.into();
        self
    }

    /// Rectangle of the `>` (move right) and `<` (move left) buttons.
    #[must_use]
    pub fn action_button_rects(&self) -> (Rect, Rect) {
        let pad = 6.0;
        let mid_w = 28.0;
        let pane_w = ((self.rect.width() - pad * 4.0 - mid_w) * 0.5).max(24.0);
        let left_max_x = self.rect.origin.x + pad + pane_w;
        let mid_x = left_max_x + pad;
        let cy = self.rect.origin.y + self.rect.height() * 0.5;
        let right_btn = Rect::new(mid_x + 2.0, cy - 16.0, mid_w - 4.0, 20.0);
        let left_btn = Rect::new(mid_x + 2.0, cy + 4.0, mid_w - 4.0, 20.0);
        (right_btn, left_btn)
    }
}

/// Emit a stable-shape dual transfer list with independent `>`/`<` and item sub-hit regions.
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

    let (right_btn, left_btn) = desc.action_button_rects();
    for btn_r in [right_btn, left_btn] {
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

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let r_key = move_right_node(node);
        out.with_sub_control(
            r_key,
            HitRegion::new(r_key, CONTROL_LAYER, u32::from(desc.z) + 2, right_btn)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Move to target", right_btn).clickable(),
            true,
        );
        let l_key = move_left_node(node);
        out.with_sub_control(
            l_key,
            HitRegion::new(l_key, CONTROL_LAYER, u32::from(desc.z) + 2, left_btn)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Move to source", left_btn).clickable(),
            true,
        );

        let row_h = 24.0;
        let mut left_idx = 0usize;
        let mut right_idx = 0usize;
        for item in &desc.items {
            let (pane, idx, query) = if item.in_target {
                (&right_r, &mut right_idx, &desc.right_query)
            } else {
                (&left_r, &mut left_idx, &desc.left_query)
            };
            if !query.is_empty() && !item.label.to_lowercase().contains(&query.to_lowercase()) {
                continue;
            }
            let y = pane.origin.y + 4.0 + (*idx as f32) * row_h;
            *idx += 1;
            if y + row_h > pane.max().y {
                continue;
            }
            let item_r = Rect::new(pane.origin.x + 4.0, y, (pane.width() - 8.0).max(8.0), row_h - 2.0);
            let item_key = transfer_item_node(node, item.key);
            let mut item_a11y = A11yNode::new(Role::CheckBox, item.label.clone(), item_r).clickable();
            item_a11y.selected = item.checked;
            out.with_sub_control(
                item_key,
                HitRegion::new(item_key, CONTROL_LAYER, u32::from(desc.z) + 2, item_r)
                    .with_cursor(CursorKind::Pointer),
                item_a11y,
                false,
            );
        }
    }

    out
}

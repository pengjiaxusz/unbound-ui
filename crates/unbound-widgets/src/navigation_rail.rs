//! Collapsible side NavigationRail / SidebarNav with rail mode, groups & badges.

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

const MAX_RAIL_ITEMS: usize = 6;

/// One navigation item in a [`NavigationRailDesc`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavRailItem {
    /// Stable item identity.
    pub id: u64,
    /// Display label.
    pub label: String,
    /// Optional badge count (`> 0` lights up the badge pill).
    pub badge_count: u32,
}

impl NavRailItem {
    /// Create a navigation item.
    #[must_use]
    pub fn new(id: u64, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            badge_count: 0,
        }
    }

    /// Attach a badge count.
    #[must_use]
    pub fn with_badge(mut self, count: u32) -> Self {
        self.badge_count = count;
        self
    }
}

/// Interactive state for [`NavigationRailDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavigationRailState {
    /// Whether the rail is collapsed into compact icon-rail mode.
    pub collapsed: bool,
    /// Active item identity.
    pub active_id: u64,
}

/// Descriptor for a collapsible [`NavigationRail`].
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationRailDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Navigation items.
    pub items: Vec<NavRailItem>,
    /// Active item identity.
    pub active_id: u64,
    /// Whether collapsed to icon-only width.
    pub collapsed: bool,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl NavigationRailDesc {
    /// Create a navigation rail descriptor.
    #[must_use]
    pub fn new(rect: Rect, items: Vec<NavRailItem>, active_id: u64) -> Self {
        Self {
            rect,
            items,
            active_id,
            collapsed: false,
            label: "Navigation Rail".to_owned(),
            z: 600,
        }
    }
}

/// Deterministic sub-node id for a navigation rail item.
#[must_use]
pub const fn rail_item_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x500 + index as u64)
}

/// Emit a stable-shape navigation rail (1 bg + 6 item rows + 6 badge pills + 1 focus ring = 14 primitives).
#[must_use]
pub fn emit_navigation_rail(
    node: u64,
    desc: &NavigationRailDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(14 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
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

    let row_h = ((desc.rect.height() - 16.0) / MAX_RAIL_ITEMS as f32).clamp(18.0, 36.0);
    let pad_x = 6.0;
    let rail_w = if desc.collapsed {
        desc.rect.width().min(44.0)
    } else {
        desc.rect.width()
    };

    for idx in 0..MAX_RAIL_ITEMS {
        let item = desc.items.get(idx);
        let y = desc.rect.origin.y + 8.0 + idx as f32 * row_h;
        let row_r = Rect::new(
            desc.rect.origin.x + pad_x,
            y,
            (rail_w - pad_x * 2.0).max(12.0),
            (row_h - 4.0).max(12.0),
        );
        let is_active = item.is_some_and(|it| it.id == desc.active_id);
        let mut row_inst = outlined(
            row_r,
            5.0,
            if is_active {
                Token::PRIMARY_SOFT
            } else if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            if is_active {
                Token::PRIMARY
            } else {
                Token::NONE
            },
            if is_active { 1.0 } else { 0.0 },
            0.0,
            desc.z + 1,
        );
        if item.is_none() {
            row_inst.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(row_inst.depth_u16(), 0.0);
        }
        primitives.push(row_inst);

        let badge_r = Rect::new(
            row_r.max().x - 16.0,
            row_r.origin.y + (row_r.height() - 10.0) * 0.5,
            10.0,
            10.0,
        );
        let has_badge = item.is_some_and(|it| it.badge_count > 0);
        let mut badge_inst = surface(badge_r, 5.0, Token::PRIMARY, desc.z + 2);
        if !has_badge {
            badge_inst.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(badge_inst.depth_u16(), 0.0);
        }
        primitives.push(badge_inst);
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
    let a11y =
        A11yNode::new(Role::Navigation, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (idx, item) in desc.items.iter().take(MAX_RAIL_ITEMS).enumerate() {
            let y = desc.rect.origin.y + 8.0 + idx as f32 * row_h;
            let row_r = Rect::new(
                desc.rect.origin.x + pad_x,
                y,
                (rail_w - pad_x * 2.0).max(12.0),
                (row_h - 4.0).max(12.0),
            );
            let key = rail_item_node(node, idx);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, row_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, item.label.clone(), row_r).clickable(),
                true,
            );
        }
    }

    out
}

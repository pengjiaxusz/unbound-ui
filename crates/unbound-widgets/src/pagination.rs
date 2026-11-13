//! Pagination control: page buttons, ellipsis folding, page-size selector & jump.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_PAGE_SLOTS: usize = 7;

/// Descriptor for a [`Pagination`] bar.
#[derive(Clone, Debug, PartialEq)]
pub struct PaginationDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Total number of pages (>= 1).
    pub total_pages: usize,
    /// Current 1-based active page.
    pub current_page: usize,
    /// Items per page (displayed as metadata).
    pub page_size: usize,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl PaginationDesc {
    /// Create a pagination descriptor.
    #[must_use]
    pub fn new(rect: Rect, total_pages: usize, current_page: usize) -> Self {
        let total = total_pages.max(1);
        Self {
            rect,
            total_pages: total,
            current_page: current_page.clamp(1, total),
            page_size: 20,
            label: "Pagination".to_owned(),
            z: 600,
        }
    }

    /// Set page size.
    #[must_use]
    pub fn with_page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    /// Compute the 7 visible page slots (`Some(page)` or `None` for ellipsis).
    #[must_use]
    pub fn visible_slots(&self) -> [Option<usize>; MAX_PAGE_SLOTS] {
        let total = self.total_pages.max(1);
        let cur = self.current_page.clamp(1, total);
        let mut out = [None; MAX_PAGE_SLOTS];
        if total <= MAX_PAGE_SLOTS {
            for (i, slot) in out.iter_mut().take(total).enumerate() {
                *slot = Some(i + 1);
            }
            return out;
        }
        out[0] = Some(1);
        out[MAX_PAGE_SLOTS - 1] = Some(total);
        if cur <= 4 {
            out[1] = Some(2);
            out[2] = Some(3);
            out[3] = Some(4);
            out[4] = Some(5);
            out[5] = None;
        } else if cur >= total - 3 {
            out[1] = None;
            out[2] = Some(total - 4);
            out[3] = Some(total - 3);
            out[4] = Some(total - 2);
            out[5] = Some(total - 1);
        } else {
            out[1] = None;
            out[2] = Some(cur - 1);
            out[3] = Some(cur);
            out[4] = Some(cur + 1);
            out[5] = None;
        }
        out
    }
}

/// Interactive state for pagination navigation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaginationState {
    /// Current 1-based page.
    pub page: usize,
}

impl PaginationState {
    /// Create state at `page`.
    #[must_use]
    pub fn new(page: usize) -> Self {
        Self { page: page.max(1) }
    }

    /// Go to `page`, clamped to `1..=total_pages`.
    pub fn go_to(&mut self, page: usize, total_pages: usize) -> bool {
        let next = page.clamp(1, total_pages.max(1));
        if next != self.page {
            self.page = next;
            true
        } else {
            false
        }
    }

    /// Advance one page.
    pub fn next(&mut self, total_pages: usize) -> bool {
        self.go_to(self.page.saturating_add(1), total_pages)
    }

    /// Go back one page.
    pub fn prev(&mut self, total_pages: usize) -> bool {
        self.go_to(self.page.saturating_sub(1), total_pages)
    }
}

/// Emit a stable-shape pagination bar (1 container + 9 slot buttons + 1 focus ring = 11 primitives).
#[must_use]
pub fn emit_pagination(
    node: u64,
    desc: &PaginationDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(11 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let border = if state.disabled {
        Token::DISABLED
    } else if hovered || state.focused {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE_SUNKEN,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let total_slots = MAX_PAGE_SLOTS + 2;
    let gap = 4.0;
    let pad = 4.0;
    let slot_w = ((desc.rect.width() - pad * 2.0 - gap * (total_slots - 1) as f32)
        / total_slots as f32)
        .max(18.0);
    let slot_h = (desc.rect.height() - pad * 2.0).max(18.0);
    let slots = desc.visible_slots();

    for idx in 0..total_slots {
        let x = desc.rect.origin.x + pad + idx as f32 * (slot_w + gap);
        let r = Rect::new(x, desc.rect.origin.y + pad, slot_w, slot_h);
        let (active, visible) = if idx == 0 || idx == total_slots - 1 {
            (false, true)
        } else {
            let page_opt = slots[idx - 1];
            let is_cur = page_opt == Some(desc.current_page);
            let is_vis = idx - 1 < desc.total_pages || desc.total_pages > MAX_PAGE_SLOTS;
            (is_cur, is_vis)
        };
        let fill = if active {
            Token::PRIMARY
        } else if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE
        };
        let mut inst = outlined(
            r,
            4.0,
            fill,
            if active {
                Token::PRIMARY
            } else {
                Token::CARD_BORDER
            },
            if visible { 1.0 } else { 0.0 },
            0.0,
            desc.z + 1,
        );
        if !visible {
            inst.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(inst.depth_u16(), 0.0);
        }
        primitives.push(inst);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_slots_fold_middle_with_ellipsis() {
        let desc = PaginationDesc::new(Rect::new(0.0, 0.0, 320.0, 36.0), 20, 10);
        let slots = desc.visible_slots();
        assert_eq!(slots[0], Some(1));
        assert_eq!(slots[1], None);
        assert_eq!(slots[3], Some(10));
        assert_eq!(slots[5], None);
        assert_eq!(slots[6], Some(20));
    }

    #[test]
    fn pagination_state_navigates_within_bounds() {
        let mut st = PaginationState::new(1);
        assert!(!st.prev(5));
        assert!(st.next(5));
        assert_eq!(st.page, 2);
        assert!(st.go_to(99, 5));
        assert_eq!(st.page, 5);
    }
}

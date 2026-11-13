//! Single-section Collapsible / Disclosure panel with spring progress.

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

/// Interactive state for a [`CollapsibleDesc`].
#[derive(Clone, Debug, PartialEq)]
pub struct CollapsibleState {
    /// Target expanded state.
    pub open: bool,
    /// Normalized `[0.0, 1.0]` spring reveal progress.
    pub progress: f32,
}

impl Default for CollapsibleState {
    fn default() -> Self {
        Self {
            open: false,
            progress: 0.0,
        }
    }
}

impl CollapsibleState {
    /// Toggle open/closed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Advance animation toward `open`.
    pub fn tick(&mut self, dt: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        let diff = target - self.progress;
        if diff.abs() <= 0.005 {
            self.progress = target;
            false
        } else {
            self.progress += diff * (dt * 12.0).clamp(0.0, 1.0);
            true
        }
    }
}

/// Descriptor for a single-section [`Collapsible`].
#[derive(Clone, Debug, PartialEq)]
pub struct CollapsibleDesc {
    /// Outer bounding rectangle.
    pub rect: Rect,
    /// Header trigger title.
    pub title: String,
    /// Whether expanded (`progress` in `0.0..=1.0`).
    pub open: bool,
    /// Normalized reveal progress `[0.0, 1.0]`.
    pub progress: f32,
    /// Base z-index.
    pub z: u16,
}

impl CollapsibleDesc {
    /// Create a collapsible descriptor.
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>, open: bool) -> Self {
        Self {
            rect,
            title: title.into(),
            open,
            progress: if open { 1.0 } else { 0.0 },
            z: 600,
        }
    }
}

/// Emit a stable-shape collapsible disclosure (1 outer + 1 header + 1 chevron + 1 body + 1 focus ring = 5 primitives).
#[must_use]
pub fn emit_collapsible(
    node: u64,
    desc: &CollapsibleDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let border = if hovered || state.focused {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let header_h = 32.0_f32.min(desc.rect.height());
    let header_r = Rect::new(
        desc.rect.origin.x,
        desc.rect.origin.y,
        desc.rect.width(),
        header_h,
    );
    primitives.push(surface(
        header_r,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        desc.z + 1,
    ));

    let chev_r = Rect::new(
        header_r.max().x - 20.0,
        header_r.origin.y + (header_h - 8.0) * 0.5,
        8.0,
        8.0,
    );
    primitives.push(surface(
        chev_r,
        2.0,
        if desc.open {
            Token::PRIMARY
        } else {
            Token::TEXT_SECONDARY
        },
        desc.z + 2,
    ));

    let body_h = ((desc.rect.height() - header_h) * desc.progress.clamp(0.0, 1.0)).max(0.0);
    let body_r = Rect::new(
        desc.rect.origin.x + 2.0,
        desc.rect.origin.y + header_h,
        (desc.rect.width() - 4.0).max(1.0),
        body_h.max(1.0),
    );
    let mut body = surface(body_r, 4.0, Token::SURFACE_SUNKEN, desc.z + 1);
    if desc.progress <= 0.01 {
        body.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(body.depth_u16(), 0.0);
    }
    primitives.push(body);

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
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z + 1), header_r)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::DisclosureTriangle, desc.title.clone(), header_r)
        .disabled(state.disabled);
    a11y.expanded = Some(desc.open);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

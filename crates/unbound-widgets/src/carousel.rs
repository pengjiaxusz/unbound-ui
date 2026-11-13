//! Carousel / Slider deck with spring snap pagination & indicator dots.

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

const MAX_DOTS: usize = 5;

/// Deterministic sub-node id for a carousel indicator dot.
#[must_use]
pub const fn carousel_dot_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x400 + index as u64)
}

/// Interactive state for [`CarouselDesc`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CarouselState {
    /// Active slide index.
    pub active: usize,
    /// Normalized slide offset for spring snap.
    pub offset: f32,
}

impl CarouselState {
    /// Step forward or backward wrapping around `count`.
    pub fn step(&mut self, delta: isize, count: usize) {
        if count == 0 {
            return;
        }
        self.active = (self.active as isize + delta).rem_euclid(count as isize) as usize;
        self.offset = self.active as f32;
    }
}

/// Descriptor for a [`Carousel`].
#[derive(Clone, Debug, PartialEq)]
pub struct CarouselDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Total number of slides.
    pub slide_count: usize,
    /// Active slide index.
    pub active: usize,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl CarouselDesc {
    /// Create a carousel descriptor.
    #[must_use]
    pub fn new(rect: Rect, slide_count: usize, active: usize) -> Self {
        let count = slide_count.max(1);
        Self {
            rect,
            slide_count: count,
            active: active % count,
            label: "Carousel".to_owned(),
            z: 600,
        }
    }
}

/// Emit a stable-shape carousel (1 stage + 5 indicator dots + 1 focus ring = 7 primitives).
#[must_use]
pub fn emit_carousel(
    node: u64,
    desc: &CarouselDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(7 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let dots = desc.slide_count.min(MAX_DOTS);
    let dot_sz = 8.0;
    let gap = 6.0;
    let total_w = dots as f32 * dot_sz + (dots.saturating_sub(1)) as f32 * gap;
    let start_x = desc.rect.origin.x + (desc.rect.width() - total_w) * 0.5;
    let dot_y = desc.rect.max().y - dot_sz - 8.0;
    for i in 0..MAX_DOTS {
        let visible = i < dots;
        let is_active = i == desc.active.min(dots.saturating_sub(1));
        let mut dot = surface(
            Rect::new(start_x + i as f32 * (dot_sz + gap), dot_y, dot_sz, dot_sz),
            dot_sz * 0.5,
            if is_active {
                Token::PRIMARY
            } else {
                Token::CARD_BORDER
            },
            desc.z + 1,
        );
        if !visible {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
    }

    primitives.push(focus_ring(
        desc.rect,
        8.0,
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

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for i in 0..dots {
            let dot_r = Rect::new(start_x + i as f32 * (dot_sz + gap), dot_y, dot_sz, dot_sz);
            let key = carousel_dot_node(node, i);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, dot_r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, format!("Slide {}", i + 1), dot_r).clickable(),
                true,
            );
        }
    }

    out
}

//! Watermark non-interactive tiled overlay layer.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{glyphs, surface};
use crate::{ControlState, WidgetOutput};

const WATERMARK_TILES: usize = 6;

/// Descriptor for a [`Watermark`] overlay layer.
#[derive(Clone, Debug, PartialEq)]
pub struct WatermarkDesc {
    /// Area covered by the watermark.
    pub rect: Rect,
    /// Watermark text.
    pub text: String,
    /// Tile opacity (`0.0..=1.0`).
    pub opacity: f32,
    /// Base z-index.
    pub z: u16,
}

impl WatermarkDesc {
    /// Create a watermark descriptor.
    #[must_use]
    pub fn new(rect: Rect, text: impl Into<String>) -> Self {
        Self {
            rect,
            text: text.into(),
            opacity: 0.16,
            z: 950,
        }
    }
}

/// Emit a stable-shape watermark grid (6 low-opacity tiled stamps).
#[must_use]
pub fn emit_watermark(
    desc: &WatermarkDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(WATERMARK_TILES + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let cols = 3;
    let rows = 2;
    let cw = (desc.rect.width() / cols as f32).max(24.0);
    let ch = (desc.rect.height() / rows as f32).max(18.0);
    let opacity = if state.disabled {
        0.05
    } else {
        desc.opacity.clamp(0.02, 0.5)
    };
    for r in 0..rows {
        for c in 0..cols {
            let stamp_r = Rect::new(
                desc.rect.origin.x + c as f32 * cw + cw * 0.2,
                desc.rect.origin.y + r as f32 * ch + ch * 0.3,
                (cw * 0.6).max(16.0),
                12.0,
            );
            let mut stamp = surface(stamp_r, 3.0, Token::TEXT_SECONDARY, desc.z);
            stamp.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(stamp.depth_u16(), opacity);
            primitives.push(stamp);
        }
    }

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_SECONDARY,
            desc.z + 1,
            &mut primitives,
            &mut payloads,
        );
    }

    let a11y = A11yNode::new(Role::Label, desc.text.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

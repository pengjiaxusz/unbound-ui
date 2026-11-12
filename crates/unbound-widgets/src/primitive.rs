//! Shared primitive construction for the control emitters.
//!
//! Every control draws the same four things — a surface, a border, a run of
//! glyphs and an optional focus ring — so the construction lives here once.
//! The emitters only decide *what* to draw and *where*; this module decides
//! *how* a primitive instance is packed.

use unbound_gpu::data::{
    BlendMode, GpuPayload, GpuPrimitiveInstance, PrimType, SubElement, CLIP_NONE,
};
use unbound_math::{f16, Rect};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

/// Build a primitive instance.
#[allow(clippy::too_many_arguments)]
pub(crate) fn instance(
    rect: Rect,
    radii: [f32; 4],
    fill: Token,
    border: Token,
    border_width: f32,
    shadow: f32,
    kind: PrimType,
    sub: SubElement,
    z: u16,
) -> GpuPrimitiveInstance {
    GpuPrimitiveInstance {
        chunk_id: [0, 0],
        local_pos: [rect.origin.x, rect.origin.y],
        size: [rect.size.x, rect.size.y],
        corner_radii: f16::pack_radii(radii),
        border_width: f16::f32_to_f16(border_width),
        shadow_blur: f16::f32_to_f16(shadow),
        transform_idx: 0,
        clip_idx: CLIP_NONE,
        fill_token_or_uv: u32::from(fill.raw()),
        border_token_or_tex: u32::from(border.raw()),
        prim_type_and_flags: GpuPrimitiveInstance::pack_type(kind, BlendMode::Normal, sub as u16),
        z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
        extra_payload_idx: 0,
    }
}

/// A filled rounded rectangle.
pub(crate) fn surface(rect: Rect, radius: f32, fill: Token, z: u16) -> GpuPrimitiveInstance {
    instance(
        rect,
        [radius; 4],
        fill,
        Token::NONE,
        0.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Background,
        z,
    )
}

/// A rounded rectangle with a hairline border.
pub(crate) fn outlined(
    rect: Rect,
    radius: f32,
    fill: Token,
    border: Token,
    border_width: f32,
    shadow: f32,
    z: u16,
) -> GpuPrimitiveInstance {
    instance(
        rect,
        [radius; 4],
        fill,
        border,
        border_width,
        shadow,
        PrimType::RoundedRect,
        SubElement::Background,
        z,
    )
}

/// An outward stroke, used for the keyboard focus ring.
pub(crate) fn focus_ring(
    rect: Rect,
    radius: f32,
    color: Token,
    width: f32,
    z: u16,
) -> GpuPrimitiveInstance {
    instance(
        rect,
        [radius; 4],
        Token::NONE,
        color,
        width,
        0.0,
        PrimType::Stroke,
        SubElement::FocusRing,
        z,
    )
}

/// A run of glyphs, already positioned by the caller.
///
/// The caller shapes the text with its own text engine — which knows the
/// baseline, the raster scale and the atlas — so the emitter never has to
/// know anything about fonts.
pub(crate) fn glyphs(
    quads: &[GlyphQuad],
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    for quad in quads {
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
        let mut glyph = instance(
            quad.rect,
            [0.0; 4],
            color,
            Token::NONE,
            0.0,
            0.0,
            if quad.color {
                PrimType::GlyphColor
            } else {
                PrimType::Glyph
            },
            SubElement::Content,
            z,
        );
        // The atlas layer travels in the border slot, which the background
        // does not use.
        glyph.border_token_or_tex = quad.layer;
        glyph.extra_payload_idx = payload_idx;
        primitives.push(glyph);
    }
}

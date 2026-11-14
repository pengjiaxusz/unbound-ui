//! 控件输出器共用的图元构建辅助函数。
//!
//! 每个控件都绘制相同的四类元素——表面、边框、字形串与可选的焦点环——
//! 因此将构建逻辑集中于此。各输出器只需决定*画什么*和*画在哪里*；
//! 本模块决定*如何*打包图元实例。
//!

use unbound_gpu::data::{
    BlendMode, GpuPayload, GpuPrimitiveInstance, PrimFlags, PrimType, SubElement, CLIP_NONE,
};
use unbound_math::{f16, Rect};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

/// 构建图元实例。
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

/// 填充圆角矩形。
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

/// 带细边框的圆角矩形。
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

/// 向外描边，用于键盘焦点环。
///
/// 无论控件是否聚焦都会输出焦点环，`focused` 仅决定其不透明度。
/// 这样获得或失去焦点只是一次*绘制*变更（图元池中的单个图元），
/// 而非会使其后所有索引失效并触发全量重传的*结构*变更。
pub(crate) fn focus_ring(
    rect: Rect,
    radius: f32,
    color: Token,
    width: f32,
    z: u16,
    focused: bool,
) -> GpuPrimitiveInstance {
    let mut ring = instance(
        rect,
        [radius; 4],
        Token::NONE,
        color,
        width,
        0.0,
        PrimType::Stroke,
        SubElement::FocusRing,
        z,
    );
    ring.z_and_opacity =
        GpuPrimitiveInstance::pack_z_opacity(ring.depth_u16(), if focused { 1.0 } else { 0.0 });
    ring
}

/// 已由调用方定位好的字形串。
///
/// 调用方使用自身的文本引擎（了解基线、光栅化缩放与图集）进行塑形，
/// 因此输出器完全无需关心字体细节。
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
        // 图集层级通过边框槽位传递（背景图元不使用该槽位）。
        //
        glyph.border_token_or_tex = quad.layer;
        glyph.extra_payload_idx = payload_idx;
        // 距离场字形必须按距离场解码；该标志位于类型字的高半部分。
        //
        if quad.msdf {
            glyph.prim_type_and_flags |= u32::from(PrimFlags::MSDF.bits()) << 16;
        }
        primitives.push(glyph);
    }
}

/// 以退化二次贝塞尔曲线编码的直线描边（用于斜线、箭头与关闭叉号，避免 `PrimType::Line` 被绘制为实心矩形）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn curve_stroke(
    start: [f32; 2],
    end: [f32; 2],
    thickness: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let origin_x = start[0].min(end[0]);
    let origin_y = start[1].min(end[1]);
    let w = (start[0] - end[0]).abs().max(1.0);
    let h = (start[1] - end[1]).abs().max(1.0);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(
        [start[0] - origin_x, start[1] - origin_y],
        [
            (start[0] + end[0]) * 0.5 - origin_x,
            (start[1] + end[1]) * 0.5 - origin_y,
        ],
        [end[0] - origin_x, end[1] - origin_y],
        thickness,
    ));
    let mut prim = instance(
        Rect::new(origin_x, origin_y, w, h),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    prim.extra_payload_idx = payload_idx;
    primitives.push(prim);
}

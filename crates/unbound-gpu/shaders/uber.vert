#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_nonuniform_qualifier : require

// ---------------------------------------------------------------------------
// Uber vertex shader (docs §3.1 / §3.2 / §4.4)
//
// Order of operations matters and is load-bearing:
//   1. chunk-anchored camera-relative coordinates  (zero per-frame uploads)
//   2. independent animation transform             (never enters Taffy)
//   3. pixel-grid alignment of the primitive's own box
//   4. quad expansion by 3σ for analytic shadows
//   5. clip the screen AABB against the folded clip rect
//   6. reconstruct local coordinates through M⁻¹  (no shear on rotated quads)
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 2, std430) readonly buffer SortedIndices {
    uint indices[];
} u_sorted;

layout(location = 0) noperspective out vec2 v_local;
layout(location = 1) noperspective out vec2 v_rel;
layout(location = 2) flat out vec2 v_size;
layout(location = 3) flat out vec4 v_radii;
layout(location = 4) flat out vec2 v_border_pad;  // border width, shadow pad
layout(location = 5) flat out uvec4 v_tokens;   // fill, border/tex, type|blend|flags, clip
layout(location = 6) flat out float v_opacity;
layout(location = 7) flat out float v_shadow;
layout(location = 8) flat out uvec2 v_misc;     // payload index, depth
layout(location = 9) flat out vec2 v_chunk_base; // chunk_delta*4096 - cam_local

void main() {
    PrimBuf prims = PrimBuf(pc.primitives_bda);
    TransformBuf transforms = TransformBuf(pc.transforms_bda);
    ClipBuf clips = ClipBuf(pc.clips_bda);

    uint prim_index = u_sorted.indices[gl_InstanceIndex];
    Prim p = prims.p[prim_index];

    // ---- 1. chunk-anchored camera-relative origin ------------------------
    vec2 chunk_base = vec2(p.chunk_id - pc.camera_chunk) * CHUNK_SIZE - pc.camera_local;
    vec2 origin_rel = chunk_base + p.local_pos;

    // ---- 2. animation transform -----------------------------------------
    Transform t = transforms.t[p.transform_idx & TRANSFORM_INDEX_MASK];
    bool animated = (p.transform_idx & TRANSFORM_ANIMATED_BIT) != 0u;
    mat2 m = mat2(t.linear.x, t.linear.y, t.linear.z, t.linear.w);
    vec2 trans = t.translation;

    // ---- 3. shadow padding ----------------------------------------------
    float shadow_blur = unpack_shadow(p.border_and_shadow);
    float pad = shadow_blur > 0.0 ? shadow_blur * 3.0 : 0.0;

    vec2 uv = vec2(float(gl_VertexIndex & 1u), float((gl_VertexIndex >> 1u) & 1u));

    float scale = pc.camera_zoom * pc.dpi_scale;
    vec2 half_vp = pc.viewport_size * 0.5;

    // ---- 4. pixel-grid alignment -----------------------------------------
    // The primitive's *own* box is snapped to the physical pixel grid: its
    // top-left corner moves onto a pixel and its size is taken from the
    // snapped corners.
    //
    // Snapping the box (rather than the shadow-padded hull) is what makes a
    // 1 px border land exactly on a pixel boundary, so the analytic
    // antialiasing resolves it to one fully covered pixel instead of a half
    // transparent smear across two.
    //
    // The size stays exact for glyphs: a glyph quad is exactly as wide as its
    // atlas allocation in texels, and an integer texel count survives the
    // snap (`floor(a + n) - floor(a) == n` for integer `n`). That keeps the
    // texture mapping at one texel per physical pixel — no stretch, no blur,
    // and every letter of a word weighted the same.
    //
    // A rotated quad is left alone: its screen-space box is only a
    // conservative hull, so snapping it would clip the shape.
    bool axis_aligned = abs(m[0][1]) < 1e-5 && abs(m[1][0]) < 1e-5
                      && abs(m[0][0] - 1.0) < 1e-5 && abs(m[1][1] - 1.0) < 1e-5;

    vec2 eff_origin = origin_rel + trans;
    vec2 eff_size = p.size;
    if (!animated && axis_aligned) {
        vec2 box_min_s = floor(rel_to_screen(eff_origin, scale, half_vp) + 0.5);
        vec2 box_max_s = floor(rel_to_screen(eff_origin + p.size, scale, half_vp) + 0.5);
        eff_origin = screen_to_rel(box_min_s, scale, half_vp);
        eff_size = max((box_max_s - box_min_s) / scale, vec2(1.0 / scale));
    }

    vec2 lo = vec2(-pad);
    vec2 hi = eff_size + vec2(pad);

    // Local -> camera-relative (animation matrix, then primitive origin).
    vec2 c0 = eff_origin + m * vec2(lo.x, lo.y);
    vec2 c1 = eff_origin + m * vec2(hi.x, lo.y);
    vec2 c2 = eff_origin + m * vec2(lo.x, hi.y);
    vec2 c3 = eff_origin + m * vec2(hi.x, hi.y);

    vec2 s0 = rel_to_screen(c0, scale, half_vp);
    vec2 s1 = rel_to_screen(c1, scale, half_vp);
    vec2 s2 = rel_to_screen(c2, scale, half_vp);
    vec2 s3 = rel_to_screen(c3, scale, half_vp);

    vec2 bb_min = min(min(s0, s1), min(s2, s3));
    vec2 bb_max = max(max(s0, s1), max(s2, s3));

    // ---- 5. clip ---------------------------------------------------------
    if (p.clip_idx != CLIP_NONE) {
        ClipNode c = clips.c[p.clip_idx];
        vec2 clip_min = rel_to_screen(chunk_base + c.aabb.xy, scale, half_vp);
        vec2 clip_max = rel_to_screen(chunk_base + c.aabb.zw, scale, half_vp);
        bb_min = max(bb_min, min(clip_min, clip_max));
        bb_max = min(bb_max, max(clip_min, clip_max));
    }

    // Empty intersection: collapse the primitive. The rasteriser then discards
    // it for free — no fragment ever runs (docs §3.2).
    if (bb_min.x >= bb_max.x || bb_min.y >= bb_max.y) {
        gl_Position = vec4(0.0);
        v_local = vec2(0.0);
        v_rel = vec2(0.0);
        v_size = vec2(0.0);
        v_radii = vec4(0.0);
        v_border_pad = vec2(0.0);
        v_tokens = uvec4(0u);
        v_opacity = 0.0;
        v_shadow = 0.0;
        v_misc = uvec2(0u);
        v_chunk_base = chunk_base;
        return;
    }

    vec2 p_screen = mix(bb_min, bb_max, uv);

    // ---- 6. inverse local reconstruction --------------------------------
    // screen -> camera-relative -> local. Both steps are affine, so the
    // `noperspective` varyings below stay exact for rotated primitives, and the
    // SDF in the fragment shader is evaluated in true local space.
    //
    // The origin used here is the *snapped* one, so the local coordinate of a
    // fragment is measured from the box's real (pixel-aligned) corner.
    vec2 p_rel = screen_to_rel(p_screen, scale, half_vp);
    mat2 inv_m = inverse2x2(m);
    vec2 p_local = inv_m * (p_rel - eff_origin);

    v_local = p_local;
    v_rel = p_rel;
    v_size = eff_size;
    v_radii = unpack_radii(p.radii_packed);
    v_border_pad = vec2(unpack_border(p.border_and_shadow), pad);    v_tokens = uvec4(p.fill, p.border_or_tex, p.type_flags, p.clip_idx);
    v_opacity = unpack_opacity(p.z_opacity);
    v_shadow = shadow_blur;
    v_misc = uvec2(p.extra_payload_idx, p.z_opacity >> 16);
    v_chunk_base = chunk_base;

    gl_Position = vec4(p_screen / half_vp - vec2(1.0), 0.0, 1.0);
    // Reverse-Z: 1.0 is nearest. The culling pass already normalised the value.
    gl_Position.z = float(p.z_opacity >> 16) / 65535.0;
}

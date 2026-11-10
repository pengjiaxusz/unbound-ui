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
//   3. quad expansion by 3σ for analytic shadows
//   4. clip the screen AABB against the folded clip rect
//   5. reconstruct local coordinates through M⁻¹  (no shear on rotated quads)
//   6. pixel snapping for static geometry, bypassed while animating
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 2, std430) readonly buffer SortedIndices {
    uint indices[];
} u_sorted;

layout(location = 0) noperspective out vec2 v_local;
layout(location = 1) noperspective out vec2 v_rel;
layout(location = 2) flat out vec2 v_size;
layout(location = 3) flat out vec4 v_radii;
layout(location = 4) flat out vec2 v_border_pad;
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

    vec2 lo = vec2(-pad);
    vec2 hi = p.size + vec2(pad);
    vec2 uv = vec2(float(gl_VertexIndex & 1u), float((gl_VertexIndex >> 1u) & 1u));

    float scale = pc.camera_zoom * pc.dpi_scale;
    vec2 half_vp = pc.viewport_size * 0.5;

    // Local -> camera-relative (animation matrix, then primitive origin).
    vec2 c0 = origin_rel + (m * vec2(lo.x, lo.y) + trans);
    vec2 c1 = origin_rel + (m * vec2(hi.x, lo.y) + trans);
    vec2 c2 = origin_rel + (m * vec2(lo.x, hi.y) + trans);
    vec2 c3 = origin_rel + (m * vec2(hi.x, hi.y) + trans);

    vec2 s0 = rel_to_screen(c0, scale, half_vp);
    vec2 s1 = rel_to_screen(c1, scale, half_vp);
    vec2 s2 = rel_to_screen(c2, scale, half_vp);
    vec2 s3 = rel_to_screen(c3, scale, half_vp);

    vec2 bb_min = min(min(s0, s1), min(s2, s3));
    vec2 bb_max = max(max(s0, s1), max(s2, s3));

    // ---- 6. pixel snapping ----------------------------------------------
    // Snap the AABB, never the individual vertices: snapping vertices would
    // change the aspect ratio of a rotated quad. Animated quads keep full
    // sub-pixel precision so spring tails never crawl.
    if (!animated) {
        bb_min = floor(bb_min + 0.5);
        bb_max = floor(bb_max + 0.5);
    }

    // ---- 4. clip ---------------------------------------------------------
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

    // ---- 5. inverse local reconstruction --------------------------------
    // screen -> camera-relative -> local. Both steps are affine, so the
    // `noperspective` varyings below stay exact for rotated primitives, and the
    // SDF in the fragment shader is evaluated in true local space.
    vec2 p_rel = screen_to_rel(p_screen, scale, half_vp);
    mat2 inv_m = inverse2x2(m);
    vec2 p_local = inv_m * (p_rel - origin_rel - trans);

    v_local = p_local;
    v_rel = p_rel;
    v_size = p.size;
    v_radii = unpack_radii(p.radii_packed);
    v_border_pad = vec2(unpack_border(p.border_and_shadow), pad);
    v_tokens = uvec4(p.fill, p.border_or_tex, p.type_flags, p.clip_idx);
    v_opacity = unpack_opacity(p.z_opacity);
    v_shadow = shadow_blur;
    v_misc = uvec2(p.extra_payload_idx, p.z_opacity >> 16);
    v_chunk_base = chunk_base;

    gl_Position = vec4(p_screen / half_vp - vec2(1.0), 0.0, 1.0);
    // Reverse-Z: 1.0 is nearest. The culling pass already normalised the value.
    gl_Position.z = float(p.z_opacity >> 16) / 65535.0;
}

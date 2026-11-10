#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require

// ---------------------------------------------------------------------------
// Opaque Core vertex shader (docs §3.1).
//
// Identical position maths to `uber.vert`, but it declares **no** varyings: the
// prepass writes depth only, so interpolating colour, radii and clip data would
// be pure bandwidth. Keeping it separate also removes the "VS output without a
// matching FS input" interface warning.
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 2, std430) readonly buffer SortedIndices {
    uint indices[];
} u_sorted;

void main() {
    PrimBuf prims = PrimBuf(pc.primitives_bda);
    TransformBuf transforms = TransformBuf(pc.transforms_bda);
    ClipBuf clips = ClipBuf(pc.clips_bda);

    Prim p = prims.p[u_sorted.indices[gl_InstanceIndex]];

    vec2 chunk_base = vec2(p.chunk_id - pc.camera_chunk) * CHUNK_SIZE - pc.camera_local;
    vec2 origin_rel = chunk_base + p.local_pos;

    Transform t = transforms.t[p.transform_idx & TRANSFORM_INDEX_MASK];
    mat2 m = mat2(t.linear.x, t.linear.y, t.linear.z, t.linear.w);

    float scale = pc.camera_zoom * pc.dpi_scale;
    vec2 half_vp = pc.viewport_size * 0.5;

    vec2 c0 = origin_rel + (m * vec2(0.0) + t.translation);
    vec2 c1 = origin_rel + (m * vec2(p.size.x, 0.0) + t.translation);
    vec2 c2 = origin_rel + (m * vec2(0.0, p.size.y) + t.translation);
    vec2 c3 = origin_rel + (m * p.size + t.translation);

    vec2 s0 = rel_to_screen(c0, scale, half_vp);
    vec2 s1 = rel_to_screen(c1, scale, half_vp);
    vec2 s2 = rel_to_screen(c2, scale, half_vp);
    vec2 s3 = rel_to_screen(c3, scale, half_vp);

    vec2 bb_min = min(min(s0, s1), min(s2, s3));
    vec2 bb_max = max(max(s0, s1), max(s2, s3));

    if (p.clip_idx != CLIP_NONE) {
        ClipNode c = clips.c[p.clip_idx];
        vec2 clip_min = rel_to_screen(chunk_base + c.aabb.xy, scale, half_vp);
        vec2 clip_max = rel_to_screen(chunk_base + c.aabb.zw, scale, half_vp);
        bb_min = max(bb_min, min(clip_min, clip_max));
        bb_max = min(bb_max, max(clip_min, clip_max));
    }

    if (bb_min.x >= bb_max.x || bb_min.y >= bb_max.y) {
        gl_Position = vec4(0.0);
        return;
    }

    vec2 uv = vec2(float(gl_VertexIndex & 1u), float((gl_VertexIndex >> 1u) & 1u));
    vec2 p_screen = mix(bb_min, bb_max, uv);
    gl_Position = vec4(p_screen / half_vp - vec2(1.0), 0.0, 1.0);
    gl_Position.z = float(p.z_opacity >> 16) / 65535.0;
}

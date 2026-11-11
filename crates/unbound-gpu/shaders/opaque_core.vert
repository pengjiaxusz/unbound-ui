#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require

// ---------------------------------------------------------------------------
// Opaque Core pre-pass vertex shader (docs §3.1).
//
// This pass exists only to write depth for large opaque containers so that
// the hardware Early-Z unit can reject the thousands of primitives behind
// them. It intentionally produces no colour output: the render target masks
// colour writes off and the shader has no outputs, so the whole pass is pure
// depth.
//
// Two rules make the pass *safe*, and both are load-bearing:
//
//   1. **Only opaque containers get a core.** A rounded rectangle that is
//      fully opaque, large enough to be worth the depth write and not
//      clipped qualifies; everything else (glyphs, curves, strokes, borders,
//      translucent panels) collapses to a degenerate quad and writes no depth
//      at all. A core for a translucent primitive would be worse than no core
//      at all: the depth it writes would reject the opaque pixels *behind*
//      it, and the painter's algorithm would stop blending.
//
//   2. **The core is the box shrunk by its largest corner radius.** The
//      rounded rim stays out of the depth buffer, so the uber pass can still
//      blend its antialiased edge over whatever is behind the container.
//
// The depth written is the primitive's own depth, so the uber pass — which
// draws the very same primitive at the very same depth — still passes the
// `GREATER_OR_EQUAL` test on the core, while everything behind it fails.
//
// The shader declares **no varyings**: the prepass writes depth only, so
// interpolating colour, radii and clip data would be pure bandwidth.
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 2, std430) readonly buffer SortedIndices {
    uint indices[];
} u_sorted;

// A container only becomes an opaque core when it is big enough for the
// Early-Z win to outweigh the depth write: 64 x 64 logical pixels.
#define CORE_MIN_AREA 4096.0

void main() {
    PrimBuf prims = PrimBuf(pc.primitives_bda);
    TransformBuf transforms = TransformBuf(pc.transforms_bda);

    Prim p = prims.p[u_sorted.indices[gl_InstanceIndex]];

    // ---- 1. eligibility --------------------------------------------------
    // The container has to be opaque twice over: the primitive's own opacity
    // *and* the alpha of the theme colour it fills with. A translucent panel
    // may not own a core, because the core would hide the content behind it
    // that the panel is supposed to blend with.
    float opacity = float(p.z_opacity & 0xFFFFu) / 65535.0;
    vec4 fill = theme_color(p.fill);
    bool eligible = (p.type_flags & 0xFFu) == PRIM_ROUNDED_RECT
        && opacity >= 0.999
        && fill.a >= 0.999
        && p.clip_idx == CLIP_NONE
        && p.size.x * p.size.y >= CORE_MIN_AREA;

    // ---- 2. the core rectangle -------------------------------------------
    // `r` is the largest corner radius; the core is the box pulled in by it
    // on every side.
    vec4 radii = unpack_radii(p.radii_packed);
    float r = max(max(radii.x, radii.y), max(radii.z, radii.w));
    vec2 core_size = p.size - vec2(2.0 * r);
    if (core_size.x <= 1.0 || core_size.y <= 1.0) {
        eligible = false;
    }

    if (!eligible) {
        // A degenerate quad: every vertex lands on the same point, so the
        // rasteriser never produces a fragment and no depth is written.
        gl_Position = vec4(0.0);
        return;
    }

    vec2 lo = vec2(r);
    vec2 hi = p.size - vec2(r);

    Transform t = transforms.t[p.transform_idx & TRANSFORM_INDEX_MASK];
    mat2 m = mat2(t.linear.x, t.linear.y, t.linear.z, t.linear.w);

    vec2 chunk_base = vec2(p.chunk_id - pc.camera_chunk) * CHUNK_SIZE - pc.camera_local;
    vec2 origin_rel = chunk_base + p.local_pos;

    float scale = pc.camera_zoom * pc.dpi_scale;
    vec2 half_vp = pc.viewport_size * 0.5;

    vec2 c0 = origin_rel + (m * vec2(lo.x, lo.y) + t.translation);
    vec2 c1 = origin_rel + (m * vec2(hi.x, lo.y) + t.translation);
    vec2 c2 = origin_rel + (m * vec2(lo.x, hi.y) + t.translation);
    vec2 c3 = origin_rel + (m * vec2(hi.x, hi.y) + t.translation);

    vec2 s0 = rel_to_screen(c0, scale, half_vp);
    vec2 s1 = rel_to_screen(c1, scale, half_vp);
    vec2 s2 = rel_to_screen(c2, scale, half_vp);
    vec2 s3 = rel_to_screen(c3, scale, half_vp);

    vec2 bb_min = min(min(s0, s1), min(s2, s3));
    vec2 bb_max = max(max(s0, s1), max(s2, s3));

    if (bb_min.x >= bb_max.x || bb_min.y >= bb_max.y) {
        gl_Position = vec4(0.0);
        return;
    }

    vec2 uv = vec2(float(gl_VertexIndex & 1u), float((gl_VertexIndex >> 1u) & 1u));
    vec2 p_screen = mix(bb_min, bb_max, uv);
    gl_Position = vec4(p_screen / half_vp - vec2(1.0), 0.0, 1.0);
    gl_Position.z = float(p.z_opacity >> 16) / 65535.0;
}

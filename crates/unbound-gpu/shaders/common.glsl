#ifndef UNBOUND_COMMON_GLSL
#define UNBOUND_COMMON_GLSL

// ---------------------------------------------------------------------------
// Shared GPU-side declarations.
//
// The structs below mirror `unbound-gpu::data` byte for byte. Nothing here may
// be reordered without also changing the Rust side: the binding is by physical
// device address, so there is no reflection to catch a mismatch.
//
//   Prim          64B  (align 64)
//   Transform     32B  (align 16)
//   ClipNode      64B  (align 16)
//   Payload       48B  (align 16)
//   PushConstants 72B
// ---------------------------------------------------------------------------

#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_nonuniform_qualifier : require

#define CHUNK_SIZE 4096.0
#define TRANSFORM_ANIMATED_BIT 0x80000000u
#define TRANSFORM_INDEX_MASK   0x7FFFFFFFu
#define CLIP_NONE              0xFFFFFFFFu

// Primitive kinds, matching `PrimType`.
#define PRIM_ROUNDED_RECT 0u
#define PRIM_IMAGE        1u
#define PRIM_GLYPH        2u
#define PRIM_GLYPH_COLOR  3u
#define PRIM_CURVE        4u
#define PRIM_STROKE       5u
#define PRIM_SHADOW       6u
#define PRIM_LINE         7u

// Primitive flags, mirroring `PrimFlags` on the Rust side. They travel in the
// high 16 bits of `type_flags`, so both the vertex and the fragment stage can
// read them without a second unpacking convention.
#define SUB_ELEMENT_MASK 0x7u
#define FLAG_DARK_BG      (1u << 3)
#define FLAG_MSDF         (1u << 4)
#define FLAG_ELLIPSIS     (1u << 5)
#define FLAG_SUBPIXEL     (1u << 6)
// Bits 8..10 are the P3.2 graphics markers. They skip 5 and 6 because those
// already carry the text-run markers above.
#define FLAG_GRADIENT_LINEAR (1u << 8)
#define FLAG_GRADIENT_RADIAL (1u << 9)
#define FLAG_INNER_SHADOW    (1u << 10)
#define FLAG_GRADIENT_ANY    (FLAG_GRADIENT_LINEAR | FLAG_GRADIENT_RADIAL)

// `SubElement::InnerShadow`, low three bits of the flags word.
#define SUB_INNER_SHADOW 3u

// Theme LUT slot 0 is always fully transparent so "no colour" is free.
#define TOKEN_NONE 0u

// Theme LUT slot 1 holds the shadow colour. The shadow is a *theme* concept
// — every primitive shares it — so it lives in a fixed slot rather than in a
// per-primitive field, and the slot index is part of the engine's contract
// with whoever fills the LUT.
#define SHADOW_TOKEN 1u

struct Prim {
    ivec2 chunk_id;          //  0
    vec2  local_pos;         //  8
    vec2  size;              // 16
    uvec2 radii_packed;      // 24 (4 × f16)
    uint  border_and_shadow; // 32 (u16 border in the low half | u16 shadow in the high half)
    uint  transform_idx;     // 36
    uint  clip_idx;          // 40
    uint  fill;              // 44
    uint  border_or_tex;     // 48
    uint  type_flags;        // 52
    uint  z_opacity;         // 56
    uint  extra_payload_idx; // 60
};

struct Transform {
    vec4  linear;      // 2×2 column major
    vec2  translation;
    float scale_hint;
    uint  flags;
};

// The `vec4`s come first, then the two `uvec2`s. In std430 a `vec4` must be
// 16-byte aligned, so interleaving `vec4` and `uvec2` members would push the
// second `vec4` to offset 48 and give the struct an 80-byte stride — while
// the `repr(C)` Rust mirror (`GpuClipNode`) has `radii_0` as a `[u16; 4]` with
// 2-byte alignment and stays 64 bytes. Keeping the wide members together
// makes both layouts agree; `data.rs` pins the offsets in a unit test.
struct ClipNode {
    vec4  aabb;      // chunk-local (min_x, min_y, max_x, max_y)
    vec4  rounded0;  // (center_x, center_y, half_w, half_h)
    vec4  rounded1;
    uvec2 radii0;    // 4 × f16
    uvec2 radii1;
};

struct Payload {
    vec2  p0;
    vec2  p1;
    vec2  p2;
    float line_width;
    uint  extra;
    uint  flags;
    uint  pad0;
    uint  pad1;
    uint  pad2;
};

layout(buffer_reference, std430, buffer_reference_align = 64) buffer PrimBuf {
    Prim p[];
};
layout(buffer_reference, std430, buffer_reference_align = 16) buffer TransformBuf {
    Transform t[];
};
layout(buffer_reference, std430, buffer_reference_align = 16) buffer ClipBuf {
    ClipNode c[];
};
layout(buffer_reference, std430, buffer_reference_align = 16) buffer PayloadBuf {
    Payload p[];
};
// Theme LUT: one RGBA16F entry per token, stored as two uvec2 halves.
layout(buffer_reference, std430, buffer_reference_align = 8) buffer LutBuf {
    uvec2 e[];
};

// One push-constant block for every pipeline in the frame.
//
// The first 72 bytes are the documented `FramePushConstants`; the tail is the
// compute-stage scratch (digit index, counts, auxiliary buffer addresses).
// Graphics pipelines push all 128 bytes with a zeroed tail, so a single
// `VkPipelineLayout` serves the uber shader and every compute stage.
layout(push_constant) uniform PC {
    uint64_t primitives_bda;   //   0
    uint64_t transforms_bda;   //   8
    uint64_t clips_bda;        //  16
    uint64_t theme_lut_bda;    //  24
    uint64_t payloads_bda;     //  32
    ivec2    camera_chunk;     //  40
    vec2     camera_local;     //  48
    vec2     viewport_size;    //  56
    float    camera_zoom;      //  64
    float    dpi_scale;        //  68
    uint     prim_count;       //  72
    uint     digit;            //  76
    uint     block_count;      //  80
    uint     aux;              //  84
    uint64_t scratch_bda;      //  88
    uint64_t scratch_bda2;     //  96
    uint64_t scratch_bda3;     // 104
    uint64_t scratch_bda4;     // 112
    uint64_t scratch_bda5;     // 120
} pc;

// `prim_count` etc. are only meaningful for compute stages; graphics stages
// receive zeros there.

// ---------------------------------------------------------------------------
// Unpacking
// ---------------------------------------------------------------------------

vec4 unpack_radii(uvec2 packed) {
    return vec4(unpackHalf2x16(packed.x), unpackHalf2x16(packed.y));
}

// `border_and_shadow` packs the border width in the *low* half and the
// shadow blur in the high half, matching the Rust struct layout
// (`border_width: u16` at offset 32, then `shadow_blur: u16` at offset 34).
//
// `unpackHalf2x16` returns the 16 *least*-significant bits in `.x` and the
// 16 most-significant bits in `.y` (GLSL 4.x reference / SPIR-V
// `GLSL.std.450` §UnpackHalf2x16), so the two map straight across.
float unpack_border(uint v) {
    return unpackHalf2x16(v).x;
}

float unpack_shadow(uint v) {
    return unpackHalf2x16(v).y;
}

float unpack_opacity(uint z_opacity) {
    return float(z_opacity & 0xFFFFu) / 65535.0;
}

vec4 theme_color(uint token) {
    LutBuf lut = LutBuf(pc.theme_lut_bda);
    uvec2 e = lut.e[token];
    return vec4(unpackHalf2x16(e.x), unpackHalf2x16(e.y));
}

// Gradient stops travel as two 16-bit theme tokens packed into the bits of
// `Payload::p2.x` (see `GpuPayload::gradient_linear`).
vec4 gradient_color(Payload pl, float t) {
    uint packed = floatBitsToUint(pl.p2.x);
    vec4 c0 = theme_color(packed & 0xFFFFu);
    vec4 c1 = theme_color(packed >> 16);
    return mix(c0, c1, clamp(t, 0.0, 1.0));
}

// One LSB of ordered noise, centred on zero. Long gradients on a linear
// swapchain show Mach bands without it.
float gradient_dither(vec2 frag_coord) {
    float n = fract(sin(dot(frag_coord, vec2(12.9898, 78.233))) * 43758.5453);
    return (n - 0.5) * (1.0 / 255.0);
}

// ---------------------------------------------------------------------------
// Space conversions. `scale` folds canvas zoom and DPI together because both
// are uniform positive scales: (cam_zoom * dpi) is exactly the pixel-per-unit
// factor the canvas needs, and for UI primitives cam_zoom == 1.
// ---------------------------------------------------------------------------

vec2 rel_to_screen(vec2 rel, float scale, vec2 half_vp) {
    return rel * scale + half_vp;
}

vec2 screen_to_rel(vec2 screen, float scale, vec2 half_vp) {
    return (screen - half_vp) / scale;
}

mat2 inverse2x2(mat2 m) {
    float det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    // A singular matrix cannot be inverted; fall back to identity so the
    // primitive still rasterises (just un-rotated) instead of vanishing.
    if (abs(det) < 1e-12) {
        return mat2(1.0);
    }
    float inv = 1.0 / det;
    return mat2(m[1][1] * inv, -m[0][1] * inv, -m[1][0] * inv, m[0][0] * inv);
}

// ---------------------------------------------------------------------------
// Signed distance fields
// ---------------------------------------------------------------------------

// Rounded box with four independent corner radii, in the box's local space
// where the centre is the origin.
float sd_rounded_box(vec2 p, vec2 half_size, vec4 radii) {
    // Pick the radius belonging to the quadrant of `p`.
    float r = (p.x > 0.0)
        ? ((p.y > 0.0) ? radii.z : radii.y)   // bottom-right / top-right
        : ((p.y > 0.0) ? radii.w : radii.x);  // bottom-left  / top-left
    // Clamp so opposite radii can never overlap and fold the SDF inside out.
    r = min(r, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

float coverage_from_sd(float sd, float aa_width) {
    // 1 inside, 0 outside, linear ramp one pixel wide. `aa_width` is the
    // derivative of the SDF along the screen, so the edge is always exactly one
    // pixel wide regardless of zoom.
    return clamp(0.5 - sd / max(aa_width, 1e-6), 0.0, 1.0);
}

// Abramowitz & Stegun 7.1.26 — max error 1.5e-7, no texture, no branches.
float erf_approx(float x) {
    float s = sign(x);
    float a = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * a);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t
                      - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

// Analytic Gaussian-blurred box coverage: the convolution of a half-plane with
// a Gaussian is an erf, and for distances beyond ~3σ the error is < 0.3%.
float box_shadow_alpha(float sd, float sigma) {
    if (sigma <= 0.0) {
        return sd < 0.0 ? 1.0 : 0.0;
    }
    return 0.5 - 0.5 * erf_approx(sd / (sigma * 1.41421356));
}

// ---------------------------------------------------------------------------
// Quadratic Bézier: numerically stable closest-point distance (Cardano).
// `p0`, `p1`, `p2` are the control polygon; the function returns the distance
// from `p` to the curve, not to the polygon.
// ---------------------------------------------------------------------------

float dot2(vec2 v) {
    return dot(v, v);
}

/// Distance from `p` to the line segment `a..b`.
float sd_segment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

float sd_quadratic_bezier(vec2 pos, vec2 p0, vec2 p1, vec2 p2) {
    vec2 a = p1 - p0;
    vec2 b = p0 - 2.0 * p1 + p2;
    vec2 c = a * 2.0;
    vec2 d = p0 - pos;

    // A collinear control polygon — a "curve" that is really a straight line —
    // makes `b` vanish, and the Cardano solve below divides by `|b|²` clamped
    // at 1e-12. The roots then explode to ±1e12, clamp to `t = 1`, and the
    // distance collapses to the *end point*: the stroke disappears and only a
    // dot is left. Straight strokes (the window close glyph, a flat ribbon)
    // take this path, so fall back to the exact segment distance.
    if (dot(b, b) < 1e-8) {
        return sd_segment(pos, p0, p2);
    }

    float kk = 1.0 / max(dot(b, b), 1e-12);
    float kx = kk * dot(a, b);
    float ky = kk * (2.0 * dot(a, a) + dot(d, b)) / 3.0;
    float kz = kk * dot(d, a);

    float res;
    float p = ky - kx * kx;
    float p3 = p * p * p;
    float q = kx * (2.0 * kx * kx - 3.0 * ky) + kz;
    float h = q * q + 4.0 * p3;

    if (h >= 0.0) {
        // One real root.
        h = sqrt(h);
        vec2 x = (vec2(h, -h) - q) * 0.5;
        vec2 uv = sign(x) * pow(abs(x), vec2(1.0 / 3.0));
        float t = clamp(uv.x + uv.y - kx, 0.0, 1.0);
        res = dot2(d + (c + b * t) * t);
    } else {
        // Three real roots: use the trigonometric form to stay stable.
        float z = sqrt(-p);
        float v = acos(clamp(q / (p * z * 2.0), -1.0, 1.0)) / 3.0;
        float m = cos(v);
        float n = sin(v) * 1.732050808;
        vec3 t = clamp(vec3(m + m, -n - m, n - m) * z - kx, 0.0, 1.0);
        res = min(
            dot2(d + (c + b * t.x) * t.x),
            min(dot2(d + (c + b * t.y) * t.y), dot2(d + (c + b * t.z) * t.z)));
    }
    return sqrt(res);
}

#endif // UNBOUND_COMMON_GLSL

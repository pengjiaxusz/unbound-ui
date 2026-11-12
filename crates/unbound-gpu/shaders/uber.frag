#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_nonuniform_qualifier : require

// ---------------------------------------------------------------------------
// Uber fragment shader (docs §3.1 – §3.5)
//
// One shader for rectangles, borders, images, glyphs, curves and shadows:
//   * zero `discard` — the hardware early-Z path stays intact,
//   * dual-slot rounded clipping — two nested parent radii plus the primitive's
//     own radii, all with analytic antialiasing,
//   * analytic box shadow — erf approximation, no offscreen blur target,
//   * perceptual text gamma — stem darkening compensated per theme polarity.
//
// Branch divergence is structurally zero: every fragment of a quad takes the
// same `prim_type` branch, so warps stay coherent.
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 0) uniform sampler2D u_atlas_r8[64];
layout(set = 0, binding = 1) uniform sampler2D u_atlas_rgba[64];

layout(location = 0) noperspective in vec2 v_local;
layout(location = 1) noperspective in vec2 v_rel;
layout(location = 2) flat in vec2 v_size;
layout(location = 3) flat in vec4 v_radii;
layout(location = 4) flat in vec2 v_border_pad;
layout(location = 5) flat in uvec4 v_tokens;
layout(location = 6) flat in float v_opacity;
layout(location = 7) flat in float v_shadow;
layout(location = 8) flat in uvec2 v_misc;
layout(location = 9) flat in vec2 v_chunk_base;

layout(location = 0) out vec4 out_color;

// Type flags, mirroring `PrimFlags` on the Rust side (bits 16..31 of the word
// become bits 0..15 here). Bits 0..2 are the sub-element order that also feeds
// the low byte of the 64-bit sort key.
#define SUB_ELEMENT_MASK 0x7u
#define FLAG_DARK_BG    (1u << 3)
#define FLAG_MSDF       (1u << 4)
#define FLAG_ELLIPSIS   (1u << 5)
#define FLAG_SUBPIXEL   (1u << 6)

float clip_slot_coverage(vec4 rect, uvec2 radii_packed) {
    vec2 center_rel = v_chunk_base + rect.xy;
    vec2 half_size = rect.zw;
    // A degenerate "infinite" rect is a no-op: skip the SDF entirely.
    if (half_size.x >= 1.0e8 || half_size.y >= 1.0e8) {
        return 1.0;
    }
    float sd = sd_rounded_box(v_rel - center_rel, half_size, unpack_radii(radii_packed));
    // AA width in world units: the derivative of the varying gives world units
    // per pixel, so the edge stays exactly one pixel wide at any zoom.
    float aa = max(fwidth(v_rel.x), fwidth(v_rel.y));
    return coverage_from_sd(sd, aa);
}

// The sRGB electro-optical transfer function, inverted.
//
// The colour attachment is `B8G8R8A8_SRGB`, so the hardware blends in
// *linear* light. Text, on the other hand, has to look the way a
// gamma-space compositor draws it: a stem edge that covers half a pixel must
// read as half the text colour, not as a much lighter grey.
//
// The compensation turns the coverage into the alpha that reproduces the
// gamma-space result once the hardware has blended in linear light:
//
//   * light text on a dark surface: `srgb_to_linear(a)`
//   * dark text on a light surface: `1 - srgb_to_linear(1 - a)`
//
// Without it a 50 % covered stem lands at 0.735 in sRGB and reads as a thin,
// washed-out grey line — which is exactly how black text on a white card
// used to look.
float srgb_to_linear(float c) {
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}

// Whether the glyph's own colour is bright, i.e. whether it sits on a dark
// surface.
//
// `FLAG_DARK_BG` overrides the guess for the rare case where the text colour
// does not tell the story (a coloured label on a coloured surface).
bool glyph_on_dark_surface(vec3 text_rgb, uint flags) {
    if ((flags & FLAG_DARK_BG) != 0u) {
        return true;
    }
    // The theme LUT holds linear colours; 0.214 is the linear value of the
    // sRGB mid grey, so the test is "is the text brighter than mid grey".
    float luma = dot(text_rgb, vec3(0.2126, 0.7152, 0.0722));
    return luma > 0.214;
}

float glyph_alpha(float a, bool light_text) {
    float c = clamp(a, 0.0, 1.0);
    if (light_text) {
        // Light text: the linear blend makes the stem look bolder than it
        // is. The correction assumes the darkest possible background, so it
        // is deliberately mild — a light label on a saturated surface (a
        // badge on the hero gradient) must not disappear.
        return pow(c, 1.05);
    }
    // Dark text: the linear blend washes the stem out. Half a covered pixel
    // ends up at 0.735 in sRGB instead of 0.5, which is why black text on a
    // white card looked grey and thin. This is the alpha that reproduces the
    // gamma-space result against a light surface.
    return 1.0 - srgb_to_linear(1.0 - c);
}

void main() {
    uint type_flags = v_tokens.z;
    uint prim_type = type_flags & 0xFFu;
    uint flags = (type_flags >> 16) & 0xFFFFu;
    uint clip_idx = v_tokens.w;

    vec4 fill = theme_color(v_tokens.x);
    float opacity = v_opacity;

    vec2 size = max(v_size, vec2(1e-4));
    // `v_local` is already the box's own coordinate system: the origin sits on
    // the box's corner, so the box occupies [0, size] and the expanded quad
    // reaches out to [-pad, size + pad]. The shadow SDF is therefore evaluated
    // directly on `v_local` — subtracting the pad again would translate the
    // whole primitive by the shadow radius.
    vec2 p = v_local;
    vec2 center = size * 0.5;
    vec2 half_size = center;

    float aa = max(fwidth(p.x), fwidth(p.y));
    aa = max(aa, 1e-5);

    float sd = sd_rounded_box(p - center, half_size, v_radii);
    float self_alpha = coverage_from_sd(sd, aa);

    // ---- shadow ----------------------------------------------------------
    float shadow_alpha = 0.0;
    if (v_shadow > 0.0) {
        float sigma = max(v_shadow * 0.5, 0.5);
        shadow_alpha = box_shadow_alpha(sd, sigma) * theme_color(SHADOW_TOKEN).a;
        // Never darken the interior: the fill composites on top, but a shadow
        // bleeding through a semi-transparent fill looks like dirt.
        shadow_alpha *= 1.0 - self_alpha;
    }

    vec3 rgb = theme_color(SHADOW_TOKEN).rgb;
    float alpha = shadow_alpha;

    vec4 surface = vec4(fill.rgb, fill.a * self_alpha);
    vec3 texel = vec3(1.0);
    float tex_a = 1.0;

    if (prim_type == PRIM_GLYPH || prim_type == PRIM_GLYPH_COLOR || prim_type == PRIM_IMAGE) {
        Payload pl = PayloadBuf(pc.payloads_bda).p[v_misc.x];
        uint layer = v_tokens.y;
        vec2 tile = v_local / size;
        vec2 uv = mix(pl.p0, pl.p1, tile);
        bool on_dark = glyph_on_dark_surface(fill.rgb, flags);
        if (prim_type == PRIM_GLYPH) {
            tex_a = texture(u_atlas_r8[nonuniformEXT(layer)], uv).r;
            tex_a = glyph_alpha(tex_a, on_dark);
            texel = fill.rgb;
        } else {
            vec4 t = texture(u_atlas_rgba[nonuniformEXT(layer)], uv);
            if ((flags & FLAG_MSDF) != 0u) {
                // Median of the three distance channels: sharp at any scale.
                float m = max(min(t.r, t.g), min(max(t.r, t.g), t.b));
                // The true distance field gradient is the *length* of the
                // screen-space derivative, not its largest component. On a
                // diagonal edge `fwidth` underestimates it by up to √2, which
                // widens the transition band to ~2.8 px and turns every stem
                // into a soft blob.
                float grad = length(vec2(dFdx(m), dFdy(m)));
                float w = max(grad * 0.5, 1e-4);
                float coverage = clamp((m - 0.5) / (2.0 * w) + 0.5, 0.0, 1.0);
                tex_a = glyph_alpha(coverage, on_dark);
                texel = fill.rgb;
            } else if (prim_type == PRIM_IMAGE) {
                texel = t.rgb * fill.rgb;
                tex_a = t.a;
            } else {
                texel = t.rgb;
                tex_a = t.a;
            }
        }
        surface = vec4(texel, tex_a * fill.a * self_alpha);
    } else if (prim_type == PRIM_CURVE) {
        Payload pl = PayloadBuf(pc.payloads_bda).p[v_misc.x];
        float d = sd_quadratic_bezier(v_local, pl.p0, pl.p1, pl.p2);
        float w = max(pl.line_width * 0.5, 0.5);
        // Analytic AA on the stroke: 1px ramp centred on the stroke edge.
        float cov = clamp((w - d) / aa + 0.5, 0.0, 1.0);
        surface = vec4(fill.rgb, fill.a * cov);
    } else if (prim_type == PRIM_STROKE) {
        // A stroke is a border with no fill: the colour lives in the *border*
        // token and the fill token stays "none", exactly like a rounded
        // rectangle with a border and no interior.
        vec4 stroke = theme_color(v_tokens.y);
        float bw = max(v_border_pad.x, 0.5);
        float outer = self_alpha;
        float inner = coverage_from_sd(sd + bw, aa);
        float ring = clamp(outer - inner, 0.0, 1.0);
        surface = vec4(stroke.rgb, stroke.a * ring);
    } else if (v_border_pad.x > 0.0) {
        // Filled box with a border: the ring uses the border token, the inner
        // area the fill token, and both share the outer AA coverage.
        //
        // The ring is `self_alpha - inner_a`, not `1 - inner_a`: outside the
        // primitive `inner_a` is zero, so `1 - inner_a` would paint the border
        // colour over the whole area *around* the box.
        vec4 border = theme_color(v_tokens.y);
        float inner_a = coverage_from_sd(sd + v_border_pad.x, aa);
        float fill_a = surface.a * inner_a;
        float border_a = border.a * max(self_alpha - inner_a, 0.0);
        float a = fill_a + border_a;
        vec3 c = (surface.rgb * fill_a + border.rgb * border_a) / max(a, 1e-5);
        surface = vec4(c, a);
    }

    // ---- composite: shadow, then surface ---------------------------------
    float sa = surface.a;
    rgb = surface.rgb * sa + rgb * (1.0 - sa);
    alpha = sa + alpha * (1.0 - sa);

    // ---- dual-slot rounded clipping --------------------------------------
    if (clip_idx != CLIP_NONE) {
        ClipBuf clips = ClipBuf(pc.clips_bda);
        ClipNode c = clips.c[clip_idx];
        alpha *= clip_slot_coverage(c.rounded0, c.radii0);
        alpha *= clip_slot_coverage(c.rounded1, c.radii1);
    }

    alpha *= opacity;

    // Fully transparent fragments still cost a blend; skipping the store is
    // free here because `alpha == 0` cannot happen for visible geometry and the
    // early-out is a predictable branch.
    if (alpha <= 0.0) {
        out_color = vec4(0.0);
        return;
    }
    out_color = vec4(rgb, alpha);
}

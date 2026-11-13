#version 460

// ---------------------------------------------------------------------------
// Backdrop composite (P1.4).
//
// Samples the full-resolution blurred backdrop and keeps it only inside the
// rounded rectangle that requested it. The scissor-free full-screen triangle
// keeps the pass to one draw; the SDF mask supplies the rounded corners.
// ---------------------------------------------------------------------------

layout(set = 0, binding = 0) uniform sampler2D u_blurred;

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

layout(push_constant) uniform BackdropCompositePush {
    // Rectangle in physical pixels: (x, y, width, height).
    vec4 rect;
    // Corner radii: top-left, top-right, bottom-right, bottom-left.
    vec4 radii;
} pc;

float sd_rounded_box(vec2 p, vec2 half_size, vec4 radii) {
    float r = (p.x < 0.0)
        ? ((p.y < 0.0) ? radii.x : radii.w)
        : ((p.y < 0.0) ? radii.y : radii.z);
    r = clamp(r, 0.0, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 size = vec2(textureSize(u_blurred, 0));
    vec2 center = pc.rect.xy + pc.rect.zw * 0.5;
    float sd = sd_rounded_box(gl_FragCoord.xy - center, pc.rect.zw * 0.5, pc.radii);
    float alpha = clamp(0.5 - sd, 0.0, 1.0);
    if (alpha <= 0.0) {
        discard;
    }
    vec4 texel = texture(u_blurred, gl_FragCoord.xy / size);
    out_color = vec4(texel.rgb, texel.a * alpha);
}

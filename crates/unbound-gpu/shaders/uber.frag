#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_nonuniform_qualifier : require

// ---------------------------------------------------------------------------
// Uber 片元着色器（参见 `docs/architecture/gpu-renderer.md`）。
//
// 单一着色器统一处理矩形、边框、图像、字形、曲线与阴影：
//   * 零 `discard`——保持硬件 Early-Z 路径完整，
//   * 双槽位圆角裁剪——支持两层嵌套父级圆角加图元自身圆角，均具备解析抗锯齿，
//   * 解析盒阴影——基于 erf 近似，无需离屏模糊目标，
//   * 感知文本伽马校正——按主题明暗极性补偿字干变粗效应。
//
// 结构上无分支发散：同一四边形的所有片元走相同的 `prim_type` 分支，warp 保持高度一致。
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

// 类型标志位，镜像 Rust 侧 `PrimFlags`（字的第 16..31 位在此变为 0..15 位）。
// 第 0..2 位为子元素顺序，同时用于构成 64 位排序键的低字节。`FLAG_*` 位定义在
// `common.glsl` 中，因为顶点着色器也需要 `FLAG_MSDF`。

float clip_slot_coverage(vec4 rect, uvec2 radii_packed) {
    vec2 center_rel = v_chunk_base + rect.xy;
    vec2 half_size = rect.zw;
    // 退化的“无限大”矩形为空操作：直接跳过 SDF 计算。
    if (half_size.x >= 1.0e8 || half_size.y >= 1.0e8) {
        return 1.0;
    }
    float sd = sd_rounded_box(v_rel - center_rel, half_size, unpack_radii(radii_packed));
    // 世界空间单位下的抗锯齿宽度：varying 的导数给出每像素对应的世界单位，
    // 使边缘在任意缩放下恒为 1 像素宽。
    float aa = max(fwidth(v_rel.x), fwidth(v_rel.y));
    return coverage_from_sd(sd, aa);
}

// 判断字形自身颜色是否较亮（即是否位于深色表面上）。
//
// `FLAG_DARK_BG` 用于覆盖少数仅凭文本颜色无法判断的场景（如彩色背景上的彩色标签）。
bool glyph_on_dark_surface(vec3 text_rgb, uint flags) {
    if ((flags & FLAG_DARK_BG) != 0u) {
        return true;
    }
    // 主题 LUT 存储线性颜色；0.214 为 sRGB 中灰的线性值，此处判断文本是否亮于中灰。
    float luma = dot(text_rgb, vec3(0.2126, 0.7152, 0.0722));
    return luma > 0.214;
}

float glyph_alpha(float a, bool light_text) {
    float c = clamp(a, 0.0, 1.0);
    // 在单处执行温和校正。
    //
    // 交换链格式为 `B8G8R8A8_SRGB`，硬件在线性光空间混合字形覆盖率：半覆盖的黑字
    // 像素在 sRGB 下会落在 0.735，视觉上发灰。适度的伽马提升可恢复原生伽马空间
    // 合成器的字干粗细。
    //
    // 提升幅度刻意控制得较小。过去在图集构建时叠加第二层曲线（以及本函数内再叠第三层）
    // 会将 10% 的抗锯齿曲线边缘放大到 28%，使每个圆角（如字母 `O` 的上下左右切点）
    // 变成发黑的方形污迹。
    return light_text ? pow(c, 1.15) : pow(c, 0.75);
}

void main() {
    uint type_flags = v_tokens.z;
    uint prim_type = type_flags & 0xFFu;
    uint flags = (type_flags >> 16) & 0xFFFFu;
    uint clip_idx = v_tokens.w;

    vec4 fill = theme_color(v_tokens.x);
    float opacity = v_opacity;

    vec2 size = max(v_size, vec2(1e-4));
    // `v_local` 已处于矩形自身坐标系：原点在矩形角点，矩形占据 [0, size]，而外扩后的
    // 四边形范围为 [-pad, size + pad]。因此阴影 SDF 直接基于 `v_local` 求值——
    // 若再次减去 pad 会导致整个图元按阴影半径发生平移。
    vec2 p = v_local;
    vec2 center = size * 0.5;
    vec2 half_size = center;

    float aa = max(fwidth(p.x), fwidth(p.y));
    aa = max(aa, 1e-5);

    // 字形四边形是图集矩形而非圆角框：覆盖率已存储在字形位图中，圆角矩形 SDF
    // 绝不可对其裁剪或淡化。跳过它还能省去每个字形片元上的 `fwidth`/SDF 计算。
    bool is_glyph = prim_type == PRIM_GLYPH || prim_type == PRIM_GLYPH_COLOR;
    float sd = sd_rounded_box(p - center, half_size, v_radii);
    float self_alpha = is_glyph ? 1.0 : coverage_from_sd(sd, aa);

    // `InnerShadow` 子元素绘制在矩形内部，绝不可同时投射外部投影。
    uint sub_element = flags & SUB_ELEMENT_MASK;
    bool is_inner_shadow =
        (flags & FLAG_INNER_SHADOW) != 0u || sub_element == SUB_INNER_SHADOW;

    // ---- 外阴影 ----------------------------------------------------------
    float shadow_alpha = 0.0;
    if (v_shadow > 0.0 && !is_inner_shadow) {
        float sigma = max(v_shadow * 0.5, 0.5);
        shadow_alpha = box_shadow_alpha(sd, sigma) * theme_color(SHADOW_TOKEN).a;
        // 绝不加深内部区域：虽然填充层会合成在上方，但阴影透出半透明填充会显得发脏。
        shadow_alpha *= 1.0 - self_alpha;
    }

    vec3 rgb = theme_color(SHADOW_TOKEN).rgb;
    float alpha = shadow_alpha;

    vec4 surface = vec4(fill.rgb, fill.a * self_alpha);
    vec3 texel = vec3(1.0);
    float tex_a = 1.0;

    // ---- 渐变填充 --------------------------------------------------------
    //
    // 在边框分支之前执行，使渐变保留外环：边框分支仅合成 `surface`，不重新计算内部。
    if (prim_type == PRIM_ROUNDED_RECT && (flags & FLAG_GRADIENT_ANY) != 0u) {
        Payload pl = PayloadBuf(pc.payloads_bda).p[v_misc.x];
        vec2 n = v_local / size;
        float t;
        if ((flags & FLAG_GRADIENT_RADIAL) != 0u) {
            // `p1` 为逐轴半径向量，因此天然支持椭圆渐变。
            vec2 r = max(abs(pl.p1), vec2(1e-5));
            t = length((n - pl.p0) / r);
        } else {
            vec2 d = pl.p1 - pl.p0;
            t = dot(n - pl.p0, d) / max(dot(d, d), 1e-8);
        }
        t = clamp(t, 0.0, 1.0);
        float e = pl.p2.y;
        if (e > 0.0 && abs(e - 1.0) > 1e-4) {
            t = pow(t, e);
        }
        vec4 g = gradient_color(pl, t);
        // 叠加 1 LSB 蓝噪声抖动：在线性交换链上，8 位输出量化会使长渐变出现马赫带。
        surface = vec4(g.rgb + gradient_dither(gl_FragCoord.xy), g.a * self_alpha);
    }

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
                // 三通道距离取中值：在任意缩放下均保持锐利。
                float m = max(min(t.r, t.g), min(max(t.r, t.g), t.b));
                // 真实的距离场梯度是屏幕空间导数的*模长*，而非最大分量。在斜边上 `fwidth`
                // 会低估多达 √2 倍，导致过渡带展宽至 ~2.8 px 并使字干糊成一团。
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
        // 描边解析抗锯齿：以描边边缘为中心的 1px 过渡带。
        float cov = clamp((w - d) / aa + 0.5, 0.0, 1.0);
        surface = vec4(fill.rgb, fill.a * cov);
    } else if (prim_type == PRIM_STROKE) {
        // 描边即无填充的边框：颜色存放在边框令牌中，填充令牌保持为“无”，
        // 与有边框无内部填充的圆角矩形完全一致。
        vec4 stroke = theme_color(v_tokens.y);
        float bw = max(v_border_pad.x, 0.5);
        float outer = self_alpha;
        float inner = coverage_from_sd(sd + bw, aa);
        float ring = clamp(outer - inner, 0.0, 1.0);
        surface = vec4(stroke.rgb, stroke.a * ring);
    } else if (v_border_pad.x > 0.0) {
        // 带边框的填充矩形：外环使用边框令牌，内部使用填充令牌，两者共享外边缘抗锯齿覆盖率。
        //
        // 外环覆盖率为 `self_alpha - inner_a` 而非 `1 - inner_a`：在图元外部 `inner_a` 为 0，
        // `1 - inner_a` 会把边框颜色涂满矩形周围的整个区域。
        vec4 border = theme_color(v_tokens.y);
        float inner_a = coverage_from_sd(sd + v_border_pad.x, aa);
        float fill_a = surface.a * inner_a;
        float border_a = border.a * max(self_alpha - inner_a, 0.0);
        float a = fill_a + border_a;
        vec3 c = (surface.rgb * fill_a + border.rgb * border_a) / max(a, 1e-5);
        surface = vec4(c, a);
    }

    // ---- 解析内阴影 ------------------------------------------------------
    //
    // `1 - blurred coverage` 即经典内阴影：边缘处恰好为 0.5，向内约 3σ 处衰减至 0，
    // 并由 `self_alpha` 裁剪以防溢出圆角矩形外部。填充令牌即为阴影颜色；
    // 子元素排序会将其绘制在内容之上。
    if (is_inner_shadow && !is_glyph) {
        float sigma = max(v_shadow * 0.5, 0.5);
        float inner_a = (1.0 - box_shadow_alpha(sd, sigma)) * self_alpha;
        surface = vec4(fill.rgb, fill.a * inner_a);
    }

    // ---- 合成：先阴影，后表面 ---------------------------------------------
    //
    // 混合状态为 `SRC_ALPHA, ONE_MINUS_SRC_ALPHA`，即硬件会再次将输出颜色乘以输出 alpha。
    // 因此着色器必须输出*非预乘*（straight）RGB：若在此将表面与阴影颜色混合并保留
    // `surface.rgb * sa`，硬件会二次乘以 `sa`，产生 `sa^2` 衰减并使深色阴影渗入每条
    // 抗锯齿边缘（导致白字带黑边、同色圆角矩形出现灰弧）。
    //
    // 非预乘 alpha 的 Over 合成需在分母除以合成后的总 alpha：
    // `out = (src·a_src + dst·a_dst·(1-a_src)) / a_out`。
    float sa = surface.a;
    float shadow_term = alpha * (1.0 - sa);
    float out_a = sa + shadow_term;
    vec3 premul = surface.rgb * sa + rgb * shadow_term;
    rgb = out_a > 1e-5 ? premul / out_a : vec3(0.0);
    alpha = out_a;

    // ---- 双槽位圆角裁剪 --------------------------------------------------
    if (clip_idx != CLIP_NONE) {
        ClipBuf clips = ClipBuf(pc.clips_bda);
        ClipNode c = clips.c[clip_idx];
        alpha *= clip_slot_coverage(c.rounded0, c.radii0);
        alpha *= clip_slot_coverage(c.rounded1, c.radii1);
    }

    alpha *= opacity;

    // 全透明片元仍会消耗一次混合开销；此处跳过输出写入几乎零成本，
    // 因为可见几何体不会出现 `alpha == 0`，且该早退是高度可预测的分支。
    if (alpha <= 0.0) {
        out_color = vec4(0.0);
        return;
    }
    out_color = vec4(rgb, alpha);
}

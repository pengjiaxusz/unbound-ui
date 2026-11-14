#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require
#extension GL_EXT_nonuniform_qualifier : require

// ---------------------------------------------------------------------------
// Uber 顶点着色器（参见 `docs/architecture/gpu-renderer.md`）。
//
// 计算顺序至关重要，不可随意调换：
//   1. 基于 chunk 锚点的相机相对坐标（每帧零上传）
//   2. 独立动画变换（不进入 Taffy 布局）
//   3. 图元自身矩形的像素网格对齐
//   4. 按 3σ 外扩四边形以容纳解析阴影
//   5. 将屏幕 AABB 与折叠裁剪矩形求交裁剪
//   6. 通过 M⁻¹ 重建局部坐标（旋转四边形无切变）
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 2, std430) readonly buffer SortedIndices {
    uint indices[];
} u_sorted;

layout(location = 0) noperspective out vec2 v_local;
layout(location = 1) noperspective out vec2 v_rel;
layout(location = 2) flat out vec2 v_size;
layout(location = 3) flat out vec4 v_radii;
layout(location = 4) flat out vec2 v_border_pad;  // 边框宽度，阴影外扩半径
layout(location = 5) flat out uvec4 v_tokens;   // 填充、边框/纹理、类型|混合|标志、裁剪索引
layout(location = 6) flat out float v_opacity;
layout(location = 7) flat out float v_shadow;
layout(location = 8) flat out uvec2 v_misc;     // 载荷索引，深度
layout(location = 9) flat out vec2 v_chunk_base; // chunk_delta*4096 - cam_local

void main() {
    PrimBuf prims = PrimBuf(pc.primitives_bda);
    TransformBuf transforms = TransformBuf(pc.transforms_bda);
    ClipBuf clips = ClipBuf(pc.clips_bda);

    uint prim_index = u_sorted.indices[gl_InstanceIndex];
    Prim p = prims.p[prim_index];

    // ---- 1. 基于 chunk 锚点的相机相对原点 ----------------------------------
    vec2 chunk_base = vec2(p.chunk_id - pc.camera_chunk) * CHUNK_SIZE - pc.camera_local;
    vec2 origin_rel = chunk_base + p.local_pos;

    // ---- 2. 动画变换 -----------------------------------------------------
    Transform t = transforms.t[p.transform_idx & TRANSFORM_INDEX_MASK];
    bool animated = (p.transform_idx & TRANSFORM_ANIMATED_BIT) != 0u;
    mat2 m = mat2(t.linear.x, t.linear.y, t.linear.z, t.linear.w);
    vec2 trans = t.translation;

    // ---- 3. 阴影外扩 -----------------------------------------------------
    float shadow_blur = unpack_shadow(p.border_and_shadow);
    float pad = shadow_blur > 0.0 ? shadow_blur * 3.0 : 0.0;

    vec2 uv = vec2(float(gl_VertexIndex & 1u), float((gl_VertexIndex >> 1u) & 1u));

    float scale = pc.camera_zoom * pc.dpi_scale;
    vec2 half_vp = pc.viewport_size * 0.5;

    // ---- 4. 像素网格对齐 -------------------------------------------------
    // 将图元*自身*矩形对齐到物理像素网格：左上角对齐至像素中心/边界，
    // 尺寸由对齐后的角点决定。
    //
    // 对齐图元自身矩形（而非外扩阴影后的外框）才能让 1 px 边框精确落在像素边界上，
    // 使解析抗锯齿将其解析为单个完全覆盖的像素，而非跨两像素的半透明模糊。
    //
    // 字形尺寸必须保持精确：字形四边形的宽度严格等于图集分配的纹素数。同时对齐
    // *两个*角点会破坏这一点：CPU 以逻辑单位（`texels / scale`）存储四边形，
    // 两次缩放往返可能使某角点落在 `.5`，导致 `min` 和 `max` 各自对齐出 `W ± 1` px。
    // 将 `W` 纹素的字形拉伸到 `W + 1` 像素会导致 `VK_FILTER_LINEAR` 采样将 1 px 字干
    // 涂抹成 2–3 px 的重影。
    //
    // 因此公告牌图元（字形与非 MSDF 彩色字形）仅对齐左上角并保持整数纹素尺寸；
    // 常规几何图元继续对齐双边以确保 1 px 边框落在像素边界。
    //
    // 旋转四边形不做对齐：其屏幕空间包围盒仅为保守外包盒，对齐会裁剪掉图形。
    bool axis_aligned = abs(m[0][1]) < 1e-5 && abs(m[1][0]) < 1e-5
                      && abs(m[0][0] - 1.0) < 1e-5 && abs(m[1][1] - 1.0) < 1e-5;

    uint prim_type = p.type_flags & 0xFFu;
    uint prim_flags = (p.type_flags >> 16) & 0xFFFFu;
    bool bitmap_glyph = prim_type == PRIM_GLYPH
        || (prim_type == PRIM_GLYPH_COLOR && (prim_flags & FLAG_MSDF) == 0u);

    vec2 eff_origin = origin_rel + trans;
    vec2 eff_size = p.size;
    if (!animated && axis_aligned) {
        vec2 box_min_s = floor(rel_to_screen(eff_origin, scale, half_vp) + 0.5);
        vec2 size_s;
        if (bitmap_glyph) {
            // 一纹素对应一像素：四边形逻辑尺寸乘以缩放因子恰好等于图集分配的整数纹素数。
            size_s = max(floor(p.size * scale + 0.5), vec2(1.0));
        } else {
            vec2 box_max_s = floor(rel_to_screen(eff_origin + p.size, scale, half_vp) + 0.5);
            size_s = max(box_max_s - box_min_s, vec2(1.0));
        }
        eff_origin = screen_to_rel(box_min_s, scale, half_vp);
        eff_size = size_s / scale;
    }

    vec2 lo = vec2(-pad);
    vec2 hi = eff_size + vec2(pad);

    // 局部空间 -> 相机相对空间（先应用动画矩阵，再叠加图元原点）。
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

    // ---- 5. 裁剪 ---------------------------------------------------------
    if (p.clip_idx != CLIP_NONE) {
        ClipNode c = clips.c[p.clip_idx];
        vec2 clip_min = rel_to_screen(chunk_base + c.aabb.xy, scale, half_vp);
        vec2 clip_max = rel_to_screen(chunk_base + c.aabb.zw, scale, half_vp);
        bb_min = max(bb_min, min(clip_min, clip_max));
        bb_max = min(bb_max, max(clip_min, clip_max));
    }

    // 交集为空：折叠图元。光栅化器会零开销丢弃它——不会执行任何片元（参见 `docs/architecture/gpu-renderer.md`）。
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

    // ---- 6. 逆向局部坐标重建 ---------------------------------------------
    // 屏幕空间 -> 相机相对空间 -> 局部空间。两步均为仿射变换，因此下方 `noperspective`
    // varying 对旋转图元仍保持精确，片元着色器中的 SDF 可在真实局部空间中求值。
    //
    // 此处使用的原点为*对齐后*的原点，因此片元局部坐标是从像素对齐后的真实角点量起的。
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
    // 反向 Z（Reverse-Z）：1.0 为最近。剔除 Pass 已完成该值的归一化。
    gl_Position.z = float(p.z_opacity >> 16) / 65535.0;
}

#version 460
#extension GL_EXT_buffer_reference : require
#extension GL_EXT_shader_explicit_arithmetic_types_int64 : require

// ---------------------------------------------------------------------------
// 不透明核心预 Pass 顶点着色器（参见 `docs/architecture/gpu-renderer.md`）。
//
// 此 Pass 仅用于为大型不透明容器写入深度，使硬件 Early-Z 单元能够剔除其后方的
// 数千个图元。它刻意不产生颜色输出：渲染目标屏蔽了颜色写入且着色器无输出，
// 整个 Pass 为纯深度写入。
//
// 两条核心规则确保该 Pass 的安全性，缺一不可：
//
//   1. **仅不透明容器生成核心。** 只有完全不透明、尺寸大到值得写入深度且未被
//      裁剪的圆角矩形才符合条件；其余图元（字形、曲线、描边、边框、半透明面板）
//      均折叠为退化四边形，完全不写深度。为半透明图元生成核心比不生成更糟：
//      它写入的深度会剔除其后方的不透明像素，导致画家算法无法正确混合。
//
//   2. **核心是矩形按最大圆角半径向内收缩后的区域。** 圆角边缘不写入深度缓冲，
//      从而使 uber Pass 仍能将其抗锯齿边缘混合到容器后方的内容上。
//
// 写入的深度即为图元自身深度，因此 uber Pass 以相同深度绘制同一图元时，在核心区域
// 仍能通过 `GREATER_OR_EQUAL` 测试，而其后方所有图元均会测试失败。
//
// 本着色器不声明任何 varying：预 Pass 仅写入深度，插值颜色、圆角和裁剪数据纯粹浪费带宽。
// ---------------------------------------------------------------------------

#include "common.glsl"

layout(set = 0, binding = 2, std430) readonly buffer SortedIndices {
    uint indices[];
} u_sorted;

// 仅当容器足够大（64 x 64 逻辑像素）使 Early-Z 收益大于深度写入开销时，才生成不透明核心。
#define CORE_MIN_AREA 4096.0

void main() {
    PrimBuf prims = PrimBuf(pc.primitives_bda);
    TransformBuf transforms = TransformBuf(pc.transforms_bda);

    Prim p = prims.p[u_sorted.indices[gl_InstanceIndex]];

    // ---- 1. 资格判定 -----------------------------------------------------
    // 容器必须满足双重不透明：图元自身不透明度与填充主题色的 alpha 均须为 1。
    // 半透明面板不得拥有核心，否则会遮挡本应与之混合的后方内容。
    float opacity = float(p.z_opacity & 0xFFFFu) / 65535.0;
    vec4 fill = theme_color(p.fill);
    bool eligible = (p.type_flags & 0xFFu) == PRIM_ROUNDED_RECT
        && opacity >= 0.999
        && fill.a >= 0.999
        && p.clip_idx == CLIP_NONE
        && p.size.x * p.size.y >= CORE_MIN_AREA;

    // ---- 2. 核心矩形计算 -------------------------------------------------
    // `r` 为最大圆角半径；核心即矩形四边各向内收缩 `r` 后的区域。
    vec4 radii = unpack_radii(p.radii_packed);
    float r = max(max(radii.x, radii.y), max(radii.z, radii.w));
    vec2 core_size = p.size - vec2(2.0 * r);
    if (core_size.x <= 1.0 || core_size.y <= 1.0) {
        eligible = false;
    }

    if (!eligible) {
        // 退化四边形：所有顶点落在同一点，光栅化器不会产生任何片元，也不会写入深度。
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

#ifndef UNBOUND_COMMON_GLSL
#define UNBOUND_COMMON_GLSL

// ---------------------------------------------------------------------------
// GPU 侧共享声明。
//
// 以下结构体与 `unbound-gpu::data` 按字节严格镜像。未同步修改 Rust 侧前严禁
// 调整字段顺序：绑定基于物理设备地址（BDA），无反射机制可捕获布局不匹配。
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

// 图元类型，与 `PrimType` 一一对应。
#define PRIM_ROUNDED_RECT 0u
#define PRIM_IMAGE        1u
#define PRIM_GLYPH        2u
#define PRIM_GLYPH_COLOR  3u
#define PRIM_CURVE        4u
#define PRIM_STROKE       5u
#define PRIM_SHADOW       6u
#define PRIM_LINE         7u

// 图元标志位，镜像 Rust 侧的 `PrimFlags`。它们位于 `type_flags` 的高 16 位，
// 使顶点和片元阶段无需第二套解包约定即可直接读取。
#define SUB_ELEMENT_MASK 0x7u
#define FLAG_DARK_BG      (1u << 3)
#define FLAG_MSDF         (1u << 4)
#define FLAG_ELLIPSIS     (1u << 5)
#define FLAG_SUBPIXEL     (1u << 6)
// 第 8..10 位为 P3.2 图形标记；跳过 5 和 6 是因为这两位已被上方文本运行标记占用。
#define FLAG_GRADIENT_LINEAR (1u << 8)
#define FLAG_GRADIENT_RADIAL (1u << 9)
#define FLAG_INNER_SHADOW    (1u << 10)
#define FLAG_GRADIENT_ANY    (FLAG_GRADIENT_LINEAR | FLAG_GRADIENT_RADIAL)

// `SubElement::InnerShadow`，位于标志字的低 3 位。
#define SUB_INNER_SHADOW 3u

// 主题 LUT 槽位 0 恒为全透明，使“无颜色”零开销。
#define TOKEN_NONE 0u

// 主题 LUT 槽位 1 存放阴影颜色。阴影属于主题级概念（所有图元共享），
// 因此存放在固定槽位而非逐图元字段中；该槽位索引是引擎与 LUT 填充方之间的契约。
#define SHADOW_TOKEN 1u

struct Prim {
    ivec2 chunk_id;          //  0
    vec2  local_pos;         //  8
    vec2  size;              // 16
    uvec2 radii_packed;      // 24 (4 × f16)
    uint  border_and_shadow; // 32（低半字 u16 边框 | 高半字 u16 阴影）
    uint  transform_idx;     // 36
    uint  clip_idx;          // 40
    uint  fill;              // 44
    uint  border_or_tex;     // 48
    uint  type_flags;        // 52
    uint  z_opacity;         // 56
    uint  extra_payload_idx; // 60
};

struct Transform {
    vec4  linear;      // 2×2 列主序
    vec2  translation;
    float scale_hint;
    uint  flags;
};

// 先放 `vec4` 再放两个 `uvec2`。在 std430 中 `vec4` 必须 16 字节对齐，若交错排布
// 会将第二个 `vec4` 推至偏移 48，导致结构体步长变为 80 字节；而 Rust 侧 `repr(C)`
// 镜像（`GpuClipNode`）的 `radii_0` 为 2 字节对齐的 `[u16; 4]`，总长仍为 64 字节。
// 将宽成员集中放置可使两侧布局一致；`data.rs` 中通过单元测试锁定了这些偏移量。
struct ClipNode {
    vec4  aabb;      // chunk 局部坐标 (min_x, min_y, max_x, max_y)
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
// 主题 LUT：每个令牌占一个 RGBA16F 条目，以两个 uvec2 半部存储。
layout(buffer_reference, std430, buffer_reference_align = 8) buffer LutBuf {
    uvec2 e[];
};

// 帧内所有管线共用的 Push Constant 块。
//
// 前 72 字节为标准的 `FramePushConstants`；尾部为计算阶段暂存字段
// （位索引、计数、辅助缓冲区地址）。图形管线推送全部 128 字节并将尾部清零，
// 从而使单个 `VkPipelineLayout` 可同时服务于 uber 着色器和所有计算阶段。
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

// `prim_count` 等字段仅对计算阶段有意义；图形阶段在此处接收零值。

// ---------------------------------------------------------------------------
// 解包辅助函数
// ---------------------------------------------------------------------------

vec4 unpack_radii(uvec2 packed) {
    return vec4(unpackHalf2x16(packed.x), unpackHalf2x16(packed.y));
}

// `border_and_shadow` 将边框宽度打包在低半字、阴影模糊半径打包在高半字，
// 与 Rust 结构体布局一致（偏移 32 为 `border_width: u16`，偏移 34 为 `shadow_blur: u16`）。
//
// `unpackHalf2x16` 在 `.x` 中返回最低 16 位、在 `.y` 中返回最高 16 位
// （符合 GLSL 4.x / SPIR-V `GLSL.std.450` UnpackHalf2x16 规范），两者直接对应。
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

// 渐变色标作为两个 16 位主题令牌打包在 `Payload::p2.x` 的位中（参见 `GpuPayload::gradient_linear`）。
vec4 gradient_color(Payload pl, float t) {
    uint packed = floatBitsToUint(pl.p2.x);
    vec4 c0 = theme_color(packed & 0xFFFFu);
    vec4 c1 = theme_color(packed >> 16);
    return mix(c0, c1, clamp(t, 0.0, 1.0));
}

// 零均值、幅值为 1 LSB 的有序抖动噪声。在线性交换链上，若无此抖动，长渐变会出现马赫带。
float gradient_dither(vec2 frag_coord) {
    float n = fract(sin(dot(frag_coord, vec2(12.9898, 78.233))) * 43758.5453);
    return (n - 0.5) * (1.0 / 255.0);
}

// ---------------------------------------------------------------------------
// 空间转换。`scale` 将画布缩放与 DPI 合并，因为两者均为均匀正缩放：
// (cam_zoom * dpi) 即画布所需的每单位像素因子，且 UI 图元的 cam_zoom == 1。
// ---------------------------------------------------------------------------

vec2 rel_to_screen(vec2 rel, float scale, vec2 half_vp) {
    return rel * scale + half_vp;
}

vec2 screen_to_rel(vec2 screen, float scale, vec2 half_vp) {
    return (screen - half_vp) / scale;
}

mat2 inverse2x2(mat2 m) {
    float det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    // 奇异矩阵不可求逆；回退为单位矩阵，使图元仍能光栅化（仅不旋转）而不至于消失。
    if (abs(det) < 1e-12) {
        return mat2(1.0);
    }
    float inv = 1.0 / det;
    return mat2(m[1][1] * inv, -m[0][1] * inv, -m[1][0] * inv, m[0][0] * inv);
}

// ---------------------------------------------------------------------------
// 有向距离场（SDF）
// ---------------------------------------------------------------------------

// 具有四个独立圆角半径的圆角矩形 SDF，位于以矩形中心为原点的局部空间。
float sd_rounded_box(vec2 p, vec2 half_size, vec4 radii) {
    // 选取 `p` 所在象限对应的圆角半径。
    float r = (p.x > 0.0)
        ? ((p.y > 0.0) ? radii.z : radii.y)   // 右下 / 右上
        : ((p.y > 0.0) ? radii.w : radii.x);  // 左下 / 左上
    // 钳制半径，防止对角圆角重叠导致 SDF 内外翻转。
    r = min(r, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

float coverage_from_sd(float sd, float aa_width) {
    // 内部为 1、外部为 0，过渡带为 1 像素宽的线性斜坡。`aa_width` 为 SDF
    // 沿屏幕空间的导数，因此无论缩放比例如何，边缘抗锯齿宽度恒为 1 像素。
    return clamp(0.5 - sd / max(aa_width, 1e-6), 0.0, 1.0);
}

// Abramowitz & Stegun 7.1.26 近似——最大误差 1.5e-7，无需纹理查表，无分支。
float erf_approx(float x) {
    float s = sign(x);
    float a = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * a);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t
                      - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

// 解析高斯模糊矩形覆盖率：半平面与高斯核的卷积即为 erf，距离超过约 3σ 时误差 < 0.3%。
float box_shadow_alpha(float sd, float sigma) {
    if (sigma <= 0.0) {
        return sd < 0.0 ? 1.0 : 0.0;
    }
    return 0.5 - 0.5 * erf_approx(sd / (sigma * 1.41421356));
}

// ---------------------------------------------------------------------------
// 二次 Bézier 曲线：数值稳定的最近点距离（Cardano 求根法）。
// `p0`、`p1`、`p2` 为控制多边形顶点；函数返回 `p` 到曲线本身的距离，而非到控制多边形的距离。
// ---------------------------------------------------------------------------

float dot2(vec2 v) {
    return dot(v, v);
}

/// 点 `p` 到线段 `a..b` 的距离。
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

    // 当控制多边形共线（即实际退化为直线的“曲线”）时，`b` 趋近于零，下方 Cardano 求解
    // 会除以被钳制在 1e-12 的 `|b|²`。这会导致根爆炸至 ±1e12 并被钳制到 `t = 1`，
    // 距离退化为到端点的距离——整条描边消失，只剩一个点。直描边（如窗口关闭图标、平直带）
    // 会走此路径，因此回退到精确线段距离。
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
        // 单个实根。
        h = sqrt(h);
        vec2 x = (vec2(h, -h) - q) * 0.5;
        vec2 uv = sign(x) * pow(abs(x), vec2(1.0 / 3.0));
        float t = clamp(uv.x + uv.y - kx, 0.0, 1.0);
        res = dot2(d + (c + b * t) * t);
    } else {
        // 三个实根：采用三角函数形式以保持数值稳定。
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

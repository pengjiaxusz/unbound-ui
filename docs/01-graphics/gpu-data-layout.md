# GPU 数据结构与 BDA 内存协议

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/src/data.rs` |
> | **关键类型** | `FramePushConstants`、`GpuPrimitiveInstance`、`GpuClipNode`、`GpuTransform`、`GpuPayload` |
> | **测试** | `crates/unbound-gpu/src/data.rs` 内的布局与尺寸断言 |

所有传往 GPU 的数据结构严格使用 `#[repr(C)]` 并对齐至 $16\text{B}$ 或
$64\text{B}$ 边界。

## FramePushConstants

每帧仅 64 字节，统领全局所有 GPU 物理缓冲地址：

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FramePushConstants {
    pub primitives_bda: u64,      // 指向持久驻留池 / 瞬态环中的 GpuPrimitiveInstance 数组
    pub transforms_bda: u64,      // 指向独立动画变换通道 GpuTransform 数组 (跳开 Taffy)
    pub clips_bda: u64,           // 指向双槽位嵌套圆角裁剪块 GpuClipNode 数组
    pub theme_lut_bda: u64,       // 指向全局零开销主题颜色表 (RGBA16F)
    pub payloads_bda: u64,        // 指向曲线段控制点 / 渐变色标 / 扩展属性缓冲
    pub camera_chunk: [i32; 2],   // 摄像机所在世界区块坐标 (4096x4096 为一区块)
    pub camera_local: [f32; 2],   // 摄像机在区块内的局部 f32 坐标
    pub viewport_size: [f32; 2],  // 物理视口像素尺寸
    pub camera_zoom: f32,         // 画布摄像机缩放倍率
    pub dpi_scale: f32,           // 系统物理 DPI * 应用全局 UI 缩放
}
```

摄像机只占 $16\text{B}$，因此画布平移缩放时 GPU 侧无需更新任何图元数据。

## GpuPrimitiveInstance

64 字节紧凑型统一图元实例，恰好占满 1 条 64B CPU/GPU Cache Line：

```rust
#[repr(C, align(64))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuPrimitiveInstance {
    pub chunk_id: [i32; 2],       // 8B: 世界区块 ID (静态 UI 控件固定为 [0, 0])
    pub local_pos: [f32; 2],      // 8B: 区块内局部坐标 [0.0, 4096.0)
    pub size: [f32; 2],           // 8B: 图元几何宽高
    pub corner_radii: [u16; 4],   // 8B: 左上、右上、右下、左下圆角半径 (f16 编码)
    pub border_width: u16,        // 2B: 边框宽度 (f16 编码)
    pub shadow_blur: u16,         // 2B: 解析解阴影模糊半径 (f16 编码，0 表示无阴影)
    pub transform_idx: u32,       // 4B: 索引至 transforms_bda (最高位 bit31 标记动画激活态以旁路像素吸附)
    pub clip_idx: u32,            // 4B: 索引至 clips_bda (含硬裁剪 AABB + 两层父级圆角裁剪)
    pub fill_token_or_uv: u32,    // 4B: 颜色 Token ID 或 图集打包归一化 UV 索引
    pub border_token_or_tex: u32, // 4B: 边框颜色 Token ID 或 Bindless Texture/Atlas 层索引
    pub prim_type_and_flags: u32, // 4B: [8b 图元类型 | 8b 混合模式 | 16b 扩展标记(如亚像素相位)]
    pub z_and_opacity: u32,       // 4B: [16b 归一化深度值 | 16b UNORM 透明度]
    pub extra_payload_idx: u32,   // 4B: 指向 payloads_bda 中的二次曲线段或复杂渐变描述符
}
```

颜色以 16 位 Token ID 存储，实际颜色由主题 LUT 解析，这是[零开销主题切换](../02-ui/theme-and-zoom.md)
的前提。

### 两个索引的宿主约定

`transform_idx` 与 `extra_payload_idx` 都是**全局数组下标**：着色器直接
`TransformBuf(...).t[p.transform_idx]` / `PayloadBuf(...).p[v_misc.x]` 取值，
越界或空表读到的是未初始化的 device-local 显存。

- 静态 UI 场景不产生动画变换，`transforms` 是空表，但每个图元仍引用索引 0。
  `Renderer::prepare_upload` 因此对空表补一个 `GpuTransform::IDENTITY` 再上传；
  否则一个垃圾仿射矩阵会把整帧剔除，窗口只剩清屏色。
- 每个 widget 的 `extra_payload_idx` 是相对自身 `WidgetOutput::payloads`
  的局部下标。`SceneCollector::push` 把各 widget 的 payload 拼成一张全局表，
  必须先以 `self.payloads.len()` 为基址重定位这些下标；否则第二段文字会
  采样第一段文字的字形 UV，多个标签互相串叠。

这两类问题都不会触发验证层，只有对窗口做像素级抓屏才能发现。

## GpuClipNode

64 字节双槽位裁剪节点，支持任意嵌套 AABB + 两级父容器圆角交集裁剪：

```rust
#[repr(C, align(16))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuClipNode {
    pub aabb_clip: [f32; 4],      // 16B: CPU 预求交折叠后的硬裁剪边界 (min_x, min_y, max_x, max_y)  @ 0
    pub rounded_clip_0: [f32; 4], // 16B: 第一级父圆角容器 (center_x, center_y, half_w, half_h)        @ 16
    pub rounded_clip_1: [f32; 4], // 16B: 第二级父圆角容器 (center_x, center_y, half_w, half_h)        @ 32
    pub radii_0: [u16; 4],        // 8B:  第一级父圆角四角的半径 (f16)                              @ 48
    pub radii_1: [u16; 4],        // 8B:  第二级父圆角四角的半径 (f16)                              @ 56
}
```

> **字段顺序不是随便排的**：宽成员（`vec4`）必须挨在一起。GLSL 的 std430 强制
> `vec4` 对齐到 16 字节，如果把 `radii` 这样的窄成员插在两个 `vec4` 之间，shader
> 侧的 `ArrayStride` 会变成 80 字节，而 Rust 侧这个 `repr(C)` 结构体仍是 64 字节
> ——索引 ≥ 1 的裁剪节点就会从错误的偏移处读出来。`common.glsl` 里的 `ClipNode`
> 与本结构体**逐字段同序**，两者必须同时改。

## 显存协议约定

| 规则 | 原因 |
| :--- | :--- |
| 图元实例按 64 B 对齐 | 恰好占满缓存行，避免跨线的伪共享 |
| 颜色只存 Token ID | 主题切换只需改 4 KiB 表 |
| 变换走独立通道 | 动画不触发排版，见[动画引擎](../02-ui/animation.md) |
| 裁剪框在 CPU 折叠 | GPU 端只做两次轻量 SDF 求交 |
| 圆角/阴影半径用 f16 | 保留足够精度，同时省下字节 |

### 半精度打包的端序约定

图元实例里的 `border_width`（偏移 32）与 `shadow_blur`（偏移 34）是两个相邻
的 `u16`，着色器把它们当作一个 `uint` 读出。小端下 **低 16 位是边框宽度、
高 16 位是阴影半径**。

GLSL 的 `unpackHalf2x16` 遵循同样的约定：**`.x` 取最低 16 位，`.y` 取最高
16 位**（GLSL 4.x 参考手册与 SPIR-V `GLSL.std.450` 的 `UnpackHalf2x16`
条目）。因此 `common.glsl` 里的两个辅助函数是直通的：

```glsl
float unpack_border(uint v) { return unpackHalf2x16(v).x; }  // 低位
float unpack_shadow(uint v) { return unpackHalf2x16(v).y; }  // 高位
```

主题颜色表用的是同一套约定：每个 RGBA16F 条目占两个 `uvec2`，四个通道依次
落在 `e.x` 的低/高位与 `e.y` 的低/高位上，`theme_color()` 直接按
`.x/.y` 取用。

---

**相关文档**：[显存分层](memory-hierarchy.md) · [双阶裁剪](clipping.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

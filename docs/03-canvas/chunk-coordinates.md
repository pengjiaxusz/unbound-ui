# 分块锚定相对坐标体系

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/src/data.rs`（`FramePushConstants`）、`crates/unbound-gpu/shaders/uber.vert`、`crates/unbound-vector/src/lib.rs`（`to_chunk_local`） |
> | **关键类型** | `Camera`、`FramePushConstants::camera_chunk` / `camera_local` |

为彻底解决传统方案中"为了防止大世界坐标抖动而每帧在 CPU 重算所有可见图元
相对坐标并重传 GPU"导致的带宽崩溃问题，引擎采用**分块锚定相对坐标体系**。

## 逻辑分块与静态驻留

将无限画布世界空间按 $4096 \times 4096$ 逻辑像素划分为规则网格区块（Chunk）。

GPU 持久驻留池中的每个画布实体**静态存储**：

| 字段 | 大小 | 含义 |
| :--- | ---: | :--- |
| `chunk_id: [i32; 2]` | $8\text{B}$ | 世界区块 ID |
| `local_pos: [f32; 2]` | $8\text{B}$ | 区块内局部坐标，值域恒定限制在 $[0.0, 4096.0)$ |

值域限制保证浮点尾数精度恒定高于 $0.00024\text{px}$——即使世界坐标高达
$10^{12}$，图元自身存储的坐标依然精确。

## GPU 原位相对坐标解算

摄像机同样维护 `camera_chunk: [i32; 2]` 与 `camera_local: [f32; 2]`，每帧仅通过
$16\text{B}$ Push Constants 传入 GPU。

在 Vertex Shader 与 Culling Compute Shader 中原位执行整数区块差值运算：

$$\Delta_{\text{chunk}} = \text{vec2}(\text{prim.chunk\_id} - \text{camera.chunk\_id})$$

$$P_{\text{camera\_relative}} = \Delta_{\text{chunk}} \times 4096.0 + (\text{prim.local\_pos} - \text{camera.local\_pos})$$

## 核心收益

画布任意平移或缩放时，**GPU 持久驻留池实现真正的 $0\text{ Byte}$ 更新**。

同时在坐标绝对值高达 $10^{12}$ 的超大画布边缘，依然保持亚像素级绝对丝滑，
绝无顶点抖动。

| 操作 | 传输量 | 原因 |
| :--- | ---: | :--- |
| 摄像机平移 | 16 B | 只有 Push Constants 变化 |
| 摄像机缩放 | 16 B | 同上 |
| 主题切换 | 4 KiB | 颜色表变化 |
| 节点移动 | 64 B/节点 | 单节点 Scatter 修补 |

对照[显存分层](../01-graphics/memory-hierarchy.md)：前两项都是 CPU→GPU 的
Push Constants 写入，**不触及持久驻留池**，因此不存在 GPU 正在读旧地址的
问题。

## CPU 侧的对应工具

`to_chunk_local(world)` 把任意世界坐标转为 `(chunk_id, local_pos)`，是 CPU 侧
唯一需要知道的坐标转换。

---

**相关文档**：[显存分层](../01-graphics/memory-hierarchy.md) · [空间索引](spatial-index.md)

**返回**：[03-canvas](README.md) · [文档索引](../README.md)

# 异步资源流送管线

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-asset/src/{pipeline,decode,ring,transfer}.rs` |
> | **关键类型** | `AssetPipeline`、`PipelineConfig`、`StagingRing`、`StagingRef`、`TimelineGate`、`OwnershipTransfer` |

```
[磁盘 / 网络异步 IO] ──> [Rayon 后台解码线程池] ──> [CPU Staging Ring Buffer]
                    (PNG/JPEG/WebP/KTX2/SVG/Font)            │
                                                             ▼ (独立 DMA 硬件通道)
                                                [Vulkan Dedicated Transfer Queue]
                                                             │
                                                             ▼ (Timeline Semaphore 释放/获取所有权)
                                                [GPU 持久显存池 / Bindless Texture]
```

## 零主线程阻塞

图片解码、KTX2 超压缩纹理转码、字体文件解析与大体积 SVG 曲线提取全部在后台
线程池完成。

`AssetPipeline` 用 crossbeam channel 组织工作队列：

| 通道 | 方向 | 用途 |
| :--- | :--- | :--- |
| `jobs` | 主线程 → 工作线程 | 有界队列（`queue_capacity`），满时明确报错而非静默丢弃 |
| `done` | 工作线程 → 主线程 | 无界，收集解码结果 |

主线程调用 `poll` 取回本帧完成的资源，不阻塞。

## CPU 暂存环

`StagingRing` 是 CPU 侧的环形暂存区，槽位按帧轮转。

**槽位复用规则**：只有当整环绕回一圈后，最早的槽位才可再次写入。GPU 可能
仍在读旧槽位，提前复用会读到撕裂的数据。

`StagingRef` 是对暂存区的引用（槽位 + 偏移 + 长度）。读取通过
`with(reference, |bytes| ...)` 闭包完成，避免在持有锁的同时把字节借用出去。

## 跨队列所有权无缝移交

使用独立的 Vulkan 专用传输队列（Dedicated Transfer Queue）通过 PCIe DMA 引擎
异步上传纹素与顶点数据。

上传完毕后通过 Timeline Semaphore 与队列族所有权转移屏障（Queue Family
Ownership Transfer）通知主渲染队列：

```rust
pub fn ownership_barrier(transfer: &OwnershipTransfer) -> TransferBarrier
```

`TimelineGate` 封装时间线信号量值的管理：主渲染队列在资源就绪前自动渲染低
分辨率占位符或骨架屏，保证 $120\text{FPS}$ 渲染绝对零卡顿（Zero Stutter）。

## 解码器

| 类型 | 能力 |
| :--- | :--- |
| `PngDecoder` | PNG 解码 |
| `RawDecoder` | 已解码的原始像素（测试与预烘焙路径） |

`KindDecoder` trait 让解码器可插拔；`AssetKind` 枚举描述资源类别。

---

**相关文档**：[显存分层](../01-graphics/memory-hierarchy.md) · [调试工具链](debugging.md)

**返回**：[06-tooling](README.md) · [文档索引](../README.md)

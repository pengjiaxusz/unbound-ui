# 核心 API 与现代特性开关

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/src/device.rs` |
> | **关键类型** | `Gpu`、`GpuConfig`、`GpuFeatures`、`MemoryKind`、`Timeline` |
> | **测试** | `crates/unbound-gpu/src/device.rs` 内的特性校验用例 |

## 图形 API

Vulkan 1.3+，基于 Rust `ash` 底层绑定，显存由 `gpu-allocator` / 自研 TLSF 管理。

## 特性开关

`GpuFeatures` 在设备创建时逐项校验，缺一项就拒绝创建设备而不是静默降级：

| 特性 | 用途 | 缺失时 |
| :--- | :--- | :--- |
| `buffer_device_address` | BDA 物理地址寻址 | 拒绝创建设备 |
| `descriptor_indexing` | Bindless 描述符 | 拒绝创设备备 |
| `dynamic_rendering` | 免 `VkRenderPass` 样板 | 拒绝创建设备 |
| `synchronization2` | 队列族正确屏障 | 拒绝创建设备 |
| `timeline_semaphore` | 多队列无锁同步 | 拒绝创建设备 |
| `multi_draw_indirect` | MDI 批量绘制 | 拒绝创建设备 |

其余可选能力（如 `host_query_reset`）缺失时降级而非失败：查询池重置改为
命令缓冲区内重置，时间戳精度不受影响。

### 显存指针寻址

强制启用 BDA，通过 $64\text{B}$ 全局 Push Constants 直接向着色器传递物理显存
地址，彻底解耦 CPU/GPU 内存结构传递。图元实例、变换、裁剪节点、颜色表、
载荷五类数据各自是一个连续数组，着色器用 `uint64` 基址 + 索引寻址。

### 描述符机制

全局统一 Descriptor Set（基于 `VK_EXT_descriptor_indexing` 的 Bindless 架构），
废弃传统描述符集频繁绑定与切换。纹理图集以 `Texture2DArray` 形式由索引层
选择。

### 动态渲染

强制开启 `dynamicRendering`，废弃旧式 `VkRenderPass` 与 `VkFramebuffer` 样板
代码。管线直接声明附件格式与加载/存储操作。

### 多队列与同步

`synchronization2` + `timelineSemaphore` 支撑 CPU、主渲染队列、专用传输队列
与异步计算队列的精细多队列无锁同步。资源跨队列移交走
[所有权转移屏障](../06-tooling/asset-streaming.md)。

### 多批次合并

`multiDrawIndirect` (MDI) 与 `drawIndirectCount` 使 GPU Compute Shader 能动态
裁剪、紧凑化并组装绘制命令。10 万图元最终塌缩为 1 次绘制调用，见
[Uber-Shader](uber-shader.md)。

## 灾难恢复

`is_device_lost` 捕获 `VK_ERROR_DEVICE_LOST`，`recover_with` 执行原子化重建：
销毁并重建 `VkDevice`、显存池与管线，将纯内存领域状态树原地重新灌注
（Re-inflate）至新持久池。

---

**相关文档**：[显存分层](memory-hierarchy.md) · [BDA 数据布局](gpu-data-layout.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

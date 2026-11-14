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
| `descriptor_indexing` | Bindless 描述符 | 拒绝创建设备 |
| `dynamic_rendering` | 免 `VkRenderPass` 样板 | 拒绝创建设备 |
| `synchronization2` | 队列族正确屏障 | 拒绝创建设备 |
| `timeline_semaphore` | 多队列无锁同步 | 拒绝创建设备 |
| `multi_draw_indirect` | MDI 批量绘制 | 拒绝创建设备 |
| `draw_indirect_first_instance` | MDI 分组的非零 `firstInstance` | 拒绝创建设备 |
| `draw_indirect_count` | GPU 自定的绘制次数 | 拒绝创建设备 |
| `shader_demote_to_helper_invocation` | `discard` 在 `--target-env=vulkan1.3` 下编译成 `OpDemoteToHelperInvocation` | 拒绝创建设备 |

1.3 设备从 `VkPhysicalDeviceVulkan13Features` 查询并启用最后一项；1.2 设备
改走 `VK_EXT_shader_demote_to_helper_invocation` 及其独立特性结构。

其余可选能力（如 `host_query_reset`）缺失时降级而非失败：查询池重置改为
命令缓冲区内重置，时间戳精度不受影响。

### 实例扩展由窗口句柄与设备请求共同决定

实例必须先于设备创建，而设备扩展的依赖可能在**实例**侧，因此 `Gpu::new`
不能只透传调用方给的列表：

- 平台相关的 `VK_KHR_surface` + `VK_KHR_win32_surface`（或 X11/Wayland
  对应项）来自**窗口的 display handle**，由
  `surface_extensions::required_instance_extensions()` 纯函数派生；硬编码
  会让整个宿主变成 Windows 专用。
- `resolve_instance_extension_dependencies()` 再从设备请求反推依赖链：
  `VK_KHR_swapchain → VK_KHR_surface`；
  `VK_EXT_swapchain_maintenance1 → VK_EXT_surface_maintenance1 →
  VK_KHR_get_surface_capabilities2 → VK_KHR_surface`。这样即使调用方漏填
  实例扩展，`vkCreateDevice` 也不会违反
  `VUID-vkCreateDevice-ppEnabledExtensionNames-01387`。
- `Surface::new_win32` 在创建 ash 加载器**之前**核对启用列表：ash 的函数
  指针加载失败会 panic，且该 panic 位于不可展开的 `extern "system"` 表里，
  会直接 abort 整个进程而不是返回一个可诊断的错误。

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

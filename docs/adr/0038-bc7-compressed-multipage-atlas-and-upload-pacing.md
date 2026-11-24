# ADR 0038：Native BC7 硬件块压缩、Bindless 动态多页图集与帧级上传节拍契约

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0019（AssetHub 动态纹理上传与 LRU 驱逐），协同 ADR 0036（Bindless 外部槽位对齐）

## 背景

`unbound-gpu` 现有内部图集仅支持未压缩的 `R8_UNORM` 与 `R8G8B8A8_UNORM`（固定 `2048×2048`、静态 48 层上限），且存在四类制约高密度小图与海量缩略图场景的结构性瓶颈：

1. **未压缩显存与带宽开销过高**：`RGBA8` 每像素占 4 字节，在高密度网格场景下极易触达显存预算上限并加剧 PCIe 上传带宽压力；设备层尚未探测或启用 `texture_compression_bc` 与 `VK_FORMAT_BC7_UNORM_BLOCK`。
2. **纯 Bindless 独立纹理不适配数千张小图**：虽然 Bindless 消除了传统 DrawCall 合批限制，但对 `<= 512px` 小图若采用一图一 `VkImage`，会撞上驱动最小对齐浪费（如 16 KiB 的 128×128 BC7 在 64 KiB 对齐下浪费 4× 显存）、高速滚动期高频 `vkCreateImage` / `vkUpdateDescriptorSets` / 逐图屏障的 CPU 尖刺，以及 Bindless 槽位膨胀问题。
3. **既有图集缺少回收、驱逐与异步上传**：`unbound-app` 现有 `ShelfAtlas` 满 48 层即直接分配失败，无空闲区间回收与 LRU 驱逐；`AtlasSet::upload_batch` 仍走图形队列同步阻塞等待（`end_single_time`），且缺少每帧上传配额闸门，在高速滚动或瞬时切档时易引发单帧卡顿。
4. **缺少无 BC7 硬件与无头环境的透明降级契约**：在不支持 BC7 块压缩的驱动或无头 CI 环境下，需要明确的设备能力探测与安全降级行为。

## 决策

### 1. 预压缩 `VK_FORMAT_BC7_UNORM_BLOCK` 直传与非阻塞异步 Staging 上传

- **格式与色彩空间闭环**：首版统一采用 **`VK_FORMAT_BC7_UNORM_BLOCK`**（4×4 块对齐、16 字节/块，存储 sRGB 编码块），直接复用主图元着色器 `uber.frag` 现有的 `srgb_to_linear(t.rgb) * tint` 采样路径，零着色器分支改动且避免 `BC7_SRGB_BLOCK` 引发的二次压暗。
- **预压缩块直传与零跨帧借用**：引擎接收已按 4×4 块对齐的预编码 BC7 原始块，写入持久化异步 Staging 环后即可立即释放调用方切片（无需跨帧持有外部内存锚点）；上传路径采用非阻塞命令提交与批量 `vkCmdCopyBufferToImage`（禁止沿用同步 `end_single_time`）。GPU Compute 实时 BC7 编码留作后续独立扩展。
- **未分配区域安全清零**：每个新分配的 BC7 图集页在投入采样前以 **BC7 Mode 6 透明黑块**（首字节 `0x40` + 后续 15 字节 `0x00`）初始化，防止边缘双线性采样产生未初始化显存噪点。

### 2. 大小分流 + Bindless 固定 `2048×2048` 多页图集与分级货架即时回收

- **大小纹理分流**：大于图集阈值（默认 `> 512px`）的大纹理继续走独立 Bindless `VkImage`（`ExternalTextureHandle`），避免占用图集页与碎片；`<= 512px` 高密度小图统一进入 BC7 动态多页图集。
- **固定 `2048×2048` 多页动态槽位**：每页固定 `2048×2048`（BC7 仅占 4 MiB/页），按需向 Bindless 表（`48..304` 区间）动态申请与归还槽位（释放严格走 `frames_in_flight` 延迟销毁）。既保留单 DrawCall 跨页合批与批量 `vkCmdCopyBufferToImage` 合并屏障优势，又彻底免去传统单页翻倍扩容时的整页拷贝尖刺。
- **分级高度货架装箱 + 即时 `freeSpans` 合并 + 冷页回收**：页内按 4px 块对齐的高度档位（如 `64` / `128` / `256` / `512`）支持任意长宽比打包，避免固定方格对非 1:1 图像的 20–40% 空间浪费；条目释放时**即时回收并合并相邻空闲区间（`freeSpans`）**（禁止只打墓碑）；当整页清空或超出预算触发冷页淘汰时，直接回收该页 GPU 显存与 Bindless 槽位。

### 3. 帧级上传节拍闸门（Pacing）与 `AssetHub` 真实字节预算治理

- **双维度可调上传配额**：在 `unbound-asset` 的压缩资产池中提供可按帧动态覆写的上传节拍闸门（`max_uploads_per_frame` 与 `max_bytes_per_frame` 双重封顶），允许上层在高速滚动期严控削峰、静止首屏期放宽冲刷；超出本帧配额的待传项安全留在有界队列顺延至后续帧，并支持离屏取消或新版本覆写。
- **真实字节记账与 LRU 驱逐**：`AssetHub` 按 BC7 真实显存占用（1 B/px，而非 RGBA8 的 4 B/px）统一记入可配置 `vram_budget_bytes`，结合帧级 `touch` 与离屏 LRU 驱逐维持可预测显存上限，并在 `FrameReport` 暴露 `bc7_active_pages`、`bc7_vram_bytes`、`bc7_uploads_this_frame`、`bc7_pending_uploads`、`bc7_eviction_count` 供无头测试与性能门禁断言。
- **设备丢失恢复**：窗口运行器触发 `recover_gpu` 后，压缩资产池与图集页自动失效重建，待重新提交时无缝恢复。

### 4. 设备能力探测与透明安全降级

- **显式能力探测**：`unbound-gpu` 设备初始化探测 `VkPhysicalDeviceFeatures::texture_compression_bc` 与 `VK_FORMAT_BC7_UNORM_BLOCK` 格式特性，公开 `CompressedTextureCapabilities { supports_bc7, active_backend }`。
- **双轨安全降级**：当硬件不支持 BC7 或运行于无头环境时，上层可直接依据 `supports_bc7 == false` 改走 `RGBA8` 上传路径；同时压缩资产池在收到 BC7 载荷时支持安全降级（可选纯 Rust 软解回退至 `RGBA8` 或无头确定性模拟），绝不崩溃或黑屏，确保无头 CI 零阻断覆盖装箱、区间合并、节拍与 LRU 驱逐全链路。

## 备选方案

- **彻底不装箱、全量走纯 Bindless 独立 `VkImage`**：虽可省去装箱器，但面对数千张 `64–256px` 小图会承受 4×–16× 的驱动最小对齐显存浪费、高速滚动期高频 `vkCreateImage`/描述符更新/逐图屏障尖刺，以及远超当前 304 槽的 Bindless 描述表扩容压力。
- **多页内改用固定正方形 Cell/Tile 位图池**：分配器最简，但对 16:9、3:2 或全景图会带来 20–40% 的页内透明像素浪费，且单行货架 `freeSpans` 合并逻辑本身纯 CPU、体量小且极易单测。
- **首版在 `unbound-gpu` 内置 GPU Compute BC7 实时编码器**：多模式 BC7 编码着色器开发与质量调优成本极高，会阻塞多页图集与上传节拍底座落地；拆分到后续扩展更合理。

## 影响

- **正向收益**：高密度小图显存占用降至未压缩 `RGBA8` 的约 1/4，单帧批量上传屏障与驱动调用降至常数级，彻底消除旧 `ShelfAtlas` 48 层溢出与碎片泄漏，并为高刷稳态渲染提供引擎级上传节拍与显存预算护栏。
- **工程约束与代价**：
  - 需在 `unbound-gpu` 启用可选 `texture_compression_bc` 设备特性并同步扩展 `unbound-asset` 与 `unbound-app` 的压缩资产队列与 `recover_gpu` 失效路径；
  - 所有 BC7 子区域坐标与宽高均须严格满足 4×4 像素对齐约束（由引擎在边界校验并拒绝非法输入）；
  - 真实硬件 BC7 采样与异步批量上传的像素级验证依赖具备 BC7 支持的 Vulkan 真机环境，跨平台 CI 覆盖无头装箱、区间合并、节拍闸门、LRU 驱逐与降级回归。

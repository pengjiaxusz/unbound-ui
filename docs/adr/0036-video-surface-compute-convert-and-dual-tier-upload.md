# ADR 0036：动态视频帧 Compute 预转换、双路径上传与视频表面治理契约

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0019（AssetHub RGBA8 动态纹理上传与 LRU 驱逐）

## 背景

ADR 0019 建立了基于 `ExternalTextureHandle` 与 `ExternalTextureOp` 的单平面 RGBA8 动态外部纹理通路，满足了静态大图与低频位图更新的需求。但面对高频动态视频帧（30–60fps 单路 4K/8K 播放，或单视口多路并发缩略图播放）时，现有通路存在四类结构性缺口：

1. **像素格式与色彩空间不匹配**：硬件与软件视频帧的原生输出为多平面 YUV（`NV12`、`P010`、`I420`），且伴随 `BT.601` / `BT.709` / `BT.2020` 色彩矩阵、`Limited` / `Full` 量化范围及 `PQ` / `HLG` 传递函数；在 CPU 侧先转 RGBA8 再上传会造成极高内存带宽与 CPU 占用。
2. **同步阻塞上传导致帧抖动**：现有 `ExternalTextureOp` 上传路径每次临时分配 `HostVisible` 缓冲并在图形队列执行单次同步等待（`end_single_time`），在高频逐帧提交下会造成显著管线停顿与内存抖动。
3. **缺少跨 API 零拷贝互操作与平滑降级契约**：`unbound-gpu` 尚未在设备层协商 `VK_KHR_external_memory_win32` 等外部显存扩展，无法零拷贝导入来自其他图形 API 的共享帧表面；同时在非 Windows 平台、旧驱动、软解或无头测试环境下需要无缝降级至跨平台异步上传。
4. **缺少视频表面语义与多路并发治理**：复用原始 `ExternalTextureHandle` 无法统一表达首帧未就绪占位、分辨率/格式切换期间的零闪烁保帧、最新帧覆盖防积压（Latest-Wins）、多路并发显存上限与离屏 LRU 驱逐，也缺乏面向无头测试的确定性观测指标。

## 决策

### 1. Compute 着色器按新帧预转换（`video_convert.comp` → RGBA 外部纹理槽）

- **单次新帧预转换，主图元着色器零分支膨胀**：新增独立计算着色器 `video_convert.comp`，仅在某路视频表面收到新帧时执行一次 Compute Dispatch，将多平面 YUV 转换为目标 `R8G8B8A8_UNORM` 外部纹理槽（分配自既有 Bindless 外部纹理槽区间）；渲染阶段直接复用 `PRIM_IMAGE` 图元管线。
- **零侵入视觉能力复用与高刷节能**：由于不修改 `uber.frag` 的图像采样分支，圆角裁剪、父级圆角裁剪、透明度（`opacity`）、描边、裁剪子矩形（`sample_uv`）与多路缩略图并发渲染全部自动兼容；在高刷新率界面（120–240Hz）播放 30–60fps 视频时无需每渲染帧重复转色。
- **色彩空间与 EOTF 闭环**：鉴于 `uber.frag` 对 `PRIM_IMAGE` 统一执行 `srgb_to_linear(t.rgb) * tint`，`video_convert.comp` 统一输出 **sRGB 编码值** 至目标 `R8G8B8A8_UNORM` 存储图像，避免二次 sRGB 解码。同时对齐 `uber.frag` 的 `u_atlas_rgba` Bindless 数组声明与管线层 `MAX_RGBA_BINDLESS_SLOTS`（304），确保多路视频表面在扩展槽位区间内安全采样。
- **首版格式与色彩全覆盖**：
  - 像素格式：`NV12`（8-bit 4:2:0 半平面）、`P010`（10-bit 高位对齐 4:2:0 半平面）、`I420`（8-bit 4:2:0 三平面）。
  - 色彩矩阵与范围：`BT.601`、`BT.709`、`BT.2020` × `Limited` / `Full`。
  - HDR 基础色调映射：`P010` 配合 `PQ`（ST 2084）或 `HLG` 传递函数时，在 `video_convert.comp` 内执行「EOTF 线性化 → BT.2020 至 BT.709 色域映射 → 基础 SDR 色调压缩 → sRGB 编码」，无需变更后续引擎 ABI。

### 2. 两级阶梯上传与统一能力降级（Tier 1 + Tier 2 同契约）

- **Tier 1（Windows 跨 API 零拷贝直通）**：`unbound-gpu` 在设备初始化阶段可选探测并协商 `VK_KHR_external_memory_win32`（配合 `VK_KHR_external_memory` 与必要专用分配能力），基于既有的外部内存描述符契约将外部 D3D11 DXGI NT 共享句柄直接导入为只读源 `VkImage`（带帧级同步与生命周期保护），供 `video_convert.comp` 直接采样，实现零 CPU 拷贝与零 PCIe 折返。
- **Tier 2（跨平台异步 Staging 回退）**：构建双缓冲/环形 `HostVisible` 异步 Staging 缓冲，通过非阻塞命令提交与时间线信号量上传多平面像素（禁止使用阻塞式 `end_single_time`），在 Linux、旧驱动、软解帧或无头测试环境下自动透明降级。
- **上层零感知与可观测降级**：两级通路共享同一 `VideoSurfaceHandle` 与 `video_convert.comp` 转换管线；设备能力探测结果（当前活跃上传阶梯）对诊断与测试暴露，单次外部句柄导入失败时可按表面平滑回退至 Tier 2 或返回结构化错误，不导致渲染器崩溃或黑屏。

### 3. 独立 `VideoSurfaceHandle`、`AssetHub` 流式治理与 `ViewBuilder::video_surface`

- **跨 crate 职责分层**：
  - `unbound-asset`（保持零 GPU 依赖）：定义纯 CPU 侧视频帧格式、色彩空间元数据、多平面帧载荷与有界最新帧（Latest-Wins）流式帧队列抽象。
  - `unbound-gpu`：负责外部内存扩展探测、Tier 1 零拷贝导入、Tier 2 异步 Staging 上传、`video_convert.comp` 管线与 `frames_in_flight` 延迟销毁。
  - `unbound-app`（`AssetHub` 与 `ViewBuilder`）：管理 `VideoSurfaceHandle` 生命周期、首帧占位、分辨率/像素格式切换期间的零闪烁保帧、并发视频流显存上限与离屏 LRU 驱逐，并暴露 `ViewBuilder::video_surface(key, desc)` 声明式视图原语（支持尺寸、`ObjectFit`、圆角、裁剪子矩形、边框、着色、透明度与占位模式）。
- **零布局重算与按需唤醒**：同一表面在尺寸未变时提交新帧仅更新目标外部纹理槽内容，不触发场景布局重算；所有活跃视频表面均无新帧提交时引擎自动回到空闲休眠态，不空转渲染。
- **无头验收接缝**：在帧诊断与资产中心暴露 `video_frame_upload_count`、`active_video_surfaces`、`video_vram_bytes` 与 `video_eviction_count` 等公开观测指标，支持无真实 GPU 与解码器环境下的合成帧端到端确定性测试。

## 备选方案

- **在 `uber.frag` 中双槽位直采 Y/UV 平面**：虽可省去一次约 0.1ms 的目标纹理写入，但会增加主图元着色器的分支复杂度与寄存器压力，存在影响全套控件渲染性能与像素回归的风险；且每路视频占用 2–3 个 Bindless 槽位，高刷 UI 播放低帧率视频时还会每帧重复执行色彩矩阵运算与色调映射。
- **首版仅做 `NV12 + BT.709` 或仅做 Tier 2 异步 Staging**：后续补充 `P010` / `I420`、HDR 色调映射或 Tier 1 零拷贝导入时将不可避免地修改公开帧描述符 ABI 与资源生命周期状态机，且无法在首版支撑高分辨率低 CPU 播放目标。
- **仅复用 `ExternalTextureHandle` 不新增 `VideoSurfaceHandle`**：会把双缓冲帧合并、切帧零闪烁保帧、并发显存预算与离屏 LRU 回收全部推给上层调用方重复实现，且无头测试缺乏清晰的流式帧观测接缝。

## 影响

- **正向收益**：渲染器获得统一、跨平台、支持 8/10-bit 多平面 YUV 与可选跨 API 零拷贝的视频帧渲染底座；`uber.frag` 零分支膨胀，现有视觉裁剪/圆角/透明度能力全部免费继承；多路并发播放具备可预测的显存上限与无头回归门禁。
- **工程约束与代价**：
  - 每路活跃视频表面需额外占用一张同分辨率 `R8G8B8A8_UNORM` 目标转换纹理（通过并发显存预算上限与离屏 LRU 驱逐兜底控制）；
  - 新增 `video_convert.comp` 需纳入着色器编译脚本与构建清单，变更后须同步刷新预编译 `.spv`；
  - Windows Tier 1 零拷贝路径的真机互操作验证依赖具备真实 Vulkan/D3D11 驱动的 Windows 环境，CI 侧以 Tier 2 与无头合成帧回归作为跨平台门禁。

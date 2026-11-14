# 架构：gpu-renderer
状态：已落地
路径：`crates/unbound-gpu/`、`crates/unbound-gpu/shaders/`、`crates/unbound-math/`
入口：`crates/unbound-gpu/src/lib.rs::Renderer`、`crates/unbound-gpu/src/device.rs::Gpu`、`GpuError::diagnostic_report`
守卫：`cargo test -p unbound-gpu --test vulkan_probe` / `just compile-shaders`

## 范围
- [负责] Vulkan 1.3（含 Vulkan 1.2 向后移植扩展路径）设备与内存池、实例与设备扩展跨层兼容协商、GPU 初始化结构化诊断报告（`GpuInitDiagnostics`）、GPU Compute 视口剔除与 64 位基数排序、MDI 间接绘制、双通道深度、双阶裁剪、erf 解析阴影
- [不负责] 窗口表面/交换链宿主与帧调度策略 -> 委托给 `crates/unbound-platform/`；UI 树与组件逻辑 -> 委托给 `crates/unbound-widgets/`

## 拓扑与流程
- 主链路：`SceneUpload` -> Compute 视口粗筛 + 64 位基数排序（`SortedEntry`） -> Opaque Inner Kernel 预剔除（Early-Z） -> 统一 Uber-Shader Pass（`DrawIndirectCommand` MDI）
- 内存分层：持久池（静态图元/字体，0B/帧上传）+ 瞬态环（每帧动态补丁/PushConstants）+ 离屏池（毛玻璃/后处理）+ Epoch 延迟回收
- 裁剪与阴影：CPU 折叠嵌套圆角裁剪为 `GpuClipNode` -> VS 逆变换至局部坐标 -> FS 解析 SDF 无 `discard` 抗锯齿；阴影使用 `erf` 近似积分
- 扩展协商与 Win10 兼容过滤：`Gpu::new` 从设备扩展反推实例依赖链（`VK_KHR_swapchain` -> `VK_KHR_surface`；`VK_EXT_swapchain_maintenance1` -> `VK_EXT_surface_maintenance1` -> `VK_KHR_get_surface_capabilities2` -> `VK_KHR_surface`），并通过 `filter_instance_extensions` 对照 `vkEnumerateInstanceExtensionProperties` 过滤掉当前加载器不支持的可选实例扩展；随后通过 `resolve_device_extensions` 对照物理设备扩展列表与已启用实例扩展，确保仅在实例已启用 `VK_EXT_surface_maintenance1` 时才启用 `VK_EXT_swapchain_maintenance1`，且仅在设备同时支持 `VK_KHR_present_wait` 与 `VK_KHR_present_id`（且对应特性位均为 `VK_TRUE`）时才启用并挂载其 `pNext` 结构体 -> [位置] `crates/unbound-gpu/src/device.rs::{filter_instance_extensions, resolve_device_extensions}`
- 初始化错误诊断：`Gpu::new_inner` 逐阶段记录 `GpuInitDiagnostics`（加载器版本、请求/启用/跳过的实例与设备扩展、选中的 GPU 详情与淘汰原因），失败时可通过 `GpuError::diagnostic_report()` 输出完整的结构化排障报告 -> [位置] `crates/unbound-gpu/src/device.rs::GpuError::diagnostic_report`
- 必需特性：`shaderDemoteToHelperInvocation`（1.3 核心 / 1.2 `VK_EXT_shader_demote_to_helper_invocation`）；`glslc --target-env=vulkan1.3` 把 `discard` 编译为 `OpDemoteToHelperInvocation`，缺特性触发 `VUID-VkShaderModuleCreateInfo-pCode-08740`
- 图集容量：RGBA 图集钳制 48 层（48 内部槽 + 16 外部纹理 = 64 绑定槽），R8 保留 64 层 -> [位置] `crates/unbound-gpu/src/atlas.rs::AtlasSet::new`

## 取舍（禁止重构）
- [代价] 使用 Buffer Device Address (BDA) + Bindless 描述符数组，禁用传统每图元描述符集绑定 -> [原因] 实现单次 MDI 绘制 10 万图元与零描述符切换开销 -> [位置] `crates/unbound-gpu/src/data.rs::FramePushConstants`
- [代价] 圆角半径/线宽/阴影参数打包为 `f16`（`unbound-math::f16`）嵌入 `GpuPrimitiveInstance` -> [原因] 压缩实例步长（`PRIMITIVE_STRIDE`）以提升显存带宽利用率 -> [位置] `crates/unbound-math/src/f16.rs`

## 不变量与红线
- [禁止] `unbound-gpu` 依赖 `unbound-world`、`unbound-layout` 或 `unbound-widgets` -> [原因] 保持纯渲染内核与无头可测性 -> [守卫] `crates/unbound-gpu/Cargo.toml`
- [禁止] 未经 `vkEnumerateInstanceExtensionProperties` 校验就向 `vkCreateInstance` 传入可选实例扩展（如 `VK_EXT_surface_maintenance1`），或在未启用 `VK_KHR_present_id` / `VK_KHR_present_wait` 扩展时无条件将 `present_id_features` / `present_wait_features` 链入 `VkDeviceCreateInfo::pNext` -> [原因] 在 Windows 10 旧版加载器或旧驱动上会直接触发 `VK_ERROR_EXTENSION_NOT_PRESENT` -> [守卫] `crates/unbound-gpu/src/device.rs::tests::win10_older_driver_gracefully_skips_missing_optional_extensions`
- [禁止] 片元着色器中使用 `discard` 处理圆角/裁剪抗锯齿 -> [原因] 破坏硬件 Early-Z 与驱动并发优化 -> [守卫] `crates/unbound-gpu/shaders/`
- [必须] 无 Vulkan 1.3 设备的 CI 环境下 `vulkan_probe` 自动跳过而非失败 -> [守卫] `crates/unbound-gpu/tests/vulkan_probe.rs`
- [必须] 空 `transforms` 表补 `GpuTransform::IDENTITY` 后再上传 -> [原因] 剔除/顶点着色器无条件读 `transforms[0]`，未初始化显存会把全部图元移出视口 -> [守卫] `crates/unbound-gpu/src/frame.rs::a_scene_without_transforms_still_draws`
- [必须] `Surface::new_win32` 先校验实例扩展再调 ash -> [原因] ash 加载器 panic 位于不可展开的 `extern "system"`，会 abort 而非返回错误 -> [守卫] `crates/unbound-gpu/src/surface.rs::new_win32_reports_missing_instance_extensions`


## 阶段 1 扩展能力（WP1：外部显存导入、256 槽纹理与通道检视）

* **外部显存句柄导入（`unbound_gpu::inspector`）**：通过 `ExternalMemoryHandleKind`（支持 `OpaqueWin32`、`D3D11Texture`、`D3D12Resource`）与 `ExternalImageDesc` 校验并封装跨进程/跨 API 零拷贝共享纹理导入协议。
* **256 槽扩展 Bindless 分配器（`ExtendedExternalSlots`）**：采用 4×64 位无堆分配位图，在 `O(1)` 时间内分配与回收多达 256 个外部纹理槽位，满足 `dunting` 大规模资产缩略图与 `chahu-render-debugger` 多 RenderTarget 同屏检视需求。
* **2D 纹理通道检视器与 3D 视口轨道相机**：`TextureInspectorParams` 提供 `ChannelMask`（`Rgba`/`Rgb`/`Red`/`Green`/`Blue`/`Alpha`/`Luminance`）、曝光（EV）、Gamma、Mip 层级与值域归一化计算；`ViewportCamera3D` 与 `RenderGraph` 的 `PassKind::CustomViewport` / `PassKind::TextureInspect` 支撑内嵌 3D 网格预览与 RT 单通道分析。

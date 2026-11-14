# 架构：gpu-renderer
状态：已落地
路径：`crates/unbound-gpu/`、`crates/unbound-gpu/shaders/`、`crates/unbound-math/`
入口：`crates/unbound-gpu/src/lib.rs::Renderer`、`crates/unbound-gpu/src/device.rs::Gpu`
守卫：`cargo test -p unbound-gpu --test vulkan_probe` / `just compile-shaders`

## 范围
- [负责] Vulkan 1.3 设备与内存池、GPU Compute 视口剔除与 64 位基数排序、MDI 间接绘制、双通道深度、双阶裁剪、erf 解析阴影
- [不负责] 窗口表面/交换链宿主与帧调度策略 -> 委托给 `crates/unbound-platform/`；UI 树与组件逻辑 -> 委托给 `crates/unbound-widgets/`

## 拓扑与流程
- 主链路：`SceneUpload` -> Compute 视口粗筛 + 64 位基数排序（`SortedEntry`） -> Opaque Inner Kernel 预剔除（Early-Z） -> 统一 Uber-Shader Pass（`DrawIndirectCommand` MDI）
- 内存分层：持久池（静态图元/字体，0B/帧上传）+ 瞬态环（每帧动态补丁/PushConstants）+ 离屏池（毛玻璃/后处理）+ Epoch 延迟回收
- 裁剪与阴影：CPU 折叠嵌套圆角裁剪为 `GpuClipNode` -> VS 逆变换至局部坐标 -> FS 解析 SDF 无 `discard` 抗锯齿；阴影使用 `erf` 近似积分
- 扩展协商：`Gpu::new` 从设备扩展反推实例依赖链（`VK_KHR_swapchain` -> `VK_KHR_surface`；`VK_EXT_swapchain_maintenance1` -> `VK_EXT_surface_maintenance1` -> `VK_KHR_get_surface_capabilities2` -> `VK_KHR_surface`）；平台扩展由窗口显示句柄派生 -> [位置] `crates/unbound-gpu/src/device.rs::resolve_instance_extension_dependencies`
- 必需特性：`shaderDemoteToHelperInvocation`（1.3 核心 / 1.2 `VK_EXT_shader_demote_to_helper_invocation`）；`glslc --target-env=vulkan1.3` 把 `discard` 编译为 `OpDemoteToHelperInvocation`，缺特性触发 `VUID-VkShaderModuleCreateInfo-pCode-08740`
- 图集容量：RGBA 图集钳制 48 层（48 内部槽 + 16 外部纹理 = 64 绑定槽），R8 保留 64 层 -> [位置] `crates/unbound-gpu/src/atlas.rs::AtlasSet::new`

## 取舍（禁止重构）
- [代价] 使用 Buffer Device Address (BDA) + Bindless 描述符数组，禁用传统每图元描述符集绑定 -> [原因] 实现单次 MDI 绘制 10 万图元与零描述符切换开销 -> [位置] `crates/unbound-gpu/src/data.rs::FramePushConstants`
- [代价] 圆角半径/线宽/阴影参数打包为 `f16`（`unbound-math::f16`）嵌入 `GpuPrimitiveInstance` -> [原因] 压缩实例步长（`PRIMITIVE_STRIDE`）以提升显存带宽利用率 -> [位置] `crates/unbound-math/src/f16.rs`

## 不变量与红线
- [禁止] `unbound-gpu` 依赖 `unbound-world`、`unbound-layout` 或 `unbound-widgets` -> [原因] 保持纯渲染内核与无头可测性 -> [守卫] `crates/unbound-gpu/Cargo.toml`
- [禁止] 片元着色器中使用 `discard` 处理圆角/裁剪抗锯齿 -> [原因] 破坏硬件 Early-Z 与驱动并发优化 -> [守卫] `crates/unbound-gpu/shaders/`
- [必须] 无 Vulkan 1.3 设备的 CI 环境下 `vulkan_probe` 自动跳过而非失败 -> [守卫] `crates/unbound-gpu/tests/vulkan_probe.rs`
- [必须] 空 `transforms` 表补 `GpuTransform::IDENTITY` 后再上传 -> [原因] 剔除/顶点着色器无条件读 `transforms[0]`，未初始化显存会把全部图元移出视口 -> [守卫] `crates/unbound-gpu/src/frame.rs::a_scene_without_transforms_still_draws`
- [必须] `Surface::new_win32` 先校验实例扩展再调 ash -> [原因] ash 加载器 panic 位于不可展开的 `extern "system"`，会 abort 而非返回错误 -> [守卫] `crates/unbound-gpu/src/surface.rs::new_win32_reports_missing_instance_extensions`

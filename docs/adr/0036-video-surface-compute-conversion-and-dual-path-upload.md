# ADR-0036：动态视频表面 Compute 预转换与双路径上传契约

- **状态**：已采纳
- **影响 crate**：`unbound-gpu`、`unbound-asset`、`unbound-app`、`unbound-headless`
- **关联**：ADR-0019（`AssetHub` 外部纹理槽与 LRU 回收）

## 背景

`AssetHub` 当前仅支持单平面 RGBA8 外部纹理槽（ADR-0019）。面对高频动态视频帧（4K@60fps / 8K 高码率与多路并发缩略图视频播放），若在 CPU 侧把硬件解码产出的 NV12 / P010 / I420 转换为 RGBA8 再上传，会造成严重的 CPU 占用与 PCIe 带宽瓶颈；而若直接把多平面 YUV 采样与色彩矩阵分支塞进 `uber.frag`，则会增加所有常规 UI 图元的着色器寄存器压力、占用双倍 bindless 槽位，并引入 WinUI 3 像素对照回归风险。

## 决策

1. **Compute Shader 按新帧预转换（而非 `uber.frag` 直采）**：
   - 新增独立计算着色器 `video_convert.comp`，仅在视频表面收到**新帧**时执行一次 GPU YUV→RGBA 转换并写入该视频表面绑定的目标外部纹理槽；
   - 渲染主通路保持零改动，继续走既有外部纹理 `Image` 图元渲染，免费继承圆角、裁切、透明度、变换与多路并发合批能力，同时避免高刷 UI（120～240fps）对低帧率视频（30～60fps）重复做片元转色。
2. **输入格式与色彩空间首版全覆盖**：
   - 支持 **NV12**（8-bit 4:2:0）、**P010**（10-bit HDR）与 **I420**（三平面软件解码）三种主流像素格式；
   - 支持 **BT.601 / BT.709 / BT.2020** 色彩矩阵与 **Limited / Full Range**，P010 内置基础 PQ/HLG → SDR 色调映射。
3. **两级上传阶梯与自动降级**：
   - **Tier 1（Windows 零拷贝直通）**：在 `unbound-gpu` 可选启用 `VK_KHR_external_memory_win32`（探测支持后开启），支持导入 D3D11 DXGI NT 共享句柄为 `VkImage`，实现零 CPU 拷贝与零 PCIe 折返，并配合帧级同步保障读写安全；
   - **Tier 2（跨平台与降级通路）**：双缓冲异步 Staging 多平面上传；当设备不支持外部内存扩展、处于非 Windows 平台、使用软件解码或无头测试模式时自动无缝降级，上层 API 零感知。
4. **独立 `VideoSurfaceHandle` 与并发显存预算**：
   - 在 `AssetHub` 与 `ViewBuilder` 暴露语义化的 `VideoSurfaceHandle` 与 `ViewBuilder::video_surface`，封装首帧占位、切帧零闪烁保持、多路并发流显存上限与离屏 LRU 回收，并向无头运行器输出结构化帧上传与活跃流观测计数。

## 后果

- `uber.frag` 与全套 WinUI 3 Parity 像素基线保持 100% 零干扰。
- 多路并发视频播放与 4K/8K 零拷贝播放共享统一抽象，调用方无需编写平台分支代码。

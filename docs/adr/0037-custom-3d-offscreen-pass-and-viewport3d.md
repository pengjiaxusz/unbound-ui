# ADR 0037：自定义 3D 离屏 RenderPass、动态渲染缩放与 Viewport3D 容器契约

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0019（外部纹理 Bindless 槽位治理），协同 ADR 0036（动态外部表面渲染）

## 背景

`unbound-gpu` 已全面启用 Vulkan 1.3 `dynamic_rendering` 与 `synchronization2`，并在内部原型中预留了 `CustomViewportPass`、`ViewportCamera3D` 与 `ViewportRenderPassDesc` 的初步类型，但尚未形成可投入生产的 3D 离屏渲染与声明式视口闭环，主要存在四类结构性缺口：

1. **原语未接入主帧管线且附件托管缺失**：现有原型未接入 `Renderer` 单帧命令录制，缺少独立的 3D 离屏颜色附件、深度缓冲（`D32_SFLOAT`）、可选 MSAA Resolve 附件、`pipeline_barrier2` 布局转换，以及与 Bindless 外部纹理槽（`48..304`）和 `frames_in_flight` 延迟销毁的集成。
2. **缺少动态分辨率缩放与零闪烁重建**：高分屏复杂 3D 场景交互拖拽时需要在不改变布局矩形的前提下临时降采渲染并由 GPU 双线性放大，松手时瞬时恢复原生分辨率；同时在窗口缩放、跨屏 DPI 切换或缩放比变更时，若直接重建并释放旧附件，极易引发在飞引用悬空或黑屏闪帧。
3. **相机原型与输入桥接不完备**：既有 `ViewportCamera3D` 缺少平移（`pan`）方法且俯仰角与距离边界为硬编码常量，无法适配不同尺度 3D 场景；控件层缺少统一的 `Viewport3D` 容器来承接指针捕获、多键手势区分（轨道/平移/缩放）、DPI 归一化增量事件，以及与上层悬浮 HUD 控件的 Z 序命中优先级协调。
4. **缺少无头验收接缝与多实例并发支持**：无头测试框架仅快照常规布局几何，无法观测自定义 3D Pass 的活跃实例数、调度次数、目标重建次数与手势事件回环，难以在无 GPU CI 环境中防止假闭环回归。

## 决策

### 1. 受控 Dynamic Rendering 离屏 Pass 托管与原型收口

- **统一收口至 `unbound-gpu::custom_pass`**：将分散在旧原型模块中的 `CustomViewportPass`、`ViewportRenderPassDesc` 与 `ViewportCamera3D` 统一收口重构（保持必要向后兼容导出，杜绝两套并行 3D Pass API）。
- **引擎托管全部附件与同步边界**：在单帧命令缓冲区进入 UI 主 Pass `cmd_begin_rendering` 之前，由 `Renderer` 按注册顺序执行各活跃 3D 视口离屏 Pass：
  - 引擎全权创建并托管每个 `ViewportId(u64)` 对应的离屏颜色附件、独立深度缓冲（`D32_SFLOAT`，支持标准或反向 Z 清屏配置）及可选 MSAA 附件与 Resolve 目标；
  - 引擎负责附件布局转换、`cmd_begin_rendering` / `cmd_end_rendering` 起止，以及 Pass 结束后的 `cmd_pipeline_barrier2`（`COLOR_ATTACHMENT_OUTPUT` / `COLOR_ATTACHMENT_WRITE` → `FRAGMENT_SHADER` / `SHADER_SAMPLED_READ`，`COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL`），随后自动绑定至外部纹理 Bindless 槽位供 UI 主 Pass 通过 `PRIM_IMAGE` 合成；
  - 外部自定义回调仅在受控上下文内录制 3D 绘制命令，禁止越权修改主交换链状态或绕过同步屏障。
- **软件/外部帧直推降级兼容**：同一 `Viewport3D` 容器支持在无自定义 Vulkan 回调或无 GPU/软件渲染环境下无缝回退到外部 RGBA8 帧直推模式，上层布局与手势交互契约保持一致。

### 2. 可配置 MSAA、独立 `render_scale` 与 sRGB 透明底合成

- **MSAA 与动态渲染缩放**：`ViewportRenderPassDesc` 支持配置 `sample_count`（`1x` / `4x`，4x 自动托管 Resolve 到单采样目标）以及独立于布局物理尺寸的 **`render_scale`（`0.25..=1.0`）**。实际离屏目标物理像素尺寸为 `clamp(round(layout_rect * effective_scale * render_scale))`；当 `render_scale < 1.0` 时由 UI 主 Pass 通过双线性采样平滑放大至目标布局矩形，切换 `render_scale` 不触发布局重算。
- **色彩空间与透明底闭环**：离屏目标支持自定义 `clear_color`（含 `alpha = 0.0` 透明底）与标准 sRGB 编码输出，与主图元着色器 `uber.frag` 的 `srgb_to_linear` 采样路径精确闭环；3D 视口作为常规图层嵌入 UI 树，自动继承圆角、父级裁剪、透明度与上层 HUD 叠加。

### 3. `Viewport3D` 容器、通用手势桥接与可配置轨道相机

- **手势桥接与多键区分**：在 `unbound-widgets` 提供 `Viewport3D` 控件，并在 `unbound-app` 暴露 `ViewBuilder::viewport_3d(key, desc)`。容器内置命中测试、指针捕获与多按键手势识别（默认：左键拖拽产出 `OrbitDelta`、右键或 `Shift + 左键` 拖拽产出 `PanDelta`、滚轮产出 `ZoomDelta`，均按 DPI 归一化），并补齐拖拽过程中的按键状态与帧间位移跟踪。
- **可配置相机边界与业务可覆写**：重构 `ViewportCamera3D`，补齐观察平面 `pan()` 能力，并将俯仰角范围与缩放距离范围改为可配置边界（不再硬编码狭窄角度或固定绝对距离），同时允许调用方仅消费原始增量事件由自定义相机求解。
- **上层 HUD 命中优先**：`Viewport3D` 严格遵循常规节点 Z 序与命中测试规则；叠放在视口区域上方的工具栏、安全区标题条或悬浮 HUD 子节点优先消费指针事件，不被视口背景抢占。

### 4. 多实例并发、零闪烁延迟回收、无头审计与按需单次回读

- **多实例与零闪烁重建**：按 `ViewportId(u64)` 支持单窗与多窗并发多个 3D 视口（含设备丢失恢复 `recover_gpu` 后的自动重建）。当逻辑尺寸、DPI 缩放或 `render_scale` 变化时，旧附件进入 `frames_in_flight` 延迟销毁队列，在新目标完成首次渲染提交前继续保留上一帧有效 Bindless 槽位，杜绝缩放黑屏与闪帧。
- **按需单次离屏回读**：在 `Renderer` 上提供可选的单次离屏像素回读接口（`readback_viewport_rgba8(viewport_id)`），在渲染线程受控完成单次拷贝至主机内存；保持 Vulkan 上下文单线程安全，不将后台多线程渲染复杂度强绑定进交互视口管线。
- **无头验收接缝**：在 `FrameReport`（及无头诊断快照）中暴露 `active_viewports_3d`、`viewport_pass_dispatches`、`viewport_resize_count` 与手势增量事件统计，确保无 GPU CI 环境可确定性断言调度次数、尺寸同步与交互回环。

## 备选方案

- **在 `unbound-gpu` 中直接内置完整 3D 场景图与固定 PBR 管线**：会把网格解析、材质系统、光照模型与动画系统耦合进通用 UI 引擎，既膨胀核心体量又限制上层定制着色器/线框/检视叠加的能力。
- **首版固定 1x MSAA 且不支持 `render_scale`**：虽可减少 Resolve 附件管理，但高分屏锯齿明显且重负载拖拽期缺乏动态降采抓手，后续补加将破坏 Pass 描述符与尺寸同步契约。
- **仅提供裸外部纹理槽，由业务自行管理指针捕获与轨道数学**：迫使每个 3D 视口调用方重复编写按键拖拽状态机、DPI 物理像素换算与 HUD 事件穿透处理，易出现交互不一致与闪烁回归。

## 影响

- **正向收益**：引擎获得通用、多实例、支持 MSAA 与动态分辨率缩放的受控 3D 离屏渲染底座，与现有 `PRIM_IMAGE` 裁剪/圆角/HUD 叠加无缝融合，且具备完整的无头测试审计接缝。
- **工程约束与代价**：
  - 每个活跃 `Viewport3D` 需独立占用一套离屏 Color + Depth（及 4x MSAA 时的多重采样缓冲）显存，需通过生命周期解绑与延迟回收严格控制峰值；
  - 需在 `unbound-app` 的 `FrameReport` 与窗口运行器中补齐逻辑矩形到物理像素尺寸的逐帧同步以及 `recover_gpu` 重建接线；
  - 真实 Vulkan 离屏 3D 绘制、MSAA Resolve 与单次回读的像素级验收依赖具备真实 GPU 驱动的验证环境，无头 CI 负责容器布局、DPI 尺寸推导、手势事件桥接、零闪烁状态机与调度计数门禁。

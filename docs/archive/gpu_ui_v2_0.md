# 下一代高性能 GPU 驱动 UI 与无限画布引擎架构蓝图 (V2.0 生产级定稿)

---

# 1. 架构定位与核心设计准则

- **性能绝对优先 (Performance-First)**:
  - 核心路径零每帧内存分配（Zero Frame Allocation）。
  - 全数据连续布局且严格对齐 CPU/GPU 缓存行（$64\text{B}$ Cache-Line Friendly）。
  - 全链路 GPU 驱动渲染（GPU-Driven Rendering）：视口粗筛、排序键生成、基数排序与间接绘制命令组装均在 GPU 端完成。
- **面向数据设计 (DOD)**:
  - 彻底废弃面向对象的指针树与引用计数结构，全面采用扁平结构体数组（SoA）、连续物理缓冲与代际索引（`SlotMap<NodeId, T>` 与 `SecondaryMap`）。
  - 空间层次与动态集合内部全面采用固定数组侵入式链表（Intrusive Linked List），消灭嵌套 `Vec<Vec<T>>` 带来的二级堆分配与缓存未命中。
- **无锁响应式调度与超低延迟呈现**:
  - 主线程仅处理轻量 UI 意图、输入路由与瞬态更新；排版求解、字形轮廓提取、资源解码与后台运算均通过任务系统（Rayon）完全并行化。
  - 引入交换链完成等待信号量与独立呈现通道，将输入到光子延迟（Input-to-Photon Latency）严格控制在 $1$ 帧以内（$120\text{Hz}$ 下小于 $8.3\text{ms}$）。
- **Agent 机器与人类双模友好**:
  - 统一以 `AccessKit` 语义树作为人机交互与 AI Agent 自省的一级输出底座。
  - 提供小于 $1\text{ms}$ 的纯内存无头闭环验证（Headless Verification）、内置 $5$ 项几何体检断言与小于 $600\text{ms}$ 的 C-ABI 热重载开发闭环。

---

# 2. 图形技术栈与 GPU 驱动渲染底座

## 2.1 核心 API 与现代特性开关

- **图形 API**: Vulkan 1.3+（基于 Rust `ash` 底层绑定與 `gpu-allocator` / 自研 TLSF 管理）。
- **显存指针寻址**: 强制启用 `bufferDeviceAddress` (BDA)，通过 $64\text{B}$ 全局 Push Constants 直接向着色器传递物理显存地址，彻底解耦 CPU/GPU 内存结构传递。
- **描述符机制**: 全局统一 Descriptor Set（基于 `VK_EXT_descriptor_indexing` 的 Bindless 架构），废弃传统描述符集频繁绑定与切换。
- **现代管线特性**:
  - 强制开启 `dynamicRendering`（废弃旧式 `VkRenderPass` 与 `VkFramebuffer` 样板代码）。
  - 强制开启 `synchronization2` 与 `timelineSemaphore`（实现 CPU、主渲染队列、专用传输队列与异步计算队列的精细多队列无锁同步）。
- **多批次合并**: 启用 `multiDrawIndirect` (MDI) 与 `drawIndirectCount`，支持 GPU Compute Shader 动态裁剪、紧凑化并组装绘制命令。

## 2.2 显存分层与多帧并发安全架构

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                              Vulkan 物理显存管理体系                              │
├───────────────────────────┬──────────────────────────┬───────────────────────────┤
│   持久驻留池 (TLSF Pool)   │   帧瞬态环 (Ring Buffer)  │   离屏特效池 (TexturePool) │
│ • 静态 UI / 分块画布实体   │ • 每帧相机/PushConstants  │ • 背景毛玻璃双パス模糊 RT  │
│ • 双格式多页字形图集       │ • 高频动画独立变换矩阵    │ • 图层组透明度离屏合成 RT  │
│ • Compute Scatter 增量更新│ • 瞬态框选框/对齐红线/补丁│ • 尺寸分级按帧复用池       │
│ • Epoch 延迟隔离回收队列   │ • Staging 异步上传暂存区  │ • Render Graph 自动屏障    │
└───────────────────────────┴──────────────────────────┴───────────────────────────┘
```

- **持久驻留池 (Persistent Pool)**:
  - 基于 TLSF (Two-Level Segregated Fit) / Offset Allocator 算法在 `DEVICE_LOCAL` 显存上自管理大块连续 `VkBuffer`。
  - 存放无限画布持久实体、静态 UI 节点、裁剪节点以及曲线控制点数据。
  - **局部增量修补 (Compute Incremental Scatter)**: 当部分节点属性发生微量变化时，CPU 仅将变更的稀疏槽位索引与新数据打包写入当前帧的瞬态环，由主渲染队列在帧首派发单次 Compute Shader 执行 Scatter 拷贝至持久池，紧接 `VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT` 到渲染阶段的管线屏障，严禁全量重传。
  - **多帧并发安全回收 (Epoch Quarantine Free-List)**: 为防止双缓冲/三缓冲（Frames-in-Flight）下 GPU 正在读取第 $N$ 帧的持久池地址而 CPU 在第 $N+1$ 帧将其释放重分配，被删除或扩容搬迁的旧 TLSF 内存块禁止立即归还空闲链表，而是推入绑定当前帧 Timeline Semaphore 值的延迟回收队列 `pending_frees[frame_epoch]`，待 GPU 确认第 $N$ 帧执行完毕后才真正回收复用。
- **帧瞬态环 (Transient Ring Buffer)**:
  - 采用 `HOST_VISIBLE | HOST_COHERENT | DEVICE_LOCAL`（ReBAR / Smart Access Memory 优先）的按帧轮转环形缓冲。
  - 存放每帧活跃动画变换矩阵、高频鼠标指示线、框选虚线框、增量 Scatter 补丁包与瞬态绘制参数。
- **离屏特效池 (Texture Pool) 与轻量渲染图 (Render Graph)**:
  - 维护按 $2$ 的幂次或屏幕比例分级的离屏纹理对象池。
  - 由轻量级 **Render Graph** 统一分析依赖并自动插入图像布局转换屏障（Layout Transitions），支持背景毛玻璃（Dual-Kawase Backdrop Blur）与混合透明度图层组（Group Opacity）的按需申请与单帧内原地复用。
- **色彩管理与宽色域管线 (Color Management)**:
  - Swapchain 统一优先采用 `VK_FORMAT_B8G8R8A8_SRGB`（或 HDR 宽色域 `VK_EXT_swapchain_colorspace`）。
  - 全局颜色 Token 存储于线性空间（或 Oklab 感知均匀空间），确保半透明叠加、渐变插值与边缘抗锯齿在线性光强下完成物理正确混合，消除暗边（Dark Fringe）。
- **灾难恢复机制 (Device Lost Recovery)**:
  - 全局捕获 `VK_ERROR_DEVICE_LOST` 异常。
  - 保持 Rust 纯内存领域状态树完好，原子化销毁并重建 `VkDevice`、交换链、显存池与管线，将内存状态树原地重新灌注（Re-inflate）至新持久池并重栅格化可见字形，实现用户无感知热恢复。

## 2.3 Windows DWM 无闪烁窗口缩放与超低延迟呈现

- **模态缩放阻塞破除**: 在 Windows 平台下，脱离默认阻塞式 Win32 模态 `WM_SIZE` / `WM_MOVESIZEMOVE` 消息循环（采用独立渲染提交线程或 Win32 Fiber 钩子），避免拖拽窗口边缘时主线程挂起。
- **零黑边同步与低延迟呈现**:
  - 启用 `VK_EXT_swapchain_maintenance1` 与 `VK_KHR_present_wait`（或 DirectComposition 视觉表面绑定），使窗口尺寸改变时 Swapchain 重建与桌面窗口管理器（DWM）合成严格同帧对齐，实现 $120\text{Hz}$ 拖拽窗口边缘零黑块、零画面拉伸抖动。
  - 利用可等待交换链图像就绪信号量控制 CPU 采样输入时刻，将 Input-to-Photon 延迟压缩至小于 $8.3\text{ms}$。

## 2.4 核心 GPU 数据结构与 BDA 内存协议

所有传往 GPU 的数据结构严格使用 `#[repr(C)]` 并对齐至 $16\text{B}$ 或 $64\text{B}$ 边界：

```rust
/// 每帧仅 64 字节的 Push Constants，统领全局所有 GPU 物理缓冲地址
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FramePushConstants {
    pub primitives_bda: u64,      // 指向持久驻留池 / 瞬态环中的 GpuPrimitiveInstance 数组
    pub transforms_bda: u64,      // 指向独立动画变换通道 GpuTransform 数组 (跳开 Taffy)
    pub clips_bda: u64,           // 指向双槽位嵌套圆角裁剪块 GpuClipNode 数组
    pub theme_lut_bda: u64,       // 指向全局零开销主题颜色表 (RGBA16F)
    pub payloads_bda: u64,        // 指向曲线段控制点 / 渐变色标 / 扩展属性缓冲
    pub camera_chunk: [i32; 2],   // 摄像机所在世界区块坐标 (4096x4096 为一区块)
    pub camera_local: [f32; 2],   // 摄像机在区块内的局部 f32 坐标
    pub viewport_size: [f32; 2],  // 物理视口像素尺寸
    pub camera_zoom: f32,         // 画布摄像机缩放倍率
    pub dpi_scale: f32,           // 系统物理 DPI * 应用全局 UI 缩放
}

/// 64 字节紧凑型统一图元实例（恰好占满 1 条 64B CPU/GPU Cache Line）
#[repr(C, align(64))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuPrimitiveInstance {
    pub chunk_id: [i32; 2],       // 8B: 世界区块 ID (静态 UI 控件固定为 [0, 0])
    pub local_pos: [f32; 2],      // 8B: 区块内局部坐标 [0.0, 4096.0)
    pub size: [f32; 2],           // 8B: 图元几何宽高
    pub corner_radii: [u16; 4],   // 8B: 左上、右上、右下、左下圆角半径 (f16 编码)
    pub border_width: u16,        // 2B: 边框宽度 (f16 编码)
    pub shadow_blur: u16,         // 2B: 解析解阴影模糊半径 (f16 编码，0 表示无阴影)
    pub transform_idx: u32,       // 4B: 索引至 transforms_bda (最高位 bit31 标记动画激活态以旁路像素吸附)
    pub clip_idx: u32,            // 4B: 索引至 clips_bda (含硬裁剪 AABB + 两层父级圆角裁剪)
    pub fill_token_or_uv: u32,    // 4B: 颜色 Token ID 或 图集打包归一化 UV 索引
    pub border_token_or_tex: u32, // 4B: 边框颜色 Token ID 或 Bindless Texture/Atlas 层索引
    pub prim_type_and_flags: u32, // 4B: [8b 图元类型 | 8b 混合模式 | 16b 扩展标记(如亚像素相位)]
    pub z_and_opacity: u32,       // 4B: [16b 归一化深度值 | 16b UNORM 透明度]
    pub extra_payload_idx: u32,   // 4B: 指向 payloads_bda 中的二次曲线段或复杂渐变描述符
}

/// 32 字节双槽位裁剪节点（支持任意嵌套 AABB + 两级父容器圆角交集裁剪）
#[repr(C, align(16))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuClipNode {
    pub aabb_clip: [f32; 4],      // 16B: CPU 预求交折叠后的硬裁剪边界 (min_x, min_y, max_x, max_y)
    pub rounded_clip_0: [f32; 4], // 16B: 第一级父圆角容器 (center_x, center_y, half_w, half_h)
    pub radii_0: [u16; 4],        // 8B:  第一级父圆角四角的半径 (f16)
    pub rounded_clip_1: [f32; 4], // 16B: 第二级父圆角容器 (center_x, center_y, half_w, half_h)
    pub radii_1: [u16; 4],        // 8B:  第二级父圆角四角的半径 (f16)
}
```

---

# 3. 统一图元、矢量曲线与工业级文字渲染管线

## 3.1 统一 Uber-Shader 架构与双通道深度合作 (解决 Early-Z 与半透明混合悖论)

为彻底消除管线状态切换（PSO Switch）并同时获得硬件 Early-Z 遮挡剔除与完美的半透明边缘混合，渲染管线采用 **Uber-Shader 单一管线 + 不透明内核预剔除双通道（Opaque Core Pre-Pass + Blended Uber-Pass）**：

- **零材质切换 Uber-Shader**:
  - 既然已具备 BDA + Bindless + `Texture2DArray`，标准矩形、圆角边框、解析解内外阴影、图片、字形、二次贝塞尔曲线全部合并进**同一个 Uber-Shader 管线**。
  - 通过 `prim_type` 执行分支：由于同一个 Quad 的所有片元必然执行完全相同的分支路径，GPU Subgroup/Warp 内的分支发散率恒为 $0\%$。
- **双阶段深度合作机制**:
  - **Pass A（Opaque Core Pre-Pass，从前向后）**: 针对大面积不透明容器（如背景面板、窗口卡片、画布节点背景，满足 `opacity == 1.0`），GPU Compute 阶段自动将其 AABB 向内收缩最大圆角半径 $R$，生成纯不透明内核矩形（Opaque Core）。按**从前向后**顺序仅向 Depth Buffer（采用 Reverse-Z，`VK_COMPARE_OP_GREATER_OR_EQUAL`）写入深度值，不输出颜色，极速建立硬件遮挡掩码。
  - **Pass B（Unified Uber-Shader Pass，从后向前）**: 按拓扑 Z 序**从后向前**执行单次 MDI 批量绘制，开启 `depthTestEnable = true, depthWriteEnable = false`。被顶层大面板完全遮挡的底层成千上万个图元直接被硬件 Early-Z / Hi-Z 拦截丢弃（零片元着色开销），而可见图元的抗锯齿圆角边缘、文字与半透明阴影则按严格画家算法完美混合。

## 3.2 双阶零开销裁剪与逆变换局部坐标重构 (Dual-Stage Clipping)

- **CPU 端 AABB 预求交**: 扁平化遍历父子裁剪层次，在 CPU 端将所有嵌套轴对齐 AABB 矩形交并折叠为唯一一个 `vec4(min_x, min_y, max_x, max_y)` 硬裁剪框。
- **顶点着色器 (VS) 逆变换局部坐标重构法 (Inverse-Transform Local Reconstruction)**:
  - **完全视口外坍缩**: 若图元屏幕外接包围盒与 `aabb_clip` 交集为空（`min_x >= max_x` 或 `min_y >= max_y`），在 VS 中直接将 4 个顶点全部赋值为 `(0.0, 0.0, 0.0, 0.0)`，由硬件图元装配阶段零开销剔除。
  - **消除旋转畸变与 UV 拉伸**: 当图元携带旋转/斜切变换矩阵 $M$ 且与父级轴对齐 `aabb_clip` 发生部分相交时，严禁直接钳制旋转后的顶点（否则平行四边形会扭曲成不规则多边形）。VS 先计算图元变换后 4 个顶点的屏幕外接 AABB 并钳制到 `aabb_clip` 内得到屏幕裁剪顶点 $P_{\text{screen}}$；随后在 VS 中乘以图元变换矩阵的逆矩阵 $M^{-1}$ 重构出对应的图元局部坐标 $P_{\text{local}} = M^{-1} \cdot P_{\text{screen}}$，并以 `noperspective` 插值传入 FS。
- **片元着色器 (FS) 废除 `discard` 与双槽位嵌套圆角**:
  - 全面废除 FS `discard` 指令，保持硬件 Early-Z 管线畅通。
  - FS 接收 `P_local`，通过有向距离场（SDF）配合 `smoothstep` 计算图元自身边界与自身圆角 `corner_radii` 的抗锯齿覆盖率 $\alpha_{\text{self}}$。
  - 同时读取 `GpuClipNode` 中的两个父级圆角裁剪槽位 `rounded_clip_0` 与 `rounded_clip_1`，执行两次极轻量 SDF Box 求交：
    $$\alpha_{\text{final}} = \alpha_{\text{self}} \times \min(\alpha_{\text{clip0}}, \alpha_{\text{clip1}})$$
  - 零离屏模板缓冲（Stencil Buffer）开销即可完美处理高达两层嵌套圆角容器与自身圆角的任意复合裁剪。

## 3.3 解析解阴影与内阴影 (Analytical Box & Inner Shadow)

- 严禁使用离屏高斯模糊渲染常规 UI 卡片或画布节点的投影（Drop Shadow）与内阴影（Inner Shadow）。
- 在 Uber-Shader 中内置基于误差函数近似（Error Function $\text{erf}(x)$ Approximation）的**解析圆角矩形阴影积分算法**：
  - 仅需将阴影图元尺寸向外扩展 $3 \times \text{shadow\_blur}$，在单次 Quad 片元着色中直接用解析公式计算出高斯卷积后的精确 Alpha 衰减值，实现同屏上万个带柔和阴影节点零离屏 RT 切换、零额外带宽消耗。

## 3.4 曲线与矢量渲染 (Compute 降阶细分 + OBB + GPU 解析解 SDF)

- 彻底放弃高 CPU 消耗且缩放需重新细分的离线三角化方案（如 `lyon`）。
- **三次贝塞尔自适应降阶为二次曲线段 (Cubic-to-Quadratic Subdivision)**:
  - 由于点到三次贝塞尔曲线（Cubic Bezier）的距离等价于求解五次多项式方程（代数上无解析求根公式，片元牛顿迭代算力开销极大），所有三次贝塞尔连线在写入持久缓冲或通过前置 Compute Shader 处理时，利用 Wang's Formula 自适应细分为 $2 \sim 6$ 段**二次贝塞尔曲线（Quadratic Bezier）**（拟合几何误差恒小于 $0.05\text{px}$）。
- **紧致有向包围盒 (Oriented Bounding Box - OBB)**:
  - 废弃轴对齐粗包围盒（对角线长连线的 AABB 有 $80\%$ 以上面积为空白像素）。
  - 在 VS 阶段沿着每段二次贝塞尔曲线的首尾弦向量方向生成**紧致有向包围盒（OBB，宽度仅为控制多边形拱高加上线宽与抗锯齿边缘）**，将光栅化无效片元数量降低 $80\%$ 以上。
- **数值稳定二次贝塞尔解析解**:
  - 在 FS 阶段采用消除浮点灾难性抵消（Catastrophic Cancellation）的改进卡尔达诺解析算法，实时解算像素点到二次贝塞尔曲线的精确代数距离，支持无限缩放而显存占用恒定为 $O(1)$。

## 3.5 工业级文字渲染体系 (`cosmic-text` 强化 + 双图集 + GPU MTSDF)

- **排版塑形双级缓存**:
  - 第一级：字符级/词级 Shape 结果缓存（针对单行标签、按钮与固定短语，基于 FNV/xxHash 键值索引）。
  - 第二级：带折行宽度约束的段落 Wrap 结果缓存。
- **双格式分离多页动态字形图集 (`Atlas_R8` + `Atlas_RGBA8`)**:
  - 避免因兼容彩色 Emoji 而将全部字形升级为 4 通道导致显存暴增 $4$ 倍，系统维护两个独立的 `VkImageViewType::TYPE_2D_ARRAY`（单层 $2048 \times 2048$，利用 Bindless 动态扩容层数）：
    1. `Atlas_R8` (`VK_FORMAT_R8_UNORM`，单字节/像素)：专供标准 UI 控件高精度灰度字形，支持 **$4$ 档水平亚像素相位（$0.0, 0.25, 0.5, 0.75$）** 光栅化，保证小字号（$12\text{px} \sim 16\text{px}$）拥有极致锐利的边缘与均匀字距。
    2. `Atlas_RGBA8` (`VK_FORMAT_R8G8B8A8_UNORM`，四字节/像素)：专供彩色 Emoji 与无限画布多通道有向距离场（MTSDF）字形。
  - **帧代际 LRU 驱逐策略**: 每个图集槽位记录 `last_accessed_frame`，当图集层满时原地覆盖长期未引用的冷字形槽位。
- **GPU Compute 直接生成 MTSDF 字形 (消除画布缩放 Pop-in)**:
  - 放弃缓慢的 CPU 端 MSDF 生成算法。无限画布文字通过 `skrifa` 直接提取字形轮廓二次曲线段上传至 Staging Buffer，由极轻量的 **GPU Compute Shader 并行光栅化生成 MTSDF 图集槽位**（单个字形耗时小于 $0.05\text{ms}$）。
  - 画布从 $10\%$ 到 $6400\%$ 连续缩放时全程保持边缘刀锋般锐利，彻底消除缩放停顿时的异步重绘跳变（Pop-in）。
- **感知伽马校正 (Perceived Text Gamma Correction)**:
  - 在 Uber-Shader 字形分支中注入基于背景感知亮度的字干粗细补偿（Stem Darkening）与伽马校正算法：亮色模式（白底黑字）下微调收缩边缘 Alpha，暗色模式（黑底白字）下适度补偿字体骨架，彻底消除黑白颠倒时的视觉发虚与膨胀现象。

---

# 4. UI 核心范式、增量排版、虚拟化与动画引擎

## 4.1 细粒度保留模式与面向数据拓扑

- **实体组织**: 采用扁平 `SlotMap<NodeId, LayoutNode>` 与按需挂载的 `SecondaryMap<NodeId, ComponentData>` 替代传统面向对象指针树。
- **单向数据流与零拷贝快照**: 业务逻辑严格遵循 `State -> Action -> Reducer`，大集合状态采用基于写时复制（COW，`im` / `rpds` 或自研 Arc-Slice）的结构共享容器。
- **行为数据化 (Action as Data)**: UI 组件事件回调仅允许绑定强类型可序列化 `Enum Action`，严禁在视图树节点中挂载捕获环境的动态闭包（`Box<dyn Fn>`）。

## 4.2 增量排版引擎 (Taffy 深度定制 + 隔离边界 + 三级脏标记)

- **排版隔离边界 (`RelayoutBoundary`)**:
  - 凡满足“宽高明确设定（Fixed / Percentage）且 `overflow != visible`”的容器（如固定尺寸卡片、滚动容器、无限画布宿主叶节点）自动标记为 `RelayoutBoundary`。
  - 容器内部子组件发生尺寸或文本变动时，脏标记向上回溯至最近的 `RelayoutBoundary` 即刻截断，严禁向根节点冒泡；多个独立的脏边界子树通过 Rayon 线程池并行调用 Taffy 求解。
- **视图记忆化 (Memoized Views)**:
  - 每个组件节点维护 `state_generation: u64`。当父组件重绘而子组件输入状态代际版本号未变时，直接跳过子树 `view()` 构造函数与排版过程，原地复用持久池中的 GPU 实例区间。
- **三级脏标记分流 (Dirty Propagation Triage)**:
  1. **Level 1 - 纯外观/变换脏变（Color / Opacity / Transform）**: **零排版开销**。仅标记 `dirty_paint_nodes`，直通帧瞬态环或通过 Compute Scatter 修补持久池对应槽位。
  2. **Level 2 - 隔离边界内布局脏变**: 仅重算局部 `RelayoutBoundary` 子树并增量更新受影响节点的坐标。
  3. **Level 3 - 全局拓扑脏变**: 窗口尺寸变化或顶层分割线拖拽时触发。

## 4.3 一等公民视口虚拟化引擎 (Viewport Virtualization Engine)

- 针对包含 $10$ 万级条目的图层树、资产网格或日志面板，框架核心层内置 `VirtualList`、`VirtualTree` 与 `VirtualGrid` 容器协议。
- **环形槽位复用 (Ring-Buffer Slot Recycling)**:
  - 仅在 `SlotMap` 与 Taffy 树中实例化当前视口可见区域加上下各 $1$ 屏缓冲区的节点（通常恒定在 $60 \sim 120$ 个实体）。
  - 滚动发生时，未跨越单行高度阈值前仅更新 GPU 端 `Transform` 平移偏移量（耗时为 $0$）；跨越行高阈值时，将滑出视口顶部的节点槽位原地复用于底部新进入视口的数据项，保证 $10$ 万行列表 $120\text{FPS}$ 极速滚动零内存分配。

## 4.4 动画引擎与 Taffy 彻底解耦 + 动态像素吸附旁路

```
                    ┌───────────────────────────┐
                    │     UI 状态 / Action      │
                    └─────────────┬─────────────┘
                                  │
                 ┌────────────────┴────────────────┐
                 ▼                                 ▼
        [布局几何相关属性]                 [变换与外观独立通道]
     • 宽高 / Margin / Padding         • Transform (平移/缩放/旋转/斜切)
     • Flex / Grid 约束规则            • Opacity / Color Token ID
                 │                                 │
                 ▼                                 ▼
          Taffy 增量排版求解               物理弹簧插值器 (Spring Engine)
       (仅在尺寸脏变时局部触发)              (完全跳过 Taffy，直写瞬态环)
                 │                                 │
                 ▼                                 ▼
     [静态态: 强制物理像素网格吸附]      [动画激活态: 自动旁路吸附，亚像素平滑插值]
                 │                                 │
                 └────────────────┬────────────────┘
                                  ▼
                     合入 GPU BDA 实例数据并渲染
```

- **属性通道物理分离**:
  - `Transform`（2D 仿射变换）与 `Opacity`（透明度）作为独立 GPU 通道存储在 `transforms_bda` 中，绝不进入 Taffy 布局管线。
  - 物理弹簧动画引擎（Spring Physics）与贝塞尔曲线插值器每帧仅更新活跃节点的 `GpuTransform` 槽位，维持持续 $120\text{FPS}$ 渲染而不引发任何排版计算。
- **静态像素吸附与动画亚像素平滑切换 (Pixel Snapping vs. Subpixel Interpolation)**:
  - **静态清晰度保障**: 静止状态的 UI 节点（`transform_idx` 最高位为 $0$）在 VS 中强制将坐标与宽高对齐至物理像素整数网格（`round(pos * dpi) / dpi`），确保 $1\text{px}$ 边框绝对锐利，杜绝半像素模糊。
  - **动态丝滑度保障**: 一旦节点处于弹簧动画或拖拽平滑过渡中（`transform_idx` 最高位标记为 $1$），VS 自动旁路整数像素吸附，进入全精度浮点亚像素插值模式，彻底消除动画减速收尾阶段的“阶梯跳动感（Pixel Crawl）”；动画收敛静止的最后一帧平滑恢复像素吸附。
- **响应式帧调度 (Reactive Frame Pacing)**:
  - 系统全局维护 `active_animations_count` 与输入事件队列。当计数器归零且无待处理事件时，主循环主动挂起（`WaitMessage` / `epoll_wait`），CPU 与 GPU 占用率瞬时归零（$0.0\%$）。

## 4.5 多层级正交缩放与零开销主题切换

- **三层缩放严格正交分离**:
  1. **系统物理 DPI (`OS Scale`)**: 控制 Swapchain 物理分辨率、高精文字图集光栅化字号及静态 $1\text{px}$ 物理像素网格吸附基准。
  2. **应用全局 UI 缩放 (`App Zoom`)**: 缩放 Taffy 样式逻辑尺寸基准与标准控件字号，作用于外围面板与工具栏。
  3. **画布摄像机缩放 (`Canvas Zoom`)**: 纯 GPU 视图投影矩阵与分块坐标变换，仅影响画布视口内部图元与 MTSDF 字形采样，与外层 UI 树尺寸计算物理隔离。
- **零开销深浅色主题切换 (Zero-Cost Theme Switching)**:
  - 提取所有语义化 Design Tokens 为扁平全局颜色查找表（`theme_lut_bda`，大小通常小于 $4\text{KB}$）。
  - 图元实例数据中仅存储 $16\text{bit}$ 颜色 Token ID。切换亮色/暗色/高对比度主题时，仅需向 GPU 单次写入更新 $4\text{KB}$ 颜色表缓冲，**耗时小于 $0.02\text{ms}$，完全阻断 Taffy 重新排版与图元实例重传**。

---

# 5. 无限画布与空间领域引擎

## 5.1 空间索引与双层混合剔除架构 (CPU Loose Quadtree + GPU Chunk Culling)

- **拒绝固定网格哈希退化**: 废弃在跨尺度图元（微小图标与巨型分组 Frame 并存）下严重退化的固定尺寸网格哈希，采用**连续内存数组实现的松散四叉树 (Loose Quadtree)**。
- **松散边界定义**:
  - 四叉树节点包围盒在其标准几何边界基础上向外扩展 $k$ 倍（取 $k = 0.5$，即宽松度为 $2\times$），使位于分割线交界处的物体无需切分即可完全落入对应深度的单一节点内，$O(1)$ 完成层级定位（利用前导零指令 `leading_zeros` 直接算出四叉树深度与 Morton 码索引）。
- **零堆碎片 SoA 数据结构**:
  - 采用纯扁平数组与侵入式单向链表存储：`node_aabbs: Vec<AABB>`, `node_children: Vec<[u32; 4]>`, `node_element_head: Vec<u32>`, `element_next: Vec<u32>`。
  - 元素移动时仅需在链表头尾修改 `u32` 索引，**全程零堆内存分配**。
- **CPU + GPU 双层协同剔除**:
  - CPU 端利用松散四叉树在小于 $0.1\text{ms}$ 内筛出与当前摄像机视口相交的活跃区块（Active Chunks）与四叉树叶节点范围。
  - GPU Compute Shader 对活跃区块内的图元执行并行细粒度视口相交测试（Frustum Culling）与屏占像素面积计算，自动剔除小于 $0.5\text{px}$ 的极小图元并输出可见实例列表。

## 5.2 分块锚定相对坐标体系 (Chunk-Anchored Camera-Relative Rendering)

为彻底解决传统方案中“为了防止大世界坐标抖动而每帧在 CPU 重算所有可见图元相对坐标并重传 GPU”导致的带宽崩溃问题，引擎采用 **分块锚定相对坐标体系**：

- **逻辑分块与静态驻留**:
  - 将无限画布世界空间按 $4096 \times 4096$ 逻辑像素划分为规则网格区块（Chunk）。
  - GPU 持久驻留池中的每个画布实体**静态存储** `chunk_id: [i32; 2]`（$8\text{B}$）与区块内局部浮点偏移 `local_pos: [f32; 2]`（$8\text{B}$，值域恒定限制在 $[0.0, 4096.0)$ 内，浮点尾数精度恒定高于 $0.00024\text{px}$）。
- **GPU 原位相对坐标解算**:
  - 摄像机同样维护 `camera_chunk: [i32; 2]` 与 `camera_local: [f32; 2]`，每帧仅通过 $16\text{B}$ Push Constants 传入 GPU。
  - 在 Vertex Shader 与 Culling Compute Shader 中原位执行整数区块差值运算：
    $$\Delta_{\text{chunk}} = \text{vec2}(\text{prim.chunk\_id} - \text{camera.chunk\_id})$$
    $$P_{\text{camera\_relative}} = \Delta_{\text{chunk}} \times 4096.0 + (\text{prim.local\_pos} - \text{camera.local\_pos})$$
- **核心收益**:
  - 画布任意平移或缩放时，**GPU 持久驻留池实现真正的 $0\text{ Byte}$ 更新**！同时在坐标绝对值高达 $10^{12}$ 的超大画布边缘，依然保持亚像素级绝对丝滑，绝无顶点抖动。

## 5.3 交互编辑：瞬态隔离、视口级智能磁吸与事务化撤销

- **瞬态与持久状态严格物理隔离**:
  - 框选虚线框、鼠标悬浮高亮框、实时对齐磁吸红线、连接线拖拽预览属于**瞬态状态 (Ephemeral State)**，在输入处理阶段直接生成图元写入帧瞬态环（Transient Ring Buffer），绝不污染 Reducer 业务状态树与 Undo 历史栈。
- **视口级智能磁吸计算 (Viewport-Bounded Smart Snapping)**:
  - 实体拖拽移动时，利用松散四叉树仅检索当前屏幕视口及外扩 $64\text{px}$ 范围内的候选图元。
  - 若视口内图元密集，按面积与距离优先级截断前 $128$ 个显著对象，构建 X/Y 轴向一维有序区间表，在小于 $0.05\text{ms}$ 内完成左/中/右、顶/中/底边缘对齐与等间距（Equal Spacing）判定（屏幕空间距离小于或等于 $5\text{px}$ 时触发坐标吸附修正并生成瞬态红线）。
- **原子事务撤销/重做与输入合并 (Transaction Scope & Coalescing)**:
  - 复杂拖拽交互开始时派发 `BeginTransaction`，拖拽期间高频派发轻量 `UpdateTransaction`（仅原地修改当前活跃快照，不压入历史栈），释放鼠标时派发 `CommitTransaction`。
  - **文本输入合并**: 针对连续键盘输入，基于时间窗口（如 $500\text{ms}$）与词边界（空格/标点/换行）自动合并連続字符变更，结合 COW 结构共享，使百万级节点画布的单步 Undo 快照内存增量恒定在数 KB 以内。

---

# 6. 输入管线、富文本交互、浮层排序与无障碍语义

## 6.1 双层拾取系统 (Hit-Testing) 与事件捕获

```
                         OS 硬件输入 / 原生文件拖拽事件
                                      │
                                      ▼
                        全局浮层 / Portal Overlay 检测
                    (按 Z-Order 逆序遍历顶层 Menu / Modal)
                                      │ (若未命中或穿透)
                                      ▼
                          Taffy UI 树 AABB 递归拾取
                    (结合双槽位圆角 ClipMask 验证有效点击区)
                                      │ (若命中 Canvas 宿主视口叶节点)
                                      ▼
                        松散四叉树画布空间 O(log N) 粗筛
                                      │ (输出粗筛候选列表)
                                      ▼
                          精确几何窄相判定 (Narrow Phase)
                  • 点到二次贝塞尔曲线段解析距离 <= 判定线宽
                  • 逆变换局部坐标点在圆角矩形 / 多边形内判定
```

- **独占事件捕获 (Pointer Capture)**:
  - 当激活节点拖拽、画布平移、滚动条滑块拖动或文本选区划选时，自动锁定 `Pointer Capture` 并调用 OS 级鼠标捕获；即使光标移出物理窗口边界，后续运动与释放事件仍 $100\%$ 独占路由至激活组件。
- **OS 原生拖拽穿透 (Native Drag-and-Drop)**:
  - 将操作系统的文件/富媒体拖入事件映射为内部带坐标的瞬态 `DragOver` Action，直接复用上述双层拾取管线实时高亮目标 UI 槽位或画布投递区域。

## 6.2 输入法 (IME)、富文本选区合并与光标栈

- **CJK 完整输入法管线**:
  - 聚焦文本组件或光标移动时，实时结合 UI 布局或画布摄像机变换矩阵算出光标在物理窗口中的屏幕坐标，调用平台层 `set_ime_cursor_area` 精确锚定系统候选词浮窗。
  - 完整处理 `Preedit / Composition` 预编辑文本流，在 `cosmic-text` 层面支持瞬态行内波浪下划线、分段高亮与光标切分。
- **富文本精细交互与选区圆角布尔合并**:
  - 支持双向文本（BiDi）光标视觉跳跃与逻辑移动、连字（Ligature）内部按比例光标切分。
  - 多行文本划选时，对相邻行的选区矩形执行**几何联合与外轮廓圆角化处理（Rounded Selection Polygon Union）**，消除传统逐行独立矩形拼接处的生硬锯齿。
- **确定性光标状态栈 (Cursor Stack)**:
  - 采用基于拾取路径优先级的覆盖机制：`Active Pointer Capture > 顶层 Overlay > 尺寸调节控制柄 (Resize Handles) > 画布自定义工具光标 > 交互式文本 (I-Beam) > 默认箭头`。

## 6.3 浮层 Portal 逃逸与改良版 64-Bit 深度键基数排序

- **Portal 逃逸与视口自适应防溢出**:
  - 下拉菜单（Dropdown）、气泡提示（Tooltip）、上下文菜单挂载在逻辑组件树任意深处，但通过 `Portal` 机制直接投递至根级 Overlay 图元层，天然免除父级容器的 `GpuClipNode` 裁剪。
  - 内置 `Flip / Shift / Size` 锚定定位求解器，当弹窗靠近物理窗口边缘时自动翻转展开方向或微调偏移，确保 $100\%$ 处于物理窗口可见范围内。
- **改良版 64-Bit 深度排序键与 GPU OneSweep 基数排序**:
  - 得益于 Unified Uber-Shader 消除了材质切换，64 位无符号整型排序键专职服务于**渲染阶段屏障隔离**与**严格拓扑遮挡顺序**：

| 位域范围 | 字段名称 | 位宽 | 编码规则与架构作用说明 |
| :--- | :--- | :--- | :--- |
| **Bits 63..56** | `Pass / Portal Layer` | 8-bit | `0`=无限画布层，`1`=主 UI 面板层，`2..254`=Portal 模态弹窗与菜单层，`255`=顶层诊断调试层 |
| **Bits 55..40** | `RenderGraph Barrier ID` | 16-bit | 常规图元保持不变；仅当遇到背景毛玻璃（Backdrop Blur）或图层组透明度合成时递增，指示命令流插入离屏 RT 切换屏障 |
| **Bits 39..36** | `Pipeline Variant` | 4-bit | `0`=标准 Unified Uber-Shader（覆盖 $99.9\%$ 图元），`1..15`=用户自定义扩展着色器变体 |
| **Bits 35..8** | `Topological Z-Order` | 28-bit | 支持高达 $2.68$ 亿层的严格前后拓扑遮挡顺序（保证 Alpha Blending 绝对正确） |
| **Bits 7..0** | `Sub-Element Order` | 8-bit | 同一组件内部子图元固定次序：`0`=外阴影，`1`=背景填充，`2`=内容/图像/文本，`3`=内阴影，`4`=外边框，`5`=焦点环 |

  - 在 GPU Compute Shader 中执行单次 **OneSweep 64-Bit 基数排序（Radix Sort）**，$10$ 万级图元排序耗时小于 $0.15\text{ms}$，随后通过 Prefix-Sum 自动合并具有相同 `RenderGraph Barrier ID` 与 `Pipeline Variant` 的连续区间，生成极少数 `VkDrawIndirectCommand`。

## 6.4 无障碍语义体系 (`AccessKit`) 双模赋能

- 将 `AccessKit` 作为 UI 框架的一级语义输出端，与 `SlotMap` 节点生命周期和增量脏标记深度绑定。
- 仅在节点发生状态或布局变更时，增量构建 `accesskit::TreeUpdate` 推送给操作系统无障碍接口（Windows UI Automation / macOS NSAccessibility / Linux AT-SPI）。
- 同时将该语义树直接暴露为 **AI Agent 的首选零成本自省协议**，无需维护第二套自定义 AST。

---

# 7. Agent 特化设计与无头闭环验证

## 7.1 编译时效与开发期护栏 (保证增量编译小于 800ms)

- **Workspace 物理分层与依赖下沉**:
  - `engine_core` 基础库：封装 Vulkan 1.3 底座、TLSF 显存池、Taffy、`cosmic-text`、`AccessKit` 与松散四叉树。一旦编译完成，在业务开发期长期保持静态缓存。
  - `app_guest` 业务层：仅依赖轻量纯数据结构定义（POD）、`Action` 枚举与声明式构建器接口，零重型宏与零图形库依赖。
- **扁平具体类型 (Flat Concrete Types)**:
  - 全面禁止深层嵌套的泛型 Typestate 视图树（如 `VStack<HStack<Button<...>>>`），统一采用扁平描述符结构体与轻量 `NodeId` 句柄，彻底杜绝 Rust 泛型单态化（Monomorphization）代码膨胀与 LLVM 优化瓶颈。
- **正交极简 API (Orthogonal Minimal API)**:
  - 每个布局属性、样式约束与交互行为仅提供唯一一种正交声明方式，拒绝语法糖宏，最小化 API 表面积，从根源上消除大模型生成代码时的 API 幻觉。

## 7.2 无头闭环驱动验证 (Headless Verification Harness)

- **亚毫秒级纯内存驱动环境**:
  - 完全脱离操作系统窗口句柄（HWND）与 Vulkan GPU 实例，在纯 CPU 内存中实例化 `UiWorld` 与无限画布空间索引。
  - 在 **小于 $1.0\text{ms}$** 内完成：模拟输入事件序列派发 $\rightarrow$ Reducer 状态演进 $\rightarrow$ Taffy 局部增量排版 $\rightarrow$ 松散四叉树同步 $\rightarrow$ 图元生成与自动几何体检。
- **内置 5 项自动化几何体检断言 (Layout Sanitizers)**:
  1. `AssertNoNaNOrInf`: 拦截任何排版或坐标变换计算得出的 `NaN`、`Infinity` 或负数宽高异常。
  2. `AssertTextNotClipped`: 结合 `cosmic-text` 塑形结果，检测文本真实墨水包围盒（Ink Bounding Box）是否在未声明 `text-overflow: ellipsis` 或滚动容器的情况下被父容器裁剪截断。
  3. `AssertInteractiveTargetSize`: 检查所有绑定了交互 `Action` 的可点击组件，其最终裁剪后的有效拾取屏幕面积是否小于最小可交互阈值（默认 $24 \times 24\text{px}$），或中心点是否被非预期的兄弟节点完全遮挡。
  4. `AssertNoUnintendedSiblingOverlap`: 检测同一 Flex/Grid 布局容器内的同级子元素是否由于约束冲突发生非预期的几何包围盒重叠。
  5. `AssertViewportContainment`: 验证所有通过 `Portal` 弹出的菜单、对话框与 Tooltip 是否 $100\%$ 处于物理视口边界内部。
- **精准源码定位与轻量二进制快照**:
  - 所有组件构建函数自动携带 Rust `#[track_caller]` 属性，记录创建该节点的源码文件名与行号（`Location::caller()`）。
  - 一旦几何体检断言触发，立即输出包含 `NodeId`、源码精确行号、父子约束链、实际计算出的 AABB 以及 `AccessKit` 语义路径的结构化 JSON 错误栈，供 Agent 直接精准修复代码。
  - 时间旅行调试（Time-Travel Debugging）与状态回滚采用零拷贝二进制序列化（`rkyv` / `postcard`），仅在 Agent 显式请求自省时按需序列化为 JSON Schema。

---

# 8. 热重载与工程工具链

## 8.1 代码级热重载、全局分配器劫持与双模状态迁移

- **C-ABI 物理隔离边界**:
  - `Host`（持有窗口、Vulkan 实例、显存池与主循环）与 `Guest`（编译为 `cdylib` 动态库的业务逻辑）之间严格通过 `#[repr(C)]` 稳定函数指针虚表 `GuestPluginVTable` 通信：

```rust
#[repr(C)]
pub struct HostAllocatorVTable {
    pub alloc: extern "C" fn(size: usize, align: usize) -> *mut u8,
    pub dealloc: extern "C" fn(ptr: *mut u8, size: usize, align: usize),
    pub realloc: extern "C" fn(ptr: *mut u8, old_size: usize, align: usize, new_size: usize) -> *mut u8,
}

#[repr(C)]
pub struct GuestPluginVTable {
    pub abi_version: u32,
    pub state_schema_hash: u64, // 编译期由派生宏计算的 State 内存布局指纹
    pub on_load: extern "C" fn(host_alloc: *const HostAllocatorVTable),
    pub init_state: extern "C" fn() -> *mut core::ffi::c_void,
    pub migrate_export: extern "C" fn(state: *const core::ffi::c_void, out_buf: *mut Vec<u8>),
    pub migrate_import: extern "C" fn(buf_ptr: *const u8, buf_len: usize) -> *mut core::ffi::c_void,
    pub reduce: extern "C" fn(state: *mut core::ffi::c_void, action: *const RawAction),
    pub build_view: extern "C" fn(state: *const core::ffi::c_void, ctx: *mut HostRenderContext),
}
```

- **Host 全局分配器统一劫持 (防止跨 DLL 堆污染)**:
  - 在 `app_guest` 动态库中，通过自定义 `#[global_allocator]` 将 Guest 内部所有的 Rust 堆内存分配（`Box`, `Vec`, `String`, `Arc`）全部转发至 `HostAllocatorVTable` 指向的宿主进程唯一堆分配器（如 `mimalloc`）。
  - 彻底杜绝 Windows 下不同 DLL 拥有独立 CRT 堆导致的跨边界释放崩溃（Heap Corruption）。
- **Schema Hash 双模状态迁移 (兼顾微秒级零拷贝与改结构防崩)**:
  - 派生宏在编译期递归计算业务 `State` 结构体所有字段的名称、类型与偏移量哈希值 `state_schema_hash: u64`。
  - **快速通道（Fast Path，`old_hash == new_hash`）**: 当开发者仅修改了 UI 布局、配色、动画参数或事件处理逻辑而未改动 `State` 结构体字段定义时，直接将旧 `State` 裸指针原封不动移交给新加载的 `Guest.dll`，**热重载状态交接耗时小于 $0.01\text{ms}$（真正零拷贝）**。
  - **安全容错通道（Safe Fallback Path，`old_hash != new_hash`）**: 当开发者增删或修改了 `State` 结构体字段时，自动触发旧 DLL 的 `migrate_export` 导出为带字段标识的自描述紧凑二进制流（MessagePack / FlexBuffers），并由新 DLL 的 `migrate_import` 结合 `#[serde(default)]` 完成字段默认值填充与结构演进，彻底告别修改状态结构体后热重载必崩溃的顽疾。
- **极速链接与 Windows Shadow Copy**:
  - Windows 强制配置 `lld-link`（或 `radlink`），Linux 配置 `mold`；开发期配置 `split-debuginfo = "unpacked"` 与 `debug = "line-tables-only"`。
  - 内置文件监听器检测到 `guest.dll` 编译完成后，先等待所有后台 Rayon 任务栅栏清空，随后自动复制为带递增序号的 `guest_gen_N.dll` 副本并调用 `LoadLibraryW` 重新挂载，彻底绕过 Windows NTFS 对已加载二进制文件和 `.pdb` 符号文件的写锁定。

## 8.2 异步资源流送管线 (Async Asset Streaming)

```
[磁盘 / 网络异步 IO] ──> [Rayon 后台解码线程池] ──> [CPU Staging Ring Buffer]
                    (PNG/JPEG/WebP/KTX2/SVG/Font)            │
                                                             ▼ (独立 DMA 硬件通道)
                                                [Vulkan Dedicated Transfer Queue]
                                                             │
                                                             ▼ (Timeline Semaphore 释放/获取所有权)
                                                [GPU 持久显存池 / Bindless Texture]
```

- **零主线程阻塞**: 图片解码、KTX2 超压缩纹理转码、字体文件解析与大体积 SVG 曲线提取全部在后台线程池完成。
- **跨队列所有权无缝移交**: 使用独立的 Vulkan 专用传输队列（Dedicated Transfer Queue）通过 PCIe DMA 引擎异步上传纹素与顶点数据；上传完毕后通过 Timeline Semaphore 与队列族所有权转移屏障（Queue Family Ownership Transfer）通知主渲染队列，主渲染队列在资源就绪前自动渲染低分辨率占位符或骨架屏，保证 $120\text{FPS}$ 渲染绝对零卡顿（Zero Stutter）。

## 8.3 调试与安全工具链

- **应用内 RenderDoc 编程钩子 (In-App RenderDoc API)**:
  - 运行时动态探测并挂载 `renderdoc.dll` / `librenderdoc.so` API 指针。
  - 支持通过快捷键或 Agent 测试指令在特定 Action 前后自动调用 `StartFrameCapture` / `EndFrameCapture`，并在每个渲染阶段注入 `vkCmdBeginDebugUtilsLabelEXT` 标记，自动断言单帧 MDI DrawCall 数量与显存池碎片率。
- **Vulkan Validation Layers (VVL) CI 自动化守卫**:
  - 在 CI 管线中结合软件光栅化器（Lavapipe / SwiftShader）或 GPU Runner 强制开启全套 VVL（含 GPU-Assisted Validation 与 Synchronization Validation），对 BDA 物理指针越界、描述符竞争及管线屏障缺失实施零容忍拦截。

---

# 9. 架构全景执行数据流 (V2.0 全链路闭环)

```
 ┌────────────────────────────────────────────────────────────────────────────┐
 │                   OS 物理输入 / 原生文件拖拽 / AI Agent 驱动                 │
 └──────────────────────────────────────┬─────────────────────────────────────┘
                                        │
 ┌──────────────────────────────────────▼─────────────────────────────────────┐
 │                          双层拾取与输入路由状态机                            │
 │   • Portal Overlay 逆序检测 -> UI 树双槽位圆角裁剪拾取 -> 松散四叉树空间粗筛    │
 │   • 窄相精确几何判定 (二次贝塞尔解析距离 / 逆变换局部坐标内点判定)               │
 │   • 瞬态交互状态分流 (框选框 / 智能磁吸红线直写帧瞬态环，绕过 Reducer)          │
 └──────────────────────────────────────┬─────────────────────────────────────┘
                                        │ 派发强类型可序列化 Action
 ┌──────────────────────────────────────▼─────────────────────────────────────┐
 │                        状态演进层 (UDF / COW Reducer)                       │
 │   • COW 结构共享状态树更新           • 事务作用域封装与文本输入自动合并         │
 │   • AccessKit 语义树增量同步         • Headless 模式下触发 5 项几何体检断言     │
 └──────────────────┬───────────────────────────────────────┬─────────────────┘
                    │ Level 2/3 布局脏标记                   │ Level 1 外观/动画脏标记
 ┌──────────────────▼───────────────────┐ ┌─────────────────▼─────────────────┐
 │       增量排版引擎 (Taffy 定制)       │ │      独立变换与外观通道 (Spring)    │
 │   • RelayoutBoundary 阻断向根冒泡     │ │   • 物理弹簧插值更新 GpuTransform   │
 │   • Rayon 并行求解独立脏边界子树       │ │   • 动画激活期自动旁路物理像素吸附   │
 │   • VirtualList 环形槽位原地复用       │ │   • 主题切换仅单次更新 4KB 颜色 LUT │
 │   • cosmic-text 双级缓存 + GPU MTSDF │ │   • 完全跳过 Taffy 排版求解         │
 └──────────────────┬───────────────────┘ └─────────────────┬─────────────────┘
                    │                                       │
 ┌──────────────────▼───────────────────────────────────────▼─────────────────┐
 │                   空间粗筛与增量修补 (CPU + GPU 协同阶段)                    │
 │   • CPU 折叠嵌套 AABB 裁剪框，生成双槽位圆角 GpuClipNode                     │
 │   • 三次贝塞尔曲线自适应降阶为二次曲线段并写入持久池 / 瞬态环                  │
 │   • Stage 1 Compute: 执行 Incremental Scatter 将微量脏节点修补至 TLSF 持久池  │
 │   • Stage 2 Compute: 分块相对坐标解算 + 视口剔除 + 拆出大容器 Opaque Core     │
 │   • Stage 3 Compute: GPU OneSweep 64-Bit 基数排序 + 生成 MDI 间接绘制命令    │
 └──────────────────────────────────────┬─────────────────────────────────────┘
                                        │
 ┌──────────────────────────────────────▼─────────────────────────────────────┐
 │                    Vulkan 1.3+ 双通道 Uber-Shader 渲染底座                  │
 │   • Pass A (Opaque Core Pre-Pass): 从前向后仅写 Reverse-Z 深度，建立遮挡掩码  │
 │   • Pass B (Unified Uber-Shader):  从后向前单次 MDI 批量绘制，开启 Early-Z   │
 │     - VS: 分块锚定相对坐标解算 + 逆变换旋转裁剪局部坐标重构 + 二次曲线紧致 OBB  │
 │     - FS: 零 Discard 抗锯齿 + 双槽位圆角求交 + erf 解析阴影 + 感知伽马校正文字 │
 │   • 轻量 Render Graph: 按需调度离屏 TexturePool 处理毛玻璃与图层组透明度      │
 │   • Present Wait 低延迟同步: Input-to-Photon 延迟严格小于 8.3ms              │
 └────────────────────────────────────────────────────────────────────────────┘
```

---

# 10. 阶段演进落地路径与量化验收红线 (Milestones & KPIs)

## 10.1 五阶段工程实施路径

1. **第一阶段 (M1)：Vulkan 1.3 GPU 驱动底座与双通道 Uber-Shader**
   - 搭建 `ash` + Vulkan 1.3 环境（强制开启 BDA、Bindless、Dynamic Rendering、Synchronization2）。
   - 实现 TLSF 持久驻留池（含 Epoch 延迟回收队列）、帧瞬态环与 Compute Incremental Scatter 修补管线。
   - 落地分块锚定相对坐标体系（Chunk-Anchored `f32`）、VS 逆变换任意旋转裁剪、GPU 64-Bit OneSweep 基数排序以及 `Opaque Core Pre-Pass + Unified Uber-Shader`（内置圆角 SDF 与 `erf` 解析解阴影）。
   - 集成应用内 RenderDoc API 钩子与 CI VVL 自动化校验。
2. **第二阶段 (M2)：工业级双图集文字与二次贝塞尔 OBB 矢量引擎**
   - 集成 `cosmic-text` 并实现 Shape / Wrap 双级哈希缓存。
   - 搭建 `Atlas_R8`（支持 $4$ 档亚像素相位）与 `Atlas_RGBA8` 双格式多页 `Texture2DArray` 及帧代际 LRU 驱逐器。
   - 编写 GPU Compute MTSDF 字形轮廓实时生成器与字形感知伽马校正着色器。
   - 实现三次贝塞尔自适应降阶细分算法、VS 紧致有向包围盒（OBB）生成与 FS 数值稳定二次贝塞尔解析解求距。
3. **第三阶段 (M3)：DOD UI 内核、增量排版隔离与虚拟化容器**
   - 构建基于 `SlotMap` 与 `SecondaryMap` 的扁平状态树、三级脏标记分流器与视图代际记忆化机制。
   - 深度集成 Taffy 布局引擎，落地 `RelayoutBoundary` 局部重排拦截与 Rayon 多子树并行求解。
   - 实现 `VirtualList / VirtualTree / VirtualGrid` 环形槽位复用虚拟化容器。
   - 实现独立于 Taffy 的物理弹簧动画引擎、静态像素网格吸附与动态亚像素平滑插值自动切换、全局颜色 Token LUT 零开销主题切换、响应式帧挂起机制以及 Render Graph 毛玻璃离屏池。
4. **第四阶段 (M4)：无限画布空间索引、全栈交互与低延迟窗口同步**
   - 编写基于扁平 SoA 与侵入式链表的连续内存松散四叉树（Loose Quadtree, $k = 0.5$）。
   - 搭建双层拾取管线（含二次曲线与圆角多边形窄相判定）、Pointer Capture、确定性光标栈、Portal 逃逸自适应定位与 OS 原生文件拖拽穿透。
   - 落地视口级智能磁吸（Smart Snapping）与瞬态对齐红线直写环、COW 事务化 Undo/Redo 栈及文本输入自动合并。
   - 接入完整 CJK IME 候选框对齐、富文本多行选区圆角布尔合并，以及 Windows `VK_EXT_swapchain_maintenance1` 零黑边窗口缩放与低延迟呈现。
5. **第五阶段 (M5)：无障碍语义、Agent 1ms 无头验证与极速热重载**
   - 深度绑定 `AccessKit` 增量语义输出。
   - 构建脱离 GPU 的纯内存无头验证器（Headless Harness），实装 $5$ 项自动化几何体检断言（附带 `#[track_caller]` 源码定位）与 `rkyv` 二进制时间旅行快照。
   - 落地 Host-Guest C-ABI 物理隔离架构、Host 全局分配器统一劫持、Windows Shadow Copy 自动重挂载与基于 `state_schema_hash` 的双模状态无缝热迁移。

## 10.2 量化性能验收红线表 (Engineering Acceptance KPIs)

| 考核维度 | 核心测试场景描述 | 量化验收红线指标 (Release / Dev-Opt) |
| :--- | :--- | :--- |
| **同屏海量图元渲染吞吐** | $4\text{K}$ 分辨率下同屏渲染 $100,000$ 个带圆角、边框与解析阴影的画布节点 | GPU 渲染总耗时 $\le 1.5\text{ms}$；主绘制 MDI DrawCall $\le 4$ 次；持久池每帧 PCIe 上传量为 $0\text{ B}$ |
| **矢量连线光栅化性能** | 同屏显示 $10,000$ 条长跨度三次贝塞尔节点连线并连续缩放视口 | 经二次降阶与紧致 OBB 裁剪后，GPU 曲线着色耗时 $\le 0.8\text{ms}$；几何拟合误差 $\le 0.05\text{px}$ |
| **无限画布极端缩放与防抖** | 在世界坐标 $(10^9, 10^9)$ 处，将画布从 $10\%$ 连续缩放至 $6400\%$ | 顶点与边框抖动误差为 $0.0\text{px}$；GPU MTSDF 字形生成耗时 $\le 0.05\text{ms}$；全程无文字模糊跳变 |
| **空间剔除与交互拾取耗时** | 画布总计驻留 $1,000,000$ 个实体，执行单帧视口粗筛、鼠标点选与拖拽磁吸 | 松散四叉树视口粗筛 $\le 0.15\text{ms}$；单次点选 Hit-Test $\le 5\mu\text{s}$；视口内智能磁吸计算 $\le 0.05\text{ms}$ |
| **增量排版与虚拟化滚动** | $100,000$ 行复杂属性列表以最高速连续滚动；局部卡片内部文本高频变更 | 持续稳定 $120\text{FPS}$ 零掉帧；单次局部 `RelayoutBoundary` 排版求解耗时 $\le 0.2\text{ms}$ |
| **主题切换与静止态功耗** | 一键切换全局深浅色主题；停止所有交互与动画进入静止态 | 主题切换 CPU+GPU 总耗时 $\le 0.05\text{ms}$（触发 $0$ 个节点重排）；静止态 CPU 与 GPU 占用率均为 $0.0\%$ |
| **端到端交互响应延迟** | 鼠标高速拖拽画布节点或拖拽调整 Windows 物理窗口边缘大小 | Input-to-Photon 延迟 $\le 8.3\text{ms}$；窗口边缘缩放全程 $120\text{Hz}$ 无黑边、无拉伸闪烁 |
| **Agent 验证与热重载闭环** | 修改业务 UI 代码触发增量编译热重载；运行无头模式执行全套几何体检断言 | 增量编译至画面热更新生效 $\le 600\text{ms}$（未改状态结构时状态交接 $\le 0.01\text{ms}$）；无头单帧验证 $\le 1.0\text{ms}$ |
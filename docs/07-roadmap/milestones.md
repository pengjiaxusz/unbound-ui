# 五阶段工程实施路径

> **实现状态**：`已落地` —— 五个阶段的内容全部进入代码。

## 第一阶段 (M1)：Vulkan 1.3 GPU 驱动底座与双通道 Uber-Shader

- 搭建 `ash` + Vulkan 1.3 环境（强制开启 BDA、Bindless、Dynamic Rendering、
  Synchronization2）。
- 实现 TLSF 持久驻留池（含 Epoch 延迟回收队列）、帧瞬态环与 Compute Incremental
  Scatter 修补管线。
- 落地分块锚定相对坐标体系（Chunk-Anchored `f32`）、VS 逆变换任意旋转裁剪、
  GPU 64-Bit OneSweep 基数排序以及 `Opaque Core Pre-Pass + Unified Uber-Shader`
  （内置圆角 SDF 与 `erf` 解析解阴影）。
- 集成应用内 RenderDoc API 钩子与 CI VVL 自动化校验。

**落地位置**：`unbound-gpu`

## 第二阶段 (M2)：工业级双图集文字与二次贝塞尔 OBB 矢量引擎

- 集成 `cosmic-text` 并实现 Shape / Wrap 双级哈希缓存。
- 搭建 `Atlas_R8`（支持 $4$ 档亚像素相位）与 `Atlas_RGBA8` 双格式多页
  `Texture2DArray` 及帧代际 LRU 驱逐器。
- 编写 GPU Compute MTSDF 字形轮廓实时生成器与字形感知伽马校正着色器。
- 实现三次贝塞尔自适应降阶细分算法、VS 紧致有向包围盒（OBB）生成与 FS 数值
  稳定二次贝塞尔解析解求距。

**落地位置**：`unbound-text`、`unbound-vector`

## 第三阶段 (M3)：DOD UI 内核、增量排版隔离与虚拟化容器

- 构建基于 `SlotMap` 与 `SecondaryMap` 的扁平状态树、三级脏标记分流器与视图
  代际记忆化机制。
- 深度集成 Taffy 布局引擎，落地 `RelayoutBoundary` 局部重排拦截与 Rayon 多子树
  并行求解。
- 实现 `VirtualList / VirtualTree / VirtualGrid` 环形槽位复用虚拟化容器。
- 实现独立于 Taffy 的物理弹簧动画引擎、静态像素网格吸附与动态亚像素平滑插值
  自动切换、全局颜色 Token LUT 零开销主题切换、响应式帧挂起机制以及 Render
  Graph 毛玻璃离屏池。

**落地位置**：`unbound-world`、`unbound-layout`、`unbound-anim`、`unbound-theme`、
`unbound-graph`

## 第四阶段 (M4)：无限画布空间索引、全栈交互与低延迟窗口同步

- 编写基于扁平 SoA 与侵入式链表的连续内存松散四叉树（Loose Quadtree,
  $k = 0.5$）。
- 搭建双层拾取管线（含二次曲线与圆角多边形窄相判定）、Pointer Capture、
  确定性光标栈、Portal 逃逸自适应定位与 OS 原生文件拖拽穿透。
- 落地视口级智能磁吸（Smart Snapping）与瞬态对齐红线直写环、COW 事务化
  Undo/Redo 栈及文本输入自动合并。
- 接入完整 CJK IME 候选框对齐、富文本多行选区圆角布尔合并，以及 Windows
  `VK_EXT_swapchain_maintenance1` 零黑边窗口缩放与低延迟呈现。

**落地位置**：`unbound-canvas`、`unbound-input`、`unbound-platform`

## 第五阶段 (M5)：无障碍语义、Agent 1ms 无头验证与极速热重载

- 深度绑定 `AccessKit` 增量语义输出。
- 构建脱离 GPU 的纯内存无头验证器（Headless Harness），实装 $5$ 项自动化
  几何体检断言（附带 `#[track_caller]` 源码定位）与 `rkyv` 二进制时间旅行
  快照。
- 落地 Host-Guest C-ABI 物理隔离架构、Host 全局分配器统一劫持、Windows Shadow
  Copy 自动重挂载与基于 `state_schema_hash` 的双模状态无缝热迁移。

**落地位置**：`unbound-a11y`、`unbound-headless`、`unbound-host`

## 组件层补强（框架审计后续，M6）

- 在 `unbound-widgets` 落地「扁平描述符 + 纯发射函数」组件契约，覆盖工作区容器、
  导航命令、高阶表单、富文本/数据集合、反馈浮层与无限画布套件（#1–#41）。
- `unbound-gpu` 补齐线性/径向渐变填充、解析内阴影、多级圆角裁剪折叠与外部纹理
  绑定槽。
- `unbound-app` 提供 `AppRuntime` 每帧闭环、`F12` DevTools Inspector 与
  `enable_debug_sanitizers` 的 per-frame 无头体检。

**落地位置**：`unbound-widgets`、`unbound-gpu`、`unbound-app`、`unbound-world`、
`unbound-math`

## 尚未闭环的部分

| 阶段 | 缺口 |
| :--- | :--- |
| M4 | Vulkan surface 与真实 swapchain 未创建，窗口呈现策略已实现但未接真实呈现路径 |
| M4 | IME 预编辑事件流未接入平台层 |
| M1 | 10 万图元场景在 512×512 下计量，文档红线描述的 4K 分辨率未复测 |

详见[窗口呈现](../01-graphics/window-presentation.md)。

---

**相关文档**：[量化验收红线](acceptance-kpis.md) · [crate 地图](../00-overview/crate-map.md)

**返回**：[07-roadmap](README.md) · [文档索引](../README.md)

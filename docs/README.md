# Unbound UI 架构文档

下一代高性能 GPU 驱动 UI 与无限画布引擎的架构蓝图，按主题域拆分为可独立
阅读与维护的目录结构。

> **本文档的状态标注约定**
>
> | 标记 | 含义 |
> | :--- | :--- |
> | `已落地` | 设计已进入代码，并有自动化测试覆盖 |
> | `部分落地` | 核心机制已实现，但存在明确缺口 |
> | `蓝图` | 设计已定义，尚未进入代码 |
>
> 每篇文档开头都有「实现状态」小节，指向对应的 crate、源文件与测试用例。

## 阅读路径

- **第一次了解本项目** → [总览与设计准则](00-overview/principles.md) →
  [端到端数据流](00-overview/dataflow.md) → [里程碑与验收红线](07-roadmap/milestones.md)
- **想先看看它长什么样** → [组件展台](06-tooling/showcase.md)
- **改渲染管线** → [图形技术栈](01-graphics/api-and-features.md) →
  [Uber-Shader](01-graphics/uber-shader.md) → [裁剪](01-graphics/clipping.md) →
  [曲线](01-graphics/curves.md)
- **改 UI 框架** → [增量排版](02-ui/layout.md) →
  [虚拟化](02-ui/virtualization.md) → [动画与像素吸附](02-ui/animation.md)
- **改画布交互** → [空间索引](03-canvas/spatial-index.md) →
  [交互编辑](03-canvas/interaction.md)
- **接入 AI Agent** → [无头验证](05-agent/headless.md) →
  [依赖分层护栏](05-agent/layering.md)

## 目录

### [00-overview 总览](00-overview/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [架构定位与设计准则](00-overview/principles.md) | 性能优先、面向数据、无锁调度、Agent 双模 | `蓝图` |
| [端到端数据流](00-overview/dataflow.md) | 从 OS 输入到 GPU 呈现的全链路闭环 | `已落地` |
| [crate 地图](00-overview/crate-map.md) | 19 个 crate 的职责边界与依赖方向 | `已落地` |

### [01-graphics 图形底座](01-graphics/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [核心 API 与特性开关](01-graphics/api-and-features.md) | Vulkan 1.3、BDA、Bindless、动态渲染 | `已落地` |
| [显存分层与并发安全](01-graphics/memory-hierarchy.md) | 持久池、瞬态环、离屏池、Epoch 回收 | `已落地` |
| [窗口呈现与零黑边](01-graphics/window-presentation.md) | DWM 对齐、present wait、延迟预算 | `部分落地` |
| [GPU 数据结构与 BDA 协议](01-graphics/gpu-data-layout.md) | Push Constants、图元实例、裁剪节点 | `已落地` |
| [Uber-Shader 与双通道深度](01-graphics/uber-shader.md) | 不透明内核预剔除 + 统一 uber pass | `已落地` |
| [双阶裁剪与局部坐标重构](01-graphics/clipping.md) | CPU 折叠 + VS 逆变换 + FS 无 discard | `已落地` |
| [解析解阴影](01-graphics/shadow.md) | erf 近似圆角矩形阴影积分 | `已落地` |
| [曲线与矢量渲染](01-graphics/curves.md) | 降阶细分、紧致 OBB、解析解 SDF | `已落地` |
| [工业级文字渲染](01-graphics/text.md) | 双图集、亚像素相位、GPU MTSDF | `已落地` |

### [02-ui UI 核心范式](02-ui/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [面向数据拓扑](02-ui/dod-model.md) | SlotMap、COW、Action as Data | `已落地` |
| [增量排版引擎](02-ui/layout.md) | RelayoutBoundary、三级脏标记、并行求解 | `已落地` |
| [视口虚拟化](02-ui/virtualization.md) | VirtualList/Tree/Grid 与环形槽位复用 | `已落地` |
| [动画与像素吸附](02-ui/animation.md) | 物理弹簧、亚像素旁路、帧调度 | `已落地` |
| [正交缩放与主题切换](02-ui/theme-and-zoom.md) | 三层缩放、4 KiB 颜色表 | `已落地` |

### [03-canvas 无限画布](03-canvas/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [空间索引与双层剔除](03-canvas/spatial-index.md) | 松散四叉树、GPU 细粒度剔除 | `已落地` |
| [分块锚定相对坐标](03-canvas/chunk-coordinates.md) | 4096 区块、GPU 原位解算 | `已落地` |
| [交互编辑](03-canvas/interaction.md) | 瞬态隔离、智能磁吸、事务化撤销 | `已落地` |

### [04-input 输入与语义](04-input/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [双层拾取系统](04-input/hit-testing.md) | UI 树 + 画布空间粗筛、指针捕获 | `已落地` |
| [IME 与富文本选区](04-input/ime-and-selection.md) | 候选框锚定、圆角选区合并 | `部分落地` |
| [浮层 Portal 与深度排序](04-input/portal-and-sorting.md) | Portal 逃逸、64 位排序键 | `已落地` |
| [无障碍语义](04-input/accessibility.md) | AccessKit 增量语义树 | `已落地` |

### [05-agent Agent 友好](05-agent/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [编译时效与依赖分层](05-agent/layering.md) | 增量编译 < 800 ms、物理分层护栏 | `已落地` |
| [无头闭环验证](05-agent/headless.md) | 五项几何体检、rkyv 时间旅行 | `已落地` |

### [06-tooling 工程工具链](06-tooling/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [热重载与状态迁移](06-tooling/hot-reload.md) | C-ABI 隔离、分配器劫持、双模迁移 | `已落地` |
| [异步资源流送](06-tooling/asset-streaming.md) | 后台解码、暂存环、传输队列交接 | `已落地` |
| [调试与安全工具链](06-tooling/debugging.md) | RenderDoc 钩子、VVL 断言 | `已落地` |
| [组件展台](06-tooling/showcase.md) | 开窗看全所有图元、主题切换、无头出图 | `已落地` |

### [07-roadmap 路线与验收](07-roadmap/README.md)

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [里程碑路径](07-roadmap/milestones.md) | M1–M5 交付内容与落地位置 | `已落地` |
| [量化验收红线](07-roadmap/acceptance-kpis.md) | 性能预算与实测值 | `已落地` |

## 历史版本

原始的单文件蓝图保留在 [`archive/gpu_ui_v2_0.md`](archive/gpu_ui_v2_0.md)，作为
分卷前的完整快照，仅供追溯对照，不再更新。

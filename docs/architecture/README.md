# Unbound UI 架构主索引

> **AI / 开发者查阅指南**：本文件是 Unbound UI 项目的**唯一权威架构主索引**（遵照 `arch-align` 技能规范）。
> 在处理新需求、跨模块重构或定位复杂问题前，必须先查阅对应子系统领域的权威架构文档，对齐架构红线与系统边界后再展开编码。

---

## 1. 全局系统架构与核心骨架

| 权威架构文档 | 核心内容 / 职责范围 | 关联模块 / 源码路径 | 规范级别 |
| :--- | :--- | :--- | :--- |
| [`总览与设计准则`](../00-overview/principles.md) | 性能优先（每帧成本与场景规模无关）、面向数据拓扑、零每帧分配、GPU 端剔除与排序 | 全局架构底座 | 【权威规范】 |
| [`端到端数据流`](../00-overview/dataflow.md) | 从 OS 事件输入、布局排版、GPU 剔除排序到帧呈现的全链路闭环 | 全局闭环数据流 | 【权威规范】 |
| [`Crate 依赖与分层地图`](../00-overview/crate-map.md) | 21 个 crate 与 2 个 app 的职责边界、依赖拓扑与物理隔离护栏 | `crates/*`, `apps/*` | 【权威规范】 |
| [`路线与量化验收红线`](../07-roadmap/acceptance-kpis.md) | 10 万图元同屏、1 万三次贝塞尔连线、100 万实体视口粗筛等量化预算 | `crates/unbound-kpi/` | 【权威规范】 |

---

## 2. 核心子系统与领域模块

| 权威架构文档 | 核心内容 / 职责范围 | 关联模块 / 源码路径 | 规范级别 |
| :--- | :--- | :--- | :--- |
| [`图形底座与 Vulkan 1.3`](../01-graphics/README.md) | Vulkan 1.3、Bindless、BDA 协议、显存池、瞬态环与呈现同步 | `crates/unbound-gpu/` | 【设计实现】 |
| [`Uber-Shader 与双通道深度`](../01-graphics/uber-shader.md) | 不透明内核预剔除 + 统一 uber pass，GPU 64 位基数排序 | `crates/unbound-gpu/` | 【权威规范】 |
| [`矢量曲线与字形渲染`](../01-graphics/curves.md) | 三次贝塞尔自适应降阶、紧致 OBB、解析解 SDF、cosmic-text 双图集与 MTSDF | `crates/unbound-vector/`, `crates/unbound-text/` | 【设计实现】 |
| [`面向数据拓扑与 UI 状态树`](../02-ui/dod-model.md) | SlotMap 扁平状态树、COW、Action as Data、RelayoutBoundary 隔离增量排版 | `crates/unbound-world/`, `crates/unbound-layout/` | 【权威规范】 |
| [`视口虚拟化与弹簧动画`](../02-ui/virtualization.md) | 环形槽位复用、物理弹簧引擎、像素吸附、4 KiB 颜色表与主题正交缩放 | `crates/unbound-anim/`, `crates/unbound-theme/` | 【设计实现】 |
| [`无限画布与空间索引`](../03-canvas/spatial-index.md) | 松散线性四叉树、4096 分块锚定相对坐标、智能磁吸与事务化撤销 | `crates/unbound-canvas/` | 【权威规范】 |
| [`输入、无障碍与 Portal 排序`](../04-input/README.md) | 双层拾取、指针捕获、CJK IME 候选框、AccessKit 增量语义树、Portal 浮层深度逃逸 | `crates/unbound-input/`, `crates/unbound-a11y/` | 【设计实现】 |
| [`无头验证与 Agent 友好环境`](../05-agent/headless.md) | 五项几何体检、rkyv 时间旅行快照、增量编译防卡顿护栏 | `crates/unbound-headless/` | 【权威规范】 |
| [`工程工具链与热重载`](../06-tooling/README.md) | C-ABI 隔离、分配器劫持、双模状态迁移、组件展台验收台 | `crates/unbound-host/`, `apps/showcase/` | 【设计实现】 |

---

## 3. 完整主题域索引主卷

- 完整多卷架构阅读路径可直接参考：[`docs/README.md`](../README.md)

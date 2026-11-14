# 架构索引（智能体路由图）

> 只读与目标路径或任务意图匹配的 1~2 篇规范。
> 标签：`[规范]` = 强制全局契约 | `[模块]` = 子系统规范 | 状态：`[已落地]` | `[部分落地]` | `[蓝图]`

## 任务路由（一跳查找）
- 全局边界、依赖分层与零分配原则 -> `./principles.md`
- 端到端输入/布局/渲染/呈现管道 -> `./dataflow.md`
- 性能红线、KPI 预算与基准门禁 -> `./kpi-budgets.md`
- Vulkan 1.3 渲染管线、显存池、Uber-Shader、裁剪与阴影 -> `./gpu-renderer.md`
- 三次贝塞尔曲线、SVG 路径与 MTSDF 文字渲染 -> `./vector-and-text.md`
- DOD 状态树、Taffy 增量排版与视口虚拟化 -> `./ui-and-layout.md`
- 99 款标准控件、24 状态 Stable Shape 与场景差分 -> `./widgets.md`
- 弹簧物理动画、像素吸附、三层缩放与 4 KiB 主题 LUT -> `./animation-and-theme.md`
- 无限画布四叉树、4096 分块坐标、磁吸撤销与渲染图 -> `./canvas-and-graph.md`
- 双层拾取、焦点管理、IME 选区与 AccessKit 无障碍 -> `./input-and-a11y.md`
- Win32/Winit 窗口、Snap Layouts、对话框、剪贴板与延迟 -> `./platform-and-window.md`
- 无头几何体检、TestApp 与 rkyv 时间旅行快照 -> `./headless-and-agent.md`
- C-ABI 热重载、状态迁移与异步资源流送 -> `./hot-reload-and-assets.md`
- 组件展台 Showcase 与 Gallery 演示入口 -> `./showcase.md`

## 模块地图
- `./principles.md` [规范][已落地] | 路径：`crates/*`、`apps/*` | 依赖分层护栏、扁平 POD、正交极简 API
- `./dataflow.md` [规范][已落地] | 路径：`crates/unbound-app/`、`crates/unbound-abi/` | OS 事件 -> 归约 -> 排版 -> 发射 -> GPU 呈现
- `./kpi-budgets.md` [规范][已落地] | 路径：`crates/unbound-kpi/`、`apps/gallery/` | 10 万图元/万条曲线/百万实体粗筛等红线
- `./gpu-renderer.md` [模块][已落地] | 路径：`crates/unbound-gpu/`、`crates/unbound-math/` | BDA/Bindless、MDI、双通道深度、裁剪、erf 阴影
- `./vector-and-text.md` [模块][已落地] | 路径：`crates/unbound-vector/`、`crates/unbound-text/` | 降阶 OBB、解析 SDF、cosmic-text 双图集、MTSDF
- `./ui-and-layout.md` [模块][已落地] | 路径：`crates/unbound-world/`、`crates/unbound-layout/` | SlotMap、RelayoutBoundary、VirtualList/Tree/Grid
- `./widgets.md` [模块][已落地] | 路径：`crates/unbound-widgets/` | `define_widget_catalog!`、24 状态图元恒定、局部补丁
- `./animation-and-theme.md` [模块][已落地] | 路径：`crates/unbound-anim/`、`crates/unbound-theme/` | 临界阻尼弹簧、亚像素吸附、4 KiB Color LUT
- `./canvas-and-graph.md` [模块][已落地] | 路径：`crates/unbound-canvas/`、`crates/unbound-graph/` | 松散四叉树、事务撤销、RenderGraph 瞬态纹理池
- `./input-and-a11y.md` [模块][部分落地] | 路径：`crates/unbound-input/`、`crates/unbound-a11y/` | `pick_stack`、Portal 排序、IME 锚定、AccessKit
- `./platform-and-window.md` [模块][部分落地] | 路径：`crates/unbound-platform/` | 多窗口事件、Win32 DWM 贴靠、present wait 预算
- `./headless-and-agent.md` [模块][已落地] | 路径：`crates/unbound-headless/`、`crates/unbound-app/` | 5 项几何体检、`Harness`、rkyv 快照回放
- `./hot-reload-and-assets.md` [模块][已落地] | 路径：`crates/unbound-host/`、`crates/app-guest/`、`crates/unbound-asset/` | 分配器劫持、双模迁移、暂存环交接
- `./showcase.md` [模块][已落地] | 路径：`apps/showcase/`、`apps/gallery/` | 21 页自举展台、Knob 调参、无头截图与 KPI 报告

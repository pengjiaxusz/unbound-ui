# 架构：canvas-and-graph
状态：已落地
路径：`crates/unbound-canvas/`、`crates/unbound-graph/`
入口：`crates/unbound-canvas/src/lib.rs::LooseQuadtree`、`crates/unbound-graph/src/lib.rs::RenderGraph`
守卫：`cargo test -p unbound-canvas` / `cargo test -p unbound-graph`

## 范围
- [负责] 无限画布松散线性四叉树空间索引、4096 分块锚定相对坐标、智能磁吸、框选变换、事务化撤销与渲染图瞬态纹理池
- [不负责] UI Flexbox 排版 -> 委托给 `crates/unbound-layout/`；底层 Vulkan 物理显存分配 -> 委托给 `crates/unbound-gpu/`

## 拓扑与流程
- 空间索引：实体写入 `LooseQuadtree`（`MAX_DEPTH`） -> 视口粗筛仅提交相交区块 -> GPU 计算着色器细粒度剔除
- 坐标与交互：`Marquee` 框选 -> `HandleDrag` 拖拽 -> `Snapper::snap` 吸附节点边/中心 -> `UndoStack::push` 提交 `Transaction` 局部撤销
- 渲染图：`RenderGraph` 声明 `Pass` 与 `TextureRole` -> 编译期校验 `Lifetime` -> `OffscreenPool` 复用毛玻璃/高斯模糊瞬态纹理

## 取舍（禁止重构）
- [代价] 画布采用 4096 大小分块锚定相对坐标，由 GPU 原位解算世界偏移 -> [原因] 在 `(10^9, 10^9)` 极端世界坐标下彻底消除 `f32` 精度丢失与顶点抖动 -> [位置] `crates/unbound-canvas/src/`
- [代价] 编辑操作先写入瞬态隔离层再通过 `Transaction` 提交 `UndoStack` -> [原因] 拖拽预览期间不污染持久空间索引与历史栈 -> [位置] `crates/unbound-canvas/src/undo.rs::Transaction`

## 不变量与红线
- [禁止] 在世界坐标 `(10^9, 10^9)` 直接将 `f64` 坐标截断为 `f32` 送入着色器 -> [原因] 导致严重量化抖动 -> [守卫] `crates/unbound-kpi/src/lib.rs::snap`
- [必须] `RenderGraph` 编译时验证所有瞬态 `TextureId` 生命周期无悬空读写 -> [守卫] `crates/unbound-graph/src/graph.rs::CompiledGraph`

# 架构：ui-and-layout
状态：已落地
路径：`crates/unbound-world/`、`crates/unbound-layout/`
入口：`crates/unbound-world/src/lib.rs::NodeData`、`crates/unbound-layout/src/lib.rs::LayoutEngine`
守卫：`cargo test -p unbound-world` / `cargo test -p unbound-layout` / `cargo test -p unbound-app --test layout_runtime`

## 范围
- [负责] 基于 `SlotMap` 的扁平 UI 拓扑树、三级脏标记、COW 状态、Taffy 增量排版（`RelayoutBoundary`）与视口虚拟化容器
- [不负责] 具体控件外观描述符 -> 委托给 `crates/unbound-widgets/`；物理空间索引 -> 委托给 `crates/unbound-canvas/`

## 拓扑与流程
- 状态树：`NodeId`（`SlotMap` 键）索引 `NodeData` 扁平数组，父子兄弟采用侵入式索引链表，消除 `Rc`/`Arc`/虚表指针跳跃
- 增量排版：属性变更触发 `DirtyLevel` 向上冒泡，遇到 `is_relayout_boundary` 节点即截断 -> `LayoutEngine::solve_boundaries` 仅重算脏边界子树
- 虚拟化：`VirtualList` / `VirtualTree` / `VirtualGrid` 维护可见窗口与环形槽位复用，输出 `WindowUpdate` 增量挂载/卸载节点

## 取舍（禁止重构）
- [代价] 使用 `SlotMap` + 侵入式索引替代 `Rc<RefCell<Node>>` 树 -> [原因] 保证 CPU 缓存行连续、零引用计数原子操作且无借用检查器运行时 panic -> [位置] `crates/unbound-world/src/lib.rs::NodeId`
- [代价] 显式标注 `RelayoutBoundary` 隔离局部文本/列表更新 -> [原因] 防止局部微调引发全树 Taffy 重排（单边界求解实测 3.2µs） -> [位置] `crates/unbound-layout/src/lib.rs::LayoutEngine::solve_boundaries`

## 不变量与红线
- [禁止] 10 万行虚拟列表滚动时分配与总行数成正比的 `NodeData` -> [原因] 仅允许活跃视口槽位数量的节点常驻 -> [守卫] `crates/unbound-world/src/virtualize.rs`
- [必须] 所有跨帧节点引用使用带代际校验的 `NodeId`，禁止裸数组下标跨帧持久化 -> [守卫] `crates/unbound-world/src/lib.rs::NodeId`

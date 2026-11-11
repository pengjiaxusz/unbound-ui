# 面向数据拓扑

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-world/src/lib.rs`、`crates/unbound-abi/src/{desc,action}.rs` |
> | **关键类型** | `UiWorld`、`NodeId`、`NodeData`、`RawNodeDesc`、`RawAction`、`DirtyLevel` |

## 实体组织

采用扁平 `SlotMap<NodeId, LayoutNode>` 与按需挂载的 `SecondaryMap<NodeId,
ComponentData>` 替代传统面向对象指针树。

节点数据集中在 `NodeData` 结构体里，组件数据按需挂载——不需要某个组件的节点
不承担那份内存。节点删除后槽位进入代际索引的空闲链，句柄仍指向旧代际因此
不会误命中新节点。

## 单向数据流与零拷贝快照

业务逻辑严格遵循 `State -> Action -> Reducer`，大集合状态采用基于写时复制
（COW，`im` / `rpds` 或自研 Arc-Slice）的结构共享容器。

这带来两个性质：

1. 撤销只需保存**补丁**而非场景副本，见[事务化撤销](../03-canvas/interaction.md)。
2. 快照之间共享底层结构，百万级节点的单步撤销内存增量恒定在数 KB。

## 行为数据化 (Action as Data)

UI 组件事件回调仅允许绑定强类型可序列化 `Enum Action`，严禁在视图树节点中
挂载捕获环境的动态闭包（`Box<dyn Fn>`）。

这条约束是[热重载](../06-tooling/hot-reload.md)与
[无头验证](../05-agent/headless.md) 能成立的前提：状态是纯数据，因此可以被
序列化、迁移、回放。

## 三级脏标记

`DirtyLevel` 定义了脏变的严重程度，排版引擎据此决定工作量：

| 级别 | 含义 | 处理路径 |
| :--- | :--- | :--- |
| `Paint` | 纯外观/变换 | 零排版开销，直通瞬态环或 Compute Scatter |
| `Layout` | 隔离边界内布局 | 只重算最近的 `RelayoutBoundary` |
| `Topology` | 全局拓扑 | 全量重排 |

详见[增量排版引擎](layout.md)。

---

**相关文档**：[增量排版](layout.md) · [热重载](../06-tooling/hot-reload.md)

**返回**：[02-ui](README.md) · [文档索引](../README.md)

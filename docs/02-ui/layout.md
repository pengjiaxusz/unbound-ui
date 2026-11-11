# 增量排版引擎

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-layout/src/lib.rs`、`crates/unbound-world/src/lib.rs` |
> | **关键类型** | `LayoutEngine`、`LayoutStats`、`DirtyLevel` |
> | **测试** | `crates/unbound-layout` 的隔离边界用例；实测单个边界求解 3.2 µs |

## 排版隔离边界 (RelayoutBoundary)

凡满足「宽高明确设定（Fixed / Percentage）且 `overflow != visible`」的容器
（如固定尺寸卡片、滚动容器、无限画布宿主叶节点）自动标记为
`RelayoutBoundary`。

容器内部子组件发生尺寸或文本变动时，脏标记向上回溯至最近的 `RelayoutBoundary`
**即刻截断**，严禁向根节点冒泡；多个独立的脏边界子树通过 Rayon 线程池并行
调用 Taffy 求解。

```
        根节点
          │  ← 脏标记在此被截断，不会继续向上
     ┌────┴────┐
     │         │
  Boundary   Boundary      ← 两者可并行求解
     │         │
  子树 A     子树 B
```

## 视图记忆化 (Memoized Views)

每个组件节点维护 `state_generation: u64`。当父组件重绘而子组件输入状态代际
版本号未变时，直接跳过子树 `view()` 构造函数与排版过程，原地复用持久池中的
GPU 实例区间。

## 三级脏标记分流

| 级别 | 触发条件 | 处理 | 排版开销 |
| :--- | :--- | :--- | :--- |
| Level 1 — 纯外观/变换 | Color / Opacity / Transform | 标记 `dirty_paint_nodes`，直通帧瞬态环或通过 Compute Scatter 修补持久池对应槽位 | **零** |
| Level 2 — 边界内布局 | 隔离边界子树尺寸或文本变动 | 只重算该 `RelayoutBoundary` 子树并增量更新受影响节点的坐标 | 局部 |
| Level 3 — 全局拓扑 | 窗口尺寸变化、顶层分割线拖拽 | 全量重排 | 全局 |

## LayoutEngine 接口

| 方法 | 用途 |
| :--- | :--- |
| `sync(world, root)` | 把世界状态同步进 Taffy 树 |
| `solve(world, root, viewport)` | 全量求解 |
| `solve_boundaries(world, boundaries, viewport)` | 只求解指定的隔离边界子树 |
| `solve_subtree(world, root, viewport)` | 求解单棵子树 |
| `rect(id)` | 读取最终布局矩形 |

## 增量求解的两条不变量

子树是以**边界自己为根**重算的，于是有两处必须显式接回去，否则增量结果与
全量结果对不上：

| 不变量 | 违反时的症状 |
| :--- | :--- |
| **边界自身的位置属于父节点**。重算返回的边界矩形是它在**那次计算里**的坐标（原点），不是它在树里的位置 | 边界被写回后**挪到父节点左上角**——长列表里被重解的那一行会盖住第一行 |
| **裁剪链属于祖先**。子树的*有效*裁剪是所有裁剪祖先的交集，从子树起点开始折叠必须继承父节点已存的裁剪与圆角 | 子树**忘记**祖先的裁剪与圆角——本该被滚动容器/圆角面板裁住的内容溢出到外面 |

第二条由 `UiWorld::fold_clips` 承担：它从父节点已存的 `clip` / `rounded_clips`
继承，而不是从空值开始。

## 实测

单个 `RelayoutBoundary` 求解耗时 **3.2 µs**，预算 0.2 ms。这让"局部重排"在
实践中比"全量重排"便宜两个数量级。

---

**相关文档**：[面向数据拓扑](dod-model.md) · [动画与像素吸附](animation.md)

**返回**：[02-ui](README.md) · [文档索引](../README.md)

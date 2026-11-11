# 无障碍语义体系

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-a11y/src/lib.rs` |
> | **关键类型** | `Semantics`、`A11yNode`、`A11yAction`、`A11yActions` |
> | **依赖** | `accesskit` crate |

## 语义树作为一级输出端

将 `AccessKit` 作为 UI 框架的一级语义输出端，与 `SlotMap` 节点生命周期和增量
脏标记深度绑定。

仅在节点发生状态或布局变更时，增量构建 `accesskit::TreeUpdate` 推送给操作系统
无障碍接口（Windows UI Automation / macOS NSAccessibility / Linux AT-SPI）。

## 双模赋能

同一棵语义树同时服务于两类消费者：

```
                  ┌──────────────────────────┐
                  │      Semantics 树         │
                  │   (A11yNode 层次结构)      │
                  └───────────┬──────────────┘
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
   ┌────────────────────┐         ┌────────────────────┐
   │ 操作系统无障碍接口   │         │   AI Agent 自省      │
   │ (UIA / NSAccessibility│        │ (首选零成本协议)      │
   │  / AT-SPI)          │         │                    │
   └────────────────────┘         └────────────────────┘
```

同时将该语义树直接暴露为 **AI Agent 的首选零成本自省协议**，无需维护第二套
自定义 AST。

这是[Agent 友好设计](../05-agent/README.md)的基石：Agent 不需要理解渲染树，
只需要读语义树。

## 节点与动作

| 类型 | 职责 |
| :--- | :--- |
| `A11yNode` | 单个语义节点：角色、标签、边界、可选动作集 |
| `A11yActions` | 动作位集合（16 位），支持插入/移除/查询 |
| `Semantics` | 语义树的根，持有根节点 ID 与节点表 |

`A11yNode::to_accesskit` 把内部表示转换为 `accesskit::Node`，其中边界使用
`accesskit::Rect::from_origin_size` 转换——语义树直接使用物理屏幕坐标。

## 边界同步

节点边界来自布局结果，因此语义树与视觉呈现**永远一致**——不存在"语义树说按钮
在左边，视觉上在右边"的可能。这是增量推送能成立的前提：只有边界变化才需要
推送更新。

---

**相关文档**：[无头闭环验证](../05-agent/headless.md) · [面向数据拓扑](../02-ui/dod-model.md)

**返回**：[04-input](README.md) · [文档索引](../README.md)

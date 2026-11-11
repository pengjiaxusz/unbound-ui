# 02-ui UI 核心范式

UI 框架层：状态拓扑、增量排版、虚拟化、动画与主题。

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [面向数据拓扑](dod-model.md) | SlotMap、COW、Action as Data | `已落地` |
| [增量排版引擎](layout.md) | RelayoutBoundary、三级脏标记、并行求解 | `已落地` |
| [视口虚拟化](virtualization.md) | VirtualList/Tree/Grid 与环形槽位复用 | `已落地` |
| [动画与像素吸附](animation.md) | 物理弹簧、亚像素旁路、帧调度 | `已落地` |
| [正交缩放与主题切换](theme-and-zoom.md) | 三层缩放、4 KiB 颜色表 | `已落地` |

相关 crate：`unbound-world`、`unbound-layout`、`unbound-anim`、`unbound-theme`。

---

**返回**：[文档索引](../README.md)

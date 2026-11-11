# 03-canvas 无限画布

画布领域引擎：空间索引、坐标体系与交互编辑。

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [空间索引与双层剔除](spatial-index.md) | 松散四叉树、GPU 细粒度剔除 | `已落地` |
| [分块锚定相对坐标](chunk-coordinates.md) | 4096 区块、GPU 原位解算 | `已落地` |
| [交互编辑](interaction.md) | 瞬态隔离、智能磁吸、事务化撤销 | `已落地` |

相关 crate：`unbound-canvas`、`unbound-world`（状态树部分）。

---

**返回**：[文档索引](../README.md)

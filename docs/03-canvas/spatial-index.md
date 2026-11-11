# 空间索引与双层剔除

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-canvas/src/quadtree.rs`、`crates/unbound-gpu/src/frame.rs`（剔除 compute 阶段） |
> | **关键类型** | `LooseQuadtree`、`Element`、`QueryStats` |
> | **实测** | 100 万实体视口粗筛 17.5 µs（预算 0.15 ms） |

## 拒绝固定网格哈希退化

废弃在跨尺度图元（微小图标与巨型分组 Frame 并存）下严重退化的固定尺寸网格
哈希，采用**连续内存数组实现的松散四叉树 (Loose Quadtree)**。

## 松散边界定义

四叉树节点包围盒在其标准几何边界基础上向外扩展 $k$ 倍（取 $k = 0.5$，即宽松度
为 $2\times$），使位于分割线交界处的物体无需切分即可完全落入对应深度的单一
节点内，$O(1)$ 完成层级定位（利用前导零指令 `leading_zeros` 直接算出四叉树
深度与 Morton 码索引）。

最大深度 `MAX_DEPTH = 8`，即 256×256 的块层级。

## 零堆碎片 SoA 数据结构

采用纯扁平数组与侵入式单向链表存储：

```
node_aabbs:      Vec<AABB>
node_children:   Vec<[u32; 4]>
node_element_head: Vec<u32>
element_next:    Vec<u32>
```

元素移动时仅需在链表头尾修改 `u32` 索引，**全程零堆内存分配**。

## CPU + GPU 双层协同剔除

| 层 | 工作 | 成本 |
| :--- | :--- | :--- |
| CPU | 松散四叉树筛出与视口相交的活跃区块与叶节点范围 | < 0.1 ms（实测 17.5 µs @ 100 万实体） |
| GPU | Compute 阶段对活跃区块内图元执行并行视口相交测试与屏占面积计算，剔除小于 $0.5\text{px}$ 的极小图元 | 并行，摊薄 |

这个分工是"成本与场景规模无关"的关键：CPU 的工作量取决于**视口内的区块数**，
而非实体总数；GPU 的工作量取决于**活跃区块内的图元数**。

## 查询接口

| 方法 | 用途 |
| :--- | :--- |
| `insert(entity, aabb, layer)` | 插入实体，分配侵入式链表节点 |
| `remove(entity)` | 移除 |
| `update(entity, aabb)` | 移动实体（链表节点原地复用） |
| `query(query, out)` | 粗筛：返回与区域相交的候选实体 |
| `query_exact(query, out)` | 精确相交测试 |
| `bounds_of(entity)` | 查询实体当前包围盒 |

`QueryStats` 记录查询次数、命中数与命中率，`hit_rate` 用于诊断索引是否退化。

---

**相关文档**：[分块锚定相对坐标](chunk-coordinates.md) · [双层拾取](../04-input/hit-testing.md)

**返回**：[03-canvas](README.md) · [文档索引](../README.md)

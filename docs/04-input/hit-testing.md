# 双层拾取系统与事件捕获

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-input/src/{hit,router,pointer,cursor,dnd}.rs` |
> | **关键类型** | `HitRegion`、`ClipRect`、`Shape`、`Router`、`PointerState`、`CursorStack`、`DragPayload` |
> | **实测** | 单次点选命中 0.8 µs（预算 5 µs） |

```
                         OS 硬件输入 / 原生文件拖拽事件
                                      │
                                      ▼
                        全局浮层 / Portal Overlay 检测
                    (按 Z-Order 逆序遍历顶层 Menu / Modal)
                                      │ (若未命中或穿透)
                                      ▼
                          Taffy UI 树 AABB 递归拾取
                    (结合双槽位圆角 ClipMask 验证有效点击区)
                                      │ (若命中 Canvas 宿主视口叶节点)
                                      ▼
                        松散四叉树画布空间 O(log N) 粗筛
                                      │ (输出粗筛候选列表)
                                      ▼
                          精确几何窄相判定 (Narrow Phase)
                  • 点到二次贝塞尔曲线段解析距离 <= 判定线宽
                  • 逆变换局部坐标点在圆角矩形 / 多边形内判定
```

## 拾取区域模型

`HitRegion` 是可拾取区域的统一描述：

| 字段 | 含义 |
| :--- | :--- |
| `node` | 命中的节点 ID |
| `layer` | 层级（用于浮层优先） |
| `z` | 同层内的 Z 序 |
| `bounds` | 轴对齐包围盒 |
| `shape` | 实际形状（矩形 / 圆角矩形 / 多边形） |
| `clip` | 裁剪矩形（支持圆角） |
| `pass_through` | 是否允许穿透到下层 |

`ClipRect` 既支持普通矩形也支持圆角矩形，`contains` 内部用
`rounded_rect_sdf` 判定。

## 查询层级

| 函数 | 语义 |
| :--- | :--- |
| `pick` | 命中第一个（按 layer/z 排序后最上层） |
| `pick_index` | 同上但返回索引，便于避免借用 |
| `pick_stack` | 返回命中栈（全部层级），供浮层逃逸判定使用 |

## 独占事件捕获 (Pointer Capture)

当激活节点拖拽、画布平移、滚动条滑块拖动或文本选区划选时，自动锁定
`Pointer Capture` 并调用 OS 级鼠标捕获；即使光标移出物理窗口边界，后续运动与
释放事件仍 $100\%$ 独占路由至激活组件。

`PointerState` 维护 `capture()` 与 `pressed()` 两套状态：`pressed` 是按下瞬间
命中的目标，`capture` 是独占路由的目标。

## 确定性光标栈

光标形态按以下优先级覆盖：

```
Active Pointer Capture > 顶层 Overlay > 尺寸调节控制柄 > 画布自定义工具光标
    > 交互式文本 (I-Beam) > 默认箭头
```

`CursorStack` 保证同一输入状态下光标形态是确定的——这让 UI 行为可被自动化
测试断言。

## OS 原生拖拽穿透 (Native Drag-and-Drop)

将操作系统的文件/富媒体拖入事件映射为内部带坐标的瞬态 `DragOver` Action，
直接复用上述双层拾取管线实时高亮目标 UI 槽位或画布投递区域。

`PayloadArena` 用 ID 引用载荷内容，避免在拖拽过程中反复克隆文件列表或文本。

---

**相关文档**：[浮层 Portal 与深度排序](portal-and-sorting.md) · [空间索引](../03-canvas/spatial-index.md)

**返回**：[04-input](README.md) · [文档索引](../README.md)

# 曲线与矢量渲染

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-vector/src/curve.rs`、`crates/unbound-gpu/src/curve.rs` |
> | **关键类型** | `QuadSegment`、`Obb`、`CurveInstance`、`CurveDraw`、`cubic_to_draws` |
> | **测试** | `crates/unbound-vector` 降阶精度用例、`crates/unbound-gpu/src/frame.rs` 的解析光栅化像素断言 |

彻底放弃高 CPU 消耗且缩放需重新细分的离线三角化方案（如 `lyon`）。

## 三次贝塞尔自适应降阶为二次曲线段

点到三次贝塞尔曲线的距离等价于求解五次多项式方程（代数上无解析求根公式，
片元牛顿迭代算力开销极大）。所有三次贝塞尔连线在写入持久缓冲或通过前置
Compute Shader 处理时，利用 Wang's Formula 自适应细分为 $2 \sim 6$ 段**二次
贝塞尔曲线**（拟合几何误差恒小于 $0.05\text{px}$）。

```rust
pub fn cubic_to_quads(
    p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2,
    tolerance: f32,
) -> Vec<QuadSegment>
```

## 紧致有向包围盒 (OBB)

废弃轴对齐粗包围盒（对角线长连线的 AABB 有 $80\%$ 以上面积为空白像素）。

在 VS 阶段沿着每段二次贝塞尔曲线的首尾弦向量方向生成**紧致有向包围盒
（OBB，宽度仅为控制多边形拱高加上线宽与抗锯齿边缘）**，将光栅化无效片元
数量降低 $80\%$ 以上。

OBB 通过**变换矩阵旋转**实现：图元实例仍是轴对齐的，但携带一个旋转矩阵，
VS 逆变换局部坐标重构（见[双阶裁剪](clipping.md)）能正确处理这一情形。

## 数值稳定二次贝塞尔解析解

在 FS 阶段采用消除浮点灾难性抵消（Catastrophic Cancellation）的改进卡尔达诺
解析算法，实时解算像素点到二次贝塞尔曲线的精确代数距离，支持无限缩放而显存
占用恒定为 $O(1)$。

CPU 侧提供了对应的窄相判定函数，用于点击命中：

| 函数 | 用途 |
| :--- | :--- |
| `closest_on_quad` | 最近点与参数 t |
| `distance_to_quad` | 精确距离 |
| `hit_quad` | 带阈值命中判定 |

## 实测

| 场景 | 预算 | 实测 |
| :--- | ---: | ---: |
| 1 万条三次贝塞尔连线 | ≤ 0.8 ms | **0.52 ms**（12.4 万段） |
| 几何拟合误差 | ≤ 0.05 px | 满足 |

> 计量口径为文档场景：连接线散布在大画布上、视口只显示少数。若让 1 万条曲线
> 全部同屏可见，片元成本约 31 ms。

---

**相关文档**：[Uber-Shader](uber-shader.md) · [双阶裁剪](clipping.md) · [双层拾取](../04-input/hit-testing.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

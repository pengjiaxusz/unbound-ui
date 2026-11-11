# 双阶零开销裁剪与逆变换局部坐标重构

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/shaders/uber.vert`（第 4、5 步）、`uber.frag`、`crates/unbound-gpu/src/data.rs`（`GpuClipNode`） |
> | **关键类型** | `GpuClipNode`、`RoundedClip` |
> | **测试** | `crates/unbound-gpu/src/frame.rs` 的裁剪与曲线像素断言 |

## CPU 端 AABB 预求交

扁平化遍历父子裁剪层次，在 CPU 端将所有嵌套轴对齐 AABB 矩形交并折叠为唯一
一个 `vec4(min_x, min_y, max_x, max_y)` 硬裁剪框，写入 `GpuClipNode`。

GPU 端因此只需处理**一个**硬裁剪框和**两个**父级圆角槽位，与嵌套深度无关。

## 顶点着色器逆变换局部坐标重构

### 完全视口外坍缩

若图元屏幕外接包围盒与 `aabb_clip` 交集为空（`min_x >= max_x` 或
`min_y >= max_y`），在 VS 中直接将 4 个顶点全部赋值为 `(0, 0, 0, 0)`，由硬件
图元装配阶段零开销剔除。

### 消除旋转畸变与 UV 拉伸

当图元携带旋转/斜切变换矩阵 $M$ 且与父级轴对齐 `aabb_clip` 发生部分相交时，
**严禁**直接钳制旋转后的顶点（否则平行四边形会扭曲成不规则多边形）。

VS 先计算图元变换后 4 个顶点的屏幕外接 AABB 并钳制到 `aabb_clip` 内得到屏幕
裁剪顶点 $P_{\text{screen}}$；随后在 VS 中乘以图元变换矩阵的逆矩阵 $M^{-1}$
重构出对应的图元局部坐标 $P_{\text{local}} = M^{-1} \cdot P_{\text{screen}}$，
并以 `noperspective` 插值传入 FS。

```glsl
// ---- 5. inverse local reconstruction ----
mat2 inv_m = inverse2x2(m);
```

## 片元着色器废除 discard 与双槽位嵌套圆角

- 全面废除 FS `discard` 指令，保持硬件 Early-Z 管线畅通。
- FS 接收 `P_local`，通过有向距离场（SDF）配合 `smoothstep` 计算图元自身边界
  与自身圆角 `corner_radii` 的抗锯齿覆盖率 $\alpha_{\text{self}}$。
- 同时读取 `GpuClipNode` 中的两个父级圆角裁剪槽位 `rounded_clip_0` 与
  `rounded_clip_1`，执行两次极轻量 SDF Box 求交：

$$\alpha_{\text{final}} = \alpha_{\text{self}} \times \min(\alpha_{\text{clip0}}, \alpha_{\text{clip1}})$$

零离屏模板缓冲（Stencil Buffer）开销即可完美处理高达两层嵌套圆角容器与自身
圆角的任意复合裁剪。

## 三级裁剪的分工

| 级别 | 执行位置 | 输入 | 成本 |
| :--- | :--- | :--- | :--- |
| 层级 1 | CPU | 嵌套 AABB 交并折叠 | 每帧一次遍历 |
| 层级 2 | VS | 屏幕 AABB 钳制 + 逆变换重构 | 4 顶点 × 1 次逆矩阵 |
| 层级 3 | FS | 自身圆角 SDF × 父级圆角 SDF | 每片元 3 次距离求值 |

没有任何一级依赖模板缓冲或离屏 RT。

---

**相关文档**：[Uber-Shader](uber-shader.md) · [BDA 数据布局](gpu-data-layout.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

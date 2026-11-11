# 解析解阴影与内阴影

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/shaders/uber.frag`（shadow 分支、`box_shadow_alpha`）、`common.glsl`（`SHADOW_TOKEN`） |
> | **相关字段** | `GpuPrimitiveInstance::shadow_blur`（f16，0 表示无阴影） |

严禁使用离屏高斯模糊渲染常规 UI 卡片或画布节点的投影（Drop Shadow）与内阴影
（Inner Shadow）。

## erf 近似积分算法

在 Uber-Shader 中内置基于误差函数近似（Error Function $\text{erf}(x)$
Approximation）的**解析圆角矩形阴影积分算法**：

- 仅需将阴影图元尺寸向外扩展 $3 \times \text{shadow\_blur}$；
- 在单次 Quad 片元着色中直接用解析公式计算出高斯卷积后的精确 Alpha 衰减值；
- 实现同屏上万个带柔和阴影节点**零离屏 RT 切换、零额外带宽消耗**。

```glsl
// ---- shadow ----------------------------------------------------------
float shadow_alpha = 0.0;
if (v_shadow > 0.0) {
    float sigma = max(v_shadow * 0.5, 0.5);
    shadow_alpha = box_shadow_alpha(sd, sigma) * theme_color(SHADOW_TOKEN).a;
    // Never darken the interior: the fill composites on top, but a shadow
    // behind it must not bleed through.
}
```

阴影色是**主题级**的概念——所有图元共用一份——所以它不占图元实例的字段，而是
固定住在颜色表的第 1 槽（`common.glsl` 里的 `SHADOW_TOKEN`）。第 0 槽恒为
全透明，保留给"无颜色"用。这样填颜色表的一方必须遵守这条约定：第 1 槽放阴影
色，第 0 槽放透明。

## 局部坐标系：扩张量不许再减一次

顶点着色器为了容纳阴影，把 Quad 向外扩张了 $3 \times \text{shadow\_blur}$，
于是片元着色器拿到的局部坐标 $v_{local}$ 的范围是
$[-\text{pad},\ \text{size} + \text{pad}]$，而**盒子本身仍然占据
$[0,\ \text{size}]$**——局部坐标的原点就在盒子的角上。

因此阴影的距离场直接用 $v_{local}$ 求：

```glsl
vec2 p = v_local;   // 不是 v_local - pad
```

早期实现多减了一次 `pad`，把整个图元平移了阴影半径那么远。阴影越大的图元
偏得越远，看起来就像"图元被顶下去了"。

## 为什么这比离屏模糊便宜

| 方案 | 每阴影成本 | 带宽 | 额外 RT |
| :--- | :--- | :--- | :--- |
| 离屏高斯模糊 | 多次全屏 pass | 每帧读回全屏 | 需要 |
| erf 解析解 | 1 次 SDF 求值 | 0 | 不需要 |

阴影与填充共用同一个 Quad、同一趟 uber pass，不引入任何管线状态切换。

## 内外阴影

外阴影（Drop Shadow）由阴影图元在填充图元之前绘制实现；内阴影（Inner
Shadow）通过反向使用同一套距离场（比较内部区域与圆角边界）实现。两者都复用
`box_shadow_alpha`，因此新增一种阴影形态不需要新增 shader 变体。

## 参数编码

`shadow_blur` 以 f16 存在图元实例里，半径为 0 即"无阴影"，着色器直接跳过
该分支。这是零成本的——绝大多数 UI 节点没有阴影。

---

**相关文档**：[Uber-Shader](uber-shader.md) · [双阶裁剪](clipping.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

# Uber-Shader 与双通道深度合作

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/src/pipeline.rs`、`crates/unbound-gpu/shaders/uber.{vert,frag}`、`opaque_core.{vert,frag}` |
> | **关键类型** | `Pipelines::uber`、`Pipelines::opaque_core`、`PrimType` |
> | **测试** | `crates/unbound-gpu/src/frame.rs` 的单四边形像素断言；`apps/showcase` 的整屏出图 |

为彻底消除管线状态切换（PSO Switch）并同时获得硬件 Early-Z 遮挡剔除与完美的
半透明边缘混合，渲染管线采用 **Uber-Shader 单一管线 + 不透明内核预剔除双通道
（Opaque Core Pre-Pass + Blended Uber-Pass）**。

## 零材质切换 Uber-Shader

既然已具备 BDA + Bindless + `Texture2DArray`，标准矩形、圆角边框、解析解内外
阴影、图片、字形、二次贝塞尔曲线全部合并进**同一个 Uber-Shader 管线**。

通过 `prim_type` 执行分支：由于同一个 Quad 的所有片元必然执行完全相同的分支
路径，GPU Subgroup/Warp 内的分支发散率恒为 $0\%$。

一个 uber pass 支持的图元类型：

| 类型 | 说明 |
| :--- | :--- |
| 矩形 | 基础填充与圆角 |
| 边框 | 解析解描边 |
| 阴影 | erf 近似，见[解析解阴影](shadow.md) |
| 图片 | Bindless 纹理采样 |
| 字形 | MTSDF 图集采样，见[工业级文字](text.md) |
| 曲线 | 二次贝塞尔解析距离，见[曲线与矢量](curves.md) |

## 双阶段深度合作机制

### Pass A — Opaque Core Pre-Pass（从前向后）

针对大面积不透明容器（如背景面板、窗口卡片、画布节点背景，满足
`opacity == 1.0`），GPU Compute 阶段自动将其 AABB 向内收缩最大圆角半径 $R$，
生成纯不透明内核矩形（Opaque Core）。按**从前向后**顺序仅向 Depth Buffer
（采用 Reverse-Z，`VK_COMPARE_OP_GREATER_OR_EQUAL`）写入深度值，不输出颜色，
极速建立硬件遮挡掩码。

内核矩形由**预通道顶点着色器**现场算出，规则有两条，缺一不可：

| 规则 | 内容 | 不这么做会怎样 |
| :--- | :--- | :--- |
| 只有不透明容器有内核 | 圆角矩形、图元不透明度为 1、主题填充色的 alpha 为 1、未被裁剪、面积不小于 64×64 | 半透明图元也会写深度，它**背后**的不透明像素会被深度测试拒绝，画家算法直接失效 |
| 内核 = 盒子按最大圆角半径 $R$ 向内收缩 | 圆角外沿不进入深度缓冲 | 圆角边缘的抗锯齿混合被深度测试挡掉 |

不满足条件的图元在预通道里退化为一个退化四边形，**不产生任何片元**，因此
不写深度。uber 通道随后仍然完整地绘制它们。

### Pass B — Unified Uber-Shader Pass（从后向前）

按拓扑 Z 序**从后向前**执行单次 MDI 批量绘制，开启
`depthTestEnable = true, depthWriteEnable = false`。被顶层大面板完全遮挡的底层
成千上万个图元直接被硬件 Early-Z / Hi-Z 拦截丢弃（零片元着色开销），而可见
图元的抗锯齿圆角边缘、文字与半透明阴影则按严格画家算法完美混合。

```
        ┌─────────────────────────────┐
        │      Pass A: 预剔除          │
        │  仅写 Reverse-Z 深度          │  从前向后
        │  大面板建立遮挡掩码            │
        └──────────────┬──────────────┘
                       │  底层图元被 Early-Z 丢弃
        ┌──────────────▼──────────────┐
        │      Pass B: 统一绘制        │
        │  单次 MDI，从后向前            │
        │  depthTest 开启 / write 关闭   │
        └─────────────────────────────┘
```

## 排序与间接命令生成

渲染前有三个 Compute 阶段：

| 阶段 | 职责 |
| :--- | :--- |
| Stage 1 | Incremental Scatter：把微量脏节点修补进 TLSF 持久池 |
| Stage 2 | 分块相对坐标解算 + 视口剔除 + 拆出大容器 Opaque Core |
| Stage 3 | 64 位基数排序 + 生成 MDI 间接绘制命令 |

Stage 3 之后整个场景塌缩为极少数 `vkCmdDrawIndirectCount` 调用。10 万图元
实测为 **1 次**绘制调用，见[验收红线](../07-roadmap/acceptance-kpis.md)。

> 早期实现中 `indirect.comp` 用单线程串行扫描 10 万个排序键，整帧耗时
> 12.46 ms；并行化后降至 0.67 ms。

## 实测

| 场景 | 耗时 | 绘制调用 |
| :--- | ---: | ---: |
| 10 万图元同屏（512×512） | 0.67 ms | 1 |
| 1 万条三次贝塞尔连线 | 0.52 ms | 1 |

---

**相关文档**：[双阶裁剪](clipping.md) · [BDA 数据布局](gpu-data-layout.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

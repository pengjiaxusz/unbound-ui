# 正交缩放与零开销主题切换

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-theme/src/{zoom,lib,snapping}.rs` |
> | **关键类型** | `ZoomLayers`、`ThemeState`、`ThemeLut`、`ThemeKind`、`Token`、`SwitchOutcome` |
> | **测试** | `crates/unbound-theme` 的令牌与切换用例；`crates/unbound-gpu` 的主题切换像素断言 |

## 三层缩放严格正交分离

| 层 | 控制什么 | 影响范围 |
| :--- | :--- | :--- |
| 系统物理 DPI (`OS Scale`) | Swapchain 物理分辨率、高精文字图集光栅化字号、静态 $1\text{px}$ 物理像素网格吸附基准 | 全局 |
| 应用全局 UI 缩放 (`App Zoom`) | Taffy 样式逻辑尺寸基准与标准控件字号 | 外围面板与工具栏 |
| 画布摄像机缩放 (`Canvas Zoom`) | 纯 GPU 视图投影矩阵与分块坐标变换 | 画布视口内部的图元与 MTSDF 字形采样 |

三者**物理隔离**：画布缩放不改变 UI 树的任何尺寸，UI 缩放不改变画布的坐标
体系，DPI 缩放不改变逻辑布局。

`ZoomLayers` 是这三层的显式载体。

## 零开销深浅色主题切换

### 语义设计令牌

所有颜色都以语义化 Design Token 表达，而不是硬编码 RGB。Token 是 16 位索引
（`Token(u16)`），存在图元实例的 `fill_token_or_uv` 字段里。

### 颜色查找表

Token 解析为颜色靠一张全局扁平查找表 `theme_lut_bda`，大小通常小于
$4\text{KB}$。

切换亮色 / 暗色 / 高对比度主题时，仅需向 GPU 单次写入更新 $4\text{KB}$ 颜色
表缓冲，**完全阻断 Taffy 重新排版与图元实例重传**。

### 切换语义被显式建模

`ThemeState::switch` 返回一个 `SwitchOutcome`，它把"这次切换到底做了什么"
变成可断言的数据：

| 字段 | 含义 |
| :--- | :--- |
| `bytes_uploaded` | 上传的字节数，应为 4096 |
| `relayout_required` | 是否需要重排，必须为 `false` |
| `instance_reupload_required` | 是否需要重传实例，必须为 `false` |

这三个字段被 GPU 集成测试直接断言——主题切换的正确性不是靠"看起来没卡"判断的，
而是靠"上传量恰为 4 KiB 且重排次数为零"判断的。

## 实测

| 指标 | 预算 | 实测 |
| :--- | ---: | ---: |
| 主题切换总耗时 | ≤ 0.05 ms | 满足（4 KiB 单次写入） |
| 触发重排的节点数 | 0 | **0** |
| 触发重传的实例数 | 0 | **0** |

## 颜色空间

主题调色板定义在感知均匀空间（Oklab），保证亮色与暗色主题切换时同一 Token
的感知亮度变化一致。半透明叠加与边缘抗锯齿在着色器里以线性光强混合，消除暗边。

---

**相关文档**：[BDA 数据布局](../01-graphics/gpu-data-layout.md) · [动画与像素吸附](animation.md)

**返回**：[02-ui](README.md) · [文档索引](../README.md)

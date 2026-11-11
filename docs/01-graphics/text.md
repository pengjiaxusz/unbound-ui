# 工业级文字渲染

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-text/src/{shaping,atlas,glyph,sdf}.rs`、`crates/unbound-gpu/src/sdf.rs` |
> | **关键类型** | `Shaper`、`GlyphAtlas`、`Atlas`、`GlyphCache`、`SdfPipeline` |
> | **测试** | `crates/unbound-gpu/src/sdf.rs` 的 MTSDF 计时用例（预算 0.05 ms） |

## 排版塑形双级缓存

| 级别 | 缓存键 | 命中内容 |
| :--- | :--- | :--- |
| 第一级 | 字形度量哈希 | 单行标签、按钮与固定短语的 Shape 结果 |
| 第二级 | 字形度量哈希 + 折行宽度 | 带折行宽度约束的段落 Wrap 结果 |

两级缓存均以 `Arc` 共享，命中时零分配。缓存条目数超过上限时整体清空，避免
LRU 链表带来的额外开销——塑形结果本身计算昂贵，偶尔重算是可接受的。

## 双格式分离多页动态字形图集

避免因兼容彩色 Emoji 而将全部字形升级为 4 通道导致显存暴增 $4$ 倍，系统维护
两个独立的 `Texture2DArray`：

| 图集 | 格式 | 用途 | 亚像素相位 |
| :--- | :--- | :--- | :--- |
| `Atlas_R8` | `VK_FORMAT_R8_UNORM` | 标准 UI 控件高精度灰度字形 | **4 档**（0.0 / 0.25 / 0.5 / 0.75） |
| `Atlas_RGBA8` | `VK_FORMAT_R8G8B8A8_UNORM` | 彩色 Emoji 与 MTSDF 字形 | 无 |

单层 $2048 \times 2048$，利用 Bindless 动态扩容层数。

### 帧代际 LRU 驱逐策略

每个图集槽位记录 `last_accessed_frame`，当图集层满时原地覆盖长期未引用的冷
字形槽位。

## GPU Compute 直接生成 MTSDF 字形

放弃缓慢的 CPU 端 MSDF 生成算法。无限画布文字通过 `skrifa` 直接提取字形轮廓
二次曲线段上传至 Staging Buffer，由**GPU Compute Shader 并行光栅化生成 MTSDF
图集槽位**。

画布从 $10\%$ 到 $6400\%$ 连续缩放时全程保持边缘刀锋般锐利，彻底消除缩放停顿
时的异步重绘跳变（Pop-in）。

### 实测

单个 $64 \times 64$ 字形块的 MTSDF 生成耗时 **0.0495 ms**（预算 0.05 ms）。

> 早期实现把距离场计算与 RGBA 打包放在同一个 compute shader 中，打包阶段的
> 寄存器压力拖慢了整条流水线。拆成 `mtsdf.comp` + `mtsdf_pack.comp` 两个通道
> 后达标。

## 感知伽马校正 (Perceived Text Gamma Correction)

在 Uber-Shader 字形分支中注入基于背景感知亮度的字干粗细补偿（Stem
Darkening）与伽马校正算法：亮色模式（白底黑字）下微调收缩边缘 Alpha，暗色模式
（黑底白字）下适度补偿字体骨架，彻底消除黑白颠倒时的视觉发虚与膨胀现象。

```glsl
// Perceived-gamma compensation: dark text on a light surface needs a
// slightly thinner stem, light text on a dark surface a slightly thicker
```

## 与其余系统的衔接

| 需求 | 落点 |
| :--- | :--- |
| 字形随摄像机缩放保持锐利 | MTSDF 而非位图缩放 |
| 主题切换影响文字颜色 | 颜色走 Token → [4 KiB LUT](../02-ui/theme-and-zoom.md) |
| 文本命中判定 | [双层拾取](../04-input/hit-testing.md) 的窄相判定 |
| 选区几何合并 | [IME 与富文本选区](../04-input/ime-and-selection.md) |

---

**相关文档**：[Uber-Shader](uber-shader.md) · [主题切换](../02-ui/theme-and-zoom.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

# 浮层 Portal 逃逸与 64 位深度键基数排序

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-world/src/lib.rs`（Portal 逃逸）、`crates/unbound-gpu/src/sort.rs`（排序键与基数排序） |
> | **关键类型** | `SortKey`、`bits`、`PassLayer`、`SubElement`、`build_indirect_runs` |

## Portal 逃逸与视口自适应防溢出

- 下拉菜单（Dropdown）、气泡提示（Tooltip）、上下文菜单挂载在逻辑组件树任意
  深处，但通过 `Portal` 机制直接投递至根级 Overlay 图元层，天然免除父级容器的
  `GpuClipNode` 裁剪。
- 内置 `Flip / Shift / Size` 锚定定位求解器，当弹窗靠近物理窗口边缘时自动翻转
  展开方向或微调偏移，确保 $100\%$ 处于物理窗口可见范围内。

世界状态树中，标记为 `NodeTag::Portal` 的节点会**跳过所有祖先裁剪**——它的裁剪
栈在插入时就被截断了，而不是在渲染时逐层判断。

## 改良版 64 位深度排序键

得益于 Unified Uber-Shader 消除了材质切换，64 位无符号整型排序键专职服务于
**渲染阶段屏障隔离**与**严格拓扑遮挡顺序**：

| 位域范围 | 字段 | 位宽 | 编码规则与作用 |
| :--- | :--- | :--- | :--- |
| **Bits 63..56** | `Pass / Portal Layer` | 8-bit | `0`=无限画布层，`1`=主 UI 面板层，`2..254`=Portal 模态弹窗与菜单层，`255`=顶层诊断调试层 |
| **Bits 55..40** | `RenderGraph Barrier ID` | 16-bit | 常规图元保持不变；仅当遇到背景毛玻璃或图层组透明度合成时递增，指示命令流插入离屏 RT 切换屏障 |
| **Bits 39..36** | `Pipeline Variant` | 4-bit | `0`=标准 Unified Uber-Shader（覆盖 $99.9\%$ 图元），`1..15`=用户自定义扩展着色器变体 |
| **Bits 35..8** | `Topological Z-Order` | 28-bit | 支持高达 $2.68$ 亿层的严格前后拓扑遮挡顺序 |
| **Bits 7..0** | `Sub-Element Order` | 8-bit | 同一组件内部子图元固定次序 |

### 子图元次序

| 值 | 元素 |
| :---: | :--- |
| 0 | 外阴影 |
| 1 | 背景填充 |
| 2 | 内容 / 图像 / 文本 |
| 3 | 内阴影 |
| 4 | 外边框 |
| 5 | 焦点环 |

这个次序保证了"阴影在背景之下、边框在内容之上"，不需要任何额外排序逻辑。

### 位域常量

```rust
pub mod bits {
    pub const PASS_SHIFT: u32 = 56;      // 8-bit
    pub const BARRIER_SHIFT: u32 = 40;   // 16-bit
    pub const VARIANT_SHIFT: u32 = 36;   // 4-bit
    pub const Z_SHIFT: u32 = 8;          // 28-bit
    pub const MAX_Z: u32 = 0x0fff_ffff;  // 268 M layers
}
```

`SortKey::new` 在构造时对每个字段做钳制，因此超范围的输入不会静默溢出到相邻
位域。

## GPU 基数排序与间接命令生成

在 GPU Compute Shader 中执行单次 **64 位基数排序（Radix Sort）**，随后通过
Prefix-Sum 自动合并具有相同 `RenderGraph Barrier ID` 与 `Pipeline Variant` 的
连续区间，生成极少数 `VkDrawIndirectCommand`。

`run_id()` 把 `(layer, barrier, variant)` 三元组压成一个整数——**相同 run_id 的
相邻图元可以合并成一次间接绘制**。

### CPU 参考实现

`radix_sort_u64` 是 CPU 侧的 LSD 基数排序参考实现，稳定、$O(8n)$、分配有界
（一个暂存向量）。它的用途有两个：

1. 校验 GPU 实现的正确性；
2. 在设备缺少 `shaderInt64` 时作为降级路径。

---

**相关文档**：[Uber-Shader](../01-graphics/uber-shader.md) · [双层拾取](hit-testing.md)

**返回**：[04-input](README.md) · [文档索引](../README.md)

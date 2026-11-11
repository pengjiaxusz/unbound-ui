# 动画引擎与像素吸附

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-anim/src/lib.rs`、`crates/unbound-theme/src/snapping.rs` |
> | **关键类型** | `Spring`、`Spring2`、`CubicBezier`、`Animator`、`FrameScheduler`、`Channel` |

```
                    ┌───────────────────────────┐
                    │     UI 状态 / Action      │
                    └─────────────┬─────────────┘
                                  │
                 ┌────────────────┴────────────────┐
                 ▼                                 ▼
        [布局几何相关属性]                 [变换与外观独立通道]
     • 宽高 / Margin / Padding         • Transform (平移/缩放/旋转/斜切)
     • Flex / Grid 约束规则            • Opacity / Color Token ID
                 │                                 │
                 ▼                                 ▼
          Taffy 增量排版求解               物理弹簧插值器 (Spring Engine)
       (仅在尺寸脏变时局部触发)              (完全跳过 Taffy，直写瞬态环)
                 │                                 │
                 ▼                                 ▼
     [静态态: 强制物理像素网格吸附]      [动画激活态: 自动旁路吸附，亚像素平滑插值]
                 │                                 │
                 └────────────────┬────────────────┘
                                  ▼
                     合入 GPU BDA 实例数据并渲染
```

## 属性通道物理分离

`Transform`（2D 仿射变换）与 `Opacity`（透明度）作为独立 GPU 通道存储在
`transforms_bda` 中，**绝不进入 Taffy 布局管线**。

物理弹簧动画引擎与贝塞尔曲线插值器每帧仅更新活跃节点的 `GpuTransform` 槽位，
维持持续 $120\text{FPS}$ 渲染而不引发任何排版计算。

动画通道由 `Channel` 枚举区分（位置、缩放、旋转等），每帧只写变化的通道。

## 静态像素吸附与动态亚像素平滑切换

### 静态清晰度保障

静止状态的 UI 节点（`transform_idx` 最高位为 $0$）在 VS 中强制将坐标与宽高
对齐至物理像素整数网格（`round(pos * dpi) / dpi`），确保 $1\text{px}$ 边框绝对
锐利，杜绝半像素模糊。

### 动态丝滑度保障

一旦节点处于弹簧动画或拖拽平滑过渡中（`transform_idx` 最高位标记为 $1$），
VS 自动旁路整数像素吸附，进入全精度浮点亚像素插值模式，彻底消除动画减速收尾
阶段的"阶梯跳动感（Pixel Crawl）"；动画收敛静止的最后一帧平滑恢复像素吸附。

判定入口是一个纯函数，便于测试：

```rust
pub fn should_snap(animating: bool, transform_is_identity: bool) -> bool
```

## 吸附误差量化

`unbound-theme/src/snapping.rs` 提供吸附误差与抖动的量化函数：

| 函数 | 用途 |
| :--- | :--- |
| `snap_screen` | 屏幕空间像素吸附 |
| `edge_screen` | 边到边的屏幕距离 |
| `snap_error` | 吸附前后的位置偏差 |
| `jitter` | 缩放过程中的像素抖动量 |

在 6400% 缩放下，摄像机移动一个像素对应 $1/64$ 个世界单位，抖动误差仍可被
精确度量。

## 响应式帧调度 (Reactive Frame Pacing)

系统全局维护 `active_animations_count` 与输入事件队列。当计数器归零且无待处理
事件时，主循环主动挂起（`WaitMessage` / `epoll_wait`），CPU 与 GPU 占用率瞬时
归零（$0.0\%$）。

`FrameScheduler` 以 `WakeReason` 表达唤醒原因，区分"有新输入"、"有活跃动画"
与"外部请求重绘"。

---

**相关文档**：[增量排版](layout.md) · [正交缩放与主题切换](theme-and-zoom.md)

**返回**：[02-ui](README.md) · [文档索引](../README.md)

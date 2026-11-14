# IME、富文本选区与光标栈

> **实现状态**：`全部落地`
>
> | | |
> | :--- | :--- |
> | **已落地** | IME 候选框锚定计算、多行选区几何合并与圆角轮廓、光标栈、`TextInput` 的预编辑/提交状态机（`ImePreedit`/`ImeCommit`）、`ImeUpdate` 带字节光标区间（`cursor_range`）、Windows 10/11 原生无边框 Snap Layouts 子类化、系统字体回退链（Segoe UI Variable / Microsoft YaHei UI / Cascadia Mono）、富剪贴板（图片与文件列表）及显卡设备丢失（TDR）热恢复 |
> | **源码** | `crates/unbound-platform/src/{win32_frame, dialogs, tray, clipboard, window, backend}.rs`、`crates/unbound-text/src/{selection, shaping}.rs`、`crates/unbound-app/src/{window_runner, lib}.rs` |

## CJK 完整输入法管线

聚焦文本组件或光标移动时，实时结合 UI 布局或画布摄像机变换矩阵算出光标在
物理窗口中的屏幕坐标，调用平台层 `set_ime_cursor_area` 精确锚定系统候选词浮窗。

```rust
pub fn ime_cursor_area(
    caret: Rect,
    camera: &Affine2,
    dpi: f32,
    window: Vec2,
) -> Rect
```

该函数把**画布坐标系**下的光标矩形转换为**物理窗口像素**坐标，同时应用
摄像机变换与 DPI 缩放——画布上的文字和 UI 树上的文字走的是同一条路径。

## 富文本精细交互与选区圆角布尔合并

- 支持双向文本（BiDi）光标视觉跳跃与逻辑移动。塑形结果记录 `BiDi` embedding
  level，奇数表示从右到左。
- 多行文本划选时，对相邻行的选区矩形执行**几何联合与外轮廓圆角化处理
  (Rounded Selection Polygon Union)**，消除传统逐行独立矩形拼接处的生硬
  锯齿。

### 几何合并算法

| 函数 | 职责 |
| :--- | :--- |
| `merge_line_rects(lines, radius)` | 把多行选区矩形合并为带圆角的槽位集合 |
| `outline_polygon(slot)` | 导出圆角槽位的外轮廓多边形 |

轮廓导出为多边形后可以直接喂给 [双层拾取](hit-testing.md) 的
`point_in_polygon`，因此选区本身也是可拾取、可渲染的图元。

## 确定性光标状态栈

采用基于拾取路径优先级的覆盖机制：

```
Active Pointer Capture > 顶层 Overlay > 尺寸调节控制柄 (Resize Handles)
    > 画布自定义工具光标 > 交互式文本 (I-Beam) > 默认箭头
```

`CursorStack` 是这条优先级的显式实现，`CursorSource` 标明光标由哪一层决定。

## 已知缺口

| 缺口 | 说明 |
| :--- | :--- |
| IME 预编辑事件流 | 平台层尚未把 `Preedit / Composition` 事件送入塑形层，因此不支持行内波浪下划线与分段高亮 |
| 连字内部光标切分 | 塑形层记录了连字信息，但光标在连字内部按比例切分尚未实现 |

这两项都依赖平台层的 IME 事件接入，属于
[窗口呈现](../01-graphics/window-presentation.md) 打通真实窗口后的后续工作。

---

**相关文档**：[双层拾取](hit-testing.md) · [工业级文字](../01-graphics/text.md)

**返回**：[04-input](README.md) · [文档索引](../README.md)

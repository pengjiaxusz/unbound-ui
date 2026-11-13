# 02-ui UI 核心范式

UI 框架层：状态拓扑、增量排版、虚拟化、动画与主题。

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [面向数据拓扑](dod-model.md) | SlotMap、COW、Action as Data | `已落地` |
| [增量排版引擎](layout.md) | RelayoutBoundary、三级脏标记、并行求解 | `已落地` |
| [视口虚拟化](virtualization.md) | VirtualList/Tree/Grid 与环形槽位复用 | `已落地` |
| [动画与像素吸附](animation.md) | 物理弹簧、亚像素旁路、帧调度 | `已落地` |
| [正交缩放与主题切换](theme-and-zoom.md) | 三层缩放、4 KiB 颜色表 | `已落地` |

相关 crate：`unbound-world`、`unbound-layout`、`unbound-anim`、`unbound-theme`、`unbound-app`、`unbound-widgets`。

## 框架级核心子系统（Stage 5）

| 子系统 | 归属路径 | 核心职责 |
| :--- | :--- | :--- |
| `FocusGroup` / `RovingRegistry` | `crates/unbound-input/src/focus.rs` + `unbound-app` | `Linear` / `Grid` / `Tree` Roving TabIndex 方向键与 `Home`/`End` 巡航 |
| `OverlayManager` | `crates/unbound-app/src/overlay_manager.rs` | 浮层栈、模态焦点陷阱联动、外部点击关闭、`Escape` 弹出与焦点回归 |
| `FormController` / `FormValidator` | `crates/unbound-widgets/src/form.rs` | 字段 `touched`/`dirty` 追踪、链式规则校验与提交首错滚动聚焦 |
| `RouterStack` | `crates/unbound-app/src/router.rs` | 命名路由历史栈（`push`/`replace`/`pop`/`forward`）、过渡动画与面包屑同步 |
| `I18nBundle` | `crates/unbound-app/src/i18n.rs` | 多语言词条插值、Locale 数字/日期格式化与 RTL 方向切换 |
| `SvgPath` / `IconPack` | `crates/unbound-vector/src/svg_path.rs` + `unbound-widgets/src/image.rs` | SVG Path (`M/L/H/V/C/S/Q/T/A/Z`) 二次贝塞尔降阶缓存与 66 个内置矢量图标 |
| `UiWidget` / 声明式人体工学 | `crates/unbound-widgets/src/ergonomics.rs` + `unbound-app` | 声明式组件契约、Rect-free 构建器、自动固有尺寸测量、局部状态派生、事件自动派发与一键运行器 (`run_windowed`) |

---

**返回**：[文档索引](../README.md)

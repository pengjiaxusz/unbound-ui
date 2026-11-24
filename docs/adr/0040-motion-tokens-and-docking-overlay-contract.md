# ADR 0040：两级动效令牌、MotionPolicy 统一治理与纯区域跨窗停靠预览契约

- 状态：已采纳
- 日期：2026-11-24
- 关联：协同 ADR 0034（主题过渡减弱动画联动）与 ADR 0039（幽灵窗与悬浮浮岛宿主）

## 背景

`unbound-anim`、`unbound-widgets` 与 `unbound-app` 在动效治理和跨窗拖拽停靠预览方面存在两类结构性缺口：

1. **动效令牌与无障碍治理分散**：`unbound-anim` 仅提供 4 组 `SpringPreset` 与 3 个 `CubicBezier` 常量，缺少统一时长令牌（`DurationToken`）、基于单调 `dt` 的时间轴缓动（`Tween`）以及集中式 `reduced_motion` / 速率倍率治理；各控件与主题模块只能分散硬编码毫秒常量或直接查询平台减弱动画开关。
2. **跨窗停靠预览协议与防闪烁机制缺失**：既有 `unbound-widgets::dock` 仅支持单棵 `DockTree` 内的静态边缘预览，缺少跨窗口候选仲裁、整窗外缘吸附带、标签插入指示线、区分允许/禁止/嵌套的三态语义配色、整窗跨视口防闪烁驻留防抖、多层级滚轮下钻以及边界钳制的指针提示胶囊。

## 决策

### 1. 两级动效令牌与零平台依赖 `MotionPolicy`（`unbound-anim`）

- **基础与语义两级令牌**：在 `unbound-anim::tokens` 定义基础时长令牌 `DurationToken`（`Instant` 0ms、`Quick` 90ms、`Short` 120ms、`Medium` 180ms、`Slow` 280ms）与对齐 Fluent 的缓动令牌 `EasingToken`（`FastOutSlowIn`、`Accelerate`、`Decelerate`、`Linear`），组合为语义 `MotionToken`（微交互、停靠预览、浮层切入/退出、面板调整与主题过渡）。
- **单调 `dt` 时间轴 `Tween` 与 `MotionPolicy`**：引入 `Tween`（精确 `CubicBezier` 采样、支持中途 `set_target` 平滑打断、60Hz/120Hz/240Hz 一致收敛、终点精确吸附）与纯值 `MotionPolicy { reduced_motion: bool, speed_scale: f32 /* clamp 0.05..=2.0 */ }`，并接入 `Animator`。`unbound-anim` 严格保持零 OS/平台依赖。
- **减弱动画瞬切红线**：当 `reduced_motion == true` 时，所有时长令牌强制坍缩为 `0ms`，`Tween` 与 `Animator` 弹簧同步瞬切至终态（显式声明为必要非空间淡变者除外）；由 `unbound-app` 统一从平台采集并透传至 `AppContext` 及无头测试覆写接缝。

### 2. 纯区域高亮停靠预览与三态语义配色（`unbound-widgets::docking_overlay`）

- **取消传统五向指南针，采用直接悬停区域预览**：根据跨窗统一坐标系下的候选窗口/面板矩形，直接发射半透明预览矩形（`Edge` / `RootEdge` 半区、`Center` 避开顶部标签栏的内容区、`Reorder` `2px` 标签栏插入指示线）与紧贴光标的边界钳制提示胶囊（`HintPill`）；窗外自由空间（`TearOut`）不在宿主内绘制预览框，交由外部拖拽代理（`DockDragProxy`）表达。
- **三态语义配色（`DockOverlayTokens`）**：
  - `Allowed`：主题强调色 `Token::PRIMARY`（填充 `0.22` alpha + `1px` 边框）；
  - `Blocked`（如拖回自身原标签组中心或受限边界）：中性静音灰（填充 `0.12–0.16` alpha + `1px` 灰边框）；
  - `NestArmed`（可选嵌套停靠）：次级强调色槽位（未配置时回退 `Token::PRIMARY`）。
  - 预览显隐与状态切换过渡消费 `MotionToken::OverlayPreview`（`Short` 120ms），自动响应 `MotionPolicy`。

### 3. 跨窗防闪烁驻留防抖与多层级滚轮下钻

- **整窗跨视口 `500ms / 12px` 驻留防抖**：整窗外壳（`Shell`）跨视口拖入目标窗口时，必须在 `12px` 容差内停留满 `500ms` 才武装显示蓝色确认预览（武装后微调移动不再重置），避免窗口拖过其他视口时乱闪；普通单标签拖拽保持即时响应零延迟。
- **多层级滚轮下钻**：同一指针位置命中多层嵌套候选（内层面板 / 外层面板 / 整窗外缘）时，支持 `nudge_level(delta)` 在候选层级间循环切换，默认优先最内层。
- **纯协议与宿主解耦**：`unbound-widgets::docking_overlay` 保持纯几何协议、状态机与图元发射器职责；OS 跨窗指针捕获、屏幕坐标映射与顶层幽灵窗/拖拽代理宿主由 `unbound-platform` 与 `unbound-app`（协同 ADR 0039）按该协议驱动。

## 备选方案

- **在停靠预览中保留传统五向指南针悬浮图标按钮**：在多屏高密度面板下视觉噪点重、需要二次瞄准中心图标，交互效率低于现代直接悬停区域高亮预览。
- **在 `unbound-anim` 内直接依赖 `unbound-platform` 读取系统减弱动画设置**：会破坏 `unbound-anim` 的纯数学零平台依赖边界，并使无头单元测试无法通过纯值注入切换 `reduced_motion`。
- **整窗跨视口拖入不做驻留防抖、立即显示停靠预览**：用户在多显示器间拖拽移动窗口经过另一窗口上方时会高频闪出蓝色预览框，严重干扰正常移窗。

## 影响

- **正向收益**：全引擎获得单一事实源的时长/缓动/减弱动画治理体系，以及现代、防闪烁、支持多层级下钻与三态反馈的跨窗停靠预览协议，两层均 100% 支持纯 CPU 无头确定性测试。
- **工程约束与代价**：
  - 存量分散硬编码动画时长的控件可分批渐进迁移至 `MotionToken` 与 `AppContext` 的 `MotionPolicy`；
  - 跨窗真实拖拽代理呈现依赖 `MultiWindowCoordinator` 的屏幕坐标路由与 ADR 0039 的无激活幽灵窗底座协同。

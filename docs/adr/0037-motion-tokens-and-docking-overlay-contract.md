# ADR 0037：动效令牌统一治理与跨窗停靠区域预览叠加层契约

**Status**: accepted

## Context

1. **动效分散治理与无障碍缺失**：`unbound-anim` 当前仅提供 4 组 `SpringPreset` 与 3 个 `CubicBezier` 常量，缺少统一的时长令牌（`DurationToken`）、基于单调 `dt` 采样的时间轴缓动状态机（`Tween`）以及集中式的 `reduced_motion` / 全局速率倍率治理；各消费方只能在业务层零散读取平台减弱动画开关，高刷新率（120Hz–240Hz）下的连续插值与统一瞬切语义缺少单一事实源。
2. **跨窗拖拽停靠预览协议缺失**：`unbound-widgets::dock` 当前仅支持单棵 `DockTree` 内的 `0.25` 边缘四分判定与静态预览矩形，缺少跨窗口候选仲裁、整窗外缘吸附带、标签插入指示线、区分“允许 / 禁止 / 嵌套”的三态语义配色、整窗跨视口防闪烁驻留防抖、多层级滚轮下钻以及紧贴光标的状态提示胶囊。

## Decision

1. **两级动效令牌与无平台依赖策略 (`unbound-anim`)**：
   - 在 `crates/unbound-anim/src/tokens.rs` 中建立**基础令牌 + 语义令牌**体系：基础层定义 `DurationToken`（`Instant` 0ms / `Quick` 90ms / `Short` 120ms / `Medium` 180ms / `Slow` 280ms）与 `EasingToken`（对齐 Fluent 的 `FastOutSlowIn`、`Accelerate`、`Decelerate`、`Linear`）；语义层定义 `MotionToken`（覆盖微交互、停靠预览、浮层切入/退出、面板调整与主题过渡）及基于单调 `dt` 推进的 `Tween` 插值器，与现有 `Spring` / `Animator` 互补并存。
   - 保持 `unbound-anim` 零平台依赖，通过纯值 `MotionPolicy { reduced_motion: bool, speed_scale: f32 /* 0.05..=2.0 */ }` 统一治理：`reduced_motion == true` 时所有时长令牌坍缩为 `0ms`、`Tween` 与 `Spring` 同步瞬切至终态（除非显式声明为必要非空间淡变）。
2. **纯区域高亮停靠预览叠加层 (`unbound-widgets::docking_overlay`)**：
   - 摒弃传统五向指南针图标按钮，采用**直接悬停区域预览**：根据跨窗统一坐标系下的候选窗口/面板几何，直接发射半透明预览矩形（`Edge` / `RootEdge` 半区、`Center` 避开标签栏的内容区、`Reorder` 2px 标签栏插入线）与紧贴光标的提示胶囊（`HintPill`，钳制在叠加层边界内），窗外 `TearOut` 自由空间不绘制宿主框而交由外部幽灵窗/拖拽代理（`DockDragProxy`）表达。
   - 引入三态语义配色（`DockOverlayTokens`）：允许停靠使用主题强调色（`Token::PRIMARY`，0.22 alpha + 1px 边框），禁止停靠（如拖回自身原位置或受限边界）使用中性静音灰（0.12–0.16 alpha + 1px 灰边框），嵌套停靠支持可选次级强调色（未配置时回退 `PRIMARY`）。
   - 预览矩形的显隐与切换过渡消费 `MotionToken::OverlayPreview`（`Short` 120ms），与 `MotionPolicy` 联动。
3. **跨窗防抖与多层级滚轮下钻内置于纯状态机**：
   - 整窗外壳（`Shell`）跨视口拖入目标窗口时，必须在 `12px` 容差内原地停留满 `500ms` 才武装显示确认预览（已武装后微调移动不再重置）；普通单标签拖拽即时响应无延迟。
   - 同一指针位置存在多层嵌套候选时，支持通过 `nudge_level(delta)` 在「内层面板 → 外层面板 → 整窗外缘」间切换，默认优先最内层。
   - `unbound-widgets` 仅承载纯几何协议、状态机与渲染图元发射，OS 级跨窗指针捕获与顶层叠加窗宿主由 `unbound-platform` / `unbound-app` 按该协议驱动。

## Consequences

- `unbound-anim` 的 `Animator` 与新建 `Tween` 统一受 `MotionPolicy` 约束，后续新增动画原语不得绕过令牌体系硬编码毫秒数。
- `unbound-widgets::dock` 保持向后兼容，跨窗仲裁、区域预览与拖拽代理图元收敛在独立的 `docking_overlay.rs` 模块中，便于无头单测独立验证几何、防抖与三态配色。
- 上层多窗宿主接入跨窗拖拽时需在窗口坐标系与桌面逻辑坐标系之间提供统一映射，并在触发整窗拖入时透传 `dt` 以驱动 `500ms` 驻留计时器。

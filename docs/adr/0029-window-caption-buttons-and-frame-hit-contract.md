# ADR 0029：独立窗口控制按钮组（WindowCaptionButtons）与统一 FrameHitMetrics 热区契约

- 状态：已采纳
- 日期：2026-11-23
- 关联：ADR 0014（窗口外观意图账本）、ADR 0024（壳层融合标题栏）、ADR 0026（NavigationView 规范复刻与 showcase dogfood）

## 背景

`crates/unbound-widgets/src/title_bar.rs` 已提供单枚 `WindowCaptionButton` 与整条 `TitleBarDesc`，`crates/unbound-app` 也提供了 `ViewBuilder::title_bar_custom`。但在实际构建自定义顶栏应用（如 `apps/showcase`）时暴露出两个结构性问题：

1. **缺少独立三联组合组件**：不采用整条 `TitleBarDesc` 的自定义顶栏无法开箱即用地接入最小化 / 最大化还原 / 关闭按钮组，导致 `apps/showcase` 另起炉灶手绘三枚按钮，日后外部框架用户也会重复造轮子。
2. **Win32 `FrameHitMetrics` 热区策略分裂**：`unbound-app::window_runner` 刻意仅将最大化按钮矩形写入 `FrameHitMetrics`（触发 `HTMAXBUTTON` 以获得原生 Win11 Snap Layout），将最小化与关闭留给框架 UI 事件层以便应用拦截关闭确认；而 `apps/showcase/src/main.rs` 却把全部三枚按钮硬编码写入了 `FrameHitMetrics`，导致 Windows 上最小化与关闭被 `WM_NCLBUTTONUP` 直接截走、绕过框架事件。

## 决策

- **抽出独立一等组件 `WindowCaptionButtons`**：在 `unbound-widgets` 中提供独立的窗口控制按钮组组件，默认严格遵循 Win11 Fluent 规范（单钮宽 `46.0px`、默认高 `32.0px`、Win11 标准悬停与关闭红底反白、自动跟随窗口最大化态切换最大化/还原图标）。同时支持配置高度以贴合自定义顶栏，并支持 `show_minimize` / `show_maximize` 开关以覆盖对话框或工具面板。
- **`TitleBarDesc` 与自定义顶栏统一复用**：`TitleBarDesc` 内部改为组合 `WindowCaptionButtons`，`apps/showcase` 等自定义顶栏直接调用同一组件，消除重复绘制实现。
- **收口唯一 `FrameHitMetrics` 契约**：`WindowCaptionButtons` 暴露与 `unbound-app::window_runner` 一致的热区契约——仅将最大化按钮矩形提交给 Win32 `FrameHitMetrics`（`HTMAXBUTTON`），最小化与关闭始终经框架事件分发，保证关闭拦截、无障碍树与跨平台行为完全一致。

## 备选方案

- **仅扩展 `TitleBarDesc` 支持自定义高度与插槽**：无法覆盖只需独立三联控制按钮组的自定义布局或工具窗口场景，对框架使用者耦合偏重。
- **不新增组合组件，要求调用方手动拼装 3 枚 `WindowCaptionButton`**：无法收口三联排列间距、状态同步与 `FrameHitMetrics` 热区契约，调用方仍容易把最小化/关闭误写入 `WM_NCHITTEST`。

## 影响

- `unbound-widgets` 新增并导出 `WindowCaptionButtons` 描述符与发射器，`TitleBarDesc` 改为内部复用且保持对外行为兼容；
- `apps/showcase` 移除手绘 `draw_window_controls` 及硬编码三钮 `FrameHitMetrics`，改走统一契约；
- 外部框架使用者构建自定义顶栏时可直接复用一套默认即 Win11 风格的窗口控制按钮组。

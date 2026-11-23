# ADR 0030：可见性驱动节流、高分辨率帧等待与富内容 ToolTip 契约

- 状态：已采纳
- 日期：2026-11-23
- 关联：修订 ADR 0022（取消默认失焦 10fps 降帧策略）、ADR 0023（多窗口每窗独立 Pacer/遮挡判定）、ADR 0024（浮层 `CARD_BORDER` 描边约定）

## 背景

在实际桌面使用与 Showcase 验收中暴露出三类影响观感与流畅度的结构性问题：

1. **启动激活态锁死 `~33 fps` 与 Win32 15.625ms 睡眠抖动**：`Showcase::resumed()` 启动建链硬编码 `allow_tearing = false` 且首帧跳过重建，叠加 Windows 默认 `15.625 ms` 时钟粒度下 `MsgWaitForMultipleObjectsEx` 超时唤醒过晚，与 `FIFO + vkWaitForPresentKHR` 形成双周期 VBlank 互锁（实测 `30.77 ms / 33 fps`）。
2. **可见窗口失焦被强制降到 `10 fps`**：ADR 0022 将 `RenderThrottlePolicy::default().unfocused_idle_fps` 设为 `10`。当用户把窗口摆在一旁对照参考、仅将焦点切去其他应用时，窗口明明完整可见却被强行降成 `10 fps`，观感明显卡顿；而框架其实已具备完整的最小化（`IsIconic`）、隐藏（`IsWindowVisible`）与 250ms 轮询 Z 序完全遮挡检测（`probe_window_occluded`）。
3. **ToolTip 默认外观偏离 Win11 且缺乏富内容组合能力**：`TooltipDesc` 仅支持单字符串 `label` 且默认描边使用偏亮的 `Token::OUTLINE`、对称内边距与偏重阴影；Showcase 的 Tooltip 演示页甚至绕过 `emit_tooltip` 手绘了亮蓝 `Token::PRIMARY` 边框。同时框架用户无法像 `Popover` 一样往 ToolTip 中组合图标、标题、正文、`Kbd` 快捷键徽章等任意展示元素。

## 决策

- **修订 ADR 0022 为「可见性驱动节流」**：将 `RenderThrottlePolicy::default().unfocused_idle_fps` 改为 `None`（类型调整为 `Option<u32>`，默认失焦但仍可见时不降帧，维持正常帧率上限与按需重绘），仅在最小化、隐藏或完全被遮挡时进入 `Suspended` 停绘；同步清理 `overlay_host.rs` 中残留的 `ThrottledFps(30)` 分支，并让 `Showcase::frame_impl` 显式短路 `is_suspended()`。多窗口仍按 ADR 0023 每窗独立判定可见性。
- **Win32 高分辨率可等待定时器与启动建链一致化**：`Showcase::resumed()` 与 `recreate_swapchain()` 共用同一 `allow_tearing` 判定逻辑；Win32 帧调度等待改用 `CreateWaitableTimerExW`（`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`，按可用性回退），避免调用 `timeBeginPeriod(1)` 污染全系统功耗与时钟。
- **ToolTip 默认对齐 Win11 并开放纯展示富内容组合**：
  - `tooltip.rs` 默认样式对齐 Win11 Fluent ToolTip 与 `parity.rs` 基线（`4px` 圆角、`Token::OVERLAY_BG` 填充、`1px Token::CARD_BORDER` 描边、轻量阴影、`12px` 左对齐自动换行、`9/6/9/8` 非对称内边距）。
  - 保留 `TooltipDesc::new(text)` 极简纯文本构造，同时提供声明式 `ViewBuilder::tooltip_with(key, anchor, desc, |v| { ... })` 与底层 `Canvas` 富内容容器测量/绘制能力，支持塞入任意展示元素（图标、标题、正文、`Kbd` 徽章等）。
  - 严格锁定边界：`ToolTip` 始终挂载于 `OverlayKind::Tooltip`（鼠标穿透、不抢焦点、纯展示）；需点击或输入交互的浮层统一使用 `HoverCard` 或 `Popover`。

## 备选方案

- **调用全局 `timeBeginPeriod(1)` 解决 15.625ms 粒度**：会影响整机电源状态与后台功耗，高分辨率可等待定时器（Windows 10 1803+）仅作用于本进程等待句柄，副作用更小。
- **彻底移除 `unfocused_idle_fps` 字段**：虽能解决默认卡顿，但剥夺了特殊后台工具应用按需开启失焦省电的能力；改为 `Option<u32>` 默认 `None` 兼顾直觉与可扩展性。
- **允许 ToolTip 接收指针点击与焦点**：会与 `HoverCard` / `Popover` 语义重叠并破坏悬停移出即关闭的轻量体验，故明确禁止。

## 影响

- 所有默认使用 `RenderThrottlePolicy::default()` 的单窗/多窗应用在失焦但仍可见时均保持流畅帧率，仅不可见时停绘；
- `ViewBuilder` 与 `unbound-widgets` 新增富内容 `ToolTip` 组合 API，默认纯文本气泡与 Showcase Tooltip 演示页外观统一对齐 Win11 规范；
- 后续 Showcase 壳层与多语言（i18n）改造直接基于新 `ToolTip` 契约全量落地。

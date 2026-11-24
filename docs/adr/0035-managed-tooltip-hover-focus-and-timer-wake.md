# ADR 0035：运行时 ToolTip 全局托管、双触发与 TimerQueue 精确唤醒契约

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0030（ToolTip 外观与富内容纯展示契约）

## 背景

ADR 0030 统一了 ToolTip 的 Win11 Fluent 默认外观与富内容纯展示契约，但运行期悬停交互仍存在两类结构性缺口：

1. **声明式层缺少内置悬停/焦点托管**：`ViewBuilder::tooltip` 与 `ViewBuilder::tooltip_with` 均为无条件直显，业务与示例应用必须手写 `TooltipState`、`tick(dt)` 轮询与显隐判断。一旦宿主未把 `tooltip_pending` 持续映射为帧重绘（例如 Showcase 壳层 `runtime_app.rs` 丢弃了 `GalleryApp::tick(dt)` 的返回值），事件循环在鼠标静止后直接休眠，400ms 延迟永远不会到期，表现为「悬停上去没有任何反应」。
2. **轮询模式浪费帧预算且缺键盘无障碍与连续浏览体验**：在 400ms 悬停等待期间靠 `animating = true` 持续 60fps 空转违背按需重绘原则；同时缺少键盘导航聚焦（`focus_visible`）触发、相邻按钮连续浏览免延迟（Warmup）以及滚动/按键/失焦/上层浮层打开时的统一自动关闭。

## 决策

- **在 `AppRuntime` 中引入全局唯一的 ToolTip 托管状态机**：
  - 全窗口任意时刻至多展示一个托管 ToolTip，锚定到当前活跃目标的节点键（`anchor_key`）。
  - **双触发源与优先级**：
    - 鼠标悬停：默认延迟 `400ms`（可按描述符覆盖）后显示；在最近一次 ToolTip 关闭后的 **悬停预热窗口（Warmup Window，默认 `300ms`）** 内移至下一个带 ToolTip 的锚点时跳过延迟立即显示（`0ms`）。
    - 键盘焦点：仅当 `focus_visible == true`（键盘 Tab 导航聚焦，鼠标点击不误触）时 `0ms` 立即显示。
    - 指针与键盘焦点并存时采用 **最近一次输入源优先**。
  - **统一强制关闭与重弹抑制**：鼠标移出锚点、鼠标按下、滚轮滚动、键盘输入（含 `Esc`）、窗口失焦或存在更高层交互浮层（菜单/弹窗/气泡框等）时立即隐藏；通过按键关闭当前焦点锚点后，对该锚点抑制重弹直到焦点离开再重新聚焦。
- **以 `TimerQueue` 精确单次定时唤醒替代持续动画轮询**：
  - 悬停进入延迟倒计时时，由 `AppRuntime` 直接向内置 `TimerQueue` 注册一次性内部唤醒定时器，经 `AppRuntime::next_wake_in()` 交由宿主 `ControlFlow::WaitUntil` 在精确到期时刻唤醒事件循环，等待期间不触发空转重绘。
  - `TooltipState` 作为 `unbound-widgets` 纯逻辑状态机同步补充 `warmup_ms` 连续浏览免延迟与时钟推进能力，供无 `AppRuntime` 的独立宿主复用。
- **`ViewBuilder` 托管 API 与受控 API 显式分离**：
  - 新增 `ViewBuilder::tooltip_for(anchor_key, desc)` 与 `ViewBuilder::tooltip_for_with(anchor_key, desc, build)` 声明式托管挂载接口；富内容闭包 `build` 采用惰性求值，仅在对应 `anchor_key` 处于激活显示态时执行，未触发时零子树构建开销。
  - 保留既有 `ViewBuilder::tooltip` 与 `ViewBuilder::tooltip_with` 作为调用方手动控制显隐的受控直显接口，保持零破坏性兼容。
- **保持窗口内自动翻转与视口钳制边界**：
  - 托管 ToolTip 继续沿用 ADR 0030 的 `OverlayKind::Tooltip` 鼠标穿透、不抢焦点契约与 `place_tooltip_in_window` 窗口内自动翻转/安全边距钳制，暂不自动升级为原生越界系统弹出窗。

## 备选方案

- **仅在 Showcase 壳层补接 `tick` 唤醒并保持框架无托管**：无法消除后续业务页面手写 `TooltipState` 漏接唤醒的结构性隐患，且仍需在 400ms 等待期间维持动画循环空转。
- **直接改变 `ViewBuilder::tooltip_with` 为自动托管**：会破坏现有受控展示用法与集成测试契约；拆分为 `tooltip_for` / `tooltip_for_with`（托管）与 `tooltip` / `tooltip_with`（受控）语义更清晰。
- **等待期间强制 `animating = true` 轮询**：会在鼠标每次短暂停留时拉高 CPU/GPU 唤醒频率，不如复用已有的 `TimerQueue` 单次精确唤醒。

## 影响

- 应用层只需在触发控件旁声明 `view.tooltip_for(...)` 或 `view.tooltip_for_with(...)` 即可自动获得延迟悬停、连续浏览免延迟、键盘焦点无障碍与全场景自动隐藏；
- 宿主仅需把 `AppRuntime::next_wake_in()` 接入 `WaitUntil` 即可支持所有托管 ToolTip（及通用 `TimerQueue` 定时器），零空转帧开销；
- Showcase 壳层与组件画廊同步修复并接入精确唤醒，悬停响应恢复正常。

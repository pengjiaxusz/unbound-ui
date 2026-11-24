# ADR 0039：无激活顶层幽灵窗、悬浮置顶浮岛宿主与跨平台降级契约

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0030（弹窗/工具提示边界），协同 `MultiWindowCoordinator` 多窗架构

## 背景

`unbound-platform` 与 `unbound-app` 现有窗口体系仅覆盖「常规抢焦点顶层窗（`WindowConfig`）」与「强绑定父窗、无独立 `AppRuntime`/Vulkan `Swapchain` 的轻量弹出窗（`PopupHostRegistry`）」，面对瞬时预览幽灵窗与常驻可拖拽置顶浮岛两类桌面交互形态存在四类结构性缺口：

1. **既有弹出窗无法承载独立富内容渲染**：`popup_host.rs` 不创建独立 `AppRuntime` 与 Vulkan `Surface`/`Swapchain`，仅将指针坐标转发回父窗，无法承载独立声明式 UI（含透明圆角、后景材质、动态媒体或独立状态机），也无法脱离主窗独立常驻。
2. **顶层窗缺少无激活、置顶、任务栏隐藏与运行期热切换**：`WindowConfig` 与 `NativeWindow` 缺少 `always_on_top`、`skip_taskbar`（`WS_EX_TOOLWINDOW`）、`no_activate`（`WS_EX_NOACTIVATE` + `SW_SHOWNOACTIVATE` + `WM_MOUSEACTIVATE => MA_NOACTIVATE`）及 `click_through`（`WS_EX_TRANSPARENT` / 空输入区域），也缺少运行期无闪烁切换与外框重定位 API。
3. **缺少公开多屏工作区查询与边缘磁吸算法**：Win32 与 X11 后端已存在私有工作区查询与 64 位安全钳制工具，但未上升为跨平台公开 API，也缺少面向浮岛拖拽释放的四边磁吸与跨屏 DPI 避让定位。
4. **缺少 Linux Wayland/X11 显式能力探测与降级契约**：Wayland 常规 `xdg_toplevel` 受协议限制不支持全局坐标定位与强制置顶，需通过公开能力探测与确定性降级保证跨平台契约一致与无头 CI 可测。

## 决策

### 1. 与 `popup_host.rs` 明确分工，走 `MultiWindowCoordinator` 独立渲染顶层路径

- **职责红线**：`popup_host.rs` 继续专管依赖父窗生命周期的轻量弹出菜单、下拉框与 ToolTip；幽灵窗与悬浮浮岛统一通过新增 **`WindowRole::{Standard, Ghost, FloatingIsland}`** 与 **`FloatingHost`** 托管于 `MultiWindowCoordinator`，共享 `Arc<Gpu>` 与 `Arc<AtlasSet>`、各自持有独立 `Surface` / `Swapchain` / `AppRuntime` 与透明圆角渲染能力。
- **不抢主窗焦点与生命周期隔离**：`Ghost` 与 `FloatingIsland` 在显示与交互期间默认不激活、不抢占宿主/主窗键盘焦点，且不出现在系统任务栏与 `Alt+Tab` 列表；关闭主窗或显式发出关窗命令时，严格按既有 Vulkan 销毁顺序安全级联清理。

### 2. 统一 `FloatingWindowPolicy` 与运行期零闪烁热切换

- **策略统一**：在 `unbound-platform` 定义 `FloatingWindowPolicy { no_activate, always_on_top, skip_taskbar, click_through, draggable }`：
  - **幽灵窗（`Ghost`）** 默认：`no_activate = true`、`always_on_top = true`、`skip_taskbar = true`、`click_through = true`（按需热切为可交互预览态）；
  - **悬浮浮岛（`FloatingIsland`）** 默认：`no_activate = true`、`always_on_top = true`、`skip_taskbar = true`、`click_through = false`、`draggable = true`（按需热切置顶或临时穿透）。
- **运行期热切换 API**：在 `NativeWindow` 与 `WindowCommand` 暴露 `set_visible_no_activate`、`set_always_on_top`、`set_click_through` 与 `set_outer_rect`，在原窗口句柄上直接变更样式/输入区域与位置，杜绝销毁重建导致的黑屏闪帧与状态丢失。

### 3. 公开多显示器工作区查询、安全避让与浮岛边缘磁吸

- **多屏工作区公开 API**：将 Win32（`MonitorFromPoint`/`GetMonitorInfoW`）与 X11（XRandR/`_NET_WORKAREA`）的显示器工作区查询提升为 `unbound-platform` 公开只读接口（含逻辑/物理边界与 DPI 缩放因子）。
- **纯逻辑定位与磁吸算法**：复用既有 64 位溢出安全钳制逻辑，在 `unbound-platform` 提供纯 CPU 可单测的三种定位策略：
  - `CursorMonitorAvoid`：以光标或指定点所在显示器工作区为界，避让光标与任务栏安全显示；
  - `AnchorRectAvoid`：围绕锚定矩形自动翻转并钳制至工作区内部；
  - `EdgeSnap`：浮岛拖拽释放时按阈值自动吸附至当前显示器工作区四边并记录相对锚点，完整兼容负坐标辅屏与跨屏 DPI 切换。

### 4. 跨平台能力探测、Linux 确定性降级与无头测试接缝

- **显式能力探测**：公开 `FloatingWindowCapabilities { supports_no_activate, supports_topmost, supports_skip_taskbar, supports_click_through, supports_global_positioning }`：
  - **Win32**：全量支持（`WS_EX_NOACTIVATE`、`WS_EX_TOPMOST`、`WS_EX_TOOLWINDOW`、`WS_EX_TRANSPARENT`、`SW_SHOWNOACTIVATE`、`WM_MOUSEACTIVATE => MA_NOACTIVATE` 与 DWM 圆角/透明）。
  - **X11**：支持 `_NET_WM_STATE_ABOVE`、`_NET_WM_STATE_SKIP_TASKBAR` / `SKIP_PAGER`、`WM_HINTS.input = False`、`XFixes` 空输入区域穿透与运行时移动。
  - **Wayland**：探测合成器能力；缺少全局定位/置顶协议时平滑降级为无边框 `xdg_toplevel`（保留 `set_input_region` 点击穿透），并在能力结构体中如实回报，绝不崩溃。
- **无头验收接缝**：扩展 `MultiWindowCoordinator` 无头接口，支持注册 `Ghost` / `FloatingIsland` 窗口、断言焦点不抢占、运行期穿透/置顶热切换命令路由、多屏钳制/磁吸坐标解算以及关窗级联清理。

## 备选方案

- **改造 `popup_host.rs` 增加独立 Vulkan 渲染与跨屏常驻能力**：会混淆父窗绑定生命周期、坐标转发与独立顶级窗口模型，导致菜单/ToolTip 路径复杂化并引入焦点与关窗清理回归风险。
- **创建期固定窗口样式，切换穿透或置顶时重建原生窗口**：必然造成视觉闪烁、Vulkan `Swapchain` 重建开销与窗内交互状态丢失。
- **仅在 Win32 实现，Linux 侧返回未实现错误**：破坏 `unbound-ui` 跨平台编译与 Linux 无头 CI 绿灯契约。

## 影响

- **正向收益**：引擎获得统一、无闪烁、可热切换的独立无激活顶层窗与悬浮置顶浮岛底座，天然共享多窗 GPU 与图集资源，完整兼容多显示器混排 DPI 与无头 CI 回归。
- **工程约束与代价**：
  - 单窗 `WindowedRunner` 不负责管理多窗幽灵窗/浮岛，需要该能力的应用统一走 `MultiWindowCoordinator`；
  - Wayland 下无 `wlr-layer-shell` 合成器时置顶与全局定位按协议边界降级为无边框 `xdg_toplevel`（应用层可通过能力探测位做提示或适配）；
  - Win32 原生样式位（`WS_EX_NOACTIVATE` / `WS_EX_TRANSPARENT` / `WM_MOUSEACTIVATE`）与多屏真实移动的视觉/焦点验证需在 Windows 真机执行，跨平台 CI 负责纯逻辑定位/磁吸与无头多窗协调器回归门禁。

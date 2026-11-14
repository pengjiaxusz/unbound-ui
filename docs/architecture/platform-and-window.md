# 架构：platform-and-window
状态：已落地
路径：`crates/unbound-platform/`
入口：`crates/unbound-platform/src/lib.rs::PlatformWindow`、`LatencyTracker`
守卫：`cargo test -p unbound-platform --test a11y_roundtrip --test clipboard`

## 范围
- [负责] 多窗口事件循环、基于官方 [`windows = "0.62.2"`](https://microsoft.github.io/windows-docs-rs/doc/windows/) crate 的原生 Win32 API 后端集成（已淘汰 `winit` 与手写 `extern "system"` FFI 声明）、按 `WindowConfig.{decorations, resizable}` 精确合成窗口样式、无边框窗口八方向边缘/四角拖拽调整大小、DWM 无边框圆角阴影与 Snap Layouts (`HTMAXBUTTON`)、原生文件对话框、富剪贴板、系统托盘通知、AccessKit 实机桥接与延迟计量
- [不负责] UI 控件渲染与排版 -> 委托给 `crates/unbound-app/` 与 `crates/unbound-gpu/`

## 拓扑与流程
- 官方 `windows = "0.62.2"` 绑定：`crates/unbound-platform/src/{backend.rs, win32_window.rs, win32_frame.rs, clipboard.rs, dialogs.rs, tray.rs}` 统一使用 `::windows::Win32::*` 的类型安全句柄（`HWND`、`HINSTANCE`、`HDC`、`HIMC` 等）、结构体与常量，彻底消除手写 C FFI 签名维护成本
- 窗口链路：`PlatformWindow` 基于原生 Win32 消息循环托管多窗口 Surface/Swapchain -> `WM_NCCREATE` 阶段提前通过 `SetWindowLongPtrW(GWLP_USERDATA)` 绑定 `WindowState` 指针 -> `WM_NCCALCSIZE` 在 `decorations == false` 时返回 `LRESULT(0)` 移除标准标题栏与非客户区边框（最大化时自动钳制至显示器工作区 `MONITORINFO.rcWork` 避免溢出屏幕边缘） -> `WM_NCHITTEST` 与 `win32_frame::custom_frame_subclass_proc` 协同支持八方向边缘缩放（`HTLEFT..HTBOTTOMRIGHT`）、Snap Layouts（`HTMAXBUTTON`）与标题栏拖拽（`HTCAPTION`） -> 统一处理 `WM_CLOSE` / `Alt+F4`（`WM_SYSKEYDOWN`）/ 标题栏关闭按钮 -> 分发 `AppEvent`
- 装饰与缩放样式：`compute_window_style(decorations, resizable)` 由 `WindowConfig.decorations` 与 `WindowConfig.resizable` 共同决定原生样式：`decorations == true` 时使用 `WS_OVERLAPPEDWINDOW`（不可缩放时移除 `WS_THICKFRAME | WS_MAXIMIZEBOX`）；`decorations == false` 且 `resizable == true` 时使用 `WS_POPUP | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`，配合 `WM_NCCALCSIZE` 零非客户区保留原生 DWM 边缘拖拽缩放、Aero Snap 与动画；`decorations == false` 且 `resizable == false` 时省略 `WS_THICKFRAME | WS_MAXIMIZEBOX` -> [守卫] `crates/unbound-platform/src/backend.rs::style_tests`
- 双路径边缘拖拽缩放：除 `WM_NCHITTEST` 原生返回 `HTLEFT..HTBOTTOMRIGHT` 外，`Window::drag_resize_window(edge)` 通过 `GetCursorPos` 获取屏幕坐标并发送 `SendMessageW(WM_SYSCOMMAND, SC_SIZE + WMSZ_*, packed_cursor_lparam)` 触发系统级模态缩放循环；`apps/showcase` 在 `Showcase::resumed` 中安装 `install_custom_frame` 并在每帧同步右上角窗口控制按钮热区，且 `resize_edge_at` 仅在指针捕获（`ctx.pressed.is_some()`）时抑制边缘缩放，确保鼠标移至窗口四周边缘与四角时均可平滑拖拽调整大小
- 延迟与呈现：`LatencyTracker` 逐帧记录 Input-to-Photon（预算 `LATENCY_BUDGET` <= 8.3ms）；`present wait` 对齐 DWM 合成节拍
- 系统服务：`dialogs::{open_file, save_file}` 调用 `::windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW}`；`SystemClipboard` 基于 `::windows::Win32::System::DataExchange` 限流 `MAX_CLIPBOARD_BYTES`；`tray` 基于 `::windows::Win32::UI::Shell::Shell_NotifyIconW` 与 `ITaskbarList3`；`Win32A11y` 桥接 `A11yBridge`
- 窗口宿主：`crates/unbound-app/src/window_runner.rs` 用 `window.display_handle()` 派生实例扩展 -> `Gpu` -> `Surface`/`Swapchain`；每帧 `AppRuntime::frame` 产出图元 + 图集批量上传 + 主题 LUT 后 `render_into_swapchain`
- 呈现刷新率：`Window::refresh_rate_hz()` 经 `GetDC` / `GetDeviceCaps(VREFRESH)` 读取窗口所在显示器刷新率，替代 winit 的 `current_monitor()`；驱动上报无效值时由调用方回退 60 Hz，用于判断是否允许撕裂
- 帧同步：acquire 前 `Renderer::wait_for_previous_frame()`（在途帧信号量槽位）；交换链重建后按 `swapchain.extent()` 同步深度附件；`recover_gpu` 按 swapchain -> surface -> renderer 显式 `destroy()`
- 主题交接：宿主持有 `ThemeState`，每帧比对 `ctx.theme` 并 `write_theme_lut`；设备重建后清空标记强制重写

## 取舍（禁止重构）
- [代价] Windows 10/11 无边框可缩放窗口保留 `WS_THICKFRAME` 并在 `WM_NCCALCSIZE` 中返回 `0`（最大化时钳制至 `rcWork`） -> [原因] 同时兼顾全客户区自绘外观、系统级八方向边缘拖拽调整大小、Snap Layouts 贴靠与 Aero Snap -> [位置] `crates/unbound-platform/src/backend.rs`
- [代价] `WM_NCHITTEST` 中窗口按钮命中与四角斜向缩放判定具有明确优先级 -> [原因] 右上角优先保证关闭/最大化/最小化按钮可点击及 `HTMAXBUTTON` Snap Layouts 悬停，四周边缘与四角按 DPI 缩放后的 `resize_border_px`（6px 边 / 12px 角）返回 `HTLEFT..HTBOTTOMRIGHT` -> [位置] `crates/unbound-platform/src/win32_frame.rs::hit_test_custom_frame`
- [代价] 剪贴板读取强制设置 `MAX_CLIPBOARD_BYTES` 上限并隔离 `MemoryClipboard` 替身 -> [原因] 防止超大剪贴板阻塞主线程并支持无头单测 -> [位置] `crates/unbound-platform/src/clipboard.rs`

## 不变量与红线
- [禁止] 在 `crates/unbound-platform` 中新增手写 `#[link(name = ...)] extern "system"` Win32 FFI 块 -> [原因] 统一使用工作区 `windows = "0.62.2"` 官方绑定（注意 crate 内部含 `pub mod windows;`，引用外部 crate 时须写 `::windows::Win32::*`） -> [守卫] `crates/unbound-platform/Cargo.toml`
- [禁止] 窗口边缘拖拽缩放期间阻塞主渲染线程或触发全量字体图集重建 -> [原因] 保障 120Hz 无黑边拉伸 -> [守卫] `crates/unbound-platform/src/latency.rs`
- [必须] 所有平台测试同时提供纯内存替身（如 `MemoryClipboard`），保证无显示服务器 CI 可跑 -> [守卫] `crates/unbound-platform/tests/clipboard.rs`
- [禁止] 只填 `device_extensions` 而漏 `instance_extensions` -> [原因] `VK_KHR_swapchain` 缺 `VK_KHR_surface` 触发 VUID 01387，`vkCreateWin32SurfaceKHR` 无法解析 -> [守卫] `crates/unbound-app/src/window_runner.rs::window_instance_extensions`
- [禁止] 旧 `VkSurfaceKHR` 未销毁就重建 -> [原因] Vulkan 对象无 `Drop`，HWND 仍被占用 -> [守卫] `crates/unbound-app/src/window_runner.rs::recover_gpu`
- [必须] 开窗验收以抓屏像素为准 -> [原因] 「无 panic + 无 VUID」不证明呈现链正确；LUT 未上传、图元被剔除、payload 错位均不报校验错 -> [守卫] `just test-examples` + 10 示例开窗抓屏
- [禁止] 在指针仅悬停（未按下捕获）时拦截窗口边缘 `resize_edge_at` 判定 -> [原因] 会导致鼠标移到窗口边缘无法切换缩放光标和拖拽调整窗口大小 -> [守卫] `apps/showcase/src/main.rs::resize_edge_at`
- [必须] 写入 `FrameHitMetrics` 的标题栏/按钮包围盒与子类命中测试统一到逻辑坐标系 -> [原因] 子类把物理客户区坐标除以 DPI 缩放后再比对，直接写物理矩形会在高 DPI 下整体错位；运行时的控件命中则用物理坐标，不得二次换算 -> [守卫] `crates/unbound-app/src/window_runner.rs`
- [必须] `WindowedRunner::destroy_vulkan`、`recover_gpu` 与 `Renderer::destroy` 在销毁 `Swapchain`（含 `acquire_semaphores`、`render_finished_semaphores`）及 `present_semaphore` 前必须先调用 `gpu.wait_idle()` -> [原因] 防止在途帧仍在 `VkQueue` 执行时销毁信号量与交换链触发 `VUID-vkDestroySemaphore-semaphore-05149` / `VUID-vkDestroySwapchainKHR-swapchain-01282` -> [位置] `crates/unbound-app/src/window_runner.rs`、`crates/unbound-gpu/src/frame.rs`
- [必须] Win32 后端处理 `WM_NCMOUSEMOVE`（配合 `TME_LEAVE | TME_NONCLIENT`）与 `WM_NCMOUSELEAVE` 并换算客户区坐标转发 `PlatformEvent::CursorMoved` -> [原因] `WM_NCHITTEST` 返回 `HTMINBUTTON`/`HTMAXBUTTON`/`HTCLOSE` 时 Windows 会吞掉客户区 `WM_MOUSEMOVE`，转发非客户区移动可在保留 Win11 Snap Layouts 的同时保证右上角按钮 Hover 100% 稳定 -> [位置] `crates/unbound-platform/src/backend.rs`

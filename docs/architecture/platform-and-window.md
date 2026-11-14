# 架构：platform-and-window
状态：部分落地
路径：`crates/unbound-platform/`
入口：`crates/unbound-platform/src/lib.rs::PlatformWindow`、`LatencyTracker`
守卫：`cargo test -p unbound-platform --test a11y_roundtrip --test clipboard`

## 范围
- [负责] 多窗口事件循环、原生 Win32 API 后端集成（已淘汰 `winit`）、按 `WindowConfig.decorations` 选择原生窗口样式、DWM 无边框圆角阴影与 Snap Layouts (`HTMAXBUTTON`)、原生文件对话框、富剪贴板、AccessKit 实机桥接与延迟计量
- [不负责] UI 控件渲染与排版 -> 委托给 `crates/unbound-app/` 与 `crates/unbound-gpu/`

## 拓扑与流程
- 窗口链路：`PlatformWindow` 基于原生 Win32 消息循环托管多窗口 Surface/Swapchain -> Win32 Subclassing 拦截 `WM_NCCALCSIZE`/`WM_NCHITTEST` 实现无边框贴靠菜单（`HTCAPTION`/`HTMAXBUTTON`）与默认标题栏（`TitleBarDesc`） -> 统一处理 `WM_CLOSE` / `Alt+F4`（`WM_SYSKEYDOWN`）/ 标题栏关闭按钮 -> 分发 `AppEvent`
- 装饰样式：`create_native_window::window_style` 由 `WindowConfig.decorations` 决定原生样式；`true` 用 `WS_OVERLAPPEDWINDOW`，`false` 用 `WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`（不含 `WS_CAPTION` / `WS_THICKFRAME`），客户区覆盖整个窗口矩形。创建后统一调用 `win32_frame::apply_borderless_dwm` 恢复 DWM 圆角与阴影，因此未安装子类、自行派发拖拽与窗口按钮的 showcase 也只保留客户端自绘标题栏。`unbound-app::run_windowed` 强制把该配置置为 `false`，使自带自绘标题栏的示例与 showcase 一致 -> [守卫] `crates/unbound-platform/src/backend.rs::style_tests`
- 延迟与呈现：`LatencyTracker` 逐帧记录 Input-to-Photon（预算 `LATENCY_BUDGET` <= 8.3ms）；`present wait` 对齐 DWM 合成节拍（实机呈现全链路延迟统计待接通）
- 系统服务：`dialogs::{open_file, save_file}` 调用原生对话框；`SystemClipboard` 限流 `MAX_CLIPBOARD_BYTES`；`Win32A11y` 桥接 `A11yBridge`
- 窗口宿主：`crates/unbound-app/src/window_runner.rs` 用 `window.display_handle()` 派生实例扩展 -> `Gpu` -> `Surface`/`Swapchain`；每帧 `AppRuntime::frame` 产出图元 + 图集批量上传 + 主题 LUT 后 `render_into_swapchain`
- 呈现刷新率：`Window::refresh_rate_hz()` 经 `GetDC` / `GetDeviceCaps(VREFRESH)` 读取窗口所在显示器刷新率，替代 winit 的 `current_monitor()`；驱动上报无效值时由调用方回退 60 Hz，用于判断是否允许撕裂
- 帧同步：acquire 前 `Renderer::wait_for_previous_frame()`（在途帧信号量槽位）；交换链重建后按 `swapchain.extent()` 同步深度附件；`recover_gpu` 按 swapchain -> surface -> renderer 显式 `destroy()`
- 主题交接：宿主持有 `ThemeState`，每帧比对 `ctx.theme` 并 `write_theme_lut`；设备重建后清空标记强制重写

## 取舍（禁止重构）
- [代价] Windows 10/11 无边框窗口采用 Win32 Subclassing 而非纯自绘标题栏禁用系统命中测试 -> [原因] 保留系统级 Snap Layouts 贴靠、Aero Snap 与边缘调整零黑边 -> [位置] `crates/unbound-platform/src/backend.rs`
- [代价] `WM_NCHITTEST` 中窗口按钮命中先于 8px 边框缩放判定 -> [原因] 按钮锚定右上角，先判边框会把按钮上沿误判为 `HTTOP` / `HTTOPRIGHT`，使屏幕角落失去有效点击目标 -> [位置] `crates/unbound-platform/src/win32_frame.rs::custom_frame_subclass_proc`
- [代价] 剪贴板读取强制设置 `MAX_CLIPBOARD_BYTES` 上限并隔离 `MemoryClipboard` 替身 -> [原因] 防止超大剪贴板阻塞主线程并支持无头单测 -> [位置] `crates/unbound-platform/src/clipboard.rs`

## 不变量与红线
- [禁止] 窗口边缘拖拽缩放期间阻塞主渲染线程或触发全量字体图集重建 -> [原因] 保障 120Hz 无黑边拉伸 -> [守卫] `crates/unbound-platform/src/latency.rs`
- [必须] 所有平台测试同时提供纯内存替身（如 `MemoryClipboard`），保证无显示服务器 CI 可跑 -> [守卫] `crates/unbound-platform/tests/clipboard.rs`
- [禁止] 只填 `device_extensions` 而漏 `instance_extensions` -> [原因] `VK_KHR_swapchain` 缺 `VK_KHR_surface` 触发 VUID 01387，`vkCreateWin32SurfaceKHR` 无法解析 -> [守卫] `crates/unbound-app/src/window_runner.rs::window_instance_extensions`
- [禁止] 旧 `VkSurfaceKHR` 未销毁就重建 -> [原因] Vulkan 对象无 `Drop`，HWND 仍被占用 -> [守卫] `crates/unbound-app/src/window_runner.rs::recover_gpu`
- [必须] 开窗验收以抓屏像素为准 -> [原因] 「无 panic + 无 VUID」不证明呈现链正确；LUT 未上传、图元被剔除、payload 错位均不报校验错 -> [守卫] `just test-examples` + 10 示例开窗抓屏
- [禁止] `decorations = false` 时保留 `WS_CAPTION` 或 `WS_THICKFRAME` -> [原因] 系统仍会绘制原生标题栏与非客户区边框，与客户端自绘标题栏叠成双层 -> [守卫] `crates/unbound-platform/src/backend.rs::style_tests`
- [必须] 写入 `FrameHitMetrics` 的标题栏/按钮包围盒与子类命中测试统一到逻辑坐标系 -> [原因] 子类把物理客户区坐标除以 DPI 缩放后再比对，直接写物理矩形会在高 DPI 下整体错位；运行时的控件命中则用物理坐标，不得二次换算 -> [守卫] `crates/unbound-app/src/window_runner.rs`

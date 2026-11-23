# ADR 0032：Linux 原生 Wayland 窗口后端、Vulkan WSI 兼容与跨平台契约对齐

- 状态：已采纳
- 日期：2026-11-24
- 关联：扩展 ADR 0005 / ADR 0006（后景跨平台能力门槛与自绘策略）、ADR 0022 / ADR 0030（越界 Popup 与可见性驱动节流）、ADR 0023（共享 `VkDevice` 多窗口架构）、ADR 0025（原生矢量图标）、ADR 0029（`WindowCaptionButtons` 跨平台热区契约）

## 背景

`unbound-ui` 已有 16 个纯跨平台 crate 并在 `ubuntu-latest` CI 下全量通过 `clippy` 与 `nextest`，核心 Vulkan 1.3 渲染管线与 `cosmic-text` + `swash` 文本后端均已具备跨平台能力。但在 Linux（含 WSL2 Ubuntu / WSLg）上开窗运行 `showcase` 仍受阻于五个平台假设：

1. **窗口宿主层硬绑定 Win32**：`unbound-platform` 此前移除 `winit` 改用原生 Win32，非 Windows 分支直接返回错误，且 `PlatformApp`、`WindowAppearance` 与 `OverlayWindowSpec` 向外泄漏了 `&win32_window::Window` 与裸 `hwnd: isize`。
2. **Vulkan WSI 硬校验**：`unbound-gpu` 的 `Surface` 仅绑定 `VK_KHR_win32_surface`，且强制要求交换链暴露 `B8G8R8A8_SRGB` 并无条件携带 `TRANSFER_SRC`，在部分 Wayland / WSLg（Mesa `dzn` 或 `llvmpipe`）驱动下会建链失败。
3. **Wayland 安全模型差异**：Wayland 禁止客户端使用全局屏幕绝对坐标，也不暴露其他窗口的 Z 序几何，直接影响 ADR 0022 的越界 Popup、ADR 0023 的跨窗撕裂以及 ADR 0030 的遮挡停绘判据。
4. **Windows 字体与图标硬编码**：运行时少量控件与 `unbound-text` 回退链仍硬编码 `Segoe Fluent Icons`（PUA 码位）与 `Microsoft YaHei UI` 等 Windows 字体。
5. **WSLg 无桌面壁纸源**：`unbound-backdrop` 在非 Windows 下壁纸与主屏几何查询返回空值，而 WSLg 独立 Weston 合成器不存在 GNOME/KDE 壁纸配置。

## 决策

- **原生手写 Wayland 起步，抽象 `NativeWindow` 并预留后端扩展**：Linux 侧直接基于 `smithay-client-toolkit` 实现原生 Wayland 窗口与事件循环，不引入 `winit`，以延续对事件循环、无边框 CSD、越界 `xdg_popup` 与帧节流的精细控制；抽象平台无关的 `NativeWindow` 收口对外暴露的 `&win32_window::Window` 与裸 `hwnd: isize`（Win32 逃生舱仅保留在 `#[cfg(windows)]` 扩展方法上），合并 `unbound-platform` 与 `unbound-app` 重复的 `ClipboardProvider` trait，并预留平台后端边界以便后续按需补充 X11（`x11rb`）。
- **保持 Vulkan 1.3 核心管线不降级，仅在 WSI 兼容层做动态格式与能力适配**：离屏 `RenderTarget` 与 `read_pixels` 继续锁定 `B8G8R8A8_SRGB` 以零破坏现有截图与 Parity 链路；窗口交换链优先选用 `B8G8R8A8_SRGB`，当表面不暴露该格式而暴露 `R8G8B8A8_SRGB` 时，`Pipelines` 复用共享 `PipelineLayout` 与 shader module 按格式懒缓存对应窗口管线变体；`TRANSFER_SRC` 改为按表面 `supported_usage_flags` 能力探测开启。WSL2 下新版 Mesa `dzn`（硬件加速）与 `llvmpipe`（软件兜底）均须无报错运行，性能验收以支持所需特性的硬件 Vulkan 驱动为准。
- **Wayland 越界 Popup 走父窗相对锚点，多窗撕裂分期**：`OverlayWindowSpec` 增加父窗口 `WindowId` 与父窗内相对 `anchor_rect`，Win32 侧由父窗屏幕原点换算全局 `ScreenRect`（行为不变），Wayland 侧直译为 `xdg_popup` + `xdg_positioner`；多窗口（如 `Second Window`）走独立 `xdg_toplevel` 完整运行；跨窗标签撕裂（tear-out）的跨窗拖出留待二期结合 `wl_data_device` DnD 补齐。
- **泛化可见性驱动节流（修订 ADR 0030 平台判据）**：Linux/Wayland 侧以窗口最小化/隐藏以及合成器 `wl_surface::frame` 回调挂起作为 `Suspended` 停绘依据，不再要求客户端读取全局 Z 序；同步更新 `GLOSSARY.md` 中「可见性驱动节流」与「窗口控制按钮组」的跨平台定义。
- **清理运行时 `Segoe Fluent Icons` 依赖并补齐 Linux 字体/壁纸回退**：运行时控件中残余的 `Segoe Fluent Icons` PUA 码位统一迁移至现有 `ShellIcon` 矢量图标（保持 `parity.rs` 与 WinUI 3 基线 1:1 对齐不动），`unbound-text` 按平台补齐 `Noto Sans` / `Noto Sans CJK` / `Noto Color Emoji` 回退链；后景维持 ADR 0005 原生档位禁用策略，自绘云母在 Linux/WSLg 下采用三级壁纸回退源（`UNBOUND_WALLPAPER_PATH` → GNOME/KDE 壁纸配置 → 内置轻量程序化渐变底图），主屏几何由 Wayland `wl_output` 回填。

## 备选方案

- **在 Linux 侧引入 `winit`**：初期开窗更快且同时覆盖 X11/Wayland，但与项目先前移除 `winit` 的决策冲突，在原生 `xdg_popup` 越界弹层、细粒度 CSD 交互与精确帧节流上会受限于封装边界。
- **先手写 X11（`x11rb`）而非 Wayland**：虽具备全局坐标与无焦点覆盖窗，但在 WSLg 与现代 Linux 桌面走 XWayland 存在分数缩放模糊问题；作为后续按需补齐的第二后端更合理。
- **降级 `unbound-gpu` 的 Vulkan 1.3 特性需求以迁就旧版 Mesa `dzn`**：会伤及核心 GPU Compute/Dynamic Rendering 架构，得不偿失。
- **WSL2 阶段依赖挂载 `/mnt/c/Windows/Fonts` 而不清理控件 PUA 图标**：离开 WSL2 到原生 Linux 仍会出现大面积缺字框，无法达成真正的 Linux 支持。

## 影响

- `unbound-platform`、`unbound-app` 与 `apps/showcase` 的窗口创建与外观下发接口完成跨平台收口，消除业务层对 `RawWindowHandle::Win32` 与裸 `HWND` 的直接依赖；
- Linux 支持按两期推进：一期完成 Linux / WSLg 下 `showcase` 28 页完整可运行闭环，二期补齐 Wayland IME、X11 后端、`xdg-desktop-portal`、跨应用/跨窗 DnD、`accesskit_unix` 无障碍、托盘与截屏。

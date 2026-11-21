# ADR-0022：Win32 OLE 向外拖出、空闲 10fps/遮挡停绘与越界 Popup 宿主

- **状态**：已采纳（对应 `GAP-UI-003`、`GAP-UI-004`、`GAP-UI-005`）
- **影响 crate**：`unbound-platform`、`unbound-app`

## 背景

`crates/unbound-platform/src/overlay_host.rs` 已为桌面工作台预留了纯策略层：`FileDragDropBridge`、`OcclusionMonitor`、`OverlayWindowSpec`。但这三项尚未接通真实的 Win32 平台后端与 `window_runner` 主循环：
1. `backend.rs` 仅处理 `WM_DROPFILES` 拖入，缺少 COM/OLE `IDropSource` + `IDataObject(CF_HDROP)` + `DoDragDrop` 向外拖出；
2. `pacing.rs` 将 `MIN_FPS_LIMIT` 硬编码钳制为 `60`，且 `OcclusionMonitor` 未挂入 `window_runner`；
3. `OverlayManager` 仅在主窗口客户区内绘制弹层，右键菜单贴近屏幕边缘或副窗时会被主窗裁剪。

## 决策

1. **OLE 向外文件拖出（`GAP-UI-003`）**：
   - 在 `unbound-platform` 新增 `ole_drag` 子模块（`#[cfg(windows)]`）：实现最小 COM `IDropSource` 与 `IDataObject`（`CF_HDROP` UTF-16 双 `\0` 结尾路径数组），暴露 `pub fn start_external_file_drag(hwnd: isize, paths: &[PathBuf], allowed: FileDragEffect) -> Result<FileDragEffect, PlatformError>`；
   - 通过 `WindowCommand::StartFileDrag { paths, allowed }` 透出给 `unbound-app`，并在 `FileDragDropBridge` 保留无 Win32 依赖的单元测试替身。
2. **空闲降频与遮挡停绘（`GAP-UI-004`）**：
   - 将 `pacing.rs` 中的 `MIN_FPS_LIMIT` 降至 `10`（新增 `IDLE_FPS_LIMIT = 10` 常量，活跃态默认仍为 60/120Hz，不降低正常交互流畅度）；
   - 在 `window_runner.rs` 接入 `OcclusionMonitor`：最小化（`IsIconic`）或被上层不透明窗口完全遮挡时切换为 `PresentSync::Occluded`，跳过布局/渲染提交，仅保留事件循环与低频唤醒，恢复可见时自动 `request_full_rebuild()`。
3. **跨客户区边界 Popup 宿主（`GAP-UI-005`）**：
   - 复用 `OverlayWindowSpec::utility_floating`（`WS_POPUP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`），在 `unbound-platform` 提供 `PopupHost` 轻量无焦点弹出窗口；
   - 当 `OverlayManager` 打开标记为 `allow_overflow_host: true` 的顶层菜单且其计算矩形超出主窗客户区时，由 `window_runner` 将该菜单路由到 `PopupHost` 渲染，主窗保持激活焦点不闪烁。

## 后果

- 三项能力全部建立在既有 `overlay_host.rs` 策略结构上，保持平台无关单测与 Win32 真机调用清晰分层。

## 实现修订（2026-11-21）

交接补丁集落地后，本 ADR 中的函数名与宿主模块名与实现不一致；以代码为准的对照如下。

| 决策稿写法 | 实际 API |
| :--- | :--- |
| `start_external_file_drag` | `unbound_platform::begin_outbound_file_drag`（请求体 `OutboundFileDragRequest`，回执 `OutboundFileDragOutcome`）+ `build_cf_hdrop_payload` |
| 复用 `OverlayWindowSpec::utility_floating` 的 Popup 宿主 | `unbound_platform::popup_host::PopupHostRegistry` / `PopupWindowConfig` / `clamp_popup_to_work_area` / `PopupSyncAction` |
| 越界判定 | `overlay_host::{place_popup_in_work_area, should_use_popup_host}` |

回执经 `AppEvent::ExternalFileDragFinished` 回到应用，浮层关闭经 `AppEvent::NativePopupDismissed` 回流。

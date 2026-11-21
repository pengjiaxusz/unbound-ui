# ADR-0023：共享 `VkDevice` 的多窗口运行器与跨窗标签撕裂（tear-out）

- **状态**：已采纳（对应 `GAP-UI-002`，依赖 ADR-0022）
- **影响 crate**：`unbound-gpu`、`unbound-platform`、`unbound-app`

## 背景

`crates/unbound-app/src/window_runner.rs` 当前的 `WindowedRunner<A>` 与 `run_windowed` 假定整个进程只有单一窗口、单一 `Swapchain` 与单一 `AppRuntime`。桌面工作台（下游资产应用 多窗分屏、下游渲染工具 独立诊断浮窗）需要在同一进程内动态创建/销毁多个原生窗口，且支持把一个标签页拖出成为新窗口或拖入另一窗口停靠。

## 决策

1. **保持单窗 API 零破坏**：现有 `run_windowed` / `run_windowed_with_hooks` 签名与行为完全保留，内部可逐步复用多窗核心。
2. **共享 `Arc<Gpu>` + 独立每窗 `Surface` / `Swapchain`**：
   - 引入 `MultiWindowApp` trait 与 `run_multi_window` 入口；
   - 整个进程只初始化一次 `Arc<Gpu>`、`AtlasSet` 与 `Pipelines`，每个 `WindowId` 仅持有各自的 `PlatformWindow`、`Surface`、`Swapchain`、`AppRuntime`（或共享应用态 + 每窗视图路由）与 `OcclusionMonitor`，统一托管在 `MultiWindowHostPool` 中。
3. **跨窗拖拽与 tear-out 协调事件**：
   - `unbound-platform` 为指针事件补充屏幕绝对坐标 `screen_pos: Vec2` 与 `window_id: WindowId`；
   - 当拖拽越过当前窗口边界达到撕裂阈值，应用可发出 `MultiWindowCommand::SpawnWindow { spec, initial_route }` 创建副窗并接管拖拽；当指针落入另一窗口客户区释放时，发出 `AppEvent::CrossWindowDrop { source_window, target_window, screen_pos, payload }`。
4. **生命周期隔离**：
   - 关闭副窗仅销毁其 `Surface`/`Swapchain` 与 `WindowSlot`，绝不触发主窗退出；关闭最后一个主窗时安全等待 GPU 空闲并按序释放全部资源。

## 后果

- 下游资产应用 现有的 `下游资产应用-workspace::SlotTransfer`（跨窗槽位转移载荷）可直接无缝对接 `CrossWindowDrop` 与 `SpawnWindow`。

## 实现修订（2026-11-21）

交接补丁集落地后，多窗口入口以泛型协调器形式实现，与决策稿的 `MultiWindowApp` trait 不同。

| 决策稿写法 | 实际 API（`unbound-app::multi_window`） |
| :--- | :--- |
| `MultiWindowApp` trait | `MultiWindowFactory<A>`（`FnMut(WindowId, &WindowConfig) -> A` 自动实现） |
| `run_multi_window` | `run_multi_window`，配 `MultiWindowCoordinator<A>` 与每窗 `WindowContext<A>` |
| `MultiWindowHostPool` | `unbound_platform::overlay_host::MultiWindowHostPool<C>`（宿主侧窗口池） |

跨窗拖拽/撕出仍走 `MultiWindowCommand` 与 `CrossWindowDrop`；遮挡与降频由每窗 `pacer` 独立判定。

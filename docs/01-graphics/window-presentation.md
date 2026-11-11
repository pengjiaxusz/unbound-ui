# 窗口呈现与零黑边

> **实现状态**：`部分落地`
>
> | | |
> | :--- | :--- |
> | **已落地** | Vulkan surface 与 swapchain 创建、acquire → render → present 循环、非阻塞缩放状态机、输入到光子延迟计量、winit 事件层 |
> | **缺口** | DWM 实际行为未在真机测量；零黑边仍是设计目标而非实测结论 |
> | **源码** | `crates/unbound-platform/src/{resize,pacing,latency,backend,window}.rs`、`crates/unbound-gpu/src/surface.rs` |
> | **验收** | `apps/showcase` 开窗运行；`--shot` 模式可无头出图 |

## 呈现链路

`unbound-gpu` 现在能真正把画面送上屏幕：

```
  PlatformApp::frame()
        │
        ├─ swapchain.acquire()          → (image_index, image_view)
        ├─ renderer.render_to_view()     → 在借来的交换链图像上直接绘制
        └─ swapchain.present(finished)   → 等待 render_finished 信号量后呈现
```

- `Surface::new_win32` 用 `vkCreateWin32SurfaceKHR` 把窗口的 `HWND` 交给
  Vulkan。
- `Swapchain` 负责创建、`acquire` 与 `present`；呈现时用
  `vkWaitForFences` + `PresentInfoKHR.pWaitSemaphores` 等到渲染完成的那张图。
- `Renderer::render_to_view` 借用交换链图像作为颜色附件，同时跳过自己离屏
  目标的那道颜色屏障——所以窗口里画出来的和 `--shot` 离屏画出来的是同一条
  路径的同一条代码。
- 提交走 `vkQueueSubmit2`：时间线信号量（帧计数）与二元信号量（呈现栅栏）
  可以在同一次提交里混合使用，这正是 `vkQueueSubmit` 做不到的。

## 模态缩放阻塞破除

在 Windows 平台下，脱离默认阻塞式 Win32 模态 `WM_SIZE` / `WM_MOVESIZEMOVE`
消息循环（采用独立渲染提交线程或 Win32 Fiber 钩子），避免拖拽窗口边缘时
主线程挂起。

`unbound-platform` 的 `ResizeTracker` 实现了这一策略：拖拽期间
（`on_drag_started` → `on_drag_ended`）交换链保持不动，只更新待定尺寸；
尺寸在**稳定窗口期**内连续不变才提交重建，从而避免每一帧重建交换链导致的
闪烁。

### 非阻塞缩放状态机

```
       尺寸变化                      稳定窗口期内尺寸不再变化
  ──────────────►  pending  ─────────────────────────────►  committed
                        │                                        │
                        │ 拖拽中则继续等待                        │ 重建交换链
                        ▼                                        ▼
                   继续吸收变化                            提交新 extent
```

`ResizeTracker` 的三个出口动作分别是 `ResizeAction::None`（无需动作）、
`ResizeAction::Rebuild`（重建交换链）、`ResizeAction::Defer`（推迟到下一次
轮询）。

## 零黑边同步与低延迟呈现

- 启用 `VK_EXT_swapchain_maintenance1` 与 `VK_KHR_present_wait`（或
  DirectComposition 视觉表面绑定），使窗口尺寸改变时 Swapchain 重建与桌面
  窗口管理器（DWM）合成严格同帧对齐，实现 $120\text{Hz}$ 拖拽窗口边缘零黑块、
  零画面拉伸抖动。
- 利用可等待交换链图像就绪信号量控制 CPU 采样输入时刻，将 Input-to-Photon
  延迟压缩至小于 $8.3\text{ms}$。

### 呈现策略的单一决策点

平台层与渲染器必须对呈现模式达成一致，否则 CPU 会等待一个 GPU 从未执行的
present。`renderer_present_config` 是把两者绑在一起的唯一位置：

```rust
pub fn renderer_present_config(
    extensions: &[String],
    allow_tearing: bool,
) -> unbound_gpu::PresentConfig {
    unbound_gpu::PresentConfig::from_capabilities(extensions, allow_tearing)
}
```

`PresentWaitPolicy::from_extensions` 决定 CPU 侧行为（`WaitForPresent` 或
`Immediate`），`PresentConfig::from_capabilities` 决定渲染器侧行为，两者共用
同一份扩展列表输入。

## 输入到光子延迟计量

`LatencyTracker` 逐帧记录从输入事件产生到该帧呈现的时间戳，统计中位数与
最差值，对照 $8.3\text{ms}$ 预算（`LATENCY_BUDGET`）。该计量不依赖任何
假设——它测的是真实的时间戳差。

## 已知缺口

呈现链路已经接通，剩下的是**实测**层面的缺口：

| 缺口 | 影响 | 后续 |
| :--- | :--- | :--- |
| DWM 实际行为未验证 | 零黑边为设计目标而非实测 | 需在 Windows 真机测量 |
| 缩放时的交换链重建未做压力测试 | 极端拖拽下可能出现一两帧撕裂 | 需要真机长时间拖拽验证 |
| `VK_KHR_present_wait` 尚未接入 | 低延迟路径仍走 FIFO 等待 | 接入可等待交换链图像就绪信号量 |

---

**相关文档**：[核心 API](api-and-features.md) · [组件展台](../06-tooling/showcase.md) ·
[无头验证](../05-agent/headless.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

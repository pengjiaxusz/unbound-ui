# 窗口呈现与零黑边

> **实现状态**：`部分落地`
>
> | | |
> | :--- | :--- |
> | **已落地** | 呈现模式策略判定、非阻塞缩放状态机、输入到光子延迟计量、winit 事件层 |
> | **缺口** | Vulkan surface 与真实 swapchain 尚未创建；GPU crate 仍为离屏渲染器 |
> | **源码** | `crates/unbound-platform/src/{resize,pacing,latency,backend,window}.rs`、`crates/unbound-gpu/src/swapchain.rs` |

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

当前 `unbound-platform` 已具备完整的**策略层**（可测、可断言），但尚未接通
真实 swapchain：

| 缺口 | 影响 | 后续 |
| :--- | :--- | :--- |
| 无 `VkSurface` 创建 | 渲染器只能离屏出图 | 需接入 `VK_KHR_surface` / `VK_KHR_win32_surface` |
| 无交换链图像获取与呈现 | `Renderer` 不直接上屏 | 需实现 acquire → render → present 循环 |
| DWM 实际行为未验证 | 零黑边为设计目标而非实测 | 需在 Windows 真机测量 |

---

**相关文档**：[核心 API](api-and-features.md) · [无头验证](../05-agent/headless.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

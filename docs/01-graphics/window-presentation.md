# 窗口呈现与零黑边

> **实现状态**：`部分落地`
>
> | | |
> | :--- | :--- |
> | **已落地** | Vulkan surface 与 swapchain 创建、acquire → render → present 闭环、延迟重建策略、`VK_KHR_present_wait` 低延迟路径、事件循环自驱动、输入到光子延迟计量、多窗口 Surface/Swapchain 宿主、AccessKit 实机适配 |
> | **缺口** | DWM 实际行为未在真机测量；零黑边仍是设计目标而非实测结论 |
> | **源码** | `crates/unbound-platform/src/{resize,pacing,latency,backend,window,a11y_winit}.rs`、`crates/unbound-gpu/src/surface.rs`、`crates/unbound-app/src/window_runner.rs` |
> | **验收** | `apps/showcase` 开窗运行（`Ctrl+N` 开第二窗口）；`--shot` 模式可无头出图；`the_borrowed_attachment_path_matches_the_offscreen_path` 逐像素比对窗口路径；`unbound-app` 的 10 个示例逐个跑 `--headless-check` 与开窗烟测 |

## 呈现链路

`unbound-gpu` 现在能真正把画面送上屏幕：

```
  PlatformApp::frame()
        │
        ├─ renderer.wait_for_previous_frame()   → 上一帧退役，信号量槽位可复用
        ├─ swapchain.acquire()                  → (index, image, view, available)
        ├─ renderer.render_to_view()             → 在借来的交换链图像上直接绘制
        ├─ swapchain.present(finished)           → 等待 render_finished 信号量后呈现
        └─ pacer.on_present() + request_redraw() → 帧节奏由应用自己驱动
```

- `Surface::new_win32` 用 `vkCreateWin32SurfaceKHR` 把窗口的 `HWND` 交给
  Vulkan。
- `Swapchain` 负责创建、`acquire` 与 `present`；呈现时用
  `PresentInfoKHR.pWaitSemaphores` 等到渲染完成的那张图。
- `Renderer::render_to_view` 借用交换链图像作为颜色附件，并把它从
  `UNDEFINED` 转成 `COLOR_ATTACHMENT_OPTIMAL`——**视图不能做布局转换，
  必须拿得到图像本身**。
- 提交走 `vkQueueSubmit2`：时间线信号量（帧计数）与二元信号量（呈现栅栏）
  可以在同一次提交里混合使用，这正是 `vkQueueSubmit` 做不到的。

### 三条同步不变量

窗口路径的正确性依赖三条不变量，少一条就会出现"画面偶尔错乱"或者"整个
程序卡死"：

| 不变量 | 违反后的后果 |
| :--- | :--- |
| `vkAcquireNextImageKHR` 必须拿到一个有效的信号量 | 两个句柄都为空是**非法用法**。有的驱动会容忍，但同步语义就没有了 |
| 渲染提交必须等待 `image_available` 信号量 | 渲染可能与呈现引擎抢同一张图像 |
| 再次 `acquire` 之前，上一帧的信号量必须已被消费 | 二元信号量只允许一个挂起信号；重复 signal 会**丢信号**，下一帧永远等不到 |

第三条是最隐蔽的一条：信号量池的深度等于「在途帧数」，槽位一个帧周期后才会
复用。因此**必须在 acquire 之前**等上一帧退役（`Renderer::wait_for_previous_frame`），
否则 GPU 还没开始执行上一帧的提交时，二次 signal 会把第一次的信号覆盖掉。

### 两个宿主必须共用同一条链

`apps/showcase` 与 `unbound-app::WindowedRunner` 是两条独立的窗口宿主。上面
的不变量只在 showcase 里落实是不够的：后者每漏一步，故障就沿链路往后移一格，
且前一步的 abort 会把后面几步全部遮住。宿主侧必须遵守的完整清单：

| 环节 | 漏掉的后果 | 落点 |
| :--- | :--- | :--- |
| 创建 `Gpu` 前用 `display_handle()` 派生实例扩展 | `VK_KHR_swapchain` 缺 `VK_KHR_surface`，且 `vkCreateWin32SurfaceKHR` 函数指针加载失败直接 abort | `window_instance_extensions()`；`Gpu::new` 再做一次依赖反推 |
| 交换链重建后同步深度附件 | 深度图小于 `renderArea`（`VUID-...-06079/06080`），驱动可能直接丢设备 | `ensure_depth_size(swapchain.extent())`；`Renderer::render_external` 另有按借用图像尺寸的兜底 |
| acquire 前等上一帧 fence | acquire 信号量槽位被提前复用（`VUID-...-01779`） | `wait_for_previous_frame()` 在 `acquire_outcome()` 之前 |
| 设备丢失恢复时显式销毁 | Vulkan 对象没有 `Drop`，旧 `VkSurfaceKHR` 仍占着 HWND，重建报 `ERROR_NATIVE_WINDOW_IN_USE_KHR` | 按 swapchain → surface → renderer 顺序 `destroy()` |

深度附件按 256 px 步进增长、只增不减：动态渲染允许附件大于 `renderArea`，
多出的行列不参与绘制，因此拖拽时不会因为每像素变化而重新分配显存。

### 多窗口

平台层现在按 `WindowId` 维护一组成员：每个窗口有自己的 `PlatformState`
（resize tracker、pacer、IME）、自己的 `WinitA11y` 适配器，以及应用侧自己的
`Surface` + `Swapchain`。设备、渲染器、管线与字形图集仍然共享：

- `PlatformApp::take_window_request` 申请窗口，平台在下一帧创建后调用
  `window_created`；关闭非主窗口只移除该成员并调用 `window_closing`，主窗口
  关闭才退出事件循环。
- 所有窗口共用一个 `Renderer` 时，实例池只有一份基线，因此每个窗口都要用
  新的 `generation` 全量上传；只有单窗口时才走局部补丁路径。
- `Renderer::ensure_depth_size` 只增不减，两个窗口按各自尺寸绘制前调用即可
  共享同一张深度附件。

### 事件循环必须自驱动

`winit` 的默认 `ControlFlow::Wait` 会把事件循环**睡到下一个输入事件**才醒。
一个只在有输入时才画画的窗口，看起来就是冻住了。平台层因此在每帧结束时按
帧节奏设置 `ControlFlow::WaitUntil(下一次出图时刻)`，由应用在呈现后调用
`request_redraw()`。

### 低延迟呈现与它的降级

`VK_KHR_present_wait` 让 CPU 能等到「图像真的上了屏」。实现要点：

- 呈现时把 `VkPresentIdKHR` 挂进 `VkPresentInfoKHR` 的 `pNext`，用一个
  单调递增的非零标识符标记本次呈现（规范要求它大于此前所有标识符）。
- `vkWaitForPresentKHR` 的等待**必须有上限**：窗口没有被合成时（被遮挡、
  最小化、无桌面会话），呈现永远不会完成，无限等待会把帧循环彻底卡死。
- 连续几次超时之后，低延迟路径**自动关闭**——等一个永远不会到来的呈现，
  只会给每一帧白白加上一个超时。

## 零黑边：延迟重建交换链

拖拽窗口边缘会产生一串 `WM_SIZE`。每收到一个就重建交换链，正是黑边与拉伸
画面的来源：新的交换链还没准备好，合成器已经在按新的窗口尺寸取帧了。

`unbound-platform` 的 `ResizeTracker` 把这个策略落到了实处：

| 阶段 | 行为 |
| :--- | :--- |
| 尺寸变化 | 只**记录**新尺寸，不重建 |
| 稳定窗口期内 | 继续用旧交换链出图，合成器把最后一帧缩放到新窗口尺寸——既没有黑边，也没有重建风暴 |
| 尺寸稳定后 | 重建**一次**交换链，尺寸精确 |

有一个容易漏掉的细节：**稳定窗口需要一个不由事件驱动的 tick**。一次拖拽的
最后一条 `WM_SIZE` 之后如果不再有事件，那个尺寸就永远不会被提交，窗口会一直
停在旧尺寸上。所以 `PlatformState::poll_resize` 必须每帧调用。

绘制尺寸取自**已提交的**交换链尺寸（`Swapchain::extent()`），而不是窗口当前
的尺寸——这正是"拖拽期间用旧尺寸渲染"这条策略的落点。

### 请求尺寸与链尺寸是两件事

窗口尺寸是**请求**，交换链尺寸是**驱动给的答案**，两者不可混为一谈：

| 量 | 来源 | 用途 |
| :--- | :--- | :--- |
| 请求尺寸 | `window.inner_size()` | 只用来判断**要不要重建**交换链 |
| 链的实际尺寸 | `Swapchain::extent()` | 布局视口与 UI 相机的**唯一**来源 |

`resolve_extent` 取驱动的 `currentExtent` 而不是请求值——驱动会按自己的上限
裁剪，也可能与请求差一个像素。于是把链的实际尺寸拿去和请求尺寸比较，等于
每帧都判定"尺寸变了"，重建永不收敛；而拿窗口尺寸去布局、拿链尺寸去投影，
则是两张不同的画面：右侧或底部要么留白，要么被裁掉。

正确做法是让**所有**呈现相关量都从链的实际尺寸推导：布局视口
（`physical / (OS scale × app zoom)`）与 UI 相机共用同一个 extent，因此两者
永远描述同一张图。

## 上传必须与"已提交"绑定

交换链重建之外还有一条同源的纪律：**代次（generation）只能在帧真正到达
GPU 之后才被消费**。

`AppRuntime::frame` 每次重建都会前移差分基线，无论这一帧是否画了出来。一帧
被丢掉（交换链过期、渲染失败）时，池里的内容就落在了基线后面；此时若仍按
差分的切片去补丁，修的是错位的字节，而下一帧又会报"没有变化"——那一块就
永远停在旧画面上。

因此：

- 渲染器侧：`UploadGeneration` 分"已提交"与"已暂存"两个态，代次在提交成功
  之后才推进。
- 宿主侧：上一帧被丢掉时，下一次上传强制走**全量**，把池拉回基线。

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
| 无头会话下窗口不会被合成 | 呈现永远不完成，低延迟等待只能超时降级 | 需要有桌面会话的机器实测呈现时序 |

---

**相关文档**：[核心 API](api-and-features.md) · [组件展台](../06-tooling/showcase.md) ·
[无头验证](../05-agent/headless.md)

**返回**：[01-graphics](README.md) · [文档索引](../README.md)

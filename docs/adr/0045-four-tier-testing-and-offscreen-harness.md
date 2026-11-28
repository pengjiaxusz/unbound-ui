# ADR-0045: 四级测试分层架构与通用 L3 离屏渲染测试宿主

Status: accepted

## 背景

`unbound-ui` 已具备基于 `unbound-headless` 的零 GPU 无头几何/交互校验能力，并在 `apps/showcase`（`shot.rs`、`parity.rs`）与 `unbound-gpu` 内部构建了单实例离屏渲染器。但在面向下游宿主应用提供测试支撑时，暴露出明显的架构断层：

1. **L3 离屏渲染能力未下沉开放**：下游项目目前只能用 `AppRuntime` 跑到图元发射阶段（L2），无法在不弹出真实窗口的情况下验证外部纹理上传、透明遮挡、视口裁剪与自定义 3D 渲染结果，导致大量渲染/显影缺陷被迫留到真机人工点击才暴露。
2. **L2/L3 高频交互驱动缺乏统一抽象**：`AppRuntime` 上缺少开箱即用的 `drag`、`wheel`、`key_tap`、`resize` 等高层交互驱动原语，下游测试往往要手动拼接底层事件序列，容易绕过真实的鼠标防抖或命中链路。

## 决策

1. **确立 L1–L4 四级测试分层模型**：
   - **L1（逻辑单测）**：纯数据/数学/状态机测试，零 UI 与 GPU 依赖。
   - **L2（无头交互）**：驱动真实 `AppRuntime` 与 `unbound-headless` 五项几何体检，零 GPU 与零显示器依赖；保持 `crates/unbound-headless` 零图形依赖的纯 CPU 定位不变。
   - **L3（离屏渲染）**：在 `crates/unbound-app` 下通过 `offscreen-test` feature 提供 `unbound_app::testing::OffscreenTestHarness<A: Application>`，不创建操作系统窗口与交换链，完整驱动 `AppRuntime`、`on_before_render`、`AssetHub` 外部纹理上传与 `CustomViewportPass` 并回读 `RGBA8` 像素。
   - **L4（真机窗口）**：仅覆盖 L3 无法替代的原生平台边界（多原生窗口拖拽停靠、Win32 IME `VK_PROCESSKEY`、OLE 拖放、系统托盘、DWM 原生后景合成与跨屏 DPI）。

2. **L3 离屏设备共享与无物理 GPU 兼容**：
   - 进程内共享单实例无头 `VkInstance / VkDevice`（`OnceLock` 初始化，GPU Pass 互斥串行提交），每个 `OffscreenTestHarness` 实例保有完全独立的 `RenderTarget`、`AssetHub` 与 `AppRuntime` 状态，兼容多线程并行 `cargo test`。
   - 优先枚举硬件 Vulkan GPU；在无物理 GPU 或无显示器的云端/CI 环境中自动回退至 Lavapipe / SwiftShader 等软光栅 Vulkan ICD（两者均不可用时显式跳过或报错）。

3. **L3 轻量像素断言定位与 L2/L3 统一驱动**：
   - L3 对外提供轻量结构与像素不变量断言（如区域非纯色/方差检测、前后帧局部差分、遮挡/纹理上屏核验与失败 PNG 截图落盘），不强制下游接入 `showcase` 的参考截图拼版宿主。
   - 在 `unbound_app::testing` 中提供统一交互驱动原语（`click`、`drag`、`wheel`、`key_tap`、`resize`、`step_frames`），使下游测试从 L2 升级至 L3 几乎零改写。

## 权衡与备选方案

- **将 L3 放进 `unbound-headless`（加 `gpu` feature）**：会破坏 `unbound-headless`“零图形依赖、亚毫秒 CI 校验”的核心承诺，且容易引入 `unbound-app` 与 `unbound-headless` 的循环依赖，故否决。
- **新建独立 `unbound-testkit` crate**：需要将 `unbound-app` 内部大量 `pub(crate)` 帧调度与 `AssetHub` 生命周期字段暴露为 `pub`，增加跨 crate 维护面；置于 `unbound-app`（`offscreen-test` feature）可由下游一行 `[dev-dependencies]` 直接启用。
- **每测试独立创建 Vulkan 设备**：初始化开销高且并发下易触发驱动锁死或显存不足，因此采用进程级共享单实例设备 + 测试级隔离状态。

## 影响

- 下游宿主应用可将 80% 以上的渲染遮挡、纹理上屏与裁剪回归测试前移至 L3 无窗自动化门禁，大幅减少对 L4 真机人工测试的依赖。
- `GLOSSARY.md` 与 `docs/architecture/03-扩展系统（subsystems）/无头验证与展台（verification-and-showcase）.md` 同步更新四级测试术语与能力边界。

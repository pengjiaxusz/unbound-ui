# 调试与安全工具链

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-gpu/src/debug.rs`、`crates/unbound-app/src/inspector.rs` |
> | **关键类型** | `RenderDoc`、`ValidationCounter`、`RecoveryPolicy`、`VALIDATION`、`Inspector`、`InspectorScene` |

## 应用内 RenderDoc 编程钩子

运行时动态探测并挂载 `renderdoc.dll` / `librenderdoc.so` API 指针。

支持通过快捷键或 Agent 测试指令在特定 Action 前后自动调用
`StartFrameCapture` / `EndFrameCapture`，并在每个渲染阶段注入
`vkCmdBeginDebugUtilsLabelEXT` 标记，自动断言单帧 MDI DrawCall 数量与显存池碎片
率。

`RenderDoc::load()` 返回 `Option<Self>`——未安装 RenderDoc 时静默降级，不影响
正常运行。`set_capture_path` 指定捕获输出模板，`is_usable` 判断当前是否可捕获。

> `begin_capture` / `end_capture` 标记为 `unsafe`：窗口句柄被直接交给注入库，
> 不做额外校验，调用方必须保证句柄有效且捕获配对。

## Vulkan Validation Layers (VVL) CI 自动化守卫

在 CI 管线中结合软件光栅化器（Lavapipe / SwiftShader）或 GPU Runner 强制开启
全套 VVL（含 GPU-Assisted Validation 与 Synchronization Validation），对 BDA
物理指针越界、描述符竞争及管线屏障缺失实施零容忍拦截。

`ValidationCounter` 记录验证层消息：

| 方法 | 用途 |
| :--- | :--- |
| `record(severity, message)` | 记录一条消息 |
| `errors()` / `warnings()` | 计数 |
| `last_message()` | 最近一条消息 |
| `assert_clean()` | 有错误时返回 `(错误数, 警告数, 最近消息)` |

### 实践中修掉的三个真实缺陷

VVL 守卫不是摆设——开发期间它抓出了：

1. **查询池未启用 `hostQueryReset`**：命令缓冲区内重复重置查询池被验证层
   拦截。修复为启用 Vulkan 1.2 的 `hostQueryReset` 特性。
2. **时间戳精度丢失**：绝对时间戳（量级 $10^{18}$）先转 f32 再求差，把 2 ms 的
   差值直接抹成 0。改为先在 u64 上求差再换算。
3. **scatter 着色器的单线程串行拷贝**：不是验证层发现的，是分段时间戳定位
   出来的。

## 运行时 DevTools Inspector 与诊断闭环

> **源码**：`crates/unbound-app/src/inspector.rs`

按 `F12` 切换 Inspector 覆盖层。它不进入布局树，而是直接向场景收集器追加
图元，并画在调试通道（最高 `z`，最后绘制），因此开关不会触发一次重新排版。

- 指针悬停通过 `pick_stack` 命中当前帧的 hit region；落在非交互空隙时回退到
  最深的包含节点，因此普通容器同样能看盒模型。
- 盒模型用四条描边带显示 margin / border / padding / content，颜色分别取
  `WARNING`、`ACCENT`、`SUCCESS`、`PRIMARY`。
- 右侧面板给出 `SourceLoc`、语义 key、矩形与违规列表；文本行由宿主用自己
  的字体绘制（`InspectorScene::lines`），面板本身只画背景与行标记。

### 调试体检闭环

`AppRuntime::enable_debug_sanitizers(true)` 打开后，每帧把 `UiWorld` 镜像成
`unbound_headless::Frame`（节点 id 直接使用语义 key），交给
`Harness::run` 跑五条 sanitizer。违规项以闪烁的 `ERROR` 色框叠加，并写入面板
文本，形成“改代码 → 渲染 → 发现违规”的闭环。

```rust
runtime.enable_debug_sanitizers(true);
runtime.key(AppKey::F(12), true, Modifiers::default());
let report = runtime.frame(1.0 / 60.0);
assert!(!runtime.inspector().violations().is_empty());
```

> 该开关是调试功能：每帧多一次整树遍历与五条规则扫描，正式发布时应关闭。

## 设备丢失恢复

`is_device_lost(err)` 判断错误是否设备丢失；`recover_with` 执行恢复策略：

```rust
pub fn recover_with<T, F>(...) -> ...
```

`RecoveryPolicy` 描述恢复行为。恢复过程保持纯内存领域状态树完好，原子化重建
`VkDevice`、显存池与管线，将状态树原地重新灌注至新持久池。

---

**相关文档**：[核心 API](../01-graphics/api-and-features.md) · [热重载](hot-reload.md)

**返回**：[06-tooling](README.md) · [文档索引](../README.md)

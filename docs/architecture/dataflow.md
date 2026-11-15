# 架构：dataflow
状态：已落地
路径：`crates/unbound-app/`、`crates/unbound-abi/`
入口：`crates/unbound-app/src/lib.rs::AppContext`、`crates/unbound-abi/src/lib.rs`
守卫：`cargo test -p unbound-app --test runtime_headless --test runtime_subsystems`

## 范围
- [负责] 串联 OS 输入事件、状态归约、增量排版、场景发射与 GPU 呈现的全链路闭环
- [不负责] 窗口后端事件采集 -> 委托给 `crates/unbound-platform/`；GPU 管线执行 -> 委托给 `crates/unbound-gpu/`

## 拓扑与流程
- 主链路：`PlatformWindow` 事件 -> `AppEvent` -> `Application::reduce` [同步状态归约] -> `UiWorld` 标脏 -> `LayoutEngine::solve_boundaries` -> `SceneCollector` 发射图元 -> `diff` 局部补丁 / 全量上传 -> `Renderer::render` [GPU 剔除+64位基数排序+MDI] -> 呈现
- 静止态调度：无输入事件、无活跃 `Spring` 动画、无异步资源就绪时挂起帧渲染，CPU/GPU 零空转
- 并发：主线程单线程驱动 `UiWorld`/`LayoutEngine`/`SceneCollector`；资源解码与着色器异步任务通过有界队列回传主线程
- 跨线程唤醒：`AppContext::event_sender` -> `AppEventSender{ Sender<AppEvent>, Option<Arc<dyn AppWaker>> }` -> `send` / `send_custom` 入队后立即 `wake()` -> 下一帧 `poll_async` 投递；新增 `AppEvent::{UiScaleChanged, Custom}` -> [必须] 新增变体须同步所有 `Application::reduce` 穷尽 match（`apps/showcase`、`examples/*`、`tests/*`），否则 E0004 -> [守卫] `just check`
- UI 缩放与 OSD：`AppContext::{ui_scale, scale_osd_remaining}` 经 `set_ui_scale` / `step_ui_scale` 吸附 `unbound_theme::presets::CANONICAL_SCALE_STEPS`（7 档 `0.75..2.0`）并激活 1.5s OSD；`I18nBundle::format` 兼容 `{name}` 与 `{{name}}`，`t_ns` 按 `命名空间:键` / `命名空间.键` 回退，`key_count` 供词条审计
- 声明式发射扩展：`WidgetSpec` 新增 `TextInline`（单行自适应宽度文本 + 语义色令牌）、`BoxSurface`（容器填充/边框/阴影圆角表面）、`ScrollScopeBegin`/`ScrollScopeEnd`（裁剪与平移作用域）四类变体；`ViewBuilder::container` 依据 `RawNodeDesc.{fill, border_color, radius, shadow_blur}` 自动补发表面图元，`scroll_view_with` 以独立内容节点配合 `push_clip`/`push_transform` 建立滚动作用域；`PendingWidget.z_base` 与树深度 `depth` 共同决定图元 Z（`80 + 6 * depth`，上限 480），浮层（`z_index >= 1000`）经 `apply_z_base` 抬升图元 Z 与 `HitRegion.layer/z` -> [守卫] `just check` + `just test`
- 滚动输入闭环：`AppRuntime::scroll_regions` 每帧重建，`wheel` 逆序命中包含指针的滚动视口后以 44px 步长驱动 `ScrollState::jump_to`，位移有效时置 `force_rebuild` 并标脏 `PAINT|LAYOUT`，同时仍照常派发 `AppEvent::Wheel` -> [位置] `crates/unbound-app/src/lib.rs::AppRuntime::wheel`
- 表单状态与声明式构建器：`AppContext` 新增 `text_input_value`/`select_index`/`switch_value`/`slider_value`/`number_input_value`/`tab_active` 及对应 `set_*`，按控件键直接读写 `WidgetStateStore` 中的 `*State`（不标记视图已渲染，供宿主预填与无头断言）；`ViewBuilder` 补齐 15 个声明式控件构建器与 `modal_scrim`/`drawer_right` 浮层容器 -> [位置] `crates/unbound-app/src/lib.rs`
- 上传契约：`SceneUpload` 的 `transforms`/`clips`/`payloads` 均为全局数组；空 `transforms` 由 `Renderer::prepare_upload` 补 identity；图集更新整帧一次批量上传
- 描述符构造器契约：`RawNodeDesc` 的绘制/布局字段统一由 `unbound-abi` 的 builder 写入（`fill(impl Into<u32>)`、`border(impl Into<u32>, f32)`、`flex_grow(f32)`、`z_index(i32)`、`inset(Edges)`、`shadow(u32, f32, f32)`），颜色参数兼容 `Token::raw()` 的 `u16`；调用方不得把字段当方法调用，也不得臆造不存在的关联项 -> [守卫] `just check`

## 取舍（禁止重构）
- [代价] 采用 Action as Data（可序列化枚举 `ActionKind`）替代闭包回调 -> [原因] 跨越 Host-Guest C-ABI 边界、支持确定性回放与 `rkyv` 时间旅行快照 -> [位置] `crates/unbound-abi/src/action.rs::ActionKind`
- [代价] 帧间使用双缓冲场景差分（`diff`）仅上传变更区段 -> [原因] 悬停/聚焦等高频微交互避免全量重建 GPU 实例缓冲 -> [位置] `crates/unbound-widgets/src/diff.rs`

## 不变量与红线
- [禁止] 在 `view()` 视图构建阶段直接修改业务状态或触发 I/O -> [原因] 保证视图发射纯函数化与无头可复现 -> [守卫] `crates/unbound-app/tests/runtime_headless.rs`
- [必须] 每帧提交前通过 `SceneCollector::push_clip`/`pop_clip` 配对校验 -> [原因] 防止裁剪栈失衡污染后续图元 -> [守卫] `crates/unbound-widgets/src/lib.rs::SceneCollector`
- [必须] 场景发射侧索引在跨 widget 合并后仍为全局索引 -> [原因] 着色器无偏移换算，局部索引会串读相邻数据 -> [守卫] `crates/unbound-gpu/src/frame.rs`
- [必须] 窗口 host 同步的外部窗口状态（如 `window_maximized`）必须在 `AppRuntime::frame` 之前经 `AppRuntime::context_mut` 写入 `AppContext`，再由 `ViewContext` 快照传给 `ViewBuilder` -> [原因] 视图在 `frame` 内构建，帧末写入会让默认标题栏的最大化/还原图标滞后一帧；视图阶段只读上下文、不得回写业务状态 -> [位置] `crates/unbound-app/src/window_runner.rs`、`crates/unbound-app/src/lib.rs`

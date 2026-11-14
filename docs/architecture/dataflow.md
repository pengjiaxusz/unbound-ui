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
- 上传契约：`SceneUpload` 的 `transforms`/`clips`/`payloads` 均为全局数组；空 `transforms` 由 `Renderer::prepare_upload` 补 identity；图集更新整帧一次批量上传

## 取舍（禁止重构）
- [代价] 采用 Action as Data（可序列化枚举 `ActionKind`）替代闭包回调 -> [原因] 跨越 Host-Guest C-ABI 边界、支持确定性回放与 `rkyv` 时间旅行快照 -> [位置] `crates/unbound-abi/src/action.rs::ActionKind`
- [代价] 帧间使用双缓冲场景差分（`diff`）仅上传变更区段 -> [原因] 悬停/聚焦等高频微交互避免全量重建 GPU 实例缓冲 -> [位置] `crates/unbound-widgets/src/diff.rs`

## 不变量与红线
- [禁止] 在 `view()` 视图构建阶段直接修改业务状态或触发 I/O -> [原因] 保证视图发射纯函数化与无头可复现 -> [守卫] `crates/unbound-app/tests/runtime_headless.rs`
- [必须] 每帧提交前通过 `SceneCollector::push_clip`/`pop_clip` 配对校验 -> [原因] 防止裁剪栈失衡污染后续图元 -> [守卫] `crates/unbound-widgets/src/lib.rs::SceneCollector`
- [必须] 场景发射侧索引在跨 widget 合并后仍为全局索引 -> [原因] 着色器无偏移换算，局部索引会串读相邻数据 -> [守卫] `crates/unbound-gpu/src/frame.rs`

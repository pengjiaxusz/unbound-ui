# 架构：headless-and-agent
状态：已落地
路径：`crates/unbound-headless/`、`crates/unbound-app/`
入口：`crates/unbound-headless/src/lib.rs::Harness`、`crates/unbound-app/src/inspector.rs::Inspector`
守卫：`cargo test -p unbound-headless --test driver` / `cargo test -p unbound-app --test runtime_headless`

## 范围
- [负责] 脱离窗口与 GPU 的纯内存无头闭环验证、5 项几何体检规则（`Rule`）、源码行号定位（`SourceLoc`）、`rkyv` 时间旅行快照与 `Inspector` 自省
- [不负责] Vulkan 真实像素对比 -> 委托给 `apps/showcase` `--shot` 截图模式

## 拓扑与流程
- 体检链路：`FrameBuilder` 收集 `NodeSnapshot`（含 `SourceLoc`） -> `Harness::run` 执行 `rules::check` 5 项几何断言（越界/重叠/零尺寸/裁剪失衡/对比度等） -> `Report::assert_clean`
- 交互测试：`TestApp` 驱动 `Application::{init,reduce,view}` 注入模拟输入事件并校验状态与 `A11yNode`
- 时间旅行：`snapshot::capture` 将帧状态序列化为 `rkyv` 零拷贝 `FrameSnapshot` -> `TimeTravel` 支持 `restore` 任意历史帧回放

## 取舍（禁止重构）
- [代价] `NodeSnapshot` 携带编译期捕获的 `SourceLoc`（文件/行号） -> [原因] AI Agent 无头检测到几何缺陷时可一跳定位源码，无需人工截图肉眼排查 -> [位置] `crates/unbound-headless/src/frame.rs::SourceLoc`
- [代价] 快照采用 `rkyv` 零拷贝反序列化而非 `serde_json` -> [原因] 满足无头单帧验证 <= 1.0ms（实测 544µs）预算 -> [位置] `crates/unbound-headless/src/snapshot.rs`

## 不变量与红线
- [禁止] `unbound-headless` 依赖 Win32 窗口后端、`ash` 或任何图形驱动 -> [原因] 保障无 GPU/无头 CI 与 Agent 亚毫秒自检 -> [守卫] `crates/unbound-headless/Cargo.toml`
- [必须] 无头单帧验证耗时 <= 1.0ms -> [守卫] `crates/unbound-kpi/src/lib.rs`

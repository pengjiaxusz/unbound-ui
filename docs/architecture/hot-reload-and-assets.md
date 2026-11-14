# 架构：hot-reload-and-assets
状态：已落地
路径：`crates/unbound-host/`、`crates/app-guest/`、`crates/unbound-asset/`
入口：`crates/unbound-host/src/lib.rs::GuestHost`、`crates/unbound-asset/src/lib.rs::AssetPipeline`
守卫：`cargo test -p app-guest --test host_roundtrip --test schema_evolution` / `cargo test -p unbound-app --test asset_pipeline`

## 范围
- [负责] Host-Guest C-ABI 动态库热重载、分配器劫持、双模状态迁移（零拷贝 vs `postcard` 字段演化）与后台异步资源解码流送
- [不负责] Guest 业务 UI 具体逻辑 -> 委托给 `crates/app-guest/src/lib.rs`

## 拓扑与流程
- 热重载链路：`GuestHost` 加载 `cdylib`（`GUEST_ENTRY_SYMBOL`） -> `check_abi` 校验 `ABI_VERSION` -> `classify_handoff` 比对 `StateSchema` 哈希：一致走零拷贝交接（<= 0.01ms），变更走 `migration::migrate`（`postcard` 字段级迁移）
- 分配器隔离：`HostAllocatorVTable` 将 Guest 堆分配代理至宿主分配器，防止卸载旧 DLL 时悬空释放
- 资源流送：`AssetPipeline` 后台线程池解码（`PngDecoder`/`RawDecoder`） -> 写入 `StagingRing` -> `TimelineGate` 与 `ownership_barrier` 队列所有权转移至渲染队列

## 取舍（禁止重构）
- [代价] Host-Guest 边界严禁传递 Rust `String`/`Vec`/Trait 对象，仅允许 `#[repr(C)]` POD 与 `HostAllocatorVTable` -> [原因] Rust 无稳定 ABI，防止编译器版本或布局差异导致未定义行为 -> [位置] `crates/unbound-abi/src/plugin.rs::GuestPluginVTable`
- [代价] 异步资源在后台线程解码并经由 `StagingRing` 交接 -> [原因] 主渲染线程零磁盘 I/O 与解码阻塞 -> [位置] `crates/unbound-asset/src/pipeline.rs::AssetPipeline`

## 不变量与红线
- [禁止] 在未比对 `state_schema_hash` 的情况下直接强转热重载前后的 `GuestState` 指针 -> [原因] 字段增删会静默破坏内存 -> [守卫] `crates/app-guest/tests/schema_evolution.rs`
- [必须] 卸载旧 Guest 动态库前确认所有 `HostNode` 生命周期已迁出 -> [守卫] `crates/app-guest/tests/host_roundtrip.rs`

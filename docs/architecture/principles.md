# 架构：principles
状态：已落地
路径：`crates/*`、`apps/*`
入口：`Cargo.toml`、`crates/app-guest/tests/layering.rs`
守卫：`just check` / `cargo test -p app-guest --test layering`

## 范围
- [负责] 定义全工作区依赖方向、扁平数据约束、热路径零分配与正交极简 API 红线
- [不负责] 具体子系统算法与平台细节 -> 委托给各 `./<module>.md`

## 拓扑与流程
- 依赖自底向上：`unbound-math` -> `unbound-abi` -> 核心能力层（`gpu`/`vector`/`text`/`world`/`layout`/`anim`/`canvas`/`input`/`a11y`/`theme`/`graph`/`asset`/`platform`） -> `unbound-widgets` -> `unbound-host` / `app-guest` -> `unbound-app` -> `apps/*`
- 编译隔离：`app-guest` 仅依赖 `unbound-abi`/`serde`/`postcard`，业务改动增量编译 <= 800ms（实测 282ms）

## 取舍（禁止重构）
- [代价] 禁止深层嵌套泛型 Typestate 视图树（如 `VStack<HStack<Button<...>>>`） -> [原因] 避免 Rust 单态化膨胀与 LLVM 慢编译，统一使用扁平 POD 描述符与 `NodeId` -> [位置] `crates/unbound-abi/src/desc.rs::RawNodeDesc`
- [代价] 每个属性/样式仅保留唯一正交声明方式，拒绝语法糖宏 -> [原因] 最小化 API 表面积，消除 AI Agent 生成代码时的 API 幻觉 -> [位置] `crates/unbound-widgets/src/lib.rs`

## 不变量与红线
- [禁止] `app-guest` 引入图形栈（`unbound-gpu`/`unbound-platform`/`ash`）或重型序列化库 -> [原因] 保持业务层热重载增量编译 < 800ms -> [守卫] `crates/app-guest/tests/layering.rs`
- [禁止] `unbound-gpu` 反向依赖上层 UI/应用框架 -> [原因] 保证渲染器可脱离窗口独立无头测试 -> [守卫] `Cargo.toml`
- [禁止] 热路径（每帧排版/剔除/发射/排序）发生堆内存分配 -> [原因] 每帧成本与场景规模无关 -> [守卫] `crates/unbound-kpi/src/lib.rs`
- [必须] 业务层状态结构体标注 `#[repr(C)]` 并由 `impl_schema_hash!` 生成布局指纹 -> [原因] 防止编译器重排字段导致热重载静默内存损坏 -> [守卫] `crates/app-guest/tests/schema_evolution.rs`

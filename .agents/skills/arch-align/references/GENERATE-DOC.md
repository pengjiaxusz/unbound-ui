
# 领域架构规范指南（`docs/architecture/<module>.md`）

用本指南编写或压缩专门供人工智能智能体消费的领域架构文件。

## 一、智能体原生写作规则

- **硬性长度上限**：每文件最多 **80 行**（目标 40~60 行）。
- **零散文**：不写完整句子、导语摘要或设计历史。用片段与符号（`->`、`!=`、`<=`、`::`）。
- **符号锚定**：每个论断必须指向真实路径或符号（`src/foo/bar.rs::Engine`）。
- **只记增量**：省略标准框架模式与显而易见的目录清单，只记录读单个文件看不出来的东西。
- **禁止表格**：表格在竖线、补齐与冗长表头上浪费令牌。改用紧凑的标签列表。

## 二、标准智能体原生模板

生成 `docs/architecture/<module>.md` 时使用以下精确骨架（标识符、路径与命令保留原文，其余说明一律中文）：

```markdown
# 架构：<module-name>
状态：已落地 | 部分落地 | 蓝图
路径：`src/xxx/`、`crates/yyy/`
入口：`src/xxx/mod.rs::PublicFacade`、`src/xxx/core.rs::CoreState`
守卫：`cargo test --test layering` | `pytest tests/arch/test_bounds.py`

## 范围
- [负责] <核心职责一 —— 电报式短语>
- [负责] <核心职责二>
- [不负责] <委托出去的关注点一> -> 委托给 `src/other/mod.rs`
- [不负责] <禁止承担的关注点二> -> 由外部调用方处理

## 拓扑与流程
- 主链路：`InputReq` -> `Validator::check` [同步] -> `CoreState::apply` [可变，单线程] -> `WalWriter::append` [异步，有界通道(1024)]
- 生命周期：`init(Config)` -> `run_loop()` -> `shutdown(drain_timeout=5s)`
- 并发：`CoreState` 由单个工作线程独占；外部调用方通过 `mpsc` 发送 `CmdEnum`；热路径零锁。

## 取舍（禁止重构）
- [代价] `StateNode` 禁止嵌套泛型 -> [原因] 避免单态化膨胀，增量编译 < 300ms -> [位置] `src/xxx/desc.rs`
- [代价] 回调只能使用可序列化的 `ActionId` 枚举 -> [原因] 确定性回放与快照恢复的前提 -> [位置] `src/xxx/action.rs`

## 不变量与红线
- [禁止] `src/xxx/core.rs` 引入 `src/xxx/adapter/*` -> [原因] 保持核心纯净、无输入输出 -> [守卫] `tests/layering.rs::core_no_io`
- [禁止] `process_frame()` 内堆分配 -> [原因] 帧预算 <= 1.0ms（实测 0.62ms） -> [守卫] `tests/alloc_guard.rs`
- [必须] 所有状态变更在返回前发出 `ChangeEvent` -> [守卫] `tests/invariants.rs::audit_trail`

## 扩展配方（可选，最多 5 行）
- 新增命令：1) 在 `src/xxx/action.rs::Cmd` 加分支 -> 2) 在 `CoreState::dispatch` 中处理 -> 3) 在 `tests/replay.rs` 补回放用例。
```

## 三、保存前压缩检查

- [ ] 文件是否不超过 80 行？
- [ ] 所有表格是否已移除，并替换为 `[负责]`、`[代价]`、`[禁止]` 标签列表？
- [ ] 每条 `[代价]` 是否说明了 `[原因]`，让智能体重构时不会把它“修好”？
- [ ] 新文件是否已登记到 `docs/architecture/README.md`？

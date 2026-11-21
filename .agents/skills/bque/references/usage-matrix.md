
# bque 命令判定矩阵与安全指南

## 1. 原生命令到 bque 调度的映射对照表

| 原始原生命令 | 推荐执行方式（接入 bque） | 底层 bque 模式 | 进程优先级类别 |
| :--- | :--- | :--- | :--- |
| `cargo build` | `just build` | `bque run` | `Idle`（编译子进程树全量继承） |
| `cargo check --workspace` | `just check` | `bque run` | `Idle` |
| `cargo clippy -- -D warnings` | `just clippy` 或 `bque run -- cargo clippy ...` | `bque run` | `Idle` |
| `cargo test --workspace` | `just test` | `bque run` | `Idle` |
| `pnpm build` | `just build` 或 `bque run -- pnpm build` | `bque run` | `Idle` |
| `cmake --build target/debug` | `just build` | `bque run` | `Idle` |
| `ninja -C target/debug <目标>` | `just test-build <目标>` 或 `bque run -- ninja ...` | `bque run` | `Idle` |
| `ctest --output-on-failure` | `just test` 或 `bque run -- ctest ...` | `bque run` | `Idle` |
| `cargo bench` | `just bench` | `bque bench` | `High`（自动秒级挂起后台编译） |
| `pnpm bench` | `just bench` | `bque bench` | `High`（自动秒级挂起后台编译） |
| `cargo fmt` | `cargo fmt`（直接运行） | **旁路直跑** | 前台默认优先级 |
| `git status` / `git commit` | 直接运行 | **旁路直跑** | 前台默认优先级 |
| 任意临时重命令 | `bque run -- <命令>` | `bque run` | `Idle` |
| 任意临时跑分命令 | `bque bench -- <命令>` | `bque bench` | `High`（自动秒级挂起后台编译） |
| 常驻服务：dev server、watch 循环、`xtask dev`、`pnpm dev`、`cargo watch` | `bque run --observer -- <命令>`（**绝不**裸 `bque run`） | `bque run --observer` | `Idle` |

`bque run --observer` 的优先级与 CPU 预算和普通排队一致，但**不取**全局编译锁。常驻服务一旦持锁，
其他所有构建在它运行的整段时间里都会被堵住——「我的构建莫名其妙卡了几小时」就是这么来的。
`bque status` 对持有超过 30 分钟的记录单独标记，正是为了这个。

临时命令直接走原生 CLI。本层原本提供的 `just q-run` / `just q-bench` 包装是有意撤掉的：包装层
只能转发命令，传不进 bque 自己的 `--cwd` / `--observer` / `--priority`，恰好覆盖不了临时调用最
常见的需求。某条命令反复要用，就在项目专属区写一个具体配方把它包住。
bque 是 Windows 全局锁：Unix 与 CI 环境下没有 wrapper 可套，直跑即可。

---

## 2. Windows 4KB 匿名管道死锁原理解析与防范姿势

### 致命陷阱（反例）
在 PowerShell 中对原生控制台程序使用管道截断：
```powershell
# 严禁写法（反例）：千万不要这样写！
cargo test 2>&1 | Select-Object -First 100  # pipeline-hygiene: ok 反例演示
```
- PowerShell 为管道创建了仅 4KB 容量的 Windows 匿名管道缓冲区。
- `Select-Object -First 100` 在读取满足 100 行后协同结束读取，不再消费管道数据。
- 上游的原生编译器进程（`cargo.exe`、`cl.exe` 或 `ninja.exe`）继续向标准输出写日志。
- 一旦 4KB 缓冲区写满，底层内核 `WriteFile` 系统调用无限期阻塞等待读者读取。
- 整个进程树彻底卡死，直到 120 秒工具调用超时被强杀。

### 正确姿势（先重定向落盘，再读取截断）
```powershell
# 正确姿势：使用 *> 将全部输出重定向至临时文件，再通过 Get-Content 读取
cargo test *> test_out.log
Get-Content test_out.log -TotalCount 100
Remove-Item test_out.log -ErrorAction SilentlyContinue
```

---

## 3. 异常状态诊断与死锁自愈

每条队列记录都绑定写它的进程实例（PID **加**进程创建时间，因此 PID 被回收也不会把僵尸记录认成
活人），并由心跳刷新。死记录与幻影记录会在下一次任何 `status` / `run` / `bench` 访问队列时被自动
清掉——卡住的条目通常自己就消失了。

若进程是异常崩溃的，或者队列被一条你想立刻清掉的记录占着：
```powershell
# 0. 自检安装与 PATH 回退
just doctor

# 1. 检查持锁状态（这一步同时执行自愈清理）
bque status

# 2. 清理残留锁与孤儿等待记录，并如实报告删了什么
bque clean

# 3. 应急解锁：连「看起来还活着」的持有者也一并清掉
bque clean --force
# 内核 mutex 仍是最终裁决者，真在跑的构建不会被这条打断

# 4. 手动暂停与恢复后台构建（在进行重度游戏或系统级压测时）
bque pause
bque resume
```

`bque clean --force` 清的是状态记录，不是进程：一个仍持有内核 mutex 的常驻 dev server 会把队列
一直占着。要真正放锁，得停掉那个进程（或用 `bque run --observer` 重启它）。


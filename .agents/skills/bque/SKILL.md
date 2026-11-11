---
name: bque
description: >-

  通用跨工作树全局编译排队、后台 CPU 降级防卡顿与纯净基准测试保护技能。规范 Agent 何时必须走排队调度、何时直跑，提供纯净跑分独占执行（秒级挂起后台构建）、构建管道安全铁律（防 Windows 4KB 管道永久死锁）、全局状态自省与死锁自愈。当需要执行重型构建、编译、检查、单测、性能跑分（bench）、性能画像分析，或遇到 CPU 占满卡顿、命令挂死、编译锁争用时使用。
  Triggers: bque, build-queue, 排队构建, 全局排队, 编译排队, 跑分保护, 纯净跑分, 冻结后台构建, 挂起构建, cpu降级, idle优先级, queue-status, queue-clean, 编译锁, 管道死锁, 管道卫生, ninja, ctest, cmake, cargo, 编译, 构建, 跑测试, 运行测试, 单测, 性能跑分, bench, ninja -C, cmake --build, cargo build, cargo test.

---

<!-- PENGJ_TEMPLATE_START -->

# bque — 全局编译排队、CPU 调速与纯净跑分保护技能

用于协调跨工作树构建排队、前台防卡顿与纯净基准测试的工程基础设施技能。通过 Windows 内核原生对象将重型编译置于 `Idle` 空闲优先级，让路前台交互；在性能跑分时秒级挂起（Freeze）后台编译，保证 0% 后台 CPU 干扰与干净的 L3 缓存。

---

## ⚡ 1. 何时使用 vs 何时不使用 bque

| 场景分类 | 是否使用 bque？ | 推荐调用方式 | 选型理由 |
| :--- | :---: | :--- | :--- |
| **重型编译构建**（`cargo build`、`cmake`、`ninja`、`pnpm build`、`tsc`、`xtask pack`） | **必须使用** | `just build` 或 `bque run -- <命令>` | 跨工作树串行排队，进程优先级降为 `Idle`，杜绝卡死前台 IDE 与桌面 |
| **工作区检查与测试**（`cargo check`、`cargo clippy`、`cargo test`、`vitest run`） | **必须使用** | `just check` / `just test` 或 `bque run -- <命令>` | 高负载 CPU 任务，接入排队避免多工作树争抢挤爆 CPU 与磁盘 IO |
| **性能跑分与基准测试**（`bench`、`perf`、耗时画像探针） | **必须使用（Bench 模式）** | `just bench` 或 `bque bench -- <命令>` | **纯净跑分保护**：秒级挂起后台正在跑的编译，提供干净 CPU 环境，结束后原位恢复 |
| **递归/嵌套会话**（检测到 `BQUE_ACTIVE_PID` 环境变量） | **禁止使用** | 原生命令直跑（`& <命令>`）或 `bque run`（原生自动穿透） | 当前进程树已被外层 bque 纳管，直接执行避免重复排队造成嵌套死锁 |
| **轻量格式化与单文件检查**（`cargo fmt`、单文件 `prettier`/`eslint`） | **无需使用** | 原生命令直跑 | 秒级极速完成，几乎不占 CPU，无需额外排队开销 |
| **Git 工作流操作**（`git status`、`commit`、`push`、`rebase`） | **无需使用** | 原生命令直跑 | I/O 与网络操作，绝对严禁堵在编译队列后面 |
| **队列自省与运维**（`queue-status`、`queue-clean`） | **无需使用** | `just queue-status` / `just queue-clean` 或 `bque status` | 控制面与排查诊断工具，必须瞬时直达 |

完整场景判定表与命令映射详见 [references/usage-matrix.md](references/usage-matrix.md)。

---

## 🛑 2. 严禁裸跑原生构建/测试命令（硬性禁令与自动转换表）

AI 助手**绝对严禁**在终端中直接裸跑任何底层构建或编译测试命令！绕过 `bque` / `just` 会吃满所有 CPU 逻辑核心导致系统交互冻结、跨工作树产生编译锁争用、破坏性能跑分置信度，并在 PowerShell 管道下引发 4KB 匿名管道死锁。必须严格按下表映射为包装调用：

| 严禁裸跑的原生命令 (FORBIDDEN RAW) | 危害与问题 | 强制替换调用的包装形式 (MANDATORY WRAPPED FORM) |
| :--- | :--- | :--- |
| ❌ `ninja -C <dir> <target>` / `ninja ...` | 抢占全部 CPU 逻辑核心导致系统卡死，多工作树冲突 | `just test-build <target>` 或 `bque run -- ninja -C <dir> <target>` |
| ❌ `cmake --build <dir> ...` | 争用 CMake/Ninja 编译锁，打死前台交互 | `just build` 或 `bque run -- cmake --build <dir> ...` |
| ❌ `cargo build` / `cargo test` / `cargo clippy` | 产生 cargo target/ 锁争用，打满 CPU | `just build` / `just test` / `just check` 或 `bque run -- cargo ...` |
| ❌ 原生基准测试（如 `ninja -C target/debug DuntingThumbnailPipelineBenchmark` 或 `./bench.exe`） | **致命跑分污染**：未冻结后台构建，CPU 与 L3 缓存严重受扰，测试数据失真 | `just test-bench` 或 `bque bench -- <命令>`（自动挂起后台编译以提供 0% 干扰环境） |
| ❌ `ctest ...` / `<TestBinary>.exe` | 未降级的多线程并发测试抢占 CPU 造成前台卡顿 | `just test` 或 `bque run -- ctest ...` |
| ❌ `pnpm build` / `tsc`（大型前端项目） | Node.js/V8 引擎内存与 CPU 突增卡顿 | `just build` 或 `bque run -- pnpm build` |

---

## 🚀 3. 双阶梯执行规范（按优先级选用）

### 阶梯 1：统一优先使用 `just` 命令（首选）
项目日常命令已由 `justfile` 原生接入 `bque`，AI 助手应优先运行 `just`：
```powershell
just build           # 全局排队 + Idle 优先级构建
just check           # 全局排队代码检查
just test            # 全局排队单测运行
just bench           # 纯净基准测试（自动冻结后台其他编译）
just q-run cargo build --release   # 临时重命令，`--` 不要加
just q-bench pnpm bench           # 临时跑分命令，纯净独占模式
just queue-status    # 查看当前全局持锁者、排队任务与状态
just queue-clean     # 应急清理异常残留的编译锁状态
just queue-doctor    # 自检本机 bque 安装、PATH 回退与锁状态
```

无现成配方时优先用 `just q-run <cmd>`，原生 CLI 才是最后手段。`justfile` 框架区位于
`# PENGJ_TEMPLATE_START/END` 托管块内，项目自有配方写在块外、模板更新不会覆盖。
bque 是 Windows 全局锁：Unix 与 CI 环境下同一批 `just` 配方的 `[unix]` 分支会直跑透传，
属预期行为，严禁在那里强行套用原生 bque。

### 阶梯 2：原生 CLI 直调（定制或临时任务）
当无预设的 `just` 命令时，直接调用 `bque`：
```powershell
# 常规排队任务（默认 Idle 优先级，原生防重入穿透）
bque run -- cargo build --release

# 纯净跑分模式（独占排他锁，秒级挂起后台所有编译任务）
bque bench -- pnpm bench

# 状态查询与锁清理
bque status
bque clean
```

---

## 🛡️ 4. 构建安全两铁律（硬性红线）

1. **管道卫生安全（严禁截断原生管道）**：
   - **反例（严禁）**：`cargo test 2>&1 | Select-Object -First 100` 或 `cmake --build ... | Select-Object -Last 50`（# pipeline-hygiene: ok 反例演示）。
   - **原理解析**：原生 Windows 进程不响应 PowerShell 的协同终止信号，4KB 匿名管道缓冲区写满后触发内核级系统调用阻塞，整条调用链永久死锁（卡死 120 秒超时）。
   - **正确姿势**：必须先重定向落盘，再通过 PowerShell 读取：`cargo test *> out.log; Get-Content out.log -TotalCount 100`。
2. **多工作树进程精准终止**：
   - 在清理构建残留或重启服务时，**严禁使用进程名无差别盲杀**（如 `Stop-Process -Name Dunting`）；
   - 必须通过命令行路径精确匹配**当前工作树路径**后再终止，严禁误杀其他 worktree 的前台调试或测试实例。

---

## 🔧 5. 环境检测与安装指引

先运行 `just queue-doctor`：它会探测 `PATH`、回退到 `$env:LOCALAPPDATA\bque\bin`，
缺失时直接打印一键安装命令。完整安装指引：
👉 详见 [references/install.md](references/install.md)。


<!-- PENGJ_TEMPLATE_END -->


<!-- 以下为项目专属区域：模板更新只替换上方托管块，本区域归项目所有、完整保留。 -->
## 项目专属排队策略与定制配置

> 可在此登记本项目特殊的耗时任务标签（如 Heavy 测试、特殊离线渲染等）、专属超时阈值或前置环境准备脚本。

# justfile — 项目统一任务入口（由 just 层托管）
# 框架区在下方托管块内，由模板更新原位替换；块外的项目专属区归项目所有。
# 重型任务直接走 `bque run -- <cmd>`，性能跑分走 `bque bench -- <cmd>`（bque 层不再产出包装配方）。
# PENGJ_TEMPLATE_START
[unix]
set shell := ["sh", "-uc"]
[windows]
set shell := ["pwsh", "-NoProfile", "-Command"]
# 允许项目专属区用同名 recipe 覆盖框架配方（例如 `test` 前需要先产出某个产物），避免去改受管的 import 文件。
set allow-duplicate-recipes := true


import '.pengj-templates/just/gate.just'


import '.pengj-templates/just/worktree.just'

import '.pengj-templates/just/rust-workspace.just'


# 列出所有可用任务
default:
    @just --list

# 环境自检：工具链、提交钩子与工作区链接（可用 .pengj-templates/scripts/doctor.d/ 扩展）
doctor *ARGS:
    @pengj-templates-cli doctor --dir . {{ARGS}}
# PENGJ_TEMPLATE_END

# ---------------------------------------------------------------------------
# 项目专属区：模板更新只替换上方托管块，本区域归项目所有、完整保留。
# 在这里登记项目自己的 recipe；新增重任务请直接调用 `bque run -- <cmd>` / `bque bench -- <cmd>`，
# 不要裸跑 cargo / cmake / ninja / pnpm build / tsc 等编译命令。
# ---------------------------------------------------------------------------

# 排队约定：重型命令整条走 `bque run --` / `bque bench --`（bque 是 Windows 全局锁，`[unix]` 分支直跑）。
# 开窗交互程序（showcase / gallery / example）只把编译段排队，编译完立即释放全局锁并以前台
# 优先级直接运行二进制；若把常驻窗口塞进 bque，会长期阻塞其他工作树编译，还会让窗口跑在 Idle 优先级。

# 运行组件展台（可视验收台：开窗查看所有图元）
# 示例：just showcase 或 just showcase -- --shot shot.png
[windows]
showcase *ARGS:
    @bque run -- cargo build -p showcase
    @& "{{CARGO_TARGET_DIR}}/debug/showcase.exe" {{ARGS}}
[unix]
showcase *ARGS:
    @cargo build -p showcase
    @"{{CARGO_TARGET_DIR}}/debug/showcase" {{ARGS}}

# 运行全链路演示程序
# 示例：just gallery 或 just gallery -- --kpi
[windows]
gallery *ARGS:
    @bque run -- cargo build -p gallery
    @& "{{CARGO_TARGET_DIR}}/debug/gallery.exe" {{ARGS}}
[unix]
gallery *ARGS:
    @cargo build -p gallery
    @"{{CARGO_TARGET_DIR}}/debug/gallery" {{ARGS}}

# ---------------------------------------------------------------------------
# 参考图与像素差分（WinUI 3 基准）
#
# 唯一入口是 `just parity <动词>`，五个动词分别对应「全闭环 / 采参考图 / 跑差分 /
# 人工翻页 / 看报告」。两侧的 `--refresh` 同义：不要相信缓存。
# 参考图固定在 tools/winui-reference/captures（绝对路径），与调用者的工作目录无关。
# 宿主已把强调色钉死为 Fluent 蓝；系统强调色变化、或参考图早于「钉死强调色」改动时，
# 必须 `just parity capture --refresh --theme all`。
# ---------------------------------------------------------------------------

# parity 统一入口（动词：all | capture | diff | review | report，缺省 all）
# 示例：just parity / just parity review / just parity diff --control Button --refresh
parity action="all" *ARGS:
    @just "parity-{{action}}" {{ARGS}}

# 全闭环：增量采集参考图 → ROI 四门禁差分；`--refresh` 等公共旗标会同时作用于两段
[windows]
parity-all *ARGS:
    @just parity-capture {{ARGS}}
    @just parity-diff {{ARGS}}
[unix]
parity-all *ARGS:
    @echo "parity-all needs Windows (WinUI 3 reference host)" >&2
    @exit 1

# 采集 WinUI 3 参考图（增量；命中指纹缓存的用例毫秒级跳过）
[windows]
parity-capture *ARGS:
    @just _parity-build
    @just _parity-exe --out "{{justfile_directory()}}/tools/winui-reference/captures" {{ARGS}}
[unix]
parity-capture *ARGS:
    @echo "parity-capture needs Windows (WinUI 3 reference host)" >&2
    @exit 1

# 人工翻页检视：打开参考宿主逐页看（←/→ 或 PgUp/PgDn 翻页、Ctrl+S 导出本页、Esc 退出）
# 缺省同时装载浅色与深色两套页面；需要只看一套时再传 `--theme light|dark` 覆盖。
[windows]
parity-review *ARGS:
    @just _parity-build
    @just _parity-exe --review --theme all --out "{{justfile_directory()}}/tools/winui-reference/captures" {{ARGS}}
[unix]
parity-review *ARGS:
    @echo "parity-review needs Windows (WinUI 3 reference host)" >&2
    @exit 1

# 只跑差分：本框架侧离屏渲染 + ROI 四门禁，产出 target/parity/report.json
# 本命令绝不重采 WinUI 3 参考图；参考图过期请先 `just parity capture --refresh --theme all`。
# 示例：just parity diff --control Button --refresh / just parity diff --only-failed
parity-diff *ARGS:
    @bque run -- cargo run -p showcase -- parity-diff --reference tools/winui-reference/captures --out target/parity {{ARGS}}

# 打开展台的 Pixel parity 报告页（参考图 / 本框架 / 热力图三列并排）
[windows]
parity-report *ARGS:
    @bque run -- cargo build -p showcase
    @& "{{CARGO_TARGET_DIR}}/debug/showcase.exe" gallery --page ParityReport {{ARGS}}
[unix]
parity-report *ARGS:
    @cargo build -p showcase
    @"{{CARGO_TARGET_DIR}}/debug/showcase" gallery --page ParityReport {{ARGS}}

# WinUI 3 参考宿主的唯一构建入口；WinAppSDK 走自包含，不要求机器预装 Windows App Runtime。
# Windows 的 PATH 里 `C:\Program Files\dotnet` 只有 runtime、没有 SDK，且排在 scoop 的 dotnet8-sdk 之前，
# 直接写 `dotnet build` 会解析到空壳并报 "No .NET SDKs were found"；这里优先用 DOTNET_ROOT 指向的 SDK。
[windows]
_parity-build:
    @bque run -- $(if ($env:DOTNET_ROOT) { "$env:DOTNET_ROOT\dotnet.exe" } else { "dotnet" }) build tools/winui-reference/WinUiReference.csproj -c Release -p:Platform=x64 -p:WindowsAppSDKSelfContained=true -r win-x64

# 直接运行参考宿主（开窗程序不占全局锁；编译段已经在 _parity-build 里排过队）
[windows]
_parity-exe *ARGS:
    @& "{{justfile_directory()}}/tools/winui-reference/bin/x64/Release/net8.0-windows10.0.19041.0/win-x64/WinUiReference.exe" {{ARGS}}

# 编译 Vulkan 着色器（glslc 逐个编译并校验，同样纳入排队）
compile-shaders:
    @bque run -- bash scripts/compile-shaders.sh

# 运行指定示例程序
# 示例：just example 01_hello_counter
[windows]
example NAME *ARGS:
    @bque run -- cargo build -p unbound-app --example {{NAME}}
    @& "{{CARGO_TARGET_DIR}}/debug/examples/{{NAME}}.exe" {{ARGS}}
[unix]
example NAME *ARGS:
    @cargo build -p unbound-app --example {{NAME}}
    @"{{CARGO_TARGET_DIR}}/debug/examples/{{NAME}}" {{ARGS}}

# 运行所有示例程序的无头自检套件
test-examples:
    @bque run -- cargo test -p unbound-app --examples

# 运行文本渲染性能与构建裁剪基准测试套件（走 bque bench，秒级挂起后台编译）
text-baseline *ARGS:
    @bque bench -- cargo test -p unbound-text --test text_render_baseline --all-features --release -- --nocapture {{ARGS}}

# app-guest 的 host_roundtrip 集成测试要加载真实 cdylib，而 `cargo test`
# 不保证在 target/<profile> 下生成该产物，故先显式构建一次，避免干净环境硬失败。
# 覆盖机制见托管块的 allow-duplicate-recipes。

# 覆盖框架 test：先产出 app-guest cdylib 再跑测试
test *ARGS:
    @bque run -- cargo build -p app-guest
    @bque run -- cargo test --workspace --all-features {{ARGS}}

# ---------- 共享编译目录（worktree 层） ----------

# 编译产物落在仓库外、按机器推导的共享目录，多个工作树/多份克隆复用同一份依赖产物。
# 推导逻辑只有 CLI 一份；框架配方与 `just wt-check` 读的是同一个值，这里导出给整份
# justfile，showcase/gallery/example 这类直连 cargo 的 recipe 同样继承。
export CARGO_TARGET_DIR := `pengj-templates-cli worktree target-dir --dir .`

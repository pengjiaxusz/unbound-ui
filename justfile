# justfile — 项目统一任务入口（由 just 层托管）
# 框架区在下方托管块内，由模板更新原位替换；块外的项目专属区归项目所有。
# 重型任务统一走 `just q-run <cmd>`，性能跑分走 `just q-bench <cmd>`。
# PENGJ_TEMPLATE_START
[unix]
set shell := ["sh", "-uc"]
[windows]
set shell := ["pwsh", "-NoProfile", "-Command"]
# 允许项目专属区用同名 recipe 覆盖框架配方（例如 `test` 前需要先产出某个产物），避免去改受管的 import 文件。
set allow-duplicate-recipes := true


import 'just/bque.just'


import 'just/gate.just'


import 'just/rust-workspace.just'


# 列出所有可用任务
default:
    @just --list

# 环境自检：工具链与提交钩子激活状态（可用 scripts/doctor.d/ 扩展）
[unix]
doctor *ARGS:
    @sh scripts/doctor.sh {{ARGS}}
# 环境自检：工具链与提交钩子激活状态（可用 scripts/doctor.d/ 扩展）
[windows]
doctor *ARGS:
    @pwsh -NoProfile -File scripts/doctor.ps1 {{ARGS}}
# PENGJ_TEMPLATE_END

# ---------------------------------------------------------------------------
# 项目专属区：模板更新只替换上方托管块，本区域归项目所有、完整保留。
# 在这里登记项目自己的 recipe；新增重任务请调用 `just q-run <cmd>` / `just q-bench <cmd>`，
# 不要裸跑 cargo / cmake / ninja / pnpm build / tsc 等编译命令。
# ---------------------------------------------------------------------------

# 运行组件展台（可视验收台：开窗查看所有图元）
# 示例：just showcase 或 just showcase -- --shot shot.png
showcase *ARGS:
    cargo run -p showcase {{ARGS}}

# 运行全链路演示程序
# 示例：just gallery 或 just gallery -- --kpi
gallery *ARGS:
    cargo run -p gallery {{ARGS}}

# 编译 Vulkan 着色器
compile-shaders:
    bash scripts/compile-shaders.sh

# 运行指定示例程序
# 示例：just example 01_hello_counter
example NAME *ARGS:
    cargo run -p unbound-app --example {{NAME}} {{ARGS}}

# 运行所有示例程序的无头自检套件
test-examples:
    just q-run cargo test -p unbound-app --examples

# app-guest 的 host_roundtrip 集成测试要加载真实 cdylib，而 `cargo test`
# 不保证在 target/<profile> 下生成该产物，故先显式构建一次，避免干净环境硬失败。
# 覆盖机制见托管块的 allow-duplicate-recipes。

# 覆盖框架 test：先产出 app-guest cdylib 再跑测试
[windows]
test *ARGS:
    @bque run -- cargo build -p app-guest
    @bque run -- cargo test --workspace --all-features {{ARGS}}
[unix]
test *ARGS:
    cargo build -p app-guest
    cargo test --workspace --all-features {{ARGS}}

# justfile — 项目统一任务入口（由 just 层托管）
# 框架区在下方托管块内，由模板更新原位替换；块外的项目专属区归项目所有。
# 重型任务统一走 `just q-run <cmd>`，性能跑分走 `just q-bench <cmd>`。
# PENGJ_TEMPLATE_START
[unix]
set shell := ["sh", "-uc"]
[windows]
set shell := ["pwsh", "-NoProfile", "-Command"]


import 'just/bque.just'


import 'just/rust-workspace.just'


# 列出所有可用任务
default:
    @just --list
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

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


import '.pengj-templates/just/bque.just'


import '.pengj-templates/just/gate.just'


import '.pengj-templates/just/worktree.just'

import '.pengj-templates/just/rust-workspace.just'


# 列出所有可用任务
default:
    @just --list

# 环境自检：工具链、提交钩子与工作区链接（可用 .pengj-templates/scripts/doctor.d/ 扩展）
[unix]
doctor *ARGS:
    @sh .pengj-templates/scripts/doctor.sh {{ARGS}}
# 环境自检：工具链、提交钩子与工作区链接（可用 .pengj-templates/scripts/doctor.d/ 扩展）
[windows]
doctor *ARGS:
    @pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1 {{ARGS}}
# PENGJ_TEMPLATE_END

# ---------------------------------------------------------------------------
# 项目专属区：模板更新只替换上方托管块，本区域归项目所有、完整保留。
# 在这里登记项目自己的 recipe；新增重任务请调用 `just q-run <cmd>` / `just q-bench <cmd>`，
# 不要裸跑 cargo / cmake / ninja / pnpm build / tsc 等编译命令。
# ---------------------------------------------------------------------------

# 排队约定：有限的重型命令整条走 `just q-run` / `just q-bench`（Unix 分支自动透传）。
# 开窗交互程序（showcase / gallery / example）只把编译段排队，编译完立即释放全局锁并以前台
# 优先级直接运行二进制；若把常驻窗口塞进 bque，会长期阻塞其他工作树编译，还会让窗口跑在 Idle 优先级。

# 运行组件展台（可视验收台：开窗查看所有图元）
# 示例：just showcase 或 just showcase -- --shot shot.png
[windows]
showcase *ARGS:
    @just q-run cargo build -p showcase
    @& "{{CARGO_TARGET_DIR}}/debug/showcase.exe" {{ARGS}}
[unix]
showcase *ARGS:
    @just q-run cargo build -p showcase
    @"{{CARGO_TARGET_DIR}}/debug/showcase" {{ARGS}}

# 运行全链路演示程序
# 示例：just gallery 或 just gallery -- --kpi
[windows]
gallery *ARGS:
    @just q-run cargo build -p gallery
    @& "{{CARGO_TARGET_DIR}}/debug/gallery.exe" {{ARGS}}
[unix]
gallery *ARGS:
    @just q-run cargo build -p gallery
    @"{{CARGO_TARGET_DIR}}/debug/gallery" {{ARGS}}

# 先构建 tools/winui-reference 再批量截图；WinAppSDK 走自包含，不要求机器预装 Windows App Runtime。
# 产物已被 gitignore，控件样式变动后才需要重跑；采集器自带指纹缓存，命中时会毫秒级跳过。
# 采集 WinUI 3 参考截图到 captures/（需 .NET 8+ SDK，示例：just parity-capture --no-cache）
# 采集器自身也走排队，但 q-run 不支持指定工作目录；这里用 bque 原生 --cwd 保留 ./captures 的语义。
[windows]
parity-capture *ARGS:
    @just q-run dotnet build tools/winui-reference/WinUiReference.csproj -c Release -p:Platform=x64 -p:WindowsAppSDKSelfContained=true -r win-x64
    @bque run --cwd "{{justfile_directory()}}/tools/winui-reference" -- "{{justfile_directory()}}/tools/winui-reference/bin/x64/Release/net8.0-windows10.0.19041.0/win-x64/WinUiReference.exe" --batch --out ./captures {{ARGS}}

# 纯离屏 Vulkan 渲染，不开窗；默认复用源码指纹缓存。
# 跑 WinUI 3 ROI 像素差分，产出 target/parity/report.json 供展台 Pixel parity 页读取
# 示例：just parity 或 just parity --no-cache
parity *ARGS:
    @just q-run cargo run -p showcase -- --parity-run --reference tools/winui-reference/captures --out target/parity {{ARGS}}

# 编译 Vulkan 着色器（glslc 逐个编译并校验，同样纳入排队）
compile-shaders:
    @just q-run bash scripts/compile-shaders.sh

# 运行指定示例程序
# 示例：just example 01_hello_counter
[windows]
example NAME *ARGS:
    @just q-run cargo build -p unbound-app --example {{NAME}}
    @& "{{CARGO_TARGET_DIR}}/debug/examples/{{NAME}}.exe" {{ARGS}}
[unix]
example NAME *ARGS:
    @just q-run cargo build -p unbound-app --example {{NAME}}
    @"{{CARGO_TARGET_DIR}}/debug/examples/{{NAME}}" {{ARGS}}

# 运行所有示例程序的无头自检套件
test-examples:
    @just q-run cargo test -p unbound-app --examples

# 运行文本渲染性能与构建裁剪基准测试套件（走 q-bench，Unix/CI 下自动透传）
text-baseline *ARGS:
    @just q-bench cargo test -p unbound-text --test text_render_baseline --all-features --release -- --nocapture {{ARGS}}

# app-guest 的 host_roundtrip 集成测试要加载真实 cdylib，而 `cargo test`
# 不保证在 target/<profile> 下生成该产物，故先显式构建一次，避免干净环境硬失败。
# 覆盖机制见托管块的 allow-duplicate-recipes。

# 覆盖框架 test：先产出 app-guest cdylib 再跑测试
test *ARGS:
    @just q-run cargo build -p app-guest
    @just q-run cargo test --workspace --all-features {{ARGS}}

# ---------- 共享编译目录（worktree 层） ----------

# 编译产物落在仓库外、按机器推导的共享目录，多个工作树/多份克隆复用同一份依赖产物。
# 推导逻辑只有 CLI 一份；框架配方与 `just wt-check` 读的是同一个值，这里导出给整份
# justfile，showcase/gallery/example 这类直连 cargo 的 recipe 同样继承。
export CARGO_TARGET_DIR := `pengj-templates-cli worktree target-dir --dir .`

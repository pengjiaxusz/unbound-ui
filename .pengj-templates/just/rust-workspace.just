# .pengj-templates/just/rust-workspace.just — Rust 多 crate 工作区常用任务（rust-workspace 层托管）
# 由 justfile 主入口按层条件 import；项目若有自己的编排，请在 manifest 里设
# options["just_recipes"] = "none"，把这些配方留给项目专属区。

# 全工作区构建（Windows 走全局排队 + Idle 优先级）
[windows]
build *ARGS:
    @$d = if (Get-Command pengj-templates-cli -ErrorAction SilentlyContinue) { pengj-templates-cli worktree target-dir --dir . 2>$null }; $env:CARGO_TARGET_DIR = if ($LASTEXITCODE -eq 0 -and $d) { $d } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }; bque run -- cargo build --workspace {{ARGS}}
[unix]
build *ARGS:
    CARGO_TARGET_DIR="$(command -v pengj-templates-cli >/dev/null 2>&1 && pengj-templates-cli worktree target-dir --dir . 2>/dev/null || printf '%s' "${CARGO_TARGET_DIR:-target}")" cargo build --workspace {{ARGS}}

# 全工作区检查（clippy 警告视为错误）
[windows]
check *ARGS:
    @$d = if (Get-Command pengj-templates-cli -ErrorAction SilentlyContinue) { pengj-templates-cli worktree target-dir --dir . 2>$null }; $env:CARGO_TARGET_DIR = if ($LASTEXITCODE -eq 0 -and $d) { $d } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }; bque run -- cargo clippy --workspace --all-targets --all-features -- -D warnings {{ARGS}}
[unix]
check *ARGS:
    CARGO_TARGET_DIR="$(command -v pengj-templates-cli >/dev/null 2>&1 && pengj-templates-cli worktree target-dir --dir . 2>/dev/null || printf '%s' "${CARGO_TARGET_DIR:-target}")" cargo clippy --workspace --all-targets --all-features -- -D warnings {{ARGS}}

# 全工作区测试
[windows]
test *ARGS:
    @$d = if (Get-Command pengj-templates-cli -ErrorAction SilentlyContinue) { pengj-templates-cli worktree target-dir --dir . 2>$null }; $env:CARGO_TARGET_DIR = if ($LASTEXITCODE -eq 0 -and $d) { $d } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }; bque run -- cargo test --workspace --all-features {{ARGS}}
[unix]
test *ARGS:
    CARGO_TARGET_DIR="$(command -v pengj-templates-cli >/dev/null 2>&1 && pengj-templates-cli worktree target-dir --dir . 2>/dev/null || printf '%s' "${CARGO_TARGET_DIR:-target}")" cargo test --workspace --all-features {{ARGS}}

# 专项基准测试（自动挂起后台编译，提供纯净 CPU 环境）
[windows]
bench *ARGS:
    @$d = if (Get-Command pengj-templates-cli -ErrorAction SilentlyContinue) { pengj-templates-cli worktree target-dir --dir . 2>$null }; $env:CARGO_TARGET_DIR = if ($LASTEXITCODE -eq 0 -and $d) { $d } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }; bque bench -- cargo bench --workspace {{ARGS}}
[unix]
bench *ARGS:
    CARGO_TARGET_DIR="$(command -v pengj-templates-cli >/dev/null 2>&1 && pengj-templates-cli worktree target-dir --dir . 2>/dev/null || printf '%s' "${CARGO_TARGET_DIR:-target}")" cargo bench --workspace {{ARGS}}


# 代码就地格式化（轻量，不排队）
fmt:
    cargo fmt

# 格式化检查（CI 校验用，不修改文件）
fmt-check:
    cargo fmt --check

# 一键全检查（CI 用；各子命令各自取锁，bque 原生防重入穿透）
ci: fmt-check check test
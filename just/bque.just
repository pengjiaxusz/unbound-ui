# just/bque.just — 全局跨工作树编译排队（bque 层托管，模板更新整页覆盖）
# bque 是 Windows 全局内核锁调度器：unix 分支不排队，只做等价透传与提示。
# 任何重型命令都应经 q-run / q-bench，避免裸跑吃满 CPU 与污染跑分。

# 查看全局跨工作树排队状态（持锁者 / 排队者 / CPU 预算）
[windows]
queue-status:
    @bque status
[unix]
queue-status:
    @echo "bque is Windows-only: no global build queue on this platform"

# 应急清理异常残留的编译锁与孤儿等待记录
[windows]
queue-clean:
    @bque clean
[unix]
queue-clean:
    @echo "bque is Windows-only: no global build lock to clean"

# 手动暂停后台编译（让路游戏 / 交互压测 / 系统级画像）
[windows]
queue-pause:
    @bque pause
[unix]
queue-pause:
    @echo "bque is Windows-only: nothing to pause"

# 手动恢复后台编译
[windows]
queue-resume:
    @bque resume
[unix]
queue-resume:
    @echo "bque is Windows-only: nothing to resume"

# 环境自检：探测 PATH 中的 bque，回退到默认安装目录，缺失时给出一键安装命令
[windows]
queue-doctor:
    @$cmd = Get-Command bque -ErrorAction SilentlyContinue; if (-not $cmd -and (Test-Path "$env:LOCALAPPDATA\bque\bin\bque.exe")) { $env:Path = "$env:LOCALAPPDATA\bque\bin;$env:Path"; $cmd = Get-Command bque -ErrorAction SilentlyContinue }; if ($cmd) { Write-Host "bque: $($cmd.Source)"; bque --version; bque status } else { Write-Host 'bque missing. Install: irm https://github.com/pengjiaxusz/bque/releases/latest/download/install.ps1 | iex' }
[unix]
queue-doctor:
    @echo "bque is Windows-only: no installation check needed"

# 用法：just q-run cargo build --release（不要加 `--`，它会成为命令的一部分）
# 通用排队执行入口：把任意重型命令纳入全局锁 + Idle 优先级
[windows]
q-run *ARGS:
    @bque run -- {{ARGS}}
[unix]
q-run *ARGS:
    {{ARGS}}

# 用法：just q-bench cargo bench --workspace（不要加 `--`）
# 纯净跑分入口：独占排他锁 + 秒级挂起后台编译，保证 0% CPU 干扰
[windows]
q-bench *ARGS:
    @bque bench -- {{ARGS}}
[unix]
q-bench *ARGS:
    {{ARGS}}
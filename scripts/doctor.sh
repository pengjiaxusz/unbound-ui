#!/bin/sh
# scripts/doctor.sh - 项目环境自检.
#
# 只检查所选模板层实际用到的工具，再按文件名顺序运行 scripts/doctor.d/*.sh
# 下的所有自定义检查。
#
# 用法：
#   sh scripts/doctor.sh          仅输出报告
#   sh scripts/doctor.sh --fix    同时激活提交钩子并修正相关模式
#
# 自定义检查：把 *.sh 放进 scripts/doctor.d/。每个脚本从项目根目录运行，
# DOCTOR_ROOT 指向项目根目录，带 --fix 时设置 DOCTOR_FIX=1。
# 退出码 0 为通过，非 0 为失败；输出按原样显示。

set -u

fix=0
for arg in "$@"; do
    case "$arg" in
        --fix) fix=1 ;;
        -h|--help)
            echo "用法：sh scripts/doctor.sh [--fix]"
            exit 0
            ;;
        *)
            echo "[doctor] 未知参数： $arg" >&2
            exit 2
            ;;
    esac
done

root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
cd "$root" || exit 1
export DOCTOR_ROOT="$root"
if [ "$fix" = "1" ]; then
    export DOCTOR_FIX=1
fi

checks=0
errors=0
warnings=0

ok()   { checks=$((checks + 1)); printf '[doctor] ok    %s\n' "$1"; }
warn() { checks=$((checks + 1)); warnings=$((warnings + 1)); printf '[doctor] warn  %s\n' "$1"; }
fail() { checks=$((checks + 1)); errors=$((errors + 1)); printf '[doctor] error %s\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }
is_windows() {
    case "$(uname -s 2>/dev/null || echo unknown)" in
        MINGW*|MSYS*|CYGWIN*) return 0 ;;
        *) return 1 ;;
    esac
}

# --- built-in checks: only what the selected layers use ---------------------

if have git; then
    ok "git $(git --version | sed 's/^git version //')"
else
    fail "未找到 git"
fi


if have just; then
    ok "just $(just --version | sed 's/^just //')"
else
    fail "未找到 just；安装地址：https://github.com/casey/just#installation"
fi
if is_windows; then
    if have pwsh; then
        ok "pwsh $(pwsh --version)"
    else
        fail "未找到 pwsh；Windows 上的 just 配方需要 PowerShell 7"
    fi
fi



if have cargo; then
    ok "cargo $(cargo --version | sed 's/^cargo //')"
else
    fail "未找到 cargo；请安装 Rust 工具链"
fi
if have rustc; then
    ok "rustc $(rustc --version | sed 's/^rustc //')"
else
    fail "未找到 rustc；请安装 Rust 工具链"
fi



if is_windows; then
    if have bque; then
        ok "bque $(bque --version 2>/dev/null | head -n 1)"
    else
        warn "未找到 bque；安装命令：irm https://github.com/pengjiaxusz/bque/releases/latest/download/install.ps1 | iex"
    fi
else
    ok "当前平台不需要 bque"
fi



if have pengj-templates-cli; then
    ok "pengj-templates-cli $(pengj-templates-cli --version 2>/dev/null | head -n 1)"
else
    warn "未找到 pengj-templates-cli；安装命令：curl -fsSL https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.sh | sh"
fi



if have node; then
    ok "node $(node --version 2>/dev/null)"
else
    warn "未找到 node；mcp-bootstrap 技能需要它（请安装 Node.js，或从 options.skills 中移除该技能）"
fi



# --- commit gate activation ------------------------------------------------

if have git && git rev-parse --show-toplevel >/dev/null 2>&1; then
    local_hooks=$(git config --local --get core.hooksPath 2>/dev/null || true)

    if [ ! -f .githooks/pre-commit ] || [ ! -f .githooks/commit-msg ]; then
        fail ".githooks 不完整；应包含 pre-commit 与 commit-msg"
    elif [ "$local_hooks" = ".githooks" ]; then
        ok "提交钩子已激活（core.hooksPath=.githooks）"
    elif [ -n "$local_hooks" ]; then
        warn "core.hooksPath 为 '$local_hooks'，并非 .githooks；提交门禁未在此生效"
    elif [ "$fix" = "1" ]; then
        # Activate through the zero-dependency installer shipped by the gate
        # layer, but judge the outcome by the resulting git state: an
        # informational line failing to print must not turn a successful
        # activation into an error.
        if [ -f .githooks/install.sh ]; then
            sh .githooks/install.sh >/dev/null || true
        fi
        # Record the mode bit when the hooks are already tracked, so clones on
        # Unix do not silently skip them.
        git update-index --chmod=+x .githooks/pre-commit .githooks/commit-msg 2>/dev/null || true
        if [ "$(git config --local --get core.hooksPath 2>/dev/null || true)" = ".githooks" ]; then
            ok "提交钩子已激活（core.hooksPath=.githooks）"
        else
            fail "无法激活提交钩子；请运行：git config core.hooksPath .githooks"
        fi
    else
        warn "提交钩子未激活；请运行 'sh scripts/doctor.sh --fix' 或 'git config core.hooksPath .githooks'"
    fi

    # git silently skips a hook without the executable bit on Unix
    if ! is_windows; then
        for hook in pre-commit commit-msg; do
            if [ -f ".githooks/$hook" ] && [ ! -x ".githooks/$hook" ]; then
                if [ "$fix" = "1" ]; then
                    chmod +x ".githooks/$hook" && ok ".githooks/$hook 已具备可执行权限"
                else
                    warn ".githooks/$hook 不可执行；请带 --fix 运行"
                fi
            fi
        done
    fi
else
    warn "当前不是 git 工作树；跳过提交钩子检查"
fi



# --- agent client workspace links -----------------------------------------

if [ -d .agents ]; then
    if have pengj-templates-cli; then
        if pengj-templates-cli repo status --dir . >/dev/null 2>&1; then
            ok "工作区链接已就绪（agent 客户端）"
        elif [ "$fix" = "1" ] && pengj-templates-cli repo init --dir . >/dev/null 2>&1 \
            && pengj-templates-cli repo status --dir . >/dev/null 2>&1; then
            ok "工作区链接已创建（agent 客户端）"
        elif [ "$fix" = "1" ]; then
            fail "无法创建工作区链接；请运行：pengj-templates-cli repo init"
        else
            warn "工作区链接未就绪；请运行 'just doctor --fix'"
        fi
    else
        warn "未找到 pengj-templates-cli；跳过工作区链接检查"
    fi
fi


# --- custom checks ---------------------------------------------------------

custom=0
if [ -d scripts/doctor.d ]; then
    for script in scripts/doctor.d/*.sh; do
        [ -f "$script" ] || continue
        custom=$((custom + 1))
        name=$(basename "$script")
        if sh "$script"; then
            checks=$((checks + 1))
            printf '[doctor] custom ok    %s\n' "$name"
        else
            checks=$((checks + 1))
            errors=$((errors + 1))
            printf '[doctor] custom error %s\n' "$name"
        fi
    done
fi
if [ "$custom" -eq 0 ]; then
    printf '[doctor] 自定义检查：无（在 scripts/doctor.d/ 下添加 *.sh 脚本即可扩展本检查）\n'
fi

printf '[doctor] 共 %d 项检查：%d 个错误，%d 个警告\n' "$checks" "$errors" "$warnings"
if [ "$errors" -gt 0 ]; then
    exit 1
fi
exit 0
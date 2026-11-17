#!/bin/sh
# scripts/doctor.sh - project environment check.
#
# Checks only the tools the selected template layers actually use, then runs
# every custom check in scripts/doctor.d/*.sh in sorted order.
#
# Usage:
#   sh scripts/doctor.sh          report only
#   sh scripts/doctor.sh --fix    also activate the commit hooks and fix modes
#
# Custom checks: drop *.sh files into scripts/doctor.d/. Each script runs from
# the project root with DOCTOR_ROOT set to the project root and DOCTOR_FIX=1
# when --fix is given. Exit 0 = pass, non-zero = failure; output is shown as-is.

set -u

fix=0
for arg in "$@"; do
    case "$arg" in
        --fix) fix=1 ;;
        -h|--help)
            echo "usage: sh scripts/doctor.sh [--fix]"
            exit 0
            ;;
        *)
            echo "[doctor] unknown argument: $arg" >&2
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
    fail "git not found"
fi


if have just; then
    ok "just $(just --version | sed 's/^just //')"
else
    fail "just not found; install: https://github.com/casey/just#installation"
fi
if is_windows; then
    if have pwsh; then
        ok "pwsh $(pwsh --version)"
    else
        fail "pwsh not found; the just recipes use PowerShell 7 on Windows"
    fi
fi



if have cargo; then
    ok "cargo $(cargo --version | sed 's/^cargo //')"
else
    fail "cargo not found; install a Rust toolchain"
fi
if have rustc; then
    ok "rustc $(rustc --version | sed 's/^rustc //')"
else
    fail "rustc not found; install a Rust toolchain"
fi



if is_windows; then
    if have bque; then
        ok "bque $(bque --version 2>/dev/null | head -n 1)"
    else
        warn "bque not found; install: irm https://github.com/pengjiaxusz/bque/releases/latest/download/install.ps1 | iex"
    fi
else
    ok "bque not required on this platform"
fi



if have pengj-templates-cli; then
    ok "pengj-templates-cli $(pengj-templates-cli --version 2>/dev/null | head -n 1)"
else
    warn "pengj-templates-cli not found; install: curl -fsSL https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.sh | sh"
fi



if have node; then
    ok "node $(node --version 2>/dev/null)"
else
    warn "node not found; it is required by the mcp-bootstrap skill (install Node.js or drop that skill from options.skills)"
fi



# --- commit gate activation ------------------------------------------------

if have git && git rev-parse --show-toplevel >/dev/null 2>&1; then
    local_hooks=$(git config --local --get core.hooksPath 2>/dev/null || true)
    global_hooks=$(git config --global --get core.hooksPath 2>/dev/null || true)
    legacy_global=0
    case "$global_hooks" in
        *pengj-templates/hooks*) legacy_global=1 ;;
    esac

    if [ ! -f .githooks/pre-commit ] || [ ! -f .githooks/commit-msg ]; then
        fail ".githooks is incomplete; expected pre-commit and commit-msg"
    elif [ "$local_hooks" = ".githooks" ]; then
        ok "commit hooks active (core.hooksPath=.githooks)"
    elif [ -n "$local_hooks" ]; then
        warn "core.hooksPath is '$local_hooks', not .githooks; the commit gate is not active here"
    elif [ "$fix" = "1" ]; then
        # Activate through the first available path, but judge the outcome by
        # the resulting git state: an informational line failing to print must
        # not turn a successful activation into an error.
        if [ -f .githooks/install.sh ]; then
            sh .githooks/install.sh >/dev/null || true
        elif have pengj-templates-cli; then
            pengj-templates-cli gate hooks install >/dev/null 2>&1 || true
        else
            git config core.hooksPath .githooks || true
        fi
        # Record the mode bit when the hooks are already tracked, so clones on
        # Unix do not silently skip them.
        git update-index --chmod=+x .githooks/pre-commit .githooks/commit-msg 2>/dev/null || true
        if [ "$(git config --local --get core.hooksPath 2>/dev/null || true)" = ".githooks" ]; then
            ok "commit hooks activated (core.hooksPath=.githooks)"
        else
            fail "could not activate the commit hooks; run: git config core.hooksPath .githooks"
        fi
    elif [ "$legacy_global" = "1" ]; then
        warn "commit hooks are active through the legacy global dispatcher; run 'sh scripts/doctor.sh --fix' to switch to per-repository activation"
    else
        warn "commit hooks are not active; run 'sh scripts/doctor.sh --fix' or 'git config core.hooksPath .githooks'"
    fi

    # git silently skips a hook without the executable bit on Unix
    if ! is_windows; then
        for hook in pre-commit commit-msg; do
            if [ -f ".githooks/$hook" ] && [ ! -x ".githooks/$hook" ]; then
                if [ "$fix" = "1" ]; then
                    chmod +x ".githooks/$hook" && ok ".githooks/$hook is executable"
                else
                    warn ".githooks/$hook is not executable; run with --fix"
                fi
            fi
        done
    fi

    # Older installs wrote a machine-wide dispatcher. It still forwards, but
    # activation is now per project; report it so the leftover can be removed.
    if [ "$legacy_global" = "1" ]; then
        warn "legacy global gate dispatcher detected ($global_hooks); remove it with: git config --global --unset core.hooksPath"
    fi
else
    warn "not a git work tree; skipping the commit hook check"
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
    printf '[doctor] custom: none (add *.sh scripts to scripts/doctor.d/ to extend this check)\n'
fi

printf '[doctor] %d checks: %d errors, %d warnings\n' "$checks" "$errors" "$warnings"
if [ "$errors" -gt 0 ]; then
    exit 1
fi
exit 0
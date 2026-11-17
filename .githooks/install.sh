#!/bin/sh
# gate install: 不依赖 CLI 启用本地提交门禁.
#
# 这个脚本是给「机器上没有 pengj-templates-cli」准备的答案：只需要 git 与一个 POSIX shell，别的不用。每个克隆跑一次即可。
#
# 此后门禁以降级模式运行：提交信息照样被检查，只是走 `gate-fallback.sh` 里那套缩减规则，而不是 CLI。完整规则集无论如何都由服务端 CI 兜住。

set -eu

root=$(git rev-parse --show-toplevel 2>/dev/null) || {
    echo "[gate] 当前目录不是 git 工作区" >&2
    exit 1
}
cd "$root"

if [ ! -f .githooks/commit-msg ]; then
    echo "[gate] 这个仓库没有携带门禁钩子" >&2
    exit 1
fi

# git 默认只读 `.git/hooks`，而 `core.hooksPath` 存在 `.git/config` 里、不会随克隆传播。把它指向仓库里托管的 `.githooks/`，门禁才真的跑得起来。
git config core.hooksPath .githooks

# Unix 上没有可执行位的钩子会被 git 静默跳过。模式位以索引记录为准，`update-index` 负责把它写进去。
for hook in commit-msg pre-commit; do
    [ -f ".githooks/$hook" ] || continue
    if git ls-files --error-unmatch ".githooks/$hook" >/dev/null 2>&1; then
        git update-index --chmod=+x ".githooks/$hook"
    else
        chmod +x ".githooks/$hook" 2>/dev/null || true
        echo "[gate] .githooks/$hook 还没纳入版本管理，可执行位只写到了磁盘上" >&2
    fi
done

if command -v pengj-templates-cli >/dev/null 2>&1; then
    echo "[gate] 检测到 pengj-templates-cli：完整规则集已就位"
else
    echo "[gate] 没检测到 pengj-templates-cli：钩子会走缩减规则集，服务端 CI 仍按完整规则集把关" >&2
    echo "[gate]   装一次就能在本地拿到完整规则集：" >&2
    echo "[gate]   irm https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.ps1 | iex" >&2
    echo "[gate]   curl -fsSL https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.sh | sh" >&2
fi

configured=$(git config --local --get core.hooksPath 2>/dev/null || true)
if [ "$configured" = ".githooks" ]; then
    echo "[gate] 本地提交门禁已启用：core.hooksPath=.githooks" 2>/dev/null || true
else
    echo "[gate] 未能写入 core.hooksPath；请手动执行 git config core.hooksPath .githooks" >&2
    exit 1
fi
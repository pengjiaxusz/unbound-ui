#!/bin/sh
# gate install: 在当前克隆上激活门禁钩子.
#
# 沿用 `core.hooksPath` 存在 `.git/config` 里、不会随克隆传播，所以每个克隆跑一次即可。脚本只需要 git 与一个 POSIX shell——它不需要 CLI，因为「激活钩子」与「跑钩子」是两件事。
#
# 门禁本体在 `pengj-templates-cli` 里，本脚本不自带规则。没有 CLI 时钩子会拦下提交并打印安装命令，所以下面的检测只做告警。

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
for hook in commit-msg pre-commit pre-push; do
    [ -f ".githooks/$hook" ] || continue
    if git ls-files --error-unmatch ".githooks/$hook" >/dev/null 2>&1; then
        git update-index --chmod=+x ".githooks/$hook"
    else
        chmod +x ".githooks/$hook" 2>/dev/null || true
        echo "[gate] .githooks/$hook 还没纳入版本管理，可执行位只写到了磁盘上" >&2
    fi
done

if command -v pengj-templates-cli >/dev/null 2>&1; then
    echo "[gate] 检测到 pengj-templates-cli：钩子会跑完整规则集"
else
    echo "[gate] 钩子已激活，但没检测到 pengj-templates-cli：装好之前提交会被拦下" >&2
    echo "[gate]   门禁就在 CLI 里，装一次即可：" >&2
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
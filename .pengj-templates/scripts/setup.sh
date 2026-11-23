#!/bin/sh
# 由 pengj-templates（common 层）托管。
# 仓库一键装机的零前置薄引导脚本：若缺少 pengj-templates-cli 则自动安装，随后转交 `pengj-templates-cli setup --dir .`。

set -eu

PENGJ_BIN="${XDG_DATA_HOME:-$HOME/.local/share}/pengj-templates/bin"
case ":$PATH:" in
  *":$PENGJ_BIN:"*) ;;
  *) PATH="$PATH:$PENGJ_BIN"; export PATH ;;
esac

if ! command -v pengj-templates-cli >/dev/null 2>&1; then
  DRY_RUN=0
  for arg in "$@"; do
    if [ "$arg" = "--dry-run" ]; then
      DRY_RUN=1
      break
    fi
  done

  if [ "$DRY_RUN" -eq 1 ]; then
    echo "[setup] [dry-run] 未检测到 pengj-templates-cli。"
    echo "[setup] [dry-run] 将下载安装 pengj-templates-cli（目标目录：$PENGJ_BIN）并执行 pengj-templates-cli setup --dir ."
    exit 0
  fi

  echo "[setup] 正在安装 pengj-templates-cli 到 $PENGJ_BIN..." >&2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.sh | sh
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.sh | sh
  else
    echo "[setup] 错误：未找到 curl 或 wget，无法自动下载安装 pengj-templates-cli。" >&2
    exit 1
  fi

  if [ -n "${GITHUB_PATH:-}" ]; then
    printf '%s\n' "$PENGJ_BIN" >> "$GITHUB_PATH"
  fi

  if ! command -v pengj-templates-cli >/dev/null 2>&1; then
    echo "[setup] 错误：安装完成后仍未在 PATH 中找到 pengj-templates-cli（预期目录：$PENGJ_BIN）。" >&2
    exit 1
  fi
fi

exec pengj-templates-cli setup --dir . "$@"
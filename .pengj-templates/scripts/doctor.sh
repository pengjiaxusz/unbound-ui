#!/bin/sh
# 由 pengj-templates（common 层）托管。
# 仓库环境体检的无交互引导脚本：永不自动下载/安装，永不弹 y/n 阻塞输入。

set -eu

PENGJ_BIN="${XDG_DATA_HOME:-$HOME/.local/share}/pengj-templates/bin"
LOCAL_BIN="$HOME/.local/bin"
case ":$PATH:" in
  *":$LOCAL_BIN:"*) ;;
  *) PATH="$LOCAL_BIN:$PATH"; export PATH ;;
esac
case ":$PATH:" in
  *":$PENGJ_BIN:"*) ;;
  *) PATH="$PENGJ_BIN:$PATH"; export PATH ;;
esac

if command -v just >/dev/null 2>&1 \
  && command -v pengj-templates-cli >/dev/null 2>&1 \
  && { [ -f justfile ] || [ -f Justfile ]; }; then
  exec just doctor "$@"
fi

if command -v pengj-templates-cli >/dev/null 2>&1; then
  exec pengj-templates-cli doctor --dir . "$@"
fi

echo "[doctor] 检测到缺失必要工具（本脚本不会自动下载或修改本机环境）：" >&2
echo "" >&2
echo "  - pengj-templates-cli（约 5–10 MB）" >&2
echo "    默认安装路径：$PENGJ_BIN" >&2
echo "    安装命令（Linux/macOS）：" >&2
echo "      curl -fsSL https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.sh | sh" >&2

if ! command -v just >/dev/null 2>&1; then
  echo "" >&2
  echo "  - just（需 >= 1.56.0，约 3–5 MB；若直接调用 CLI 可选装）" >&2
  echo "    常见安装位置：~/.local/bin 或 ~/.cargo/bin（Ubuntu/Debian 请勿使用旧版 apt install just）" >&2
  echo "    推荐安装：curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | sh -s -- --to ~/.local/bin" >&2
  echo "    其他方式：brew install just / cargo install just / https://github.com/casey/just#installation" >&2
fi

echo "" >&2
echo "安装完成后请重新运行：" >&2
echo "  sh .pengj-templates/scripts/doctor.sh" >&2
echo "  # 或：just doctor（如需修复本地钩子/链接，再显式追加 --fix）" >&2
echo "  # 若需在新虚拟机 / CI 一键装齐：sh .pengj-templates/scripts/setup.sh（可先加 --dry-run 预览）" >&2
exit 1
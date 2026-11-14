#!/usr/bin/env bash
# 重新生成仓库中提交的 SPIR-V 二进制文件。
#
# `crates/unbound-gpu/build.rs` 优先使用真实的 `glslc`，在缺少 Vulkan SDK 时
# 回退到 `shaders/spv/` 中的预编译二进制。该回退仅在二进制与 GLSL 源码一致时
# 才正确，因此在着色器变更后需运行此脚本刷新。
#
# 用法：scripts/compile-shaders.sh [output-dir]

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"
shader_dir="$root/crates/unbound-gpu/shaders"
out_dir="${1:-$shader_dir/spv}"

if ! command -v glslc >/dev/null 2>&1; then
    if [ -n "${VULKAN_SDK:-}" ] && [ -x "$VULKAN_SDK/bin/glslc" ]; then
        glslc="$VULKAN_SDK/bin/glslc"
    else
        echo "glslc not found; install the Vulkan SDK or put it on PATH" >&2
        exit 1
    fi
else
    glslc="$(command -v glslc)"
fi

mkdir -p "$out_dir"

for src in "$shader_dir"/*.vert "$shader_dir"/*.frag "$shader_dir"/*.comp; do
    name="$(basename "$src")"
    case "$name" in
        *.vert) stage=vert ;;
        *.frag) stage=frag ;;
        *.comp) stage=comp ;;
        *) continue ;;
    esac
    "$glslc" --target-env=vulkan1.3 -O -I "$shader_dir" \
        -fshader-stage="$stage" "$src" -o "$out_dir/$name.spv"
    echo "  $name.spv"
done

if command -v spirv-val >/dev/null 2>&1; then
    for blob in "$out_dir"/*.spv; do
        spirv-val --target-env vulkan1.3 "$blob"
    done
    echo "validated $(ls -1 "$out_dir"/*.spv | wc -l) modules"
fi

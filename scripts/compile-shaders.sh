#!/usr/bin/env bash
# Regenerate the checked-in SPIR-V blobs.
#
# `crates/unbound-gpu/build.rs` prefers a real `glslc` and falls back to the
# blobs in `shaders/spv/` when the Vulkan SDK is absent. That fallback is only
# correct if the blobs match the GLSL sources, so this script refreshes them
# whenever a shader changes.
#
# Usage: scripts/compile-shaders.sh [output-dir]

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

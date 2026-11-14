//! 在构建时将 GLSL 源码编译为 SPIR-V。
//!
//! 优先在 `$VULKAN_SDK/bin` 中查找 `glslc`，其次在 `PATH` 中查找。若缺失
//! （如未安装 SDK 的 CI 环境），则回退使用 `shaders/spv/` 中已签入的 SPIR-V
//! 二进制文件，确保干净检出仍可正常构建。仅当两者均不可用时才显式报错失败——
//! 静默打包缺少着色器的渲染器比构建失败更危险。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 着色器源文件列表，按报告顺序排列。
const SHADERS: &[&str] = &[
    "uber.vert",
    "uber.frag",
    "opaque_core.frag",
    "opaque_core.vert",
    "cull.comp",
    "scatter.comp",
    "sort_hist.comp",
    "sort_scan.comp",
    "sort_scatter.comp",
    "indirect.comp",
    "mtsdf.comp",
    "mtsdf_pack.comp",
    "blur_down.comp",
    "blur_up.comp",
    "backdrop.vert",
    "backdrop.frag",
];

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn find_tool(name: &str) -> Option<PathBuf> {
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = Path::new(&sdk).join("bin").join(exe(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=shaders");
    for s in SHADERS {
        println!("cargo:rerun-if-changed=shaders/{s}");
    }
    println!("cargo:rerun-if-changed=shaders/common.glsl");
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let spv_dir = out_dir.join("spv");
    std::fs::create_dir_all(&spv_dir).expect("create spv dir");

    let glslc = find_tool("glslc");
    let Some(glslc) = glslc else {
        // 回退使用仓库中已签入的 SPIR-V 二进制文件。
        let prebuilt = Path::new("shaders/spv");
        for s in SHADERS {
            let src = prebuilt.join(format!("{s}.spv"));
            let dst = spv_dir.join(format!("{s}.spv"));
            if !src.is_file() {
                panic!(
                    "glslc not found and no prebuilt SPIR-V at {}.\n\
                     Install the Vulkan SDK (or put glslc on PATH), or commit the prebuilt blobs.",
                    src.display()
                );
            }
            std::fs::copy(&src, &dst).expect("copy prebuilt spv");
        }
        println!("cargo:warning=glslc not found; using prebuilt SPIR-V from shaders/spv");
        return;
    };

    for s in SHADERS {
        let src = Path::new("shaders").join(s);
        let dst = spv_dir.join(format!("{s}.spv"));
        let stage = if s.ends_with(".vert") {
            "vert"
        } else if s.ends_with(".frag") {
            "frag"
        } else {
            "comp"
        };
        let output = Command::new(&glslc)
            .arg("--target-env=vulkan1.3")
            .arg("-O")
            .arg("-I")
            .arg("shaders")
            .arg(format!("-fshader-stage={stage}"))
            .arg(&src)
            .arg("-o")
            .arg(&dst)
            .output()
            .unwrap_or_else(|e| panic!("failed to run {}: {e}", glslc.display()));
        if !output.status.success() {
            panic!(
                "shader compilation failed for {}:\n{}\n{}",
                src.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if !dst.is_file() {
            panic!("glslc reported success but {} is missing", dst.display());
        }
    }

    // 若校验器可用则验证每个模块：在构建期捕获格式异常但可链接的模块，
    // 比留到驱动层再排查代价低得多。
    if let Some(validator) = find_tool("spirv-val") {
        for s in SHADERS {
            let dst = spv_dir.join(format!("{s}.spv"));
            let output = Command::new(&validator)
                .arg("--target-env")
                .arg("vulkan1.3")
                .arg(&dst)
                .output();
            if let Ok(output) = output {
                if !output.status.success() {
                    panic!(
                        "spirv-val rejected {}:\n{}\n{}",
                        dst.display(),
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }
}

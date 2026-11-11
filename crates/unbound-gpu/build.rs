//! Compiles the GLSL sources to SPIR-V at build time.
//!
//! `glslc` is looked up in `$VULKAN_SDK/bin` first, then on `PATH`. When it is
//! absent (CI images without the SDK) the checked-in SPIR-V blobs in
//! `shaders/spv/` are used instead, so a clean checkout still builds. The build
//! fails loudly only when neither is available — silently shipping a renderer
//! without shaders is worse than a red build.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Shader sources, in the order they are reported.
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
        // Fall back to the checked-in blobs.
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

    // Validate every module when the validator is available: a malformed but
    // linkable module is much cheaper to catch here than in a driver.
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

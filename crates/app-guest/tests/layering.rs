//! 依赖分层守卫（`docs` §7.1）。
//!
//! 架构设计承诺业务层保持极低编译开销：它只能依赖纯数据定义
//! （POD 描述符、动作枚举），不得引入图形栈、着色器编译器或重型宏。
//! 该承诺是开发者在应用层迭代时保持增量编译低于 800 ms 的关键。
//!
//!
//!
//! 该规则在此作为测试强制执行，防止随时间腐化：任何破坏分层的依赖都会导致构建失败。
//!

use std::path::PathBuf;

/// 业务层绝不可依赖的 crate。
const FORBIDDEN: [&str; 8] = [
    "unbound-gpu",
    "unbound-text",
    "unbound-layout",
    "unbound-world",
    "unbound-canvas",
    "unbound-input",
    "unbound-a11y",
    "unbound-platform",
];

/// 允许业务层依赖的 crate。
const ALLOWED: [&str; 3] = ["unbound-abi", "serde", "postcard"];

/// 业务层的 manifest 文件。
fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

#[test]
fn the_business_layer_only_depends_on_plain_data() {
    let text = std::fs::read_to_string(manifest()).expect("the manifest is readable");
    for forbidden in FORBIDDEN {
        assert!(
            !text.contains(&format!("{forbidden}.workspace")),
            "the business layer must not depend on {forbidden}: it would drag the \
             graphics stack into every incremental build"
        );
    }
    for allowed in ALLOWED {
        assert!(
            text.contains(&format!("{allowed}.workspace")),
            "the business layer is expected to depend on {allowed}"
        );
    }
}

#[test]
fn the_business_layer_declares_no_graphics_dependencies() {
    let text = std::fs::read_to_string(manifest()).expect("the manifest is readable");
    for crate_name in ["ash", "winit", "taffy", "cosmic-text", "accesskit", "png"] {
        assert!(
            !text.contains(crate_name),
            "the business layer must not depend on {crate_name}"
        );
    }
}

#[test]
fn the_business_layer_is_a_cdylib() {
    let text = std::fs::read_to_string(manifest()).expect("the manifest is readable");
    assert!(
        text.contains("crate-type"),
        "the business layer must be loadable as a dynamic library"
    );
    assert!(text.contains("cdylib"));
}

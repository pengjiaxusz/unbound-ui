//! The dependency layering guardrail (`docs` §7.1).
//!
//! The design promises that the business layer stays cheap to compile: it may
//! only depend on the plain data definitions (the POD descriptors, the action
//! enum) and must not pull in a graphics stack, a shader compiler or a heavy
//! macro. That promise is what keeps the incremental build under 800 ms while
//! a developer works on the application layer.
//!
//! The rule is enforced here, as a test, so it cannot rot: a dependency that
//! would break the layering fails the build.

use std::path::PathBuf;

/// Crates the business layer must never depend on.
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

/// The crates the business layer is allowed to depend on.
const ALLOWED: [&str; 3] = ["unbound-abi", "serde", "postcard"];

/// The manifest of the business layer.
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

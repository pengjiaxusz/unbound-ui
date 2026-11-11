//! Workspace manifest hygiene.
//!
//! Cargo only understands `[lints.cargo]` when invoked on nightly with
//! `-Zcargo-lints`. On the stable toolchain this project pins, every manifest
//! that declares it makes Cargo print
//! `warning: Cargo.toml: unused manifest key: lints.cargo` — once per crate,
//! on every single command.
//!
//! A workspace-level guard test is the only place that always runs: the
//! lefthook hooks are optional installs, and plain `cargo build` has no way to
//! turn a manifest warning into a failure. Keep it that way.

use std::path::{Path, PathBuf};

/// No manifest may use the nightly-only Cargo lint table.
#[test]
fn no_manifest_declares_nightly_only_cargo_lints() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut offenders = Vec::new();
    for manifest in manifests(&root) {
        let text = std::fs::read_to_string(&manifest).expect("manifest is readable");
        if text.lines().any(is_cargo_lint_table) {
            offenders.push(manifest);
        }
    }
    assert!(
        offenders.is_empty(),
        "remove `[lints.cargo]` / `[workspace.lints.cargo]`: stable Cargo does not \
         support it and warns on every command: {offenders:?}"
    );
}

/// A line that opens a Cargo lint table (including sub-tables).
fn is_cargo_lint_table(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("[lints.cargo")
        || line.starts_with("[workspace.lints.cargo")
        || line.starts_with("lints.cargo =")
        || line.starts_with("workspace.lints.cargo =")
}

/// Every `Cargo.toml` under `root`, skipping build output and VCS metadata.
fn manifests(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory).expect("workspace directory is readable");
        for entry in entries {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !matches!(name, "target" | ".git" | "node_modules") {
                    pending.push(path);
                }
            } else if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
                found.push(path);
            }
        }
    }
    found
}

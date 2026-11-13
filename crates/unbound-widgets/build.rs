//! Build-time AST and symbol coverage validator for `unbound-widgets`.
//!
//! Enforces the four-fold compile-time gates:
//! 1. No bare `pub mod` in `lib.rs` (must be declared in `define_widget_catalog!`).
//! 2. Every `pub struct *Desc` must be covered in `catalog::emit_widget`.
//! 3. Every `pub fn emit_*` must be called in `catalog::emit_widget`.
//! 4. Every business component module must be re-exported in `lib.rs` via `pub use <mod>::`.

use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_dir = Path::new(&manifest_dir).join("src");

    let lib_rs = fs::read_to_string(src_dir.join("lib.rs")).expect("failed to read src/lib.rs");
    let catalog_rs =
        fs::read_to_string(src_dir.join("catalog.rs")).expect("failed to read src/catalog.rs");

    // Rule 1: No bare `pub mod` in `lib.rs` other than infrastructure modules
    const ALLOWED_BARE_MODS: &[&str] = &["catalog", "diff", "primitive"];
    for line in lib_rs.lines() {
        let trimmed = line.trim();
        if let Some(stripped) = trimmed.strip_prefix("pub mod ") {
            if let Some(mod_name) = stripped.strip_suffix(';') {
                let mod_name = mod_name.trim();
                if !ALLOWED_BARE_MODS.contains(&mod_name) {
                    panic!(
                        "Compile error: module `{mod_name}` must be declared inside `define_widget_catalog!` macro in src/lib.rs"
                    );
                }
            }
        }
    }

    // Extract modules declared inside define_widget_catalog!
    let catalog_macro_start = lib_rs
        .find("define_widget_catalog!")
        .expect("define_widget_catalog! not found");
    let catalog_macro_body = &lib_rs[catalog_macro_start..];

    // Scan all files in src/*.rs
    let entries = fs::read_dir(&src_dir).expect("failed to read src dir");
    for entry in entries {
        let entry = entry.expect("failed to read dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }

        let file_name = path.file_name().unwrap().to_str().unwrap();
        if file_name == "lib.rs"
            || file_name == "catalog.rs"
            || file_name == "diff.rs"
            || file_name == "primitive.rs"
        {
            continue;
        }

        let mod_name = path.file_stem().unwrap().to_str().unwrap();

        // Check module is inside define_widget_catalog!
        if !catalog_macro_body.contains(mod_name) {
            panic!(
                "Compile error: module `{mod_name}` in `src/{file_name}` is not declared in `define_widget_catalog!` in src/lib.rs"
            );
        }

        // Rule 4: Module must be re-exported in lib.rs
        let expected_pub_use = format!("pub use {}::", mod_name);
        if !lib_rs.contains(&expected_pub_use) {
            panic!(
                "Compile error: module `{mod_name}` in `src/{file_name}` is missing top-level re-export in src/lib.rs (expected `{expected_pub_use}`)"
            );
        }

        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        // Rule 2: Every pub struct *Desc must be in catalog.rs
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("pub struct ") {
                let rest = stripped.trim();
                let ident: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if ident.ends_with("Desc") && !catalog_rs.contains(&ident) {
                    panic!(
                        "Compile error: widget descriptor `{ident}` in `src/{file_name}` is not covered by `catalog::emit_widget` and Showcase!"
                    );
                }
            }
        }

        // Rule 3: Every pub fn emit_* must be called in catalog.rs
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("pub fn emit_") {
                let rest = stripped.trim();
                let suffix: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let ident = format!("emit_{suffix}");
                if !catalog_rs.contains(&ident) {
                    panic!(
                        "Compile error: emitter function `{ident}` in `src/{file_name}` is never called in `catalog::emit_widget`!"
                    );
                }
            }
        }
    }
}

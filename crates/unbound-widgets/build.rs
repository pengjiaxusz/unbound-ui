//! `unbound-widgets` 的构建期 AST 与符号覆盖校验器。
//!
//! 强制执行四项编译期门禁检查：
//! 1. `lib.rs` 中禁止裸 `pub mod`（必须在 `define_widget_catalog!` 中声明）。
//! 2. 每个 `pub struct *Desc` 都必须在 `catalog::emit_widget` 中覆盖。
//! 3. 每个 `pub fn emit_*` 都必须在 `catalog::emit_widget` 中调用。
//! 4. 每个业务组件模块都必须通过 `pub use <mod>::` 在 `lib.rs` 中重导出。

use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_dir = Path::new(&manifest_dir).join("src");

    let lib_rs = fs::read_to_string(src_dir.join("lib.rs")).expect("failed to read src/lib.rs");
    let catalog_rs =
        fs::read_to_string(src_dir.join("catalog.rs")).expect("failed to read src/catalog.rs");

    // 规则 1：`lib.rs` 中除基础设施模块外禁止裸 `pub mod`。
    // `title_bar` 仅提供默认标题栏描述符与保留节点 ID，场景由 `unbound-app` 的
    // `ViewBuilder` 组合既有容器/徽章/按钮生成，没有独立的 `emit_*`，因此按基础设施
    // 模块处理，不纳入 `define_widget_catalog!` 展示目录。
    const ALLOWED_BARE_MODS: &[&str] = &["catalog", "diff", "primitive", "ergonomics", "title_bar"];
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

    // 提取 `define_widget_catalog!` 内部声明的模块
    let catalog_macro_start = lib_rs
        .find("define_widget_catalog!")
        .expect("define_widget_catalog! not found");
    let catalog_macro_body = &lib_rs[catalog_macro_start..];

    // 扫描 `src/*.rs` 下的所有文件
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
            || file_name == "ergonomics.rs"
            || file_name == "title_bar.rs"
        {
            continue;
        }

        let mod_name = path.file_stem().unwrap().to_str().unwrap();

        // 检查模块是否在 `define_widget_catalog!` 中声明
        if !catalog_macro_body.contains(mod_name) {
            panic!(
                "Compile error: module `{mod_name}` in `src/{file_name}` is not declared in `define_widget_catalog!` in src/lib.rs"
            );
        }

        // 规则 4：模块必须在 `lib.rs` 中重导出
        let expected_pub_use = format!("pub use {}::", mod_name);
        if !lib_rs.contains(&expected_pub_use) {
            panic!(
                "Compile error: module `{mod_name}` in `src/{file_name}` is missing top-level re-export in src/lib.rs (expected `{expected_pub_use}`)"
            );
        }

        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        // 规则 2：每个 `pub struct *Desc` 都必须在 `catalog.rs` 中出现
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

        // 规则 3：每个 `pub fn emit_*` 都必须在 `catalog.rs` 中调用
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

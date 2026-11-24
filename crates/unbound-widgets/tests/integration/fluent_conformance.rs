//! 自有独有控件的 Fluent 设计规范合规巡检。
//!
//! 三合一清单第五节列出的 44 个自有独有控件没有 WinUI 3 一对一参考图，因此不能
//! 走像素差分；它们依靠本测试守住三条可机械验证的底线：
//!
//! 1. 分类与数量稳定（44 个，全部为 [`ControlClass::Exclusive`]）；
//! 2. 生产代码不出现硬编码颜色构造，所有颜色必须经主题令牌解析；
//! 3. 圆角字面量只允许 `0`、`4`、`8` 三档（胶囊圆角由高度计算，不写死字面量）。
//!
//! 24 种 `ControlState` 组合下的图元数量恒定由 `tests/stable_shape.rs` 统一覆盖，
//! 本测试不重复探针逻辑，只在文档中保留交叉引用。

use std::collections::BTreeSet;
use std::path::PathBuf;

use unbound_widgets::canonical::{ControlClass, EXCLUSIVE_COUNT};
use unbound_widgets::WidgetCatalogId;

/// 去掉单元测试区段，避免把测试里的示意数值当成生产样式。
fn production_source(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

/// 提取形如 `radius: 12.0` 与 `.with_radius(6.0)` 的字面量。
fn radius_literals(source: &str) -> Vec<&str> {
    let mut literals = Vec::new();
    for line in source.lines() {
        for needle in ["radius:", ".with_radius("] {
            let Some(start) = line.find(needle) else {
                continue;
            };
            let rest = &line[start + needle.len()..];
            let rest = rest.trim_start();
            let end = rest
                .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
                .unwrap_or(rest.len());
            let literal = &rest[..end];
            if !literal.is_empty() {
                literals.push(literal);
            }
        }
    }
    literals
}

#[test]
fn exactly_forty_three_exclusive_controls_are_registered() {
    let ids: Vec<WidgetCatalogId> =
        WidgetCatalogId::ids_in_class(ControlClass::Exclusive).collect();
    assert_eq!(ids.len(), EXCLUSIVE_COUNT);
    assert_eq!(EXCLUSIVE_COUNT, 44);
    for id in ids {
        assert!(id.is_exclusive());
        assert!(!id.is_baseline_aligned());
        assert!(!id.winui3_name().is_empty());
    }
}

#[test]
fn exclusive_controls_never_hardcode_colors_or_off_ladder_radii() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut modules = BTreeSet::new();
    for id in WidgetCatalogId::ids_in_class(ControlClass::Exclusive) {
        modules.insert(id.module_name());
    }

    let mut failures = Vec::new();
    for module in modules {
        let path = root.join(format!("{module}.rs"));
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let source = production_source(&source);
        if source.contains("Color::rgb")
            || source.contains("Color::rgba")
            || source.contains("Color::from_rgba")
        {
            failures.push(format!("{module}: hardcoded color constructor"));
        }
        for literal in radius_literals(source) {
            // 统一补齐小数形式后比较：`0` 与 `0.0` 等价，`4.0` 与 `4` 等价。
            let normalized = literal.trim_end_matches(".0");
            if normalized != "0" && normalized != "4" && normalized != "8" {
                failures.push(format!("{module}: off-ladder radius {literal}"));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Fluent conformance failures:\n{}",
        failures.join("\n")
    );
}

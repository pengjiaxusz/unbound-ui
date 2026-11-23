//! `unbound-app` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p unbound-app --test integration <模块名>` 按模块过滤。

mod appearance_ledger;
mod asset_pipeline;
mod declarative_ergonomics;
mod event_bubbling;
mod form_binding;
mod form_validation;
mod guest_adapter;
mod key_scope_action;
mod layout_runtime;
mod overlay_focus_trap;
mod router_navigation;
mod runtime_headless;
mod runtime_subsystems;
mod settings_and_select;

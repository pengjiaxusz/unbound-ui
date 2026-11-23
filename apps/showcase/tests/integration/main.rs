//! `showcase` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p showcase --test integration <模块名>` 按模块过滤。

/// 进程级环境变量互斥锁，防止修改 `UNBOUND_TEXT_FORCE_DW_INIT_FAIL` 等钩子时与同二进制内其他测试并发竞态。
pub(crate) static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod backdrop;
mod frame_plan;
mod gallery;
mod i18n_regression;
mod runtime_migration;
mod shell;
mod text_render;
mod theme;

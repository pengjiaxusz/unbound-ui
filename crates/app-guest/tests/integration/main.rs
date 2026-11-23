//! `app-guest` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p app-guest --test integration <模块名>` 按模块过滤。

mod host_roundtrip;
mod layering;
mod schema_evolution;

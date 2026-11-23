//! `unbound-agent` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p unbound-agent --test integration <模块名>` 按模块过滤。

mod agent_bridge;
mod determinism;
mod pixel_layer;
mod replay_minimize;
mod semantic_loop;
mod snapshot_restore;

//! `unbound-backdrop` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p unbound-backdrop --test integration <模块名>` 按模块过滤。

mod backdrop_accent_tint;
mod backdrop_acrylic;
mod backdrop_fallback;
mod backdrop_immersive;
mod backdrop_mica_alt;
mod backdrop_mica_alt_self_drawn;

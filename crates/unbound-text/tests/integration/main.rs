//! `unbound-text` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p unbound-text --test integration <模块名>` 按模块过滤。

/// 进程级环境变量互斥锁，防止修改 `UNBOUND_TEXT_FORCE_DW_INIT_FAIL` 等钩子时与同二进制内其他测试并发竞态。
pub(crate) static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod atlas_lazy_allocation;
mod backend_matrix;
#[cfg(all(windows, feature = "backend-directwrite"))]
mod color_bitmap_layer;
#[cfg(all(windows, feature = "backend-directwrite"))]
mod color_cluster_directwrite;
mod color_composite;
mod distance_field_console;
mod emoji_settings;
mod mono_silhouette;
mod phase_mode;
mod raster_switching;
mod render_settings;

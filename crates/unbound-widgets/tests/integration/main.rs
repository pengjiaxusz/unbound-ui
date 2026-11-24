//! `unbound-widgets` 聚合集成测试入口。
//!
//! 将多个集成测试模块聚合到单一测试二进制中，减少重复链接开销；
//! 可通过 `cargo test -p unbound-widgets --test integration <模块名>` 按模块过滤。

mod button_variants;
mod button_vector_icon;
mod capability_scrollbar_contract;
mod checkbox_states;
mod fluent_conformance;
mod health;
mod image_fit_and_radius;
mod menu_submenu;
mod modal_focus_trap;
mod multi_hit_and_widgets;
mod navigation_view;
mod overlay_placement;
mod pilot_geometry;
mod popover_arrow;
mod radio_group;
mod schema_form_adaptive;
mod scroll_view;
mod segmented_nav;
mod select_control;
mod slice_1_parity;
mod slice_2_parity;
mod slice_3_parity;
mod slice_4_parity;
mod slider_keyboard_and_range;
mod slot_fixtures;
mod stable_shape;
mod switch_spring;
mod symbol_font_fallback;
mod text_input;
mod tooltip_timer;

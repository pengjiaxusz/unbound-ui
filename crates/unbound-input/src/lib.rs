//! 输入管线（`docs` §6.1 – §6.3）。
//!
//! 本 crate 对应架构文档中的四级漏斗：浮层 → UI 树 → 四叉树粗筛 →
//! 解析精测。它自身不持有任何几何数据：调用方传入由布局树、画布索引和
//! Portal 浮层构建的扁平 [`HitRegion`] 列表，本 crate 确定性地回答
//! “谁拥有该坐标点”。
//!
//!
//! 确定性至关重要，因为同一次拾取同时驱动三个消费方——
//! 点击目标、悬停高亮与光标样式——三者绝不能产生分歧。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod cursor;
pub mod dnd;
pub mod focus;
pub mod hit;
pub mod keymap;
pub mod pointer;
pub mod router;

pub use cursor::{CursorKind, CursorSource, CursorStack};
pub use dnd::{DragOver, DragPayload, PayloadArena};
pub use focus::{
    FocusCause, FocusConfig, FocusGroup, FocusGroupMode, FocusKey, FocusManager, RovingKey,
    RovingOrientation, RovingRegistry,
};
pub use hit::{
    pick, pick_index, pick_stack, point_in_polygon, resize_edge, rounded_rect_sdf, ClipRect,
    HitRegion, ResizeEdge, Shape, PORTAL_LAYER,
};
pub use keymap::{KeyBinding, KeyChord, KeyName, Keymap};
pub use pointer::{Buttons, PointerButton, PointerConfig, PointerEvent, PointerState};
pub use router::Router;

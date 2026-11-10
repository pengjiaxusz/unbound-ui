//! 无限画布的空间领域层（`docs` §5）。
//!
//! This crate owns everything that is *space*, not layout: the loose quadtree
//! that answers "what is near this rectangle", the smart-snapping solver that
//! turns a drag into a pixel-aligned coordinate plus transient guides, the
//! marquee/transform selection model, and the copy-on-write transaction stack
//! that makes every edit undoable.
//!
//! Everything here is pure CPU and allocation-disciplined: it runs unchanged in
//! the headless harness (§7.2) and inside the GPU-driven renderer.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod quadtree;
pub mod snap;

pub use quadtree::{Element, LooseQuadtree, QueryStats, MAX_DEPTH};
pub use snap::{Axis, SnapConfig, SnapGuide, SnapKind, SnapResult, Snapper};

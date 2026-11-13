//! 输入管线（`docs` §6.1 – §6.3）。
//!
//! The crate mirrors the doc's four-stage funnel: overlay → UI tree → quadtree
//! broad phase → analytic narrow phase. It keeps no geometry of its own: the
//! caller hands over a flat list of [`HitRegion`]s built from the layout tree,
//! the canvas index and the Portal overlay, and this crate answers "who owns
//! this point" deterministically.
//!
//! Determinism matters because the same pick drives three consumers at once —
//! the click target, the hover highlight and the cursor — and they must never
//! disagree.

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

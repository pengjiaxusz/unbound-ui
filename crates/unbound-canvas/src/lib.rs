//! 无限画布的空间领域层。
//!
//! 本 crate 负责所有与*空间*（而非布局）相关的逻辑：用于回答“此矩形附近有哪些元素”的
//! 松散四叉树（LooseQuadtree）、将拖拽转换为像素对齐坐标并生成临时参考线的智能吸附求解器、
//! 框选/变换选择模型、512+4px Halo 大图瓦片金字塔求解器（ADR-0020），以及让每次编辑都可撤销的写时复制事务栈。
//!
//! 本 crate 完全基于纯 CPU 且严格控制内存分配：可在无头测试套件与 GPU 驱动渲染器中原样运行。
//! （本 crate 不包含任何平台或图形后端依赖。）

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod quadtree;
pub mod select;
pub mod snap;
pub mod tile_pyramid;
pub mod undo;

pub use quadtree::{Element, LooseQuadtree, QueryStats, MAX_DEPTH};
pub use select::{Handle, HandleDrag, Marquee, Modifiers, Selection, SelectionMode, Transform2};
pub use snap::{Axis, SnapConfig, SnapGuide, SnapKind, SnapResult, Snapper};
pub use tile_pyramid::{
    plan_visible_tiles, select_lod, tile_uv_rect, TileDrawCommand, TileKey, TilePlan,
    TilePyramidSpec, TILE_CONTENT_SIZE, TILE_HALO_PX, TILE_PHYSICAL_SIZE,
};
pub use undo::{Edit, Transaction, TransactionKind, UndoStack};

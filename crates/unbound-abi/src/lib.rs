//! `unbound-abi` —— host 进程与可热重载 guest UI 逻辑之间的稳定边界。
//!
//! 仅以下三项核心契约驻留于此，因为它们必须在 `guest_gen_N.dll` 切换
//! 过程中保持二进制稳定：
//!
//! 1. [`desc`] —— guest 发出的扁平 POD 视图树描述符。
//! 2. [`action`] —— 可序列化的动作记录（树中不含闭包）。
//! 3. [`plugin`] —— 分配器与插件调用的 `#[repr(C)]` 虚表。
//!
//! 本 crate 不依赖任何图形、布局或文本模块，因此 `app_guest` 可在
//! 毫秒级内完成编译（`docs` §7.1）。
//!

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

#[macro_use]
mod macros;

pub mod action;
pub mod desc;
pub mod migration;
pub mod plugin;
pub mod schema;

pub use action::{ActionKind, RawAction};
pub use desc::{
    Align, Dim, DimUnit, Display, Edges, FlexDirection, Justify, NodeFlags, NodeTag, Position,
    RawNodeDesc, RawNodeId, NO_NODE,
};
pub use migration::{
    decode, encode, migrate, Field, FieldSpec, MigratedField, MigrationError, ENVELOPE_VERSION,
    MAGIC,
};
pub use plugin::{
    check_abi, classify_handoff, AbiCompat, GuestEntryFn, GuestPluginVTable, GuestState,
    HostAllocatorVTable, HostRenderContext, StateHandoff, ABI_VERSION, GUEST_ENTRY_SYMBOL,
};
pub use schema::{StateSchema, FNV_OFFSET_BASIS};

/// 为 `bitflags_like!` 宏重导出符号，使调用方无需直接依赖 `bytemuck`。
///
#[doc(hidden)]
pub mod __private {
    pub use bytemuck::{Pod, Zeroable};
}

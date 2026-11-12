//! `unbound-abi` — the stable boundary between the host process and
//! hot-reloadable guest UI logic.
//!
//! Three concerns live here and nowhere else, because all three must remain
//! binary-stable across a `guest_gen_N.dll` swap:
//!
//! 1. [`desc`] — the flat POD view-tree descriptor the guest emits.
//! 2. [`action`] — the serialisable action record (no closures in the tree).
//! 3. [`plugin`] — the `#[repr(C)]` vtables for allocator and plugin calls.
//!
//! Nothing in this crate depends on graphics, layout or text, so `app_guest`
//! compiles in milliseconds (`docs` §7.1).

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

/// Re-exports for the `bitflags_like!` macro so callers do not need a direct
/// `bytemuck` dependency.
#[doc(hidden)]
pub mod __private {
    pub use bytemuck::{Pod, Zeroable};
}

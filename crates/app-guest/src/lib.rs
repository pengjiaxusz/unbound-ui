//! 业务层 Guest（`docs` §8.1）。
//!
//! This crate *is* the business layer: a `cdylib` that the host mounts, unmounts
//! and remounts while the window, the Vulkan device and every GPU pool stay
//! alive. It touches no graphics API and depends only on `unbound-abi` — the
//! lightweight POD/`Action` crate — which is what keeps a guest rebuild in the
//! hundreds of milliseconds instead of the tens of seconds.
//!
//! Two mechanisms make the remount safe:
//!
//! * **Allocator hijack.** [`ForwardingAllocator`] replaces the guest's global
//!   allocator and forwards to the host's vtable, so a `Vec<u8>` the host passes
//!   in and the guest fills is one heap, not two. Without it, Windows would
//!   corrupt the heap the moment `migrate_export` pushed a byte.
//! * **Schema fingerprint.** [`State`] implements `StateSchema`; equal hashes
//!   let the host hand the old pointer over verbatim, a changed hash routes
//!   through `migrate_export`/`migrate_import`.

#![forbid(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::alloc::{GlobalAlloc, Layout, System};

use serde::{Deserialize, Serialize};
use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::{RawNodeDesc, RawNodeId, NO_NODE};
use unbound_abi::plugin::{
    GuestPluginVTable, GuestState, HostAllocatorVTable, HostRenderContext, ABI_VERSION,
};
use unbound_abi::schema::StateSchema;

// ---------------------------------------------------------------------------
// Allocator hijack
// ---------------------------------------------------------------------------

/// Host allocator, installed by `on_load`. Null until then, which is why the
/// fallback below exists: `on_load` itself needs to allocate.
static HOST_ALLOC: AtomicPtr<HostAllocatorVTable> = AtomicPtr::new(core::ptr::null_mut());

/// Guest allocator that forwards every request to the host.
struct ForwardingAllocator;

impl ForwardingAllocator {
    fn table() -> *mut HostAllocatorVTable {
        HOST_ALLOC.load(Ordering::Acquire)
    }
}

unsafe impl GlobalAlloc for ForwardingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let table = Self::table();
        if table.is_null() {
            // SAFETY: the system allocator is always valid.
            return unsafe { System.alloc(layout) };
        }
        // SAFETY: the host installed the vtable in `on_load` and keeps it alive
        // for the whole mount.
        unsafe { ((*table).alloc)(layout.size(), layout.align()) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let table = Self::table();
        if table.is_null() {
            // SAFETY: the block came from `System` because the table was null.
            return unsafe { System.dealloc(ptr, layout) };
        }
        // SAFETY: same allocation source as `alloc`.
        unsafe { ((*table).dealloc)(ptr, layout.size(), layout.align()) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let table = Self::table();
        if table.is_null() {
            return unsafe { System.realloc(ptr, layout, new_size) };
        }
        // SAFETY: same allocation source as `alloc`.
        unsafe { ((*table).realloc)(ptr, layout.size(), layout.align(), new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: ForwardingAllocator = ForwardingAllocator;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One row in the demo list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub label: String,
    pub done: bool,
}

/// The guest's whole business state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub count: i64,
    pub title: String,
    pub rows: Vec<Row>,
}

// Field order and offsets are the contract: adding `rows` changes the hash, and
// the host falls back to `migrate_*` instead of reinterpreting the old memory.
unbound_abi::impl_schema_hash!(
    State,
    0x5542_4f55_4e44, // "UNBOUND"
    "count",
    "i64",
    0,
    "title",
    "String",
    8,
    "rows",
    "Vec<Row>",
    32,
);

impl State {
    fn seed() -> Self {
        Self {
            count: 0,
            title: "Hot reload demo".to_owned(),
            rows: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Guest lifecycle
// ---------------------------------------------------------------------------

unsafe extern "C" fn on_load(alloc: *const HostAllocatorVTable) {
    HOST_ALLOC.store(alloc.cast_mut(), Ordering::Release);
}

unsafe extern "C" fn on_unload() {
    // Leave the pointer in place: dropping the guest does not invalidate the
    // host's vtable, and a re-`on_load` simply overwrites it.
}

unsafe extern "C" fn init_state() -> GuestState {
    Box::into_raw(Box::new(State::seed())).cast::<c_void>()
}

unsafe extern "C" fn drop_state(state: GuestState) {
    if state.is_null() {
        return;
    }
    // SAFETY: `state` came from `Box::into_raw` in this crate (or in a previous
    // generation with the same layout, which is the whole point of ZeroCopy).
    drop(unsafe { Box::from_raw(state.cast::<State>()) });
}

unsafe extern "C" fn adopt_state(_state: GuestState) {
    // Nothing to do: the host keeps owning the pointer and hands it back to us
    // on every call. The schema hash proved the layout is identical.
}

unsafe extern "C" fn migrate_export(state: GuestState, out: *mut Vec<u8>) {
    if state.is_null() || out.is_null() {
        return;
    }
    // SAFETY: both pointers are host-provided and live for this call.
    let state = unsafe { &*state.cast::<State>() };
    let out = unsafe { &mut *out };
    if let Ok(bytes) = postcard::to_allocvec(state) {
        out.extend_from_slice(&bytes);
    }
}

unsafe extern "C" fn migrate_import(ptr: *const u8, len: usize) -> GuestState {
    if ptr.is_null() {
        // SAFETY: `init_state` only allocates.
        return unsafe { init_state() };
    }
    // SAFETY: the host passes the exact buffer `migrate_export` filled.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    let state = postcard::from_bytes::<State>(bytes).unwrap_or_else(|_| State::seed());
    Box::into_raw(Box::new(state)).cast::<c_void>()
}

unsafe extern "C" fn reduce(state: GuestState, action: *const RawAction) {
    if state.is_null() || action.is_null() {
        return;
    }
    // SAFETY: the host guarantees both pointers for the duration of the call.
    let state = unsafe { &mut *state.cast::<State>() };
    let action = unsafe { &*action };
    match action.kind() {
        ActionKind::Press => {
            state.count += 1;
            state.rows.push(Row {
                label: format!("item {}", state.count),
                done: false,
            });
        }
        ActionKind::Undo => {
            if state.rows.pop().is_some() {
                state.count -= 1;
            }
        }
        ActionKind::Custom => {
            let index = action.payload[0] as usize;
            if let Some(row) = state.rows.get_mut(index) {
                row.done = !row.done;
            }
        }
        ActionKind::SetTheme => {
            // Demonstrates that the whole business layer is just data.
            state.count = 0;
            state.rows.clear();
        }
        _ => {}
    }
}

unsafe extern "C" fn build_view(state: GuestState, ctx: *mut HostRenderContext) {
    if state.is_null() || ctx.is_null() {
        return;
    }
    // SAFETY: the host passes a state it owns and a context valid for this call.
    let state = unsafe { &*state.cast::<State>() };
    let ctx = unsafe { &*ctx };

    let root_desc = RawNodeDesc::text().size(720.0, 480.0).row();
    let root = unsafe { ctx.push(RawNodeId(NO_NODE), &root_desc) };

    let title = format!("{} — {} items", state.title, state.rows.len());
    let title_desc = RawNodeDesc::text().size(720.0, 32.0);
    let title_node = unsafe { ctx.push(root, &title_desc) };
    unsafe { ctx.text(title_node, title.as_ptr(), title.len()) };

    for (index, row) in state.rows.iter().enumerate() {
        let desc = RawNodeDesc::text().size(720.0, 24.0);
        let node = unsafe { ctx.push(root, &desc) };
        let marker = if row.done { "[x]" } else { "[ ]" };
        let label = format!("{marker} {}", row.label);
        unsafe { ctx.text(node, label.as_ptr(), label.len()) };
        ctx.bind(
            node,
            RawAction {
                kind: ActionKind::Custom as u32,
                target: node.0,
                payload: [index as u64, 0, 0],
                timestamp_us: 0,
            },
        );
    }
    ctx.done(root);
}

unsafe extern "C" fn snapshot(state: GuestState, out: *mut Vec<u8>) {
    // SAFETY: same contract as `migrate_export`.
    unsafe { migrate_export(state, out) };
}

/// The vtable the host reads through [`unbound_abi::plugin::GUEST_ENTRY_SYMBOL`].
static VTABLE: GuestPluginVTable = GuestPluginVTable {
    abi_version: ABI_VERSION,
    state_schema_hash: <State as StateSchema>::SCHEMA_HASH,
    on_load,
    on_unload,
    init_state,
    drop_state,
    adopt_state,
    migrate_export,
    migrate_import,
    reduce,
    build_view,
    snapshot,
};

/// The single exported symbol.
///
/// # Safety
/// Callers must keep the returned pointer valid for the lifetime of the loaded
/// library; it points at a `static`, so it always is.
#[no_mangle]
pub extern "C" fn unbound_guest_entry() -> *const GuestPluginVTable {
    &VTABLE
}

/// The vtable, for statically linked hosts and for tests.
#[must_use]
pub fn vtable() -> GuestPluginVTable {
    VTABLE
}

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
use unbound_abi::migration::{
    encode, encode_list, migrate, migrate_list, Field, FieldSpec, MigratedField, MigrationError,
};
use unbound_abi::plugin::{
    ByteBuffer, GuestPluginVTable, GuestState, HostAllocatorVTable, HostRenderContext, ABI_VERSION,
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
///
/// `repr(C)` is not optional here. The host hands the *old* state to the
/// *new* guest verbatim when the schema hashes match, so both builds must
/// agree on the byte layout. Under the default `repr(Rust)` the compiler is
/// free to reorder fields — and it does: it moved `State::count` to offset
/// 48 — and it may do so differently in another compiler version. A
/// hand-written fingerprint cannot protect against that; only a fixed layout
/// can.
///
/// `priority` and `tags` were added after V1 shipped. Rows travel as one UBMF
/// envelope each (see [`encode_rows`]), so a V1 payload simply has no field 3
/// or 4 and the new guest fills the defaults instead of losing the rows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct Row {
    pub label: String,
    pub done: bool,
    pub priority: u8,
    pub tags: Vec<String>,
}

// The row's own layout is part of the state's layout: `Vec<Row>` is 24 bytes
// whatever `Row` looks like, so without this the fingerprint would not change
// when `Row` gains a field — and the host would hand the old memory to the
// new guest as if the layouts still matched.
//
// The seed changed when `Row` gained `repr(C)`: the field list is unchanged,
// but the byte layout is not, and the fingerprint has to say so.
unbound_abi::impl_schema_hash!(
    Row,
    0x524f_5745_4e43, // "ROWNB" + repr(C)
    "label",
    "String",
    0,
    "done",
    "bool",
    24,
    "priority",
    "u8",
    25,
    "tags",
    "Vec<String>",
    32,
);

/// The guest's whole business state.
///
/// See [`Row`] for why this is `repr(C)`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct State {
    pub count: i64,
    pub title: String,
    pub rows: Vec<Row>,
}

// The offsets written into the schema hashes above are the contract: if they
// drift away from the real layout, the fingerprint stops describing the
// layout and the host's zero-copy fast path becomes unsafe. Pin them here so
// a wrong offset fails to compile instead of silently weakening the hash.
const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(Row, label) == 0);
    assert!(
        offset_of!(Row, done) == 24,
        "Row::done follows a 24-byte String"
    );
    assert!(offset_of!(State, count) == 0);
    assert!(offset_of!(State, title) == 8);
    assert!(offset_of!(State, rows) == 32);
    assert!(offset_of!(Row, priority) == 25);
    assert!(offset_of!(Row, tags) == 32);
    assert!(core::mem::size_of::<Row>() == 56);
    assert!(core::mem::size_of::<State>() == 56);
};

// Field order and offsets are the contract: adding `rows` changes the hash, and
// the host falls back to `migrate_*` instead of reinterpreting the old memory.
//
// `Row` is listed as a nested type: a change inside it changes the state's
// layout too, even though `Vec<Row>` itself never changes size.
//
// The seed changed when `State` gained `repr(C)`, for the same reason as
// `Row`'s: the field list is unchanged, the byte layout is not.
unbound_abi::impl_schema_hash!(
    State,
    0x5542_4f55_4e45, // "UNBOUND" + repr(C)
    nested: [Row],
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
// Migratable state
// ---------------------------------------------------------------------------

/// Stable field ids for the versioned envelope. They must never be reused.
const FIELD_COUNT: u16 = 1;
const FIELD_TITLE: u16 = 2;
const FIELD_ROWS: u16 = 3;
/// `postcard` encoding of an empty `String` / `Vec` / zero `u8`.
const EMPTY_COLLECTION: &[u8] = &[0];
/// An empty row list: the `encode_list` header is a `u32` zero.
const EMPTY_ROWS: &[u8] = &[0, 0, 0, 0];

/// The new shape of the state. Fields absent from an older payload take a
/// default; a missing `count` is an error the host can see.
const STATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::required(FIELD_COUNT),
    FieldSpec::optional(FIELD_TITLE, EMPTY_COLLECTION),
    FieldSpec::optional(FIELD_ROWS, EMPTY_ROWS),
];

// Row field ids. A V1 row only carries 1 and 2; 3 and 4 are the fields this
// version added, and their defaults are what a V1 payload gets.
const ROW_LABEL: u16 = 1;
const ROW_DONE: u16 = 2;
const ROW_PRIORITY: u16 = 3;
const ROW_TAGS: u16 = 4;

const ROW_FIELDS: &[FieldSpec] = &[
    FieldSpec::required(ROW_LABEL),
    FieldSpec::required(ROW_DONE),
    FieldSpec::optional(ROW_PRIORITY, EMPTY_COLLECTION),
    FieldSpec::optional(ROW_TAGS, EMPTY_COLLECTION),
];

/// Encode rows as a list of self-describing envelopes.
///
/// # Errors
/// Propagates a field that cannot be serialised.
pub fn encode_rows(rows: &[Row]) -> Result<Vec<u8>, MigrationError> {
    encode_list(rows, |row| {
        let label = postcard::to_allocvec(&row.label).map_err(|_| MigrationError::FieldTooLarge)?;
        let done = postcard::to_allocvec(&row.done).map_err(|_| MigrationError::FieldTooLarge)?;
        let priority =
            postcard::to_allocvec(&row.priority).map_err(|_| MigrationError::FieldTooLarge)?;
        let tags = postcard::to_allocvec(&row.tags).map_err(|_| MigrationError::FieldTooLarge)?;
        encode(&[
            Field::new(ROW_LABEL, &label),
            Field::new(ROW_DONE, &done),
            Field::new(ROW_PRIORITY, &priority),
            Field::new(ROW_TAGS, &tags),
        ])
    })
}

/// Decode rows, filling the fields an older row did not have.
///
/// # Errors
/// Returns a structural [`MigrationError`] when the list or one row is
/// corrupt, and [`MigrationError::MissingField`] when a required field is
/// absent.
pub fn decode_rows(bytes: &[u8]) -> Result<Vec<Row>, MigrationError> {
    migrate_list(bytes, ROW_FIELDS, |fields| {
        Ok(Row {
            label: row_field(fields, ROW_LABEL)?,
            done: row_field(fields, ROW_DONE)?,
            priority: row_field(fields, ROW_PRIORITY)?,
            tags: row_field(fields, ROW_TAGS)?,
        })
    })
}

fn row_field<T: serde::de::DeserializeOwned>(
    fields: &[MigratedField<'_>],
    id: u16,
) -> Result<T, MigrationError> {
    let field = fields
        .iter()
        .find(|field| field.id == id)
        .ok_or(MigrationError::MissingField(id))?;
    postcard::from_bytes(&field.bytes).map_err(|_| MigrationError::Truncated)
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

unsafe extern "C" fn migrate_export(state: GuestState, out: *mut ByteBuffer) {
    if state.is_null() || out.is_null() {
        return;
    }
    // SAFETY: both pointers are host-provided and live for this call.
    let state = unsafe { &*state.cast::<State>() };
    let out = unsafe { &mut *out };
    // Each field is encoded on its own so a new field can be added without
    // reshuffling the bytes of the old ones.
    let (Ok(count), Ok(title), Ok(rows)) = (
        postcard::to_allocvec(&state.count),
        postcard::to_allocvec(&state.title),
        encode_rows(&state.rows),
    ) else {
        out.len = 0;
        return;
    };
    let envelope = unbound_abi::migration::encode(&[
        Field::new(FIELD_COUNT, &count),
        Field::new(FIELD_TITLE, &title),
        Field::new(FIELD_ROWS, &rows),
    ]);
    let Ok(bytes) = envelope else {
        // The state could not be serialised. Report "needs nothing" so the
        // host does not allocate a buffer for a payload that does not exist.
        out.len = 0;
        return;
    };
    write_bytes(out, &bytes);
}

unsafe extern "C" fn migrate_import(ptr: *const u8, len: usize) -> GuestState {
    if ptr.is_null() {
        // SAFETY: `init_state` only allocates.
        return unsafe { init_state() };
    }
    // SAFETY: the host passes the exact buffer `migrate_export` filled.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    // A decode failure must be *visible*: the host falls back to a fresh state
    // when this returns null, which is better than silently handing back an
    // empty seed as if the migration had succeeded.
    let Ok(fields) = migrate(bytes, STATE_FIELDS) else {
        return core::ptr::null_mut();
    };
    let decode = |id: u16| fields.iter().find(|field| field.id == id);
    let Some(count) =
        decode(FIELD_COUNT).and_then(|field| postcard::from_bytes::<i64>(&field.bytes).ok())
    else {
        return core::ptr::null_mut();
    };
    let Some(title) =
        decode(FIELD_TITLE).and_then(|field| postcard::from_bytes::<String>(&field.bytes).ok())
    else {
        return core::ptr::null_mut();
    };
    let Some(rows) = decode(FIELD_ROWS).and_then(|field| decode_rows(&field.bytes).ok()) else {
        return core::ptr::null_mut();
    };
    let state = State { count, title, rows };
    Box::into_raw(Box::new(state)).cast::<c_void>()
}

/// Write `bytes` into the host's buffer.
///
/// The host owns the allocation; the guest only fills it. When the buffer is
/// too small the guest writes nothing and reports the size it needs, so the
/// host can allocate a bigger one and call again.
fn write_bytes(out: &mut ByteBuffer, bytes: &[u8]) {
    let needed = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    if out.capacity < needed || out.ptr.is_null() {
        // Not enough room: report the size the host has to provide.
        out.len = needed;
        return;
    }
    // SAFETY: the host guarantees `capacity` writable bytes at `ptr`.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out.ptr, bytes.len());
    }
    out.len = needed;
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
                priority: 0,
                tags: Vec::new(),
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

unsafe extern "C" fn snapshot(state: GuestState, out: *mut ByteBuffer) {
    // SAFETY: same contract as `migrate_export`.
    unsafe { migrate_export(state, out) };
}

/// The vtable the host reads through [`unbound_abi::plugin::GUEST_ENTRY_SYMBOL`].
static VTABLE: GuestPluginVTable = GuestPluginVTable {
    abi_version: ABI_VERSION,
    struct_size: core::mem::size_of::<GuestPluginVTable>() as u32,
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

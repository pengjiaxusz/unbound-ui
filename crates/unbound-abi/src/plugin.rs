//! The Host ↔ Guest C ABI.
//!
//! `docs` §8.1 requires a strictly `#[repr(C)]` function-pointer vtable so the
//! guest can be unloaded and rebuilt while the host keeps its window, Vulkan
//! device and memory pools alive.

use core::ffi::c_void;

use crate::action::RawAction;
use crate::desc::{RawNodeDesc, RawNodeId};

/// Bumped whenever the *layout* of any POD in this crate changes in a
/// non-additive way. The host refuses to load a guest with a different major
/// version instead of corrupting its heap.
///
/// The version is a plain counter, not a major/minor pair: a guest is either
/// ABI-compatible or it is not. Every layout change bumps it, and the host
/// compares the guest's `struct_size` as well, so an additive change that grew
/// a struct is caught by the size check rather than by the version.
pub const ABI_VERSION: u32 = 4;

/// A growable byte buffer the guest fills.
///
/// `Vec<u8>` cannot cross the ABI: its layout is not `#[repr(C)]` and is not
/// covered by Rust's stability guarantees, so a guest built by a different
/// compiler could interpret it differently. The host owns the allocation and
/// the guest only writes into it.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct ByteBuffer {
    /// The bytes. Null when `capacity` is zero.
    pub ptr: *mut u8,
    /// How many bytes the buffer can hold.
    pub capacity: u32,
    /// How many bytes were written.
    pub len: u32,
}

/// Allocator vtable handed to the guest at load time.
///
/// The guest installs a `#[global_allocator]` that forwards here, so every
/// `Box`/`Vec`/`String` allocated inside `guest.dll` lives in the host's single
/// heap (`mimalloc`). This eliminates the Windows cross-CRT-heap corruption
/// class of bugs (`docs` §8.1).
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct HostAllocatorVTable {
    /// Allocate `size` bytes aligned to `align`. Returns null on failure.
    pub alloc: unsafe extern "C" fn(size: usize, align: usize) -> *mut u8,
    /// Free a block previously returned by `alloc`/`realloc`.
    pub dealloc: unsafe extern "C" fn(ptr: *mut u8, size: usize, align: usize),
    /// Grow/shrink in place or move. Returns null on failure (old block stays
    /// valid, as required by `GlobalAlloc`).
    pub realloc: unsafe extern "C" fn(
        ptr: *mut u8,
        old_size: usize,
        align: usize,
        new_size: usize,
    ) -> *mut u8,
}

/// Sink the host exposes to the guest so the guest can emit its view tree
/// without ever allocating host-owned memory or knowing the host's types.
///
/// Every function is a plain `extern "C"` pointer: no Rust trait objects cross
/// the dylib boundary, which keeps the ABI stable across compiler versions.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct HostRenderContext {
    pub abi_version: u32,
    /// Size of this struct in bytes.
    ///
    /// The guest checks it before reading any field: the host may be older
    /// than the guest and the struct may have grown since.
    pub struct_size: u32,
    /// Bumped by the host every frame; the guest uses it for memoized views.
    pub state_generation: u64,
    /// Opaque host pointer, passed back verbatim.
    pub ctx: *mut c_void,

    /// Append a child under `parent` and return its id. Roots pass
    /// `parent = RawNodeId(NO_NODE)`.
    pub push_node: unsafe extern "C" fn(
        ctx: *mut c_void,
        parent: RawNodeId,
        desc: *const RawNodeDesc,
    ) -> RawNodeId,
    /// Attach UTF-8 text (copied by the host) to a text node.
    pub set_text:
        unsafe extern "C" fn(ctx: *mut c_void, node: RawNodeId, ptr: *const u8, len: usize),
    /// Bind a strongly typed action to a node's activation.
    pub bind_action: unsafe extern "C" fn(ctx: *mut c_void, node: RawNodeId, action: RawAction),
    /// Declare the build finished and hand over the root.
    pub finish: unsafe extern "C" fn(ctx: *mut c_void, root: RawNodeId),
}

impl HostRenderContext {
    /// Safe wrapper: append a node under `parent`.
    ///
    /// # Safety
    /// `desc` must point to a valid `RawNodeDesc` for the duration of the call.
    #[inline]
    pub unsafe fn push(&self, parent: RawNodeId, desc: *const RawNodeDesc) -> RawNodeId {
        // SAFETY: forwarded to the host-provided function, which validates its
        // own `ctx`; `desc` is the caller's responsibility per the signature.
        unsafe { (self.push_node)(self.ctx, parent, desc) }
    }

    /// Safe wrapper: attach UTF-8 text to a node.
    ///
    /// # Safety
    /// `ptr`/`len` must describe an initialised UTF-8 byte slice.
    #[inline]
    pub unsafe fn text(&self, node: RawNodeId, ptr: *const u8, len: usize) {
        // SAFETY: see above.
        unsafe { (self.set_text)(self.ctx, node, ptr, len) }
    }

    /// Safe wrapper: bind an action to a node.
    #[inline]
    pub fn bind(&self, node: RawNodeId, action: RawAction) {
        // SAFETY: the host validates `ctx`; the action is a POD value.
        unsafe { (self.bind_action)(self.ctx, node, action) }
    }

    /// Safe wrapper: finish the build pass.
    #[inline]
    pub fn done(&self, root: RawNodeId) {
        // SAFETY: the host validates `ctx`.
        unsafe { (self.finish)(self.ctx, root) }
    }

    /// True when the host installed real callbacks *and* the context is one
    /// this guest understands.
    ///
    /// The size check matters: the host may be older than the guest, and the
    /// context struct may have grown since the guest was built. Reading a
    /// field past the end of the host's struct would be undefined behaviour.
    #[inline]
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.ctx.is_null()
            && self.abi_version == ABI_VERSION
            && self.struct_size as usize >= core::mem::size_of::<Self>()
    }
}

impl Default for HostRenderContext {
    fn default() -> Self {
        Self {
            abi_version: ABI_VERSION,
            struct_size: core::mem::size_of::<Self>() as u32,
            state_generation: 0,
            ctx: core::ptr::null_mut(),
            push_node: no_push,
            set_text: no_set_text,
            bind_action: no_bind,
            finish: no_finish,
        }
    }
}

unsafe extern "C" fn no_push(
    _ctx: *mut c_void,
    _parent: RawNodeId,
    _desc: *const RawNodeDesc,
) -> RawNodeId {
    RawNodeId(crate::desc::NO_NODE)
}
unsafe extern "C" fn no_set_text(
    _ctx: *mut c_void,
    _node: RawNodeId,
    _ptr: *const u8,
    _len: usize,
) {
}
unsafe extern "C" fn no_bind(_ctx: *mut c_void, _node: RawNodeId, _action: RawAction) {}
unsafe extern "C" fn no_finish(_ctx: *mut c_void, _root: RawNodeId) {}

/// Opaque guest state handle.
pub type GuestState = *mut c_void;

/// The guest's exported vtable. Loaded by symbol name [`GUEST_ENTRY_SYMBOL`].
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GuestPluginVTable {
    pub abi_version: u32,
    /// Size of this struct in bytes.
    ///
    /// The host reads it *before* copying the vtable, so that a guest built
    /// against an older ABI — whose struct is smaller — is refused instead of
    /// being read past its end.
    pub struct_size: u32,
    /// Compiled-in fingerprint of the guest's `State` memory layout. Equal
    /// hashes enable the zero-copy hot-reload fast path (`docs` §8.1).
    pub state_schema_hash: u64,

    pub on_load: unsafe extern "C" fn(host_alloc: *const HostAllocatorVTable),
    pub on_unload: unsafe extern "C" fn(),

    /// Create a fresh state. The host owns the returned pointer and frees it by
    /// calling [`GuestPluginVTable::drop_state`] through the *current* guest.
    pub init_state: unsafe extern "C" fn() -> GuestState,
    pub drop_state: unsafe extern "C" fn(state: GuestState),

    /// Fast path: adopt an existing state pointer verbatim (same schema hash).
    pub adopt_state: unsafe extern "C" fn(state: GuestState),
    /// Safe path: export the state to a self-describing binary buffer so the
    /// next guest generation can migrate it field by field.
    ///
    /// The guest writes at most `out.capacity` bytes and reports how many it
    /// wrote in `out.len`. When the buffer is too small the guest writes
    /// nothing and sets `out.len` to the size it needs, so the host can
    /// allocate a bigger one and call again.
    pub migrate_export: unsafe extern "C" fn(state: GuestState, out: *mut ByteBuffer),
    /// Safe path: rebuild a state from [`Self::migrate_export`] output, applying
    /// `serde` defaults for fields the old schema did not have.
    ///
    /// Returns null when the buffer cannot be decoded; the host then falls
    /// back to a fresh state instead of silently losing the data.
    pub migrate_import: unsafe extern "C" fn(buf_ptr: *const u8, buf_len: usize) -> GuestState,

    pub reduce: unsafe extern "C" fn(state: GuestState, action: *const RawAction),
    pub build_view: unsafe extern "C" fn(state: GuestState, ctx: *mut HostRenderContext),
    /// Serialise the whole state for time-travel snapshots.
    pub snapshot: unsafe extern "C" fn(state: GuestState, out: *mut ByteBuffer),
}

// SAFETY: the vtable is a plain aggregate of function pointers and integers.
unsafe impl Send for GuestPluginVTable {}
unsafe impl Sync for GuestPluginVTable {}

impl core::fmt::Debug for GuestPluginVTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GuestPluginVTable")
            .field("abi_version", &self.abi_version)
            .field(
                "state_schema_hash",
                &format_args!("{:#018x}", self.state_schema_hash),
            )
            .finish_non_exhaustive()
    }
}

/// Name of the single exported symbol every guest `cdylib` must provide.
pub const GUEST_ENTRY_SYMBOL: &[u8] = b"unbound_guest_entry\0";

/// Signature of [`GUEST_ENTRY_SYMBOL`].
pub type GuestEntryFn = unsafe extern "C" fn() -> *const GuestPluginVTable;

/// Compatibility check performed before any vtable field is called.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AbiCompat {
    /// Same major version: safe to call.
    Ok,
    /// Host is older than the guest build.
    HostTooOld { host: u32, guest: u32 },
    /// Guest is older than the host build.
    GuestTooOld { host: u32, guest: u32 },
}

#[must_use]
pub fn check_abi(guest_version: u32) -> AbiCompat {
    if guest_version == ABI_VERSION {
        AbiCompat::Ok
    } else if guest_version > ABI_VERSION {
        AbiCompat::HostTooOld {
            host: ABI_VERSION,
            guest: guest_version,
        }
    } else {
        AbiCompat::GuestTooOld {
            host: ABI_VERSION,
            guest: guest_version,
        }
    }
}

/// What the host should do with the guest's state after a reload.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StateHandoff {
    /// Schema hashes match: hand the raw pointer over, zero copies.
    ZeroCopy,
    /// Schema changed: round-trip through `migrate_export`/`migrate_import`.
    Migrate,
    /// No previous state (first load).
    Fresh,
}

/// Decide the handoff mode purely from the two schema hashes.
#[must_use]
pub fn classify_handoff(old_hash: Option<u64>, new_hash: u64) -> StateHandoff {
    match old_hash {
        None => StateHandoff::Fresh,
        Some(h) if h == new_hash => StateHandoff::ZeroCopy,
        Some(_) => StateHandoff::Migrate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_checks() {
        assert_eq!(check_abi(ABI_VERSION), AbiCompat::Ok);
        assert!(matches!(
            check_abi(ABI_VERSION + 1),
            AbiCompat::HostTooOld { .. }
        ));
        assert!(matches!(
            check_abi(ABI_VERSION - 1),
            AbiCompat::GuestTooOld { .. }
        ));
    }

    #[test]
    fn handoff_classification() {
        assert_eq!(classify_handoff(None, 42), StateHandoff::Fresh);
        assert_eq!(classify_handoff(Some(42), 42), StateHandoff::ZeroCopy);
        assert_eq!(classify_handoff(Some(41), 42), StateHandoff::Migrate);
    }

    #[test]
    fn default_context_is_inert_but_callable() {
        let ctx = HostRenderContext::default();
        assert_eq!(ctx.abi_version, ABI_VERSION);
        // # Safety: the default functions are no-ops by construction.
        assert!(!ctx.is_connected());
        unsafe {
            assert!(ctx.push(RawNodeId::ROOT, core::ptr::null()).is_none());
            ctx.text(RawNodeId::ROOT, core::ptr::null(), 0);
        }
        ctx.bind(RawNodeId::ROOT, RawAction::NONE);
        ctx.done(RawNodeId::ROOT);
    }

    #[test]
    fn entry_symbol_is_nul_terminated() {
        assert_eq!(*GUEST_ENTRY_SYMBOL.last().unwrap(), 0);
        assert_eq!(
            &GUEST_ENTRY_SYMBOL[..GUEST_ENTRY_SYMBOL.len() - 1],
            b"unbound_guest_entry"
        );
    }
}

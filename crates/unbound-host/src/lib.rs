//! 宿主侧热重载（`docs` §8.1）。
//!
//! The host keeps the window, the Vulkan device and the persistent pools alive
//! while the guest — the whole business layer — is unloaded, rebuilt and
//! remounted. Three things make that safe:
//!
//! 1. **A `#[repr(C)]` vtable with a version check** ([`unbound_abi::plugin`]):
//!    a mismatched guest is refused, never called.
//! 2. **A single heap.** The host hands the guest an allocator vtable and the
//!    guest installs a `#[global_allocator]` forwarding to it, so a `Vec<u8>`
//!    built by the guest can be freed by the host (the classic Windows
//!    cross-CRT corruption bug).
//! 3. **A state handoff decision made from the schema hash**, not from a guess:
//!    equal hashes adopt the old pointer verbatim, different hashes round-trip
//!    through a self-describing buffer.
//!
//! On Windows a loaded `.dll` is locked, so a rebuild cannot overwrite it.
//! [`GuestHost::reload`] therefore shadow-copies the fresh artefact to a unique
//! temp path first, which is the doc's "Shadow Copy 自动重挂载".

#![forbid(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use std::alloc::GlobalAlloc as _;
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use rustc_hash::FxHashMap;
use unbound_abi::action::RawAction;
use unbound_abi::desc::{RawNodeDesc, RawNodeId};
use unbound_abi::plugin::{
    check_abi, classify_handoff, AbiCompat, ByteBuffer, GuestEntryFn, GuestPluginVTable,
    GuestState, HostAllocatorVTable, HostRenderContext, ABI_VERSION, GUEST_ENTRY_SYMBOL,
};

pub use unbound_abi::plugin::{StateHandoff, ABI_VERSION as GUEST_ABI_VERSION};

/// Anything that can go wrong while mounting a guest.
#[derive(Debug)]
pub enum HostError {
    /// The dynamic library could not be opened.
    Load(libloading::Error),
    /// The library does not export [`GUEST_ENTRY_SYMBOL`].
    MissingEntry,
    /// The guest was built against a different ABI.
    Abi(AbiCompat),
    /// Filesystem failure while shadow-copying.
    Io(std::io::Error),
    /// No guest is mounted.
    NoGuest,
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Load(error) => write!(f, "failed to load guest library: {error}"),
            Self::MissingEntry => write!(f, "guest does not export unbound_guest_entry"),
            Self::Abi(compat) => write!(f, "guest ABI mismatch: {compat:?}"),
            Self::Io(error) => write!(f, "shadow copy failed: {error}"),
            Self::NoGuest => write!(f, "no guest is mounted"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<libloading::Error> for HostError {
    fn from(error: libloading::Error) -> Self {
        Self::Load(error)
    }
}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// One node the guest asked the host to create.
#[derive(Clone, Debug)]
pub struct HostNode {
    pub id: RawNodeId,
    pub parent: RawNodeId,
    pub desc: RawNodeDesc,
    pub text: String,
    pub action: Option<RawAction>,
    pub children: Vec<RawNodeId>,
}

/// Flat node arena filled by the guest's `build_view` pass.
#[derive(Default, Debug)]
pub struct NodeArena {
    nodes: Vec<HostNode>,
    index: FxHashMap<u32, usize>,
    roots: Vec<RawNodeId>,
    finished_root: Option<RawNodeId>,
}

impl NodeArena {
    /// Drop the previous frame but keep the allocation.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.index.clear();
        self.roots.clear();
        self.finished_root = None;
    }

    /// Number of nodes built this frame.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Nodes in build order.
    #[must_use]
    pub fn nodes(&self) -> &[HostNode] {
        &self.nodes
    }

    /// Root nodes (those pushed with a null parent).
    #[must_use]
    pub fn roots(&self) -> &[RawNodeId] {
        &self.roots
    }

    /// Root handed over by `finish`.
    #[must_use]
    pub fn finished_root(&self) -> Option<RawNodeId> {
        self.finished_root
    }

    #[must_use]
    pub fn get(&self, id: RawNodeId) -> Option<&HostNode> {
        self.index.get(&id.0).map(|slot| &self.nodes[*slot])
    }

    fn push(&mut self, parent: RawNodeId, desc: RawNodeDesc) -> RawNodeId {
        // Ids start at 1: `RawNodeId::ROOT` (0) is the "virtual" root that a
        // guest may use as a parent without owning a node of its own.
        let id = RawNodeId(self.nodes.len() as u32 + 1);
        if parent.is_none() || parent == RawNodeId::ROOT {
            // `ROOT` is a valid parent: the node becomes a root of the tree.
            // It used to fall through to the `else` branch, where the lookup
            // of id 0 always failed — the node was then in neither the root
            // list nor any child list, and silently vanished from the tree.
            self.roots.push(id);
        } else if let Some(slot) = self.index.get(&parent.0).copied() {
            self.nodes[slot].children.push(id);
        }
        self.index.insert(id.0, self.nodes.len());
        self.nodes.push(HostNode {
            id,
            parent,
            desc,
            text: String::new(),
            action: None,
            children: Vec::new(),
        });
        id
    }

    fn set_text(&mut self, id: RawNodeId, text: &str) {
        if let Some(slot) = self.index.get(&id.0).copied() {
            self.nodes[slot].text.clear();
            self.nodes[slot].text.push_str(text);
        }
    }

    fn bind(&mut self, id: RawNodeId, action: RawAction) {
        if let Some(slot) = self.index.get(&id.0).copied() {
            self.nodes[slot].action = Some(action);
        }
    }

    fn finish(&mut self, root: RawNodeId) {
        self.finished_root = Some(root);
    }
}

/// The host's view of one mounted guest.
pub struct GuestHost {
    library: Option<Library>,
    vtable: Option<GuestPluginVTable>,
    state: GuestState,
    schema_hash: Option<u64>,
    /// The vtable must outlive the guest: a statically linked guest keeps the
    /// pointer in a process-wide static, so the host never hands out a pointer
    /// into its own stack frame.
    ///
    /// The leak is deliberate. The guest caches this pointer for as long as it
    /// is mounted, and a statically linked guest shares one static across
    /// every host instance in the process — so the address has to stay valid
    /// for the whole process, not just for one mount. Storing the vtable in a
    /// `Box` inside the host does not work: the box is freed with the host,
    /// and the next host's allocation then goes through a dangling pointer.
    alloc: &'static HostAllocatorVTable,
    arena: NodeArena,
    generation: u64,
    loads: u32,
    shadow_counter: u32,
    /// The shadow copies this host created, removed when the guest is unloaded.
    ///
    /// Without this a long dev session with hundreds of reloads leaves
    /// hundreds of DLL copies in the temp directory.
    shadows: Vec<PathBuf>,
    /// The shadow copy the currently loaded library came from.
    ///
    /// Only this one may be deleted while the library is loaded: the file is
    /// locked for as long as the library is mapped.
    library_shadow: Option<PathBuf>,
}

impl core::fmt::Debug for GuestHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GuestHost")
            .field("mounted", &self.vtable.is_some())
            .field("schema_hash", &self.schema_hash)
            .field("generation", &self.generation)
            .field("loads", &self.loads)
            .field("nodes", &self.arena.len())
            .finish_non_exhaustive()
    }
}

impl Default for GuestHost {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestHost {
    /// Create an empty host with the default (system) allocator vtable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            library: None,
            vtable: None,
            state: core::ptr::null_mut(),
            schema_hash: None,
            alloc: &SYSTEM_ALLOCATOR,
            arena: NodeArena::default(),
            generation: 0,
            loads: 0,
            shadow_counter: 0,
            shadows: Vec::new(),
            library_shadow: None,
        }
    }

    /// Create a host whose guest allocations are served by `alloc`.
    ///
    /// The vtable is leaked on purpose — see the field's documentation: the
    /// guest keeps the pointer in a process-wide static.
    #[must_use]
    pub fn with_allocator(alloc: HostAllocatorVTable) -> Self {
        let mut host = Self::new();
        host.alloc = Box::leak(Box::new(alloc));
        host
    }

    /// Mount a guest from a dynamic library, shadow-copying it first.
    ///
    /// # Errors
    /// Returns [`HostError`] when the copy, the load, the ABI check or the
    /// guest's own `on_load` fails.
    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<(), HostError> {
        let shadow = self.shadow_copy(path.as_ref())?;
        // SAFETY: the path is a valid dynamic library produced by cargo; the
        // library is kept alive for as long as the vtable is used.
        let library = unsafe { Library::new(&shadow)? };
        let vtable = unsafe { read_vtable(&library)? };
        self.mount(library, vtable)?;
        self.library_shadow = Some(shadow);
        Ok(())
    }

    /// Mount a vtable that is already in memory (tests, statically linked apps).
    ///
    /// # Errors
    /// Returns [`HostError::Abi`] when the version does not match.
    pub fn mount_vtable(&mut self, vtable: GuestPluginVTable) -> Result<(), HostError> {
        self.mount_linked(vtable)
    }

    fn mount(&mut self, library: Library, vtable: GuestPluginVTable) -> Result<(), HostError> {
        self.mount_linked(vtable)?;
        self.library = Some(library);
        Ok(())
    }

    fn mount_linked(&mut self, vtable: GuestPluginVTable) -> Result<(), HostError> {
        let compat = check_abi(vtable.abi_version);
        if compat != AbiCompat::Ok {
            return Err(HostError::Abi(compat));
        }
        self.unmount();
        unsafe {
            // SAFETY: the ABI version matched, so the signature is ours.
            (vtable.on_load)(self.alloc);
            self.state = (vtable.init_state)();
        }
        self.schema_hash = Some(vtable.state_schema_hash);
        self.vtable = Some(vtable);
        self.loads += 1;
        Ok(())
    }

    /// Hot-reload from a freshly built artefact.
    ///
    /// The old guest stays mounted until the new one has been verified, then the
    /// state is handed over according to [`classify_handoff`].
    ///
    /// # Errors
    /// Returns [`HostError`] for the same reasons as [`Self::load`]; on error the
    /// old guest is left untouched.
    pub fn reload(&mut self, path: impl AsRef<Path>) -> Result<StateHandoff, HostError> {
        let shadow = self.shadow_copy(path.as_ref())?;
        // SAFETY: see `load`.
        let library = unsafe { Library::new(&shadow)? };
        let fresh = unsafe { read_vtable(&library)? };
        let handoff = self.reload_vtable(fresh)?;
        // The old library is unloaded by `reload_vtable`, so the old shadow
        // copy is unlocked and can go.
        self.release_library_shadow();
        self.library = Some(library);
        self.library_shadow = Some(shadow);
        Ok(handoff)
    }

    /// Perform the handoff against an already-read vtable.
    ///
    /// Split out of [`Self::reload`] so a statically linked guest (and the tests)
    /// can exercise the same code path without a dynamic library.
    ///
    /// # Errors
    /// Returns [`HostError::Abi`] when the fresh guest was built against another
    /// ABI version; the old guest stays mounted in that case.
    pub fn reload_vtable(&mut self, fresh: GuestPluginVTable) -> Result<StateHandoff, HostError> {
        let compat = check_abi(fresh.abi_version);
        if compat != AbiCompat::Ok {
            return Err(HostError::Abi(compat));
        }
        let handoff = classify_handoff(self.schema_hash, fresh.state_schema_hash);

        let old = self.vtable;
        let old_state = core::mem::replace(&mut self.state, core::ptr::null_mut());
        unsafe {
            // The new guest's allocator has to be installed *before* it
            // allocates anything: `init_state` and `migrate_import` both
            // allocate through the guest's global allocator, which forwards
            // to the host's vtable. Without the forwarding table the guest
            // would fall back to a different heap and the state would later
            // be freed through the host's allocator — a cross-allocator free.
            (fresh.on_load)(self.alloc);
            // SAFETY: `old` and `old_state` belong together and the old library
            // is still loaded at this point.
            match (handoff, old) {
                (StateHandoff::ZeroCopy, Some(_old)) => {
                    // Same layout: hand the pointer straight over. The old
                    // `drop_state` must NOT run — the new guest owns it now —
                    // but the host keeps using the same pointer.
                    (fresh.adopt_state)(old_state);
                    self.state = old_state;
                }
                (StateHandoff::Migrate, Some(old)) => {
                    // A migration exports through the *migration* hook, not
                    // through `snapshot`: the guest may reshape the state on
                    // the way out to match the new schema.
                    let buffer = export_state(old_state, old.migrate_export);
                    (old.drop_state)(old_state);
                    // The old guest is done: let it release whatever it
                    // holds before its library is unloaded.
                    (old.on_unload)();
                    self.state = (fresh.migrate_import)(buffer.as_ptr(), buffer.len());
                    if self.state.is_null() {
                        // The guest could not decode the buffer. Falling back
                        // to a fresh state is better than handing the host a
                        // null pointer it would dereference.
                        self.state = (fresh.init_state)();
                    }
                }
                _ => {
                    if let Some(old) = old {
                        (old.drop_state)(old_state);
                        (old.on_unload)();
                    }
                    self.state = (fresh.init_state)();
                }
            }
        }
        // The old artefact is only released once nothing can call into it.
        self.library = None;
        self.vtable = Some(fresh);
        self.schema_hash = Some(fresh.state_schema_hash);
        self.loads += 1;
        self.arena.clear();
        Ok(handoff)
    }

    /// Unmount the guest, dropping its state through its own vtable.
    pub fn unmount(&mut self) {
        if let Some(vtable) = self.vtable.take() {
            unsafe {
                // SAFETY: the vtable is live and `state` was produced by it.
                if !self.state.is_null() {
                    (vtable.drop_state)(self.state);
                }
                (vtable.on_unload)();
            }
            self.state = core::ptr::null_mut();
        }
        self.library = None;
        self.schema_hash = None;
        self.remove_shadows();
    }

    /// Delete the shadow copies this host created.
    ///
    /// The library is unloaded by now, so the files are no longer locked.
    fn remove_shadows(&mut self) {
        for path in self.shadows.drain(..) {
            let _ = std::fs::remove_file(path);
        }
        self.library_shadow = None;
    }

    /// Delete the shadow copy the unloaded library came from.
    fn release_library_shadow(&mut self) {
        if let Some(path) = self.library_shadow.take() {
            let _ = std::fs::remove_file(&path);
            self.shadows.retain(|p| p != &path);
        }
    }

    /// True when a guest is mounted.
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        self.vtable.is_some()
    }

    /// Schema hash of the mounted guest.
    #[must_use]
    pub fn schema_hash(&self) -> Option<u64> {
        self.schema_hash
    }

    /// Number of times a guest was mounted.
    #[must_use]
    pub fn loads(&self) -> u32 {
        self.loads
    }

    /// Nodes built by the last [`Self::build`].
    #[must_use]
    pub fn arena(&self) -> &NodeArena {
        &self.arena
    }

    /// Dispatch one action to the guest.
    ///
    /// # Errors
    /// Returns [`HostError::NoGuest`] when nothing is mounted.
    pub fn reduce(&mut self, action: &RawAction) -> Result<(), HostError> {
        let Some(vtable) = self.vtable else {
            return Err(HostError::NoGuest);
        };
        unsafe {
            // SAFETY: the vtable is live and the state belongs to it.
            (vtable.reduce)(self.state, action);
        }
        Ok(())
    }

    /// Run the guest's view build into a fresh arena.
    ///
    /// # Errors
    /// Returns [`HostError::NoGuest`] when nothing is mounted.
    pub fn build(&mut self) -> Result<(), HostError> {
        let Some(vtable) = self.vtable else {
            return Err(HostError::NoGuest);
        };
        self.arena.clear();
        self.generation += 1;
        let mut context = HostRenderContext {
            abi_version: ABI_VERSION,
            struct_size: core::mem::size_of::<HostRenderContext>() as u32,
            state_generation: self.generation,
            ctx: core::ptr::addr_of_mut!(self.arena).cast::<c_void>(),
            push_node: host_push_node,
            set_text: host_set_text,
            bind_action: host_bind_action,
            finish: host_finish,
        };
        unsafe {
            // SAFETY: the callbacks only touch the arena pointer we just handed
            // over, and the borrow of `self.arena` does not escape this block.
            (vtable.build_view)(self.state, &mut context);
        }
        Ok(())
    }

    /// Ask the guest for a binary state snapshot.
    ///
    /// # Errors
    /// Returns [`HostError::NoGuest`] when nothing is mounted.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, HostError> {
        let Some(vtable) = self.vtable else {
            return Err(HostError::NoGuest);
        };
        Ok(export_state(self.state, vtable.snapshot))
    }

    /// Copy `path` to a unique temp file so the original can be rebuilt.
    ///
    /// The copy is remembered so it can be deleted when the guest is unloaded:
    /// without that, a long dev session with hundreds of reloads leaves
    /// hundreds of DLL copies in the temp directory.
    fn shadow_copy(&mut self, path: &Path) -> Result<PathBuf, HostError> {
        if !path.exists() {
            // Nothing to copy (e.g. a test fixture); let the loader report it.
            return Ok(path.to_path_buf());
        }
        self.shadow_counter += 1;
        let file_name = path.file_name().map_or_else(
            || "guest.dll".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let shadow = std::env::temp_dir().join(format!(
            "unbound-shadow-{}-{}-{file_name}",
            std::process::id(),
            self.shadow_counter
        ));
        std::fs::copy(path, &shadow)?;
        self.shadows.push(shadow.clone());
        Ok(shadow)
    }
}

impl Drop for GuestHost {
    fn drop(&mut self) {
        self.unmount();
    }
}

/// Read the guest vtable out of a freshly opened library.
///
/// The ABI version and the struct size are read *first*, from the head of the
/// struct, and the whole vtable is only copied once both have been checked.
/// Copying it first would read past the end of a guest built against an older
/// ABI — whose vtable is smaller — before the host got a chance to refuse it.
///
/// # Safety
/// `library` must stay loaded for as long as the returned vtable is used, and it
/// must export [`GUEST_ENTRY_SYMBOL`] with the documented signature.
unsafe fn read_vtable(library: &Library) -> Result<GuestPluginVTable, HostError> {
    // The symbol name excludes the trailing NUL that the ABI constant carries.
    let name = &GUEST_ENTRY_SYMBOL[..GUEST_ENTRY_SYMBOL.len() - 1];
    let entry: Symbol<'_, GuestEntryFn> =
        unsafe { library.get(name) }.map_err(|_| HostError::MissingEntry)?;
    let pointer = unsafe { entry() };
    if pointer.is_null() {
        return Err(HostError::MissingEntry);
    }
    // The first two words are the version and the size. Both are present in
    // every ABI version, so reading them can never go past the guest's
    // struct.
    let header = unsafe { *(pointer.cast::<VTableHeader>()) };
    let compat = check_abi(header.abi_version);
    if compat != AbiCompat::Ok {
        return Err(HostError::Abi(compat));
    }
    let expected = core::mem::size_of::<GuestPluginVTable>();
    if (header.struct_size as usize) < expected {
        return Err(HostError::Abi(AbiCompat::GuestTooOld {
            host: ABI_VERSION,
            guest: header.abi_version,
        }));
    }
    // A guest whose table is *larger* than ours is accepted on purpose: the
    // extra members sit at the end, so copying only the fields this host
    // knows about is still correct — that is the forward-compatible case.
    // A guest whose table is smaller is refused: the copy would read past its
    // end. Inserting a member in the *middle* shifts every later field, and
    // no size check can catch that — which is why `ABI_VERSION` has to be
    // bumped for every layout change.
    Ok(unsafe { *pointer })
}

/// The leading words of [`GuestPluginVTable`], read before the whole struct.
///
/// Keeping them in their own `#[repr(C)]` struct makes the offset of the
/// version and the size explicit, and guarantees that reading them cannot
/// touch anything past the guest's vtable.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct VTableHeader {
    abi_version: u32,
    struct_size: u32,
}

/// Export a guest's state into a host-owned byte buffer.
///
/// The guest writes into the buffer; it never allocates it. The buffer grows
/// by asking the guest how much room it needs, so the guest's allocator is
/// never involved and the host frees the memory with its own allocator.
///
/// `export` selects the guest callback to run, and the two callers need
/// different ones:
///
/// * [`GuestPluginVTable::snapshot`] for a plain time-travel snapshot;
/// * [`GuestPluginVTable::migrate_export`] when the bytes are handed over to
///   the next generation — a migration must go through the export hook so
///   the guest can reshape its state for the new schema.
///
/// The callback runs twice: once to probe the required size (no buffer is
/// given, the guest only reports the length it needs) and once to fill the
/// buffer the host allocated for it.
fn export_state(
    state: GuestState,
    export: unsafe extern "C" fn(GuestState, *mut ByteBuffer),
) -> Vec<u8> {
    let mut buffer: Vec<u8> = Vec::new();
    // The guest reports the size it needs when the buffer is too small.
    let needed: u32 = unsafe {
        let mut view = ByteBuffer {
            ptr: core::ptr::null_mut(),
            capacity: 0,
            len: 0,
        };
        export(state, &mut view);
        view.len
    };
    if needed == 0 {
        return buffer;
    }
    buffer.resize(needed as usize, 0);
    unsafe {
        let mut view = ByteBuffer {
            ptr: buffer.as_mut_ptr(),
            capacity: needed,
            len: 0,
        };
        export(state, &mut view);
        buffer.truncate(view.len as usize);
    }
    buffer
}

// ---- host → guest callbacks -------------------------------------------------

unsafe extern "C" fn host_push_node(
    ctx: *mut c_void,
    parent: RawNodeId,
    desc: *const RawNodeDesc,
) -> RawNodeId {
    if ctx.is_null() || desc.is_null() {
        return RawNodeId::ROOT;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    let desc = unsafe { *desc };
    arena.push(parent, desc)
}

unsafe extern "C" fn host_set_text(ctx: *mut c_void, node: RawNodeId, ptr: *const u8, len: usize) {
    if ctx.is_null() || (ptr.is_null() && len > 0) {
        return;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(ptr, len) }
    };
    if let Ok(text) = core::str::from_utf8(bytes) {
        arena.set_text(node, text);
    }
}

unsafe extern "C" fn host_bind_action(ctx: *mut c_void, node: RawNodeId, action: RawAction) {
    if ctx.is_null() {
        return;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    arena.bind(node, action);
}

unsafe extern "C" fn host_finish(ctx: *mut c_void, root: RawNodeId) {
    if ctx.is_null() {
        return;
    }
    let arena = unsafe { &mut *ctx.cast::<NodeArena>() };
    arena.finish(root);
}

// ---- default allocator ------------------------------------------------------

/// Allocator vtable backed by the system allocator.
///
/// Swapping in `mimalloc` is a one-line change: the guest only ever sees the
/// three function pointers.
#[must_use]
pub fn system_allocator() -> HostAllocatorVTable {
    SYSTEM_ALLOCATOR
}

/// The process heap, reached through `System` rather than `std::alloc::alloc`.
///
/// `std::alloc::alloc` routes through the *binary's* `#[global_allocator]`, which
/// in an in-process test is the guest's own forwarding allocator — an infinite
/// recursion. `System` always talks to the OS, which is exactly the single heap
/// the design wants.
const SYSTEM: std::alloc::System = std::alloc::System;

unsafe extern "C" fn sys_alloc(size: usize, align: usize) -> *mut u8 {
    let Ok(layout) = core::alloc::Layout::from_size_align(size.max(1), align.max(1)) else {
        return core::ptr::null_mut();
    };
    unsafe { SYSTEM.alloc(layout) }
}

unsafe extern "C" fn sys_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if ptr.is_null() {
        return;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(size.max(1), align.max(1)) else {
        return;
    };
    unsafe { SYSTEM.dealloc(ptr, layout) }
}

unsafe extern "C" fn sys_realloc(
    ptr: *mut u8,
    old_size: usize,
    align: usize,
    new_size: usize,
) -> *mut u8 {
    let Ok(layout) = core::alloc::Layout::from_size_align(old_size.max(1), align.max(1)) else {
        return core::ptr::null_mut();
    };
    if ptr.is_null() {
        return unsafe { sys_alloc(new_size, align) };
    }
    unsafe { SYSTEM.realloc(ptr, layout, new_size.max(1)) }
}

/// Default allocator vtable, promoted to `'static` so guests can cache it.
static SYSTEM_ALLOCATOR: HostAllocatorVTable = HostAllocatorVTable {
    alloc: sys_alloc,
    dealloc: sys_dealloc,
    realloc: sys_realloc,
};

#[cfg(test)]
mod tests;

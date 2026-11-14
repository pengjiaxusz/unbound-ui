//! 由进程内 guest 虚表驱动的 host 测试。
//!
//! 模拟 guest 是真实的 `GuestPluginVTable`：相同的函数指针签名、
//! 相同的所有权规则。这意味着状态移交路径的演练方式与真实 `cdylib`
//! 完全一致，无需在测试内部触发编译器往返构建。
//!

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::{RawNodeDesc, RawNodeId, NO_NODE};
use unbound_abi::plugin::{
    ByteBuffer, GuestPluginVTable, GuestState, HostAllocatorVTable, HostRenderContext, ABI_VERSION,
};

use crate::{system_allocator, GuestHost, HostError, StateHandoff};

static INITIALIZED: AtomicU32 = AtomicU32::new(0);
static ADOPTED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static EXPORTED: AtomicU32 = AtomicU32::new(0);
static SNAPSHOTTED: AtomicU32 = AtomicU32::new(0);
static IMPORTED: AtomicU32 = AtomicU32::new(0);
static ON_LOAD: AtomicU32 = AtomicU32::new(0);
static ON_UNLOAD: AtomicU32 = AtomicU32::new(0);

/// 计数器为进程全局，而 `cargo test` 在多线程中并行运行同一二进制内的测试：
/// 若无此锁，一个测试的 `reset_counters` 会在另一个测试重置与断言之间
/// 清空其计数器。
static COUNTER_LOCK: Mutex<()> = Mutex::new(());

/// 获取计数器锁并重置计数器。
fn reset_counters() -> MutexGuard<'static, ()> {
    let guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    INITIALIZED.store(0, Ordering::SeqCst);
    ADOPTED.store(0, Ordering::SeqCst);
    DROPPED.store(0, Ordering::SeqCst);
    EXPORTED.store(0, Ordering::SeqCst);
    SNAPSHOTTED.store(0, Ordering::SeqCst);
    IMPORTED.store(0, Ordering::SeqCst);
    ON_LOAD.store(0, Ordering::SeqCst);
    ON_UNLOAD.store(0, Ordering::SeqCst);
    guard
}

/// guest 业务状态。
struct TestState {
    count: i64,
    label: String,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            count: 0,
            label: "clicks".to_owned(),
        }
    }
}

unsafe extern "C" fn guest_on_load(alloc: *const HostAllocatorVTable) {
    assert!(!alloc.is_null(), "the host must hand over an allocator");
    ON_LOAD.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn guest_on_unload() {
    ON_UNLOAD.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn guest_init() -> GuestState {
    INITIALIZED.fetch_add(1, Ordering::SeqCst);
    Box::into_raw(Box::new(TestState::default())).cast::<c_void>()
}

unsafe extern "C" fn guest_drop(state: GuestState) {
    DROPPED.fetch_add(1, Ordering::SeqCst);
    drop(unsafe { Box::from_raw(state.cast::<TestState>()) });
}

unsafe extern "C" fn guest_adopt(_state: GuestState) {
    ADOPTED.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn guest_export(state: GuestState, out: *mut ByteBuffer) {
    let state = unsafe { &*state.cast::<TestState>() };
    let out = unsafe { &mut *out };
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&state.count.to_le_bytes());
    bytes.extend_from_slice(state.label.as_bytes());
    // host 会询问两次：首次探测大小（不写回数据），
    // 第二次填充缓冲区。只有第二次才是实际产出状态的导出，
    // 因此仅对第二次计数。
    if write_bytes(out, &bytes) {
        EXPORTED.fetch_add(1, Ordering::SeqCst);
    }
}

/// 以真实 guest 的方式将 `bytes` 写入 host 缓冲区。
///
/// 返回是否成功写入字节；`false` 表示缓冲区过小，仅报告了所需长度。
///
fn write_bytes(out: &mut ByteBuffer, bytes: &[u8]) -> bool {
    let needed = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    if out.capacity < needed || out.ptr.is_null() {
        out.len = needed;
        return false;
    }
    // SAFETY: host 保证在 `ptr` 处有 `capacity` 字节可写。
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out.ptr, bytes.len());
    }
    out.len = needed;
    true
}

unsafe extern "C" fn guest_import(ptr: *const u8, len: usize) -> GuestState {
    IMPORTED.fetch_add(1, Ordering::SeqCst);
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    // 格式错误的缓冲区绝不能 panic：跨 `extern "C"` 函数栈展开会直接中止进程。
    //
    let Some((count_bytes, label_bytes)) = bytes.split_at_checked(8) else {
        return core::ptr::null_mut();
    };
    let Ok(count_bytes) = <[u8; 8]>::try_from(count_bytes) else {
        return core::ptr::null_mut();
    };
    let Ok(label) = String::from_utf8(label_bytes.to_vec()) else {
        return core::ptr::null_mut();
    };
    Box::into_raw(Box::new(TestState {
        count: i64::from_le_bytes(count_bytes),
        label,
    }))
    .cast::<c_void>()
}

unsafe extern "C" fn guest_reduce(state: GuestState, action: *const RawAction) {
    let state = unsafe { &mut *state.cast::<TestState>() };
    let action = unsafe { &*action };
    match action.kind() {
        ActionKind::Press => state.count += 1,
        ActionKind::Undo => state.count -= 1,
        _ => {}
    }
}

unsafe extern "C" fn guest_build(state: GuestState, ctx: *mut HostRenderContext) {
    let state = unsafe { &*state.cast::<TestState>() };
    let ctx = unsafe { &*ctx };
    let root_desc = RawNodeDesc::text().size(800.0, 600.0);
    let root = unsafe { ctx.push(RawNodeId(NO_NODE), &root_desc) };
    let label_desc = RawNodeDesc::text().size(200.0, 40.0);
    let label = unsafe { ctx.push(root, &label_desc) };
    let text = format!("{}: {}", state.label, state.count);
    unsafe { ctx.text(label, text.as_ptr(), text.len()) };
    ctx.done(root);
}

unsafe extern "C" fn guest_snapshot(state: GuestState, out: *mut ByteBuffer) {
    let state = unsafe { &*state.cast::<TestState>() };
    let out = unsafe { &mut *out };
    if write_bytes(out, &state.count.to_le_bytes()) {
        SNAPSHOTTED.fetch_add(1, Ordering::SeqCst);
    }
}

fn vtable(schema_hash: u64) -> GuestPluginVTable {
    GuestPluginVTable {
        abi_version: ABI_VERSION,
        struct_size: core::mem::size_of::<GuestPluginVTable>() as u32,
        state_schema_hash: schema_hash,
        on_load: guest_on_load,
        on_unload: guest_on_unload,
        init_state: guest_init,
        drop_state: guest_drop,
        adopt_state: guest_adopt,
        migrate_export: guest_export,
        migrate_import: guest_import,
        reduce: guest_reduce,
        build_view: guest_build,
        snapshot: guest_snapshot,
    }
}

fn click() -> RawAction {
    RawAction {
        kind: ActionKind::Press as u32,
        ..RawAction::NONE
    }
}

#[test]
fn mounting_runs_the_guest_lifecycle() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(7)).unwrap();
    assert!(host.is_mounted());
    assert_eq!(host.schema_hash(), Some(7));
    assert_eq!(host.loads(), 1);
    assert_eq!(ON_LOAD.load(Ordering::SeqCst), 1);
    assert_eq!(INITIALIZED.load(Ordering::SeqCst), 1);
}

#[test]
fn reduce_then_build_produces_a_node_tree() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(7)).unwrap();
    host.reduce(&click()).unwrap();
    host.reduce(&click()).unwrap();
    host.build().unwrap();

    let arena = host.arena();
    assert_eq!(arena.len(), 2);
    assert_eq!(arena.roots().len(), 1);
    assert_eq!(arena.finished_root(), Some(arena.roots()[0]));
    assert_eq!(arena.nodes()[1].text, "clicks: 2");
    assert_eq!(arena.nodes()[0].children, vec![arena.nodes()[1].id]);
}

#[test]
fn building_twice_does_not_accumulate_nodes() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(7)).unwrap();
    host.build().unwrap();
    let first = host.arena().len();
    host.build().unwrap();
    assert_eq!(host.arena().len(), first);
}

#[test]
fn a_reload_with_the_same_schema_adopts_the_pointer() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(0xdead_beef)).unwrap();
    host.reduce(&click()).unwrap();

    let handoff = host.reload_vtable(vtable(0xdead_beef)).unwrap();
    assert_eq!(handoff, StateHandoff::ZeroCopy);
    assert_eq!(ADOPTED.load(Ordering::SeqCst), 1);
    assert_eq!(INITIALIZED.load(Ordering::SeqCst), 1, "no re-init");
    assert_eq!(DROPPED.load(Ordering::SeqCst), 0, "state was handed over");
    assert_eq!(host.loads(), 2);

    // 被接管的状态仍保留着计数器。
    host.build().unwrap();
    assert_eq!(host.arena().nodes()[1].text, "clicks: 1");
}

#[test]
fn a_reload_with_a_new_schema_migrates_the_state() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(1)).unwrap();
    host.reduce(&click()).unwrap();
    host.reduce(&click()).unwrap();
    host.reduce(&click()).unwrap();

    let handoff = host.reload_vtable(vtable(2)).unwrap();
    assert_eq!(handoff, StateHandoff::Migrate);
    assert_eq!(EXPORTED.load(Ordering::SeqCst), 1);
    assert_eq!(
        SNAPSHOTTED.load(Ordering::SeqCst),
        0,
        "a migration must go through migrate_export, not snapshot"
    );
    assert_eq!(IMPORTED.load(Ordering::SeqCst), 1);
    assert_eq!(DROPPED.load(Ordering::SeqCst), 1);
    assert_eq!(INITIALIZED.load(Ordering::SeqCst), 1, "imported, not fresh");

    host.build().unwrap();
    assert_eq!(
        host.arena().nodes()[1].text,
        "clicks: 3",
        "the migrated state must keep the counter"
    );
}

#[test]
fn an_abi_mismatch_is_refused_before_any_call() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    let mut broken = vtable(1);
    broken.abi_version = ABI_VERSION + 1;
    let error = host.mount_vtable(broken).unwrap_err();
    assert!(matches!(error, HostError::Abi(_)), "{error}");
    assert!(!host.is_mounted());
    assert_eq!(INITIALIZED.load(Ordering::SeqCst), 0);
}

#[test]
fn unmount_drops_the_state_through_the_guest() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(1)).unwrap();
    host.unmount();
    assert_eq!(DROPPED.load(Ordering::SeqCst), 1);
    assert!(!host.is_mounted());
    assert!(matches!(host.build(), Err(HostError::NoGuest)));
    assert!(matches!(host.reduce(&click()), Err(HostError::NoGuest)));
    assert!(matches!(host.snapshot(), Err(HostError::NoGuest)));
}

#[test]
fn dropping_the_host_unmounts_the_guest() {
    let _guard = reset_counters();
    {
        let mut host = GuestHost::new();
        host.mount_vtable(vtable(1)).unwrap();
    }
    assert_eq!(DROPPED.load(Ordering::SeqCst), 1);
}

#[test]
fn the_snapshot_round_trips_the_guest_state() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(1)).unwrap();
    host.reduce(&click()).unwrap();
    let bytes = host.snapshot().unwrap();
    assert_eq!(bytes.len(), 8);
    assert_eq!(i64::from_le_bytes(bytes[..8].try_into().unwrap()), 1);
}

#[test]
fn the_system_allocator_vtable_round_trips_a_block() {
    let alloc = system_allocator();
    unsafe {
        let ptr = (alloc.alloc)(64, 16);
        assert!(!ptr.is_null());
        ptr.write_bytes(0xAB, 64);
        assert_eq!(*ptr, 0xAB);
        let grown = (alloc.realloc)(ptr, 64, 16, 256);
        assert!(!grown.is_null());
        assert_eq!(*grown, 0xAB, "realloc must preserve the payload");
        grown.write_bytes(0xCD, 256);
        (alloc.dealloc)(grown, 256, 16);
    }
}

#[test]
fn a_null_allocator_realloc_behaves_like_alloc() {
    let alloc = system_allocator();
    unsafe {
        let ptr = (alloc.realloc)(core::ptr::null_mut(), 0, 8, 32);
        assert!(!ptr.is_null());
        (alloc.dealloc)(ptr, 32, 8);
    }
}

#[test]
fn reduce_accepts_a_full_action_without_copying_host_state() {
    let _guard = reset_counters();
    let mut host = GuestHost::new();
    host.mount_vtable(vtable(1)).unwrap();
    let decrement = RawAction {
        kind: ActionKind::Undo as u32,
        ..RawAction::NONE
    };
    host.reduce(&click()).unwrap();
    host.reduce(&decrement).unwrap();
    host.build().unwrap();
    assert_eq!(host.arena().nodes()[1].text, "clicks: 0");
}

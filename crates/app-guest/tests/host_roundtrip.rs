//! The guest, mounted through the real host code path.
//!
//! These are not unit tests of the guest in isolation: they drive it through
//! [`unbound_host::GuestHost`], so the allocator hijack, the render context and
//! the state handoff are all exercised end to end.

use app_guest::{vtable, State};
use unbound_abi::action::{ActionKind, RawAction};
use unbound_host::{GuestHost, StateHandoff};

fn press() -> RawAction {
    RawAction::new(ActionKind::Press, 0)
}

fn undo() -> RawAction {
    RawAction::new(ActionKind::Undo, 0)
}

fn mask_row(index: usize) -> RawAction {
    RawAction::new(ActionKind::Custom, 0).with_payload(0, index as u64)
}

#[test]
fn the_guest_builds_a_view_from_its_state() {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    host.build().unwrap();
    let arena = host.arena();
    assert_eq!(arena.len(), 2, "root + title");
    assert!(arena.nodes()[1].text.contains("Hot reload demo"));
}

#[test]
fn pressing_adds_rows_and_undo_removes_them() {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    host.reduce(&press()).unwrap();
    host.reduce(&press()).unwrap();
    host.build().unwrap();
    assert_eq!(host.arena().len(), 4, "root, title, two rows");
    assert!(host.arena().nodes()[3].text.contains("item 2"));

    host.reduce(&undo()).unwrap();
    host.build().unwrap();
    assert_eq!(host.arena().len(), 3);
}

#[test]
fn bound_actions_target_their_row() {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    host.reduce(&press()).unwrap();
    host.reduce(&mask_row(0)).unwrap();
    host.build().unwrap();
    let row = host.arena().nodes().last().unwrap();
    assert!(row.text.starts_with("[x]"), "{:?}", row.text);
    let action = row.action.expect("row must carry an action");
    assert_eq!(action.kind(), ActionKind::Custom);
}

#[test]
fn a_same_schema_reload_keeps_the_state_without_serialising() {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    host.reduce(&press()).unwrap();
    let handoff = host.reload_vtable(vtable()).unwrap();
    assert_eq!(handoff, StateHandoff::ZeroCopy);
    host.build().unwrap();
    assert_eq!(host.arena().len(), 3, "state survived the remount");
}

#[test]
fn a_migration_round_trip_preserves_every_row() {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    for _ in 0..3 {
        host.reduce(&press()).unwrap();
    }
    host.reduce(&mask_row(1)).unwrap();
    let mut fresh = vtable();
    // Pretend the business layer changed shape: force the migrate path.
    fresh.state_schema_hash ^= 0xFFFF;
    let handoff = host.reload_vtable(fresh).unwrap();
    assert_eq!(handoff, StateHandoff::Migrate);
    host.build().unwrap();
    let texts: Vec<&str> = host
        .arena()
        .nodes()
        .iter()
        .map(|node| node.text.as_str())
        .collect();
    assert_eq!(texts.len(), 5);
    assert!(texts[3].starts_with("[x]"), "{texts:?}");
}

#[test]
fn the_state_snapshot_is_postcard_and_round_trips() {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    host.reduce(&press()).unwrap();
    let bytes = host.snapshot().unwrap();
    let state: State = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(state.count, 1);
    assert_eq!(state.rows.len(), 1);
    assert_eq!(state.rows[0].label, "item 1");
}

#[test]
fn the_guest_allocates_through_the_host_heap() {
    // A guest-side `Vec` grown across the boundary must be freeable by the host.
    // If the allocator hijack were missing this test would corrupt the heap
    // instead of failing an assertion.
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).unwrap();
    for _ in 0..100 {
        host.reduce(&press()).unwrap();
    }
    let bytes = host.snapshot().unwrap();
    assert!(bytes.len() > 100);
    drop(bytes);
    host.build().unwrap();
    assert_eq!(host.arena().len(), 102);
}

#[test]
fn the_dynamic_library_exports_the_entry_point() {
    // Only runs when the cdylib has been built next to the test binary, which
    // `cargo test --workspace` guarantees for a workspace member.
    let Some(path) = locate_guest_dll() else {
        eprintln!("skipping: app_guest dll not built next to the test binary");
        return;
    };
    let mut host = GuestHost::new();
    host.load(&path).unwrap();
    host.reduce(&press()).unwrap();
    host.build().unwrap();
    assert_eq!(host.arena().len(), 3);
    assert_eq!(host.schema_hash(), Some(<State as unbound_abi::schema::StateSchema>::SCHEMA_HASH));
}

fn locate_guest_dll() -> Option<std::path::PathBuf> {
    const NAMES: [&str; 3] = ["app_guest.dll", "libapp_guest.so", "libapp_guest.dylib"];
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    // The workspace target directory, regardless of where the harness put the
    // test executable.
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(workspace) = manifest.parent().and_then(|p| p.parent()) {
        roots.push(workspace.join("target").join("debug"));
        roots.push(workspace.join("target").join("release"));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(std::path::Path::to_path_buf);
        for _ in 0..8 {
            let Some(current) = dir else { break };
            roots.push(current.clone());
            roots.push(current.join("deps"));
            dir = current.parent().map(std::path::Path::to_path_buf);
        }
    }
    for root in roots {
        for name in NAMES {
            let candidate = root.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

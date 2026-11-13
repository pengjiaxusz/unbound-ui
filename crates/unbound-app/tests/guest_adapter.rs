//! A guest-hosted application, driven through the real runtime.
//!
//! The guest builds a title and one clickable row per item; the adapter turns
//! each row into a button. Clicking the row must reduce the bound action in the
//! guest and repaint the row with its new state — the whole point of hosting a
//! UBMF guest as an ordinary [`Application`].

use app_guest::vtable;
use unbound_abi::action::{ActionKind, RawAction};
use unbound_app::{AppEvent, AppRuntime, DirtyFlags, GuestAppAdapter};
use unbound_host::GuestHost;
use unbound_math::Vec2;

/// The key the adapter assigns to the first row (`RawNodeId`, which starts at 1).
const FIRST_ROW: u64 = 3;

fn adapter() -> GuestAppAdapter {
    let mut host = GuestHost::new();
    host.mount_vtable(vtable()).expect("the guest mounts");
    // The demo starts empty; one press creates "item 1".
    host.reduce(&RawAction::new(ActionKind::Press, 0))
        .expect("the guest reduces a press");
    GuestAppAdapter::new(host)
}

fn row_text(runtime: &AppRuntime<GuestAppAdapter>) -> Vec<String> {
    runtime
        .app()
        .host()
        .arena()
        .nodes()
        .iter()
        .map(|node| node.text.clone())
        .collect()
}

#[test]
fn a_guest_view_renders_through_the_host_runtime() {
    let mut runtime = AppRuntime::new(adapter(), Vec2::new(800.0, 600.0));
    runtime.frame(0.0);
    let texts = row_text(&runtime);
    assert!(
        texts.iter().any(|text| text.contains("Hot reload demo")),
        "the title reached the host: {texts:?}"
    );
    assert!(
        texts.iter().any(|text| text.contains("[ ] item 1")),
        "the row reached the host: {texts:?}"
    );
    assert!(
        !runtime.scene().primitives.is_empty(),
        "the adapter emitted primitives"
    );
}

#[test]
fn clicking_a_guest_row_reduces_its_bound_action() {
    let mut runtime = AppRuntime::new(adapter(), Vec2::new(800.0, 600.0));
    runtime.frame(0.0);
    assert!(row_text(&runtime).iter().any(|text| text == "[ ] item 1"));
    assert!(
        runtime.app().action_for(FIRST_ROW).is_some(),
        "the adapter bound the row's action"
    );

    let dirty = runtime.dispatch(AppEvent::Click {
        node: FIRST_ROW,
        pos: Vec2::ZERO,
        count: 1,
    });
    assert_ne!(
        dirty,
        DirtyFlags::NONE,
        "a click invalidates the guest view"
    );

    runtime.frame(0.0);
    let after = row_text(&runtime);
    assert!(
        after.iter().any(|text| text == "[x] item 1"),
        "the guest's toggle repainted the row: {after:?}"
    );
    assert!(!after.iter().any(|text| text == "[ ] item 1"));
}

#[test]
fn clicking_an_unbound_node_leaves_the_guest_alone() {
    let mut runtime = AppRuntime::new(adapter(), Vec2::new(800.0, 600.0));
    runtime.frame(0.0);
    let dirty = runtime.dispatch(AppEvent::Click {
        node: 9_999,
        pos: Vec2::ZERO,
        count: 1,
    });
    assert_eq!(dirty, DirtyFlags::NONE);
    runtime.frame(0.0);
    assert!(row_text(&runtime).iter().any(|text| text == "[ ] item 1"));
}

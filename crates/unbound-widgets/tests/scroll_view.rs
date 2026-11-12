//! Scroll-view integration: picking, scrollbar dragging and clip/transform.
//!
//! The scroll view is the one widget whose output is split: its own
//! primitives (the scrollbars) are clipped by nothing, while every **child**
//! primitive must use the emitted clip and transform. These tests install the
//! real hit regions and assert both halves.

use unbound_headless::driver::TestApp;
use unbound_input::Router;
use unbound_math::{Rect, Vec2};
use unbound_widgets::scroll::{
    emit_scroll_view, update_virtual_list, ScrollState, ScrollViewDesc, ScrollbarAxis,
};
use unbound_widgets::ControlState;

fn desc() -> ScrollViewDesc {
    ScrollViewDesc::new(Rect::new(0.0, 0.0, 400.0, 300.0), Vec2::new(400.0, 3000.0))
}

fn tab() -> ControlState {
    ControlState::default()
}

#[test]
fn the_wheel_over_the_viewport_targets_the_scroll_view() {
    let desc = desc();
    let state = ScrollState::new();
    let output = emit_scroll_view(90, &desc, &state, tab());

    let mut router = Router::default();
    router.set_regions(vec![
        output.output.hit.clone().expect("viewport hit"),
        output.thumb.clone().expect("thumb hit"),
    ]);

    let hits = router.wheel(Vec2::new(-120.0, 0.0), Vec2::new(200.0, 150.0));
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|hit| hit.node == 90));
}

#[test]
fn the_scrollbar_thumb_is_pickable_and_draggable() {
    let desc = desc();
    let mut state = ScrollState::new();
    let thumb = emit_scroll_view(90, &desc, &state, tab())
        .thumb
        .expect("content overflows, so the thumb exists");

    let mut app = TestApp::new((800.0, 600.0));
    app.install(vec![thumb.clone()], unbound_a11y::Semantics::new(90));
    assert_eq!(app.click(thumb.bounds.center()).clicked(), Some(thumb.node));

    // A press on the thumb starts the drag; moving down scrolls the content.
    let start = thumb.bounds.center().y;
    state.begin_drag(ScrollbarAxis::Vertical, start);
    state.drag_to(&desc, start + 40.0);
    assert!(state.offset().y > 0.0);
    state.drag_to(&desc, start + 100_000.0);
    assert_eq!(state.offset().y, desc.max_offset().y);
    state.end_drag();
    assert!(state.dragging().is_none());
}

#[test]
fn the_clip_and_transform_stay_in_sync_with_the_offset() {
    let desc = desc();
    let mut state = ScrollState::new();
    state.jump_to(&desc, Vec2::new(0.0, 420.0));
    let output = emit_scroll_view(90, &desc, &state, tab());

    assert_eq!(output.clip.aabb_clip, [0.0, 0.0, 400.0, 300.0]);
    assert_eq!(output.transform.translation, [0.0, -420.0]);
    assert!(output
        .output
        .hit
        .as_ref()
        .is_some_and(|hit| hit.bounds.contains_rect(&desc.rect)));
}

#[test]
fn scrolling_a_virtual_list_reuses_slots_instead_of_rebinding() {
    let desc = desc();
    let mut state = ScrollState::new();
    let mut list = unbound_world::virtualize::VirtualList::new(28.0, 1.0);
    let first = update_virtual_list(&mut list, &desc, &state);
    let budget = first.slots;

    state.jump_to(&desc, Vec2::new(0.0, 14.0));
    let second = update_virtual_list(&mut list, &desc, &state);
    assert!(second.update.offset_only);
    assert_eq!(second.update.rebound, 0);
    assert_eq!(second.slots, budget);
}

#[test]
fn a_disabled_scroll_view_still_emits_but_is_not_pickable() {
    let desc = desc();
    let state = ScrollState::new();
    let disabled = ControlState {
        disabled: true,
        ..ControlState::default()
    };
    let output = emit_scroll_view(90, &desc, &state, disabled);
    assert!(output.output.hit.is_some(), "the viewport still clips");
    assert!(output.thumb.is_some());
}

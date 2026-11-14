//! 分段控件的方向键导航、自定义槽位宽度与禁用槽位测试。
//!

use unbound_math::Rect;
use unbound_widgets::segmented::{
    emit_segmented_segment, reduce_segmented_key, SegmentedDesc, SegmentedKey, SegmentedTokens,
};
use unbound_widgets::ControlState;

fn desc() -> SegmentedDesc {
    SegmentedDesc::new(
        Rect::new(0.0, 0.0, 300.0, 32.0),
        &["Light", "Dark", "System"],
    )
}

#[test]
fn item_widths_drive_the_cells() {
    let desc = desc().with_item_widths(vec![60.0, 90.0, 150.0]);
    assert_eq!(desc.cell_rect(0).origin.x, 0.0);
    assert_eq!(desc.cell_rect(0).size.x, 60.0);
    assert_eq!(desc.cell_rect(1).origin.x, 60.0);
    assert_eq!(desc.cell_rect(1).size.x, 90.0);
    assert_eq!(desc.cell_rect(2).origin.x, 150.0);
    assert_eq!(desc.cell_rect(2).size.x, 150.0);
}

#[test]
fn the_thumb_interpolates_over_variable_widths() {
    let desc = desc()
        .with_item_widths(vec![60.0, 90.0, 150.0])
        .with_thumb(1.5);
    let thumb = desc.thumb_rect();
    // 60 + 90 * 0.5 + 内边距 2。
    assert!((thumb.origin.x - 107.0).abs() < 0.01);
    // 90 + 60 * 0.5 - 2 * 内边距。
    assert!((thumb.size.x - 116.0).abs() < 0.01);
}

#[test]
fn arrow_keys_wrap_around_the_row() {
    let desc = desc();
    assert_eq!(reduce_segmented_key(&desc, 0, SegmentedKey::ArrowLeft), 2);
    assert_eq!(reduce_segmented_key(&desc, 2, SegmentedKey::ArrowRight), 0);
    assert_eq!(reduce_segmented_key(&desc, 0, SegmentedKey::ArrowRight), 1);
}

#[test]
fn arrow_keys_skip_disabled_slots() {
    // 槽位 1 已禁用。
    let desc = desc().with_disabled_mask(0b010);
    assert_eq!(reduce_segmented_key(&desc, 0, SegmentedKey::ArrowRight), 2);
    assert_eq!(reduce_segmented_key(&desc, 2, SegmentedKey::ArrowLeft), 0);
    assert_eq!(reduce_segmented_key(&desc, 2, SegmentedKey::ArrowRight), 0);
}

#[test]
fn all_disabled_slots_keep_the_selection() {
    let desc = desc().with_disabled_mask(0b111);
    assert_eq!(reduce_segmented_key(&desc, 1, SegmentedKey::ArrowRight), 1);
    assert_eq!(reduce_segmented_key(&desc, 1, SegmentedKey::ArrowLeft), 1);
}

#[test]
fn a_disabled_slot_leaves_the_hit_set() {
    let desc = desc().with_disabled_mask(0b010);
    let tokens = SegmentedTokens::standard();

    let disabled = emit_segmented_segment(1, &desc, 1, ControlState::default(), &[], tokens);
    assert!(disabled.hit.is_none());
    assert!(!disabled.focusable);
    assert!(disabled.a11y.expect("still reachable").disabled);

    let enabled = emit_segmented_segment(0, &desc, 0, ControlState::default(), &[], tokens);
    assert!(enabled.hit.is_some());
    assert!(enabled.focusable);
}

#[test]
fn with_disabled_round_trips_one_bit() {
    let desc = desc().with_disabled(1, true);
    assert!(desc.is_disabled(1));
    assert!(!desc.is_disabled(0));
    let desc = desc.with_disabled(1, false);
    assert!(!desc.is_disabled(1));
}

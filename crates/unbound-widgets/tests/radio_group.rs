//! Radio-group keyboard behaviour and the roving tab index.
//!
//! The group owns the selection; arrow keys cycle it and only the selected
//! option enters the tab order.

use unbound_math::Rect;
use unbound_widgets::radio::{
    emit_radio, reduce_radio_group, RadioDesc, RadioGroupDesc, RadioGroupKey, RadioTokens,
};
use unbound_widgets::ControlState;

fn items() -> Vec<RadioDesc> {
    ["Compact", "Comfortable", "Spacious"]
        .iter()
        .enumerate()
        .map(|(index, label)| {
            RadioDesc::new(Rect::new(0.0, index as f32 * 40.0, 220.0, 32.0), *label)
        })
        .collect()
}

fn group(selected: Option<usize>) -> RadioGroupDesc {
    let mut group = RadioGroupDesc::new(items());
    if let Some(selected) = selected {
        group.selected = Some(selected);
        for (index, item) in group.items.iter_mut().enumerate() {
            item.selected = index == selected;
        }
    }
    group
}

#[test]
fn the_group_adopts_the_preselected_item() {
    let mut items = items();
    items[1].selected = true;
    let group = RadioGroupDesc::new(items);
    assert_eq!(group.selected, Some(1));
    assert_eq!(group.focus_order(), vec![1]);
}

#[test]
fn an_unselected_group_puts_the_first_item_in_the_tab_order() {
    let group = group(None);
    assert_eq!(group.selected, None);
    assert_eq!(group.focus_order(), vec![0]);
}

#[test]
fn arrow_keys_cycle_the_selection_and_wrap() {
    let mut group = group(Some(0));

    let outcome = reduce_radio_group(&mut group, RadioGroupKey::Down);
    assert_eq!(outcome.selected, Some(1));
    assert!(outcome.changed);
    assert!(group.items[1].selected);
    assert!(!group.items[0].selected);

    reduce_radio_group(&mut group, RadioGroupKey::Right);
    assert_eq!(group.selected, Some(2));

    let outcome = reduce_radio_group(&mut group, RadioGroupKey::Right);
    assert_eq!(outcome.selected, Some(0), "wraps past the last option");

    reduce_radio_group(&mut group, RadioGroupKey::Up);
    assert_eq!(group.selected, Some(2), "wraps before the first option");

    reduce_radio_group(&mut group, RadioGroupKey::Left);
    assert_eq!(group.selected, Some(1));
}

#[test]
fn the_first_key_on_an_unselected_group_selects_the_first_option() {
    let mut group = group(None);
    let outcome = reduce_radio_group(&mut group, RadioGroupKey::Down);
    assert_eq!(outcome.selected, Some(0));
    assert!(outcome.changed);
    assert_eq!(group.focus_order(), vec![0]);
}

#[test]
fn only_the_selected_option_enters_the_roving_tab_order() {
    let mut group = group(Some(0));
    reduce_radio_group(&mut group, RadioGroupKey::Down);
    reduce_radio_group(&mut group, RadioGroupKey::Down);
    assert_eq!(group.selected, Some(2));
    assert_eq!(group.focus_order(), vec![2]);
    assert_eq!(group.focus_order().len(), 1, "the roving stop is unique");
}

#[test]
fn an_empty_group_ignores_keys() {
    let mut group = RadioGroupDesc::new(Vec::new());
    assert!(group.is_empty());
    assert!(group.focus_order().is_empty());
    let outcome = reduce_radio_group(&mut group, RadioGroupKey::Down);
    assert_eq!(
        outcome,
        unbound_widgets::radio::RadioGroupOutcome::default()
    );
}

#[test]
fn a_disabled_radio_paints_the_disabled_slots() {
    let tokens = RadioTokens::standard();
    let desc = items()[0].clone().with_selected(true);
    let output = emit_radio(
        1,
        &desc,
        ControlState {
            disabled: true,
            ..ControlState::default()
        },
        &[],
        tokens,
    );
    assert_eq!(
        output.primitives[0].fill_token_or_uv,
        tokens.circle_disabled.raw() as u32
    );
    assert_eq!(
        output.primitives[0].border_token_or_tex,
        tokens.border_disabled.raw() as u32
    );
    assert_eq!(
        output.primitives[1].fill_token_or_uv,
        tokens.dot_disabled.raw() as u32
    );
}

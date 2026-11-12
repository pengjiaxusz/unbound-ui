//! Menus grow nested rows and a scrollable body.
//!
//! The tests drive the highlight through `open_path` the way `ArrowRight`,
//! `ArrowLeft`, hover and type-ahead do, then check the clamped geometry and the
//! embedded [`ScrollState`] without a GPU.

use unbound_math::{Rect, Vec2};
use unbound_theme::Token;
use unbound_widgets::menu::{emit_menu, MenuDesc, MenuItem, MenuState};
use unbound_widgets::ControlState;

fn nested() -> MenuDesc {
    MenuDesc::new(
        Rect::new(0.0, 0.0, 200.0, 90.0),
        vec![
            MenuItem::action("File").with_submenu(vec![
                MenuItem::action("New"),
                MenuItem::action("Open"),
                MenuItem::action("Recent").with_submenu(vec![MenuItem::action("a.txt")]),
            ]),
            MenuItem::action("Edit"),
        ],
    )
}

#[test]
fn arrow_right_opens_the_highlighted_submenu_and_arrow_left_closes_it() {
    let desc = nested();
    let mut state = MenuState::new();
    state.highlighted = Some(0);

    assert!(state.expand(&desc), "File has a submenu");
    assert_eq!(state.open_path, vec![0]);
    assert_eq!(state.open_items(&desc).len(), 3, "the nested level");

    // A second ArrowRight opens the nested-nested level.
    state.highlighted = Some(2);
    assert!(state.expand(&desc));
    assert_eq!(state.open_path, vec![0, 2]);
    assert_eq!(state.open_items(&desc).len(), 1);

    // ArrowLeft walks back out one level at a time and re-highlights the parent.
    assert!(state.collapse());
    assert_eq!(state.open_path, vec![0]);
    assert_eq!(state.highlighted, Some(2));
    assert!(state.collapse());
    assert_eq!(state.open_path, Vec::<usize>::new());
    assert_eq!(state.highlighted, Some(0));
    assert!(!state.collapse(), "nothing is open at the root");
}

#[test]
fn hovering_a_submenu_row_expands_it_and_a_leaf_closes_it() {
    let desc = nested();
    let mut state = MenuState::new();

    state.hover(&desc, Vec2::new(10.0, 10.0));
    assert_eq!(state.open_path, vec![0], "hover opens like ArrowRight");

    state.hover(&desc, Vec2::new(10.0, 40.0));
    assert!(state.open_path.is_empty(), "a root leaf closes the submenu");
    assert_eq!(state.highlighted, Some(1));
}

#[test]
fn typeahead_selects_by_first_letter_and_skips_disabled_rows() {
    let desc = MenuDesc::new(
        Rect::new(0.0, 0.0, 200.0, 120.0),
        vec![
            MenuItem::action("File"),
            MenuItem::action("Edit"),
            MenuItem::action("View").disabled(),
            MenuItem::action("Export"),
        ],
    );
    let mut state = MenuState::new();

    assert_eq!(state.typeahead(&desc, 'e'), Some(1), "Edit comes first");
    assert_eq!(state.typeahead(&desc, 'E'), Some(3), "then Export, cycling");
    assert_eq!(state.typeahead(&desc, 'e'), Some(1), "wraps around");
    assert_eq!(
        state.typeahead(&desc, 'v'),
        Some(1),
        "View is disabled, so the highlight stays put"
    );
    assert_eq!(state.typeahead(&desc, 'f'), Some(0));
}

#[test]
fn a_menu_item_carries_an_icon_a_check_state_and_a_submenu() {
    let icon = unbound_widgets::image::IconDesc::new(
        Rect::new(0.0, 0.0, 16.0, 16.0),
        9,
        Token::TEXT_PRIMARY,
    );
    let item = MenuItem::action("Grid")
        .with_icon(icon.clone())
        .checked(true)
        .with_submenu(vec![MenuItem::action("Compact")]);

    assert_eq!(item.icon, Some(icon));
    assert_eq!(item.checked, Some(true));
    assert!(item.has_submenu());
    assert_eq!(item.submenu.as_ref().map(Vec::len), Some(1));
    assert_eq!(MenuItem::action("Plain").checked, None);
}

#[test]
fn a_tall_menu_clamps_to_the_viewport_and_embeds_a_scroll_state() {
    let items = (0..20)
        .map(|index| MenuItem::action(format!("Row {index}")))
        .collect::<Vec<_>>();
    let mut desc = MenuDesc::new(Rect::new(0.0, 0.0, 200.0, 600.0), items);
    desc.viewport = Some(Rect::new(0.0, 0.0, 800.0, 200.0));
    assert_eq!(desc.content_height(), 600.0);

    assert!(desc.clamp_height(), "20 rows do not fit in 200px");
    assert_eq!(desc.rect.height(), 200.0);

    let mut state = MenuState::new();
    assert!(state.ensure_scroll(&desc));
    assert!(state.scroll.is_some(), "the body owns a ScrollState");

    state.scroll_by(&desc, Vec2::new(0.0, 100.0));
    assert_eq!(state.scroll_offset().y, 100.0);

    let output = emit_menu(1, 2, &desc, &state, ControlState::default()).0;
    // The row primitives travel with the scroll offset.
    assert_eq!(output.primitives[2].local_pos[1], -100.0);
    // 2 chrome + 20 rows * 2 + border.
    assert_eq!(output.primitives.len(), 43);
}

#[test]
fn a_short_menu_needs_no_scroll_state() {
    let desc = nested();
    let mut state = MenuState::new();
    assert!(!state.ensure_scroll(&desc));
    assert!(state.scroll.is_none());
    assert_eq!(state.scroll_offset(), Vec2::ZERO);
}

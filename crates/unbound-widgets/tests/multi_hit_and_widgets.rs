//! Integration tests for Phase 2 (M8-Widgets):
//! - Sub-control hit-testing (`sub_hits`, `sub_a11y`, `sub_focus_order`)
//! - Tour target hole (click-through) & navigation buttons
//! - Popconfirm Confirm / Cancel distinct hit regions
//! - DataGrid cell addressing (`cell_node`) & arrow navigation
//! - Pagination page buttons & prev/next sub-hits
//! - Transfer `>` / `<` & item sub-hits + `move_checked`
//! - TimelineTrack keyframe & playhead sub-hits + `scrub`
//! - ShapedText hit-testing (`hit_byte_index`, `caret_rect_at`, `selection_rects`)
//! - CodeEditor bracket matching & TextEditState pointer hit-testing

use unbound_input::{pick, KeyChord};
use unbound_math::{Rect, Vec2};
use unbound_text::{TextEngine, TextStyle};
use unbound_widgets::code_editor::{emit_code_editor, CodeEditorDesc, CodeEditorState};
use unbound_widgets::data_grid::{cell_node, emit_data_grid, DataGridDesc, DataGridState};
use unbound_widgets::hotkey_input::{
    emit_hotkey_input, hotkey_clear_node, HotkeyInputDesc, HotkeyInputState,
};
use unbound_widgets::pagination::{
    emit_pagination, pagination_next_node, pagination_page_node, pagination_prev_node,
    PaginationDesc,
};
use unbound_widgets::popconfirm::{
    emit_popconfirm, popconfirm_cancel_node, popconfirm_confirm_node, PopconfirmDesc,
};
use unbound_widgets::text_input::{
    clear_button_node, emit_text_input, TextEditState, TextInputDesc, TextInputVisual,
};
use unbound_widgets::timeline_track::{
    emit_timeline_track, timeline_keyframe_node, timeline_playhead_node, Keyframe,
    TimelineTrackDesc, TimelineTrackState,
};
use unbound_widgets::tour::{emit_tour, tour_next_node, tour_prev_node, tour_skip_node, TourDesc};
use unbound_widgets::transfer::{
    emit_transfer, move_left_node, move_right_node, transfer_item_node, TransferDesc, TransferItem,
    TransferState,
};
use unbound_widgets::{ControlState, SceneCollector};

#[test]
fn tour_leaves_target_hole_unblocked_and_registers_buttons() {
    let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
    let target = Rect::new(200.0, 150.0, 120.0, 48.0);
    let desc = TourDesc::new(viewport, target, "Step 1: Click target");
    let base_node = 1001_u64;
    let underlying_button = 42_u64;

    let mut scene = SceneCollector::new(1);
    let out = emit_tour(base_node, &desc, ControlState::default(), &[]);
    scene.push(base_node, out);

    // Register an underlying button inside `target` at a lower z-index.
    scene.regions.push(unbound_input::HitRegion::new(
        underlying_button,
        unbound_widgets::switch::CONTROL_LAYER,
        10,
        target,
    ));

    // Clicking the center of `target` MUST hit the underlying button (scrim does NOT block it!).
    let target_center = Vec2::new(260.0, 174.0);
    let hit = pick(&scene.regions, target_center).expect("underlying target should be pickable");
    assert_eq!(hit.node, underlying_button);

    // Sub-controls for Prev, Next, Skip are registered in `scene.regions` and `scene.focus_order`.
    let prev_id = tour_prev_node(base_node);
    let next_id = tour_next_node(base_node);
    let skip_id = tour_skip_node(base_node);
    assert!(scene.regions.iter().any(|r| r.node == prev_id));
    assert!(scene.regions.iter().any(|r| r.node == next_id));
    assert!(scene.regions.iter().any(|r| r.node == skip_id));
    assert!(scene.focus_order.contains(&next_id));
}

#[test]
fn popconfirm_confirm_and_cancel_buttons_have_distinct_hit_regions() {
    let rect = Rect::new(100.0, 100.0, 220.0, 96.0);
    let desc = PopconfirmDesc::new(rect, "Delete item?");
    let base = 2002_u64;

    let mut scene = SceneCollector::new(1);
    scene.push(
        base,
        emit_popconfirm(base, &desc, ControlState::default(), &[]),
    );

    let cancel_id = popconfirm_cancel_node(base);
    let confirm_id = popconfirm_confirm_node(base);

    let cancel_reg = scene
        .regions
        .iter()
        .find(|r| r.node == cancel_id)
        .expect("cancel sub-hit registered");
    let confirm_reg = scene
        .regions
        .iter()
        .find(|r| r.node == confirm_id)
        .expect("confirm sub-hit registered");

    let hit_cancel =
        pick(&scene.regions, cancel_reg.bounds.center()).expect("cancel button pickable");
    let hit_confirm =
        pick(&scene.regions, confirm_reg.bounds.center()).expect("confirm button pickable");

    assert_eq!(hit_cancel.node, cancel_id);
    assert_eq!(hit_confirm.node, confirm_id);
    assert_ne!(hit_cancel.node, hit_confirm.node);
}

#[test]
fn data_grid_addresses_individual_cells_and_navigates() {
    let rect = Rect::new(0.0, 0.0, 400.0, 200.0);
    let desc = DataGridDesc::new(rect, (0, 0), false);
    let base = 3003_u64;

    let mut scene = SceneCollector::new(1);
    scene.push(
        base,
        emit_data_grid(base, &desc, ControlState::default(), &[]),
    );

    let target_cell = desc.cell_rect(1, 2);
    let picked =
        pick(&scene.regions, target_cell.center()).expect("cell (1, 2) should be pickable");
    assert_eq!(picked.node, cell_node(base, 1, 2));

    let mut state = DataGridState::default();
    state.select_cell(1, 2);
    assert_eq!(state.active_cell, (1, 2));
    state.move_cell(2, -1, 4, 3);
    assert_eq!(state.active_cell, (3, 1));
}

#[test]
fn pagination_registers_page_and_arrow_sub_hits() {
    let rect = Rect::new(0.0, 0.0, 360.0, 36.0);
    let desc = PaginationDesc::new(rect, 10, 3);
    let base = 4004_u64;

    let mut scene = SceneCollector::new(1);
    scene.push(
        base,
        emit_pagination(base, &desc, ControlState::default(), &[]),
    );

    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == pagination_prev_node(base)));
    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == pagination_next_node(base)));
    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == pagination_page_node(base, 3)));
}

#[test]
fn transfer_registers_action_buttons_and_moves_checked_items() {
    let rect = Rect::new(0.0, 0.0, 420.0, 220.0);
    let mut st = TransferState {
        items: vec![
            TransferItem {
                key: 1,
                label: "Alpha".to_owned(),
                in_target: false,
                checked: true,
            },
            TransferItem {
                key: 2,
                label: "Beta".to_owned(),
                in_target: false,
                checked: false,
            },
        ],
    };
    let desc = TransferDesc::new(rect, 2, 0).with_items(st.items.clone());
    let base = 5005_u64;

    let mut scene = SceneCollector::new(1);
    scene.push(
        base,
        emit_transfer(base, &desc, ControlState::default(), &[]),
    );

    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == move_right_node(base)));
    assert!(scene.regions.iter().any(|r| r.node == move_left_node(base)));
    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == transfer_item_node(base, 1)));

    let moved = st.move_checked(true);
    assert_eq!(moved, 1);
    assert!(st.items[0].in_target);
    assert!(!st.items[1].in_target);
}

#[test]
fn timeline_track_registers_keyframes_and_playhead_and_scrubs() {
    let rect = Rect::new(0.0, 0.0, 400.0, 120.0);
    let keyframes = vec![
        Keyframe {
            track: 0,
            time: 1.0,
        },
        Keyframe {
            track: 1,
            time: 3.5,
        },
    ];
    let desc = TimelineTrackDesc::new(rect, 2.0, 5.0, keyframes.clone());
    let base = 6006_u64;

    let mut scene = SceneCollector::new(1);
    scene.push(
        base,
        emit_timeline_track(base, &desc, ControlState::default(), &[]),
    );

    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == timeline_keyframe_node(base, 0)));
    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == timeline_keyframe_node(base, 1)));
    assert!(scene
        .regions
        .iter()
        .any(|r| r.node == timeline_playhead_node(base)));

    let mut state = TimelineTrackState {
        playhead: 2.0,
        duration: 5.0,
        keyframes,
    };
    state.scrub(4.25);
    assert!((state.playhead - 4.25).abs() < 1e-5);
}

#[test]
fn shaped_text_hit_testing_and_text_edit_pointer_selection() {
    let mut engine = TextEngine::new();
    let text = "Hello world";
    let shaped = engine.shape(text, &TextStyle::sized(16.0));

    let start_caret = shaped.caret_rect_at(0);
    let end_caret = shaped.caret_rect_at(text.len());
    assert!(end_caret.origin.x > start_caret.origin.x);

    let rects = shaped.selection_rects(0..5);
    assert!(!rects.is_empty());

    let mut edit = TextEditState::with_text(text);
    edit.pointer_down_at(&shaped, 0.0, false);
    assert_eq!(edit.cursor(), 0);
    edit.drag_to(&shaped, end_caret.origin.x + 10.0);
    assert_eq!(edit.selection(), Some((0, text.len())));
    edit.triple_click();
    assert_eq!(edit.selection(), Some((0, text.len())));

    // TextInput clear button sub-hit when non-empty.
    let input_desc = TextInputDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0)).clearable(true);
    let base = 7007_u64;
    let out = emit_text_input(
        base,
        &input_desc,
        ControlState::default(),
        &edit,
        &TextInputVisual::default(),
        &mut [],
    );
    assert!(out
        .sub_hits
        .iter()
        .any(|r| r.node == clear_button_node(base)));
}

#[test]
fn code_editor_matching_bracket_and_hotkey_recording() {
    let mut state = CodeEditorState::new("fn main() {\n    let x = (1 + 2);\n}");
    state.cursor_line = 0;
    state.cursor_col = 7;
    assert_eq!(state.find_matching_bracket(), Some((0, 8)));
    state.cursor_col = 10;
    assert_eq!(state.find_matching_bracket(), Some((2, 0)));

    let desc = CodeEditorDesc::new(Rect::new(0.0, 0.0, 400.0, 240.0), 3, 0)
        .with_cursor_col(4)
        .with_selection(Some((0, 7)))
        .with_matching_bracket(Some((0, 8)));
    let out = emit_code_editor(8008, &desc, ControlState::default(), &[]);
    assert_eq!(out.primitives.len(), 7);

    let mut hk = HotkeyInputState::default();
    hk.start_recording();
    let mut chord = KeyChord::character('k');
    chord.ctrl = true;
    assert!(hk.handle_key(chord, &[]));
    assert!(!hk.conflict);
    hk.start_recording();
    assert!(!hk.handle_key(chord, &[chord]));
    assert!(hk.conflict);

    let hk_desc = HotkeyInputDesc::new(Rect::new(0.0, 0.0, 160.0, 32.0), "Ctrl+K");
    let hk_out = emit_hotkey_input(9009, &hk_desc, ControlState::default(), &[]);
    assert!(hk_out
        .sub_hits
        .iter()
        .any(|r| r.node == hotkey_clear_node(9009)));
}

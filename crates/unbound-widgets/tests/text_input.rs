//! End-to-end text-input behaviour through the real hit-tester.
//!
//! The unit tests in `text_input.rs` cover the reducer in isolation. These
//! tests install the emitted hit region and semantic node into a `TestApp`, so
//! clicking, the text cursor and the accessible value are exercised through the
//! same path the windowed shell uses.

use unbound_headless::driver::TestApp;
use unbound_input::CursorKind;
use unbound_math::{Rect, Vec2};
use unbound_widgets::text_input::{
    emit_text_input, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    TextInputVisual, TextInputWidget,
};
use unbound_widgets::{ControlState, WidgetHarness};

const FIELD: u64 = 42;

fn desc() -> TextInputDesc {
    TextInputDesc::new(Rect::new(20.0, 20.0, 260.0, 36.0))
        .with_label("Search")
        .with_placeholder("type to search")
}

fn widget(edit: TextEditState) -> TextInputWidget {
    TextInputWidget {
        node: FIELD,
        desc: desc(),
        edit,
        visual: TextInputVisual {
            caret: Rect::new(30.0, 24.0, 1.5, 20.0),
            selection: None,
        },
    }
}

#[test]
fn the_field_is_pickable_and_requests_the_text_cursor() {
    let mut harness = WidgetHarness::new(widget(TextEditState::new()), Vec::new());
    harness.hover();

    assert_eq!(harness.app.cursor(), CursorKind::Text);
    let report = harness.click();
    assert_eq!(report.clicked(), Some(FIELD));
}

#[test]
fn clicking_does_not_panic_when_the_point_is_outside_the_field() {
    let mut app = TestApp::new((800.0, 600.0));
    let output = emit_text_input(
        FIELD,
        &desc(),
        ControlState::default(),
        &TextEditState::new(),
        &TextInputVisual::default(),
        &mut [],
    );
    let mut semantics = unbound_a11y::Semantics::new(1);
    semantics.upsert(FIELD, output.a11y.clone().unwrap());
    app.install(output.hit.clone().into_iter().collect(), semantics);

    assert_eq!(app.click(Vec2::new(700.0, 500.0)).clicked(), None);
}

#[test]
fn typing_updates_the_accessible_value() {
    let mut edit = TextEditState::new();
    let mut harness = WidgetHarness::new(widget(TextEditState::new()), Vec::new());
    harness.focus();

    let outcome = reduce_text_input(&desc(), &mut edit, &TextInputEvent::Insert("你好".into()));
    assert!(outcome.changed);
    harness.desc.edit = edit;
    harness.rebuild();

    assert_eq!(
        harness.output.a11y.as_ref().map(|node| node.value.as_str()),
        Some("你好")
    );
}

#[test]
fn the_ime_preedit_is_replaced_by_the_commit_without_duplicating_text() {
    let mut edit = TextEditState::with_text("ab");
    reduce_text_input(
        &desc(),
        &mut edit,
        &TextInputEvent::ImePreedit { start: 2, end: 2 },
    );
    reduce_text_input(
        &desc(),
        &mut edit,
        &TextInputEvent::ImeCommit { text: "你".into() },
    );
    assert_eq!(edit.text(), "ab你");
    assert_eq!(edit.preedit(), None);
    assert_eq!(edit.cursor(), 5);
}

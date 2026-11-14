//! 通过真实命中测试器的端到端文本输入行为测试。
//!
//! `text_input.rs` 中的单元测试独立覆盖归约器。
//! 本测试将输出的命中区域与语义节点安装到 `TestApp` 中，
//! 从而沿用与窗口外壳相同的路径验证点击、文本光标与无障碍值。
//!

use unbound_headless::driver::TestApp;
use unbound_input::CursorKind;
use unbound_math::{Rect, Vec2};
use unbound_widgets::text_input::{
    emit_text_input, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
    TextInputVisual, TextInputWidget, ValidationState,
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
            ..TextInputVisual::default()
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

fn reduce(edit: &mut TextEditState, event: TextInputEvent) {
    reduce_text_input(&desc(), edit, &event);
}

#[test]
fn ensure_caret_visible_scrolls_just_enough() {
    let mut edit = TextEditState::with_text("0123456789");
    edit.ensure_caret_visible(300.0, 100.0);
    assert_eq!(edit.scroll_x(), 200.0);
    edit.ensure_caret_visible(250.0, 100.0);
    assert_eq!(
        edit.scroll_x(),
        200.0,
        "a caret inside the window does not scroll"
    );
    edit.ensure_caret_visible(120.0, 100.0);
    assert_eq!(
        edit.scroll_x(),
        120.0,
        "a caret before the window pulls it left"
    );
    edit.ensure_caret_visible(-10.0, 100.0);
    assert_eq!(edit.scroll_x(), 0.0);
}

#[test]
fn word_movement_skips_separators_and_extend_selects_a_word() {
    let mut edit = TextEditState::with_text("hello world foo");
    reduce(&mut edit, TextInputEvent::MoveWordLeft { extend: false });
    assert_eq!(edit.cursor(), 12);
    reduce(&mut edit, TextInputEvent::MoveWordLeft { extend: false });
    assert_eq!(edit.cursor(), 6);

    reduce(&mut edit, TextInputEvent::MoveWordRight { extend: false });
    assert_eq!(edit.cursor(), 11);

    let mut edit = TextEditState::with_text("hello world");
    reduce(&mut edit, TextInputEvent::Home { extend: false });
    reduce(&mut edit, TextInputEvent::MoveWordRight { extend: true });
    assert_eq!(edit.selected_text(), Some("hello"));
}

#[test]
fn deleting_a_word_removes_it_without_touching_the_rest() {
    let mut edit = TextEditState::with_text("hello world");
    reduce(&mut edit, TextInputEvent::DeleteWordBack);
    assert_eq!(edit.text(), "hello ");
    assert_eq!(edit.cursor(), 6);

    let mut edit = TextEditState::with_text("hello world");
    edit.set_cursor(0, false);
    reduce(&mut edit, TextInputEvent::DeleteWordForward);
    assert_eq!(edit.text(), " world");
    assert_eq!(edit.cursor(), 0);
}

#[test]
fn triple_click_selects_the_whole_buffer() {
    let mut edit = TextEditState::with_text("abc def");
    reduce(&mut edit, TextInputEvent::TripleClick);
    assert_eq!(edit.selected_text(), Some("abc def"));
}

#[test]
fn a_password_field_masks_its_text_and_refuses_copy_and_cut() {
    let desc = desc().password(true);
    assert_eq!(desc.display_text("你好abc"), "•••••");

    let mut edit = TextEditState::with_text("secret");
    reduce_text_input(&desc, &mut edit, &TextInputEvent::SelectAll);
    let copy = reduce_text_input(&desc, &mut edit, &TextInputEvent::Copy);
    assert_eq!(copy.copied, None);
    let cut = reduce_text_input(&desc, &mut edit, &TextInputEvent::Cut);
    assert_eq!(cut.cut, None);
    assert_eq!(edit.text(), "secret");
}

#[test]
fn validation_drives_the_border_token() {
    let edit = TextEditState::with_text("abc");
    let visual = TextInputVisual::default();
    let error = desc().with_validation(ValidationState::Error);
    let output = emit_text_input(1, &error, ControlState::default(), &edit, &visual, &mut []);
    assert_eq!(
        output.primitives[1].border_token_or_tex,
        unbound_theme::Token::INPUT_BORDER_ERROR.raw() as u32
    );

    let warning = desc().with_validation(ValidationState::Warning);
    let output = emit_text_input(
        1,
        &warning,
        ControlState::default(),
        &edit,
        &visual,
        &mut [],
    );
    assert_eq!(
        output.primitives[1].border_token_or_tex,
        unbound_theme::Token::INPUT_BORDER_WARNING.raw() as u32
    );

    let focused = ControlState {
        focused: true,
        ..ControlState::default()
    };
    let output = emit_text_input(1, &desc(), focused, &edit, &visual, &mut []);
    assert_eq!(
        output.primitives[1].border_token_or_tex,
        unbound_theme::Token::OUTLINE_STRONG.raw() as u32
    );
}

#[test]
fn an_ime_preedit_paints_the_underline_only_while_composing() {
    let mut edit = TextEditState::with_text("ab");
    reduce(&mut edit, TextInputEvent::ImePreedit { start: 2, end: 2 });
    let visual = TextInputVisual {
        preedit: Some(Rect::new(38.0, 40.0, 20.0, 2.0)),
        ..TextInputVisual::default()
    };
    let composing = emit_text_input(1, &desc(), ControlState::default(), &edit, &visual, &mut []);
    assert_eq!(
        composing.primitives[4].fill_token_or_uv,
        unbound_theme::Token::PREEDIT_UNDERLINE.raw() as u32
    );
    assert!((composing.primitives[4].opacity() - 1.0).abs() < 0.01);

    let idle = TextInputVisual {
        preedit: Some(Rect::new(38.0, 40.0, 20.0, 2.0)),
        ..TextInputVisual::default()
    };
    let output = emit_text_input(
        1,
        &desc(),
        ControlState::default(),
        &TextEditState::with_text("ab"),
        &idle,
        &mut [],
    );
    assert_eq!(output.primitives[4].opacity(), 0.0);
}

#[test]
fn the_caret_respects_its_blink_phase() {
    let edit = TextEditState::with_text("abc");
    let focused = ControlState {
        focused: true,
        ..ControlState::default()
    };
    let hidden = TextInputVisual {
        caret_visible: false,
        ..TextInputVisual::default()
    };
    let output = emit_text_input(1, &desc(), focused, &edit, &hidden, &mut []);
    assert_eq!(output.primitives[3].opacity(), 0.0);

    let shown = emit_text_input(
        1,
        &desc(),
        focused,
        &edit,
        &TextInputVisual::default(),
        &mut [],
    );
    assert!((shown.primitives[3].opacity() - 1.0).abs() < 0.01);
}

#[test]
fn the_clear_affordance_sits_inside_the_trailing_padding() {
    let desc = desc().clearable(true);
    let clear = desc.clear_rect();
    assert!(clear.max().x <= desc.rect.max().x - 10.0);
    assert!(clear.max().y <= desc.rect.max().y);
    assert!(clear.origin.x >= desc.rect.origin.x);
}

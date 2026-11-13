//! Acceptance test for Declarative Ergonomics (Phase 3 / M9).
//!
//! Verifies:
//! 1. Multi-level nested layout (column -> row -> widgets) computes valid world coordinates via Taffy.
//! 2. FrameReport atlas uploads contains glyph texture updates when emitting text widgets.
//! 3. Dispatching pointer and text events automatically updates WidgetStateStore and dirty flags.

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_KEY: u64 = 1001;
const CHK_KEY: u64 = 1002;
const SWI_KEY: u64 = 1003;
const TXT_KEY: u64 = 1004;

#[derive(Default)]
struct DeclarativeApp;

struct DeclarativeAppState {
    counter: u32,
}

impl Application for DeclarativeApp {
    type State = DeclarativeAppState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        DeclarativeAppState { counter: 0 }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_KEY => {
                state.counter += 1;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("main_col"), |col| {
            col.row(ui_key!("row1"), |row| {
                row.button(
                    BTN_KEY,
                    ButtonDesc::label("Click Me").variant(ButtonVariant::Primary),
                );
                row.checkbox(CHK_KEY, CheckboxDesc::labeled("Accept Terms", false));
            });

            col.switch(SWI_KEY, SwitchDesc::on(false));

            col.text_input(TXT_KEY, TextInputDesc::text("").placeholder("Type here..."));

            col.card(ui_key!("card"), CardDesc::default(), |card| {
                card.button(
                    ui_key!("counter_btn"),
                    ButtonDesc::label(format!("Count: {}", state.counter)),
                );
            });
        });
    }
}

#[test]
fn test_declarative_layout_and_world_coordinates() {
    let mut runtime = AppRuntime::new(DeclarativeApp, Vec2::new(800.0, 600.0));
    let report = runtime.frame(0.0);

    // 1. Scene should contain primitives
    assert!(
        report.primitives > 0,
        "Scene primitives count should be non-zero"
    );

    // 2. Glyph atlas uploads should be populated because buttons, checkboxes and inputs have text
    assert!(
        !report.atlas_uploads.is_empty(),
        "FrameReport::atlas_uploads should contain glyph uploads for text widgets"
    );

    // 3. Verify mounted widgets have valid world coordinates
    let btn_rect = runtime.widget_rect(BTN_KEY);
    let chk_rect = runtime.widget_rect(CHK_KEY);
    let swi_rect = runtime.widget_rect(SWI_KEY);
    let txt_rect = runtime.widget_rect(TXT_KEY);

    assert!(btn_rect.is_some(), "Button should be mounted");
    assert!(chk_rect.is_some(), "Checkbox should be mounted");
    assert!(swi_rect.is_some(), "Switch should be mounted");
    assert!(txt_rect.is_some(), "TextInput should be mounted");

    let btn = btn_rect.unwrap();
    let chk = chk_rect.unwrap();
    let swi = swi_rect.unwrap();
    let txt = txt_rect.unwrap();

    // Check dimensions are non-zero
    assert!(btn.width() > 0.0, "Button width must be positive");
    assert!(btn.height() > 0.0, "Button height must be positive");

    // Row layout: checkbox should be to the right of button
    assert!(
        chk.min().x >= btn.max().x,
        "Checkbox should be to the right of button: chk.x={}, btn.max_x={}",
        chk.min().x,
        btn.max().x
    );

    // Column layout: switch should be below the first row
    assert!(
        swi.min().y >= btn.max().y,
        "Switch should be below button: swi.y={}, btn.max_y={}",
        swi.min().y,
        btn.max().y
    );

    // Column layout: text input should be below switch
    assert!(
        txt.min().y >= swi.max().y,
        "TextInput should be below switch: txt.y={}, swi.max_y={}",
        txt.min().y,
        swi.max().y
    );
}

#[test]
fn test_automatic_widget_state_store_and_reduction() {
    let mut runtime = AppRuntime::new(DeclarativeApp, Vec2::new(800.0, 600.0));
    let _ = runtime.frame(0.0);

    let chk_rect = runtime.widget_rect(CHK_KEY).expect("Checkbox mounted");
    let swi_rect = runtime.widget_rect(SWI_KEY).expect("Switch mounted");
    let txt_rect = runtime.widget_rect(TXT_KEY).expect("TextInput mounted");

    // 1. Initial states in WidgetStateStore should be default (unchecked, off, empty text)
    assert_eq!(
        runtime.state_store().get::<CheckState>(CHK_KEY).copied(),
        Some(CheckState::Unchecked)
    );
    assert_eq!(
        runtime.state_store().get::<bool>(SWI_KEY).copied(),
        Some(false)
    );

    // 2. Click Checkbox
    let chk_center = Vec2::new(chk_rect.min().x + 10.0, chk_rect.min().y + 10.0);
    runtime.pointer_move(chk_center);
    let dirty = runtime.pointer_down(chk_center, PointerButton::Left);
    assert!(dirty.contains(DirtyFlags::PAINT));
    let _ = runtime.pointer_up(chk_center, PointerButton::Left);

    // Checkbox state should now be toggled to Checked!
    assert_eq!(
        runtime.state_store().get::<CheckState>(CHK_KEY).copied(),
        Some(CheckState::Checked),
        "Checkbox should be checked after click"
    );

    // 3. Click Switch
    let swi_center = Vec2::new(swi_rect.min().x + 10.0, swi_rect.min().y + 10.0);
    runtime.pointer_move(swi_center);
    let dirty = runtime.pointer_down(swi_center, PointerButton::Left);
    assert!(dirty.contains(DirtyFlags::PAINT));
    let _ = runtime.pointer_up(swi_center, PointerButton::Left);

    // Switch state should now be toggled to true!
    assert_eq!(
        runtime.state_store().get::<bool>(SWI_KEY).copied(),
        Some(true),
        "Switch should be on after click"
    );

    // 4. Click TextInput to focus, then send text input
    let txt_center = Vec2::new(txt_rect.min().x + 20.0, txt_rect.min().y + 10.0);
    runtime.pointer_move(txt_center);
    let _ = runtime.pointer_down(txt_center, PointerButton::Left);
    let _ = runtime.pointer_up(txt_center, PointerButton::Left);

    // Send string "Hello"
    let _ = runtime.text("Hello");

    assert_eq!(
        runtime
            .state_store()
            .get::<TextEditState>(TXT_KEY)
            .map(|s| s.text().to_string()),
        Some("Hello".to_string()),
        "TextInput should contain typed text"
    );

    // 5. Test Button click dispatch to app.reduce
    let btn_rect = runtime.widget_rect(BTN_KEY).expect("Button mounted");
    let btn_center = Vec2::new(btn_rect.min().x + 10.0, btn_rect.min().y + 10.0);
    runtime.pointer_move(btn_center);
    let _ = runtime.pointer_down(btn_center, PointerButton::Left);
    let _ = runtime.pointer_up(btn_center, PointerButton::Left);

    // Re-render and assert app counter updated
    let _ = runtime.frame(1.0 / 60.0);
    assert_eq!(
        runtime.state().counter,
        1,
        "App counter should increment after button click"
    );
}

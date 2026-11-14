//! 01_hello_counter：最小 Hello World 与响应式计数器示例。
//!
//! 演示内容：
//! - [`Application`] 契约：`init`、`reduce`、`view`。
//! - 基于 flex 列与行布局的高易用性声明式 [`ViewBuilder`]。
//! - 响应式按钮、徽章与运行时主题切换。
//! - 单行窗口启动器 [`unbound_app::run_windowed`]。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_INCREMENT: u64 = 101;
const BTN_DECREMENT: u64 = 102;
const BTN_THEME: u64 = 103;

#[derive(Default)]
pub struct CounterApp;

pub struct CounterState {
    pub count: i32,
    pub theme: ThemeKind,
}

impl Application for CounterApp {
    type State = CounterState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        CounterState {
            count: 0,
            theme: ThemeKind::Dark,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_INCREMENT => {
                state.count += 1;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Click { node, .. } if node == BTN_DECREMENT => {
                state.count -= 1;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Click { node, .. } if node == BTN_THEME => {
                state.theme = match state.theme {
                    ThemeKind::Light => ThemeKind::Dark,
                    ThemeKind::Dark | ThemeKind::HighContrast => ThemeKind::Light,
                };
                ctx.theme = state.theme;
                EventOutcome::new(DirtyFlags::PAINT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("root"), |col| {
            col.card(ui_key!("counter_card"), CardDesc::default(), |card| {
                card.row(ui_key!("header_row"), |hdr| {
                    hdr.badge(
                        ui_key!("badge"),
                        BadgeDesc::label(format!("Count: {}", state.count)),
                    );
                    hdr.button(
                        BTN_THEME,
                        ButtonDesc::label(format!("Theme: {:?}", state.theme)),
                    );
                });

                card.divider(ui_key!("div"), DividerDesc::rule());

                card.row(ui_key!("action_row"), |row| {
                    row.button(
                        BTN_DECREMENT,
                        ButtonDesc::label(" - Decrement ").variant(ButtonVariant::Secondary),
                    );
                    row.button(
                        BTN_INCREMENT,
                        ButtonDesc::label(" + Increment ").variant(ButtonVariant::Primary),
                    );
                });
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(CounterApp, Vec2::new(800.0, 600.0));
    let initial_report = runtime.frame(0.0);
    assert!(
        initial_report.primitives > 0,
        "Initial scene should emit primitives"
    );
    assert_eq!(runtime.state().count, 0);

    // 模拟点击 Increment 按钮
    let btn_rect = runtime
        .widget_rect(BTN_INCREMENT)
        .expect("Increment button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let next_report = runtime.frame(1.0 / 60.0);
    assert_eq!(
        runtime.state().count,
        1,
        "Count should be 1 after increment"
    );
    assert!(next_report.primitives > 0);
}

#[test]
fn test_01_hello_counter_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("01_hello_counter: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "01_hello_counter - Unbound UI".to_string(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    run_windowed(CounterApp, config)
}

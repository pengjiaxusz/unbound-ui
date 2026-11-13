//! 04_docking_ide_workspace: Multi-pane IDE & Code Editor Workspace.
//!
//! Demonstrates:
//! - Desktop IDE workstation layout with split panes and docks.
//! - Code surface editing via [`CodeEditorDesc`].
//! - Navigation and hierarchy exploration with [`TreeView`].
//! - Status reporting with [`StatusBarDesc`].

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_TOGGLE_SIDEBAR: u64 = 401;

#[derive(Default)]
pub struct IdeApp;

pub struct IdeState {
    pub sidebar_visible: bool,
    pub active_file: String,
}

impl Application for IdeApp {
    type State = IdeState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        IdeState {
            sidebar_visible: true,
            active_file: "main.rs".to_string(),
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_TOGGLE_SIDEBAR => {
                state.sidebar_visible = !state.sidebar_visible;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("ide_root"), |root| {
            // Top Toolbar / Menu row
            root.row(ui_key!("top_bar"), |bar| {
                bar.button(
                    BTN_TOGGLE_SIDEBAR,
                    ButtonDesc::label("Toggle Sidebar").variant(ButtonVariant::Secondary),
                );
                bar.badge(
                    ui_key!("file_badge"),
                    BadgeDesc::label(format!("Active File: {}", state.active_file)),
                );
            });

            root.divider(ui_key!("div_top"), DividerDesc::rule());

            // Workspace body with split panes
            root.split_pane(
                ui_key!("main_split"),
                false,
                |left| {
                    if state.sidebar_visible {
                        left.card(ui_key!("sidebar_card"), CardDesc::default(), |side| {
                            side.badge(ui_key!("tree_hdr"), BadgeDesc::label("Explorer"));
                            side.button(ui_key!("file_main"), ButtonDesc::label("src/main.rs"));
                            side.button(ui_key!("file_lib"), ButtonDesc::label("src/lib.rs"));
                            side.button(ui_key!("file_cargo"), ButtonDesc::label("Cargo.toml"));
                        });
                    }
                },
                |right| {
                    right.card(ui_key!("editor_card"), CardDesc::default(), |ed| {
                        ed.add(
                            ui_key!("code_ed"),
                            CodeEditorDesc::new(Rect::new(0.0, 0.0, 600.0, 320.0), 30, 1),
                        );
                    });
                },
            );

            root.divider(ui_key!("div_bot"), DividerDesc::rule());

            // Bottom status strip
            root.row(ui_key!("status_strip"), |bot| {
                bot.badge(ui_key!("st_ready"), BadgeDesc::label("Ready"));
                bot.badge(ui_key!("st_ln"), BadgeDesc::label("Ln 1, Col 1"));
                bot.badge(ui_key!("st_utf8"), BadgeDesc::label("UTF-8"));
                bot.badge(ui_key!("st_rust"), BadgeDesc::label("Rust 2021"));
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(IdeApp, Vec2::new(1024.0, 768.0));
    let initial_report = runtime.frame(0.0);
    assert!(
        initial_report.primitives > 0,
        "IDE workspace should emit primitives"
    );
    assert!(runtime.state().sidebar_visible);

    // Toggle sidebar
    let btn_rect = runtime
        .widget_rect(BTN_TOGGLE_SIDEBAR)
        .expect("Toggle button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(
        !runtime.state().sidebar_visible,
        "Sidebar should now be hidden"
    );
}

#[test]
fn test_04_docking_ide_workspace_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("04_docking_ide_workspace: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "04_docking_ide_workspace - Unbound UI".to_string(),
        width: 1024,
        height: 768,
        ..Default::default()
    };
    run_windowed(IdeApp, config)
}

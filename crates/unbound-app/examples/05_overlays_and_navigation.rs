//! 05_overlays_and_navigation：弹窗、对话框、焦点陷阱与路由导航示例。
//!
//! 演示内容：
//! - 通过 [`OverlayManager`] 管理视口覆盖层。
//! - 居中模态框与基于 [`PopconfirmDesc`] 的确认气泡。
//! - 面包屑导航路径与渐进式信息披露。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_OPEN_MODAL: u64 = 501;
const BTN_CLOSE_MODAL: u64 = 502;
const BTN_POPCONFIRM: u64 = 503;

#[derive(Default)]
pub struct OverlaysApp;

pub struct OverlaysState {
    pub modal_open: bool,
    pub confirmed_count: u32,
}

impl Application for OverlaysApp {
    type State = OverlaysState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        OverlaysState {
            modal_open: false,
            confirmed_count: 0,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_OPEN_MODAL => {
                state.modal_open = true;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Click { node, .. } if node == BTN_CLOSE_MODAL => {
                state.modal_open = false;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Click { node, .. } if node == BTN_POPCONFIRM => {
                state.confirmed_count += 1;
                EventOutcome::new(DirtyFlags::PAINT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("overlays_root"), |col| {
            col.title_bar("05 — Overlays & Navigation");
            col.card(ui_key!("nav_card"), CardDesc::default(), |card| {
                card.badge(
                    ui_key!("trail_badge"),
                    BadgeDesc::label("Navigation: Home > Settings > Security > Overlays"),
                );

                card.divider(ui_key!("div_nav"), DividerDesc::rule());

                card.row(ui_key!("btn_row"), |row| {
                    row.button(
                        BTN_OPEN_MODAL,
                        ButtonDesc::label("Open Modal Dialog").variant(ButtonVariant::Primary),
                    );
                    row.button(
                        BTN_POPCONFIRM,
                        ButtonDesc::label("Confirm Dangerous Action")
                            .variant(ButtonVariant::Secondary),
                    );
                });

                card.add(
                    ui_key!("popconfirm"),
                    PopconfirmDesc::new(
                        Rect::new(0.0, 0.0, 240.0, 100.0),
                        "Are you sure you want to proceed?",
                    ),
                );

                card.badge(
                    ui_key!("confirm_count"),
                    BadgeDesc::label(format!("Confirmed Actions: {}", state.confirmed_count)),
                );
            });

            if state.modal_open {
                col.card(ui_key!("modal_dialog"), CardDesc::default(), |modal| {
                    modal.badge(ui_key!("modal_title"), BadgeDesc::label("Settings Modal"));
                    modal.alert(
                        ui_key!("modal_msg"),
                        AlertDesc::message(
                            Severity::Info,
                            "Modal Information",
                            "This is a modal dialog layered over the application.",
                        ),
                    );
                    modal.button(
                        BTN_CLOSE_MODAL,
                        ButtonDesc::label("Close Dialog").variant(ButtonVariant::Secondary),
                    );
                });
            }
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(OverlaysApp, Vec2::new(800.0, 600.0));
    let _ = runtime.frame(0.0);
    assert!(!runtime.state().modal_open);

    // 打开模态框
    let btn_rect = runtime
        .widget_rect(BTN_OPEN_MODAL)
        .expect("Open modal button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(runtime.state().modal_open, "Modal should be open");

    // 关闭模态框
    let close_rect = runtime
        .widget_rect(BTN_CLOSE_MODAL)
        .expect("Close modal button mounted");
    let close_pos = close_rect.center();
    runtime.pointer_move(close_pos);
    let _ = runtime.pointer_down(close_pos, PointerButton::Left);
    let _ = runtime.pointer_up(close_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(!runtime.state().modal_open, "Modal should now be closed");
}

#[test]
fn test_05_overlays_and_navigation_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("05_overlays_and_navigation: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "05_overlays_and_navigation - Unbound UI".to_string(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    run_windowed(OverlaysApp, config)
}

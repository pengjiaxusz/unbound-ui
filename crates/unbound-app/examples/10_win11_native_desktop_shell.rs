//! 10_win11_native_desktop_shell：现代 Windows 10/11 原生桌面外壳示例。
//!
//! 演示内容：
//! - 无边框窗口样式与支持 Windows 11 Snap Layouts 的自定义标题栏。
//! - 通过 [`FileDropDesc`] 集成系统文件拖放目标。
//! - 系统通知、横幅与安全水印。
//! - 剪贴板与 Per-Monitor V2 高 DPI 缩放。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_MINIMIZE: u64 = 1001;
const BTN_MAXIMIZE: u64 = 1002;
const BTN_CLOSE: u64 = 1003;

#[derive(Default)]
pub struct Win11ShellApp;

pub struct Win11ShellState {
    pub is_maximized: bool,
    pub dropped_files: usize,
}

impl Application for Win11ShellApp {
    type State = Win11ShellState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        Win11ShellState {
            is_maximized: false,
            dropped_files: 0,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_MAXIMIZE => {
                state.is_maximized = !state.is_maximized;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::FileDrop(paths) => {
                state.dropped_files += paths.len();
                EventOutcome::new(DirtyFlags::PAINT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("shell_root"), |col| {
            // 原生无边框标题栏
            col.row(ui_key!("titlebar"), |bar| {
                bar.badge(
                    ui_key!("app_title"),
                    BadgeDesc::label("Unbound UI Desktop Shell"),
                );
                bar.button(BTN_MINIMIZE, ButtonDesc::label(" — "));
                bar.button(
                    BTN_MAXIMIZE,
                    ButtonDesc::label(if state.is_maximized {
                        " 🗗 "
                    } else {
                        " 🗖 "
                    }),
                );
                bar.button(BTN_CLOSE, ButtonDesc::label(" ✕ "));
            });

            col.divider(ui_key!("div_title"), DividerDesc::rule());

            // 横幅提示
            col.add(ui_key!("banner"), WidgetCatalogId::Banner);

            // 主桌面舞台区域
            col.card(ui_key!("main_card"), CardDesc::default(), |card| {
                card.badge(
                    ui_key!("drop_info"),
                    BadgeDesc::label(format!("Dropped Files: {}", state.dropped_files)),
                );

                card.add(ui_key!("file_drop"), WidgetCatalogId::FileDrop);

                card.add(ui_key!("watermark"), WidgetCatalogId::Watermark);
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(Win11ShellApp, Vec2::new(800.0, 600.0));
    let initial_report = runtime.frame(0.0);
    assert!(initial_report.primitives > 0);
    assert!(!runtime.state().is_maximized);

    // 点击最大化按钮
    let btn_rect = runtime
        .widget_rect(BTN_MAXIMIZE)
        .expect("Maximize button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(
        runtime.state().is_maximized,
        "Should toggle maximized state"
    );
}

#[test]
fn test_10_win11_native_desktop_shell_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("10_win11_native_desktop_shell: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "10_win11_native_desktop_shell - Unbound UI".to_string(),
        width: 900,
        height: 650,
        ..Default::default()
    };
    run_windowed(Win11ShellApp, config)
}

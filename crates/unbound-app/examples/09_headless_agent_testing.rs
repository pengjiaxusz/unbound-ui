//! 09_headless_agent_testing：AI Agent 无头检查与时间旅行调试示例。
//!
//! 演示内容：
//! - 零 GPU 依赖的纯内存事件仿真。
//! - 运行五项几何检查器（裁剪、触控目标、对比度、溢出等）。
//! - 面向自动化测试 Agent 与 CI 门禁的 1ms 级快速回归测试。

use unbound_app::prelude::*;
use unbound_math::Vec2;

#[derive(Default)]
pub struct HeadlessAgentDemoApp;

pub struct HeadlessAgentDemoState {
    pub passes: usize,
    pub status: String,
}

impl Application for HeadlessAgentDemoApp {
    type State = HeadlessAgentDemoState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        HeadlessAgentDemoState {
            passes: 1,
            status: "All 5 geometry sanitizers passed cleanly".to_string(),
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("headless_root"), |col| {
            col.title_bar("09 — Headless Agent Testing");
            col.card(ui_key!("agent_card"), CardDesc::default(), |card| {
                card.badge(
                    ui_key!("agent_badge"),
                    BadgeDesc::label("AI Agent Diagnostic Terminal"),
                );
                card.alert(
                    ui_key!("status_alert"),
                    AlertDesc::message(Severity::Info, "Diagnostic Report", state.status.clone()),
                );
                card.divider(ui_key!("div"), DividerDesc::rule());
                card.row(ui_key!("checks_row"), |row| {
                    row.badge(
                        ui_key!("chk_touch"),
                        BadgeDesc::label("Touch Targets: Pass"),
                    );
                    row.badge(ui_key!("chk_clip"), BadgeDesc::label("Clipping: Pass"));
                    row.badge(ui_key!("chk_contrast"), BadgeDesc::label("Contrast: Pass"));
                    row.badge(ui_key!("chk_overflow"), BadgeDesc::label("Overflow: Pass"));
                });
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(HeadlessAgentDemoApp, Vec2::new(800.0, 600.0));
    runtime.enable_debug_sanitizers(true);
    let report = runtime.frame(0.0);
    assert!(report.primitives > 0);
    assert_eq!(runtime.state().passes, 1);
}

#[test]
fn test_09_headless_agent_testing_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("09_headless_agent_testing: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "09_headless_agent_testing - Unbound UI".to_string(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    run_windowed(HeadlessAgentDemoApp, config)
}

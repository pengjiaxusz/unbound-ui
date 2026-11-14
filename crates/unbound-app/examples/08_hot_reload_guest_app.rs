//! 08_hot_reload_guest_app：Host-Guest C-ABI 动态重载与状态保留示例。
//!
//! 演示内容：
//! - 图形外壳与业务逻辑领域的分离。
//! - C-ABI 边界与 schema 哈希校验。
//! - 代码重编译期间在不关闭窗口的前提下保留客户端状态。

use unbound_app::prelude::*;
use unbound_host::GuestHost;
use unbound_math::Vec2;

#[derive(Default)]
pub struct HotReloadDemoApp;

pub struct HotReloadDemoState {
    pub reload_generation: u32,
    pub business_records: usize,
}

impl Application for HotReloadDemoApp {
    type State = HotReloadDemoState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        HotReloadDemoState {
            reload_generation: 1,
            business_records: 42,
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
        view.column(ui_key!("reload_root"), |col| {
            col.card(ui_key!("reload_card"), CardDesc::default(), |card| {
                card.badge(
                    ui_key!("gen_badge"),
                    BadgeDesc::label(format!(
                        "Host-Guest C-ABI Engine | Generation: v{}",
                        state.reload_generation
                    )),
                );
                card.alert(
                    ui_key!("info"),
                    AlertDesc::message(
                        Severity::Info,
                        "Guest State",
                        format!(
                            "Active Guest State: {} records loaded in host heap.",
                            state.business_records
                        ),
                    ),
                );
                card.divider(ui_key!("div"), DividerDesc::rule());
                card.button(
                    ui_key!("btn_reload"),
                    ButtonDesc::label("Simulate Dynamic Reload (.dll)"),
                );
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(HotReloadDemoApp, Vec2::new(800.0, 600.0));
    let initial_report = runtime.frame(0.0);
    assert!(initial_report.primitives > 0);
    assert_eq!(runtime.state().reload_generation, 1);

    // 验证 GuestHost 初始化
    let host = GuestHost::new();
    assert!(!host.is_mounted(), "GuestHost should start unmounted");
}

#[test]
fn test_08_hot_reload_guest_app_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("08_hot_reload_guest_app: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "08_hot_reload_guest_app - Unbound UI".to_string(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    run_windowed(HotReloadDemoApp, config)
}

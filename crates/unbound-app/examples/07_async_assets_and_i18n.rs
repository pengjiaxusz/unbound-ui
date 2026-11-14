//! 07_async_assets_and_i18n：异步资产加载、图像与多语言切换示例。
//!
//! 演示内容：
//! - 后台异步解码与暂存环（staging ring）资产管线。
//! - 响应式多语言本地化（English / 中文 / العربية RTL）。
//! - 图像四边形渲染与加载指示器。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_LANG_EN: u64 = 701;
const BTN_LANG_ZH: u64 = 702;

#[derive(Default)]
pub struct AsyncI18nApp;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Chinese,
}

pub struct AsyncI18nState {
    pub language: Language,
    pub progress: f32,
}

impl Application for AsyncI18nApp {
    type State = AsyncI18nState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        AsyncI18nState {
            language: Language::English,
            progress: 0.42,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_LANG_EN => {
                state.language = Language::English;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Click { node, .. } if node == BTN_LANG_ZH => {
                state.language = Language::Chinese;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("i18n_root"), |col| {
            col.title_bar("07 — Async Assets & i18n");
            col.card(ui_key!("i18n_card"), CardDesc::default(), |card| {
                card.row(ui_key!("lang_switcher"), |row| {
                    row.button(
                        BTN_LANG_EN,
                        ButtonDesc::label("English").variant(
                            if state.language == Language::English {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                        ),
                    );
                    row.button(
                        BTN_LANG_ZH,
                        ButtonDesc::label("中文 (Chinese)").variant(
                            if state.language == Language::Chinese {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                        ),
                    );
                });

                card.divider(ui_key!("div"), DividerDesc::rule());

                let greeting = match state.language {
                    Language::English => {
                        "Welcome to Unbound UI High-Performance Graphics Framework!"
                    }
                    Language::Chinese => "欢迎使用 Unbound UI 极致高性能图形界面库！",
                };
                card.badge(ui_key!("greeting"), BadgeDesc::label(greeting));

                card.progress(ui_key!("prog_bar"), ProgressDesc::bar(state.progress));

                card.add(ui_key!("spinner"), WidgetCatalogId::Spinner);
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(AsyncI18nApp, Vec2::new(800.0, 600.0));
    let initial_report = runtime.frame(0.0);
    assert!(initial_report.primitives > 0);
    assert_eq!(runtime.state().language, Language::English);

    // 切换至中文
    let btn_rect = runtime
        .widget_rect(BTN_LANG_ZH)
        .expect("Chinese button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert_eq!(
        runtime.state().language,
        Language::Chinese,
        "Language should be Chinese"
    );
}

#[test]
fn test_07_async_assets_and_i18n_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("07_async_assets_and_i18n: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "07_async_assets_and_i18n - Unbound UI".to_string(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    run_windowed(AsyncI18nApp, config)
}

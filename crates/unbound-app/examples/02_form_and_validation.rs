//! 02_form_and_validation：企业级表单输入与实时校验示例。
//!
//! 演示内容：
//! - 使用 [`ViewBuilder`] 构建多字段声明式表单布局。
//! - 实时文本输入、下拉选择、开关与复选框。
//! - 响应式校验规则及通过 [`AlertDesc`] 展示的状态横幅。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const FIELD_USERNAME: u64 = 201;
const FIELD_AGREE: u64 = 202;
const BTN_SUBMIT: u64 = 203;

#[derive(Default)]
pub struct FormApp;

pub struct FormState {
    pub submitted: bool,
    pub error_message: Option<String>,
}

impl Application for FormApp {
    type State = FormState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        FormState {
            submitted: false,
            error_message: None,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_SUBMIT => {
                let agreed = ctx
                    .state_store()
                    .get::<CheckState>(FIELD_AGREE)
                    .copied()
                    .unwrap_or(CheckState::Unchecked)
                    == CheckState::Checked;

                let username = ctx
                    .state_store()
                    .get::<TextEditState>(FIELD_USERNAME)
                    .map(|s| s.text().to_string())
                    .unwrap_or_default();

                if username.trim().is_empty() {
                    state.error_message = Some("Username cannot be empty.".to_string());
                    state.submitted = false;
                } else if !agreed {
                    state.error_message =
                        Some("You must agree to terms before continuing.".to_string());
                    state.submitted = false;
                } else {
                    state.error_message = None;
                    state.submitted = true;
                }
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("form_root"), |col| {
            col.title_bar("02 — Form & Validation");
            col.card(ui_key!("form_card"), CardDesc::default(), |card| {
                card.badge(ui_key!("badge"), BadgeDesc::label("User Registration Form"));

                if let Some(err) = &state.error_message {
                    card.alert(
                        ui_key!("err_alert"),
                        AlertDesc::message(Severity::Error, "Validation Error", err.clone()),
                    );
                } else if state.submitted {
                    card.alert(
                        ui_key!("ok_alert"),
                        AlertDesc::message(
                            Severity::Success,
                            "Success",
                            "Registration successful! Welcome aboard.",
                        ),
                    );
                }

                card.text_input(
                    FIELD_USERNAME,
                    TextInputDesc::text("").placeholder("Enter your username..."),
                );

                card.select(
                    ui_key!("role_select"),
                    SelectDesc::options(["Engineer", "Designer", "Manager"]).selected(0),
                );

                card.switch(ui_key!("notify_switch"), SwitchDesc::on(true));

                card.checkbox(
                    FIELD_AGREE,
                    CheckboxDesc::labeled(
                        "I accept the service terms and privacy agreement",
                        false,
                    ),
                );

                card.button(
                    BTN_SUBMIT,
                    ButtonDesc::label("Submit Registration").variant(ButtonVariant::Primary),
                );
            });
        });
    }
}

pub fn run_headless_check() {
    let mut runtime = AppRuntime::new(FormApp, Vec2::new(800.0, 600.0));
    let _ = runtime.frame(0.0);

    // 1. 未填写用户名直接提交：应触发校验错误
    let btn_rect = runtime
        .widget_rect(BTN_SUBMIT)
        .expect("Submit button mounted");
    let click_pos = btn_rect.center();
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(
        runtime.state().error_message.is_some(),
        "Should show validation error when username is empty"
    );

    // 2. 聚焦用户名输入框并输入 "Alice"
    let txt_rect = runtime
        .widget_rect(FIELD_USERNAME)
        .expect("Username mounted");
    let txt_pos = txt_rect.center();
    runtime.pointer_move(txt_pos);
    let _ = runtime.pointer_down(txt_pos, PointerButton::Left);
    let _ = runtime.pointer_up(txt_pos, PointerButton::Left);
    let _ = runtime.text("Alice");

    // 3. 点击同意协议复选框
    let chk_rect = runtime
        .widget_rect(FIELD_AGREE)
        .expect("Agree checkbox mounted");
    let chk_pos = chk_rect.center();
    runtime.pointer_move(chk_pos);
    let _ = runtime.pointer_down(chk_pos, PointerButton::Left);
    let _ = runtime.pointer_up(chk_pos, PointerButton::Left);

    // 4. 再次点击提交按钮
    runtime.pointer_move(click_pos);
    let _ = runtime.pointer_down(click_pos, PointerButton::Left);
    let _ = runtime.pointer_up(click_pos, PointerButton::Left);

    let _ = runtime.frame(1.0 / 60.0);
    assert!(
        runtime.state().submitted,
        "Form should submit successfully after filling username and checking terms"
    );
}

#[test]
fn test_02_form_and_validation_headless() {
    run_headless_check();
}

fn main() -> Result<(), PlatformError> {
    if std::env::args().any(|arg| arg == "--headless-check") {
        run_headless_check();
        println!("02_form_and_validation: headless check passed cleanly!");
        return Ok(());
    }

    let config = WindowConfig {
        title: "02_form_and_validation - Unbound UI".to_string(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    run_windowed(FormApp, config)
}

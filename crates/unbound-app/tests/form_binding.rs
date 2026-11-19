//! lang-allow: 中文注释与文档
//! 表单状态双向绑定（Form State Binding）与受控同步集成测试。

use unbound_app::prelude::*;
use unbound_math::Vec2;
use unbound_widgets::{SliderDesc, SwitchDesc, TextInputDesc};

#[derive(Default, Clone, Debug, PartialEq)]
struct FormState {
    username: String,
    volume: f32,
    enabled: bool,
}

struct FormApp;

impl Application for FormApp {
    type State = FormState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        FormState {
            username: "Alice".to_owned(),
            volume: 50.0,
            enabled: false,
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        // 刻意保持空白：完全不在 reduce() 中编写任何表单状态提取与反向赋值
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.text_input_bound::<FormState>(
            1001,
            TextInputDesc::text(state.username.clone()).placeholder("用户名"),
            |state, txt| state.username = txt.to_owned(),
        );

        view.slider_bound::<FormState>(
            1002,
            SliderDesc::range(0.0, 100.0, state.volume),
            |state, val| state.volume = val,
        );

        view.switch_bound::<FormState>(
            1003,
            SwitchDesc::labeled("启用", state.enabled),
            |state, on| state.enabled = on,
        );
    }
}

#[test]
fn test_text_input_bound_automatic_sync() {
    let mut runtime = AppRuntime::new(FormApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    assert_eq!(runtime.state().username, "Alice");

    // 聚焦文本框（点击在文本末尾）并输入字符
    let _ = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(100.0, 10.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1001),
    });

    let dirty = runtime.dispatch(AppEvent::Text {
        text: " in Wonderland".to_owned(),
    });

    // 校验：无需 reduce() 中任何手动赋值，应用状态已自动更新！
    assert!(dirty.contains(DirtyFlags::ALL));
    assert_eq!(runtime.state().username, "Alice in Wonderland");
}

#[test]
fn test_slider_bound_automatic_sync() {
    let mut runtime = AppRuntime::new(FormApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    assert_eq!(runtime.state().volume, 50.0);

    // 模拟点击滑块特定位置
    let dirty = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(80.0, 50.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1002),
    });

    assert!(dirty.contains(DirtyFlags::ALL));
    assert_ne!(runtime.state().volume, 50.0);
}

#[test]
fn test_switch_bound_automatic_sync() {
    let mut runtime = AppRuntime::new(FormApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    assert_eq!(runtime.state().enabled, false);

    // 模拟点击开关
    let dirty = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(10.0, 100.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1003),
    });

    assert!(dirty.contains(DirtyFlags::ALL));
    assert_eq!(runtime.state().enabled, true);
}

#[test]
fn test_controlled_text_sync_preserving_cursor() {
    let mut runtime = AppRuntime::new(FormApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 外部修改业务状态
    runtime.state_mut().username = "Bob The Builder".to_owned();
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 验证局部 TextEditState 已被 TextInputDesc::sync_state 平滑同步
    assert_eq!(
        runtime.context().text_input_value(1001),
        Some("Bob The Builder".to_owned())
    );
}

#[test]
fn test_app_context_sync_helpers() {
    let mut runtime = AppRuntime::new(FormApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 聚焦输入框
    let _ = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(10.0, 10.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1001),
    });

    let mut local_text = "Old".to_owned();
    let changed = runtime.context().sync_text(1001, &mut local_text);
    assert!(changed);
    assert_eq!(local_text, "Alice");

    // 未发生变化时返回 false
    assert!(!runtime.context().sync_text(1001, &mut local_text));
}

//! lang-allow: 中文注释与文档
//!
//! 分层键作用域（Key Scope）与类型化 Action 交互接缝单元与集成测试。

use unbound_app::prelude::*;
use unbound_math::Vec2;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CounterAction {
    Increment(u32),
    SelectTab(usize),
    Reset,
}

struct TestApp {
    count: u32,
    active_tab: usize,
    last_action: Option<CounterAction>,
}

impl Default for TestApp {
    fn default() -> Self {
        Self {
            count: 0,
            active_tab: 0,
            last_action: None,
        }
    }
}

impl Application for TestApp {
    type State = ();

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        event: AppEvent,
        ctx: &mut AppContext,
    ) -> EventOutcome {
        if let Some(action) = event.action::<CounterAction>(ctx) {
            self.last_action = Some(action.clone());
            match action {
                CounterAction::Increment(delta) => {
                    self.count += delta;
                    return EventOutcome::new(DirtyFlags::PAINT);
                }
                CounterAction::SelectTab(tab) => {
                    self.active_tab = tab;
                    return EventOutcome::new(DirtyFlags::PAINT);
                }
                CounterAction::Reset => {
                    self.count = 0;
                    return EventOutcome::new(DirtyFlags::PAINT);
                }
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.scope("tabs", |tabs_ui| {
            for idx in 0..3 {
                tabs_ui.button_action(
                    idx,
                    ButtonDesc::label(format!("Tab {idx}")),
                    CounterAction::SelectTab(idx),
                );
            }
        });

        view.scope("items", |items_ui| {
            for idx in 0..4 {
                items_ui.scope(idx, |row| {
                    row.button_action(
                        "inc",
                        ButtonDesc::label("+1"),
                        CounterAction::Increment((idx + 1) as u32),
                    );
                    row.button_action("reset", ButtonDesc::label("Reset"), CounterAction::Reset);
                });
            }
        });
    }
}

#[test]
fn test_key_scope_hierarchy_and_determinism() {
    let root = KeyScope::root();
    assert!(root.is_root());
    assert_eq!(root.key(100), 100);

    let scope_a = KeyScope::new("category_a");
    let scope_b = KeyScope::new("category_b");
    assert!(!scope_a.is_root());
    assert!(!scope_b.is_root());
    assert_ne!(scope_a.raw(), scope_b.raw());

    // 相同相对局部键在不同作用域下派生出互不相同的全局键
    let key_a1 = scope_a.key(1);
    let key_b1 = scope_b.key(1);
    assert_ne!(key_a1, key_b1);

    // 确定性：重复派生必须完全一致
    assert_eq!(key_a1, scope_a.key(1));
    assert_eq!(key_b1, scope_b.key(1));

    // 嵌套作用域测试
    let item_scope_0 = scope_a.scope(0);
    let item_scope_1 = scope_a.scope(1);
    assert_ne!(item_scope_0.raw(), item_scope_1.raw());
    assert_ne!(item_scope_0.key("btn"), item_scope_1.key("btn"));

    // 匹配助手与索引查找
    assert!(scope_a.matches(key_a1, 1));
    assert!(!scope_a.matches(key_a1, 2));
    assert_eq!(scope_a.find_index(key_a1, 10), Some(1));
    assert_eq!(scope_a.find_index(999_999, 10), None);
}

#[test]
fn test_runtime_action_dispatch_and_extraction() {
    let app = TestApp::default();
    let mut runtime = AppRuntime::new(app, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 验证作用域下生成的确定性键
    let tabs_scope = KeyScope::new("tabs");
    let tab_2_key = tabs_scope.key(2);

    // 模拟点击 Tab 2 对应的按钮
    let dirty = runtime.dispatch(AppEvent::Click {
        node: tab_2_key,
        pos: Vec2::new(10.0, 10.0),
        count: 1,
    });

    assert!(dirty.contains(DirtyFlags::PAINT));
    assert_eq!(runtime.app().active_tab, 2);
    assert_eq!(runtime.app().last_action, Some(CounterAction::SelectTab(2)));

    // 模拟点击 items[2] 的 "inc" 按钮：应增加 3
    let item_2_scope = KeyScope::new("items").scope(2);
    let inc_btn_key = item_2_scope.key("inc");

    let dirty = runtime.dispatch(AppEvent::Click {
        node: inc_btn_key,
        pos: Vec2::new(10.0, 10.0),
        count: 1,
    });

    assert!(dirty.contains(DirtyFlags::PAINT));
    assert_eq!(runtime.app().count, 3);
    assert_eq!(runtime.app().last_action, Some(CounterAction::Increment(3)));

    // 模拟点击 items[0] 的 "reset" 按钮：应归零
    let item_0_scope = KeyScope::new("items").scope(0);
    let reset_btn_key = item_0_scope.key("reset");

    let _ = runtime.dispatch(AppEvent::Click {
        node: reset_btn_key,
        pos: Vec2::new(10.0, 10.0),
        count: 1,
    });

    assert_eq!(runtime.app().count, 0);
    assert_eq!(runtime.app().last_action, Some(CounterAction::Reset));
}

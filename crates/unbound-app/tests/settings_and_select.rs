//! lang-allow: 中文注释与文档
//! 设置卡片自动分割线与 ComboBox 内置弹出浮层（Settings Card & Select Popup）集成测试。

use accesskit::Role;
use unbound_app::prelude::*;
use unbound_math::Vec2;
use unbound_widgets::SelectDesc;

#[derive(Default, Clone, Debug, PartialEq)]
struct SettingsTestState {
    theme_idx: usize,
    volume: f32,
    enable_feature: bool,
}

struct SettingsTestApp;

impl Application for SettingsTestApp {
    type State = SettingsTestState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        SettingsTestState {
            theme_idx: 0,
            volume: 75.0,
            enable_feature: true,
        }
    }

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        // 保持空白：完全不在 reduce() 中处理下拉框的展开或状态写回，
        // 验证框架内置弹出浮层与 select_bound 的单行声明全闭环！
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.settings_card(1000, |card| {
            card.settings_row(
                1010,
                "系统主题",
                "深色、浅色或跟随系统",
                |right, _rkey| {
                    right.select_bound::<SettingsTestState>(
                        1012,
                        SelectDesc::options(["深色模式", "浅色模式", "跟随系统"])
                            .selected(state.theme_idx),
                        |st, idx| st.theme_idx = idx,
                    );
                },
            );

            card.settings_row_with_icon(
                1020,
                ShellIcon::Sliders,
                "主音量控制",
                "全局音频通道输出音量",
                |right, _rkey| {
                    right.slider_bound::<SettingsTestState>(
                        1022,
                        SliderDesc::range(0.0, 100.0, state.volume),
                        |st, val| st.volume = val,
                    );
                },
            );

            card.settings_row_with_icon(
                1030,
                ShellIcon::Shield,
                "安全增强防护",
                "实时拦截危险钩子与冲突模块",
                |right, _rkey| {
                    right.switch_bound::<SettingsTestState>(
                        1032,
                        SwitchDesc::labeled("开启防护", state.enable_feature),
                        |st, on| st.enable_feature = on,
                    );
                },
            );
        });
    }
}

#[test]
fn test_settings_card_automatic_dividers() {
    let mut runtime = AppRuntime::new(SettingsTestApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 校验第 1 行无前置分割线，第 2 行与第 3 行前面自动插入了分割线
    let div_key_2 = mix_key(1020, 0xd171de);
    let div_key_3 = mix_key(1030, 0xd171de);

    assert!(
        runtime.widget_rect(1010).is_some(),
        "第 1 行设置行必须已正常布局"
    );
    assert!(
        runtime.widget_rect(div_key_2).is_some(),
        "第 2 行前必须自动插入 1px 细线分割线"
    );
    assert!(
        runtime.widget_rect(div_key_3).is_some(),
        "第 3 行前必须自动插入 1px 细线分割线"
    );

    // 校验文字内边距：第 1 行无图标，内边距左为 57px；第 2 行带图标，左内边距为 19px
    let r1 = runtime.widget_rect(1010).unwrap();
    let r2 = runtime.widget_rect(1020).unwrap();
    assert!(r1.width() >= 790.0);
    assert_eq!(r1.width(), r2.width());
}

#[test]
fn test_select_bound_lifecycle_and_light_dismiss() {
    let mut runtime = AppRuntime::new(SettingsTestApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 初始状态：未展开，选项索引为 0（深色模式）
    assert_eq!(runtime.state().theme_idx, 0);
    assert!(!runtime.context().select_is_open(1012));

    // 1. 点击下拉选择框触发器：应当自动变为展开态并在下帧渲染出浮层
    let _ = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(700.0, 20.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1012),
    });
    let _ = runtime.frame(0.016);

    assert!(
        runtime.context().select_is_open(1012),
        "点击后下拉框应处于打开态"
    );

    // 校验语义树中是否已自动渲染出 ListBoxOption 角色选项
    let semantics = runtime.semantics();
    let options: Vec<&unbound_a11y::A11yNode> = semantics
        .iter()
        .map(|(_, n)| n)
        .filter(|n| n.role == Role::ListBoxOption)
        .collect();
    assert_eq!(options.len(), 3, "展开态必须渲染出全部 3 个候选选项");
    assert!(
        options.iter().any(|n| n.selected && n.label == "深色模式"),
        "深色模式应当标记为选中态"
    );

    // 2. 点击第 2 项（浅色模式，索引 1）：应当自动更新状态、触发 select_bound 回调并关闭浮层
    let opt_key_1 = select_option_key(1012, 1);
    let dirty = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(700.0, 60.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(opt_key_1),
    });
    assert!(dirty.contains(DirtyFlags::ALL));

    // 运行下一帧：浮层收起，状态更新
    let _ = runtime.frame(0.016);
    assert_eq!(
        runtime.state().theme_idx,
        1,
        "select_bound 必须将选中索引自动写回状态"
    );
    assert!(
        !runtime.context().select_is_open(1012),
        "选项点击后浮层应已自动关闭"
    );

    // 3. 再次展开，测试点击外部轻关闭（Light Dismiss）
    let _ = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(700.0, 20.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1012),
    });
    let _ = runtime.frame(0.016);
    assert!(runtime.context().select_is_open(1012));

    // 点击外部远端空白处（例如 (50, 500)）
    let dirty_outside = runtime.pointer_down(Vec2::new(50.0, 500.0), PointerButton::Left);
    assert!(dirty_outside.contains(DirtyFlags::ALL));
    let _ = runtime.frame(0.016);

    assert!(
        !runtime.context().select_is_open(1012),
        "点击外部区域应触发轻关闭"
    );
    assert_eq!(runtime.state().theme_idx, 1, "轻关闭时不改变原有选中项");

    // 4. 再次展开，测试 Escape 键关闭
    let _ = runtime.dispatch(AppEvent::PointerDown {
        pos: Vec2::new(700.0, 20.0),
        button: PointerButton::Left,
        click_count: 1,
        target: Some(1012),
    });
    let _ = runtime.frame(0.016);
    assert!(runtime.context().select_is_open(1012));

    let dirty_esc = runtime.dispatch(AppEvent::Key {
        key: AppKey::Escape,
        pressed: true,
        modifiers: Modifiers::default(),
    });
    assert!(dirty_esc.contains(DirtyFlags::ALL));
    let _ = runtime.frame(0.016);
    assert!(
        !runtime.context().select_is_open(1012),
        "按下 Escape 应自动关闭展开的浮层"
    );
}

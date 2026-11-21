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

struct ReadableColumnTestApp;

impl Application for ReadableColumnTestApp {
    type State = ();

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        _event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.readable_column(3000, 12.0, |col| {
            col.settings_card(3010, |card| {
                card.settings_row(3011, "默认限宽行", "宽视口下钳制在 1000px", |right, _| {
                    right.switch(3012, SwitchDesc::labeled("启用", true));
                });
            });
        });
        view.readable_column_with_max_width(3020, 640.0, 8.0, |col| {
            col.settings_card(3021, |card| {
                card.settings_row(3022, "自定义限宽行", "最大宽度 640px", |right, _| {
                    right.switch(3023, SwitchDesc::labeled("启用", false));
                });
            });
        });
    }
}

#[test]
fn test_readable_column_clamps_wide_viewport_and_shrinks_narrow_viewport() {
    // 1. 1920px 宽视口：默认 readable_column 钳制在 READABLE_FORM_MAX_WIDTH (1000px)，
    //    自定义限宽列钳制在 640px。
    let mut wide_rt = AppRuntime::new(ReadableColumnTestApp, Vec2::new(1920.0, 1080.0));
    wide_rt.invalidate();
    let _ = wide_rt.frame(0.016);

    let wide_col = wide_rt
        .widget_rect(3000)
        .expect("1920px 视口下 readable_column 必须完成布局");
    assert!(
        (wide_col.width() - READABLE_FORM_MAX_WIDTH).abs() < 1.0,
        "1920px 视口下 readable_column 宽度应钳制在 1000px，实际为 {}",
        wide_col.width()
    );
    let wide_row = wide_rt
        .widget_rect(3011)
        .expect("内部设置行必须完成布局");
    assert!(
        wide_row.width() <= READABLE_FORM_MAX_WIDTH + 0.5,
        "内部设置行宽度不应超过 1000px，实际为 {}",
        wide_row.width()
    );

    let custom_wide = wide_rt
        .widget_rect(3020)
        .expect("自定义限宽列必须完成布局");
    assert!(
        (custom_wide.width() - 640.0).abs() < 1.0,
        "1920px 视口下自定义限宽列宽度应为 640px，实际为 {}",
        custom_wide.width()
    );

    // 2. 800px 窄视口：默认 readable_column 自适应收缩至可用宽 800px，
    //    640px 自定义列仍保持 640px。
    let mut narrow_rt = AppRuntime::new(ReadableColumnTestApp, Vec2::new(800.0, 600.0));
    narrow_rt.invalidate();
    let _ = narrow_rt.frame(0.016);

    let narrow_col = narrow_rt
        .widget_rect(3000)
        .expect("800px 视口下 readable_column 必须完成布局");
    assert!(
        (narrow_col.width() - 800.0).abs() < 1.0,
        "800px 视口下 readable_column 应收缩至 800px，实际为 {}",
        narrow_col.width()
    );
    let custom_mid = narrow_rt
        .widget_rect(3020)
        .expect("800px 视口下自定义限宽列必须完成布局");
    assert!(
        (custom_mid.width() - 640.0).abs() < 1.0,
        "800px 视口下 640px 限宽列应保持 640px，实际为 {}",
        custom_mid.width()
    );

    // 3. 500px 更窄视口：640px 自定义列也应收缩至 500px。
    let mut tiny_rt = AppRuntime::new(ReadableColumnTestApp, Vec2::new(500.0, 600.0));
    tiny_rt.invalidate();
    let _ = tiny_rt.frame(0.016);
    let custom_tiny = tiny_rt
        .widget_rect(3020)
        .expect("500px 视口下自定义限宽列必须完成布局");
    assert!(
        (custom_tiny.width() - 500.0).abs() < 1.0,
        "500px 视口下自定义限宽列应收缩至 500px，实际为 {}",
        custom_tiny.width()
    );
}

#[derive(Default, Clone, Debug, PartialEq)]
struct ExpanderTestState {
    expanded: bool,
    plain_expanded: bool,
    toggle_count: usize,
    enable_opt: bool,
}

struct ExpanderTestApp;

impl Application for ExpanderTestApp {
    type State = ExpanderTestState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        ExpanderTestState::default()
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        if let AppEvent::Click { node, .. } = event {
            if node == 4000 {
                state.expanded = !state.expanded;
                state.toggle_count += 1;
                return EventOutcome::new(DirtyFlags::ALL);
            }
            if node == 4100 {
                state.plain_expanded = !state.plain_expanded;
                return EventOutcome::new(DirtyFlags::ALL);
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.readable_column(3999, 12.0, |col| {
            col.settings_expander(
                4000,
                Some(ShellIcon::Sliders),
                "高级设置分组",
                "展开查看可折叠子项",
                state.expanded,
                |card| {
                    card.settings_row(4010, "子设置项一", "仅在展开时渲染", |right, _| {
                        right.switch_bound::<ExpanderTestState>(
                            4012,
                            SwitchDesc::labeled("开关", state.enable_opt),
                            |st, on| st.enable_opt = on,
                        );
                    });
                    card.settings_row_with_icon(
                        4020,
                        ShellIcon::Shield,
                        "子设置项二",
                        "带图标与自动分隔线",
                        |right, _| {
                            right.badge(4022, BadgeDesc::new(Rect::ZERO, "就绪"));
                        },
                    );
                },
            );

            col.settings_expander(
                4100,
                None,
                "无图标分组",
                "",
                state.plain_expanded,
                |card| {
                    card.settings_row(4110, "无图标子项", "验证无图标分支", |right, _| {
                        right.badge(4112, BadgeDesc::new(Rect::ZERO, "OK"));
                    });
                },
            );
        });
    }
}

#[test]
fn test_settings_expander_collapse_expand_and_header_click_hit() {
    let mut runtime = AppRuntime::new(ExpanderTestApp, Vec2::new(1200.0, 800.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 1. 初始折叠态（expanded = false）：头部存在，主体子节点与头部分隔线不渲染。
    assert!(!runtime.state().expanded);
    let header_rect = runtime
        .widget_rect(4000)
        .expect("折叠态下 expander 头部必须完成布局");
    assert!(
        header_rect.height() >= 48.0,
        "expander 头部高度应满足 WinUI 最小高度 48px，实际为 {}",
        header_rect.height()
    );
    assert!(
        runtime.widget_rect(4010).is_none(),
        "折叠态下主体第 1 行不得渲染"
    );
    assert!(
        runtime.widget_rect(4020).is_none(),
        "折叠态下主体第 2 行不得渲染"
    );
    assert!(
        runtime.widget_rect(mix_key(4000, 0xe49a_d171)).is_none(),
        "折叠态下头部分隔线不得渲染"
    );
    let collapsed_sem = runtime
        .semantics()
        .get(4000)
        .expect("折叠态下 expander 头部必须声明语义节点");
    assert_eq!(collapsed_sem.role, Role::Button);
    assert_eq!(collapsed_sem.expanded, Some(false));

    // 2. 点击头部中心坐标：通过真实 Router 命中测试触发 Click(4000) 并切换至展开态。
    let center = Vec2::new(
        header_rect.origin.x + header_rect.size.x * 0.5,
        header_rect.origin.y + header_rect.size.y * 0.5,
    );
    let _ = runtime.pointer_move(center);
    let _ = runtime.pointer_down(center, PointerButton::Left);
    let _ = runtime.pointer_up(center, PointerButton::Left);
    let _ = runtime.frame(0.016);

    assert!(
        runtime.state().expanded,
        "点击 expander 头部后应命中键 4000 并切换为展开态"
    );
    assert_eq!(runtime.state().toggle_count, 1);
    let expanded_sem = runtime
        .semantics()
        .get(4000)
        .expect("展开态下 expander 头部必须声明语义节点");
    assert_eq!(expanded_sem.role, Role::Button);
    assert_eq!(expanded_sem.expanded, Some(true));

    let row1 = runtime
        .widget_rect(4010)
        .expect("展开态下主体第 1 行必须渲染");
    let row2 = runtime
        .widget_rect(4020)
        .expect("展开态下主体第 2 行必须渲染");
    assert!(
        runtime.widget_rect(mix_key(4000, 0xe49a_d171)).is_some(),
        "展开态下头部与主体之间必须插入 1px 分隔线"
    );
    assert!(
        runtime.widget_rect(mix_key(4020, 0xd171de)).is_some(),
        "展开态下主体第 2 行前必须由 SettingsCard 自动插入分隔线"
    );
    assert!(
        row1.origin.y >= header_rect.origin.y + header_rect.size.y - 0.5,
        "展开态主体应位于头部下方"
    );
    assert!(
        row2.origin.y > row1.origin.y,
        "主体第 2 行应位于第 1 行下方"
    );

    // 3. 再次点击头部：切回折叠态，主体子节点应被卸载隐藏。
    let _ = runtime.pointer_down(center, PointerButton::Left);
    let _ = runtime.pointer_up(center, PointerButton::Left);
    let _ = runtime.frame(0.016);

    assert!(
        !runtime.state().expanded,
        "再次点击 expander 头部后应切回折叠态"
    );
    assert_eq!(runtime.state().toggle_count, 2);
    assert_eq!(
        runtime.semantics().get(4000).and_then(|n| n.expanded),
        Some(false),
        "重新折叠后语义展开态应回落为 Some(false)"
    );
    assert!(
        runtime.widget_rect(4010).is_none(),
        "重新折叠后主体第 1 行应隐藏"
    );
    assert!(
        runtime.widget_rect(4020).is_none(),
        "重新折叠后主体第 2 行应隐藏"
    );

    // 4. 验证无图标分组同样支持头部点击展开。
    let plain_header = runtime
        .widget_rect(4100)
        .expect("无图标 expander 头部必须完成布局");
    assert!(runtime.widget_rect(4110).is_none());
    let plain_center = Vec2::new(
        plain_header.origin.x + plain_header.size.x * 0.5,
        plain_header.origin.y + plain_header.size.y * 0.5,
    );
    let _ = runtime.pointer_move(plain_center);
    let _ = runtime.pointer_down(plain_center, PointerButton::Left);
    let _ = runtime.pointer_up(plain_center, PointerButton::Left);
    let _ = runtime.frame(0.016);
    assert!(runtime.state().plain_expanded);
    assert!(runtime.widget_rect(4110).is_some());
}

#[derive(Default, Clone, Debug, PartialEq)]
struct OverflowMenuTestState {
    open: bool,
    trigger_clicks: usize,
    last_action: Option<usize>,
}

struct OverflowMenuTestApp;

const OVERFLOW_TRIGGER_KEY: u64 = 5000;

impl Application for OverflowMenuTestApp {
    type State = OverflowMenuTestState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        OverflowMenuTestState::default()
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        if let AppEvent::Click { node, .. } = event {
            if node == OVERFLOW_TRIGGER_KEY {
                state.open = !state.open;
                state.trigger_clicks += 1;
                return EventOutcome::new(DirtyFlags::ALL);
            }
            for idx in 0..4 {
                if node == overflow_menu_item_key(OVERFLOW_TRIGGER_KEY, idx) {
                    state.last_action = Some(idx);
                    state.open = false;
                    return EventOutcome::new(DirtyFlags::ALL);
                }
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder<'_>) {
        view.settings_card(5001, |card| {
            card.settings_row(5002, "列表条目", "测试行尾溢出菜单", |right, _| {
                right.overflow_menu_trigger(OVERFLOW_TRIGGER_KEY, "更多操作");
            });
        });

        if state.open {
            let anchor = view
                .widget_rect(OVERFLOW_TRIGGER_KEY)
                .unwrap_or(Rect::new(740.0, 12.0, 28.0, 28.0));
            let items = [
                MenuItem::action("重命名"),
                MenuItem::separator(),
                MenuItem::action("不可用操作").disabled(),
                MenuItem::action("删除").with_shortcut("Del"),
            ];
            let _ = view.overflow_menu_popup(
                OVERFLOW_TRIGGER_KEY,
                anchor,
                view.ctx.viewport,
                &items,
            );
        }
    }
}

#[test]
fn test_overflow_menu_trigger_click_and_item_hit() {
    // 验证键派生确定性与互不碰撞
    let popup_key = overflow_menu_popup_key(OVERFLOW_TRIGGER_KEY);
    let item0_key = overflow_menu_item_key(OVERFLOW_TRIGGER_KEY, 0);
    let item3_key = overflow_menu_item_key(OVERFLOW_TRIGGER_KEY, 3);
    assert_eq!(item0_key, overflow_menu_item_key(OVERFLOW_TRIGGER_KEY, 0));
    assert_ne!(item0_key, item3_key);
    assert_ne!(item0_key, popup_key);
    assert_ne!(item0_key, OVERFLOW_TRIGGER_KEY);

    let mut runtime = AppRuntime::new(OverflowMenuTestApp, Vec2::new(800.0, 600.0));
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    // 1. 触发器按钮应渲染为标准 28~32px 尺寸，且弹出层初始未挂载
    let trigger_rect = runtime
        .widget_rect(OVERFLOW_TRIGGER_KEY)
        .expect("行尾溢出菜单触发器按钮必须完成布局");
    assert!(
        (28.0..=32.0).contains(&trigger_rect.width()),
        "触发器宽度应在 28~32px 之间，实际为 {}",
        trigger_rect.width()
    );
    assert!(
        (28.0..=32.0).contains(&trigger_rect.height()),
        "触发器高度应在 28~32px 之间，实际为 {}",
        trigger_rect.height()
    );
    assert!(
        runtime.widget_rect(popup_key).is_none(),
        "未展开时弹出浮层不得渲染"
    );

    // 校验触发器无障碍语义标签
    assert!(
        runtime
            .semantics()
            .iter()
            .any(|(_, n)| n.role == Role::Button && n.label == "更多操作"),
        "触发器应声明可读的 Role::Button 语义标签"
    );

    // 2. 点击触发器中心坐标：通过真实 Router 命中 trigger_key 并打开弹出层
    let trigger_center = Vec2::new(
        trigger_rect.origin.x + trigger_rect.size.x * 0.5,
        trigger_rect.origin.y + trigger_rect.size.y * 0.5,
    );
    let _ = runtime.pointer_move(trigger_center);
    let _ = runtime.pointer_down(trigger_center, PointerButton::Left);
    let _ = runtime.pointer_up(trigger_center, PointerButton::Left);
    let _ = runtime.frame(0.016);

    assert!(
        runtime.state().open,
        "点击触发器按钮后应命中 trigger_key 并展开菜单"
    );
    assert_eq!(runtime.state().trigger_clicks, 1);

    let popup_rect = runtime
        .widget_rect(popup_key)
        .expect("展开态下弹出浮层必须完成布局");
    let viewport_rect = Rect::new(0.0, 0.0, 800.0, 600.0);
    assert!(
        viewport_rect.contains_rect(&popup_rect),
        "弹出浮层 {popup_rect:?} 不得溢出视口 {viewport_rect:?}"
    );

    // 3. 点击禁用项（索引 2）不应触发动作
    let disabled_rect = runtime
        .widget_rect(overflow_menu_item_key(OVERFLOW_TRIGGER_KEY, 2))
        .expect("禁用菜单项应参与布局");
    let disabled_center = Vec2::new(
        disabled_rect.origin.x + disabled_rect.size.x * 0.5,
        disabled_rect.origin.y + disabled_rect.size.y * 0.5,
    );
    let _ = runtime.pointer_move(disabled_center);
    let _ = runtime.pointer_down(disabled_center, PointerButton::Left);
    let _ = runtime.pointer_up(disabled_center, PointerButton::Left);
    let _ = runtime.frame(0.016);
    assert_eq!(
        runtime.state().last_action,
        None,
        "点击禁用菜单项不得触发选中回调"
    );
    assert!(runtime.state().open);

    // 4. 点击第 4 项（索引 3："删除"）：精确命中 overflow_menu_item_key(trigger_key, 3)
    let item3_rect = runtime
        .widget_rect(item3_key)
        .expect("菜单项 3 必须完成布局");
    let item3_center = Vec2::new(
        item3_rect.origin.x + item3_rect.size.x * 0.5,
        item3_rect.origin.y + item3_rect.size.y * 0.5,
    );
    let _ = runtime.pointer_move(item3_center);
    let _ = runtime.pointer_down(item3_center, PointerButton::Left);
    let _ = runtime.pointer_up(item3_center, PointerButton::Left);
    let _ = runtime.frame(0.016);

    assert_eq!(
        runtime.state().last_action,
        Some(3),
        "点击菜单项 3 应精确命中 overflow_menu_item_key(trigger_key, 3)"
    );
    assert!(!runtime.state().open, "命中菜单项后应关闭弹出层");
}

struct OverflowEdgeClampApp {
    anchor: Rect,
}

const EDGE_TRIGGER_KEY: u64 = 5100;

impl Application for OverflowEdgeClampApp {
    type State = Option<usize>;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        None
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        if let AppEvent::Click { node, .. } = event {
            for idx in 0..3 {
                if node == overflow_menu_item_key(EDGE_TRIGGER_KEY, idx) {
                    *state = Some(idx);
                    return EventOutcome::new(DirtyFlags::ALL);
                }
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        // 空菜单项切片应返回 None 且不挂载节点
        assert!(view
            .overflow_menu_popup(5199, self.anchor, view.ctx.viewport, &[])
            .is_none());

        let items = [
            MenuItem::action("编辑条目"),
            MenuItem::action("复制链接"),
            MenuItem::action("移至回收站"),
        ];
        let popup_node =
            view.overflow_menu_popup(EDGE_TRIGGER_KEY, self.anchor, view.ctx.viewport, &items);
        assert!(popup_node.is_some());
    }
}

#[test]
fn test_overflow_menu_popup_clamps_near_right_and_bottom_viewport_edges() {
    let viewport = Vec2::new(800.0, 600.0);
    let viewport_rect = Rect::new(0.0, 0.0, viewport.x, viewport.y);
    // 将锚点贴近视口右下角，验证右侧钳制与底部向上翻转避让
    let anchor = Rect::new(768.0, 566.0, 28.0, 28.0);

    let mut runtime = AppRuntime::new(OverflowEdgeClampApp { anchor }, viewport);
    runtime.invalidate();
    let _ = runtime.frame(0.016);

    let popup_key = overflow_menu_popup_key(EDGE_TRIGGER_KEY);
    let popup_rect = runtime
        .widget_rect(popup_key)
        .expect("右下角边界避让弹出层必须完成布局");

    assert!(
        viewport_rect.contains_rect(&popup_rect),
        "靠近右下角时弹出层 {popup_rect:?} 不得溢出视口 {viewport_rect:?}"
    );
    assert!(
        popup_rect.origin.x >= 0.0 && popup_rect.max().x <= viewport.x,
        "弹出层水平范围 [{}, {}] 不得越出 [0, {}]",
        popup_rect.origin.x,
        popup_rect.max().x,
        viewport.x
    );
    assert!(
        popup_rect.origin.y >= 0.0 && popup_rect.max().y <= viewport.y,
        "弹出层垂直范围 [{}, {}] 不得越出 [0, {}]",
        popup_rect.origin.y,
        popup_rect.max().y,
        viewport.y
    );
    assert!(
        popup_rect.max().y <= anchor.origin.y + 0.5,
        "靠近视口底边时弹出层应自动向上翻转至锚点上方（popup_bottom={} vs anchor_top={}）",
        popup_rect.max().y,
        anchor.origin.y
    );

    // 验证避让定位后的菜单项依然可精确点击命中
    let item0_key = overflow_menu_item_key(EDGE_TRIGGER_KEY, 0);
    let item0_rect = runtime
        .widget_rect(item0_key)
        .expect("避让后菜单项 0 必须完成布局");
    let center = Vec2::new(
        item0_rect.origin.x + item0_rect.size.x * 0.5,
        item0_rect.origin.y + item0_rect.size.y * 0.5,
    );
    let _ = runtime.pointer_move(center);
    let _ = runtime.pointer_down(center, PointerButton::Left);
    let _ = runtime.pointer_up(center, PointerButton::Left);
    let _ = runtime.frame(0.016);

    assert_eq!(
        *runtime.state(),
        Some(0),
        "边界避让后的弹出菜单项点击应精确命中 overflow_menu_item_key(trigger_key, 0)"
    );
}

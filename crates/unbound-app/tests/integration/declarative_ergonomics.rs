//! 声明式易用性验收测试（Phase 3 / M9）。
//!
//! 验证内容：
//! 1. 多层嵌套布局（column -> row -> widgets）通过 Taffy 计算出有效世界坐标。
//! 2. 发出文本控件时，FrameReport 图集上传包含字形纹理更新。
//! 3. 分发指针与文本事件自动更新 WidgetStateStore 与脏标志。

use unbound_app::prelude::*;
use unbound_app::ImeUpdate;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_KEY: u64 = 1001;
const CHK_KEY: u64 = 1002;
const SWI_KEY: u64 = 1003;
const TXT_KEY: u64 = 1004;

// `ui_key!` 按源码行列生成键，`view()` 与断言各写一次会得到两个不同的键；
// 标题栏插槽需要在两处引用同一控件，故固定为显式常量。
const TB_SLOT_BASE: u64 = const_fnv1a_hash(file!(), 0, 0);
const TB_LEFT_BADGE: u64 = mix_key(TB_SLOT_BASE, 1);
const TB_CENTER_SEARCH: u64 = mix_key(TB_SLOT_BASE, 2);
const TB_ACTION_THEME: u64 = mix_key(TB_SLOT_BASE, 3);

#[derive(Default)]
struct DeclarativeApp;

struct DeclarativeAppState {
    counter: u32,
    /// 应用层收到的输入法事件计数；控件消费后这里必须保持为零。
    ime_events: u32,
}

impl Application for DeclarativeApp {
    type State = DeclarativeAppState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        DeclarativeAppState {
            counter: 0,
            ime_events: 0,
        }
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        match event {
            AppEvent::Click { node, .. } if node == BTN_KEY => {
                state.counter += 1;
                EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::LAYOUT)
            }
            AppEvent::Ime { .. } => {
                state.ime_events += 1;
                EventOutcome::new(DirtyFlags::PAINT)
            }
            _ => EventOutcome::new(DirtyFlags::NONE),
        }
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("main_col"), |col| {
            col.row(ui_key!("row1"), |row| {
                row.button(
                    BTN_KEY,
                    ButtonDesc::label("Click Me").variant(ButtonVariant::Primary),
                );
                row.checkbox(CHK_KEY, CheckboxDesc::labeled("Accept Terms", false));
            });

            col.switch(SWI_KEY, SwitchDesc::on(false));

            col.text_input(TXT_KEY, TextInputDesc::text("").placeholder("Type here..."));

            col.card(ui_key!("card"), CardDesc::default(), |card| {
                card.button(
                    ui_key!("counter_btn"),
                    ButtonDesc::label(format!("Count: {}", state.counter)),
                );
            });
        });
    }
}

#[test]
fn test_declarative_layout_and_world_coordinates() {
    let mut runtime = AppRuntime::new(DeclarativeApp, Vec2::new(800.0, 600.0));
    let report = runtime.frame(0.0);

    // 1. 场景应包含图元
    assert!(
        report.primitives > 0,
        "Scene primitives count should be non-zero"
    );

    // 2. 按钮、复选框与输入框均含文本，字形图集上传应非空
    assert!(
        !report.atlas_uploads.is_empty(),
        "FrameReport::atlas_uploads should contain glyph uploads for text widgets"
    );

    // 3. 验证已挂载控件拥有有效的世界坐标
    let btn_rect = runtime.widget_rect(BTN_KEY);
    let chk_rect = runtime.widget_rect(CHK_KEY);
    let swi_rect = runtime.widget_rect(SWI_KEY);
    let txt_rect = runtime.widget_rect(TXT_KEY);

    assert!(btn_rect.is_some(), "Button should be mounted");
    assert!(chk_rect.is_some(), "Checkbox should be mounted");
    assert!(swi_rect.is_some(), "Switch should be mounted");
    assert!(txt_rect.is_some(), "TextInput should be mounted");

    let btn = btn_rect.unwrap();
    let chk = chk_rect.unwrap();
    let swi = swi_rect.unwrap();
    let txt = txt_rect.unwrap();

    // 检查尺寸非零
    assert!(btn.width() > 0.0, "Button width must be positive");
    assert!(btn.height() > 0.0, "Button height must be positive");

    // 行布局：复选框应位于按钮右侧
    assert!(
        chk.min().x >= btn.max().x,
        "Checkbox should be to the right of button: chk.x={}, btn.max_x={}",
        chk.min().x,
        btn.max().x
    );

    // 列布局：开关应位于第一行下方
    assert!(
        swi.min().y >= btn.max().y,
        "Switch should be below button: swi.y={}, btn.max_y={}",
        swi.min().y,
        btn.max().y
    );

    // 列布局：文本输入框应位于开关下方
    assert!(
        txt.min().y >= swi.max().y,
        "TextInput should be below switch: txt.y={}, swi.max_y={}",
        txt.min().y,
        swi.max().y
    );
}

#[test]
fn test_automatic_widget_state_store_and_reduction() {
    let mut runtime = AppRuntime::new(DeclarativeApp, Vec2::new(800.0, 600.0));
    let _ = runtime.frame(0.0);

    let chk_rect = runtime.widget_rect(CHK_KEY).expect("Checkbox mounted");
    let swi_rect = runtime.widget_rect(SWI_KEY).expect("Switch mounted");
    let txt_rect = runtime.widget_rect(TXT_KEY).expect("TextInput mounted");

    // 1. WidgetStateStore 中的初始状态应为默认值（未勾选、关闭、空文本）
    assert_eq!(
        runtime.state_store().get::<CheckState>(CHK_KEY).copied(),
        Some(CheckState::Unchecked)
    );
    assert_eq!(
        runtime.state_store().get::<bool>(SWI_KEY).copied(),
        Some(false)
    );

    // 2. 点击复选框
    let chk_center = Vec2::new(chk_rect.min().x + 10.0, chk_rect.min().y + 10.0);
    runtime.pointer_move(chk_center);
    let dirty = runtime.pointer_down(chk_center, PointerButton::Left);
    assert!(dirty.contains(DirtyFlags::PAINT));
    let _ = runtime.pointer_up(chk_center, PointerButton::Left);

    // 复选框状态此时应切换为 Checked！
    assert_eq!(
        runtime.state_store().get::<CheckState>(CHK_KEY).copied(),
        Some(CheckState::Checked),
        "Checkbox should be checked after click"
    );

    // 3. 点击开关
    let swi_center = Vec2::new(swi_rect.min().x + 10.0, swi_rect.min().y + 10.0);
    runtime.pointer_move(swi_center);
    let dirty = runtime.pointer_down(swi_center, PointerButton::Left);
    assert!(dirty.contains(DirtyFlags::PAINT));
    let _ = runtime.pointer_up(swi_center, PointerButton::Left);

    // 开关状态此时应切换为 true！
    assert_eq!(
        runtime.state_store().get::<bool>(SWI_KEY).copied(),
        Some(true),
        "Switch should be on after click"
    );

    // 4. 点击文本输入框聚焦，然后发送文本输入
    let txt_center = Vec2::new(txt_rect.min().x + 20.0, txt_rect.min().y + 10.0);
    runtime.pointer_move(txt_center);
    let _ = runtime.pointer_down(txt_center, PointerButton::Left);
    let _ = runtime.pointer_up(txt_center, PointerButton::Left);

    // 发送字符串 "Hello"
    let _ = runtime.text("Hello");

    assert_eq!(
        runtime
            .state_store()
            .get::<TextEditState>(TXT_KEY)
            .map(|s| s.text().to_string()),
        Some("Hello".to_string()),
        "TextInput should contain typed text"
    );

    // 5. 测试按钮点击分发至 app.reduce
    let btn_rect = runtime.widget_rect(BTN_KEY).expect("Button mounted");
    let btn_center = Vec2::new(btn_rect.min().x + 10.0, btn_rect.min().y + 10.0);
    runtime.pointer_move(btn_center);
    let _ = runtime.pointer_down(btn_center, PointerButton::Left);
    let _ = runtime.pointer_up(btn_center, PointerButton::Left);

    // 重新渲染并断言应用计数器已更新
    let _ = runtime.frame(1.0 / 60.0);
    assert_eq!(
        runtime.state().counter,
        1,
        "App counter should increment after button click"
    );
}

#[test]
fn ime_updates_are_consumed_by_the_focused_text_input_exactly_once() {
    let mut runtime = AppRuntime::new(DeclarativeApp, Vec2::new(800.0, 600.0));
    let _ = runtime.frame(0.0);

    let txt_rect = runtime.widget_rect(TXT_KEY).expect("TextInput mounted");
    let txt_center = Vec2::new(txt_rect.min().x + 20.0, txt_rect.min().y + 10.0);
    runtime.pointer_move(txt_center);
    let _ = runtime.pointer_down(txt_center, PointerButton::Left);
    let _ = runtime.pointer_up(txt_center, PointerButton::Left);

    // 组合文本进入控件缓冲区，而不是以“待提交”形式滞留在平台层。
    let _ = runtime.ime(ImeUpdate {
        enabled: true,
        preedit: "nihao".into(),
        cursor_range: None,
        commit: None,
        cancelled: false,
    });
    assert_eq!(
        runtime
            .state_store()
            .get::<TextEditState>(TXT_KEY)
            .map(|s| s.text().to_string()),
        Some("nihao".to_string()),
        "the composition must be visible in the buffer"
    );

    // 提交替换组合区间：整条链路上恰好插入一次。
    let _ = runtime.ime(ImeUpdate {
        enabled: false,
        preedit: String::new(),
        cursor_range: None,
        commit: Some("nihao".into()),
        cancelled: false,
    });
    assert_eq!(
        runtime
            .state_store()
            .get::<TextEditState>(TXT_KEY)
            .map(|s| s.text().to_string()),
        Some("nihao".to_string()),
        "a commit must replace the composition instead of appending"
    );

    // 控件消费后应用层不得再收到同一次输入法更新。
    assert_eq!(runtime.state().ime_events, 0);
}

// ---------------------------------------------------------------------------
// 默认标题栏：与 showcase 顶栏一致，铺满视口且窗口按钮锚定右边缘。
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TitleBarApp;

impl Application for TitleBarApp {
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

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("title_bar_root"), |col| {
            col.title_bar("Title Bar Layout");
        });
    }
}

#[test]
fn default_title_bar_spans_viewport_and_anchors_window_buttons_right() {
    let mut runtime = AppRuntime::new(TitleBarApp, Vec2::new(900.0, 600.0));
    let _ = runtime.frame(0.0);

    let drag = runtime
        .widget_rect(TITLE_BAR_DRAG_ID)
        .expect("title bar drag surface must be mounted");
    assert!(
        (drag.size.x - 900.0).abs() < 0.01,
        "the title bar must span the viewport width, got {drag:?}"
    );
    assert!(
        (drag.size.y - TITLE_BAR_HEIGHT).abs() < 0.01,
        "the title bar height must come from the descriptor, got {drag:?}"
    );
    assert_eq!(drag.origin.y, 0.0);

    let close = runtime
        .widget_rect(TITLE_BAR_CLOSE_ID)
        .expect("close button must be mounted");
    let maximize = runtime
        .widget_rect(TITLE_BAR_MAXIMIZE_ID)
        .expect("maximize button must be mounted");
    let minimize = runtime
        .widget_rect(TITLE_BAR_MINIMIZE_ID)
        .expect("minimize button must be mounted");
    assert!(
        (close.max().x - 900.0).abs() <= 1.0,
        "the close button must hug the right edge, got {close:?}"
    );
    assert!(minimize.max().x <= maximize.origin.x + 0.01);
    assert!(maximize.max().x <= close.origin.x + 0.01);
    assert!(
        minimize.origin.x > 450.0,
        "the caption button chain must sit in the right half, got {minimize:?}"
    );

    let title = runtime
        .widget_rect(unbound_app::mix_key(TITLE_BAR_DRAG_ID, 3))
        .expect("title badge must be mounted");
    assert!(
        title.max().x < minimize.origin.x,
        "the title must not overlap caption buttons, got {title:?} / {minimize:?}"
    );
}

struct CustomTitleBarApp;

impl Application for CustomTitleBarApp {
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

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
        view.column(ui_key!("custom_tb_root"), |col| {
            col.title_bar_custom(
                TitleBarDesc::new("Custom Shell").height(48.0),
                TitleBarSlots::default()
                    .left(|l| {
                        l.badge(TB_LEFT_BADGE, BadgeDesc::label("Beta"));
                    })
                    .center(|c| {
                        c.badge(TB_CENTER_SEARCH, BadgeDesc::label("Search..."));
                    })
                    .actions(|a| {
                        a.button(TB_ACTION_THEME, ButtonDesc::label("Theme"));
                    }),
            );
        });
    }
}

#[test]
fn custom_title_bar_supports_slots_and_showcase_caption_width() {
    let mut runtime = AppRuntime::new(CustomTitleBarApp, Vec2::new(1000.0, 640.0));
    runtime.context_mut().window_maximized = true;
    let _ = runtime.frame(0.0);

    let close = runtime.widget_rect(TITLE_BAR_CLOSE_ID).expect("close");
    let max = runtime.widget_rect(TITLE_BAR_MAXIMIZE_ID).expect("max");
    let min = runtime.widget_rect(TITLE_BAR_MINIMIZE_ID).expect("min");
    assert!((close.size.x - TITLE_BAR_CAPTION_WIDTH).abs() < 0.01);
    assert!((max.size.x - TITLE_BAR_CAPTION_WIDTH).abs() < 0.01);
    assert!((min.size.x - TITLE_BAR_CAPTION_WIDTH).abs() < 0.01);

    let action = runtime.widget_rect(TB_ACTION_THEME).expect("custom action");
    assert!(
        action.max().x <= min.origin.x + 0.01,
        "custom action buttons must sit left of minimize: {action:?} / {min:?}"
    );
    assert!(runtime.widget_rect(TB_LEFT_BADGE).is_some());
    assert!(runtime.widget_rect(TB_CENTER_SEARCH).is_some());
}

// ---------------------------------------------------------------------------
// 循环内无参 `ui_key!()` 子树保留、滚动坐标同步、模态滚轮隔离与缩放上下文回归测试
// ---------------------------------------------------------------------------

struct LoopAndScrollProbe {
    progress: f32,
    show_modal: bool,
    observed_scale: f32,
    observed_osd: f32,
    clicked_row: Option<usize>,
}

impl Default for LoopAndScrollProbe {
    fn default() -> Self {
        Self {
            progress: 0.2,
            show_modal: false,
            observed_scale: 1.0,
            observed_osd: 0.0,
            clicked_row: None,
        }
    }
}

struct LoopAndScrollApp;

const LOOP_SCROLL_KEY: u64 = 77_001;
const LOOP_SLIDER_KEY: u64 = 77_002;
const LOOP_PROG_KEY: u64 = 77_003;
const LOOP_ROW_BTN_BASE: usize = 78_000;

impl Application for LoopAndScrollApp {
    type State = LoopAndScrollProbe;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        LoopAndScrollProbe::default()
    }

    fn reduce(
        &mut self,
        state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        if let AppEvent::Click { node, .. } = event {
            for idx in 0..8 {
                if node == ui_key!(LOOP_ROW_BTN_BASE + idx) {
                    state.clicked_row = Some(idx);
                    return EventOutcome::stopping(DirtyFlags::PAINT);
                }
            }
        }
        EventOutcome::new(DirtyFlags::NONE)
    }

    fn view(&mut self, state: &Self::State, view: &mut ViewBuilder) {
        let _ = (state.observed_scale, state.observed_osd);
        view.column(ui_key!("loop_scroll_root"), |root| {
            root.progress(LOOP_PROG_KEY, ProgressDesc::bar(state.progress));
            root.scroll_view_with(
                LOOP_SCROLL_KEY,
                RawNodeDesc::container()
                    .width(Dim::px(400.0))
                    .height(Dim::px(180.0)),
                true,
                |sv| {
                    for idx in 0..8 {
                        // 外层容器使用索引键，但内层容器与文本特意使用无参 `ui_key!()`，
                        // 验证同帧重复键去重不会误删前序循环项的子树。
                        sv.sunken_box(ui_key!(79_000usize + idx), 8.0, 4.0, |card| {
                            card.row_between(ui_key!(), |row| {
                                row.text_primary(ui_key!(), format!("Row {idx}"), 13.0, 500);
                                row.button(
                                    ui_key!(LOOP_ROW_BTN_BASE + idx),
                                    ButtonDesc::label("Run").size(ButtonSize::Sm),
                                );
                            });
                        });
                    }
                    sv.slider(LOOP_SLIDER_KEY, SliderDesc::range(0.0, 100.0, 10.0));
                },
            );

            if state.show_modal {
                root.modal_scrim(
                    ui_key!("test_modal_scrim"),
                    ui_key!("test_modal_dlg"),
                    260.0,
                    140.0,
                    |dlg| {
                        dlg.text_primary(ui_key!(), "Modal Body", 14.0, 600);
                    },
                );
            }
        });
    }
}

#[test]
fn loop_with_bare_ui_key_preserves_all_subtrees_and_button_rects() {
    let mut rt = AppRuntime::new(LoopAndScrollApp, Vec2::new(600.0, 400.0));
    let _ = rt.frame(0.0);

    let mut prev_y = -1.0_f32;
    for idx in 0..8 {
        let btn_rect = rt
            .widget_rect(ui_key!(LOOP_ROW_BTN_BASE + idx))
            .expect("every loop button must stay mounted in world");
        assert!(
            btn_rect.width() >= 26.0 && btn_rect.height() >= 24.0,
            "row {idx} button rect collapsed: {btn_rect:?}"
        );
        assert!(
            btn_rect.origin.y > prev_y,
            "row {idx} button y ({}) must increase monotonically after {}",
            btn_rect.origin.y,
            prev_y
        );
        prev_y = btn_rect.origin.y;
    }
}

#[test]
fn scrolled_scroll_view_translates_mounted_widget_rects_and_blocks_background_wheel_under_modal() {
    let mut rt = AppRuntime::new(LoopAndScrollApp, Vec2::new(600.0, 400.0));
    let _ = rt.frame(0.0);

    let before_btn0 = rt.widget_rect(ui_key!(LOOP_ROW_BTN_BASE)).unwrap();
    // 在滚动视口内向下滚动滚轮
    let _ = rt.wheel(Vec2::new(120.0, 100.0), Vec2::new(0.0, -2.0));
    let _ = rt.frame(0.016);
    let after_btn0 = rt.widget_rect(ui_key!(LOOP_ROW_BTN_BASE)).unwrap();
    assert!(
        after_btn0.origin.y < before_btn0.origin.y - 40.0,
        "mounted widget rect must translate with ScrollView offset: before={before_btn0:?}, after={after_btn0:?}"
    );

    // 开启模态遮罩后，在模态框上方滚动滚轮不得继续滚动背景 ScrollView
    let offset_before_modal = rt
        .state_store()
        .get::<unbound_widgets::ScrollState>(LOOP_SCROLL_KEY)
        .unwrap()
        .offset();
    rt.state_mut().show_modal = true;
    rt.invalidate();
    let _ = rt.frame(0.016);
    let _ = rt.wheel(Vec2::new(300.0, 200.0), Vec2::new(0.0, -3.0));
    let _ = rt.frame(0.016);
    let offset_after_modal = rt
        .state_store()
        .get::<unbound_widgets::ScrollState>(LOOP_SCROLL_KEY)
        .unwrap()
        .offset();
    assert_eq!(
        offset_before_modal, offset_after_modal,
        "wheel over a modal portal must not scroll background ScrollView"
    );
}

#[test]
fn progress_syncs_state_across_frames_and_context_exposes_ui_scale() {
    let mut rt = AppRuntime::new(LoopAndScrollApp, Vec2::new(600.0, 400.0));
    let _ = rt.frame(0.0);
    let initial_prog = rt
        .state_store()
        .get::<unbound_widgets::ProgressState>(LOOP_PROG_KEY)
        .unwrap()
        .shown();
    assert!((initial_prog - 0.2).abs() < 1e-4);

    rt.state_mut().progress = 0.85;
    let snapped = rt.context_mut().set_ui_scale(1.5);
    assert!((snapped - 1.5).abs() < 1e-4);
    assert!(rt.context().scale_osd_remaining > 1.0);
    rt.invalidate();
    let _ = rt.frame(0.016);

    let updated_prog = rt
        .state_store()
        .get::<unbound_widgets::ProgressState>(LOOP_PROG_KEY)
        .unwrap()
        .shown();
    assert!(
        (updated_prog - 0.85).abs() < 1e-4,
        "ProgressDesc must sync external progress updates into WidgetStateStore"
    );
}

#[test]
fn ui_scale_and_os_scale_sync_viewport_immediately_and_keep_titlebar_buttons_in_bounds() {
    struct ScaleAndTitleBarApp;
    impl Application for ScaleAndTitleBarApp {
        type State = ();
        fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}
        fn reduce(
            &mut self,
            _state: &mut Self::State,
            event: AppEvent,
            ctx: &mut AppContext,
        ) -> EventOutcome {
            if let AppEvent::Key {
                key: AppKey::Character('='),
                pressed: true,
                modifiers,
            } = event
            {
                if modifiers.ctrl {
                    ctx.step_ui_scale(1);
                    return EventOutcome::stopping(DirtyFlags::ALL);
                }
            }
            EventOutcome::new(DirtyFlags::NONE)
        }
        fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder) {
            view.title_bar_custom(
                unbound_widgets::TitleBarDesc::new("Scale Test").height(46.0),
                unbound_app::TitleBarSlots::default()
                    .left(|left| {
                        left.badge(
                            ui_key!("left_badge"),
                            BadgeDesc::label("Session Online").primary(),
                        );
                    })
                    .center(|center| {
                        center.text_input(
                            ui_key!("center_search"),
                            unbound_widgets::TextInputDesc::text("")
                                .placeholder("Search shaders, textures, pipelines..."),
                        );
                    })
                    .actions(|actions| {
                        actions.button(
                            ui_key!("act_btn_1"),
                            ButtonDesc::label("Tasks (3)").size(ButtonSize::Sm),
                        );
                        actions.button(
                            ui_key!("act_btn_2"),
                            ButtonDesc::label("Floating").size(ButtonSize::Sm),
                        );
                        actions.button(
                            ui_key!("act_btn_3"),
                            ButtonDesc::label("Theme").size(ButtonSize::Sm),
                        );
                    }),
            );
        }
    }

    let mut rt = AppRuntime::new(ScaleAndTitleBarApp, Vec2::new(1440.0, 900.0));
    let _ = rt.frame(0.0);
    assert_eq!(rt.context().viewport, Vec2::new(1440.0, 900.0));

    // Ctrl + '=' in reduce must immediately recompute logical viewport in the SAME frame
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    let _ = rt.key(AppKey::Character('='), true, ctrl);
    let _ = rt.frame(0.016);
    assert!((rt.context().ui_scale - 1.125).abs() < 1e-4);
    assert!(
        (rt.context().viewport.x - 1280.0).abs() < 0.5
            && (rt.context().viewport.y - 800.0).abs() < 0.5,
        "viewport must update in the same frame after step_ui_scale: {:?}",
        rt.context().viewport
    );

    // Verify 200% zoom (ui_scale = 2.0) keeps caption buttons inside logical viewport [0, 720]
    rt.context_mut().set_ui_scale(2.0);
    let _ = rt.frame(0.016);
    assert!((rt.context().viewport.x - 720.0).abs() < 0.5);
    let close_rect = rt
        .widget_rect(unbound_widgets::TITLE_BAR_CLOSE_ID)
        .expect("close button mounted");
    let min_rect = rt
        .widget_rect(unbound_widgets::TITLE_BAR_MINIMIZE_ID)
        .expect("minimize button mounted");
    assert!(
        close_rect.max().x <= rt.context().viewport.x + 0.5,
        "close button must not overflow right edge at 200% zoom: close={close_rect:?}, viewport={:?}",
        rt.context().viewport
    );
    assert!(
        min_rect.origin.x >= 68.0,
        "caption buttons must leave room for left slot at 200% zoom: min={min_rect:?}"
    );

    // Verify OS DPI + UI scale combined via sync_physical_viewport
    let dirty = rt.sync_physical_viewport(Vec2::new(1920.0, 1080.0), 1.5);
    assert!(dirty.needs_rebuild());
    let vp = rt.context().viewport;
    assert!((rt.context().effective_scale() - 3.0).abs() < 1e-4);
    assert!((vp.x - 640.0).abs() < 0.5 && (vp.y - 360.0).abs() < 0.5);
}

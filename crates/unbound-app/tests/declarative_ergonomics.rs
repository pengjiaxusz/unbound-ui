//! 声明式易用性验收测试（Phase 3 / M9）。
//!
//! 验证内容：
//! 1. 多层嵌套布局（column -> row -> widgets）通过 Taffy 计算出有效世界坐标。
//! 2. 发出文本控件时，FrameReport 图集上传包含字形纹理更新。
//! 3. 分发指针与文本事件自动更新 WidgetStateStore 与脏标志。

use unbound_app::prelude::*;
use unbound_input::PointerButton;
use unbound_math::Vec2;

const BTN_KEY: u64 = 1001;
const CHK_KEY: u64 = 1002;
const SWI_KEY: u64 = 1003;
const TXT_KEY: u64 = 1004;

#[derive(Default)]
struct DeclarativeApp;

struct DeclarativeAppState {
    counter: u32,
}

impl Application for DeclarativeApp {
    type State = DeclarativeAppState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        DeclarativeAppState { counter: 0 }
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
        .expect("标题栏拖拽/背景控件应已挂载");
    assert!(
        (drag.size.x - 900.0).abs() < 0.01,
        "标题栏必须铺满视口宽度，实际 {drag:?}"
    );
    assert!(
        (drag.size.y - TITLE_BAR_HEIGHT).abs() < 0.01,
        "标题栏高度应来自描述符，实际 {drag:?}"
    );
    assert_eq!(drag.origin.y, 0.0);

    let close = runtime
        .widget_rect(TITLE_BAR_CLOSE_ID)
        .expect("关闭按钮应已挂载");
    let maximize = runtime
        .widget_rect(TITLE_BAR_MAXIMIZE_ID)
        .expect("最大化按钮应已挂载");
    let minimize = runtime
        .widget_rect(TITLE_BAR_MINIMIZE_ID)
        .expect("最小化按钮应已挂载");
    assert!(
        (close.max().x - 900.0).abs() <= 1.0,
        "关闭按钮应贴右边缘，实际 {close:?}"
    );
    assert!(minimize.max().x <= maximize.origin.x + 0.01);
    assert!(maximize.max().x <= close.origin.x + 0.01);
    assert!(
        minimize.origin.x > 450.0,
        "窗口按钮链应位于右半区，实际 {minimize:?}"
    );

    let title = runtime
        .widget_rect(TITLE_BAR_DRAG_ID.wrapping_add(10))
        .expect("标题徽标应已挂载");
    assert!(
        title.max().x < minimize.origin.x,
        "标题不得与窗口按钮重叠，实际 {title:?} / {minimize:?}"
    );
}

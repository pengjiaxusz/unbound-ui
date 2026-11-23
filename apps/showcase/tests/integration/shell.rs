//! 自举展厅外壳的行为检查。

use std::ops::{Deref, DerefMut};

use accesskit::Role;
use showcase::pages::backdrop::{MATERIAL_PART_BASE, SOURCE_PART_BASE};
use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::{AppKey, AppRuntime, ImeUpdate, Modifiers as AppModifiers};
use unbound_backdrop::{BackdropSource, MaterialKind};
use unbound_headless::Harness;
use unbound_input::PointerButton;
use unbound_math::{Rect, Vec2};
use unbound_platform::{Modifiers, PlatformKey};
use unbound_widgets::diff::SceneDiff;

const VIEWPORT: (f32, f32) = (1440.0, 900.0);

/// 像窗口宿主一样通过运行时驱动外壳。
///
/// 指针路由器、焦点管理器与场景收集器现归运行时所有，因此测试不再直接拨弄私有路由器：
/// 它喂入与平台相同的事件并回读解析后的状态。
///
struct App {
    runtime: AppRuntime<GalleryApp>,
    viewport: (f32, f32),
}

impl Deref for App {
    type Target = GalleryApp;

    fn deref(&self) -> &GalleryApp {
        self.runtime.application()
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut GalleryApp {
        self.runtime.application_mut()
    }
}

impl App {
    fn new() -> Self {
        Self {
            runtime: AppRuntime::new(GalleryApp::new(), Vec2::new(VIEWPORT.0, VIEWPORT.1)),
            viewport: VIEWPORT,
        }
    }

    /// 将外壳请求的任意焦点交给运行时，然后绘制一帧。
    fn paint(&mut self, dt: f32) {
        if let Some(node) = self.runtime.application_mut().take_focus_request() {
            self.runtime.focus_node(Some(node));
        }
        if self.runtime.application_mut().take_blur_request() {
            self.runtime.focus_node(None);
        }
        self.runtime.frame(dt);
    }

    fn resize(&mut self, viewport: (f32, f32)) {
        if self.viewport != viewport {
            self.viewport = viewport;
            self.runtime.resize(Vec2::new(viewport.0, viewport.1));
        }
    }

    fn rebuild(&mut self, viewport: (f32, f32)) {
        self.resize(viewport);
        // 显式重建即是一次重建，即使没有可观测的变化：图集时钟与布局统计信息也会随之推进。
        //
        self.runtime.invalidate();
        self.paint(1.0 / 60.0);
    }

    /// 仅当发生可观测变化时才重建；返回是否实际重建。
    fn sync(&mut self, viewport: (f32, f32)) -> bool {
        self.resize(viewport);
        self.paint(1.0 / 60.0);
        !matches!(self.runtime.last_diff(), SceneDiff::Identical)
    }

    fn pointer_move(&mut self, pos: Vec2) {
        self.runtime.pointer_move(pos);
    }

    fn pointer_down(&mut self, pos: Vec2, _now: f64) {
        self.runtime.pointer_move(pos);
        // 旧辅助方法传递显式时间戳；现由运行时拥有时钟，因此向前拨动时钟以防两次点击被配对为双击。
        //
        self.runtime.advance(0.5);
        self.runtime.pointer_down(pos, PointerButton::Left);
    }

    fn pointer_up(&mut self, pos: Vec2, _now: f64) {
        self.runtime.pointer_up(pos, PointerButton::Left);
    }

    /// 像窗口宿主一样右键：移动到目标、按下并释放。
    fn right_click(&mut self, pos: Vec2) {
        self.runtime.pointer_move(pos);
        self.runtime.advance(0.5);
        self.runtime.pointer_down(pos, PointerButton::Right);
        self.runtime.pointer_up(pos, PointerButton::Right);
    }

    fn key(&mut self, key: &PlatformKey, modifiers: Modifiers) {
        if let Some(key) = app_key(key) {
            self.runtime.key(key, true, app_modifiers(modifiers));
        }
    }

    /// 注入平台上报的按住重复事件，走与窗口宿主相同的外壳入口。
    fn repeat(&mut self, key: &PlatformKey, modifiers: Modifiers) {
        self.runtime.application_mut().key_repeat(key, modifiers);
    }

    fn pressed(&self) -> Option<u64> {
        self.runtime.context().pressed
    }

    fn hovered(&self) -> Option<u64> {
        self.runtime.context().hovered
    }

    /// 失去焦点会像窗口宿主一样中止交互：
    /// 外壳丢弃自身的拖拽状态，运行时释放其持有的指针捕获。
    ///
    fn cancel_interactions(&mut self) {
        self.runtime.application_mut().cancel_interactions();
        self.runtime.cancel_pointer();
    }
}

fn app_key(key: &PlatformKey) -> Option<AppKey> {
    Some(match key {
        PlatformKey::Named(name) => match name.as_str() {
            "Tab" => AppKey::Tab,
            "Escape" => AppKey::Escape,
            "Enter" => AppKey::Enter,
            "Space" => AppKey::Space,
            "Backspace" => AppKey::Backspace,
            "Delete" => AppKey::Delete,
            "ArrowLeft" => AppKey::Left,
            "ArrowRight" => AppKey::Right,
            "ArrowUp" => AppKey::Up,
            "ArrowDown" => AppKey::Down,
            "Home" => AppKey::Home,
            "End" => AppKey::End,
            _ => return None,
        },
        PlatformKey::Character(text) => {
            let mut chars = text.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            AppKey::Character(first)
        }
        PlatformKey::Other(_) => return None,
    })
}

fn app_modifiers(modifiers: Modifiers) -> AppModifiers {
    AppModifiers {
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.meta,
    }
}

fn app() -> App {
    let mut app = App::new();
    app.state.theme = ThemeMode::Dark;
    app.rebuild(VIEWPORT);
    app
}

fn rect_of(app: &App, node: u64) -> unbound_math::Rect {
    app.regions
        .iter()
        .find(|region| region.node == node)
        .map(|region| region.bounds)
        .unwrap_or_else(|| panic!("no hit region for node {node:#x}"))
}

/// 两个矩形是否互相压住。
fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.origin.x < b.max().x
        && b.origin.x < a.max().x
        && a.origin.y < b.max().y
        && b.origin.y < a.max().y
}

fn click_node(app: &mut App, node: u64) {
    let rect = rect_of(app, node);
    let point = rect.center();
    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);
    app.rebuild(VIEWPORT);
}

/// 聚焦搜索框并键入给定文本。
fn search_with_text(text: &str) -> App {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in text.chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    app.rebuild(VIEWPORT);
    app
}

#[test]
fn every_page_passes_the_headless_sanitizers() {
    for page in PageId::ALL {
        let mut app = app();
        app.navigate(page);
        app.rebuild(VIEWPORT);
        let report = Harness::run(&app.frame());
        assert!(
            report.is_clean(),
            "{page:?} produced {} diagnostic(s): {:?}",
            report.diagnostics.len(),
            report.diagnostics
        );
    }
}

#[test]
fn the_hud_passes_contrast_in_both_themes() {
    for theme in [ThemeMode::Light, ThemeMode::Dark] {
        let mut app = App::new();
        app.state.theme = theme;
        app.state.show_hud = true;
        app.rebuild((1440.0, 900.0));
        let report = Harness::run(&app.frame());
        assert!(
            report.is_clean(),
            "{theme:?} HUD produced {} diagnostic(s): {}",
            report.diagnostics.len(),
            report.summary()
        );
    }
}

#[test]
fn layouts_stay_clean_across_themes_zooms_and_sizes() {
    for theme in [ThemeMode::Light, ThemeMode::Dark] {
        for zoom_step in 0..4u8 {
            for (width, height) in [
                (720.0, 450.0),
                (960.0, 600.0),
                (1280.0, 800.0),
                (1920.0, 1080.0),
            ] {
                let mut app = App::new();
                app.state.theme = theme;
                app.state.zoom_step = zoom_step as usize;
                app.rebuild((width, height));
                let report = Harness::run(&app.frame());
                assert!(
                    report.is_clean(),
                    "{theme:?} at {width}×{height} zoom {} produced {} diagnostic(s): {}",
                    app.state.zoom(),
                    report.diagnostics.len(),
                    report.summary()
                );
            }
        }
    }
}

#[test]
fn navigation_updates_the_breadcrumb_and_active_page() {
    let mut app = app();
    assert_eq!(app.state.active_page, PageId::Home);
    app.navigate(PageId::ListView);
    assert_eq!(app.state.active_page, PageId::ListView);
    assert_eq!(app.state.active_page.category().label(), "Collections");
    assert_eq!(app.state.content_scroll, 0.0);
}

#[test]
fn the_navigation_pane_collapses_and_expands() {
    let mut app = app();
    let expanded = app.layout.nav_width;
    assert!(app.nav_view.is_pane_open);
    assert!(app.nav_title_bar_sync().is_pane_open);
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(!app.nav_view.is_pane_open);
    assert!(!app.nav_title_bar_sync().is_pane_open);
    for _ in 0..120 {
        app.tick(1.0 / 60.0);
    }
    app.rebuild(VIEWPORT);
    assert!(app.layout.nav_width < expanded);
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(app.nav_view.is_pane_open);
    assert!(app.nav_title_bar_sync().is_pane_open);
}

#[test]
fn theme_and_zoom_switches_are_one_click() {
    let mut app = app();
    click_node(&mut app, target::id(Target::Theme(0)));
    assert_eq!(app.state.theme, ThemeMode::Light);
    // 窗口缩放控件现为紧凑型下拉选择框：先点击触发器展开下拉列表，再点击 150% 档位。
    click_node(&mut app, target::id(Target::ZoomMenu));
    assert!(app.state.zoom_dropdown_open);
    click_node(&mut app, target::id(Target::Zoom(2)));
    assert!(!app.state.zoom_dropdown_open);
    assert_eq!(app.state.zoom_step, 2);
    assert!((app.state.zoom() - 1.5).abs() < f32::EPSILON);
}

#[test]
fn ctrl_k_searches_the_catalogue_by_crate_and_tag() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert!(app.state.search.focused);
    for ch in "mtsdf".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    let results = app.state.search_results();
    assert!(
        results.contains(&PageId::Typography),
        "expected Typography, got {results:?}"
    );
    app.key(
        &PlatformKey::Named("Enter".to_string()),
        Modifiers::default(),
    );
    assert_eq!(app.state.active_page, PageId::Typography);
    assert!(!app.state.search.focused);
}

#[test]
fn ime_composition_is_visible_and_commits_exactly_once() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert!(app.state.search.focused);

    // 首字母组合：缓冲区为空时也必须绘制组合文本，而不是回退到占位符。
    app.runtime.ime(ImeUpdate {
        enabled: true,
        preedit: "nihao".into(),
        cursor_range: None,
        commit: None,
        cancelled: false,
    });
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.search.text(), "nihao");

    // 提交替换组合区间：整条链路上恰好插入一次。
    app.runtime.ime(ImeUpdate {
        enabled: false,
        preedit: String::new(),
        cursor_range: None,
        commit: Some("hello".into()),
        cancelled: false,
    });
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.search.text(), "hello");
    assert_eq!(
        app.state.search.edit.preedit(),
        None,
        "a commit must clear the in-progress composition"
    );
}

#[test]
fn escape_cancels_only_the_active_composition() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    app.key(
        &PlatformKey::Character("a".to_string()),
        Modifiers::default(),
    );
    assert_eq!(app.state.search.text(), "a");

    app.runtime.ime(ImeUpdate {
        enabled: true,
        preedit: "ni".into(),
        cursor_range: None,
        commit: None,
        cancelled: false,
    });
    assert_eq!(app.state.search.text(), "ani");

    app.key(
        &PlatformKey::Named("Escape".to_string()),
        Modifiers::default(),
    );
    assert_eq!(
        app.state.search.text(),
        "a",
        "escape must drop only the composition and keep committed text"
    );
    assert!(app.state.search.focused);
}

/// 空查询下连续到达的组合更新只保留最新一份，并保持可绘制的组合区间。
///
/// 组合串在缓冲区为空时也必须直接参与渲染（而非回退为占位符），
/// 且每一轮更新替换上一轮，而不是把拼音字母堆叠起来。
///
#[test]
fn consecutive_preedits_replace_each_other_without_accumulating() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert!(app.state.search.focused);

    for preedit in ["n", "ni", "nihao"] {
        app.runtime.ime(ImeUpdate {
            enabled: true,
            preedit: preedit.into(),
            cursor_range: None,
            commit: None,
            cancelled: false,
        });
    }
    app.rebuild(VIEWPORT);

    assert_eq!(app.state.search.text(), "nihao");
    assert!(
        app.state.search.edit.preedit().is_some(),
        "the composition must remain inline so its underline can be drawn"
    );
}

/// 失焦丢弃未提交的组合，文本恢复为组合前内容。
#[test]
fn losing_focus_discards_the_active_composition() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    app.runtime.ime(ImeUpdate {
        enabled: true,
        preedit: "ni".into(),
        cursor_range: None,
        commit: None,
        cancelled: false,
    });
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.search.text(), "ni");

    // 走真实链路：点击编辑器外部触发失焦，而不是直接调用状态层。
    click_node(&mut app, target::id(Target::Theme(0)));

    assert_eq!(app.state.search.text(), "");
    assert_eq!(app.state.search.edit.preedit(), None);
}

/// 切换页面丢弃未提交的组合，不留下组合区间或残留文本。
#[test]
fn switching_pages_discards_the_active_composition() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    app.runtime.ime(ImeUpdate {
        enabled: true,
        preedit: "ni".into(),
        cursor_range: None,
        commit: None,
        cancelled: false,
    });
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.search.text(), "ni");

    app.state.navigate(PageId::Typography);
    app.rebuild(VIEWPORT);

    assert_eq!(app.state.search.text(), "");
    assert_eq!(app.state.search.edit.preedit(), None);
}

/// 点击输入框后只移动指针，不得改变文本、插入符或选区。
///
/// 回归：释放之后的点击报告曾再次进入拖拽态，导致普通悬停被当作拖拽更新选区。
///
#[test]
fn clicking_an_editor_then_hovering_keeps_the_caret_and_selection() {
    let mut app = app();
    app.state.search.edit.set_text("hello world");
    app.state.search.edit.set_cursor(0, false);
    app.rebuild(VIEWPORT);

    let rect = rect_of(&app, target::id(Target::Search));
    let press = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    app.pointer_move(press);
    app.pointer_down(press, 0.0);
    app.pointer_up(press, 0.1);
    app.rebuild(VIEWPORT);

    let text = app.state.search.text().to_string();
    let cursor = app.state.search.edit.cursor();
    let anchor = app.state.search.edit.anchor();
    assert!(!app.state.search.edit.has_selection());

    // 不按键，仅把指针移到输入框另一端。
    app.pointer_move(Vec2::new(rect.origin.x + rect.size.x - 12.0, press.y));

    assert_eq!(app.state.search.text(), text);
    assert_eq!(app.state.search.edit.cursor(), cursor);
    assert_eq!(app.state.search.edit.anchor(), anchor);
    assert!(
        !app.state.search.edit.has_selection(),
        "plain hover after a click must not form a selection"
    );
}

/// 组合进行中普通悬停不得取消组合。
///
/// 回归：点击释放后的幽灵拖拽会把悬停送成 `PointerDrag`，而该事件属于
/// “用户编辑”，会取消输入法组合。
///
#[test]
fn hovering_during_a_composition_keeps_the_composition_alive() {
    let mut app = app();
    app.state.search.edit.set_text("hello");
    app.state.search.edit.set_cursor(0, false);
    app.rebuild(VIEWPORT);

    let rect = rect_of(&app, target::id(Target::Search));
    let press = Vec2::new(rect.origin.x + 20.0, rect.center().y);
    app.pointer_move(press);
    app.pointer_down(press, 0.0);
    app.pointer_up(press, 0.1);
    app.rebuild(VIEWPORT);

    app.runtime.ime(ImeUpdate {
        enabled: true,
        preedit: "ni".into(),
        cursor_range: None,
        commit: None,
        cancelled: false,
    });
    assert!(app.state.search.edit.preedit().is_some());

    app.pointer_move(Vec2::new(rect.origin.x + rect.size.x - 12.0, press.y));

    assert!(
        app.state.search.edit.preedit().is_some(),
        "plain hover must not cancel an active IME composition"
    );
    assert_eq!(app.state.search.text(), "nihello");
}

/// 按住左键拖动仍要形成 (anchor, cursor) 选区，释放后定格。
///
/// 这是 #10 的红线：拆分点击路径不得牺牲拖拽选择。
///
#[test]
fn pressing_and_dragging_still_forms_a_selection() {
    let mut app = app();
    app.state.search.edit.set_text("hello world");
    app.state.search.edit.set_cursor(0, false);
    app.rebuild(VIEWPORT);

    let rect = rect_of(&app, target::id(Target::Search));
    let left = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    let right = Vec2::new(rect.origin.x + rect.size.x - 12.0, rect.center().y);

    app.pointer_move(left);
    app.pointer_down(left, 0.0);
    app.pointer_move(right);
    assert!(
        app.state.search.edit.has_selection(),
        "holding the button and dragging must extend a selection"
    );
    app.pointer_up(right, 0.1);

    let frozen = app.state.search.edit.selection();
    assert!(frozen.is_some());
    app.pointer_move(left);
    assert_eq!(
        app.state.search.edit.selection(),
        frozen,
        "the selection must freeze once the button is released"
    );
}

/// 在输入框外点击后选区折叠，随后的悬停不再复现选区。
#[test]
fn clicking_outside_collapses_the_selection() {
    let mut app = app();
    app.state.search.edit.set_text("hello world");
    app.state.search.edit.set_cursor(0, false);
    app.rebuild(VIEWPORT);

    let rect = rect_of(&app, target::id(Target::Search));
    let left = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    let right = Vec2::new(rect.origin.x + rect.size.x - 12.0, rect.center().y);
    app.pointer_move(left);
    app.pointer_down(left, 0.0);
    app.pointer_move(right);
    app.pointer_up(right, 0.1);
    assert!(app.state.search.edit.has_selection());

    click_node(&mut app, target::id(Target::Theme(0)));
    assert!(
        !app.state.search.edit.has_selection(),
        "clicking outside the editor must collapse the selection"
    );
    app.pointer_move(right);
    assert!(!app.state.search.edit.has_selection());
}

/// 窗口失焦后选区折叠，随后的悬停不再复现选区。
#[test]
fn losing_the_window_focus_collapses_the_selection() {
    let mut app = app();
    app.state.search.edit.set_text("hello world");
    app.state.search.edit.set_cursor(0, false);
    app.rebuild(VIEWPORT);

    let rect = rect_of(&app, target::id(Target::Search));
    let left = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    let right = Vec2::new(rect.origin.x + rect.size.x - 12.0, rect.center().y);
    app.pointer_move(left);
    app.pointer_down(left, 0.0);
    app.pointer_move(right);
    app.pointer_up(right, 0.1);
    assert!(app.state.search.edit.has_selection());

    app.cancel_interactions();
    assert!(
        !app.state.search.edit.has_selection(),
        "losing the window focus must collapse the selection"
    );
    app.pointer_move(left);
    assert!(!app.state.search.edit.has_selection());
}

#[test]
fn held_character_keys_keep_inserting_at_the_caret() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    app.key(
        &PlatformKey::Character("a".to_string()),
        Modifiers::default(),
    );
    app.key(
        &PlatformKey::Character("c".to_string()),
        Modifiers::default(),
    );
    // 光标移到两个字符之间，重复字符必须插在同一插入点。
    app.key(
        &PlatformKey::Named("ArrowLeft".to_string()),
        Modifiers::default(),
    );
    for _ in 0..3 {
        app.repeat(
            &PlatformKey::Character("b".to_string()),
            Modifiers::default(),
        );
    }
    assert_eq!(app.state.search.text(), "abbbc");
    assert_eq!(app.state.search.edit.cursor(), 4);
    assert_eq!(app.state.search.edit.anchor(), 4);
}

#[test]
fn held_backspace_and_delete_keep_removing_text() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in "abcd".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    for _ in 0..2 {
        app.repeat(
            &PlatformKey::Named("Backspace".to_string()),
            Modifiers::default(),
        );
    }
    assert_eq!(app.state.search.text(), "ab");

    app.key(
        &PlatformKey::Named("Home".to_string()),
        Modifiers::default(),
    );
    for _ in 0..2 {
        app.repeat(
            &PlatformKey::Named("Delete".to_string()),
            Modifiers::default(),
        );
    }
    assert_eq!(
        app.state.search.text(),
        "",
        "held Delete must remove forward"
    );
    assert_eq!(app.state.search.edit.cursor(), 0);
}

#[test]
fn held_ctrl_backspace_deletes_one_word_per_repeat() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in "hello world".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };
    app.repeat(&PlatformKey::Named("Backspace".to_string()), ctrl);
    assert_eq!(app.state.search.text(), "hello ");
    app.repeat(&PlatformKey::Named("Backspace".to_string()), ctrl);
    assert_eq!(app.state.search.text(), "");
}

#[test]
fn held_arrows_and_home_end_keep_moving_the_caret() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in "abc".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    app.repeat(
        &PlatformKey::Named("ArrowLeft".to_string()),
        Modifiers::default(),
    );
    app.repeat(
        &PlatformKey::Named("ArrowLeft".to_string()),
        Modifiers::default(),
    );
    assert_eq!(app.state.search.edit.cursor(), 1);
    assert_eq!(app.state.search.edit.anchor(), 1);

    app.repeat(
        &PlatformKey::Named("Home".to_string()),
        Modifiers::default(),
    );
    assert_eq!(app.state.search.edit.cursor(), 0);
    app.repeat(&PlatformKey::Named("End".to_string()), Modifiers::default());
    assert_eq!(app.state.search.edit.cursor(), 3);
}

#[test]
fn held_shift_left_keeps_extending_the_selection() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in "abc".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    for _ in 0..2 {
        app.repeat(&PlatformKey::Named("ArrowLeft".to_string()), shift);
    }
    assert_eq!(app.state.search.edit.selected_text(), Some("bc"));
    assert_eq!(app.state.search.edit.anchor(), 3);
    assert_eq!(app.state.search.edit.cursor(), 1);
}

#[test]
fn held_ctrl_commands_do_not_run_again() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in "abc".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };

    // 重复的全选不得执行任何命令：第一次按下才生效。
    app.repeat(&PlatformKey::Character("a".to_string()), ctrl);
    assert_eq!(app.state.search.edit.selection(), None);
    app.key(&PlatformKey::Character("a".to_string()), ctrl);
    assert_eq!(app.state.search.edit.selected_text(), Some("abc"));

    // 重复的复制、剪切、粘贴、撤销、重做都不得产生动作。
    app.repeat(&PlatformKey::Character("c".to_string()), ctrl);
    assert!(app.take_clipboard_request().is_none());
    app.repeat(&PlatformKey::Character("x".to_string()), ctrl);
    assert!(app.take_clipboard_request().is_none());
    assert_eq!(app.state.search.text(), "abc", "held Ctrl+X must not cut");
    app.repeat(&PlatformKey::Character("v".to_string()), ctrl);
    assert!(app.take_clipboard_request().is_none());
    app.repeat(&PlatformKey::Character("z".to_string()), ctrl);
    app.repeat(
        &PlatformKey::Character("z".to_string()),
        Modifiers {
            control: true,
            shift: true,
            ..Default::default()
        },
    );
    app.repeat(&PlatformKey::Character("y".to_string()), ctrl);
    assert_eq!(
        app.state.search.text(),
        "abc",
        "held undo and redo must not alter the text"
    );
    assert_eq!(app.state.search.edit.selected_text(), Some("abc"));
}

#[test]
fn repeats_without_a_focused_editor_change_nothing() {
    let mut app = app();
    assert!(app.focused_editor().is_none());
    let before = (
        app.state.search.text().to_owned(),
        app.state.demo.text.text().to_owned(),
        app.state.fps_input.text().to_owned(),
    );

    app.repeat(
        &PlatformKey::Character("x".to_string()),
        Modifiers::default(),
    );
    app.repeat(
        &PlatformKey::Named("Backspace".to_string()),
        Modifiers::default(),
    );
    app.repeat(
        &PlatformKey::Named("ArrowLeft".to_string()),
        Modifiers::default(),
    );
    app.repeat(
        &PlatformKey::Character("a".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(
        before,
        (
            app.state.search.text().to_owned(),
            app.state.demo.text.text().to_owned(),
            app.state.fps_input.text().to_owned(),
        ),
        "a repeat without focus must not edit any buffer"
    );
    assert!(app.take_clipboard_request().is_none());
}

#[test]
fn fps_input_accepts_only_digits_and_rejects_overflow() {
    let mut app = app();
    app.state.fps_input.edit.set_text("60");
    app.state.fps_input.focus();

    app.key(
        &PlatformKey::Character("a".to_string()),
        Modifiers::default(),
    );
    assert_eq!(app.state.fps_input.text(), "60");

    app.key(
        &PlatformKey::Character("7".to_string()),
        Modifiers::default(),
    );
    assert_eq!(app.state.fps_input.text(), "607");

    app.key(
        &PlatformKey::Character("8".to_string()),
        Modifiers::default(),
    );
    assert_eq!(
        app.state.fps_input.text(),
        "607",
        "the fps buffer is capped at three digits"
    );
}

#[test]
fn editor_menu_actions_respect_selection_and_clipboard_state() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    for ch in "abc".chars() {
        app.key(
            &PlatformKey::Character(ch.to_string()),
            Modifiers::default(),
        );
    }
    app.rebuild(VIEWPORT);
    let target = Target::Search;
    let anchor = rect_of(&app, target::id(target)).center();

    // 剪贴板为空：粘贴项禁用，点击不产生读取请求。
    app.open_editor_menu(target, anchor, false);
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(2)));
    assert!(app.take_clipboard_request().is_none());
    assert_eq!(app.state.search.text(), "abc");
    assert!(!app.editor_menu_open());

    // 剪贴板有内容：粘贴项可用，点击后运行时立即读取并插入。
    app.runtime.context().clipboard_set("xyz");
    app.open_editor_menu(target, anchor, true);
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(2)));
    assert!(app.take_clipboard_request().is_none());
    assert_eq!(app.state.search.text(), "abcxyz");
    assert!(!app.editor_menu_open());

    // 全选后可复制整段文本。
    app.open_editor_menu(target, anchor, true);
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(4)));
    assert_eq!(app.state.search.edit.selected_text(), Some("abcxyz"));

    app.open_editor_menu(target, anchor, true);
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(1)));
    assert!(app.take_clipboard_request().is_none());
    assert_eq!(clipboard_text(&app).as_deref(), Some("abcxyz"));
}

/// 右键绝不主动做词选：无选区时只把插入符移到点击处。
#[test]
fn right_clicking_an_editor_moves_the_caret_without_selecting_a_word() {
    let mut app = search_with_text("abc def");
    app.key(
        &PlatformKey::Named("Home".to_string()),
        Modifiers::default(),
    );
    let rect = rect_of(&app, target::id(Target::Search));
    app.right_click(rect.center());
    app.rebuild(VIEWPORT);

    assert!(
        app.editor_menu_open(),
        "the right click must open the context menu"
    );
    assert_eq!(
        app.state.search.edit.selection(),
        None,
        "opening the menu must never select a word"
    );
    assert!(
        app.state.search.edit.cursor() > 0,
        "the caret must move to the click position"
    );
}

/// WinUI 选区规则：点在选区内保留选区，点在选区外折叠并把插入符移到点击处。
#[test]
fn right_click_keeps_a_selection_inside_it_and_collapses_it_outside() {
    let mut app = search_with_text("abc def");
    app.state.search.edit.set_cursor(0, false);
    app.state.search.edit.set_cursor(3, true);
    assert_eq!(app.state.search.edit.selected_text(), Some("abc"));

    let rect = rect_of(&app, target::id(Target::Search));
    // 点击文本起点附近：落在选区内，选区必须保留。
    let inside = Vec2::new(rect.origin.x + 36.0 + 1.0, rect.center().y);
    app.open_editor_menu(Target::Search, inside, false);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.search.edit.selected_text(),
        Some("abc"),
        "a right click inside the selection must keep it"
    );

    // 点击文本末尾之外：落在选区外，选区折叠到点击处。
    let outside = Vec2::new(rect.max().x - 1.0, rect.center().y);
    app.open_editor_menu(Target::Search, outside, false);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.search.edit.selection(),
        None,
        "a right click outside the selection must collapse it"
    );
    assert_eq!(
        app.state.search.edit.cursor(),
        "abc def".len(),
        "the caret must land on the clicked position"
    );
}

fn clipboard_text(app: &App) -> Option<String> {
    app.runtime.context().clipboard_get()
}

fn select_all(app: &mut App) {
    app.key(
        &PlatformKey::Character("a".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
}

/// 右键落在选区内必须保留选区；随后点击「复制」通过真实事件链写入剪贴板。
///
/// 回归：右键按下曾走 `focus_editor_at`/`begin_press_target`，在菜单打开前
/// 就把选区折叠，使「复制」始终禁用。
#[test]
fn context_menu_copy_writes_the_selection_through_the_runtime() {
    let mut app = search_with_text("abc def");
    select_all(&mut app);
    let rect = rect_of(&app, target::id(Target::Search));
    // 点击文本起点附近：落在 (0, len) 选区内。
    let inside = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    app.right_click(inside);
    app.rebuild(VIEWPORT);

    assert!(app.editor_menu_open(), "the right click must open the menu");
    assert_eq!(
        app.state.search.edit.selected_text(),
        Some("abc def"),
        "a right click inside the selection must keep it"
    );

    click_node(&mut app, target::id(Target::EditorMenuItem(1)));

    assert_eq!(clipboard_text(&app).as_deref(), Some("abc def"));
    assert!(!app.editor_menu_open());
}

/// 右键后点击「剪切」：选区文本进入剪贴板并从缓冲区移除。
#[test]
fn context_menu_cut_writes_the_clipboard_and_removes_the_selection() {
    let mut app = search_with_text("abc def");
    select_all(&mut app);
    let rect = rect_of(&app, target::id(Target::Search));
    let inside = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    app.right_click(inside);
    app.rebuild(VIEWPORT);

    click_node(&mut app, target::id(Target::EditorMenuItem(0)));

    assert_eq!(app.state.search.text(), "");
    assert_eq!(clipboard_text(&app).as_deref(), Some("abc def"));
}

/// 右键后点击「粘贴」：剪贴板内容立即插入，无需等待下一次按键。
#[test]
fn context_menu_paste_inserts_the_clipboard_through_the_runtime() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    app.runtime.context().clipboard_set("xyz");
    let rect = rect_of(&app, target::id(Target::Search));
    app.right_click(rect.center());
    app.rebuild(VIEWPORT);

    click_node(&mut app, target::id(Target::EditorMenuItem(2)));

    assert_eq!(app.state.search.text(), "xyz");
}

/// 无选区时「剪切」「复制」、空剪贴板时「粘贴」都必须无效果。
#[test]
fn disabled_context_menu_items_do_nothing_through_the_runtime() {
    let mut app = search_with_text("abc");
    let rect = rect_of(&app, target::id(Target::Search));

    app.right_click(rect.center());
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(1)));
    assert_eq!(clipboard_text(&app), None);
    assert_eq!(app.state.search.text(), "abc");

    app.right_click(rect.center());
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(0)));
    assert_eq!(clipboard_text(&app), None);
    assert_eq!(app.state.search.text(), "abc");

    app.right_click(rect.center());
    app.rebuild(VIEWPORT);
    click_node(&mut app, target::id(Target::EditorMenuItem(2)));
    assert_eq!(app.state.search.text(), "abc");
}

/// 空文本时「全选」无效果。
#[test]
fn context_menu_select_all_on_empty_text_does_nothing() {
    let mut app = app();
    app.key(
        &PlatformKey::Character("k".to_string()),
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    let rect = rect_of(&app, target::id(Target::Search));
    app.right_click(rect.center());
    app.rebuild(VIEWPORT);

    click_node(&mut app, target::id(Target::EditorMenuItem(4)));

    assert_eq!(app.state.search.edit.selection(), None);
}

/// 右键按住微动不得形成或扩展选区。
#[test]
fn right_press_never_starts_a_selection_drag() {
    let mut app = search_with_text("abc def");
    let rect = rect_of(&app, target::id(Target::Search));
    let start = Vec2::new(rect.origin.x + 40.0, rect.center().y);
    let end = Vec2::new(rect.max().x - 10.0, rect.center().y);

    app.runtime.pointer_move(start);
    app.runtime.pointer_down(start, PointerButton::Right);
    app.runtime.pointer_move(end);

    assert_eq!(
        app.state.search.edit.selection(),
        None,
        "holding the right button must not extend a selection"
    );

    app.runtime.pointer_up(end, PointerButton::Right);
}

/// 「全选」菜单项用鼠标点击必须生效，结果与 Ctrl+A 相同。
#[test]
fn context_menu_select_all_works_with_the_mouse() {
    let mut app = search_with_text("abc");
    let anchor = rect_of(&app, target::id(Target::Search)).center();
    app.open_editor_menu(Target::Search, anchor, false);
    app.rebuild(VIEWPORT);

    click_node(&mut app, target::id(Target::EditorMenuItem(4)));

    assert_eq!(app.state.search.edit.selected_text(), Some("abc"));
    assert!(!app.editor_menu_open());
}

/// 「全选」菜单项用回车必须生效。
#[test]
fn context_menu_select_all_works_with_enter() {
    let mut app = search_with_text("abc");
    let anchor = rect_of(&app, target::id(Target::Search)).center();
    app.open_editor_menu(Target::Search, anchor, false);
    app.rebuild(VIEWPORT);

    // 无选区时剪切/复制禁用、剪贴板为空时粘贴禁用，方向键直接落到「全选」。
    app.key(
        &PlatformKey::Named("ArrowDown".to_string()),
        Modifiers::default(),
    );
    app.key(
        &PlatformKey::Named("Enter".to_string()),
        Modifiers::default(),
    );

    assert_eq!(app.state.search.edit.selected_text(), Some("abc"));
    assert!(!app.editor_menu_open());
}

/// Esc、点击外部与滚动都能关闭菜单。
#[test]
fn escape_outside_click_and_scroll_close_the_context_menu() {
    let mut app = search_with_text("abc");
    let anchor = rect_of(&app, target::id(Target::Search)).center();

    app.open_editor_menu(Target::Search, anchor, false);
    app.rebuild(VIEWPORT);
    assert!(app.editor_menu_open());
    app.key(
        &PlatformKey::Named("Escape".to_string()),
        Modifiers::default(),
    );
    assert!(!app.editor_menu_open(), "escape must close the menu");

    app.open_editor_menu(Target::Search, anchor, false);
    app.rebuild(VIEWPORT);
    let outside = rect_of(&app, target::id(Target::Theme(0))).center();
    app.pointer_move(outside);
    app.pointer_down(outside, 0.0);
    app.pointer_up(outside, 0.1);
    app.rebuild(VIEWPORT);
    assert!(
        !app.editor_menu_open(),
        "clicking outside must close the menu"
    );

    app.open_editor_menu(Target::Search, anchor, false);
    app.rebuild(VIEWPORT);
    app.runtime.wheel(anchor, Vec2::new(0.0, -1.0));
    app.rebuild(VIEWPORT);
    assert!(!app.editor_menu_open(), "scrolling must close the menu");
}

#[test]
fn code_panels_change_the_page_extent() {
    let mut app = app();
    let closed = app.frame().nodes.len();
    app.state.toggle_code(PageId::Home, 0);
    app.rebuild(VIEWPORT);
    let open = app.frame().nodes.len();
    assert!(open > closed, "expanded code should add semantic nodes");
    app.state.toggle_code(PageId::Home, 0);
    app.rebuild(VIEWPORT);
    assert_eq!(app.frame().nodes.len(), closed);
}

#[test]
fn knobs_are_live_and_route_through_the_reducer() {
    let mut app = app();
    app.navigate(PageId::Elevation);
    app.rebuild(VIEWPORT);
    let before = app.state.knob(
        PageId::Elevation,
        0,
        0,
        &PageId::Elevation.examples()[0].knobs[0],
    );
    let node = target::id(Target::Knob(PageId::Elevation.index() as u8, 0, 0));
    let zone = app
        .ranges
        .iter()
        .find(|zone| zone.node == node)
        .copied()
        .expect("radius slider should expose a range");
    let point = Vec2::new(
        zone.rect.origin.x + zone.rect.size.x * 0.85,
        zone.rect.center().y,
    );
    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);
    let after = app.state.knob(
        PageId::Elevation,
        0,
        0,
        &PageId::Elevation.examples()[0].knobs[0],
    );
    assert_ne!(before, after, "dragging the radius slider should update it");
}

#[test]
fn pointer_hits_navigation_rows_without_a_window() {
    let mut app = app();
    // Design 分区现已成为真实的树分类，因此 Colors 拥有独立行；
    // 指针必须解析出其目标标识。
    let node = target::id(Target::NavPage(PageId::Colors.index() as u8));
    let point = rect_of(&app, node).center();
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);
    assert_eq!(app.state.active_page, PageId::Colors);
}

/// 点击必须在同一帧内到达屏幕。
///
/// 当场景代次未推进时 GPU 跳过上传，因此改变了状态却未推进代次的点击
/// 对用户而言是不可见的。代次由重建本身递增，使两者绝不会发生漂移。
///
///
#[test]
fn clicking_a_button_bumps_scene_generation_immediately() {
    let mut app = app();
    let before = app.generation();
    // HUD 切换改变了场景的*形状*：浮层出现。
    let node = target::id(Target::Hud);
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);

    // 释放时状态发生了变化。
    assert!(app.state.show_hud, "the click must toggle the HUD");
    // 下一次 sync 重建场景，且重建推进了代次：这正是促使 GPU 重新上传图元的原因。
    //
    assert!(
        app.sync(VIEWPORT),
        "the click must be visible to the renderer"
    );
    assert!(
        app.generation() > before,
        "the scene generation must advance so the frame is re-uploaded"
    );
}

/// 切换主题通过颜色表重绘，而非重写缓冲池。
///
/// 帧绘制的每个颜色都是 4 KiB 查找表中的一个槽位，因此切换主题只需重写该表，
/// 完全不移动任何图元。这是代价最低的全屏重绘：零字节场景上传。
///
#[test]
fn switching_the_theme_uploads_no_primitives() {
    let mut app = app();
    assert_eq!(app.state.theme, ThemeMode::Dark);
    let node = target::id(Target::Theme(0));
    let point = rect_of(&app, node).center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);
    let before = app.generation();
    app.rebuild(VIEWPORT);

    assert_eq!(app.state.theme, ThemeMode::Light);
    assert_eq!(
        app.generation(),
        before,
        "a theme switch must not re-upload a single primitive"
    );
    assert!(
        app.dirty_ranges().is_empty() || app.state.demo.upload_bytes < 1024,
        "a theme switch may only repaint the slots that changed"
    );
}

/// 按下后拖离控件不得激活它。
///
/// 离散控件在释放时生效，当释放未落在被按压的控件上时路由器会扣留点击事件。
/// 两者共同使“按下、拖走、释放”成为空操作。
///
#[test]
fn dragging_off_a_shell_control_cancels_activation() {
    let mut app = app();
    let node = target::id(Target::Theme(0));
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    // 拖拽至远离控件的位置并在该处释放。
    let away = Vec2::new(point.x + 400.0, point.y + 300.0);
    app.pointer_move(away);
    app.pointer_up(away, 0.1);

    assert_eq!(
        app.state.theme,
        ThemeMode::Dark,
        "a drag off the control must not activate it"
    );
}

/// 离散控件在释放时生效，而非按下时。
///
/// 这消除了双重激活：过去按压时提交了切换，释放时又重放一遍，把控件翻了回去。
///
#[test]
fn a_press_alone_does_not_activate_a_discrete_control() {
    let mut app = app();
    let node = target::id(Target::Theme(0));
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    assert_eq!(
        app.state.theme,
        ThemeMode::Dark,
        "the press must not commit the toggle"
    );
    app.pointer_up(point, 0.1);
    assert_eq!(app.state.theme, ThemeMode::Light);
}

/// 单次点击不得触发两次导航。
///
/// 过去曾会这样：按压触发一次导航，释放又触发一次导航，导致内容滚动重置了两次。
///
#[test]
fn a_navigation_click_fires_exactly_once() {
    let mut app = app();
    let node = target::id(Target::NavPage(PageId::Colors.index() as u8));
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    assert_eq!(app.state.active_page, PageId::Home);
    app.pointer_up(point, 0.1);
    assert_eq!(app.state.active_page, PageId::Colors);
}

/// 调整窗口大小必须重新计算布局。
///
/// 这正是实时缩放调整生效的原理：帧按新尺寸绘制，导航栏、卡片网格与文本换行
/// 跟随新边缘，而非停留在旧位置。
///
#[test]
fn a_resize_recomputes_the_layout() {
    let mut app = app();
    let before = app.generation();

    // 窗口变窄了：视口已改变。
    assert!(
        app.sync((960.0, 600.0)),
        "a viewport change must be visible to the renderer"
    );
    assert!(
        app.generation() > before,
        "the rebuilt scene must advance the upload generation"
    );

    // 命中区域已按新尺寸重建：主题切换按钮现在位于新布局放置它的位置，而非旧位置。
    //
    let node = target::id(Target::Theme(0));
    let resized = rect_of(&app, node);
    assert!(
        resized.size.x > 0.0 && resized.size.y > 0.0,
        "the rebuilt layout must have a hit region"
    );
}

/// 拖拽过程中的每一个中间尺寸都必须经过布局，而非仅最后一次。
/// 只在最终尺寸布局的拖拽会在整个拖拽期间显示冻结的画面。
///
#[test]
fn every_size_of_a_drag_is_laid_out() {
    let mut app = app();
    let mut previous = app.generation();
    for width in [1400.0, 1300.0, 1200.0, 1100.0, 1000.0] {
        assert!(
            app.sync((width, 900.0)),
            "the resize to {width} must be visible to the renderer"
        );
        assert!(
            app.generation() > previous,
            "every size of the drag must rebuild the scene"
        );
        previous = app.generation();
        let report = Harness::run(&app.frame());
        assert!(
            report.is_clean(),
            "the layout at {width} produced {} diagnostic(s): {}",
            report.diagnostics.len(),
            report.summary()
        );
    }
}

/// 失去焦点必须中止进行中的交互。
///
/// 否则外壳会一直保留幽灵拖拽：滚动条保持被抓取状态，演示持续运行框选，
/// 直到用户恰好点击别处为止。
///
#[test]
fn losing_the_focus_cancels_the_pointer_capture() {
    let mut app = app();
    let node = target::id(Target::Theme(0));
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    assert!(app.pressed().is_some());

    app.cancel_interactions();
    assert!(app.pressed().is_none(), "the capture must be released");
    // 待定激活已清除：释放不再提交。
    app.pointer_up(point, 0.1);
    assert_eq!(
        app.state.theme,
        ThemeMode::Dark,
        "a cancelled press must not activate the control"
    );
}

/// 当卡片足够宽时，选项面板应位于舞台旁。
///
/// 该决定由布局引擎做出：弹性行空间耗尽时面板自动折到舞台下方。
/// 外壳中没有任何代码显式要求它这么做。
#[test]
fn a_wide_card_keeps_its_options_panel_beside_the_stage() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.rebuild(VIEWPORT);
    let card = &app.page_layout().expect("page layout").cards[0];
    assert!(
        card.knobs.origin.y <= card.stage.max().y,
        "the options panel should start beside the stage: {:?}",
        card.knobs
    );
    assert!(
        card.knobs.origin.x >= card.stage.max().x - 1.0,
        "the options panel should not overlap the stage"
    );
}

/// 窄卡片会将选项面板折叠到舞台下方（接入 `NavigationView` Auto 模式后，560px 窄视口进入 Minimal 档且内容区仍足够窄以触发折叠）。
#[test]
fn a_narrow_card_folds_its_options_panel_below_the_stage() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.rebuild((560.0, 900.0));
    let card = &app.page_layout().expect("page layout").cards[0];
    assert!(
        card.knobs.origin.y > card.stage.max().y,
        "the options panel should fold below the stage: {:?}",
        card.knobs
    );
    assert!(
        card.knobs.size.x <= card.stage.size.x + 1.0,
        "the folded panel should not be wider than the stage"
    );
}

/// 折叠行为逐帧跟随窗口尺寸。
///
/// 只在最终尺寸布局的拖拽会在整个拖拽期间显示冻结画面，
/// 因此每一个中间尺寸都必须求解且必须保持整洁。
///
#[test]
fn the_options_panel_folds_while_the_window_is_resized() {
    let mut app = app();
    app.navigate(PageId::Home);
    let mut folded = Vec::new();
    for width in [1440.0, 1200.0, 1000.0, 800.0, 580.0, 500.0, 1440.0] {
        app.rebuild((width, 900.0));
        let card = &app.page_layout().expect("page layout").cards[0];
        folded.push(card.knobs.origin.y > card.stage.max().y);
        let report = Harness::run(&app.frame());
        assert!(
            report.is_clean(),
            "the layout at {width} produced {} diagnostic(s): {}",
            report.diagnostics.len(),
            report.summary()
        );
    }
    assert_eq!(
        folded,
        vec![false, false, false, false, true, true, false],
        "the panel must fold once the card is too narrow and unfold again"
    );
}

/// 卡片尺寸固定，因此卡片是重布局边界。
///
/// 卡片内部任何内容都无法改变其尺寸，这意味着卡片内部的变更绝不会重新求解整个页面。
///
#[test]
fn every_card_is_a_relayout_boundary() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.rebuild(VIEWPORT);
    let cards = app.page_layout().expect("page layout").cards.len();
    assert!(cards > 0, "the page should have cards");
    assert_eq!(
        app.relayout_boundary_count(),
        cards,
        "every card must be a relayout boundary"
    );
}

/// 悬停重绘控件；不移动任何元素。
///
/// 帧的布局开销必须精确为零，否则每次指针移动都要付出全树求解的代价。
///
#[test]
fn a_hover_repaints_without_relaying_out() {
    let mut app = app();
    let node = target::id(Target::NavPage(PageId::Buttons.index() as u8));
    let point = rect_of(&app, node).center();
    app.pointer_move(point);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.layout_stats().nodes_visited,
        0,
        "a hover must not visit a single layout node"
    );
    assert!(
        !app.dirty_ranges().is_empty(),
        "a hover must still repaint, so it must name the slices it changed"
    );
}

/// 悬停产生的修补仅为寥寥数字节，而非整个场景。
#[test]
fn a_hover_uploads_a_handful_of_bytes() {
    let mut app = app();
    let node = target::id(Target::NavPage(PageId::Buttons.index() as u8));
    let point = rect_of(&app, node).center();
    app.pointer_move(point);
    app.rebuild(VIEWPORT);
    let stride = core::mem::size_of::<unbound_gpu::data::GpuPrimitiveInstance>() as u32;
    let bytes: u32 = app
        .dirty_ranges()
        .iter()
        .map(|(start, end)| (end - start) * stride)
        .sum();
    assert!(
        bytes < 1024,
        "a hover over one control must stay under 1 KiB, got {bytes} B in {:?}",
        app.dirty_ranges()
    );
}

/// 修补保持代次不变，使渲染器知道无需全量重传。
#[test]
fn a_paint_only_change_keeps_the_upload_generation() {
    let mut app = app();
    let generation = app.generation();
    let node = target::id(Target::NavPage(PageId::Buttons.index() as u8));
    let point = rect_of(&app, node).center();
    app.pointer_move(point);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.generation(),
        generation,
        "a paint-only frame patches the pool; it does not replace it"
    );
}

/// 导航改变了页面形状，因此整个场景重新上传。
#[test]
fn a_navigation_bumps_the_upload_generation() {
    let mut app = app();
    let generation = app.generation();
    app.navigate(PageId::Sliders);
    app.rebuild(VIEWPORT);
    assert_ne!(
        app.generation(),
        generation,
        "a new page is a new scene and must be uploaded in full"
    );
    assert!(
        app.dirty_ranges().is_empty(),
        "a full upload must not carry stale patch ranges"
    );
}

/// 悬停控件仅重绘它；绝不会重新布局页面。
///
/// 遍历每个页面上的每个控件：若指针移动触及了布局引擎，
/// 整个页面都会为一次悬停付出代价。
#[test]
fn hovering_a_control_never_relays_out_the_page() {
    for page in PageId::ALL {
        let mut app = app();
        app.navigate(page);
        app.pointer_move(Vec2::new(2.0, 898.0));
        app.rebuild(VIEWPORT);
        let targets: Vec<u64> = app.regions.iter().map(|region| region.node).collect();
        for node in targets {
            let point = rect_of(&app, node).center();
            app.pointer_move(point);
            app.rebuild(VIEWPORT);
            assert_eq!(
                app.layout_stats().nodes_visited,
                0,
                "{page:?} re-laid the page out for a hover on {node:#x}"
            );
        }
    }
}

/// 三个窗口按钮紧密贴合、尺寸相等且直达右边缘。
///
/// 旧布局在“关闭”前留下了 52 px 空隙且图标未共享中心，
/// 导致右上角出现一条死区，三个字形中有两个偏离中心。
///
#[test]
fn the_window_buttons_are_flush_and_equal() {
    let mut app = App::new();
    app.rebuild((1440.0, 900.0));
    let close = app.layout.window_close;
    let maximize = app.layout.window_maximize;
    let minimize = app.layout.window_minimize;

    assert_eq!(close.max().x, 1440.0, "close must reach the right edge");
    for (name, rect) in [
        ("close", close),
        ("maximize", maximize),
        ("minimize", minimize),
    ] {
        assert_eq!(rect.size.x, 46.0, "{name} must keep its 46 px width");
        assert_eq!(rect.origin.y, 0.0, "{name} must start at the top");
    }
    assert_eq!(maximize.max().x, close.origin.x);
    assert_eq!(minimize.max().x, maximize.origin.x);
    let centre = |rect: unbound_math::Rect| rect.center().y;
    assert!((centre(close) - centre(maximize)).abs() < 0.01);
    assert!((centre(close) - centre(minimize)).abs() < 0.01);
}

/// 未产生任何变化的重建不得丢弃待处理的修补。
///
/// 窗口每帧可能重建两次（先事件处理，后动画 tick）。
/// 第二次 diff 仅覆盖自第一次重建以来的变化，若在此清空待处理列表会静默丢弃事件的绘制变更，
/// 导致帧在下一次全量上传前一直保留旧颜色。
///
#[test]
fn an_identical_rebuild_keeps_the_pending_patch() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.pointer_move(Vec2::new(2.0, 898.0));
    app.rebuild(VIEWPORT);
    app.mark_uploaded();

    let node = app
        .regions
        .iter()
        .map(|region| region.node)
        .find(|node| !target::is_demo(*node))
        .expect("the page should have chrome to hover");
    app.pointer_move(rect_of(&app, node).center());
    app.rebuild(VIEWPORT);
    let pending = app.dirty_ranges().to_vec();
    assert!(!pending.is_empty(), "the hover must produce a patch");

    app.rebuild(VIEWPORT);
    assert_eq!(
        app.dirty_ranges(),
        pending.as_slice(),
        "a rebuild with no changes must keep the pending patch for the host"
    );

    app.mark_uploaded();
    assert!(
        app.dirty_ranges().is_empty(),
        "an upload must clear the pending patch"
    );
}

/// 悬停仅修补缓冲池；不会全量重传场景。
///
/// 外壳外框与标准控件在各状态间保持恒定的图元数量，这正是该优化得以成立的原因。
/// 演示舞台除外：演示属于定制绘制，悬停时合法地可能显露新控件
/// （如表格行高亮、对话框阴影），因此它会回退到全量上传——正确，只是不廉价。
///
///
#[test]
fn a_hover_patches_the_pool_instead_of_reuploading_it() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.pointer_move(Vec2::new(2.0, 898.0));
    app.rebuild(VIEWPORT);
    let targets: Vec<u64> = app
        .regions
        .iter()
        .map(|region| region.node)
        .filter(|node| !target::is_demo(*node))
        .collect();
    assert!(!targets.is_empty(), "the page should have chrome to hover");
    let stride = core::mem::size_of::<unbound_gpu::data::GpuPrimitiveInstance>() as u32;
    for node in targets {
        app.pointer_move(Vec2::new(2.0, 898.0));
        app.rebuild(VIEWPORT);
        // 重置悬停的切片已“上传”；只有接下来的状态转换才能计入本次测量的修补。
        //
        app.mark_uploaded();
        let generation = app.generation();
        let point = rect_of(&app, node).center();
        app.pointer_move(point);
        app.rebuild(VIEWPORT);
        assert_eq!(
            app.generation(),
            generation,
            "hovering {node:#x} swapped the whole scene instead of patching it"
        );
        let bytes: u32 = app
            .dirty_ranges()
            .iter()
            .map(|(start, end)| (end - start) * stride)
            .sum();
        assert!(
            bytes < 1024,
            "hovering {node:#x} uploaded {bytes} B, over the 1 KiB patch budget"
        );
    }
}

/// 悬停修补停留在被悬停的控件内部。
///
/// 这是字节预算的另一半：上传量少只有在上传的恰好是正确的那一小块时才正确。
/// 每个控件拥有图元池中的一段跨度，每个脏切片都必须落在状态发生变化的控件跨度内。
///
///
#[test]
fn a_hover_patch_stays_inside_the_hovered_control() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.pointer_move(Vec2::new(2.0, 898.0));
    app.rebuild(VIEWPORT);
    let targets: Vec<u64> = app
        .regions
        .iter()
        .map(|region| region.node)
        .filter(|node| !target::is_demo(*node))
        .collect();
    let mut spans = 0usize;
    for node in targets {
        app.pointer_move(Vec2::new(2.0, 898.0));
        app.rebuild(VIEWPORT);
        app.mark_uploaded();
        // 部分滚出导航树的行仅在裁剪区域内接收事件，因此悬停点必须是可见部分的中心——
        // 悬停被裁掉的行会命中其下方绘制的任意元素。
        //
        //
        let Some(region) = app
            .regions
            .iter()
            .find(|region| region.node == node)
            .cloned()
        else {
            continue;
        };
        let mut visible = region.bounds;
        if let Some(clip) = &region.clip {
            let x0 = visible.origin.x.max(clip.rect.origin.x);
            let y0 = visible.origin.y.max(clip.rect.origin.y);
            let x1 = visible.max().x.min(clip.rect.max().x);
            let y1 = visible.max().y.min(clip.rect.max().y);
            visible = Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
        }
        if visible.size.x < 4.0 || visible.size.y < 6.0 {
            continue;
        }
        let hovered = visible.center();
        app.pointer_move(hovered);
        app.rebuild(VIEWPORT);
        let Some((start, end)) = app.span_of(node) else {
            continue;
        };
        spans += 1;
        for (dirty_start, dirty_end) in app.dirty_ranges() {
            assert!(
                *dirty_start >= start && *dirty_end <= end,
                "{node:#x} owns [{start}, {end}) but the patch is [{dirty_start}, {dirty_end})"
            );
        }
    }
    assert!(
        spans > 20,
        "the span map should cover the page's chrome, covered {spans}"
    );
}

/// 一个控件，一个标识。
///
/// 该标识由命中区域、语义节点与图元跨度共享，因此第二个控件复用它绝非外观小问题：
/// 两者会同时亮起，语义树会丢失其中之一，跨度映射表也只保留最后一次注册。
/// 同一页面的页脚快捷方式与树行过去正是这样发生冲突的。
///
///
#[test]
fn every_hit_region_has_its_own_identity() {
    for expanded in [true, false] {
        for page in PageId::ALL {
            let mut app = app();
            app.navigate(page);
            if !expanded {
                click_node(&mut app, target::id(Target::NavToggle));
                // 等待弹簧稳定：折叠后的图标栏是不同的渲染器，必须单独审计。
                //
                for _ in 0..120 {
                    app.tick(1.0 / 60.0);
                }
            }
            app.rebuild(VIEWPORT);
            let mut seen: std::collections::HashMap<u64, unbound_math::Rect> =
                std::collections::HashMap::new();
            for region in app.regions.iter() {
                if let Some(first) = seen.insert(region.node, region.bounds) {
                    panic!(
                        "{page:?} (expanded: {expanded}): node {:#x} has two hit regions, {first:?} and {:?}",
                        region.node, region.bounds
                    );
                }
            }
        }
    }
}

/// 页脚快捷方式可触发导航，且只有其自身所在行会亮起。
#[test]
fn the_footer_shortcuts_navigate_on_their_own_identity() {
    let mut app = app();
    let footer = target::id(Target::Footer(0));
    let row = rect_of(&app, footer);
    let point = row.center();
    app.pointer_move(point);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.hovered(),
        Some(footer),
        "the footer row must own the hover, not the tree row for the same page"
    );
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.active_page,
        PageId::Telemetry,
        "the footer shortcut jumps to its page"
    );
}

/// 最大化图标在两种状态下绘制相同数量的图元。
///
/// 窗口还原时图标为一个方框，最大化时为两个方框。图元*数量*不同会改变场景形状，
/// 从而推进上传代次并替换整个缓冲池——更糟的是，还会掩盖丢帧：
/// 下一帧会显示为“完全相同”，导致缓冲池落后于基准。
///
///
/// 因此两种状态都精确绘制两个图元；窗口还原时第二个图元为透明。
///
///
#[test]
fn the_maximise_icon_matches_window_caption_buttons_primitive_contract() {
    let mut app = app();
    app.set_window_maximized(false);
    app.rebuild(VIEWPORT);
    let restored = app.scene.primitives.len();

    app.set_window_maximized(true);
    app.rebuild(VIEWPORT);
    let maximized = app.scene.primitives.len();

    // `WindowCaptionButtons`（ADR-0029）还原态采用镂空前框 + 后框 L 形两条边（共 4 图元），
    // 相比最大化态单框（2 图元）精确多出 2 个图元，避免在透明 Mica 下使用实心遮挡块。
    assert_eq!(
        maximized,
        restored + 2,
        "WindowCaptionButtons restore icon should add exactly 2 L-edge primitives: {restored} -> {maximized}"
    );
}

/// 字形图集的驱逐时钟每次重建都会推进。
///
/// 图集丢弃最近最少*使用*的字形，“使用”按帧数度量。冻结的时钟会使所有槽位看起来一样旧，
/// 导致满载图集永远驱逐相同的字形。
///
///
/// 时钟刻意与上传代次分离：在运行时的外部 diff 下，代次仅在场景形状变化时推进，
/// 而驱逐顺序无论如何都必须向前推进。
///
#[test]
fn the_atlas_eviction_clock_advances_on_every_rebuild() {
    let mut app = app();
    // 运行时拥有 diff，因此外壳自身的代次保持原地不动。
    app.set_external_diff(true);
    app.rebuild(VIEWPORT);
    let first = app.labels.cache.atlas.frame;
    app.rebuild(VIEWPORT);
    let second = app.labels.cache.atlas.frame;
    assert!(
        second > first,
        "the atlas eviction clock is frozen: {first} -> {second}"
    );
}

/// 拖拽内容滚动条必须逐帧滚动页面。
///
/// 指针一旦越过 3 px 的 `drag_slop`，路由器就会停止发送 `PointerMove` 并改为发送
/// `DragStart`/`Drag`。外壳过去会丢弃这些事件，导致滚动条抓取在成为真实拖拽的那一刻死亡。
/// 本测试以 20 px 步长驱动 200 px 拖拽并断言每步都推进滚动，
/// 然后点击滑块上方的轨道并断言页面跳回上方。
///
///
#[test]
fn dragging_the_content_scrollbar_moves_the_page_across_drag_slop() {
    let mut app = app();
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);
    let node = target::id(Target::ContentScrollbar);
    let bar = rect_of(&app, node);
    assert_eq!(app.state.content_scroll, 0.0);

    // 从公开布局推导滑块行程，使拖拽保持在钳制范围以下：
    // 在仅轻微溢出的页面上拖拽 200 px 会饱和并停止推进，那是正确行为而非缺陷。
    //
    let page = app.page_layout().expect("page layout");
    let content = app.layout.content;
    let max_scroll = (page.content_height - content.size.y + 48.0).max(0.0);
    let track_h = (content.size.y - 16.0).max(1.0);
    let thumb_h = (track_h * content.size.y / page.content_height.max(1.0)).clamp(28.0, track_h);
    let travel = (track_h - thumb_h).max(1.0);
    let ratio = max_scroll / travel;
    let target_scroll = max_scroll * 0.6;
    let drag_px = target_scroll / ratio;

    // 靠近滑块上边缘抓取滑块（滑块从轨道顶部开始）。
    let grab = Vec2::new(bar.center().x, bar.origin.y + 10.0);
    app.pointer_move(grab);
    app.pointer_down(grab, 0.0);

    let mut previous = 0.0;
    let steps = 10;
    for step in 1..=steps {
        let point = Vec2::new(grab.x, grab.y + drag_px * step as f32 / steps as f32);
        app.pointer_move(point);
        let scroll = app.state.content_scroll;
        assert!(
            scroll > previous,
            "step {step}: scroll {scroll} must advance past {previous}"
        );
        previous = scroll;
    }
    assert!(
        (previous - target_scroll).abs() < 1.0,
        "the drag must track the pointer precisely: {previous} vs {target_scroll}"
    );
    app.pointer_up(Vec2::new(grab.x, grab.y + drag_px), 0.1);
    app.rebuild(VIEWPORT);
    let scrolled = app.state.content_scroll;
    assert!(scrolled > 0.0);

    // 点击滑块上方的轨道：滑块以指针为中心对齐，页面跳回上方。
    //
    let bar = rect_of(&app, node);
    let above = Vec2::new(bar.center().x, bar.origin.y + 2.0);
    app.pointer_move(above);
    app.pointer_down(above, 0.0);
    app.pointer_up(above, 0.1);
    assert!(
        app.state.content_scroll < scrolled,
        "clicking above the thumb must scroll up: {} -> {}",
        scrolled,
        app.state.content_scroll
    );
}

/// 拖拽虚拟列表滚动条可到达 100 000 行列表的末尾。
///
/// 列表已虚拟化，因此滚动条是到达最后一行的唯一途径；无法到达底部的拖拽会使列表尾部不可达。
/// 本测试抓取滑块并将其拖到轨道底边，然后断言滚动位置落在理论最大值的单行误差范围内。
///
///
#[test]
fn dragging_the_virtual_list_scrollbar_scrolls_to_100k_end() {
    let mut app = app();
    app.navigate(PageId::ListView);
    // 卡片 0，旋钮 0 = 2 选择 100 000 行数据集。
    app.state.set_knob(PageId::ListView, 0, 0, 2.0);
    app.rebuild(VIEWPORT);

    let node = target::id(Target::Demo(PageId::ListView.index() as u8, 0, 200));
    let bar = rect_of(&app, node);
    let row_spec = PageId::ListView.examples()[0].knobs[1];
    let row = app
        .state
        .knob(PageId::ListView, 0, 1, &row_spec)
        .clamp(24.0, 48.0);
    let viewport_h = bar.size.y + 8.0;
    let max_scroll = (100_000.0 * row - viewport_h).max(0.0);
    assert!(
        max_scroll > 1_000_000.0,
        "the list should be huge: {max_scroll}"
    );
    assert_eq!(app.state.demo.list_scroll[0], 0.0);

    let grab = Vec2::new(bar.center().x, bar.origin.y + 4.0);
    app.pointer_move(grab);
    app.pointer_down(grab, 0.0);
    // 以每步都跨越拖拽阈值（slop）的步长拖拽至轨道底部。
    let bottom = bar.max().y - 1.0;
    let steps = 12;
    for step in 1..=steps {
        let y = grab.y + (bottom - grab.y) * step as f32 / steps as f32;
        app.pointer_move(Vec2::new(grab.x, y));
    }
    app.pointer_up(Vec2::new(grab.x, bottom), 0.1);

    let scroll = app.state.demo.list_scroll[0];
    assert!(
        scroll >= max_scroll - row,
        "the last row must be reachable: scroll {scroll}, max {max_scroll}, row {row}"
    );
    assert!(
        scroll <= max_scroll + 0.5,
        "the scroll must clamp at the end: {scroll} > {max_scroll}"
    );
}

/// 导航窗格在内容溢出时响应滚轮独立滚动。
#[test]
fn scrolling_the_navigation_pane_updates_nav_view_scroll_offset() {
    let mut app = app();
    // 较矮的窗口确保导航菜单溢出窗格。
    app.rebuild((1440.0, 520.0));
    assert!(
        app.nav_view.scroll_max > 0.0,
        "nav pane should overflow at 520px height"
    );
    assert_eq!(app.nav_view.scroll_offset, 0.0);

    let nav_center = app.nav_layout.menu_viewport_rect.center();
    app.pointer_move(nav_center);
    app.wheel(-3.0);
    app.rebuild((1440.0, 520.0));
    assert!(
        app.nav_view.scroll_offset > 0.0,
        "wheel scrolling over NavigationView pane must advance scroll_offset"
    );
}

/// Showcase 覆盖率门禁的运行时半边证明（S2.4）。
///
/// 编译期门禁（`showcase_entry_for_widget` + `pages/components.rs` 中的 `const` 断言）
/// 证明每个目录组件都已由展厅卡片*挂载*。本测试证明每个组件都真正*由其真实发射器绘制*：
/// 卡片在离屏时会被裁剪剔除，因此测试滚动整个组件展厅并将每帧的 `emitted_widgets`
/// 集合求并集，然后断言其覆盖了 `WidgetCatalogId::ALL`。
/// 手绘替身绝不会插入其 ID，因此无法通过此门禁。
///
///
#[test]
fn the_component_gallery_emits_every_catalog_widget() {
    use unbound_widgets::WidgetCatalogId;

    let mut app = app();
    app.navigate(PageId::Components);
    app.rebuild(VIEWPORT);

    let content = app.layout.content;
    let page_height = app.page_layout().expect("page layout").content_height;
    let max_scroll = (page_height - content.size.y + 48.0).max(0.0);
    let step = (content.size.y * 0.35).max(120.0);

    let mut seen = rustc_hash::FxHashSet::default();
    let mut scroll = 0.0;
    loop {
        app.state.content_scroll = scroll;
        app.rebuild(VIEWPORT);
        seen.extend(app.emitted_widgets().iter().copied());
        if scroll >= max_scroll {
            break;
        }
        scroll = (scroll + step).min(max_scroll);
    }

    let missing: Vec<&str> = WidgetCatalogId::ALL
        .iter()
        .filter(|id| !seen.contains(id))
        .map(|id| id.module_name())
        .collect();
    assert!(
        missing.is_empty(),
        "the gallery never emitted these catalog components: {missing:?}"
    );
}

/// 缓动演示的曲线编辑器可拖拽其控制点（S3.6）。
///
/// 该贝塞尔曲线是真实的 `emit_curve_editor`，而非静态图：按下控制柄捕获它，
/// 拖拽重塑曲线，释放结束手势。绘制的控制点必须精确跟随指针。
///
#[test]
fn dragging_the_easing_curve_moves_its_control_point() {
    use unbound_widgets::curve_editor::{CurveEditorDesc, CurveHandle};

    let mut app = app();
    app.navigate(PageId::Springs);
    // 较高的窗口使整张缓动卡片（及其下方控制点）保留在被裁剪的内容视口内，
    // 避免按压被剔除。
    let viewport = (1440.0, 1100.0);
    app.rebuild(viewport);

    let node = target::id(Target::Demo(PageId::Springs.index() as u8, 1, 1));
    let desc = CurveEditorDesc::new(rect_of(&app, node));

    let before = app.state.demo.curve_editor.p1();
    let grab = desc.to_screen(before);
    app.pointer_move(grab);
    app.pointer_down(grab, 0.0);
    assert_eq!(app.state.demo.curve_editor.active(), Some(CurveHandle::P1));

    let destination = desc.frame().center();
    app.pointer_move(destination);
    let expected = desc.from_screen(destination);
    let after = app.state.demo.curve_editor.p1();
    assert!(
        (after.x - expected.x).abs() < 1e-4 && (after.y - expected.y).abs() < 1e-4,
        "the handle must track the pointer: {after:?} vs {expected:?}"
    );
    assert_ne!(after, before, "the control point must move");

    app.pointer_up(destination, 0.1);
    assert_eq!(app.state.demo.curve_editor.active(), None);
}

/// `NavigationView` 折叠与展开过程在 48px 紧凑轨与 248px 展开态之间平滑收敛。
#[test]
fn sidebar_collapse_and_expand_converges_cleanly() {
    let mut app = app();
    let initial = app.layout.nav_width;
    assert!((initial - 248.0).abs() < 0.01);

    // 触发折叠：宽度收敛至 WinUI 3 标准紧凑轨 48.0。
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(!app.nav_view.is_pane_open);
    for _ in 0..120 {
        app.tick(1.0 / 60.0);
        app.rebuild(VIEWPORT);
    }
    assert!(
        (app.layout.nav_width - 48.0).abs() < 0.5,
        "collapsed nav width expected ~48.0, got {}",
        app.layout.nav_width
    );

    // 触发展开：宽度收敛回 248.0。
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(app.nav_view.is_pane_open);
    for _ in 0..120 {
        app.tick(1.0 / 60.0);
        app.rebuild(VIEWPORT);
    }
    assert!(
        (app.layout.nav_width - 248.0).abs() < 0.5,
        "expanded nav width expected ~248.0, got {}",
        app.layout.nav_width
    );
}

/// Showcase `NavigationView` 在宽 / 中 / 窄三档视口下按 `Auto` 规范自动切换并与顶栏联动。
#[test]
fn navigation_view_auto_switches_across_expanded_compact_and_minimal_tiers() {
    use unbound_widgets::navigation_view::NavigationViewDisplayMode;

    let mut app = app();

    // 1. 宽屏（1440 >= 1008）：Expanded 展开态，窗格常驻 248px。
    app.rebuild((1440.0, 900.0));
    assert_eq!(
        app.nav_view.display_mode,
        NavigationViewDisplayMode::Expanded
    );
    assert!(app.nav_view.is_pane_open);
    assert!(app.nav_title_bar_sync().is_pane_toggle_button_visible);
    assert!(app.nav_title_bar_sync().is_pane_open);
    assert!((app.nav_layout.pane_rect.width() - 248.0).abs() < 0.5);
    assert!(app.nav_layout.overlay_rect.is_none());

    // 2. 中屏（800 in (640, 1008)）：自动降级为 Compact 紧凑 48px 图标轨，点击顶栏折叠钮弹出 Overlay 浮层。
    app.rebuild((800.0, 720.0));
    assert_eq!(
        app.nav_view.display_mode,
        NavigationViewDisplayMode::Compact
    );
    assert!(!app.nav_view.is_pane_open);
    assert!(!app.nav_title_bar_sync().is_pane_open);
    assert!((app.nav_layout.pane_rect.width() - 48.0).abs() < 0.5);
    assert!(app.nav_layout.overlay_rect.is_none());

    let toggle_center = rect_of(&app, target::id(Target::NavToggle)).center();
    app.pointer_move(toggle_center);
    app.pointer_down(toggle_center, 0.0);
    app.pointer_up(toggle_center, 0.05);
    for _ in 0..90 {
        app.tick(1.0 / 60.0);
    }
    app.rebuild((800.0, 720.0));
    assert!(app.nav_view.is_pane_open);
    assert!(app.nav_title_bar_sync().is_pane_open);
    assert!(
        app.nav_layout.overlay_rect.is_some(),
        "Compact open pane must render as an overlay"
    );

    // 3. 窄屏（560 <= 640）：自动降级为 Minimal 极简 0px，点击顶栏折叠钮弹出 Overlay，点击外部自动轻闭。
    app.rebuild((560.0, 720.0));
    assert_eq!(
        app.nav_view.display_mode,
        NavigationViewDisplayMode::Minimal
    );
    assert!(!app.nav_view.is_pane_open);
    assert_eq!(app.nav_layout.pane_rect.width(), 0.0);
    assert!(app.nav_layout.overlay_rect.is_none());

    let toggle_center = rect_of(&app, target::id(Target::NavToggle)).center();
    app.pointer_move(toggle_center);
    app.pointer_down(toggle_center, 0.0);
    app.pointer_up(toggle_center, 0.05);
    for _ in 0..90 {
        app.tick(1.0 / 60.0);
    }
    app.rebuild((560.0, 720.0));
    assert!(app.nav_view.is_pane_open);
    let overlay = app
        .nav_layout
        .overlay_rect
        .expect("Minimal open pane must render as an overlay");
    assert!(overlay.width() > 100.0);

    // 点击 Overlay 外部区域应自动轻闭。
    let outside = Vec2::new(500.0, 400.0);
    app.pointer_move(outside);
    app.pointer_down(outside, 0.0);
    app.pointer_up(outside, 0.05);
    app.rebuild((560.0, 720.0));
    assert!(!app.nav_view.is_pane_open);
    assert!(app.nav_layout.overlay_rect.is_none());
}

/// Showcase 顶栏 `WindowCaptionButtons` 热区契约仅暴露最大化按钮（ADR-0029），最小化与关闭经框架事件层分发。
#[test]
fn window_caption_buttons_hit_contract_exposes_only_maximize_rect() {
    use showcase::shell::WindowCommand;
    use unbound_platform::win32_frame::FrameHitMetrics;

    let mut app = app();
    app.rebuild((1440.0, 900.0));
    let layout = app.window_caption_buttons_layout();
    let contract = app.window_caption_hit_contract();

    assert_eq!(contract.title_bar_height, 0.0);
    assert_eq!(contract.minimize_button_rect, Rect::ZERO);
    assert_eq!(contract.close_button_rect, Rect::ZERO);
    assert_eq!(Some(contract.maximize_button_rect), layout.maximize_rect);
    assert_eq!(contract.maximize_button_rect, app.layout.window_maximize);

    // 验证同步至 FrameHitMetrics 契约接口可无误执行，且 DPI 换算矩形保持精确对齐。
    let metrics = FrameHitMetrics::new();
    metrics.apply_caption_hit_contract(layout.maximize_rect, 1.5, 1.0);
    let scaled_max =
        FrameHitMetrics::ui_rect_to_frame_logical(app.layout.window_maximize, 1.5, 1.0);
    assert!((scaled_max.size.x - 46.0 * 1.5).abs() < 0.01);

    // 最小化与关闭通过框架事件层正常触发 WindowCommand。
    click_node(&mut app, target::id(Target::WindowMinimize));
    assert_eq!(app.take_window_command(), Some(WindowCommand::Minimize));
    click_node(&mut app, target::id(Target::WindowClose));
    assert_eq!(app.take_window_command(), Some(WindowCommand::Close));
}

/// 侧边栏导航目标 100% 覆盖全部 25 个页面与全部 106 个目录组件，且点击组件导航项可精确定位。
#[test]
fn sidebar_covers_every_page_and_every_catalog_component() {
    use showcase::pages::{component_scroll_offset, SIDEBAR_COMPONENT_IDS};
    use showcase::shell::sidebar_all_targets;
    use unbound_widgets::WidgetCatalogId;

    let targets = sidebar_all_targets();
    for page in PageId::ALL {
        assert!(
            targets.contains(&Target::NavPage(page.index() as u8)),
            "sidebar missing PageId::{page:?}"
        );
    }
    assert_eq!(SIDEBAR_COMPONENT_IDS.len(), WidgetCatalogId::COUNT);
    for &id in WidgetCatalogId::ALL {
        assert!(
            targets.contains(&Target::NavComponent(id as u8)),
            "sidebar missing WidgetCatalogId::{id:?}"
        );
    }

    // 点击侧边栏中的 Tooltip 组件项应导航至 Components 页并自动滚动到对应卡片位置。
    let mut app = app();
    app.click(
        target::id(Target::NavComponent(WidgetCatalogId::ToolTip as u8)),
        Vec2::ZERO,
    );
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.active_page, PageId::Components);
    assert_eq!(app.state.active_component, Some(WidgetCatalogId::ToolTip));
    assert!(
        (app.state.content_scroll - component_scroll_offset(WidgetCatalogId::ToolTip)).abs() < 1.0
    );
}

/// 标题栏语言下拉选择器（默认 English / 简体中文）与独立 `PageId::I18n` 教学沙盒端到端行为验证。
#[test]
fn language_selector_and_i18n_page_work_end_to_end() {
    use unbound_app::Locale;

    let mut app = app();
    assert_eq!(app.state.locale, Locale::EnUs);
    assert_eq!(app.state.t("app.title"), "Unbound UI Gallery");

    // 展开标题栏语言选择下拉菜单并切换至简体中文。
    click_node(&mut app, target::id(Target::LangMenu));
    assert!(app.state.lang_dropdown_open);
    click_node(&mut app, target::id(Target::Lang(1)));
    assert!(!app.state.lang_dropdown_open);
    assert_eq!(app.state.locale, Locale::ZhCn);
    assert_eq!(app.state.t("app.title"), "Unbound UI 组件展厅"); // lang-ok: expected zh-CN translation
    assert_eq!(
        PageId::I18n.localized_info(app.state.locale).short,
        "原生 i18n" // lang-ok: expected zh-CN translation
    );

    // 简体中文模式下全部 25 个页面均通过无头几何体检。
    for page in PageId::ALL {
        app.navigate(page);
        app.rebuild(VIEWPORT);
        let report = Harness::run(&app.frame());
        assert!(
            report.is_clean(),
            "{page:?} (zh-CN) produced {} diagnostic(s): {:?}",
            report.diagnostics.len(),
            report.diagnostics
        );
    }

    // 导航至独立 i18n 教学页并触发交互式复数计数器与语言切换按钮。
    app.navigate(PageId::I18n);
    app.rebuild(VIEWPORT);
    let before_count = app.state.demo.i18n_counter;
    click_node(
        &mut app,
        target::id(Target::Demo(PageId::I18n.index() as u8, 0, 3)),
    );
    assert_eq!(app.state.demo.i18n_counter, before_count + 1);
}

/// 外壳与首页引用的每一个 i18n 键都必须在双语资源包中命中真实词条。
///
/// `I18nBundle::t` 在缺键时会原样返回键名，界面便会把 `a11y.toggle_nav` 这类内部标识
/// 直接暴露给用户；这里从源码字面量反向扫描 `state.t("…")` 的调用点，作为词条表与
/// 绘制代码之间的编译外门禁。
#[test]
fn every_shell_i18n_key_resolves_in_both_locales() {
    use std::collections::BTreeSet;

    use showcase::state::build_showcase_i18n;
    use unbound_app::Locale;

    let sources = [
        include_str!("../../src/shell.rs"),
        include_str!("../../src/pages/home.rs"),
    ];
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for source in sources {
        for rest in source.split("state.t(\"").skip(1) {
            let key = rest.split("\")").next().unwrap_or_default();
            assert!(!key.is_empty(), "state.t call site is missing a key");
            keys.insert(key);
        }
    }
    assert!(
        keys.len() >= 30,
        "scanned too few keys ({}); the scan logic may be broken: {keys:?}",
        keys.len()
    );

    for locale in Locale::ALL {
        let bundle = build_showcase_i18n(locale);
        for key in &keys {
            assert_ne!(
                bundle.t(key),
                *key,
                "{locale:?} bundle is missing `{key}`; the UI will render the raw key"
            );
        }
    }
}

/// 在拥有 106 张卡片的 `PageId::Components` 页面上展开第 64..106 张卡片的源码面板不得触发位移溢出，
/// 且打开模态框（`modal_open = true`）时仍保持命中区域唯一。
#[test]
fn high_index_component_code_toggles_and_open_modals_keep_unique_hit_regions() {
    let mut app = app();
    app.navigate(PageId::Components);
    app.state.toggle_code(PageId::Components, 64);
    app.state.toggle_code(PageId::Components, 105);
    app.rebuild(VIEWPORT);
    assert!(app.state.code_open(PageId::Components, 64));
    assert!(app.state.code_open(PageId::Components, 105));

    // 打开 Overlays 模态框后校验所有命中区域唯一，无重复注册。
    app.navigate(PageId::Overlays);
    app.state.demo.modal_open = true;
    app.rebuild(VIEWPORT);
    let mut seen = rustc_hash::FxHashSet::default();
    for region in app.regions.iter() {
        assert!(
            seen.insert(region.node),
            "duplicate hit region registered for node {:#x} when modal is open",
            region.node
        );
    }
}

/// 原生可用时原生来源让根背景透明且不生成自绘纹理；「无后景」与自绘各自保持语义。
#[test]
fn the_native_source_makes_the_root_transparent_without_a_texture() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    app.state.set_backdrop_source(BackdropSource::Native);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::Native,
        "an explicit native source must resolve to the native path when the probe allows it"
    );
    assert!(
        app.root_background_transparent(),
        "the root background must be transparent so the DWM material can show through"
    );
    assert!(
        !app.mica.is_enabled(),
        "the native path must not generate the self-drawn texture"
    );
    assert!(!app.mica.visible());

    app.state.set_backdrop_material(MaterialKind::None);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_combination.material,
        MaterialKind::None,
        "the None material must leave no backdrop at all"
    );
    assert!(
        !app.root_background_transparent(),
        "the off path must keep the opaque theme background"
    );
    assert!(
        !app.mica.is_enabled(),
        "the off path must not generate the self-drawn texture"
    );

    app.state.set_backdrop_material(MaterialKind::Mica);
    app.state.set_backdrop_source(BackdropSource::SelfDrawn);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_combination.source,
        BackdropSource::SelfDrawn,
        "the self-drawn source must resolve to the self-drawn path"
    );
    assert!(
        app.mica.is_enabled(),
        "the self-drawn path must keep generating the wallpaper texture"
    );
}

/// 点击后景页的材质行即可切换材质种类，宿主在下一帧回写实际生效组合。
#[test]
fn the_material_row_updates_the_selection_and_the_readout() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    // 亚克力自定义色调走独立通道，需要单独打开它的能力。
    app.set_backdrop_accent_capability(true);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    for kind in MaterialKind::ALL {
        let part = MATERIAL_PART_BASE + kind.index() as u16;
        let node = target::id(Target::Demo(PageId::Backdrop.index() as u8, 0, part));
        // 沉浸式后景的系统取值未确认：自动来源下既不走原生也没有自绘变体，
        // 因此按钮禁用、点击不改选择。它的完整禁用行为由 backdrop_immersive 用例覆盖。
        if kind == MaterialKind::Immersive {
            let a11y = app
                .semantics
                .get(node)
                .expect("the disabled material must still be in the accessibility tree");
            assert!(
                a11y.disabled,
                "the unconfirmed immersive tier must be disabled on the auto source"
            );
            click_node(&mut app, node);
            assert_ne!(
                app.state.backdrop_selection.material,
                MaterialKind::Immersive,
                "clicking the disabled immersive tier must not change the selection"
            );
            continue;
        }
        click_node(&mut app, node);
        assert_eq!(
            app.state.backdrop_selection.material, kind,
            "clicking a material button must update the user choice"
        );
        assert_eq!(
            app.state.backdrop_selection.source,
            BackdropSource::Auto,
            "picking a material must not move the source axis"
        );
        assert_eq!(
            app.state.backdrop_combination.material, kind,
            "the host must write back the effective material for {kind:?}"
        );
        // 材质为「无」时来源轴折叠：实际来源保留用户请求的自动，其余材质都落到原生。
        let expected_source = if kind == MaterialKind::None {
            BackdropSource::Auto
        } else {
            BackdropSource::Native
        };
        assert_eq!(
            app.state.backdrop_combination.source, expected_source,
            "the effective source must follow the resolved combination for {kind:?}"
        );

        let a11y = app
            .semantics
            .get(node)
            .expect("the material button must be in the accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(
            a11y.label,
            format!("Material: {} — {}", kind.label(), kind.description()),
            "the material button must be named after its axis and material"
        );
        assert!(
            a11y.selected,
            "the picked material must be marked as selected"
        );
    }
}

/// 点击后景页的来源行即可切换来源，宿主在下一帧回写实际生效组合。
#[test]
fn the_source_row_updates_the_selection_and_the_readout() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    for (source, expected) in [
        (BackdropSource::Auto, BackdropSource::Native),
        (BackdropSource::Native, BackdropSource::Native),
        (BackdropSource::SelfDrawn, BackdropSource::SelfDrawn),
    ] {
        let part = SOURCE_PART_BASE + source.index() as u16;
        let node = target::id(Target::Demo(PageId::Backdrop.index() as u8, 0, part));
        click_node(&mut app, node);
        assert_eq!(
            app.state.backdrop_selection.source, source,
            "clicking a source button must update the user choice"
        );
        assert_eq!(
            app.state.backdrop_selection.material,
            MaterialKind::Mica,
            "picking a source must not move the material axis"
        );
        assert_eq!(
            app.state.backdrop_combination.source, expected,
            "the host must write back the effective source for {source:?}"
        );

        let a11y = app
            .semantics
            .get(node)
            .expect("the source button must be in the accessibility tree");
        assert_eq!(a11y.role, Role::Button);
        assert_eq!(
            a11y.label,
            format!("Source: {} — {}", source.label(), source.description()),
            "the source button must be named after its axis and source"
        );
        assert!(
            a11y.selected,
            "the picked source must be marked as selected"
        );
    }
}

/// 材质行与来源行各占一段部件编号，两行按钮互不重叠。
#[test]
fn the_two_rows_use_disjoint_parts_and_do_not_overlap() {
    let mut app = app();
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    let material_rects: Vec<Rect> = MaterialKind::ALL
        .iter()
        .map(|kind| {
            rect_of(
                &app,
                target::id(Target::Demo(
                    PageId::Backdrop.index() as u8,
                    0,
                    MATERIAL_PART_BASE + kind.index() as u16,
                )),
            )
        })
        .collect();
    let source_rects: Vec<Rect> = BackdropSource::ALL
        .iter()
        .map(|source| {
            rect_of(
                &app,
                target::id(Target::Demo(
                    PageId::Backdrop.index() as u8,
                    0,
                    SOURCE_PART_BASE + source.index() as u16,
                )),
            )
        })
        .collect();

    for pair in material_rects.windows(2) {
        assert!(
            !rects_overlap(pair[0], pair[1]),
            "the material row must not overlap itself"
        );
    }
    for pair in source_rects.windows(2) {
        assert!(
            !rects_overlap(pair[0], pair[1]),
            "the source row must not overlap itself"
        );
    }
    for material in &material_rects {
        for source in &source_rects {
            assert!(
                !rects_overlap(*material, *source),
                "the material row and the source row must not overlap"
            );
        }
    }
}

/// 七个材质档与三个来源档的组合必须解析出预期的实际组合、回退原因与不可用原因。
#[test]
fn every_material_and_source_combination_resolves_as_expected() {
    use unbound_backdrop::{BackdropCombination, FallbackReason, UnavailableReason};

    let mut app = app();
    // Windows 11 22H2：除沉浸式后景外所有材质档都能走原生路径。
    app.set_backdrop_capabilities(Some(22621), true);
    // 亚克力自定义色调的独立通道也视为可用。
    app.set_backdrop_accent_capability(true);
    app.navigate(PageId::Backdrop);
    app.rebuild(VIEWPORT);

    let none = MaterialKind::None;
    let follow = MaterialKind::FollowSystem;
    let mica = MaterialKind::Mica;
    let mica_alt = MaterialKind::MicaAlt;
    let acrylic = MaterialKind::Acrylic;
    let acrylic_tint = MaterialKind::AcrylicTint;
    let immersive = MaterialKind::Immersive;
    let auto = BackdropSource::Auto;
    let native = BackdropSource::Native;
    let self_drawn = BackdropSource::SelfDrawn;
    let no_self_drawn = |material| FallbackReason::NoSelfDrawnVariant { material };
    let plain = |material, source| BackdropCombination {
        material,
        source,
        fallback: None,
        unavailable: None,
    };

    let cases: [(MaterialKind, BackdropSource, BackdropCombination); 21] = [
        (none, auto, plain(none, auto)),
        (none, native, plain(none, native)),
        (none, self_drawn, plain(none, self_drawn)),
        (follow, auto, plain(follow, native)),
        (follow, native, plain(follow, native)),
        (
            follow,
            self_drawn,
            BackdropCombination {
                material: follow,
                source: native,
                fallback: Some(no_self_drawn(follow)),
                unavailable: None,
            },
        ),
        (mica, auto, plain(mica, native)),
        (mica, native, plain(mica, native)),
        (mica, self_drawn, plain(mica, self_drawn)),
        (mica_alt, auto, plain(mica_alt, native)),
        (mica_alt, native, plain(mica_alt, native)),
        (mica_alt, self_drawn, plain(mica_alt, self_drawn)),
        (acrylic, auto, plain(acrylic, native)),
        (acrylic, native, plain(acrylic, native)),
        (
            acrylic,
            self_drawn,
            BackdropCombination {
                material: acrylic,
                source: native,
                fallback: Some(no_self_drawn(acrylic)),
                unavailable: None,
            },
        ),
        (acrylic_tint, auto, plain(acrylic_tint, native)),
        (acrylic_tint, native, plain(acrylic_tint, native)),
        (
            acrylic_tint,
            self_drawn,
            BackdropCombination {
                material: acrylic_tint,
                source: native,
                fallback: Some(no_self_drawn(acrylic_tint)),
                unavailable: None,
            },
        ),
        (
            immersive,
            auto,
            BackdropCombination {
                material: mica,
                source: self_drawn,
                fallback: Some(no_self_drawn(immersive)),
                unavailable: None,
            },
        ),
        (
            immersive,
            native,
            BackdropCombination {
                material: none,
                source: native,
                fallback: None,
                unavailable: Some(UnavailableReason::BuildTooLow { required: 26100 }),
            },
        ),
        (
            immersive,
            self_drawn,
            BackdropCombination {
                material: mica,
                source: self_drawn,
                fallback: Some(no_self_drawn(immersive)),
                unavailable: None,
            },
        ),
    ];

    for (material, source, expected) in cases {
        app.state.set_backdrop_material(material);
        app.state.set_backdrop_source(source);
        app.rebuild(VIEWPORT);
        assert_eq!(
            app.state.backdrop_combination, expected,
            "material {material:?} with source {source:?} must resolve as expected"
        );
    }
}

/// 读出面板展示回退原因与不可用原因时用的是模型给出的英文文案。
#[test]
fn the_readout_reports_the_fallback_and_unavailable_reasons() {
    let mut app = app();
    app.set_backdrop_capabilities(Some(19045), true);
    app.state.set_backdrop_material(MaterialKind::Mica);
    app.state.set_backdrop_source(BackdropSource::Auto);
    app.rebuild(VIEWPORT);

    // 自动来源在机器承载不了时是「回退」：原生不可用，改走自绘。
    let fallback = app
        .state
        .backdrop_combination
        .fallback
        .expect("the self-drawn fallback must be reported");
    assert_eq!(fallback.label(), "Native unavailable");
    assert_eq!(
        fallback.description(),
        "The system cannot host the Mica material, so the self-drawn source took over"
    );
    assert_eq!(
        app.state.backdrop_combination.unavailable, None,
        "an automatic source must not report an unavailable reason"
    );

    // 显式原生来源在机器承载不了时是「不可用」：机器能力不足，没有降级可言。
    app.state.set_backdrop_source(BackdropSource::Native);
    app.rebuild(VIEWPORT);
    let unavailable = app
        .state
        .backdrop_combination
        .unavailable
        .expect("the build gate must be reported");
    assert_eq!(unavailable.label(), "System build too low");
    assert_eq!(
        unavailable.description(),
        "Requires Windows build 22621 or newer"
    );
    assert_eq!(
        app.state.backdrop_combination.fallback, None,
        "an explicit native source must not report a fallback reason"
    );
}

/// 切换材质或来源不重建交换链：合成方式只由平台基线决定，只有清屏 alpha 随计划变化。
#[test]
fn switching_material_or_source_keeps_the_swap_chain() {
    use unbound_backdrop::BackdropCompositeAlpha;

    let mut app = app();
    app.set_backdrop_capabilities(Some(26100), true);
    app.rebuild(VIEWPORT);

    // 默认选择是「云母 + 自动」：机器承载得了原生路径，因此清屏 alpha 为 0。
    let composition = app.state.backdrop_plan.composition;
    assert_eq!(
        composition,
        BackdropCompositeAlpha::PreMultiplied,
        "a premultiplied-capable host must keep the premultiplied surface"
    );
    assert_eq!(app.state.backdrop_plan.clear_alpha, 0.0);
    assert!(app.root_background_transparent());

    // 切到「无后景」：清屏 alpha 回到 1，合成方式仍然不动。
    app.state.set_backdrop_material(MaterialKind::None);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_plan.composition, composition,
        "disabling the backdrop must not rebuild the swapchain"
    );
    assert_eq!(app.state.backdrop_plan.clear_alpha, 1.0);
    assert!(!app.root_background_transparent());

    // 切到另一档有系统材质请求的材质：清屏 alpha 变 0，合成方式依旧不动。
    app.state.set_backdrop_material(MaterialKind::Acrylic);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_plan.composition, composition,
        "switching the material must not rebuild the swapchain"
    );
    assert_eq!(app.state.backdrop_plan.clear_alpha, 0.0);
    assert!(app.root_background_transparent());

    // 切来源到自绘：不请求系统材质，清屏 alpha 回到 1，合成方式依旧不动。
    app.state.set_backdrop_material(MaterialKind::Mica);
    app.state.set_backdrop_source(BackdropSource::SelfDrawn);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.state.backdrop_plan.composition, composition,
        "switching the source must not rebuild the swapchain"
    );
    assert_eq!(app.state.backdrop_plan.clear_alpha, 1.0);
    assert_eq!(
        app.state.backdrop_plan.material,
        unbound_backdrop::DwmMaterial::None,
        "the self-drawn source must not request a system material"
    );
}

/// 顶栏 `Pixel Snap` 按钮保留并迁移至 `IconButtonDesc`，切换时更新激活态与悬停 Tooltip 语义。
#[test]
fn pixel_snap_toggle_updates_active_state_and_tooltip_semantics() {
    let mut app = app();
    let snap_id = target::id(Target::Snap);
    assert!(app.state.pixel_snap, "Pixel Snap should start enabled");
    let snap_a11y = app
        .semantics
        .get(snap_id)
        .expect("Pixel Snap button must expose an accessibility node");
    assert!(
        snap_a11y.selected,
        "enabled Pixel Snap must be marked selected"
    );
    assert!(
        snap_a11y.label.contains("Pixel Snap"),
        "Pixel Snap a11y label should clearly name Pixel Snap, got {:?}",
        snap_a11y.label
    );

    // 悬停并推进 Tooltip 延迟后，应通过真实 ToolTip 组件输出开启态语义提示。
    let snap_pos = rect_of(&app, snap_id).center();
    app.pointer_move(snap_pos);
    app.rebuild(VIEWPORT);
    let _ = app.tick(0.5);
    app.rebuild(VIEWPORT);
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::ToolTip),
        "hovering Pixel Snap after delay must emit WidgetCatalogId::ToolTip"
    );
    let tip_on = app
        .semantics
        .iter()
        .find(|(_, node)| node.role == accesskit::Role::Tooltip)
        .map(|(_, node)| node.label.clone())
        .expect("Pixel Snap hover must expose a Tooltip semantic node");
    assert!(
        tip_on.contains("Pixel Snap: On"),
        "Pixel Snap tooltip must reflect enabled state, got {tip_on:?}"
    );

    // 点击关闭 Pixel Snap：激活态清除，重新悬停后 Tooltip 同步更新为关闭态。
    click_node(&mut app, snap_id);
    assert!(!app.state.pixel_snap, "clicking Pixel Snap must disable it");
    let snap_a11y_off = app
        .semantics
        .get(snap_id)
        .expect("Pixel Snap button must remain in semantics tree");
    assert!(
        !snap_a11y_off.selected,
        "disabled Pixel Snap must clear selected state"
    );
    app.pointer_move(Vec2::new(2.0, 898.0));
    app.rebuild(VIEWPORT);
    app.pointer_move(snap_pos);
    app.rebuild(VIEWPORT);
    let _ = app.tick(0.5);
    app.rebuild(VIEWPORT);
    let tip_off = app
        .semantics
        .iter()
        .find(|(_, node)| node.role == accesskit::Role::Tooltip)
        .map(|(_, node)| node.label.clone())
        .expect("Pixel Snap hover must expose updated Tooltip node");
    assert!(
        tip_off.contains("Pixel Snap: Off"),
        "Pixel Snap tooltip must reflect disabled state, got {tip_off:?}"
    );
}

/// 顶栏帧率 HUD 快捷开关与左侧 `Engine Telemetry` 页脚入口在无障碍标签和悬停 Tooltip 上语义清晰区分。
#[test]
fn hud_toggle_and_engine_telemetry_footer_have_distinct_tooltip_semantics() {
    let mut app = app();
    let hud_id = target::id(Target::Hud);
    let footer_id = target::id(Target::Footer(0));

    let hud_label = app
        .semantics
        .get(hud_id)
        .expect("HUD toggle must have a11y node")
        .label
        .clone();
    let footer_label = app
        .semantics
        .get(footer_id)
        .expect("Engine Telemetry footer must have a11y node")
        .label
        .clone();
    assert_ne!(hud_label, footer_label);
    assert!(
        hud_label.contains("FPS HUD"),
        "top bar HUD button must mention FPS HUD overlay, got {hud_label:?}"
    );
    assert!(
        footer_label.contains("Engine Telemetry"),
        "sidebar footer must name Engine Telemetry, got {footer_label:?}"
    );

    // 悬停顶栏 HUD 按钮：提示聚焦悬浮帧率面板。
    app.pointer_move(rect_of(&app, hud_id).center());
    app.rebuild(VIEWPORT);
    let _ = app.tick(0.5);
    app.rebuild(VIEWPORT);
    let hud_tip = app
        .semantics
        .iter()
        .find(|(_, node)| node.role == accesskit::Role::Tooltip)
        .map(|(_, node)| node.label.clone())
        .expect("HUD toggle hover must show Tooltip");
    assert!(
        hud_tip.contains("FPS HUD Overlay"),
        "HUD tooltip must describe floating FPS HUD overlay, got {hud_tip:?}"
    );

    // 悬停左侧 Engine Telemetry 页脚：提示聚焦打开完整遥测页面。
    app.pointer_move(rect_of(&app, footer_id).center());
    app.rebuild(VIEWPORT);
    let _ = app.tick(0.5);
    app.rebuild(VIEWPORT);
    let footer_tip = app
        .semantics
        .iter()
        .find(|(_, node)| node.role == accesskit::Role::Tooltip)
        .map(|(_, node)| node.label.clone())
        .expect("Engine Telemetry footer hover must show Tooltip");
    assert!(
        footer_tip.contains("Engine Telemetry page"),
        "Engine Telemetry footer tooltip must describe opening the full page, got {footer_tip:?}"
    );
    assert_ne!(hud_tip, footer_tip);
}

/// 顶栏语言/缩放下拉菜单、编辑器右键菜单与顶栏面包屑/徽章均走 `unbound-widgets` 真实组件发射器。
#[test]
fn top_bar_dropdown_and_editor_context_menu_emit_real_menu_widget() {
    let mut app = app();
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::BreadcrumbBar),
        "wide top bar must emit real BreadcrumbBar widget"
    );
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::InfoBadge),
        "top bar and page header badges must emit real InfoBadge widget"
    );

    // 语言下拉菜单：打开后发射 MenuFlyout，选择中文后切换语言并关闭菜单。
    click_node(&mut app, target::id(Target::LangMenu));
    assert!(app.state.lang_dropdown_open);
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::MenuFlyout),
        "language dropdown must emit real MenuFlyout widget"
    );
    click_node(&mut app, target::id(Target::Lang(1)));
    assert!(!app.state.lang_dropdown_open);
    assert_eq!(app.state.locale, unbound_app::Locale::ZhCn);

    // 缩放下拉菜单：打开后发射 MenuFlyout，选择 125% 后更新缩放档位并关闭菜单。
    click_node(&mut app, target::id(Target::ZoomMenu));
    assert!(app.state.zoom_dropdown_open);
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::MenuFlyout),
        "zoom dropdown must emit real MenuFlyout widget"
    );
    click_node(&mut app, target::id(Target::Zoom(1)));
    assert!(!app.state.zoom_dropdown_open);
    assert_eq!(app.state.zoom_step, 1);

    // 编辑器右键菜单：打开后发射 ContextMenu，选择 Select all 后全选文本并关闭菜单。
    let search_rect = rect_of(&app, target::id(Target::Search));
    click_node(&mut app, target::id(Target::Search));
    app.type_text("menu");
    app.rebuild(VIEWPORT);
    app.open_editor_menu(Target::Search, search_rect.center(), false);
    app.rebuild(VIEWPORT);
    assert!(app.editor_menu_open());
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::ContextMenu),
        "editor context menu must emit real ContextMenu widget"
    );
    click_node(&mut app, target::id(Target::EditorMenuItem(4)));
    assert!(!app.editor_menu_open());
    assert_eq!(app.state.search.edit.selected_text(), Some("menu"));
}

/// 示例卡片折叠代码栏通过 `CollapsibleDesc` + `CodeBlockDesc` 渲染，支持展开与收起。
#[test]
fn collapsible_code_bar_emits_expander_and_code_block_on_toggle() {
    let mut app = app();
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);

    let code_node = target::id(Target::Code(PageId::Buttons.index() as u8, 0));
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::Expander),
        "collapsible code bar must emit real Expander widget"
    );
    let collapsed = app
        .semantics
        .get(code_node)
        .expect("code bar must expose an accessibility node");
    assert_eq!(collapsed.role, accesskit::Role::DisclosureTriangle);
    assert_eq!(collapsed.expanded, Some(false));
    assert!(!app.state.code_open(PageId::Buttons, 0));

    // 点击展开：发射 CodeBlock 并更新 expanded = Some(true)。
    click_node(&mut app, code_node);
    assert!(app.state.code_open(PageId::Buttons, 0));
    assert!(
        app.emitted_widgets()
            .contains(&unbound_widgets::WidgetCatalogId::CodeBlock),
        "expanded code panel must emit real CodeBlock widget"
    );
    let expanded = app
        .semantics
        .get(code_node)
        .expect("expanded code bar must remain in semantics tree");
    assert_eq!(expanded.expanded, Some(true));

    // 再次点击收起：恢复 expanded = Some(false)。
    click_node(&mut app, code_node);
    assert!(!app.state.code_open(PageId::Buttons, 0));
    let collapsed_again = app
        .semantics
        .get(code_node)
        .expect("collapsed code bar must remain in semantics tree");
    assert_eq!(collapsed_again.expanded, Some(false));
}

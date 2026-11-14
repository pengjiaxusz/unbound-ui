//! 自举展厅外壳的行为检查。

use std::ops::{Deref, DerefMut};

use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_app::{AppKey, AppRuntime, Modifiers as AppModifiers};
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

    fn key(&mut self, key: &PlatformKey, modifiers: Modifiers) {
        if let Some(key) = app_key(key) {
            self.runtime.key(key, true, app_modifiers(modifiers));
        }
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

fn click_node(app: &mut App, node: u64) {
    let rect = rect_of(app, node);
    let point = rect.center();
    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);
    app.rebuild(VIEWPORT);
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
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(!app.state.nav_expanded);
    for _ in 0..120 {
        app.tick(1.0 / 60.0);
    }
    app.rebuild(VIEWPORT);
    assert!(app.layout.nav_width < expanded);
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(app.state.nav_expanded);
}

#[test]
fn theme_and_zoom_switches_are_one_click() {
    let mut app = app();
    click_node(&mut app, target::id(Target::Theme(0)));
    assert_eq!(app.state.theme, ThemeMode::Light);
    click_node(&mut app, target::id(Target::Zoom(2)));
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
    assert!(app.state.search_focused);
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
    assert!(!app.state.search_focused);
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

#[test]
fn search_and_theme_keep_the_same_target_identity() {
    assert_ne!(Target::Search, Target::Theme(0));
    let mut app = app();
    app.navigate(PageId::Buttons);
    app.rebuild(VIEWPORT);
    let clicked = app.state.demo.clicks;
    click_node(
        &mut app,
        target::id(Target::Demo(PageId::Buttons.index() as u8, 0, 1)),
    );
    assert!(app.state.demo.clicks > clicked);
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

/// 窄卡片会将选项面板折叠到舞台下方。
#[test]
fn a_narrow_card_folds_its_options_panel_below_the_stage() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.rebuild((760.0, 900.0));
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
    for width in [1440.0, 1200.0, 1000.0, 900.0, 800.0, 700.0, 1440.0] {
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
fn the_maximise_icon_keeps_its_primitive_count() {
    let mut app = app();
    app.set_window_maximized(false);
    app.rebuild(VIEWPORT);
    let restored = app.scene.primitives.len();

    app.set_window_maximized(true);
    app.rebuild(VIEWPORT);
    let maximized = app.scene.primitives.len();

    assert_eq!(
        restored, maximized,
        "the maximise icon changed the scene's shape: {restored} -> {maximized}"
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

/// 导航窗格的滚动条在跨越拖拽阈值后仍可拖动树。
#[test]
fn dragging_the_navigation_scrollbar_scrolls_the_tree() {
    let mut app = app();
    // 较矮的窗口确保树溢出窗格。
    app.rebuild((1440.0, 520.0));
    let node = target::id(Target::NavScrollbar);
    let bar = rect_of(&app, node);
    assert_eq!(app.state.nav_scroll, 0.0);

    let grab = Vec2::new(bar.center().x, bar.origin.y + 6.0);
    app.pointer_move(grab);
    app.pointer_down(grab, 0.0);
    let mut previous = 0.0;
    for step in 1..=8 {
        let point = Vec2::new(grab.x, grab.y + 15.0 * step as f32);
        app.pointer_move(point);
        let scroll = app.state.nav_scroll;
        assert!(
            scroll >= previous,
            "step {step}: nav scroll {scroll} must not regress from {previous}"
        );
        previous = scroll;
    }
    app.pointer_up(Vec2::new(grab.x, grab.y + 120.0), 0.1);
    assert!(
        app.state.nav_scroll > 0.0,
        "dragging the nav scrollbar must scroll the tree"
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

/// 侧边栏折叠与展开过程严格单调收敛，无欠阻尼过冲或水平位移抖动。
#[test]
fn sidebar_collapse_and_expand_is_monotonic_without_jitter() {
    let mut app = app();
    let initial = app.layout.nav_width;
    assert!((initial - 248.0).abs() < 0.01);

    // 触发折叠：宽度必须单调递减至 68.0，不得反弹或过冲。
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(!app.state.nav_expanded);
    let mut prev = app.layout.nav_width;
    for step in 0..90 {
        app.tick(1.0 / 60.0);
        app.rebuild(VIEWPORT);
        let cur = app.layout.nav_width;
        assert!(
            cur <= prev + 1e-4,
            "collapse step {step} jittered upward: {prev} -> {cur}"
        );
        if cur < 140.0 {
            let home_rect = rect_of(
                &app,
                target::id(Target::NavPage(PageId::Home.index() as u8)),
            );
            assert!(
                (home_rect.center().x - (app.layout.nav.origin.x + 34.0)).abs() < 0.01,
                "collapsed icon center shifted horizontally during collapse: {:?}",
                home_rect
            );
        }
        prev = cur;
    }
    assert!((app.layout.nav_width - 68.0).abs() < 0.5);

    // 触发展开：宽度必须单调递增至 248.0，不得过冲或回弹。
    click_node(&mut app, target::id(Target::NavToggle));
    assert!(app.state.nav_expanded);
    let mut prev = app.layout.nav_width;
    for step in 0..90 {
        app.tick(1.0 / 60.0);
        app.rebuild(VIEWPORT);
        let cur = app.layout.nav_width;
        assert!(
            cur >= prev - 1e-4,
            "expand step {step} jittered downward: {prev} -> {cur}"
        );
        prev = cur;
    }
    assert!((app.layout.nav_width - 248.0).abs() < 0.5);
}

/// 侧边栏导航目标 100% 覆盖全部 24 个页面与全部 91 个目录组件，且点击组件导航项可精确定位。
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
        target::id(Target::NavComponent(WidgetCatalogId::Tooltip as u8)),
        Vec2::ZERO,
    );
    app.rebuild(VIEWPORT);
    assert_eq!(app.state.active_page, PageId::Components);
    assert_eq!(app.state.active_component, Some(WidgetCatalogId::Tooltip));
    assert!(
        (app.state.content_scroll - component_scroll_offset(WidgetCatalogId::Tooltip)).abs() < 1.0
    );
}

/// `PageId::Systems` 引擎子系统全景页覆盖全部 21 个核心子系统，且支持点击切换当前检视的子系统。
#[test]
fn systems_page_covers_all_21_engine_subsystems() {
    use showcase::pages::SubsystemId;

    assert_eq!(SubsystemId::ALL.len(), 21);
    let mut app = app();
    app.navigate(PageId::Systems);
    app.rebuild(VIEWPORT);

    let node = target::id(Target::Demo(PageId::Systems.index() as u8, 0, 105));
    click_node(&mut app, node);
    assert_eq!(app.state.demo.selected_subsystem, 5);
}

//! Behavioural checks for the dogfooded gallery shell.

use showcase::pages::PageId;
use showcase::shell::GalleryApp;
use showcase::state::ThemeMode;
use showcase::target::{self, Target};
use unbound_headless::Harness;
use unbound_math::Vec2;
use unbound_platform::{Modifiers, PlatformKey};

const VIEWPORT: (f32, f32) = (1440.0, 900.0);

fn app() -> GalleryApp {
    let mut app = GalleryApp::new();
    app.state.theme = ThemeMode::Dark;
    app.rebuild(VIEWPORT);
    app
}

fn rect_of(app: &GalleryApp, node: u64) -> unbound_math::Rect {
    app.router
        .regions()
        .iter()
        .find(|region| region.node == node)
        .map(|region| region.bounds)
        .unwrap_or_else(|| panic!("no hit region for node {node:#x}"))
}

fn click_node(app: &mut GalleryApp, node: u64) {
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
        let mut app = GalleryApp::new();
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
                let mut app = GalleryApp::new();
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
    // The second navigation row is Design Guidance.
    app.pointer_down(Vec2::new(120.0, 122.0), 0.0);
    app.pointer_up(Vec2::new(120.0, 122.0), 0.1);
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

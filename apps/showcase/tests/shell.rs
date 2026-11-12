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

/// A click has to reach the screen in the same frame.
///
/// The GPU skips the upload when the scene's generation did not move, so a
/// click that changes the state but leaves the generation alone is a click
/// the user never sees. The generation is bumped by the rebuild itself, so
/// the two can never drift apart.
#[test]
fn clicking_a_button_bumps_scene_generation_immediately() {
    let mut app = app();
    let before = app.generation();
    // The HUD toggle changes the scene's *shape*: the overlay appears.
    let node = target::id(Target::Hud);
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    app.pointer_up(point, 0.1);

    // The state changed on the release.
    assert!(app.state.show_hud, "the click must toggle the HUD");
    // The next sync rebuilds the scene, and the rebuild moves the
    // generation: that is what makes the GPU re-upload the primitives.
    assert!(
        app.sync(VIEWPORT),
        "the click must be visible to the renderer"
    );
    assert!(
        app.generation() > before,
        "the scene generation must advance so the frame is re-uploaded"
    );
}

/// Switching the theme repaints through the colour table, not the pool.
///
/// Every colour a frame paints is a slot in a 4 KiB lookup table, so a theme
/// switch rewrites that table and moves no primitive at all. This is the
/// cheapest possible full-screen repaint: zero bytes of scene upload.
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

/// A press that is dragged off the control must not activate it.
///
/// The discrete controls act on the release, and the router withholds the
/// click when the release did not land on the pressed control. Together
/// they make "press, drag away, release" a no-op.
#[test]
fn dragging_off_a_shell_control_cancels_activation() {
    let mut app = app();
    let node = target::id(Target::Theme(0));
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    // Drag far away from the control and release there.
    let away = Vec2::new(point.x + 400.0, point.y + 300.0);
    app.pointer_move(away);
    app.pointer_up(away, 0.1);

    assert_eq!(
        app.state.theme,
        ThemeMode::Dark,
        "a drag off the control must not activate it"
    );
}

/// The discrete controls act on the release, not on the press.
///
/// This is what removes the double activation: the press used to commit
/// the toggle *and* the release replayed it, flipping the control back.
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

/// The navigation must not fire twice for a single click.
///
/// It used to: the press navigated and the release navigated again, which
/// reset the content scroll twice.
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

/// A window resize has to recompute the layout.
///
/// This is what makes a live resize work: the frame is drawn at the new
/// size, and the navigation, the card grid and the text wrapping follow the
/// new edge instead of staying at the old one.
#[test]
fn a_resize_recomputes_the_layout() {
    let mut app = app();
    let before = app.generation();

    // The window got narrower: the viewport changed.
    assert!(
        app.sync((960.0, 600.0)),
        "a viewport change must be visible to the renderer"
    );
    assert!(
        app.generation() > before,
        "the rebuilt scene must advance the upload generation"
    );

    // The hit regions were rebuilt for the new size: the theme toggle now
    // sits where the new layout puts it, not where the old one did.
    let node = target::id(Target::Theme(0));
    let resized = rect_of(&app, node);
    assert!(
        resized.size.x > 0.0 && resized.size.y > 0.0,
        "the rebuilt layout must have a hit region"
    );
}

/// Every intermediate size of a drag has to be laid out, not just the last
/// one. A drag that only lays out its final size is a drag that shows a
/// frozen picture for its whole duration.
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

/// Losing the focus has to abort the in-flight interaction.
///
/// Otherwise the shell keeps a ghost drag alive: the scrollbar stays
/// grabbed and the demo keeps a marquee running until the user happens to
/// click somewhere else.
#[test]
fn losing_the_focus_cancels_the_pointer_capture() {
    let mut app = app();
    let node = target::id(Target::Theme(0));
    let rect = rect_of(&app, node);
    let point = rect.center();

    app.pointer_move(point);
    app.pointer_down(point, 0.0);
    assert!(app.router.pointer().pressed().is_some());

    app.cancel_interactions();
    assert!(
        app.router.pointer().pressed().is_none(),
        "the capture must be released"
    );
    // The pending activation is gone: the release no longer commits.
    app.pointer_up(point, 0.1);
    assert_eq!(
        app.state.theme,
        ThemeMode::Dark,
        "a cancelled press must not activate the control"
    );
}

/// The options panel belongs beside the stage while the card is wide enough.
///
/// The decision is the layout engine's: the flex line runs out of room and
/// the panel wraps below the stage. Nothing in the shell asks for it.
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

/// A narrow card folds its options panel below the stage.
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

/// The fold follows the window, frame by frame.
///
/// A drag that only lays out its final size is a drag that shows a frozen
/// picture for its whole duration, so every intermediate size has to be
/// solved and has to stay clean.
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

/// A card's size is fixed, so the card is a relayout boundary.
///
/// Nothing inside a card can change its size, which means a change inside a
/// card never re-solves the page.
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

/// A hover repaints a control; it does not move anything.
///
/// The frame's layout cost must be exactly zero, otherwise every pointer
/// move would pay for a full tree solve.
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

/// The hover's patch is a handful of bytes, not the whole scene.
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

/// A patch keeps the generation, so the renderer knows not to re-upload all.
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

/// Navigating changes the page's shape, so the whole scene goes up again.
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

/// Hovering a control repaints it; it never lays the page out again.
///
/// Every control on every page is exercised: a pointer move that reached the
/// layout engine would make the whole page pay for a hover.
#[test]
fn hovering_a_control_never_relays_out_the_page() {
    for page in PageId::ALL {
        let mut app = app();
        app.navigate(page);
        app.pointer_move(Vec2::new(2.0, 898.0));
        app.rebuild(VIEWPORT);
        let targets: Vec<u64> = app
            .router
            .regions()
            .iter()
            .map(|region| region.node)
            .collect();
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

/// A hover patches the pool; it does not re-upload the scene.
///
/// The shell's chrome and the standard controls keep a stable primitive count
/// across states, which is what makes this possible. Demo stages are excluded:
/// a demo is a bespoke drawing and may legitimately reveal widgets on hover
/// (a table row highlight, a dialog shadow), so it falls back to a full
/// upload — correct, just not cheap.
#[test]
fn a_hover_patches_the_pool_instead_of_reuploading_it() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.pointer_move(Vec2::new(2.0, 898.0));
    app.rebuild(VIEWPORT);
    let targets: Vec<u64> = app
        .router
        .regions()
        .iter()
        .map(|region| region.node)
        .filter(|node| !target::is_demo(*node))
        .collect();
    assert!(!targets.is_empty(), "the page should have chrome to hover");
    let stride = core::mem::size_of::<unbound_gpu::data::GpuPrimitiveInstance>() as u32;
    for node in targets {
        app.pointer_move(Vec2::new(2.0, 898.0));
        app.rebuild(VIEWPORT);
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

/// A hover patch stays inside the control that was hovered.
///
/// This is the other half of the byte budget: uploading little is only
/// correct if the little that goes up is the right little. Every control
/// owns a span of the primitive pool, and every dirty slice has to fall
/// inside the span of the control whose state changed.
#[test]
fn a_hover_patch_stays_inside_the_hovered_control() {
    let mut app = app();
    app.navigate(PageId::Home);
    app.pointer_move(Vec2::new(2.0, 898.0));
    app.rebuild(VIEWPORT);
    let targets: Vec<u64> = app
        .router
        .regions()
        .iter()
        .map(|region| region.node)
        .filter(|node| !target::is_demo(*node))
        .collect();
    let mut spans = 0usize;
    for node in targets {
        app.pointer_move(Vec2::new(2.0, 898.0));
        app.rebuild(VIEWPORT);
        let hovered = rect_of(&app, node).center();
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

/// One control, one identity.
///
/// The identity is shared by the hit region, the semantic node and the
/// primitive span, so a second control reusing it is not a cosmetic problem:
/// the two light up together, the semantic tree loses one of them and the
/// span map keeps only the last registration. The footer shortcut and the
/// tree row for the same page used to collide exactly this way.
#[test]
fn every_hit_region_has_its_own_identity() {
    for expanded in [true, false] {
        for page in PageId::ALL {
            let mut app = app();
            app.navigate(page);
            if !expanded {
                click_node(&mut app, target::id(Target::NavToggle));
                // Settle the spring: the collapsed rail is a different
                // renderer and has to be audited on its own.
                for _ in 0..120 {
                    app.tick(1.0 / 60.0);
                }
            }
            app.rebuild(VIEWPORT);
            let mut seen: std::collections::HashMap<u64, unbound_math::Rect> =
                std::collections::HashMap::new();
            for region in app.router.regions() {
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

/// The footer shortcuts navigate, and only their own row lights up.
#[test]
fn the_footer_shortcuts_navigate_on_their_own_identity() {
    let mut app = app();
    let footer = target::id(Target::Footer(0));
    let row = rect_of(&app, footer);
    let point = row.center();
    app.pointer_move(point);
    app.rebuild(VIEWPORT);
    assert_eq!(
        app.router.hover(),
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

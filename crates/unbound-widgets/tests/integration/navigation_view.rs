//! `NavigationView` 完整无头行为、布局、浮层、层级、动效、键盘与无障碍契约测试。

use accesskit::Role;
use unbound_gpu::data::PrimType;
use unbound_input::{KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine};
use unbound_widgets::scroll::{
    scrollbar_thumb_node, scrollbar_track_node, ScrollBarVisibility, ScrollbarAxis,
    SCROLLBAR_EDGE_INSET, SCROLLBAR_HOLD, SCROLLBAR_IDLE_OPACITY, SCROLLBAR_THUMB_IDLE_WIDTH,
    SCROLLBAR_THUMB_WIDTH, SCROLLBAR_TRACK_CAPSULE_OPACITY, SCROLLBAR_WIDTH,
};
use unbound_widgets::{
    emit_navigation_view, ControlState, NavigationViewBackButtonVisible, NavigationViewDesc,
    NavigationViewDisplayMode, NavigationViewEvent, NavigationViewItem, NavigationViewMenuItem,
    NavigationViewPaneDisplayMode, NavigationViewState, PointerPhase, ShellIcon, TextCtx, UiWidget,
    WidgetInputEvent, NAVIGATION_VIEW_BACK_BUTTON_ID, NAVIGATION_VIEW_COMPACT_PANE_LENGTH,
    NAVIGATION_VIEW_OPEN_PANE_LENGTH, NAVIGATION_VIEW_OVERFLOW_BUTTON_ID,
    NAVIGATION_VIEW_PANE_PADDING, NAVIGATION_VIEW_PANE_TOGGLE_ID, NAVIGATION_VIEW_SEARCH_BUTTON_ID,
    NAVIGATION_VIEW_SETTINGS_ID,
};

fn sample_desc(rect: Rect, mode: NavigationViewPaneDisplayMode) -> NavigationViewDesc {
    let menu = vec![
        NavigationViewMenuItem::header("Workspace"),
        NavigationViewMenuItem::item(NavigationViewItem::new(1, "Home").with_icon(ShellIcon::Grid)),
        NavigationViewMenuItem::item(
            NavigationViewItem::new(2, "Collections")
                .with_icon(ShellIcon::Layers)
                .selects_on_invoked(false)
                .with_children(vec![
                    NavigationViewMenuItem::item(
                        NavigationViewItem::new(21, "Notes").with_icon(ShellIcon::Code),
                    ),
                    NavigationViewMenuItem::item(
                        NavigationViewItem::new(22, "Mail")
                            .with_icon(ShellIcon::Bolt)
                            .with_info_badge("3"),
                    ),
                ]),
        ),
        NavigationViewMenuItem::separator(),
        NavigationViewMenuItem::item(
            NavigationViewItem::new(3, "Analytics").with_icon(ShellIcon::Chart),
        ),
    ];

    NavigationViewDesc::new(rect, menu)
        .pane_display_mode(mode)
        .footer_menu_items(vec![NavigationViewMenuItem::item(
            NavigationViewItem::new(90, "Account").with_icon(ShellIcon::Sliders),
        )])
        .selected_item(Some(1))
        .header("Home")
}

#[test]
fn constant_primitive_count_across_all_24_control_states() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Left,
    );
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };
    let baseline = emit_navigation_view(7, &desc, ControlState::default(), &state, &mut ctx)
        .output
        .primitives
        .len();

    for phase in [
        PointerPhase::Idle,
        PointerPhase::Hovered,
        PointerPhase::Pressed,
    ] {
        for focused in [false, true] {
            for disabled in [false, true] {
                for selected in [false, true] {
                    let control = ControlState {
                        phase,
                        focused,
                        disabled,
                        selected,
                    };
                    let out = emit_navigation_view(7, &desc, control, &state, &mut ctx);
                    assert_eq!(
                        out.output.primitives.len(),
                        baseline,
                        "primitive count drifted for state {control:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn auto_mode_switches_breakpoints_and_emits_display_mode_changed() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Auto,
    );
    let mut state = NavigationViewState::from_desc(&desc);
    assert_eq!(state.display_mode, NavigationViewDisplayMode::Expanded);
    assert!(state.is_pane_open);

    state.sync_viewport(&desc, 800.0);
    assert_eq!(state.display_mode, NavigationViewDisplayMode::Compact);
    assert!(!state.is_pane_open);
    assert!(state
        .drain_events()
        .contains(&NavigationViewEvent::DisplayModeChanged {
            display_mode: NavigationViewDisplayMode::Compact,
        }));

    state.sync_viewport(&desc, 500.0);
    assert_eq!(state.display_mode, NavigationViewDisplayMode::Minimal);
    assert!(!state.is_pane_open);
    assert!(state
        .drain_events()
        .contains(&NavigationViewEvent::DisplayModeChanged {
            display_mode: NavigationViewDisplayMode::Minimal,
        }));
}

#[test]
fn compact_and_minimal_overlays_emit_backdrop_and_light_dismiss() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 800.0, 600.0),
        NavigationViewPaneDisplayMode::LeftCompact,
    )
    .is_pane_open(false)
    .is_back_enabled(true);
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };

    // 折叠态无 Overlay 毛玻璃请求。
    let closed = emit_navigation_view(11, &desc, ControlState::default(), &state, &mut ctx);
    assert!(closed.backdrop.is_none());
    assert_eq!(
        closed.layout.pane_rect.width(),
        NAVIGATION_VIEW_COMPACT_PANE_LENGTH
    );

    // 打开 Overlay 浮层：发射 BackdropRequest 且返回键优先关闭浮层而非触发 BackRequested。
    state.toggle_pane(&desc);
    assert!(state.is_pane_open);
    let open = emit_navigation_view(11, &desc, ControlState::default(), &state, &mut ctx);
    assert!(open.backdrop.is_some());
    assert!(open.layout.overlay_rect.is_some());
    assert_eq!(
        open.layout.overlay_rect.unwrap().width(),
        NAVIGATION_VIEW_OPEN_PANE_LENGTH
    );

    // 点击 Overlay 外部区域触发轻闭。
    let dismissed = state.handle_pointer_down(&desc, &open.layout, Vec2::new(600.0, 300.0));
    assert!(dismissed);
    assert!(!state.is_pane_open);
    assert!(state
        .drain_events()
        .contains(&NavigationViewEvent::PaneClosing));

    // 再次打开后按返回键：只关闭浮层，不发 BackRequested。
    state.set_pane_open(true, &desc);
    state.drain_events();
    assert!(state.request_back(&desc));
    assert!(!state.is_pane_open);
    let events = state.drain_events();
    assert!(events.contains(&NavigationViewEvent::PaneClosing));
    assert!(!events.contains(&NavigationViewEvent::BackRequested));

    // 浮层已关闭时再按返回键：发出 BackRequested。
    assert!(state.request_back(&desc));
    assert!(state
        .drain_events()
        .contains(&NavigationViewEvent::BackRequested));
}

#[test]
fn top_mode_overflows_on_narrow_viewport_and_opens_popup() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 260.0, 400.0),
        NavigationViewPaneDisplayMode::Top,
    );
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);

    let layout = desc.compute_layout(&state);
    assert!(layout.is_top);
    assert!(layout.overflow_button_rect.is_some());

    assert!(state.invoke_item(&desc, NAVIGATION_VIEW_OVERFLOW_BUTTON_ID));
    assert!(state.overflow_menu_open);

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };
    let emitted = emit_navigation_view(12, &desc, ControlState::default(), &state, &mut ctx);
    assert!(emitted.layout.overflow_popup_rect.is_some());
    assert!(!emitted.layout.overflow_rows.is_empty());
    assert!(emitted.backdrop.is_some());
}

#[test]
fn hierarchical_ancestor_selection_fallback_and_compact_flyout() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Left,
    );
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);

    // 父项 selects_on_invoked = false：点击只展开，不改选中项。
    assert!(state.invoke_item(&desc, 2));
    assert!(state.is_item_expanded(2));
    assert_eq!(state.selected_item_id, Some(1));
    assert!(state
        .drain_events()
        .contains(&NavigationViewEvent::Expanding { item_id: 2 }));

    // 选中子项 22，再折叠父项 2：有效指示条回退到父项 2。
    assert!(state.invoke_item(&desc, 22));
    assert_eq!(state.selected_item_id, Some(22));
    assert_eq!(state.effective_selected_id(&desc), Some(22));

    assert!(state.invoke_item(&desc, 2));
    assert!(!state.is_item_expanded(2));
    assert_eq!(state.effective_selected_id(&desc), Some(2));

    // 重新展开后指示条归位到 22。
    assert!(state.invoke_item(&desc, 2));
    assert_eq!(state.effective_selected_id(&desc), Some(22));

    // LeftCompact 折叠态点击父项 2 弹出子菜单 Flyout。
    let compact = sample_desc(
        Rect::new(0.0, 0.0, 800.0, 600.0),
        NavigationViewPaneDisplayMode::LeftCompact,
    )
    .is_pane_open(false);
    let mut cstate = NavigationViewState::from_desc(&compact);
    cstate.set_animations_enabled(false, &compact);
    assert!(cstate.invoke_item(&compact, 2));
    assert_eq!(cstate.open_flyout_parent_id, Some(2));
    let clayout = compact.compute_layout(&cstate);
    assert!(clayout.flyout_rect.is_some());
    assert_eq!(clayout.flyout_rows.len(), 2);
}

#[test]
fn independent_scroll_search_expand_title_bar_sync_and_keyboard() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 800.0, 600.0),
        NavigationViewPaneDisplayMode::LeftCompact,
    )
    .is_pane_open(false)
    .with_auto_suggest_box("Search pages")
    .is_back_button_visible(NavigationViewBackButtonVisible::Visible);
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);

    // 紧凑态点击搜索图标自动展开窗格并聚焦搜索框。
    assert!(state.invoke_item(&desc, NAVIGATION_VIEW_SEARCH_BUTTON_ID));
    assert!(state.is_pane_open);
    assert!(state.search_focused);

    // TitleBar 联动在 Top 模式自动隐藏窗格折叠按钮。
    let top_desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Top,
    );
    assert!(
        !state
            .title_bar_sync(&top_desc)
            .is_pane_toggle_button_visible
    );
    assert!(state.title_bar_sync(&desc).is_pane_toggle_button_visible);

    // 键盘方向键与 Enter 激活，以及 AccessKit 语义角色校验。
    let left_desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Left,
    );
    let mut kstate = NavigationViewState::from_desc(&left_desc);
    kstate.set_animations_enabled(false, &left_desc);
    let layout = left_desc.compute_layout(&kstate);
    assert!(kstate.handle_key_down(
        &left_desc,
        &layout,
        KeyChord::new(KeyName::Named("ArrowDown"))
    ));
    assert_eq!(kstate.focused_item_id, Some(2));

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };
    let emitted = emit_navigation_view(99, &left_desc, ControlState::default(), &kstate, &mut ctx);
    assert_eq!(emitted.output.a11y.as_ref().unwrap().role, Role::Navigation);
    assert!(emitted
        .output
        .sub_a11y
        .iter()
        .any(|(_, n)| n.role == Role::TreeItem && n.label == "Collections"));
    assert!(emitted
        .output
        .sub_a11y
        .iter()
        .any(|(_, n)| n.role == Role::Tab && n.label == "Settings"));
    const { assert!(NAVIGATION_VIEW_SETTINGS_ID != NAVIGATION_VIEW_BACK_BUTTON_ID) };
    const { assert!(NAVIGATION_VIEW_PANE_TOGGLE_ID != NAVIGATION_VIEW_SETTINGS_ID) };
}

#[test]
fn spring_motion_ticks_and_snaps_deterministically() {
    let desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Left,
    );
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(true, &desc);
    state.set_pane_open(false, &desc);
    assert!(state.tick(0.016, &desc));
    assert!(state.pane_width_spring.value < NAVIGATION_VIEW_OPEN_PANE_LENGTH);
    state.snap_animations(&desc);
    assert_eq!(
        state.pane_width_spring.value,
        NAVIGATION_VIEW_COMPACT_PANE_LENGTH
    );
}

#[test]
fn pane_collapse_and_expand_animation_keeps_icons_anchored_and_fades_text_monotonically() {
    let rect = Rect::new(20.0, 10.0, 1200.0, 720.0);
    let desc = sample_desc(rect, NavigationViewPaneDisplayMode::Left)
        .with_auto_suggest_box("Search")
        .pane_title("Menu");
    let expected_icon_x = rect.origin.x + NAVIGATION_VIEW_PANE_PADDING + 12.0;

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };

    // 辅助函数：提取 `Home` 项的 `ShellIcon::Grid` 首个 5.6x5.6 方块反算 `icon_x`，
    // 以及所有位于文字区（x >= expected_icon_x + 28.0 且在窗格内）的字形平均不透明度与最大右缘。
    let sample_frame = |state: &NavigationViewState, ctx: &mut TextCtx<'_>| {
        let emitted = emit_navigation_view(51, &desc, ControlState::default(), state, ctx);
        let pane_right = emitted.layout.pane_rect.max().x;
        let home_row = emitted
            .layout
            .rows
            .iter()
            .find(|r| r.item_id() == Some(1))
            .expect("Home row must exist");

        let grid_cell = emitted
            .output
            .primitives
            .iter()
            .find(|p| {
                (p.size[0] - 5.6).abs() < 0.05
                    && (p.size[1] - 5.6).abs() < 0.05
                    && p.local_pos[1] >= home_row.rect.origin.y
                    && p.local_pos[1] <= home_row.rect.max().y
            })
            .expect("Home ShellIcon::Grid primitive must be emitted");
        // ShellIcon::Grid 左上角方块原点为 `c.x - 6.5 = (icon_x + 8.0) - 6.5 = icon_x + 1.5`。
        let primary_icon_x = grid_cell.local_pos[0] - 1.5;

        let search_icon_x = emitted.layout.search_rect.and_then(|sr| {
            emitted
                .output
                .primitives
                .iter()
                .find(|p| {
                    (p.size[0] - 11.5).abs() < 0.05
                        && (p.size[1] - 11.5).abs() < 0.05
                        && p.local_pos[1] >= sr.origin.y
                        && p.local_pos[1] <= sr.max().y
                })
                .map(|p| {
                    // ShellIcon::Search 外圆原点为 `c.x - 7.5 = (icon_x + 8.0) - 7.5 = icon_x + 0.5`。
                    p.local_pos[0] - 0.5
                })
        });

        let text_glyphs: Vec<(f32, f32)> = emitted
            .output
            .primitives
            .iter()
            .filter(|p| {
                p.prim_type() == PrimType::Glyph
                    && p.local_pos[0] >= expected_icon_x + 28.0
                    && p.local_pos[0] < pane_right + 1.0
                    && p.local_pos[1] >= home_row.rect.origin.y
                    && p.local_pos[1] <= home_row.rect.max().y
            })
            .map(|p| (p.opacity(), p.local_pos[0] + p.size[0]))
            .collect();

        (
            emitted.layout.pane_expand_progress,
            pane_right,
            primary_icon_x,
            search_icon_x,
            text_glyphs,
        )
    };

    let compact = NAVIGATION_VIEW_COMPACT_PANE_LENGTH;
    let open = NAVIGATION_VIEW_OPEN_PANE_LENGTH;

    // 1. 展开 -> 折叠：第 0 帧刚点击折叠时（is_pane_open = false 但宽度仍为 open），文字不得硬消失、图标不得瞬移。
    let mut collapsing = NavigationViewState::from_desc(&desc);
    collapsing.set_animations_enabled(true, &desc);
    collapsing.set_pane_open(false, &desc);
    let (t0, _, icon_x_0, _, glyphs_0) = sample_frame(&collapsing, &mut ctx);
    assert!((t0 - 1.0).abs() < 1e-4);
    assert!((icon_x_0 - expected_icon_x).abs() < 1e-3);
    assert!(
        !glyphs_0.is_empty(),
        "frame 0 after collapse click must not hard-hide labels"
    );

    let mut collapse_opacities = Vec::new();
    for t in [0.75_f32, 0.5, 0.25] {
        collapsing.pane_width_spring.value = compact + (open - compact) * t;
        let (progress, pane_right, icon_x, search_x, glyphs) = sample_frame(&collapsing, &mut ctx);
        assert!((progress - t).abs() < 1e-4);
        assert!(
            (icon_x - expected_icon_x).abs() < 1e-3,
            "primary icon drifted horizontally at collapse t={t}: got {icon_x}, expected {expected_icon_x}"
        );
        let search_x = search_x.expect("search icon must remain anchored during transition");
        assert!(
            (search_x - expected_icon_x).abs() < 1e-3,
            "search icon drifted horizontally at collapse t={t}: got {search_x}, expected {expected_icon_x}"
        );
        assert!(
            !glyphs.is_empty(),
            "labels must still fade smoothly at t={t}"
        );
        for &(op, right_x) in &glyphs {
            assert!(
                (op - t).abs() < 0.02,
                "label opacity at t={t} expected ~{t}, got {op}"
            );
            assert!(
                right_x <= pane_right + 0.5,
                "label glyph right edge {right_x} overflowed pane right {pane_right} at t={t}"
            );
        }
        collapse_opacities.push(glyphs[0].0);
    }
    assert!(
        collapse_opacities[0] > collapse_opacities[1]
            && collapse_opacities[1] > collapse_opacities[2],
        "collapse text opacity must decrease monotonically: {collapse_opacities:?}"
    );

    // 折叠终态（t = 0.0）：图标仍在原位，文字完全不绘制。
    collapsing.pane_width_spring.value = compact;
    let (t_end, _, icon_x_end, search_x_end, glyphs_end) = sample_frame(&collapsing, &mut ctx);
    assert!(t_end <= 0.01);
    assert!((icon_x_end - expected_icon_x).abs() < 1e-3);
    assert!((search_x_end.expect("compact search icon") - expected_icon_x).abs() < 1e-3);
    assert!(
        glyphs_end.is_empty(),
        "labels must not be drawn when t <= 0.01"
    );

    // 2. 折叠 -> 展开：在 t = 0.25 / 0.5 / 0.75 一级图标恒定且文字不透明度严格单调递增。
    let mut expanding = NavigationViewState::from_desc(&desc);
    expanding.set_animations_enabled(false, &desc);
    expanding.set_pane_open(false, &desc);
    expanding.set_animations_enabled(true, &desc);
    expanding.set_pane_open(true, &desc);

    let mut expand_opacities = Vec::new();
    for t in [0.25_f32, 0.5, 0.75] {
        expanding.pane_width_spring.value = compact + (open - compact) * t;
        let (progress, pane_right, icon_x, search_x, glyphs) = sample_frame(&expanding, &mut ctx);
        assert!((progress - t).abs() < 1e-4);
        assert!(
            (icon_x - expected_icon_x).abs() < 1e-3,
            "primary icon drifted horizontally at expand t={t}: got {icon_x}, expected {expected_icon_x}"
        );
        let search_x = search_x.expect("search icon must remain anchored during expand");
        assert!(
            (search_x - expected_icon_x).abs() < 1e-3,
            "search icon drifted horizontally at expand t={t}: got {search_x}, expected {expected_icon_x}"
        );
        assert!(!glyphs.is_empty(), "labels must fade in at expand t={t}");
        for &(op, right_x) in &glyphs {
            assert!((op - t).abs() < 0.02);
            assert!(right_x <= pane_right + 0.5);
        }
        expand_opacities.push(glyphs[0].0);
    }
    assert!(
        expand_opacities[0] < expand_opacities[1] && expand_opacities[1] < expand_opacities[2],
        "expand text opacity must increase monotonically: {expand_opacities:?}"
    );
}

#[test]
fn left_pane_overflow_scrollbar_supports_auto_hide_hover_track_page_and_thumb_drag() {
    let rect = Rect::new(0.0, 0.0, 900.0, 260.0);
    let owner_node = 77_u64;
    let thumb_node = scrollbar_thumb_node(owner_node, ScrollbarAxis::Vertical);
    let track_node = scrollbar_track_node(owner_node, ScrollbarAxis::Vertical);

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };

    // 1. 未溢出场景：Left 窗格不发射滚动条命中区。
    let short_desc = sample_desc(
        Rect::new(0.0, 0.0, 1200.0, 720.0),
        NavigationViewPaneDisplayMode::Left,
    );
    let mut short_state = NavigationViewState::from_desc(&short_desc);
    short_state.set_animations_enabled(false, &short_desc);
    let short_out = emit_navigation_view(
        owner_node,
        &short_desc,
        ControlState::default(),
        &short_state,
        &mut ctx,
    );
    assert!(
        !short_out
            .output
            .sub_hits
            .iter()
            .any(|h| h.node == thumb_node || h.node == track_node),
        "non-overflowing Left pane must not emit scrollbar hits"
    );

    // 2. 溢出场景：12 个菜单项超出 260px 高度，自动发射 Win11 按需叠加滚动条。
    let many_items: Vec<NavigationViewMenuItem> = (1..=12_u64)
        .map(|i| {
            NavigationViewMenuItem::item(
                NavigationViewItem::new(i, format!("Section {i}")).with_icon(ShellIcon::Grid),
            )
        })
        .collect();
    let overflow_desc = NavigationViewDesc::new(rect, many_items)
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .open_pane_length(220.0)
        .selected_item(Some(1));
    let mut state = NavigationViewState::from_desc(&overflow_desc);
    state.set_animations_enabled(false, &overflow_desc);

    let idle = emit_navigation_view(
        owner_node,
        &overflow_desc,
        ControlState::default(),
        &state,
        &mut ctx,
    );
    assert!(
        idle.layout.menu_content_height > idle.layout.menu_viewport_rect.height(),
        "test fixture must overflow the menu viewport"
    );
    let thumb_hit = idle
        .output
        .sub_hits
        .iter()
        .find(|h| h.node == thumb_node)
        .cloned()
        .expect("overflowing Left pane must emit vertical scrollbar thumb hit");
    let track_hit = idle
        .output
        .sub_hits
        .iter()
        .find(|h| h.node == track_node)
        .cloned()
        .expect("overflowing Left pane must emit vertical scrollbar track hit");

    // 静止态：轨道胶囊透明（0.0），滑块为 2px 半透明细线（0.55）。
    let idle_len = idle.output.primitives.len();
    let idle_track_prim = &idle.output.primitives[idle_len - 3];
    let idle_thumb_prim = &idle.output.primitives[idle_len - 2];
    assert_eq!(idle_track_prim.opacity(), 0.0);
    assert!((idle_thumb_prim.size[0] - SCROLLBAR_THUMB_IDLE_WIDTH).abs() < 0.01);
    assert!((idle_thumb_prim.opacity() - SCROLLBAR_IDLE_OPACITY).abs() < 0.02);

    // 3. 悬停滚动条热区展开为 6px 粗滑块 + 半透明轨道胶囊，且图元数量保持恒定。
    assert!(state.handle_pointer_move(&overflow_desc, &idle.layout, track_hit.bounds.center()));
    assert!(state.scrollbar_hovered);
    let hovered = emit_navigation_view(
        owner_node,
        &overflow_desc,
        ControlState::default(),
        &state,
        &mut ctx,
    );
    assert_eq!(
        hovered.output.primitives.len(),
        idle_len,
        "hovering scrollbar must keep primitive count constant"
    );
    let hov_track_prim = &hovered.output.primitives[idle_len - 3];
    let hov_thumb_prim = &hovered.output.primitives[idle_len - 2];
    assert!((hov_track_prim.size[0] - SCROLLBAR_WIDTH).abs() < 0.01);
    assert!((hov_track_prim.opacity() - SCROLLBAR_TRACK_CAPSULE_OPACITY).abs() < 0.02);
    assert!((hov_thumb_prim.size[0] - SCROLLBAR_THUMB_WIDTH).abs() < 0.01);
    assert!(hov_thumb_prim.opacity() > 0.99);

    // 4. 点击滑块下方轨道翻页（经 UiWidget::reduce 路由），再点击上方轨道翻回顶部。
    let below_thumb = Vec2::new(track_hit.bounds.center().x, track_hit.bounds.max().y - 4.0);
    assert!(overflow_desc.reduce(
        &mut state,
        rect,
        &WidgetInputEvent::PointerDown {
            pos: below_thumb,
            click_count: 1,
        }
    ));
    let expected_page = idle
        .layout
        .menu_viewport_rect
        .height()
        .min(state.scroll_max);
    assert!((state.scroll_offset - expected_page).abs() < 0.5);

    let above_thumb = Vec2::new(track_hit.bounds.center().x, track_hit.bounds.origin.y + 2.0);
    assert!(overflow_desc.reduce(
        &mut state,
        rect,
        &WidgetInputEvent::PointerDown {
            pos: above_thumb,
            click_count: 1,
        }
    ));
    assert_eq!(state.scroll_offset, 0.0);

    // 5. 滑块拖拽：按下滑块 -> 向下拖到底 -> 释放。
    assert!(overflow_desc.reduce(
        &mut state,
        rect,
        &WidgetInputEvent::PointerDown {
            pos: thumb_hit.bounds.center(),
            click_count: 1,
        }
    ));
    assert!(state.scrollbar_drag.is_some());
    assert!(overflow_desc.reduce(
        &mut state,
        rect,
        &WidgetInputEvent::PointerDrag {
            pos: Vec2::new(
                thumb_hit.bounds.center().x,
                track_hit.bounds.max().y + 500.0
            ),
        }
    ));
    assert!((state.scroll_offset - state.scroll_max).abs() < 0.5);
    assert!(overflow_desc.reduce(
        &mut state,
        rect,
        &WidgetInputEvent::PointerUp {
            pos: thumb_hit.bounds.center(),
        }
    ));
    assert!(state.scrollbar_drag.is_none());

    // 6. ScrollBarVisibility::Hidden 抑制发射。
    let hidden_desc = overflow_desc
        .clone()
        .scrollbar_visibility(ScrollBarVisibility::Hidden);
    let hidden_out = emit_navigation_view(
        owner_node,
        &hidden_desc,
        ControlState::default(),
        &state,
        &mut ctx,
    );
    assert!(!hidden_out
        .output
        .sub_hits
        .iter()
        .any(|h| h.node == thumb_node || h.node == track_node));

    // 7. Top 模式即便条目溢出也不发射任何滚动条（继续走溢出按钮）。
    let top_overflow = overflow_desc.pane_display_mode(NavigationViewPaneDisplayMode::Top);
    let mut top_state = NavigationViewState::from_desc(&top_overflow);
    top_state.set_animations_enabled(false, &top_overflow);
    let top_out = emit_navigation_view(
        owner_node,
        &top_overflow,
        ControlState::default(),
        &top_state,
        &mut ctx,
    );
    assert!(top_out.layout.overflow_button_rect.is_some());
    assert!(!top_out
        .output
        .sub_hits
        .iter()
        .any(|h| h.node == thumb_node || h.node == track_node));
}

#[test]
fn left_pane_scrollbar_aligns_to_pane_outer_right_edge_and_hides_when_compact_idle() {
    let rect = Rect::new(0.0, 0.0, 900.0, 260.0);
    let owner_node = 88_u64;
    let thumb_node = scrollbar_thumb_node(owner_node, ScrollbarAxis::Vertical);
    let track_node = scrollbar_track_node(owner_node, ScrollbarAxis::Vertical);

    let many_items: Vec<NavigationViewMenuItem> = (1..=12_u64)
        .map(|i| {
            NavigationViewMenuItem::item(
                NavigationViewItem::new(i, format!("Item {i}")).with_icon(ShellIcon::Grid),
            )
        })
        .collect();
    let desc = NavigationViewDesc::new(rect, many_items)
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .open_pane_length(220.0)
        .selected_item(Some(1));

    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };

    // 1. 展开态：滚动条宿主右缘严格对齐 `active_pane_rect.max().x`（不再被 PANE_PADDING 二次内缩），
    //    且静止 2px 细指示条贴紧轨道外右缘。
    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);
    let expanded_out =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    let active_pane = expanded_out
        .layout
        .overlay_rect
        .unwrap_or(expanded_out.layout.pane_rect);
    let track_hit = expanded_out
        .output
        .sub_hits
        .iter()
        .find(|h| h.node == track_node)
        .cloned()
        .expect("expanded overflowing Left pane must emit track hit");
    assert!(
        (track_hit.bounds.max().x - (active_pane.max().x - SCROLLBAR_EDGE_INSET)).abs() < 1e-3,
        "scrollbar track right edge {} must align to active_pane.max().x - EDGE_INSET ({})",
        track_hit.bounds.max().x,
        active_pane.max().x - SCROLLBAR_EDGE_INSET
    );
    let exp_len = expanded_out.output.primitives.len();
    let idle_thumb_prim = &expanded_out.output.primitives[exp_len - 2];
    let expected_idle_x =
        track_hit.bounds.max().x - SCROLLBAR_EDGE_INSET - SCROLLBAR_THUMB_IDLE_WIDTH;
    assert!(
        (idle_thumb_prim.local_pos[0] - expected_idle_x).abs() < 1e-3,
        "idle 2px thumb x {} must hug outer right edge {expected_idle_x}",
        idle_thumb_prim.local_pos[0]
    );

    // 展开/折叠过渡中间态（pane_expand_progress > 0.01）：仍保持溢出即显且右缘紧贴当前窗格右缘。
    state.set_animations_enabled(true, &desc);
    state.pane_width_spring.value = 134.0;
    let mid_out =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    assert!(mid_out.layout.pane_expand_progress > 0.01);
    let mid_track = mid_out
        .output
        .sub_hits
        .iter()
        .find(|h| h.node == track_node)
        .expect("mid-transition pane must still emit scrollbar track hit");
    assert!(
        (mid_track.bounds.max().x - (mid_out.layout.pane_rect.max().x - SCROLLBAR_EDGE_INSET))
            .abs()
            < 1e-3
    );

    // 2. 紧凑折叠态（pane_expand_progress <= 0.01）：默认静止且未悬停时自动隐藏滚动条（0 命中区）。
    state.set_animations_enabled(false, &desc);
    state.set_pane_open(false, &desc);
    let compact_idle =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    assert!(compact_idle.layout.pane_expand_progress <= 0.01);
    assert!(
        compact_idle.layout.menu_content_height > compact_idle.layout.menu_viewport_rect.height(),
        "compact pane fixture must still overflow vertically"
    );
    assert!(
        !compact_idle
            .output
            .sub_hits
            .iter()
            .any(|h| h.node == thumb_node || h.node == track_node),
        "compact collapsed pane must hide scrollbar when idle and unhovered"
    );

    // 3. 紧凑折叠态悬停窗格：按需显现 2px 细指示条，且紧贴 48px 图标轨外右缘。
    let pane_center = compact_idle.layout.pane_rect.center();
    assert!(state.handle_pointer_move(&desc, &compact_idle.layout, pane_center));
    assert!(state.pane_hovered);
    let compact_pane_hov =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    let compact_track = compact_pane_hov
        .output
        .sub_hits
        .iter()
        .find(|h| h.node == track_node)
        .cloned()
        .expect("hovering compact pane must reveal scrollbar track hit");
    assert!(
        (compact_track.bounds.max().x
            - (compact_pane_hov.layout.pane_rect.max().x - SCROLLBAR_EDGE_INSET))
            .abs()
            < 1e-3
    );

    // 悬停到紧凑折叠态滚动条热区：展开为 6px 粗滑块 + 半透明轨道胶囊。
    assert!(state.handle_pointer_move(
        &desc,
        &compact_pane_hov.layout,
        compact_track.bounds.center()
    ));
    assert!(state.scrollbar_hovered);
    let compact_bar_hov =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    let bar_hov_len = compact_bar_hov.output.primitives.len();
    assert!(
        (compact_bar_hov.output.primitives[bar_hov_len - 2].size[0] - SCROLLBAR_THUMB_WIDTH).abs()
            < 0.01
    );

    // 指针移出窗格：重新隐藏滚动条。
    assert!(state.handle_pointer_move(&desc, &compact_bar_hov.layout, Vec2::new(600.0, 120.0)));
    assert!(!state.pane_hovered);
    assert!(!state.scrollbar_hovered);
    let compact_left =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    assert!(!compact_left
        .output
        .sub_hits
        .iter()
        .any(|h| h.node == thumb_node || h.node == track_node));

    // 4. 紧凑折叠态未悬停但近期发生滚轮滚动：按需显现滚动条；保持时长到期后自动重新隐藏。
    state.scroll_max = (compact_left.layout.menu_content_height
        - compact_left.layout.menu_viewport_rect.height())
    .max(0.0);
    assert!(state.scroll_by(48.0));
    let compact_scrolled =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    assert!(
        compact_scrolled
            .output
            .sub_hits
            .iter()
            .any(|h| h.node == track_node),
        "recent wheel scroll must reveal scrollbar in compact collapsed mode"
    );

    state.set_animations_enabled(true, &desc);
    let _ = state.tick(SCROLLBAR_HOLD + 0.1, &desc);
    let compact_expired =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);
    assert!(
        !compact_expired
            .output
            .sub_hits
            .iter()
            .any(|h| h.node == thumb_node || h.node == track_node),
        "compact scrollbar must hide again after recent scroll hold expires"
    );
}

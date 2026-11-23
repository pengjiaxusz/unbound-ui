//! `NavigationView` 完整无头行为、布局、浮层、层级、动效、键盘与无障碍契约测试。

use accesskit::Role;
use unbound_input::{KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine};
use unbound_widgets::{
    emit_navigation_view, ControlState, NavigationViewBackButtonVisible, NavigationViewDesc,
    NavigationViewDisplayMode, NavigationViewEvent, NavigationViewItem, NavigationViewMenuItem,
    NavigationViewPaneDisplayMode, NavigationViewState, PointerPhase, ShellIcon, TextCtx,
    NAVIGATION_VIEW_BACK_BUTTON_ID, NAVIGATION_VIEW_COMPACT_PANE_LENGTH,
    NAVIGATION_VIEW_OPEN_PANE_LENGTH, NAVIGATION_VIEW_OVERFLOW_BUTTON_ID,
    NAVIGATION_VIEW_PANE_TOGGLE_ID, NAVIGATION_VIEW_SEARCH_BUTTON_ID, NAVIGATION_VIEW_SETTINGS_ID,
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

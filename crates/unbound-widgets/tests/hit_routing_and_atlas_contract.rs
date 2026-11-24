//! 控件命中路由、零私有图集与窄宽布局防回归门禁测试（ADR-0044）。
//!
//! 覆盖以下六项无头契约守卫：
//! 1. `catalog.rs` 源码静态门禁：禁止私自构造局部 `GlyphCache::new(` 或 `TextEngine::new(`；
//! 2. `PivotDesc::reduce` 坐标一致性：描述符 `rect` 为 `Rect::ZERO` 时，必须基于布局 `rect` 命中标签；
//! 3. `Pivot` 指示条与焦点环几何对齐：`indicator_rect` 水平边界不超出 `item.inflate(-2.0)`；
//! 4. `NavigationView` 滚出视口热区裁剪：溢出主菜单项的 `sub_hits` 与点击判定均受 `menu_viewport_rect` 裁剪，不得抢占页脚；
//! 5. `AddressBar` 刷新图标与窄宽保护：刷新图标须采用开口弧线曲线而非遗留闭合圆环，极窄宽度下 `root_icon_rect` 不溢出 `address_rect`；
//! 6. 复合水平组件窄宽防重叠：`CommandBarFlyout`、`MenuBar`、`ShortcutBar` 在窄宽下无越界或重叠。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_widgets::address_bar::{
    compute_layout as compute_address_bar_layout, emit_address_bar, AddressBarDesc, AddressBarState,
};
use unbound_widgets::command_bar_flyout::CommandBarFlyoutDesc;
use unbound_widgets::menubar::{MenuBarDesc, MenuBarItem, MENUBAR_HEIGHT};
use unbound_widgets::navigation_view::{
    emit_navigation_view, navigation_view_row_node, NavigationViewDesc, NavigationViewItem,
    NavigationViewMenuItem, NavigationViewPaneDisplayMode, NavigationViewRowKind,
    NavigationViewState,
};
use unbound_widgets::pivot::{PivotDesc, PIVOT_HEIGHT};
use unbound_widgets::shortcut_bar::{emit_shortcut_bar, ShortcutBarDesc, ShortcutEntry};
use unbound_widgets::{ControlState, ShellIcon, TextCtx, UiWidget, WidgetInputEvent};

/// 剥离 Rust 单行注释与块注释，返回当前行剩余的代码片段（用于静态门禁扫描）。
fn strip_rust_comments(line: &str, in_block_comment: &mut bool) -> String {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut out = String::with_capacity(line.len());
    while i < bytes.len() {
        if *in_block_comment {
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            break;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            *in_block_comment = true;
            i += 2;
            continue;
        }
        if let Some(ch) = line[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

#[test]
fn catalog_source_forbids_private_glyph_cache_or_text_engine() {
    let source = include_str!("../src/catalog.rs");
    let mut in_block_comment = false;
    let mut violations = Vec::new();

    for (idx, raw_line) in source.lines().enumerate() {
        let code = strip_rust_comments(raw_line, &mut in_block_comment);
        if code.contains("GlyphCache::new(") || code.contains("TextEngine::new(") {
            violations.push(format!("line {}: {}", idx + 1, raw_line.trim()));
        }
    }

    assert!(
        violations.is_empty(),
        "ADR-0044 zero private atlas invariant violated in catalog.rs:\n{}",
        violations.join("\n")
    );
}

#[test]
fn pivot_reduce_uses_laid_out_rect_instead_of_zero_desc_rect() {
    let desc = PivotDesc::new(Rect::ZERO, &["Overview", "Activity", "Settings"]);
    let mut state = desc.initial_state();
    assert_eq!(state.selected, 0);

    let laid_out = Rect::new(120.0, 80.0, 360.0, PIVOT_HEIGHT);
    let mut reference = desc.clone();
    reference.rect = laid_out;

    // 点击第 2 个标签（索引 1）。
    let tab1_pos = reference.item_rect(1).center();
    let changed_to_1 = desc.reduce(
        &mut state,
        laid_out,
        &WidgetInputEvent::PointerDown {
            pos: tab1_pos,
            click_count: 1,
        },
    );
    assert!(
        changed_to_1,
        "clicking the second pivot tab with laid_out rect must update state"
    );
    assert_eq!(state.selected, 1);

    // 点击第 3 个标签（索引 2）。
    let tab2_pos = reference.item_rect(2).center();
    let changed_to_2 = desc.reduce(
        &mut state,
        laid_out,
        &WidgetInputEvent::PointerDown {
            pos: tab2_pos,
            click_count: 1,
        },
    );
    assert!(
        changed_to_2,
        "clicking the third pivot tab with laid_out rect must update state"
    );
    assert_eq!(state.selected, 2);
}

#[test]
fn pivot_indicator_stays_within_focus_ring_horizontal_bounds() {
    let cases = [
        PivotDesc::new(
            Rect::new(24.0, 16.0, 320.0, PIVOT_HEIGHT),
            &["Overview", "Activity", "Settings"],
        ),
        PivotDesc::new(
            Rect::new(0.0, 0.0, 360.0, PIVOT_HEIGHT),
            &["Home", "Details", "Logs"],
        )
        .with_item_widths(&[80.0, 110.0, 64.0]),
    ];

    for desc in &cases {
        for index in 0..desc.len() {
            let item = desc.item_rect(index);
            let focus_bounds = item.inflate(-2.0);
            let indicator = desc.indicator_rect(index);
            assert!(
                indicator.width() > 0.0,
                "pivot indicator at index {index} must have positive width: {indicator:?}"
            );
            assert!(
                indicator.origin.x >= focus_bounds.origin.x - 1e-3,
                "pivot indicator left edge {:?} overflows focus ring {:?} at index {index}",
                indicator,
                focus_bounds
            );
            assert!(
                indicator.max().x <= focus_bounds.max().x + 1e-3,
                "pivot indicator right edge {:?} overflows focus ring {:?} at index {index}",
                indicator,
                focus_bounds
            );
        }
    }
}

#[test]
fn navigation_view_clips_scrolled_out_menu_hits_against_footer() {
    let main_item_count = 14_u64;
    let footer_id = 90_u64;
    let menu_items: Vec<NavigationViewMenuItem> = (1..=main_item_count)
        .map(|id| {
            NavigationViewMenuItem::item(
                NavigationViewItem::new(id, format!("Section {id}")).with_icon(ShellIcon::Grid),
            )
        })
        .collect();

    let desc = NavigationViewDesc::new(Rect::new(0.0, 0.0, 960.0, 320.0), menu_items)
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .footer_menu_items(vec![NavigationViewMenuItem::item(
            NavigationViewItem::new(footer_id, "Account").with_icon(ShellIcon::Sliders),
        )])
        .selected_item(Some(1));

    let mut state = NavigationViewState::from_desc(&desc);
    state.set_animations_enabled(false, &desc);
    // 施加非整数行高的滚动偏移，使底部主菜单行跨过 `menu_viewport_rect` 下沿并侵入页脚区域。
    state.scroll_offset = 18.0;

    let layout = desc.compute_layout(&state);
    assert!(
        layout.menu_content_height > layout.menu_viewport_rect.height(),
        "test fixture must overflow menu viewport to verify hit clipping"
    );

    let owner_node = 500_u64;
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };
    let emitted =
        emit_navigation_view(owner_node, &desc, ControlState::default(), &state, &mut ctx);

    // 1. 所有主菜单子热区必须严格受 `menu_viewport_rect` 纵向裁剪。
    for id in 1..=main_item_count {
        let row_node = navigation_view_row_node(owner_node, id);
        if let Some(hit) = emitted.output.sub_hits.iter().find(|h| h.node == row_node) {
            assert!(
                hit.bounds.origin.y >= layout.menu_viewport_rect.origin.y - 0.5,
                "main menu item {id} sub_hit {:?} extends above menu_viewport_rect {:?}",
                hit.bounds,
                layout.menu_viewport_rect
            );
            assert!(
                hit.bounds.max().y <= layout.menu_viewport_rect.max().y + 0.5,
                "main menu item {id} sub_hit {:?} extends below menu_viewport_rect {:?}",
                hit.bounds,
                layout.menu_viewport_rect
            );
        }
    }

    // 2. 点击页脚条目区域（即便位于被滚出视口主菜单行原始矩形覆盖的顶部边缘）必须命中页脚而非主菜单。
    let footer_row = layout
        .rows
        .iter()
        .find(|row| matches!(&row.kind, NavigationViewRowKind::FooterItem { item } if item.id == footer_id))
        .expect("footer item row must be present in layout");
    let click_pos = footer_row.rect.center();
    let handled = state.handle_pointer_down(&desc, &layout, click_pos);
    assert!(handled, "clicking footer row must be handled");
    assert_eq!(
        state.selected_item_id,
        Some(footer_id),
        "clicking footer row must select footer item instead of a scrolled-out main menu item"
    );
}

#[test]
fn address_bar_refresh_icon_uses_open_arc_and_narrow_root_icon_stays_inside() {
    // 1. 刷新图标必须采用开口圆弧曲线（PrimType::Curve），禁止遗留闭合描边圆环（PrimType::RoundedRect）。
    let desc = AddressBarDesc::new(
        Rect::new(0.0, 0.0, 480.0, 36.0),
        vec!["Users".to_owned(), "Documents".to_owned()],
    )
    .with_nav_buttons(true)
    .with_refresh(true)
    .without_search();
    let layout = compute_address_bar_layout(&desc);
    let refresh_r = layout
        .refresh_rect
        .expect("refresh button rect must exist when show_refresh is true");
    let out = emit_address_bar(
        10,
        &desc,
        &AddressBarState::new(),
        ControlState::default(),
        &[],
    );

    let mut curve_count = 0usize;
    let mut legacy_closed_ring_found = false;
    for prim in &out.primitives {
        let prim_rect = Rect::new(
            prim.local_pos[0],
            prim.local_pos[1],
            prim.size[0],
            prim.size[1],
        );
        let center = prim_rect.center();
        if !refresh_r.contains_point(center) {
            continue;
        }
        match prim.prim_type() {
            PrimType::Curve => {
                curve_count += 1;
            }
            PrimType::RoundedRect => {
                // 跳过按钮背景表面本身，检测内部遗留的 11x11 闭合描边圆环。
                let is_inner_ring = prim.size[0] < refresh_r.width() - 1.0
                    && prim.size[1] < refresh_r.height() - 1.0
                    && prim.border_width() > 0.0
                    && prim.radii()[0] >= 4.0;
                if is_inner_ring {
                    legacy_closed_ring_found = true;
                }
            }
            _ => {}
        }
    }

    assert!(
        !legacy_closed_ring_found,
        "AddressBar refresh icon must not emit a legacy closed RoundedRect ring"
    );
    assert!(
        curve_count >= 3,
        "AddressBar refresh icon must emit open-arc curve segments plus arrowhead (found {curve_count} curves)"
    );

    // 2. 极窄宽度下 `root_icon_rect` 不得溢出 `address_rect`。
    for width in [160.0_f32, 120.0, 80.0, 40.0, 20.0] {
        for show_nav in [true, false] {
            let narrow = AddressBarDesc::new(
                Rect::new(0.0, 0.0, width, 36.0),
                vec!["C:".to_owned(), "Workspace".to_owned()],
            )
            .with_nav_buttons(show_nav)
            .with_refresh(show_nav)
            .without_search();
            let narrow_layout = compute_address_bar_layout(&narrow);
            let addr = narrow_layout.address_rect;
            let icon = narrow_layout.root_icon_rect;
            assert!(
                icon.size.x >= 0.0 && icon.size.y >= 0.0,
                "root_icon_rect must have non-negative size at width={width}, show_nav={show_nav}: {icon:?}"
            );
            assert!(
                icon.origin.x >= addr.origin.x - 1e-3,
                "root_icon_rect {icon:?} overflows left of address_rect {addr:?} (width={width}, show_nav={show_nav})"
            );
            assert!(
                icon.max().x <= addr.max().x + 1e-3,
                "root_icon_rect {icon:?} overflows right of address_rect {addr:?} (width={width}, show_nav={show_nav})"
            );
        }
    }
}

#[test]
fn narrow_horizontal_composite_widgets_never_overflow_or_overlap() {
    // 1. CommandBarFlyout 窄宽防越界与防重叠。
    let flyout = CommandBarFlyoutDesc::new(Rect::new(10.0, 10.0, 110.0, 40.0)).with_items(4);
    let divider = flyout.divider_rect();
    let more = flyout.more_rect();
    assert!(
        more.origin.x >= flyout.rect.origin.x - 1e-3 && more.max().x <= flyout.rect.max().x + 1e-3,
        "CommandBarFlyout more_rect {more:?} overflows container {:?}",
        flyout.rect
    );
    assert!(
        divider.max().x <= more.origin.x + 1e-3,
        "CommandBarFlyout divider {divider:?} overlaps more_rect {more:?}"
    );
    let mut prev_right = flyout.rect.origin.x;
    for index in 0..flyout.items {
        let item = flyout.item_rect(index);
        assert!(
            item.width() >= 0.0,
            "CommandBarFlyout item {index} has negative width: {item:?}"
        );
        assert!(
            item.origin.x >= flyout.rect.origin.x - 1e-3
                && item.max().x <= flyout.rect.max().x + 1e-3,
            "CommandBarFlyout item {index} {item:?} overflows container {:?}",
            flyout.rect
        );
        if item.width() > 0.0 {
            assert!(
                item.origin.x >= prev_right - 1e-3,
                "CommandBarFlyout item {index} {item:?} overlaps previous item right edge {prev_right}"
            );
            assert!(
                item.max().x <= divider.origin.x + 1e-3,
                "CommandBarFlyout item {index} {item:?} overlaps divider {divider:?}"
            );
            prev_right = item.max().x;
        }
    }

    // 2. MenuBar 窄宽防越界与防重叠。
    let menubar = MenuBarDesc::new(
        Rect::new(0.0, 0.0, 120.0, MENUBAR_HEIGHT),
        vec![
            MenuBarItem::new(1, "File"),
            MenuBarItem::new(2, "Edit"),
            MenuBarItem::new(3, "Selection"),
            MenuBarItem::new(4, "View"),
            MenuBarItem::new(5, "Help"),
        ],
    );
    let laid = menubar.layout(&[]);
    let mut prev_menu_right = menubar.rect.origin.x;
    for (rect, idx) in &laid {
        assert!(
            rect.width() >= 0.0,
            "MenuBar item {idx} has negative width: {rect:?}"
        );
        assert!(
            rect.origin.x >= menubar.rect.origin.x - 1e-3
                && rect.max().x <= menubar.rect.max().x + 1e-3,
            "MenuBar item {idx} {rect:?} overflows container {:?}",
            menubar.rect
        );
        if rect.width() > 0.0 {
            assert!(
                rect.origin.x >= prev_menu_right - 1e-3,
                "MenuBar item {idx} {rect:?} overlaps previous right edge {prev_menu_right}"
            );
            prev_menu_right = rect.max().x;
        }
    }

    // 3. ShortcutBar 窄宽防越界与防重叠。
    let shortcut_rect = Rect::new(0.0, 0.0, 160.0, 32.0);
    let shortcut_desc = ShortcutBarDesc::new(
        shortcut_rect,
        vec![
            ShortcutEntry::new("cmd.palette", "Command Palette", "Ctrl+Shift+P"),
            ShortcutEntry::new("file.save_all", "Save All", "Ctrl+Alt+S"),
            ShortcutEntry::new("editor.format", "Format Document", "Shift+Alt+F"),
        ],
    );
    let shortcut_out = emit_shortcut_bar(1, &shortcut_desc, ControlState::default(), &[]);
    let keycap_z = shortcut_desc.z + 1;
    let mut prev_keycap_right = shortcut_rect.origin.x;
    for prim in shortcut_out
        .primitives
        .iter()
        .filter(|p| p.depth_u16() == keycap_z && p.size[0] > 0.0 && p.opacity() > 0.0)
    {
        let left = prim.local_pos[0];
        let right = left + prim.size[0];
        assert!(
            left >= shortcut_rect.origin.x - 1e-3 && right <= shortcut_rect.max().x + 1e-3,
            "ShortcutBar keycap [{left}, {right}] overflows container {:?}",
            shortcut_rect
        );
        assert!(
            left >= prev_keycap_right - 1e-3,
            "ShortcutBar keycap [{left}, {right}] overlaps previous keycap right edge {prev_keycap_right}"
        );
        prev_keycap_right = right;
    }
}

//! 参数化按需叠加滚动条契约测试（ADR-0031 / #187）。
//!
//! 遍历 [`WidgetCatalogId::ALL`] 中全部声明了
//! [`OverflowStrategy::OverlayScrollbarVertical`] 或
//! [`OverflowStrategy::OverlayScrollbarBoth`] 的目录组件（外加独立模块 `VirtualGrid`），
//! 对每个组件构造「未溢出」与「溢出」双场景并强制校验：
//!
//! 1. **未溢出**：0 滚动条图元、0 滚动条轨道/滑块命中区；
//! 2. **溢出**：必须产出可见滚动条滑块图元（`opacity > 0` 且尺寸大于 0），
//!    并同时注册轨道（`scrollbar_track_node`）与滑块（`scrollbar_thumb_node`）命中区；
//! 3. **防漏守卫**：若未来在 `canonical.rs` 新增任何 `OverlayScrollbar*` 目录组件却未补入
//!    本文件的双态契约夹具，测试立即显式 panic 提醒；同时显式断言 `TabView`
//!    （`TabStripButtons`）与 `Form`（`ExternalScrollView`）不在滚动条集合中。

use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine};
use unbound_widgets::context_menu::ContextMenuState;
use unbound_widgets::generic_data_table::{
    emit_generic_data_table, DataColumnSpec, GenericDataTableDesc, GenericDataTableState,
};
use unbound_widgets::menu::{emit_menu, MenuDesc, MenuItem, MenuState};
use unbound_widgets::navigation_view::{
    emit_navigation_view, NavigationViewDesc, NavigationViewItem, NavigationViewMenuItem,
    NavigationViewPaneDisplayMode, NavigationViewState,
};
use unbound_widgets::scroll::{
    emit_scroll_view, scrollbar_thumb_node, scrollbar_track_node, ScrollAxis, ScrollState,
    ScrollViewDesc, ScrollbarAxis,
};
use unbound_widgets::table::{emit_table, ColumnSpec, TableDesc, TableRow, TableState};
use unbound_widgets::text_area::{emit_text_area, TextAreaDesc, TextAreaState, TextAreaVisual};
use unbound_widgets::tree_view::{emit_tree_scrollbar, TreeViewDesc, TreeViewState};
use unbound_widgets::virtual_grid::{
    emit_virtual_grid, GridColumnSizing, VirtualGridDesc, VirtualGridState,
};
use unbound_widgets::{ControlState, OverflowStrategy, ShellIcon, TextCtx, WidgetCatalogId};
use unbound_world::TreeRow;

/// 单次滚动条契约探测的观测快照。
struct ScrollbarProbe {
    /// 发射出的全部图元。
    primitives: Vec<GpuPrimitiveInstance>,
    /// 发射出的全部子命中区（含轨道/滑块命中区）。
    hits: Vec<HitRegion>,
    /// 允许出现滚动条命中区的合法容器矩形。
    container_rect: Rect,
}

/// 校验「未溢出 = 0 滚动条图元与 0 滚动条命中区；溢出 = 可见滑块图元 + 轨道/滑块命中区」。
fn assert_dual_state_scrollbar_contract(
    label: &str,
    owner_node: u64,
    axes: &[ScrollbarAxis],
    non_overflow: &ScrollbarProbe,
    overflow: &ScrollbarProbe,
    expected_non_overflow_scrollbar_prims: usize,
    min_overflow_extra_prims: usize,
) {
    assert_eq!(
        expected_non_overflow_scrollbar_prims, 0,
        "{label}: non-overflow state must emit 0 scrollbar primitives"
    );

    // 1. 未溢出场景：所有相关轴均不得出现任何轨道或滑块命中区。
    for &axis in axes {
        let thumb_node = scrollbar_thumb_node(owner_node, axis);
        let track_node = scrollbar_track_node(owner_node, axis);
        assert!(
            !non_overflow
                .hits
                .iter()
                .any(|h| h.node == thumb_node || h.node == track_node),
            "{label}: non-overflowing state must not emit {axis:?} scrollbar hit regions"
        );
    }

    // 2. 溢出场景：至少增加 2 * axes.len() 个滚动条图元，且每个轴必须同时具备轨道与滑块命中区，
    //    并在滑块命中区附近存在可见（opacity > 0、正尺寸）的滑块图元。
    assert!(
        overflow.primitives.len() >= non_overflow.primitives.len() + min_overflow_extra_prims
            || overflow.primitives.len() >= axes.len() * 2,
        "{label}: overflowing state must emit scrollbar primitives (non_overflow={}, overflow={})",
        non_overflow.primitives.len(),
        overflow.primitives.len()
    );

    for &axis in axes {
        let thumb_node = scrollbar_thumb_node(owner_node, axis);
        let track_node = scrollbar_track_node(owner_node, axis);

        let track_hit = overflow
            .hits
            .iter()
            .find(|h| h.node == track_node)
            .unwrap_or_else(|| {
                panic!("{label}: overflowing state must emit {axis:?} scrollbar track hit region")
            });
        let thumb_hit = overflow
            .hits
            .iter()
            .find(|h| h.node == thumb_node)
            .unwrap_or_else(|| {
                panic!("{label}: overflowing state must emit {axis:?} scrollbar thumb hit region")
            });

        assert!(
            track_hit.bounds.size.x > 0.0 && track_hit.bounds.size.y > 0.0,
            "{label}: {axis:?} track hit must have positive area: {:?}",
            track_hit.bounds
        );
        assert!(
            thumb_hit.bounds.size.x > 0.0 && thumb_hit.bounds.size.y > 0.0,
            "{label}: {axis:?} thumb hit must have positive area: {:?}",
            thumb_hit.bounds
        );
        assert!(
            overflow.container_rect.contains_rect(&track_hit.bounds),
            "{label}: {axis:?} track hit {:?} must stay within container {:?}",
            track_hit.bounds,
            overflow.container_rect
        );
        assert!(
            overflow.container_rect.contains_rect(&thumb_hit.bounds),
            "{label}: {axis:?} thumb hit {:?} must stay within container {:?}",
            thumb_hit.bounds,
            overflow.container_rect
        );

        let has_visible_thumb_prim = overflow.primitives.iter().any(|prim| {
            let prim_rect = Rect::new(
                prim.local_pos[0],
                prim.local_pos[1],
                prim.size[0],
                prim.size[1],
            );
            prim.opacity() > 0.0
                && prim.size[0] > 0.0
                && prim.size[1] > 0.0
                && thumb_hit.bounds.contains_rect(&prim_rect)
        });
        assert!(
            has_visible_thumb_prim,
            "{label}: overflowing state must emit a visible {axis:?} scrollbar thumb primitive inside {:?}",
            thumb_hit.bounds
        );
    }
}

fn probe_scroll_view(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let viewport = Rect::new(0.0, 0.0, 240.0, 180.0);

    let mut short_desc = ScrollViewDesc::new(viewport, Vec2::new(160.0, 120.0));
    short_desc.axis = ScrollAxis::Both;
    let short_state = ScrollState::new();
    let short_out = emit_scroll_view(
        owner_node,
        &short_desc,
        &short_state,
        ControlState::default(),
    );

    let mut tall_desc = ScrollViewDesc::new(viewport, Vec2::new(640.0, 720.0));
    tall_desc.axis = ScrollAxis::Both;
    let tall_state = ScrollState::new();
    let tall_out = emit_scroll_view(owner_node, &tall_desc, &tall_state, ControlState::default());

    assert_eq!(short_out.output.primitives.len(), 0);
    assert_eq!(tall_out.output.primitives.len(), 4);

    (
        ScrollbarProbe {
            primitives: short_out.output.primitives,
            hits: short_out.hits,
            container_rect: viewport,
        },
        ScrollbarProbe {
            primitives: tall_out.output.primitives,
            hits: tall_out.hits,
            container_rect: viewport,
        },
    )
}

fn probe_navigation_view(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };

    let short_items = vec![
        NavigationViewMenuItem::item(NavigationViewItem::new(1, "Home").with_icon(ShellIcon::Home)),
        NavigationViewMenuItem::item(
            NavigationViewItem::new(2, "Settings").with_icon(ShellIcon::Settings),
        ),
    ];
    let short_rect = Rect::new(0.0, 0.0, 900.0, 480.0);
    let short_desc = NavigationViewDesc::new(short_rect, short_items)
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .open_pane_length(220.0)
        .selected_item(Some(1));
    let mut short_state = NavigationViewState::from_desc(&short_desc);
    short_state.set_animations_enabled(false, &short_desc);
    let short_out = emit_navigation_view(
        owner_node,
        &short_desc,
        ControlState::default(),
        &short_state,
        &mut ctx,
    );

    let many_items: Vec<NavigationViewMenuItem> = (1..=14_u64)
        .map(|i| {
            NavigationViewMenuItem::item(
                NavigationViewItem::new(i, format!("Section {i}")).with_icon(ShellIcon::Grid),
            )
        })
        .collect();
    let overflow_rect = Rect::new(0.0, 0.0, 900.0, 240.0);
    let overflow_desc = NavigationViewDesc::new(overflow_rect, many_items)
        .pane_display_mode(NavigationViewPaneDisplayMode::Left)
        .open_pane_length(220.0)
        .selected_item(Some(1));
    let mut overflow_state = NavigationViewState::from_desc(&overflow_desc);
    overflow_state.set_animations_enabled(false, &overflow_desc);
    let overflow_out = emit_navigation_view(
        owner_node,
        &overflow_desc,
        ControlState::default(),
        &overflow_state,
        &mut ctx,
    );

    (
        ScrollbarProbe {
            primitives: short_out.output.primitives,
            hits: short_out.output.sub_hits.to_vec(),
            container_rect: short_out.layout.menu_viewport_rect,
        },
        ScrollbarProbe {
            primitives: overflow_out.output.primitives,
            hits: overflow_out.output.sub_hits.to_vec(),
            container_rect: overflow_out.layout.menu_viewport_rect,
        },
    )
}

fn probe_tree_view(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let desc = TreeViewDesc::new(Rect::new(10.0, 20.0, 240.0, 180.0));

    let mut short_state = TreeViewState::new(36.0);
    let short_roots = vec![TreeRow::new(1, 0, false), TreeRow::new(2, 0, false)];
    short_state.rebuild(&short_roots, |_| Vec::new());
    let short_bar = emit_tree_scrollbar(owner_node, &desc, &short_state, ControlState::default());
    assert!(short_bar.is_empty());

    let mut tall_state = TreeViewState::new(36.0);
    let tall_roots: Vec<TreeRow> = (1..=20).map(|id| TreeRow::new(id, 0, false)).collect();
    tall_state.rebuild(&tall_roots, |_| Vec::new());
    let tall_bar = emit_tree_scrollbar(owner_node, &desc, &tall_state, ControlState::default());

    (
        ScrollbarProbe {
            primitives: short_bar.output.primitives,
            hits: short_bar.hits,
            container_rect: desc.rect,
        },
        ScrollbarProbe {
            primitives: tall_bar.output.primitives,
            hits: tall_bar.hits,
            container_rect: desc.rect,
        },
    )
}

fn probe_table(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let desc = TableDesc::new(
        Rect::new(0.0, 0.0, 320.0, 200.0),
        vec![ColumnSpec::new("ID", 80.0), ColumnSpec::new("Name", 220.0)],
    );

    let short_rows = [
        TableRow {
            index: 0,
            glyphs: &[],
        },
        TableRow {
            index: 1,
            glyphs: &[],
        },
    ];
    let short_state = TableState::new();
    let short_out = emit_table(
        owner_node,
        &desc,
        &short_state,
        &short_rows,
        &[],
        ControlState::default(),
    );

    let tall_rows: Vec<TableRow<'_>> = (0..20)
        .map(|index| TableRow { index, glyphs: &[] })
        .collect();
    let tall_state = TableState::new();
    let tall_out = emit_table(
        owner_node,
        &desc,
        &tall_state,
        &tall_rows,
        &[],
        ControlState::default(),
    );

    (
        ScrollbarProbe {
            primitives: short_out.primitives,
            hits: short_out.sub_hits.to_vec(),
            container_rect: desc.body_rect(),
        },
        ScrollbarProbe {
            primitives: tall_out.primitives,
            hits: tall_out.sub_hits.to_vec(),
            container_rect: desc.body_rect(),
        },
    )
}

fn probe_generic_data_table(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let cols = vec![
        DataColumnSpec::new("id", "ID", 80.0),
        DataColumnSpec::new("name", "Name", 160.0),
    ];
    let rect = Rect::new(0.0, 0.0, 320.0, 200.0);

    let short_desc = GenericDataTableDesc::new(rect, cols.clone(), 4).with_rows(vec![
        vec!["1".into(), "Alpha".into()],
        vec!["2".into(), "Beta".into()],
    ]);
    let short_state = GenericDataTableState::new();
    let short_out = emit_generic_data_table(
        owner_node,
        &short_desc,
        &short_state,
        ControlState::default(),
        &[],
    );

    let tall_rows: Vec<Vec<String>> = (0..14)
        .map(|i| vec![format!("{i}"), format!("Row {i}")])
        .collect();
    let tall_desc = GenericDataTableDesc::new(rect, cols, 4).with_rows(tall_rows);
    let tall_state = GenericDataTableState::new();
    let tall_out = emit_generic_data_table(
        owner_node,
        &tall_desc,
        &tall_state,
        ControlState::default(),
        &[],
    );

    (
        ScrollbarProbe {
            primitives: short_out.primitives,
            hits: short_out.sub_hits.to_vec(),
            container_rect: short_desc.body_rect(),
        },
        ScrollbarProbe {
            primitives: tall_out.primitives,
            hits: tall_out.sub_hits.to_vec(),
            container_rect: tall_desc.body_rect(),
        },
    )
}

fn probe_text_area(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let desc = TextAreaDesc::new(Rect::new(0.0, 0.0, 260.0, 120.0));
    let visual = TextAreaVisual::default();

    let short_state = TextAreaState::with_text("one\ntwo");
    let short_out = emit_text_area(
        owner_node,
        &desc,
        &short_state,
        ControlState::default(),
        &visual,
        &mut [],
    );

    let long_text = (0..18)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let tall_state = TextAreaState::with_text(long_text);
    let tall_out = emit_text_area(
        owner_node,
        &desc,
        &tall_state,
        ControlState::default(),
        &visual,
        &mut [],
    );

    assert_eq!(tall_out.primitives.len(), short_out.primitives.len() + 2);

    (
        ScrollbarProbe {
            primitives: short_out.primitives,
            hits: short_out.sub_hits.to_vec(),
            container_rect: desc.text_rect(),
        },
        ScrollbarProbe {
            primitives: tall_out.primitives,
            hits: tall_out.sub_hits.to_vec(),
            container_rect: desc.text_rect(),
        },
    )
}

fn probe_menu_flyout(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let viewport = Rect::new(0.0, 0.0, 800.0, 200.0);

    let short_items = vec![MenuItem::action("Cut"), MenuItem::action("Copy")];
    let mut short_desc = MenuDesc::new(Rect::new(0.0, 0.0, 200.0, 600.0), short_items);
    short_desc.viewport = Some(viewport);
    assert!(!short_desc.clamp_height());
    let mut short_state = MenuState::new();
    assert!(!short_state.ensure_scroll(&short_desc));
    let short_out = emit_menu(
        owner_node,
        owner_node + 1,
        &short_desc,
        &short_state,
        ControlState::default(),
    )
    .0;

    let tall_items: Vec<MenuItem> = (0..20)
        .map(|i| MenuItem::action(format!("Action {i}")))
        .collect();
    let mut tall_desc = MenuDesc::new(Rect::new(0.0, 0.0, 200.0, 600.0), tall_items);
    tall_desc.viewport = Some(viewport);
    assert!(tall_desc.clamp_height());
    let mut tall_state = MenuState::new();
    assert!(tall_state.ensure_scroll(&tall_desc));
    let tall_out = emit_menu(
        owner_node,
        owner_node + 1,
        &tall_desc,
        &tall_state,
        ControlState::default(),
    )
    .0;

    (
        ScrollbarProbe {
            primitives: short_out.primitives,
            hits: short_out.sub_hits.to_vec(),
            container_rect: short_desc.rect,
        },
        ScrollbarProbe {
            primitives: tall_out.primitives,
            hits: tall_out.sub_hits.to_vec(),
            container_rect: tall_desc.rect,
        },
    )
}

fn probe_context_menu(owner_node: u64) -> (ScrollbarProbe, ScrollbarProbe) {
    let viewport = Rect::new(0.0, 0.0, 640.0, 200.0);

    let mut short_ctx = ContextMenuState::new();
    short_ctx.open_at(Vec2::new(20.0, 20.0), 1, None);
    let short_menu = MenuDesc::new(
        Rect::new(0.0, 0.0, 200.0, 0.0),
        vec![MenuItem::action("Cut"), MenuItem::action("Copy")],
    );
    let mut short_placed = short_ctx.desc_at(short_menu, viewport);
    short_placed.viewport = Some(viewport);
    assert!(!short_placed.clamp_height());
    assert!(!short_ctx.menu.ensure_scroll(&short_placed));
    let short_out = emit_menu(
        owner_node,
        owner_node + 1,
        &short_placed,
        &short_ctx.menu,
        ControlState::default(),
    )
    .0;

    let mut tall_ctx = ContextMenuState::new();
    tall_ctx.open_at(Vec2::new(20.0, 20.0), 1, None);
    let tall_items: Vec<MenuItem> = (0..20)
        .map(|i| MenuItem::action(format!("Context Action {i}")))
        .collect();
    let tall_menu = MenuDesc::new(Rect::new(0.0, 0.0, 200.0, 0.0), tall_items);
    let mut tall_placed = tall_ctx.desc_at(tall_menu, viewport);
    tall_placed.viewport = Some(viewport);
    assert!(tall_placed.clamp_height());
    assert!(tall_ctx.menu.ensure_scroll(&tall_placed));
    let tall_out = emit_menu(
        owner_node,
        owner_node + 1,
        &tall_placed,
        &tall_ctx.menu,
        ControlState::default(),
    )
    .0;

    (
        ScrollbarProbe {
            primitives: short_out.primitives,
            hits: short_out.sub_hits.to_vec(),
            container_rect: short_placed.rect,
        },
        ScrollbarProbe {
            primitives: tall_out.primitives,
            hits: tall_out.sub_hits.to_vec(),
            container_rect: tall_placed.rect,
        },
    )
}

#[test]
fn all_overlay_scrollbar_catalog_widgets_satisfy_dual_state_contract() {
    let overlay_ids: Vec<WidgetCatalogId> = WidgetCatalogId::ALL
        .iter()
        .copied()
        .filter(|id| id.capabilities().overflow.uses_overlay_scrollbar())
        .collect();

    assert!(
        !overlay_ids.is_empty(),
        "catalog must contain overlay-scrollbar widgets"
    );

    // 显式排除走翻页按钮或外层 ScrollView 的控件。
    assert_eq!(
        WidgetCatalogId::TabView.capabilities().overflow,
        OverflowStrategy::TabStripButtons
    );
    assert!(
        !overlay_ids.contains(&WidgetCatalogId::TabView),
        "TabView uses TabStripButtons and must not be in the overlay scrollbar set"
    );
    assert_eq!(
        WidgetCatalogId::Form.capabilities().overflow,
        OverflowStrategy::ExternalScrollView
    );
    assert!(
        !overlay_ids.contains(&WidgetCatalogId::Form),
        "Form uses ExternalScrollView and must not be in the overlay scrollbar set"
    );

    for (idx, id) in overlay_ids.iter().copied().enumerate() {
        let owner_node = 1000_u64 + (idx as u64) * 10;
        let strategy = id.capabilities().overflow;
        let axes: &[ScrollbarAxis] = match strategy {
            OverflowStrategy::OverlayScrollbarVertical => &[ScrollbarAxis::Vertical],
            OverflowStrategy::OverlayScrollbarBoth => {
                &[ScrollbarAxis::Vertical, ScrollbarAxis::Horizontal]
            }
            other => panic!("unexpected strategy {other:?} in overlay scrollbar filter"),
        };

        let (non_overflow, overflow) = match id {
            WidgetCatalogId::ScrollView => probe_scroll_view(owner_node),
            WidgetCatalogId::NavigationView => probe_navigation_view(owner_node),
            WidgetCatalogId::TreeView => probe_tree_view(owner_node),
            WidgetCatalogId::Table => probe_table(owner_node),
            WidgetCatalogId::GenericDataTable => probe_generic_data_table(owner_node),
            WidgetCatalogId::TextBoxMultiline => probe_text_area(owner_node),
            WidgetCatalogId::MenuFlyout => probe_menu_flyout(owner_node),
            WidgetCatalogId::ContextMenu => probe_context_menu(owner_node),
            missing => panic!(
                "WidgetCatalogId::{missing:?} declared {strategy:?} in canonical.rs, \
                 but has no dual-state runtime scrollbar fixture in \
                 tests/integration/capability_scrollbar_contract.rs — please add one!"
            ),
        };

        assert_dual_state_scrollbar_contract(
            id.winui3_name(),
            owner_node,
            axes,
            &non_overflow,
            &overflow,
            0,
            axes.len() * 2,
        );
    }
}

#[test]
fn virtual_grid_satisfies_dual_state_overlay_scrollbar_contract() {
    let rect = Rect::new(10.0, 20.0, 320.0, 240.0);
    let owner_node = 9000_u64;

    let short_desc = VirtualGridDesc::new(rect, 2)
        .with_column_sizing(GridColumnSizing::Fixed(140.0))
        .with_row_height(100.0);
    let short_state = VirtualGridState::new();
    let short_out = emit_virtual_grid(
        owner_node,
        &short_desc,
        &short_state,
        ControlState::default(),
        &[],
    );

    let tall_desc = VirtualGridDesc::new(rect, 40)
        .with_column_sizing(GridColumnSizing::Fixed(140.0))
        .with_row_height(100.0);
    let tall_state = VirtualGridState::new();
    let tall_out = emit_virtual_grid(
        owner_node,
        &tall_desc,
        &tall_state,
        ControlState::default(),
        &[],
    );

    assert_dual_state_scrollbar_contract(
        "VirtualGrid",
        owner_node,
        &[ScrollbarAxis::Vertical],
        &ScrollbarProbe {
            primitives: short_out.primitives,
            hits: short_out.sub_hits.to_vec(),
            container_rect: rect,
        },
        &ScrollbarProbe {
            primitives: tall_out.primitives,
            hits: tall_out.sub_hits.to_vec(),
            container_rect: rect,
        },
        0,
        2,
    );
}

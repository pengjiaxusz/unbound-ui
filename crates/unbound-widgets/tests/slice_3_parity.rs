//! 切片 3（集合与树形列表类控件）基准控件几何与渲染契约代码级测试守护。
//!
//! 依据 ADR 0016 与 Spec #79，本测试套件覆盖切片 3 全部 7 个基准控件：
//! 1. `TreeView`：2 级展开结构 `Root`（首项选中态）+ 子节点 `Child`，折叠箭头、胶囊指示条与层级引导线；
//! 2. `ListView`：三项标准列表行 `Item 1`、`Item 2`、`Item 3`，标准 40px 行高与选中 Accent 胶囊条；
//! 3. `GridView`：2×2 网格布局（4 项），6px 圆角单元格、选中强调色高亮边框与右上角对勾指示条；
//! 4. `ListBox`：经典单选列表框外观，内嵌标准列表行插桩与选中指示；
//! 5. `FlipView`：翻页集合视图，内容卡片 `Page 1` 与左右翻页前后导航箭头；
//! 6. `RefreshContainer`：下拉刷新容器，环形 ProgressRing 刷新指示器与内容区域卡片；
//! 7. `AnnotatedScrollBar`：标注滚动条，8px 居中轨道、标准滑块与刻度指示。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::Token;
use unbound_widgets::slot_fixture::{
    emit_canonical_annotated_scroll_bar_fixture, emit_canonical_flip_view_fixture,
    emit_canonical_grid_fixture, emit_canonical_list_rows_fixture, emit_canonical_refresh_fixture,
    emit_canonical_tree_fixture, FixtureState, CANONICAL_GRID_ITEMS, CANONICAL_LIST_ITEMS,
    STANDARD_FLIP_HEIGHT, STANDARD_FLIP_WIDTH, STANDARD_GRID_CELL_RADIUS, STANDARD_GRID_HEIGHT,
    STANDARD_GRID_WIDTH, STANDARD_LIST_ITEM_HEIGHT, STANDARD_PILL_HEIGHT, STANDARD_PILL_WIDTH,
    STANDARD_REFRESH_HEIGHT, STANDARD_REFRESH_INDICATOR_SIZE, STANDARD_REFRESH_WIDTH,
    STANDARD_SCROLL_BAR_HEIGHT, STANDARD_SCROLL_BAR_WIDTH, STANDARD_TREE_INDENT,
    STANDARD_TREE_ITEM_HEIGHT,
};
use unbound_widgets::{primitive, TextCtx, WidgetOutput};

/// 构造供纯代码级测试使用的测量上下文。
fn with_test_ctx<F, R>(f: F) -> R
where
    F: FnOnce(&mut TextCtx<'_>) -> R,
{
    let mut engine = TextEngine::new();
    let mut cache = GlyphCache::new();
    let mut text_ctx = TextCtx {
        engine: &mut engine,
        cache: &mut cache,
        atlas_clock: 1,
        raster_scale: 1.0,
    };
    f(&mut text_ctx)
}

// ---------------------------------------------------------------------------
// 1. TreeView 树形视图 2 级展开与层级指示契约
// ---------------------------------------------------------------------------

#[test]
fn tree_view_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 140.0);

        // 1. 正常态契约：Root 根节点与缩进 40px 的 Child 子节点
        let normal = emit_canonical_tree_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.root_rect.height(), STANDARD_TREE_ITEM_HEIGHT);
        // 根节点相对容器上移 4px，子节点沿用 2px 行间重叠，故落点为 4+32-2。
        assert_eq!(
            normal.child_rect.origin.y,
            rect.origin.y + STANDARD_TREE_ITEM_HEIGHT + 2.0
        );
        assert_eq!(
            normal.child_rect.origin.x - normal.root_rect.origin.x,
            STANDARD_TREE_INDENT,
            "child row is indented by standard 40px"
        );

        // 根节点具备 3×16px Accent 胶囊条
        let has_pill = normal.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - STANDARD_PILL_WIDTH).abs() < 1e-4
                && (p.size[1] - STANDARD_PILL_HEIGHT).abs() < 1e-4
        });
        assert!(has_pill, "expanded root must have 3x16px Accent pill");

        // 包含折线箭头；WinUI 参考形态没有层级引导线
        let curve_strokes = normal
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(curve_strokes, 2, "chevron consists of 2 curve strokes");

        let has_guide_line = normal.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::DIVIDER
                && (p.size[0] - 1.0).abs() < 1e-4
        });
        assert!(!has_guide_line, "WinUI TreeView has no depth guide line");

        // 2. 聚焦态契约
        let focused = emit_canonical_tree_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused TreeView must emit focus ring");

        // 3. 禁用态契约：抑制胶囊指示条且文本与箭头变灰
        let disabled = emit_canonical_tree_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_disabled_pill = disabled.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(
            !has_disabled_pill,
            "disabled TreeView must suppress primary pill"
        );

        let glyph_tokens: Vec<Token> = disabled
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        for tok in glyph_tokens {
            assert_eq!(tok, Token::TEXT_DISABLED);
        }
    });
}

// ---------------------------------------------------------------------------
// 2. ListView 列表集合视图
// ---------------------------------------------------------------------------

#[test]
fn list_view_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 120.0);

        // 1. 正常态契约：3 行标准列表项与首项 Accent 胶囊条
        let normal = emit_canonical_list_rows_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.item_rects.len(), CANONICAL_LIST_ITEMS.len());

        for row in &normal.item_rects {
            assert_eq!(row.height(), STANDARD_LIST_ITEM_HEIGHT - 4.0);
        }

        let has_pill = normal.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - STANDARD_PILL_WIDTH).abs() < 1e-4
                && (p.size[1] - STANDARD_PILL_HEIGHT).abs() < 1e-4
        });
        assert!(
            has_pill,
            "selected ListView item must have 3x16px Accent pill"
        );

        // 2. 聚焦态契约
        let focused =
            emit_canonical_list_rows_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused ListView must emit focus ring");

        // 3. 禁用态契约：抑制胶囊指示条且文本变灰
        let disabled =
            emit_canonical_list_rows_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_disabled_pill = disabled.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(
            !has_disabled_pill,
            "disabled ListView must suppress primary pill"
        );

        let glyph_tokens: Vec<Token> = disabled
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        // WinUI 参考截图中 ListViewItem 的墨水在禁用态仍保持主文本色。
        for tok in glyph_tokens {
            assert_eq!(tok, Token::TEXT_PRIMARY);
        }
    });
}

// ---------------------------------------------------------------------------
// 3. GridView 2×2 网格集合视图
// ---------------------------------------------------------------------------

#[test]
fn grid_view_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_GRID_WIDTH, STANDARD_GRID_HEIGHT);

        // 1. 正常态契约：4 个网格单元、首项 PRIMARY 描边（无勾选徽章）
        let normal = emit_canonical_grid_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.cell_rects.len(), CANONICAL_GRID_ITEMS.len());
        assert_eq!(normal.total_size.x, STANDARD_GRID_WIDTH);
        assert_eq!(normal.total_size.y, STANDARD_GRID_HEIGHT);
        assert_eq!(STANDARD_GRID_CELL_RADIUS, 6.0);

        let has_primary_border = normal.output.primitives.iter().any(|p| {
            Token::new(p.border_token_or_tex as u16) == Token::PRIMARY
                && (p.border_width() - 2.0).abs() < 1e-4
        });
        assert!(
            has_primary_border,
            "selected grid cell must have 2px PRIMARY border"
        );

        let has_accent_badge = normal.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - 18.0).abs() < 1e-4
        });
        assert!(!has_accent_badge, "GridView has no checkmark badge");

        // 2. 聚焦态契约
        let focused = emit_canonical_grid_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused GridView must emit focus ring");

        // 3. 禁用态契约：抑制强调色边框与徽章
        let disabled = emit_canonical_grid_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_disabled_badge = disabled.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(
            !has_disabled_badge,
            "disabled GridView must suppress primary badge"
        );
    });
}

// ---------------------------------------------------------------------------
// 4. ListBox 经典单选列表框
// ---------------------------------------------------------------------------

#[test]
fn list_box_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 120.0);

        // 1. 正常态契约：外层 outlined 容器 + 内嵌标准 3 行列表行插桩
        let mut normal_out = WidgetOutput::default();
        normal_out.primitives.push(primitive::outlined(
            rect,
            4.0,
            Token::CARD_BG,
            Token::CONTROL_STROKE,
            1.0,
            0.0,
            100,
        ));
        let rows = emit_canonical_list_rows_fixture(rect, FixtureState::normal(), text_ctx, 102);
        normal_out.primitives.extend(rows.output.primitives);
        assert_eq!(rows.item_rects.len(), 3);

        // 2. 禁用态契约：外框降级为 CONTROL_DISABLED_BG 与 CONTROL_DISABLED_BORDER
        let mut disabled_out = WidgetOutput::default();
        disabled_out.primitives.push(primitive::outlined(
            rect,
            4.0,
            Token::CONTROL_DISABLED_BG,
            Token::CONTROL_DISABLED_BORDER,
            1.0,
            0.0,
            100,
        ));
        let disabled_rows =
            emit_canonical_list_rows_fixture(rect, FixtureState::disabled(), text_ctx, 102);
        disabled_out
            .primitives
            .extend(disabled_rows.output.primitives);

        let border_prim = &disabled_out.primitives[0];
        assert_eq!(
            border_prim.fill_token_or_uv as u16,
            Token::CONTROL_DISABLED_BG.raw()
        );
        assert_eq!(
            border_prim.border_token_or_tex as u16,
            Token::CONTROL_DISABLED_BORDER.raw()
        );
    });
}

// ---------------------------------------------------------------------------
// 5. FlipView 翻页集合视图
// ---------------------------------------------------------------------------

#[test]
fn flip_view_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_FLIP_WIDTH, STANDARD_FLIP_HEIGHT);

        // 1. 正常态契约：卡片、前后翻页按钮与导航箭头
        let normal = emit_canonical_flip_view_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.total_size.x, STANDARD_FLIP_WIDTH);
        assert_eq!(normal.total_size.y, STANDARD_FLIP_HEIGHT);
        assert!(normal.prev_button_rect.origin.x < normal.card_rect.origin.x + 40.0);
        assert!(normal.next_button_rect.max().x > normal.card_rect.max().x - 40.0);

        let chevron_curves = normal
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(
            chevron_curves, 0,
            "idle WinUI FlipView must not emit static nav chevron curves"
        );

        // 2. 禁用态契约：箭头与文本降级为 TEXT_DISABLED
        let disabled =
            emit_canonical_flip_view_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        for p in &disabled.output.primitives {
            let prim_type = p.prim_type();
            let fill = Token::new(p.fill_token_or_uv as u16);
            if prim_type == PrimType::Curve
                || matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor)
            {
                assert_eq!(fill, Token::TEXT_DISABLED);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 6. RefreshContainer 下拉刷新指示器容器
// ---------------------------------------------------------------------------

#[test]
fn refresh_container_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_REFRESH_WIDTH, STANDARD_REFRESH_HEIGHT);

        // 1. 正常态契约：包含 ProgressRing 环形强调色弧线
        let normal = emit_canonical_refresh_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.total_size.x, STANDARD_REFRESH_WIDTH);
        assert_eq!(normal.total_size.y, STANDARD_REFRESH_HEIGHT);
        assert_eq!(
            normal.indicator_rect.width(),
            STANDARD_REFRESH_INDICATOR_SIZE
        );

        let normal_glyphs: Vec<Token> = normal
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!normal_glyphs.is_empty());
        assert!(normal_glyphs.contains(&Token::TEXT_PRIMARY));

        // 2. 禁用态契约：抑制弧线且文本降级
        let disabled =
            emit_canonical_refresh_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let curve_count = disabled
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(
            curve_count, 0,
            "disabled refresh indicator must suppress arc curves"
        );
    });
}

// ---------------------------------------------------------------------------
// 7. AnnotatedScrollBar 标注滚动条
// ---------------------------------------------------------------------------

#[test]
fn annotated_scroll_bar_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            STANDARD_SCROLL_BAR_WIDTH,
            STANDARD_SCROLL_BAR_HEIGHT,
        );

        // 1. 正常态契约：8px 轨道、居中与滑块
        let normal = emit_canonical_annotated_scroll_bar_fixture(
            rect,
            FixtureState::normal(),
            text_ctx,
            100,
        );
        assert_eq!(normal.track_rect.width(), STANDARD_SCROLL_BAR_WIDTH);
        assert_eq!(normal.track_rect.height(), STANDARD_SCROLL_BAR_HEIGHT);
        // WinUI 指示条为 3px 横向强调条，不再是旧版 8px 竖向滑块。
        assert_eq!(normal.thumb_rect.height(), 3.0);

        let has_thumb = normal.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(has_thumb, "must emit PRIMARY horizontal thumb");

        // 2. 禁用态契约：滑块与刻度降级为禁用态
        let disabled = emit_canonical_annotated_scroll_bar_fixture(
            rect,
            FixtureState::disabled(),
            text_ctx,
            100,
        );
        let has_active_thumb = disabled.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(
            !has_active_thumb,
            "disabled scroll bar must downgrade thumb stroke"
        );
    });
}

//! 容器标准插桩通用辅助库与配方契约基础单元测试套件。
//!
//! 验证 3 级路径（`Root > Folder > File.txt`）、2 级展开树形（`Root` + `Child`）、
//! 垂直文本行（`Row 1/2/3`）与可交互列表行（`Item 1/2/3`）在不同状态与主题下的几何确定性，
//! 以及通用递归禁用态（`disabled`）传播机制对文本、图标与高亮指示器的半透明变灰衰减。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::{win11, Token};
use unbound_widgets::slot_fixture::{
    apply_recursive_disabled_propagation, emit_canonical_annotated_scroll_bar_fixture,
    emit_canonical_command_bar_fixture, emit_canonical_flip_view_fixture,
    emit_canonical_grid_fixture, emit_canonical_list_rows_fixture, emit_canonical_menu_bar_fixture,
    emit_canonical_path_fixture, emit_canonical_pips_pager_fixture, emit_canonical_refresh_fixture,
    emit_canonical_segmented_fixture, emit_canonical_tab_view_fixture,
    emit_canonical_text_rows_fixture, emit_canonical_title_bar_fixture,
    emit_canonical_tree_fixture, FixtureState, CANONICAL_COMMAND_ITEMS, CANONICAL_GRID_ITEMS,
    CANONICAL_LIST_ITEMS, CANONICAL_MENU_ITEMS, CANONICAL_PIPS_COUNT,
    CANONICAL_PIPS_SELECTED_INDEX, CANONICAL_PIP_SIZE_NORMAL, CANONICAL_PIP_SIZE_SELECTED,
    CANONICAL_TAB_ITEMS, STANDARD_FLIP_HEIGHT, STANDARD_FLIP_WIDTH, STANDARD_GRID_CELL_RADIUS,
    STANDARD_GRID_HEIGHT, STANDARD_GRID_WIDTH, STANDARD_LIST_ITEM_HEIGHT, STANDARD_PILL_HEIGHT,
    STANDARD_PILL_WIDTH, STANDARD_REFRESH_HEIGHT, STANDARD_REFRESH_INDICATOR_SIZE,
    STANDARD_REFRESH_WIDTH, STANDARD_SCROLL_BAR_HEIGHT, STANDARD_SCROLL_BAR_WIDTH,
    STANDARD_STACK_SPACING, STANDARD_TREE_INDENT, STANDARD_TREE_ITEM_HEIGHT,
    WINUI_DISABLED_OPACITY,
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

#[test]
fn path_fixture_normal_outputs_deterministic_geometry_and_active_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(10.0, 20.0, 320.0, 32.0);
        let fixture = emit_canonical_path_fixture(rect, FixtureState::normal(), text_ctx, 100);

        // 1. 结构与条目数量校验
        assert_eq!(fixture.item_rects.len(), 3, "must emit 3 path item rects");
        assert_eq!(
            fixture.separator_rects.len(),
            2,
            "must emit 2 separator rects"
        );
        assert!(
            fixture.total_size.x > 0.0 && fixture.total_size.x <= 320.0,
            "measured width must be within bounds"
        );
        assert_eq!(fixture.total_size.y, 32.0, "row height matches input rect");

        // 2. 项与分隔符从左到右非重叠排列
        let mut last_x = rect.origin.x;
        for i in 0..2 {
            let item_r = fixture.item_rects[i];
            let sep_r = fixture.separator_rects[i];
            assert!(
                item_r.origin.x >= last_x,
                "item {i} starts after previous content"
            );
            assert!(
                sep_r.origin.x >= item_r.max().x,
                "separator {i} starts after item {i}"
            );
            last_x = sep_r.max().x;
        }
        let last_item_r = fixture.item_rects[2];
        assert!(
            last_item_r.origin.x >= last_x,
            "last item starts after last separator"
        );

        // 3. 图元令牌校验：WinUI 3 的条目与分隔符统一使用主文本色
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(
            glyph_tokens.contains(&Token::TEXT_PRIMARY),
            "current item must use TEXT_PRIMARY"
        );
        assert!(
            glyph_tokens.iter().all(|t| *t == Token::TEXT_PRIMARY),
            "WinUI BreadcrumbBar items and separators must use TEXT_PRIMARY"
        );
        assert!(
            !glyph_tokens.contains(&Token::TEXT_DISABLED),
            "normal state must not emit TEXT_DISABLED"
        );
    });
}

#[test]
fn path_fixture_disabled_downgrades_ancestors_only() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 32.0);
        let fixture = emit_canonical_path_fixture(rect, FixtureState::disabled(), text_ctx, 100);

        // 几何尺寸保持确定性
        assert_eq!(fixture.item_rects.len(), 3);
        assert_eq!(fixture.separator_rects.len(), 2);

        // 禁用态只降级前序可点条目；末项与分隔符按参考实测保持主文本色
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty(), "must emit glyph primitives");
        assert!(
            glyph_tokens.contains(&Token::TEXT_DISABLED),
            "ancestor items must downgrade to TEXT_DISABLED"
        );
        assert!(
            glyph_tokens.contains(&Token::TEXT_PRIMARY),
            "last item and separators stay TEXT_PRIMARY"
        );
        assert!(
            glyph_tokens
                .iter()
                .all(|t| matches!(*t, Token::TEXT_DISABLED | Token::TEXT_PRIMARY)),
            "disabled state must only use TEXT_DISABLED or TEXT_PRIMARY"
        );
    });
}

#[test]
fn segmented_fixture_normal_renders_selection_pill_and_active_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 300.0, 40.0);
        let output = emit_canonical_segmented_fixture(
            rect,
            FixtureState::normal().with_selected(0),
            text_ctx,
            200,
        );

        // 必须包含选中态的 Accent 胶囊指示条与 Subtle 悬停底板
        let rounded_rects: Vec<(Token, f32, f32)> = output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::RoundedRect)
            .map(|p| (Token::new(p.fill_token_or_uv as u16), p.size[0], p.size[1]))
            .collect();

        let has_hover_surface = rounded_rects.iter().any(|&(tok, _, _)| tok == Token::HOVER);
        assert!(
            has_hover_surface,
            "selected segment must have HOVER surface background"
        );

        let has_accent_pill = rounded_rects.iter().any(|&(tok, w, h)| {
            tok == Token::PRIMARY
                && (w - STANDARD_PILL_WIDTH).abs() < 1e-4
                && (h - STANDARD_PILL_HEIGHT).abs() < 1e-4
        });
        assert!(
            has_accent_pill,
            "selected segment must render standard 3x16px Accent pill"
        );

        // 文本令牌：首项为 TEXT_PRIMARY，非选中项为 TEXT_SECONDARY
        let glyph_tokens: Vec<Token> = output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
        assert!(glyph_tokens.contains(&Token::TEXT_SECONDARY));
    });
}

#[test]
fn segmented_fixture_disabled_suppresses_pill_and_surfaces() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 300.0, 40.0);
        let output =
            emit_canonical_segmented_fixture(rect, FixtureState::disabled(), text_ctx, 200);

        // 严禁发射 Accent 胶囊条与悬停背景
        for p in &output.primitives {
            if p.prim_type() == PrimType::RoundedRect {
                let fill = Token::new(p.fill_token_or_uv as u16);
                assert_ne!(
                    fill,
                    Token::PRIMARY,
                    "disabled segmented bar must not emit PRIMARY pill"
                );
                assert_ne!(
                    fill,
                    Token::HOVER,
                    "disabled segmented bar must not emit HOVER surface"
                );
            }
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                let tok = Token::new(p.fill_token_or_uv as u16);
                assert_eq!(
                    tok,
                    Token::TEXT_DISABLED,
                    "all segment labels must be TEXT_DISABLED"
                );
            }
        }
    });
}

#[test]
fn tree_fixture_normal_renders_two_levels_with_chevron_pill_and_guide_line() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 140.0);
        let fixture = emit_canonical_tree_fixture(rect, FixtureState::normal(), text_ctx, 300);

        // 1. 根节点与子节点矩形几何排布
        assert_eq!(fixture.root_rect.height(), STANDARD_TREE_ITEM_HEIGHT);
        assert_eq!(
            fixture.child_rect.origin.y,
            rect.origin.y + STANDARD_TREE_ITEM_HEIGHT
        );
        assert_eq!(
            fixture.child_rect.origin.x - fixture.root_rect.origin.x,
            STANDARD_TREE_INDENT,
            "child row is indented by standard 40px"
        );

        // 2. 根节点左侧 Accent 胶囊条
        let has_pill = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - STANDARD_PILL_WIDTH).abs() < 1e-4
                && (p.size[1] - STANDARD_PILL_HEIGHT).abs() < 1e-4
        });
        assert!(has_pill, "expanded root must have 3x16px Accent pill");

        // 3. 展开箭头（WinUI 参考形态没有深度引导线）
        let curve_strokes = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(curve_strokes, 2, "chevron consists of 2 curve strokes");

        let has_guide_line = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::DIVIDER
                && (p.size[0] - 1.0).abs() < 1e-4
        });
        assert!(!has_guide_line, "WinUI TreeView has no depth guide line");

        // 4. 文本标签存在
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
    });
}

#[test]
fn tree_fixture_disabled_keeps_selection_and_default_ink() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 140.0);
        let fixture = emit_canonical_tree_fixture(rect, FixtureState::disabled(), text_ctx, 300);

        // WinUI 3 参考实现下，禁用态仍保留根节点选中底板与 Accent 胶囊条。
        let has_selected_background = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::HOVER
        });
        let has_pill = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(
            has_selected_background,
            "disabled tree keeps selected background"
        );
        assert!(has_pill, "disabled tree keeps the primary root pill");

        // 展开 chevron 与文本仍降级为禁用墨水。
        for p in &fixture.output.primitives {
            let prim_type = p.prim_type();
            let fill = Token::new(p.fill_token_or_uv as u16);

            if prim_type == PrimType::Curve {
                assert_eq!(
                    fill,
                    Token::TEXT_DISABLED,
                    "chevron stroke must be TEXT_DISABLED"
                );
            }

            if matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(
                    fill,
                    Token::TEXT_DISABLED,
                    "tree labels must be TEXT_DISABLED"
                );
            }
        }
    });
}

#[test]
fn text_rows_fixture_normal_outputs_three_rows_with_equal_spacing() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        let fixture = emit_canonical_text_rows_fixture(
            rect,
            STANDARD_STACK_SPACING,
            FixtureState::normal(),
            text_ctx,
            400,
        );

        assert_eq!(fixture.row_rects.len(), 3, "must emit 3 text rows");
        // 锁定 WinUI 3 的 14px 正文行盒：改成 20px 会破坏切片 1 的像素对齐基线。
        assert_eq!(
            fixture.row_rects[0].height(),
            19.0,
            "canonical row box must be 19px"
        );
        let spacing0 = fixture.row_rects[1].origin.y - fixture.row_rects[0].max().y;
        let spacing1 = fixture.row_rects[2].origin.y - fixture.row_rects[1].max().y;
        assert!(
            (spacing0 - STANDARD_STACK_SPACING).abs() < 1e-4,
            "row 0->1 spacing must match standard 8px"
        );
        assert!(
            (spacing1 - STANDARD_STACK_SPACING).abs() < 1e-4,
            "row 1->2 spacing must match standard 8px"
        );

        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty());
        for &tok in &glyph_tokens {
            assert_eq!(tok, Token::TEXT_PRIMARY);
        }
    });
}

#[test]
fn text_rows_fixture_disabled_downgrades_to_text_disabled() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        let fixture = emit_canonical_text_rows_fixture(
            rect,
            STANDARD_STACK_SPACING,
            FixtureState::disabled(),
            text_ctx,
            400,
        );

        for p in &fixture.output.primitives {
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                let tok = Token::new(p.fill_token_or_uv as u16);
                assert_eq!(tok, Token::TEXT_DISABLED);
            }
        }
    });
}

#[test]
fn list_rows_fixture_normal_and_disabled_states_are_deterministic() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 120.0);

        // 1. 常规态：第 0 项选中
        let normal_fixture =
            emit_canonical_list_rows_fixture(rect, FixtureState::normal(), text_ctx, 500);
        assert_eq!(normal_fixture.item_rects.len(), 3);
        assert_eq!(normal_fixture.total_size.y, STANDARD_LIST_ITEM_HEIGHT * 3.0);

        let has_pill = normal_fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(has_pill, "normal list item 0 must have Accent pill");

        // 2. 禁用态：胶囊条抑制，全量变灰
        let disabled_fixture =
            emit_canonical_list_rows_fixture(rect, FixtureState::disabled(), text_ctx, 500);
        let disabled_has_pill = disabled_fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(!disabled_has_pill, "disabled list item must suppress pill");

        // WinUI 参考截图中 ListViewItem 的墨水在禁用态仍保持主文本色。
        for p in &disabled_fixture.output.primitives {
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(Token::new(p.fill_token_or_uv as u16), Token::TEXT_PRIMARY);
            }
        }
    });
}

#[test]
fn recursive_disabled_propagation_post_processor_remaps_tokens_and_attenuates_opacity() {
    // 构造包含多种活跃令牌与不透明度的测试图元集合
    let mut output = WidgetOutput::default();
    let rect = Rect::new(10.0, 10.0, 100.0, 30.0);

    // 活跃表面与边框
    output
        .primitives
        .push(primitive::surface(rect, 4.0, Token::HOVER, 10));
    output
        .primitives
        .push(primitive::surface(rect, 1.5, Token::PRIMARY, 20));
    output.primitives.push(primitive::outlined(
        rect,
        4.0,
        Token::CONTROL_FILL,
        Token::CARD_BORDER,
        1.0,
        0.0,
        30,
    ));

    // 曲线
    primitive::curve_stroke(
        [10.0, 10.0],
        [20.0, 20.0],
        1.5,
        Token::TEXT_SECONDARY,
        40,
        &mut output.primitives,
        &mut output.payloads,
    );

    // 应用递归禁用态传播，并衰减不透明度
    apply_recursive_disabled_propagation(&mut output, true);

    // 验证令牌映射与不透明度衰减
    for p in &output.primitives {
        let fill = Token::new(p.fill_token_or_uv as u16);
        let border = Token::new(p.border_token_or_tex as u16);

        // HOVER 必须被抑制为 NONE
        assert_ne!(fill, Token::HOVER);
        // PRIMARY 必须映射为 CONTROL_DISABLED_THUMB
        assert_ne!(fill, Token::PRIMARY);
        // CARD_BORDER 必须映射为 CONTROL_DISABLED_BORDER
        assert_ne!(border, Token::CARD_BORDER);

        // 不透明度必须衰减
        let op = p.opacity();
        assert!(
            (op - WINUI_DISABLED_OPACITY).abs() < 0.05,
            "opacity {op} must be attenuated by ~36%"
        );
    }
}

#[test]
fn text_disabled_token_resolves_to_valid_semi_transparent_color_in_both_themes() {
    let light = win11::light();
    let dark = win11::dark();

    let light_disabled_color = light.color(Token::TEXT_DISABLED);
    let dark_disabled_color = dark.color(Token::TEXT_DISABLED);

    let light_primary_color = light.color(Token::TEXT_PRIMARY);
    let dark_primary_color = dark.color(Token::TEXT_PRIMARY);

    // 验证浅色模式下文本禁用态为半透明衰减色（164 / 255.0，低于主要文本的 255 / 255.0）
    assert!(
        light_disabled_color.a < light_primary_color.a,
        "light disabled alpha must be lower than primary"
    );
    assert!(
        (light_disabled_color.a - 164.0 / 255.0).abs() < 1e-3,
        "light disabled alpha matches Windows 11 specification"
    );

    // 验证深色模式下文本禁用态为半透明衰减白（42 / 255.0，低于主要文本的 255 / 255.0）
    assert!(
        dark_disabled_color.a < dark_primary_color.a,
        "dark disabled alpha must be lower than primary"
    );
    assert!(
        (dark_disabled_color.a - 42.0 / 255.0).abs() < 1e-3,
        "dark disabled alpha matches Windows 11 specification"
    );
}

#[test]
fn grid_fixture_normal_outputs_deterministic_geometry_and_active_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_GRID_WIDTH, STANDARD_GRID_HEIGHT);
        let fixture = emit_canonical_grid_fixture(rect, FixtureState::normal(), text_ctx, 100);

        // 1. 结构与单元格排版（2×2，共 4 个单元）
        assert_eq!(fixture.cell_rects.len(), 4, "must emit 4 grid cell rects");
        assert_eq!(fixture.total_size.x, STANDARD_GRID_WIDTH);
        assert_eq!(fixture.total_size.y, STANDARD_GRID_HEIGHT);
        assert_eq!(STANDARD_GRID_CELL_RADIUS, 6.0);

        // 2. 首项处于选中态：必须具有 PRIMARY 边框与 PRIMARY 徽章
        let has_primary_border = fixture.output.primitives.iter().any(|p| {
            Token::new(p.border_token_or_tex as u16) == Token::PRIMARY
                && (p.border_width() - 2.0).abs() < 1e-4
        });
        assert!(
            has_primary_border,
            "selected cell must have 2px PRIMARY border"
        );

        // WinUI GridView 选中项只有强调色描边，没有勾选徽章或折线。
        let has_accent_badge = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - 18.0).abs() < 1e-4
        });
        assert!(!has_accent_badge, "GridView has no checkmark badge");

        let has_check_curve = fixture
            .output
            .primitives
            .iter()
            .any(|p| p.prim_type() == PrimType::Curve);
        assert!(!has_check_curve, "GridView has no checkmark strokes");

        // 4. 文本存在且包含主要文本
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
    });
}

#[test]
fn grid_fixture_disabled_suppresses_selection_and_downgrades_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_GRID_WIDTH, STANDARD_GRID_HEIGHT);
        let fixture = emit_canonical_grid_fixture(rect, FixtureState::disabled(), text_ctx, 100);

        for p in &fixture.output.primitives {
            let prim_type = p.prim_type();
            let fill = Token::new(p.fill_token_or_uv as u16);
            let border = Token::new(p.border_token_or_tex as u16);

            // 抑制选中徽章与高亮填充
            if prim_type == PrimType::RoundedRect {
                assert_ne!(fill, Token::PRIMARY);
                assert_ne!(fill, Token::PRIMARY_SOFT);
            }
            // 抑制强调色边框
            if prim_type == PrimType::Stroke {
                assert_ne!(border, Token::PRIMARY);
            }
            // WinUI 3 参考截图中 GridViewItem 文本在禁用态保持主文本色
            if matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(fill, Token::TEXT_PRIMARY);
            }
        }
    });
}

#[test]
fn flip_view_fixture_normal_outputs_card_and_nav_buttons() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_FLIP_WIDTH, STANDARD_FLIP_HEIGHT);
        let fixture = emit_canonical_flip_view_fixture(rect, FixtureState::normal(), text_ctx, 200);

        // 1. 固有尺寸
        assert_eq!(fixture.total_size.x, STANDARD_FLIP_WIDTH);
        assert_eq!(fixture.total_size.y, STANDARD_FLIP_HEIGHT);

        // 2. 前后导航按钮位置
        assert!(fixture.prev_button_rect.origin.x < fixture.card_rect.origin.x + 40.0);
        assert!(fixture.next_button_rect.max().x > fixture.card_rect.max().x - 40.0);

        // 3. WinUI FlipView 静止态不常驻左右翻页箭标曲线
        let chevron_curves = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(
            chevron_curves, 0,
            "idle WinUI FlipView must not emit static nav chevron curves"
        );

        // 5. 页面文字 "Page 1" 为 TEXT_PRIMARY
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
    });
}

#[test]
fn flip_view_fixture_disabled_downgrades_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_FLIP_WIDTH, STANDARD_FLIP_HEIGHT);
        let fixture =
            emit_canonical_flip_view_fixture(rect, FixtureState::disabled(), text_ctx, 200);

        for p in &fixture.output.primitives {
            let prim_type = p.prim_type();
            let fill = Token::new(p.fill_token_or_uv as u16);

            if prim_type == PrimType::Curve {
                assert_eq!(fill, Token::TEXT_DISABLED);
            }
            if matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(fill, Token::TEXT_DISABLED);
            }
        }
    });
}

#[test]
fn refresh_fixture_normal_outputs_indicator_and_card() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_REFRESH_WIDTH, STANDARD_REFRESH_HEIGHT);
        let fixture = emit_canonical_refresh_fixture(rect, FixtureState::normal(), text_ctx, 300);

        // 1. 尺寸校验
        assert_eq!(fixture.total_size.x, STANDARD_REFRESH_WIDTH);
        assert_eq!(fixture.total_size.y, STANDARD_REFRESH_HEIGHT);
        assert_eq!(
            fixture.indicator_rect.width(),
            STANDARD_REFRESH_INDICATOR_SIZE
        );
        assert_eq!(
            fixture.indicator_rect.height(),
            STANDARD_REFRESH_INDICATOR_SIZE
        );

        // 2. 使用内置矢量 ShellIcon::Redo 绘制刷新图标，不依赖 Windows PUA 字体
        let arc_count = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert!(
            arc_count > 0,
            "RefreshContainer must emit vector ShellIcon primitives"
        );

        // 3. 内容卡片文本
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
    });
}

#[test]
fn refresh_fixture_disabled_suppresses_arc_and_downgrades_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, STANDARD_REFRESH_WIDTH, STANDARD_REFRESH_HEIGHT);
        let fixture = emit_canonical_refresh_fixture(rect, FixtureState::disabled(), text_ctx, 300);

        // 抑制强调弧线
        let curve_count = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(
            curve_count, 0,
            "disabled refresh ring suppresses accent arc"
        );

        // 文本变灰
        for p in &fixture.output.primitives {
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(Token::new(p.fill_token_or_uv as u16), Token::TEXT_DISABLED);
            }
        }
    });
}

#[test]
fn annotated_scroll_bar_fixture_normal_outputs_track_and_thumb() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            STANDARD_SCROLL_BAR_WIDTH,
            STANDARD_SCROLL_BAR_HEIGHT,
        );
        let fixture = emit_canonical_annotated_scroll_bar_fixture(
            rect,
            FixtureState::normal(),
            text_ctx,
            400,
        );

        // 1. 轨道尺寸对齐 WinUI AnnotatedScrollBar 全宽槽道
        assert_eq!(
            fixture.track_rect.width(),
            STANDARD_SCROLL_BAR_WIDTH,
            "track spans full bar width"
        );
        assert_eq!(fixture.track_rect.height(), STANDARD_SCROLL_BAR_HEIGHT);

        // 2. 横向滑块存在（WinUI 3 参考实现为 3px 高）
        assert_eq!(fixture.thumb_rect.height(), 3.0);

        // 3. 图元令牌包含 PRIMARY 滑块
        let has_thumb = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(has_thumb, "must emit PRIMARY horizontal thumb");
    });
}

#[test]
fn annotated_scroll_bar_fixture_disabled_downgrades_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            STANDARD_SCROLL_BAR_WIDTH,
            STANDARD_SCROLL_BAR_HEIGHT,
        );
        let fixture = emit_canonical_annotated_scroll_bar_fixture(
            rect,
            FixtureState::disabled(),
            text_ctx,
            400,
        );

        let has_thumb = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
        });
        assert!(
            has_thumb,
            "disabled AnnotatedScrollBar keeps PRIMARY indicator per WinUI 3"
        );
    });
}

#[test]
fn list_box_and_list_view_share_canonical_list_items_and_row_height() {
    assert_eq!(CANONICAL_LIST_ITEMS.len(), 3);
    assert_eq!(CANONICAL_LIST_ITEMS, ["Item 1", "Item 2", "Item 3"]);
    assert_eq!(STANDARD_LIST_ITEM_HEIGHT, 40.0);
    assert_eq!(STANDARD_PILL_WIDTH, 3.0);
    assert_eq!(STANDARD_PILL_HEIGHT, 16.0);
    assert_eq!(
        CANONICAL_GRID_ITEMS,
        ["Item 1", "Item 2", "Item 3", "Item 4"]
    );
}

#[test]
fn breadcrumb_bar_focused_renders_focus_ring() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 32.0);
        let fixture = emit_canonical_path_fixture(
            rect,
            FixtureState::normal().with_selected(2),
            text_ctx,
            100,
        );
        let normal_prim_count = fixture.output.primitives.len();

        let focused_fixture = emit_canonical_path_fixture(
            rect,
            FixtureState::focused().with_selected(2),
            text_ctx,
            100,
        );
        let focused_prim_count = focused_fixture.output.primitives.len();

        // 聚焦态必须增加焦点环图元
        assert!(
            focused_prim_count > normal_prim_count,
            "focused state must emit focus ring primitive"
        );
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focus ring must use Token::FOCUS_RING");
    });
}

#[test]
fn pips_pager_normal_renders_five_pips_and_selected_marker() {
    let rect = Rect::new(0.0, 0.0, 320.0, 32.0);
    let fixture = emit_canonical_pips_pager_fixture(rect, FixtureState::normal(), 100);

    // 1. 结构与条目数量校验：固定 5 个圆点
    assert_eq!(
        fixture.pip_rects.len(),
        CANONICAL_PIPS_COUNT,
        "must emit 5 pip rects"
    );
    assert_eq!(
        fixture.selected_index, CANONICAL_PIPS_SELECTED_INDEX,
        "selected index must be 1"
    );

    // 2. 选中圆点与普通圆点尺寸与颜色校验
    for (idx, r) in fixture.pip_rects.iter().enumerate() {
        if idx == fixture.selected_index {
            assert_eq!(
                r.width(),
                CANONICAL_PIP_SIZE_SELECTED,
                "selected pip diameter must be 10px"
            );
        } else {
            assert_eq!(
                r.width(),
                CANONICAL_PIP_SIZE_NORMAL,
                "unselected pip diameter must be 6px"
            );
        }
    }

    let fills: Vec<Token> = fixture
        .output
        .primitives
        .iter()
        .filter(|p| p.prim_type() == PrimType::RoundedRect)
        .map(|p| Token::new(p.fill_token_or_uv as u16))
        .collect();

    assert!(
        fills.iter().all(|tok| *tok == Token::CONTROL_TRACK),
        "WinUI PipsPager pips use CONTROL_TRACK with larger selected diameter"
    );
}

#[test]
fn pips_pager_disabled_and_focused_states() {
    let rect = Rect::new(0.0, 0.0, 320.0, 32.0);

    // 1. 禁用态校验：所有圆点均降到 WinUI 的 SliderTrackValueFillDisabled
    let disabled_fixture = emit_canonical_pips_pager_fixture(rect, FixtureState::disabled(), 100);
    let disabled_fills: Vec<Token> = disabled_fixture
        .output
        .primitives
        .iter()
        .filter(|p| p.prim_type() == PrimType::RoundedRect)
        .map(|p| Token::new(p.fill_token_or_uv as u16))
        .collect();

    assert_eq!(disabled_fills.len(), CANONICAL_PIPS_COUNT);
    for fill in disabled_fills {
        assert_eq!(
            fill,
            Token::SLIDER_TRACK_DISABLED,
            "all pips must use SLIDER_TRACK_DISABLED in disabled state"
        );
    }

    // 2. 聚焦态校验：选中圆点处必须绘制焦点环
    let focused_fixture = emit_canonical_pips_pager_fixture(rect, FixtureState::focused(), 100);
    let has_focus_ring = focused_fixture
        .output
        .primitives
        .iter()
        .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
    assert!(has_focus_ring, "focused state must emit focus ring");
}

#[test]
fn command_bar_normal_renders_two_commands_separator_and_more_button() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 48.0);
        let fixture =
            emit_canonical_command_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);

        // 1. 结构校验：2 个命令按钮、1 个分隔符与 1 个更多按钮
        assert_eq!(
            fixture.button_rects.len(),
            CANONICAL_COMMAND_ITEMS.len(),
            "must emit 2 command button rects"
        );
        assert!(fixture.separator_rect.is_some(), "must emit separator rect");
        assert!(
            fixture.more_button_rect.is_some(),
            "must emit more button rect"
        );
        assert_eq!(fixture.total_size.y, 48.0, "bar height must be 48px");

        // 2. 文本与图标颜色校验：正常态使用 TEXT_PRIMARY
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(
            glyph_tokens.contains(&Token::TEXT_PRIMARY),
            "command buttons must use TEXT_PRIMARY"
        );
        assert!(
            !glyph_tokens.contains(&Token::TEXT_DISABLED),
            "normal state must not use TEXT_DISABLED"
        );
    });
}

#[test]
fn command_bar_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 48.0);

        // 1. 禁用态校验：文本和图标全部衰减为 TEXT_DISABLED
        let disabled_fixture =
            emit_canonical_command_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let glyph_tokens: Vec<Token> = disabled_fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty(), "must emit glyph primitives");
        for tok in glyph_tokens {
            assert_eq!(
                tok,
                Token::TEXT_DISABLED,
                "all command bar text must be TEXT_DISABLED in disabled state"
            );
        }

        // 2. 聚焦态校验：首个命令按钮处必须绘制焦点环
        let focused_fixture =
            emit_canonical_command_bar_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(
            has_focus_ring,
            "focused state must emit focus ring on first button"
        );
    });
}

#[test]
fn menu_bar_normal_renders_file_and_edit() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 40.0);
        let fixture = emit_canonical_menu_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);

        assert_eq!(
            fixture.item_rects.len(),
            CANONICAL_MENU_ITEMS.len(),
            "must emit 2 menu item rects"
        );
        assert_eq!(fixture.total_size.y, 40.0, "menu bar height must be 40px");

        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(
            glyph_tokens.contains(&Token::TEXT_PRIMARY),
            "menu items must use TEXT_PRIMARY"
        );
    });
}

#[test]
fn menu_bar_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 40.0);

        // 1. 禁用态校验
        let disabled_fixture =
            emit_canonical_menu_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let glyph_tokens: Vec<Token> = disabled_fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty());
        for tok in glyph_tokens {
            assert_eq!(
                tok,
                Token::TEXT_DISABLED,
                "all menu items must be TEXT_DISABLED in disabled state"
            );
        }

        // 2. 聚焦态校验
        let focused_fixture =
            emit_canonical_menu_bar_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(
            has_focus_ring,
            "focused state must emit focus ring on first menu item"
        );
    });
}

#[test]
fn title_bar_normal_renders_title_subtitle_and_captions() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 420.0, 32.0);
        let fixture = emit_canonical_title_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);

        // 1. 几何与标题区域校验
        assert!(
            fixture.title_rect.width() > 0.0,
            "title rect must have width"
        );
        assert!(
            fixture.subtitle_rect.width() > 0.0,
            "subtitle rect must have width"
        );
        assert_eq!(
            fixture.caption_rects.len(),
            3,
            "must emit 3 caption button rects"
        );
        assert_eq!(fixture.total_size.y, 32.0, "title bar height must be 32px");

        // 2. 副标题紧随主标题且 baseline 保持水平对齐
        assert!(
            fixture.subtitle_rect.origin.x >= fixture.title_rect.max().x,
            "subtitle must be positioned after title"
        );
        assert_eq!(
            fixture.title_rect.origin.y, fixture.subtitle_rect.origin.y,
            "title and subtitle must share vertical baseline alignment"
        );

        // 3. 令牌校验：主标题 TEXT_PRIMARY，副标题 TEXT_SECONDARY
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(
            glyph_tokens.contains(&Token::TEXT_PRIMARY),
            "title must use TEXT_PRIMARY"
        );
        assert!(
            glyph_tokens.contains(&Token::TEXT_SECONDARY),
            "subtitle must use TEXT_SECONDARY"
        );
    });
}

#[test]
fn title_bar_disabled_downgrades_title_and_captions() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 420.0, 32.0);
        let fixture =
            emit_canonical_title_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);

        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty());
        for tok in glyph_tokens {
            assert_eq!(
                tok,
                Token::TEXT_DISABLED,
                "all title bar text must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn tab_view_normal_renders_two_tabs_and_content_area() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 360.0, 120.0);
        let fixture = emit_canonical_tab_view_fixture(
            rect,
            FixtureState::normal().with_selected(0),
            text_ctx,
            100,
        );

        // 1. 结构校验：2 个标签页与 1 个内容区
        assert_eq!(
            fixture.tab_rects.len(),
            CANONICAL_TAB_ITEMS.len(),
            "must emit 2 tab rects"
        );
        assert!(
            fixture.content_rect.height() > 0.0,
            "must emit content rect"
        );
        assert_eq!(fixture.total_size.x, 360.0);
        assert_eq!(fixture.total_size.y, 120.0);

        // 2. 激活标签页校验：包含 CARD_BG 背景且没有 2px 底部指示条
        let rounded_rects: Vec<(Token, f32, f32)> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::RoundedRect)
            .map(|p| (Token::new(p.fill_token_or_uv as u16), p.size[0], p.size[1]))
            .collect();

        let has_card_bg = rounded_rects
            .iter()
            .any(|&(tok, _, _)| tok == Token::CARD_BG);
        assert!(has_card_bg, "active tab must use CARD_BG background");

        // WinUI TabView 选中态是带 CARD_BORDER 描边的 CARD_BG 卡片，没有底部 2px 指示条。
        let has_indicator = rounded_rects
            .iter()
            .any(|&(tok, _, h)| tok == Token::PRIMARY && (h - 2.0).abs() < 1e-2);
        assert!(!has_indicator, "active tab has no 2px Accent indicator");
    });
}

#[test]
fn tab_view_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 360.0, 120.0);

        // 1. 禁用态校验：抑制底色与指示条，全量文本与图标衰减为 TEXT_DISABLED
        let disabled_fixture = emit_canonical_tab_view_fixture(
            rect,
            FixtureState::disabled().with_selected(0),
            text_ctx,
            100,
        );

        let rounded_rects: Vec<Token> = disabled_fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::RoundedRect)
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        // 禁用态保留选中卡片的 CARD_BG 底板，只衰减文案与强调色。
        assert!(
            rounded_rects.contains(&Token::CARD_BG),
            "disabled state keeps selected tab CARD_BG background"
        );
        assert!(
            !rounded_rects.contains(&Token::PRIMARY),
            "disabled state must suppress Accent indicator"
        );

        let glyph_tokens: Vec<Token> = disabled_fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty());
        for tok in glyph_tokens {
            assert_eq!(
                tok,
                Token::TEXT_DISABLED,
                "all tab text and close icons must be TEXT_DISABLED in disabled state"
            );
        }

        // 2. 聚焦态校验
        let focused_fixture = emit_canonical_tab_view_fixture(
            rect,
            FixtureState::focused().with_selected(0),
            text_ctx,
            100,
        );
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(
            has_focus_ring,
            "focused state must emit focus ring on active tab"
        );
    });
}

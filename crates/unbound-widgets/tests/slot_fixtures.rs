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
    emit_canonical_flip_view_fixture, emit_canonical_grid_fixture,
    emit_canonical_list_rows_fixture, emit_canonical_path_fixture, emit_canonical_refresh_fixture,
    emit_canonical_segmented_fixture, emit_canonical_text_rows_fixture,
    emit_canonical_tree_fixture, FixtureState, CANONICAL_GRID_ITEMS, CANONICAL_LIST_ITEMS,
    STANDARD_FLIP_HEIGHT, STANDARD_FLIP_WIDTH, STANDARD_GRID_CELL_RADIUS, STANDARD_GRID_HEIGHT,
    STANDARD_GRID_WIDTH, STANDARD_LIST_ITEM_HEIGHT, STANDARD_PILL_HEIGHT, STANDARD_PILL_WIDTH,
    STANDARD_REFRESH_HEIGHT, STANDARD_REFRESH_INDICATOR_SIZE, STANDARD_REFRESH_WIDTH,
    STANDARD_SCROLL_BAR_HEIGHT, STANDARD_SCROLL_BAR_TRACK_WIDTH, STANDARD_SCROLL_BAR_WIDTH,
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

        // 3. 图元令牌校验：前序项为次要文本，当前项为主要文本
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
            glyph_tokens.contains(&Token::TEXT_SECONDARY),
            "ancestor items must use TEXT_SECONDARY"
        );
        assert!(
            !glyph_tokens.contains(&Token::TEXT_DISABLED),
            "normal state must not emit TEXT_DISABLED"
        );
    });
}

#[test]
fn path_fixture_disabled_recursively_downgrades_all_items_and_separators() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 32.0);
        let fixture = emit_canonical_path_fixture(rect, FixtureState::disabled(), text_ctx, 100);

        // 几何尺寸保持确定性
        assert_eq!(fixture.item_rects.len(), 3);
        assert_eq!(fixture.separator_rects.len(), 2);

        // 所有字形图元均必须递归降级为 TEXT_DISABLED
        let glyph_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();

        assert!(!glyph_tokens.is_empty(), "must emit glyph primitives");
        for &tok in &glyph_tokens {
            assert_eq!(
                tok,
                Token::TEXT_DISABLED,
                "all text and separators must be TEXT_DISABLED in disabled state"
            );
        }
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
        assert_eq!(fixture.root_rect.height(), STANDARD_TREE_ITEM_HEIGHT - 4.0);
        assert_eq!(
            fixture.child_rect.origin.y,
            fixture.root_rect.origin.y + STANDARD_TREE_ITEM_HEIGHT
        );
        assert_eq!(
            fixture.child_rect.origin.x - fixture.root_rect.origin.x,
            STANDARD_TREE_INDENT,
            "child row is indented by standard 16px"
        );

        // 2. 根节点左侧 Accent 胶囊条
        let has_pill = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - STANDARD_PILL_WIDTH).abs() < 1e-4
                && (p.size[1] - STANDARD_PILL_HEIGHT).abs() < 1e-4
        });
        assert!(has_pill, "expanded root must have 3x16px Accent pill");

        // 3. 展开箭头与垂直层级引导线
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
        assert!(has_guide_line, "must render 1px DIVIDER depth guide line");

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
fn tree_fixture_disabled_propagates_gray_and_suppresses_selection() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 220.0, 140.0);
        let fixture = emit_canonical_tree_fixture(rect, FixtureState::disabled(), text_ctx, 300);

        for p in &fixture.output.primitives {
            let prim_type = p.prim_type();
            let fill = Token::new(p.fill_token_or_uv as u16);

            // 抑制胶囊条与选中背景
            if prim_type == PrimType::RoundedRect {
                assert_ne!(fill, Token::PRIMARY, "disabled tree must not have pill");
                assert_ne!(fill, Token::HOVER, "disabled tree must not have hover bg");
                if (p.size[0] - 1.0).abs() < 1e-4 {
                    assert_eq!(
                        fill,
                        Token::CONTROL_DISABLED_BORDER,
                        "guide line must downgrade to CONTROL_DISABLED_BORDER"
                    );
                }
            }

            // 展开 chevron 指示箭头变灰
            if prim_type == PrimType::Curve {
                assert_eq!(
                    fill,
                    Token::TEXT_DISABLED,
                    "chevron stroke must be TEXT_DISABLED"
                );
            }

            // 文本全部变灰
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

        for p in &disabled_fixture.output.primitives {
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(Token::new(p.fill_token_or_uv as u16), Token::TEXT_DISABLED);
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

        let has_accent_badge = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                && (p.size[0] - 18.0).abs() < 1e-4
        });
        assert!(
            has_accent_badge,
            "selected cell must have PRIMARY checkmark badge"
        );

        // 3. 勾选折线
        let check_curves = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(check_curves, 2, "checkmark consists of 2 curve strokes");

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
            // 文本全部为 TEXT_DISABLED
            if matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(fill, Token::TEXT_DISABLED);
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

        // 3. 折线箭头（左 Prev 2 条折线 + 右 Next 2 条折线 = 4 条）
        let chevron_curves = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(
            chevron_curves, 4,
            "must emit 4 curve strokes for nav chevrons"
        );

        // 4. 下一页箭头活跃色为 TEXT_PRIMARY
        let curve_tokens: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(curve_tokens.contains(&Token::TEXT_PRIMARY));
        assert!(curve_tokens.contains(&Token::TEXT_DISABLED));

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

        // 2. 包含强调色圆弧（PRIMARY 曲线）
        let arc_curves: Vec<Token> = fixture
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!arc_curves.is_empty());
        for &tok in &arc_curves {
            assert_eq!(tok, Token::PRIMARY);
        }

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

        // 1. 轨道尺寸与居中
        assert_eq!(
            fixture.track_rect.width(),
            STANDARD_SCROLL_BAR_TRACK_WIDTH,
            "track width is standard 8px"
        );
        assert_eq!(fixture.track_rect.height(), STANDARD_SCROLL_BAR_HEIGHT);
        let margin_left = fixture.track_rect.origin.x - rect.origin.x;
        assert_eq!(margin_left, 4.0, "track is centered with 4px left margin");

        // 2. 滑块存在
        assert_eq!(fixture.thumb_rect.width(), STANDARD_SCROLL_BAR_TRACK_WIDTH);
        assert_eq!(fixture.thumb_rect.height(), 32.0);

        // 3. 图元令牌包含 CONTROL_TRACK_STROKE 滑块
        let has_thumb = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && Token::new(p.fill_token_or_uv as u16) == Token::CONTROL_TRACK_STROKE
        });
        assert!(has_thumb, "must emit CONTROL_TRACK_STROKE thumb");
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
                && Token::new(p.fill_token_or_uv as u16) == Token::CONTROL_DISABLED_THUMB
        });
        assert!(has_thumb, "disabled thumb must be CONTROL_DISABLED_THUMB");
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

//! 切片 2（导航与命令条类控件）基准控件几何与渲染契约代码级测试守护。
//!
//! 依据 ADR 0016 与 Spec #79，本测试套件覆盖切片 2 全部 8 个基准控件：
//! 1. `BreadcrumbBar`：路径插桩对齐 3 级路径 `Root > Folder > File.txt`，聚焦态焦点环与禁用态递归降级；
//! 2. `SelectorBar`：三段分段项 `Day`、`Week`、`Month`，选中胶囊滑块指示器与禁用态抑制；
//! 3. `PipsPager`：5 页指示圆点与选中页标记，中心间距 16px，禁用态抑制选中；
//! 4. `CommandBar`：包含 Save 与 Open 命令、分隔线与更多按钮，禁用态变灰；
//! 5. `MenuBar`：标准 File 与 Edit 菜单条目，聚焦态焦点环与禁用态降级；
//! 6. `TitleBar`：主标题 Document 与副标题 WinUiReference，三枚无边框控制按钮，焦点态与禁用态；
//! 7. `NavigationView`：顶部汉堡按钮、Home 与 Library 条目以及底部 Settings，展开态与选中条；
//! 8. `TabView`：Tab 1 与 Tab 2 双标签页、关闭按钮与 Content 内容区，选中底板与禁用态。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::Token;
use unbound_widgets::slot_fixture::{
    emit_canonical_command_bar_fixture, emit_canonical_menu_bar_fixture,
    emit_canonical_navigation_view_fixture, emit_canonical_path_fixture,
    emit_canonical_pips_pager_fixture, emit_canonical_segmented_fixture,
    emit_canonical_tab_view_fixture, emit_canonical_title_bar_fixture, FixtureState,
    CANONICAL_COMMAND_ITEMS, CANONICAL_MENU_ITEMS, CANONICAL_NAV_ITEMS, CANONICAL_PATH_ITEMS,
    CANONICAL_PIPS_COUNT, CANONICAL_PIPS_SELECTED_INDEX, CANONICAL_TAB_ITEMS, STANDARD_PILL_HEIGHT,
    STANDARD_PILL_WIDTH,
};
use unbound_widgets::TextCtx;

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
// 1. BreadcrumbBar 路径插桩与状态契约
// ---------------------------------------------------------------------------

#[test]
fn breadcrumb_bar_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 32.0);

        // 1. 正常态契约：3 级条目与 2 级分隔符
        let normal = emit_canonical_path_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.item_rects.len(), CANONICAL_PATH_ITEMS.len());
        assert_eq!(normal.separator_rects.len(), CANONICAL_PATH_ITEMS.len() - 1);
        assert_eq!(normal.total_size.y, 32.0);

        let glyph_tokens: Vec<Token> = normal
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
        assert!(glyph_tokens.contains(&Token::TEXT_SECONDARY));

        // 2. 聚焦态契约：必须发射焦点环
        let focused = emit_canonical_path_fixture(
            rect,
            FixtureState::focused().with_selected(2),
            text_ctx,
            100,
        );
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused BreadcrumbBar must emit focus ring");

        // 3. 禁用态契约：全部条目降级为 TEXT_DISABLED
        let disabled = emit_canonical_path_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let disabled_tokens: Vec<Token> = disabled
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!disabled_tokens.is_empty());
        for tok in disabled_tokens {
            assert_eq!(tok, Token::TEXT_DISABLED);
        }
    });
}

// ---------------------------------------------------------------------------
// 2. SelectorBar 分段项与滑块胶囊指示器
// ---------------------------------------------------------------------------

#[test]
fn selector_bar_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 40.0);

        // 1. 正常态契约：包含 Accent 胶囊指示条
        let normal = emit_canonical_segmented_fixture(
            rect,
            FixtureState::normal().with_selected(1),
            text_ctx,
            100,
        );
        let has_accent_pill = normal.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
                && (p.size[0] - STANDARD_PILL_WIDTH).abs() < 1e-2
                && (p.size[1] - STANDARD_PILL_HEIGHT).abs() < 1e-2
        });
        assert!(
            has_accent_pill,
            "normal SelectorBar must have selection pill"
        );

        // 2. 禁用态契约：抑制选中胶囊且文本降级
        let disabled = emit_canonical_segmented_fixture(
            rect,
            FixtureState::disabled().with_selected(1),
            text_ctx,
            100,
        );
        let has_disabled_pill = disabled.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
        });
        assert!(
            !has_disabled_pill,
            "disabled SelectorBar must suppress pill"
        );

        let glyph_tokens: Vec<Token> = disabled
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
// 3. PipsPager 分页指示点与标记
// ---------------------------------------------------------------------------

#[test]
fn pips_pager_normal_and_disabled_and_focused_states() {
    let rect = Rect::new(0.0, 0.0, 320.0, 32.0);

    // 1. 正常态契约：5 枚圆点与 1 枚选中高亮圆点
    let normal = emit_canonical_pips_pager_fixture(rect, FixtureState::normal(), 100);
    assert_eq!(normal.pip_rects.len(), CANONICAL_PIPS_COUNT);
    assert_eq!(normal.selected_index, CANONICAL_PIPS_SELECTED_INDEX);

    let circles: Vec<&unbound_gpu::data::GpuPrimitiveInstance> = normal
        .output
        .primitives
        .iter()
        .filter(|p| p.prim_type() == PrimType::RoundedRect)
        .collect();
    assert_eq!(circles.len(), CANONICAL_PIPS_COUNT);

    // 2. 聚焦态契约：选中圆点带焦点外圈
    let focused = emit_canonical_pips_pager_fixture(rect, FixtureState::focused(), 100);
    let has_focus_ring = focused
        .output
        .primitives
        .iter()
        .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
    assert!(has_focus_ring, "focused PipsPager must emit focus ring");

    // 3. 禁用态契约：圆点颜色降级且抑制高亮
    let disabled = emit_canonical_pips_pager_fixture(rect, FixtureState::disabled(), 100);
    for p in &disabled.output.primitives {
        if p.prim_type() == PrimType::RoundedRect {
            assert_eq!(
                p.fill_token_or_uv as u16,
                Token::CONTROL_DISABLED_THUMB.raw()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 4. CommandBar 命令条、分隔线与更多按钮
// ---------------------------------------------------------------------------

#[test]
fn command_bar_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 48.0);

        // 1. 正常态契约：两枚命令按钮、一枚垂直分隔线与一枚更多按钮
        let normal =
            emit_canonical_command_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.button_rects.len(), CANONICAL_COMMAND_ITEMS.len());
        assert_eq!(normal.total_size.y, 48.0);

        // 2. 聚焦态契约
        let focused =
            emit_canonical_command_bar_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused CommandBar must emit focus ring");

        // 3. 禁用态契约：命令文字与图标降级为 TEXT_DISABLED
        let disabled =
            emit_canonical_command_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let glyph_tokens: Vec<Token> = disabled
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!glyph_tokens.is_empty());
        for tok in glyph_tokens {
            assert_eq!(tok, Token::TEXT_DISABLED);
        }
    });
}

// ---------------------------------------------------------------------------
// 5. MenuBar 标准菜单栏
// ---------------------------------------------------------------------------

#[test]
fn menu_bar_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 320.0, 40.0);

        // 1. 正常态契约：File 与 Edit 两枚菜单项
        let normal = emit_canonical_menu_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.item_rects.len(), CANONICAL_MENU_ITEMS.len());
        assert_eq!(normal.total_size.y, 40.0);

        // 2. 聚焦态契约
        let focused = emit_canonical_menu_bar_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused MenuBar must emit focus ring");

        // 3. 禁用态契约
        let disabled =
            emit_canonical_menu_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
// 6. TitleBar 标题栏与窗口控制按钮
// ---------------------------------------------------------------------------

#[test]
fn title_bar_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 420.0, 32.0);

        // 1. 正常态契约：主标题、副标题与三枚控制按钮
        let normal = emit_canonical_title_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.caption_rects.len(), 3);
        assert_eq!(normal.total_size.y, 32.0);
        assert!(normal.title_rect.width() > 0.0);
        assert!(normal.subtitle_rect.width() > 0.0);

        // 2. 焦点态取自 control.focused 而非写死
        let normal_state = FixtureState::from_control_state(
            unbound_widgets::ControlState {
                focused: false,
                disabled: false,
                ..Default::default()
            },
            false,
        );
        assert!(
            !normal_state.focused,
            "TitleBar normal state must not be forced to focused"
        );

        let focused_state = FixtureState::from_control_state(
            unbound_widgets::ControlState {
                focused: true,
                disabled: false,
                ..Default::default()
            },
            false,
        );
        assert!(
            focused_state.focused,
            "TitleBar focused state must preserve focused flag"
        );

        // 3. 禁用态契约：标题与副标题变灰
        let disabled =
            emit_canonical_title_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let glyph_tokens: Vec<Token> = disabled
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!glyph_tokens.is_empty());
        for tok in glyph_tokens {
            assert_eq!(tok, Token::TEXT_DISABLED);
        }
    });
}

// ---------------------------------------------------------------------------
// 7. NavigationView 导航面板
// ---------------------------------------------------------------------------

#[test]
fn navigation_view_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 240.0, 360.0);

        // 1. 正常态契约：顶部汉堡按钮、菜单项与底部设置
        let normal =
            emit_canonical_navigation_view_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.item_rects.len(), CANONICAL_NAV_ITEMS.len());
        assert!(normal.settings_rect.width() > 0.0);

        // 首项具有 Accent 胶囊条
        let has_pill = normal.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
        });
        assert!(
            has_pill,
            "selected item in NavigationView must have primary accent pill"
        );

        // 2. 聚焦态契约
        let focused =
            emit_canonical_navigation_view_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(
            has_focus_ring,
            "focused NavigationView must emit focus ring"
        );

        // 3. 禁用态契约：抑制胶囊指示条且文本降级
        let disabled =
            emit_canonical_navigation_view_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_disabled_pill = disabled.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
        });
        assert!(
            !has_disabled_pill,
            "disabled NavigationView must suppress primary pill"
        );
    });
}

// ---------------------------------------------------------------------------
// 8. TabView 标签页视图
// ---------------------------------------------------------------------------

#[test]
fn tab_view_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 360.0, 120.0);

        // 1. 正常态契约：双标签页、关闭按钮与内容区
        let normal = emit_canonical_tab_view_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(normal.tab_rects.len(), CANONICAL_TAB_ITEMS.len());
        assert!(normal.content_rect.width() > 0.0);

        // 2. 聚焦态契约
        let focused = emit_canonical_tab_view_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused TabView must emit focus ring");

        // 3. 禁用态契约
        let disabled =
            emit_canonical_tab_view_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let glyph_tokens: Vec<Token> = disabled
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!glyph_tokens.is_empty());
        for tok in glyph_tokens {
            assert_eq!(tok, Token::TEXT_DISABLED);
        }
    });
}

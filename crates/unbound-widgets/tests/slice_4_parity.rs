//! 切片 4 浮层、日期与辅助类控件对齐单元与契约测试守护。
//!
//! 验证切片 4 所覆盖的 12 个基准控件与图像占位盒：
//! `Flyout`、`MenuFlyout`、`InfoBar`、`Expander`、`SplitView`、`CalendarView`、
//! `TimePicker`、`PersonPicture`、`FontIcon`、`AppBarButton`、`AppBarToggleButton`、
//! `Pivot`、`Image` 在正常态、禁用态、聚焦态与选中态下的几何尺寸、图元类型与颜色令牌衰减。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::Token;
use unbound_widgets::slot_fixture::{
    emit_canonical_app_bar_button_fixture, emit_canonical_app_bar_toggle_button_fixture,
    emit_canonical_calendar_view_fixture, emit_canonical_expander_fixture,
    emit_canonical_flyout_fixture, emit_canonical_font_icon_fixture, emit_canonical_image_fixture,
    emit_canonical_info_bar_fixture, emit_canonical_menu_flyout_fixture,
    emit_canonical_person_picture_fixture, emit_canonical_pivot_fixture,
    emit_canonical_split_view_fixture, emit_canonical_time_picker_fixture, FixtureState,
    CANONICAL_APP_BAR_BUTTON_HEIGHT, CANONICAL_APP_BAR_BUTTON_WIDTH,
    CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT, CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH,
    CANONICAL_CALENDAR_HEIGHT, CANONICAL_CALENDAR_WIDTH, CANONICAL_EXPANDER_HEADER_HEIGHT,
    CANONICAL_EXPANDER_WIDTH, CANONICAL_FLYOUT_HEIGHT, CANONICAL_FLYOUT_WIDTH,
    CANONICAL_FONT_ICON_SIZE, CANONICAL_IMAGE_SIZE, CANONICAL_INFOBAR_HEIGHT,
    CANONICAL_INFOBAR_WIDTH, CANONICAL_MENU_FLYOUT_WIDTH, CANONICAL_PERSON_PICTURE_SIZE,
    CANONICAL_PIVOT_HEIGHT, CANONICAL_PIVOT_WIDTH, CANONICAL_SPLIT_VIEW_HEIGHT,
    CANONICAL_SPLIT_VIEW_PANE_WIDTH, CANONICAL_SPLIT_VIEW_WIDTH, CANONICAL_TIME_COLUMNS,
    CANONICAL_TIME_PICKER_HEIGHT, CANONICAL_TIME_PICKER_WIDTH,
};
use unbound_widgets::TextCtx;

/// 构造供无头测试使用的测量上下文。
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
fn info_bar_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_INFOBAR_WIDTH, CANONICAL_INFOBAR_HEIGHT);

        // 1. 正常态校验
        let fixture = emit_canonical_info_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_INFOBAR_WIDTH);
        assert_eq!(fixture.total_size.y, CANONICAL_INFOBAR_HEIGHT);

        let has_accent_stripe = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
                && (p.size[0] - 4.0).abs() < 1e-2
        });
        assert!(
            has_accent_stripe,
            "normal InfoBar must have 4px primary accent stripe"
        );

        // 2. 聚焦态校验
        let focused_fixture =
            emit_canonical_info_bar_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused InfoBar must emit focus ring");

        // 3. 禁用态校验
        let disabled_fixture =
            emit_canonical_info_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "all InfoBar text and icons must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn expander_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_EXPANDER_WIDTH, 96.0);

        // 1. 正常态校验：展开高度包含头部 48px + 内容区 48px
        let fixture = emit_canonical_expander_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_EXPANDER_WIDTH);
        assert_eq!(fixture.total_size.y, 96.0);
        assert_eq!(
            fixture.header_rect.height(),
            CANONICAL_EXPANDER_HEADER_HEIGHT
        );
        assert_eq!(fixture.content_rect.height(), 48.0);

        // 2. 聚焦态校验
        let focused_fixture =
            emit_canonical_expander_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(
            has_focus_ring,
            "focused Expander must emit focus ring on header"
        );

        // 3. 禁用态校验
        let disabled_fixture =
            emit_canonical_expander_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "all Expander text and chevrons must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn split_view_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_SPLIT_VIEW_WIDTH,
            CANONICAL_SPLIT_VIEW_HEIGHT,
        );

        // 1. 正常态校验：120px 侧边窗格与 200px 内容区
        let fixture =
            emit_canonical_split_view_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_SPLIT_VIEW_WIDTH);
        assert_eq!(fixture.total_size.y, CANONICAL_SPLIT_VIEW_HEIGHT);
        assert_eq!(fixture.pane_rect.width(), CANONICAL_SPLIT_VIEW_PANE_WIDTH);
        assert_eq!(fixture.content_rect.width(), 200.0);

        // 2. 禁用态校验
        let disabled_fixture =
            emit_canonical_split_view_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "all SplitView pane and content text must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn calendar_view_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_CALENDAR_WIDTH,
            CANONICAL_CALENDAR_HEIGHT,
        );

        // 1. 正常态校验：包含标题、星期表头、31 日单元格与 15 日选中强调胶囊
        let fixture =
            emit_canonical_calendar_view_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_CALENDAR_WIDTH);
        assert_eq!(fixture.total_size.y, CANONICAL_CALENDAR_HEIGHT);

        let has_selected_pill = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
        });
        assert!(
            has_selected_pill,
            "normal CalendarView must render selected day pill"
        );

        // 2. 禁用态校验：抑制选中高亮胶囊，全量字形变灰
        let disabled_fixture =
            emit_canonical_calendar_view_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_disabled_pill = disabled_fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
        });
        assert!(
            !has_disabled_pill,
            "disabled CalendarView must suppress selected day pill"
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
                "all CalendarView text must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn time_picker_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_TIME_PICKER_WIDTH,
            CANONICAL_TIME_PICKER_HEIGHT,
        );

        // 1. 正常态校验：3 列时间分栏
        let fixture =
            emit_canonical_time_picker_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_TIME_PICKER_WIDTH);
        assert_eq!(fixture.total_size.y, CANONICAL_TIME_PICKER_HEIGHT);

        let glyph_count = fixture
            .output
            .primitives
            .iter()
            .filter(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .count();
        assert!(glyph_count >= CANONICAL_TIME_COLUMNS.len());

        // 2. 聚焦态校验
        let focused_fixture =
            emit_canonical_time_picker_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(has_focus_ring, "focused TimePicker must emit focus ring");

        // 3. 禁用态校验
        let disabled_fixture =
            emit_canonical_time_picker_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "all TimePicker text must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn person_picture_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_PERSON_PICTURE_SIZE,
            CANONICAL_PERSON_PICTURE_SIZE,
        );

        // 1. 正常态校验：48px 圆形头像
        let fixture =
            emit_canonical_person_picture_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_PERSON_PICTURE_SIZE);
        assert_eq!(fixture.total_size.y, CANONICAL_PERSON_PICTURE_SIZE);

        let has_circle = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && (p.radii()[0] - CANONICAL_PERSON_PICTURE_SIZE * 0.5).abs() < 1e-2
        });
        assert!(has_circle, "PersonPicture must emit circular background");

        // 2. 禁用态校验
        let disabled_fixture =
            emit_canonical_person_picture_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "PersonPicture initials must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn font_icon_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_FONT_ICON_SIZE, CANONICAL_FONT_ICON_SIZE);

        // 1. 正常态校验：32px 尺寸与字形图元
        let fixture = emit_canonical_font_icon_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_FONT_ICON_SIZE);
        assert_eq!(fixture.total_size.y, CANONICAL_FONT_ICON_SIZE);

        let has_glyph = fixture
            .output
            .primitives
            .iter()
            .any(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor));
        assert!(has_glyph, "FontIcon must emit glyph primitive");

        // 2. 禁用态校验
        let disabled_fixture =
            emit_canonical_font_icon_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "FontIcon glyph must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn pivot_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_PIVOT_WIDTH, CANONICAL_PIVOT_HEIGHT);

        // 1. 正常态校验：页签条、下划指示条与内容区
        let fixture = emit_canonical_pivot_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_PIVOT_WIDTH);
        assert_eq!(fixture.total_size.y, CANONICAL_PIVOT_HEIGHT);

        let has_underline = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
                && (p.size[1] - 3.0).abs() < 1e-2
        });
        assert!(
            has_underline,
            "normal Pivot must render 3px primary indicator underline"
        );

        // 2. 聚焦态校验
        let focused_fixture =
            emit_canonical_pivot_fixture(rect, FixtureState::focused(), text_ctx, 100);
        let has_focus_ring = focused_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::FOCUS_RING.raw());
        assert!(
            has_focus_ring,
            "focused Pivot must emit focus ring on active item"
        );

        // 3. 禁用态校验
        let disabled_fixture =
            emit_canonical_pivot_fixture(rect, FixtureState::disabled(), text_ctx, 100);
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
                "all Pivot text must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn image_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_IMAGE_SIZE.x, CANONICAL_IMAGE_SIZE.y);

        // 1. 正常态校验：64×64 占位盒
        let fixture = emit_canonical_image_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_IMAGE_SIZE.x);
        assert_eq!(fixture.total_size.y, CANONICAL_IMAGE_SIZE.y);

        // 2. 禁用态校验
        let disabled_fixture =
            emit_canonical_image_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_disabled_border = disabled_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == Token::CONTROL_DISABLED_BORDER.raw());
        assert!(
            has_disabled_border,
            "disabled Image must use CONTROL_DISABLED_BORDER"
        );
    });
}

#[test]
fn flyout_and_menu_flyout_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let flyout_rect = Rect::new(0.0, 0.0, CANONICAL_FLYOUT_WIDTH, CANONICAL_FLYOUT_HEIGHT);

        // 1. Flyout 校验
        let flyout =
            emit_canonical_flyout_fixture(flyout_rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(flyout.total_size.x, CANONICAL_FLYOUT_WIDTH);
        assert_eq!(flyout.total_size.y, CANONICAL_FLYOUT_HEIGHT);

        let has_shadow = flyout
            .output
            .primitives
            .iter()
            .any(|p| p.prim_type() == PrimType::Shadow);
        assert!(has_shadow, "Flyout must emit shadow primitive");

        // 2. MenuFlyout 校验
        let menu_rect = Rect::new(0.0, 0.0, CANONICAL_MENU_FLYOUT_WIDTH, 80.0);
        let menu_flyout =
            emit_canonical_menu_flyout_fixture(menu_rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(menu_flyout.total_size.x, CANONICAL_MENU_FLYOUT_WIDTH);
        assert_eq!(menu_flyout.total_size.y, 80.0);

        let menu_disabled =
            emit_canonical_menu_flyout_fixture(menu_rect, FixtureState::disabled(), text_ctx, 100);
        let glyph_tokens: Vec<Token> = menu_disabled
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
                "all MenuFlyout items must be TEXT_DISABLED in disabled state"
            );
        }
    });
}

#[test]
fn app_bar_button_and_toggle_button_states() {
    with_test_ctx(|text_ctx| {
        let btn_rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_APP_BAR_BUTTON_WIDTH,
            CANONICAL_APP_BAR_BUTTON_HEIGHT,
        );

        // 1. AppBarButton 校验
        let btn =
            emit_canonical_app_bar_button_fixture(btn_rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(btn.total_size.x, CANONICAL_APP_BAR_BUTTON_WIDTH);
        assert_eq!(btn.total_size.y, CANONICAL_APP_BAR_BUTTON_HEIGHT);

        // 2. AppBarToggleButton 校验（选中态包含 2px 底部指示条）
        let toggle_rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH,
            CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT,
        );
        let checked_toggle = emit_canonical_app_bar_toggle_button_fixture(
            toggle_rect,
            FixtureState::checked(),
            text_ctx,
            100,
        );
        let has_indicator = checked_toggle.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
                && (p.size[1] - 2.0).abs() < 1e-2
        });
        assert!(
            has_indicator,
            "checked AppBarToggleButton must have 2px primary indicator"
        );

        // 3. 禁用态校验：抑制指示条并变灰
        let disabled_toggle = emit_canonical_app_bar_toggle_button_fixture(
            toggle_rect,
            FixtureState::disabled(),
            text_ctx,
            100,
        );
        let has_disabled_indicator = disabled_toggle.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
        });
        assert!(
            !has_disabled_indicator,
            "disabled AppBarToggleButton must suppress indicator"
        );
    });
}

#[test]
fn slice_4_fixtures_disabled_state_attenuates_opacity() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_INFOBAR_WIDTH, CANONICAL_INFOBAR_HEIGHT);
        let disabled_fixture =
            emit_canonical_info_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        assert!(!disabled_fixture.output.primitives.is_empty());
        for p in &disabled_fixture.output.primitives {
            assert!(
                (p.opacity() - unbound_widgets::slot_fixture::WINUI_DISABLED_OPACITY).abs() < 0.05,
                "InfoBar primitive opacity {} must attenuate by WINUI_DISABLED_OPACITY",
                p.opacity()
            );
        }

        let cal_rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_CALENDAR_WIDTH,
            CANONICAL_CALENDAR_HEIGHT,
        );
        let disabled_cal =
            emit_canonical_calendar_view_fixture(cal_rect, FixtureState::disabled(), text_ctx, 100);
        assert!(!disabled_cal.output.primitives.is_empty());
        for p in &disabled_cal.output.primitives {
            assert!(
                (p.opacity() - unbound_widgets::slot_fixture::WINUI_DISABLED_OPACITY).abs() < 0.05,
                "CalendarView primitive opacity {} must attenuate by WINUI_DISABLED_OPACITY",
                p.opacity()
            );
        }
    });
}

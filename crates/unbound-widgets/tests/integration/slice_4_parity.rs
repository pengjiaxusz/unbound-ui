//! 切片 4 浮层、日期与辅助类控件对齐单元与契约测试守护。
//!
//! 验证切片 4 所覆盖的 12 个基准控件与图像占位盒：
//! `Flyout`、`MenuFlyout`、`InfoBar`、`Expander`、`SplitView`、`CalendarView`、
//! `TimePicker`、`PersonPicture`、`FontIcon`、`AppBarButton`、`AppBarToggleButton`、
//! `Pivot`、`Image` 在正常态、禁用态、聚焦态与选中态下的几何尺寸、图元类型与颜色令牌衰减。

use unbound_gpu::data::{PrimType, SubElement, CLIP_NONE};
use unbound_math::Rect;
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::Token;
use unbound_widgets::slot_fixture::{
    emit_canonical_app_bar_button_fixture, emit_canonical_app_bar_toggle_button_fixture,
    emit_canonical_expander_fixture, emit_canonical_flyout_fixture,
    emit_canonical_font_icon_fixture, emit_canonical_image_fixture,
    emit_canonical_info_bar_fixture, emit_canonical_menu_flyout_fixture,
    emit_canonical_person_picture_fixture, emit_canonical_pivot_fixture,
    emit_canonical_split_view_fixture, emit_canonical_time_picker_fixture, FixtureState,
    CANONICAL_APP_BAR_BUTTON_HEIGHT, CANONICAL_APP_BAR_BUTTON_WIDTH,
    CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT, CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH,
    CANONICAL_EXPANDER_HEADER_HEIGHT, CANONICAL_EXPANDER_WIDTH, CANONICAL_FLYOUT_HEIGHT,
    CANONICAL_FLYOUT_WIDTH, CANONICAL_FONT_ICON_SIZE, CANONICAL_IMAGE_SIZE,
    CANONICAL_INFOBAR_HEIGHT, CANONICAL_INFOBAR_WIDTH, CANONICAL_MENU_FLYOUT_WIDTH,
    CANONICAL_PERSON_PICTURE_SIZE, CANONICAL_PIVOT_HEIGHT, CANONICAL_PIVOT_WIDTH,
    CANONICAL_SPLIT_VIEW_HEIGHT, CANONICAL_SPLIT_VIEW_PANE_WIDTH, CANONICAL_SPLIT_VIEW_WIDTH,
    CANONICAL_TIME_COLUMNS, CANONICAL_TIME_PICKER_HEIGHT, CANONICAL_TIME_PICKER_WIDTH,
};
use unbound_widgets::{
    emit_calendar_view, CalendarDisplayMode, CalendarSelectionMode, CalendarState, ControlState,
    DatePickerDesc, NaiveDate, TextCtx,
};

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

        let has_info_badge = fixture.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
                && (p.size[0] - 15.0).abs() < 1e-2
                && (p.size[1] - 15.0).abs() < 1e-2
        });
        assert!(
            has_info_badge,
            "normal InfoBar must have 15px primary informational badge circle"
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
        // WinUI 3 InfoBar 禁用态仅衰减关闭按钮，标题/正文与徽章字符保持主色与反色。
        assert!(
            glyph_tokens.contains(&Token::TEXT_PRIMARY),
            "disabled InfoBar title/message stays TEXT_PRIMARY per WinUI 3"
        );
        for tok in glyph_tokens {
            assert!(
                tok == Token::TEXT_DISABLED
                    || tok == Token::TEXT_PRIMARY
                    || tok == Token::ON_PRIMARY,
                "disabled InfoBar glyphs must be TEXT_PRIMARY/ON_PRIMARY/TEXT_DISABLED, got {:?}",
                tok
            );
        }
    });
}

#[test]
fn expander_normal_and_disabled_and_focused_states() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_EXPANDER_WIDTH, 100.0);

        // 1. 正常态校验：展开高度包含头部 48px + 内容区 51px（总高 99px，兼顾两主题参考底边容差）
        let fixture = emit_canonical_expander_fixture(rect, FixtureState::normal(), text_ctx, 100);
        assert_eq!(fixture.total_size.x, CANONICAL_EXPANDER_WIDTH);
        assert_eq!(fixture.total_size.y, 99.0);
        assert_eq!(
            fixture.header_rect.height(),
            CANONICAL_EXPANDER_HEADER_HEIGHT
        );
        assert_eq!(fixture.content_rect.height(), 51.0);

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
        // WinUI 3 Expander 禁用态衰减头部文字与箭头，内容区正文保持 TEXT_PRIMARY。
        assert!(glyph_tokens.contains(&Token::TEXT_DISABLED));
        assert!(glyph_tokens.contains(&Token::TEXT_PRIMARY));
        for tok in glyph_tokens {
            assert!(
                tok == Token::TEXT_DISABLED || tok == Token::TEXT_PRIMARY,
                "disabled Expander glyphs must be TEXT_DISABLED or TEXT_PRIMARY, got {:?}",
                tok
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
        let rect = Rect::new(0.0, 0.0, 300.0, 260.0);
        let fixed_date = NaiveDate::new(2026, 1, 15).expect("valid parity calendar date");
        let desc = DatePickerDesc::new(rect).with_z(100);

        let make_base_state = |highlight_today: bool| {
            let mut state = CalendarState::new(fixed_date, CalendarSelectionMode::Single);
            state.set_first_day_of_week(0);
            state.is_today_highlighted = highlight_today;
            state.select(fixed_date);
            state
        };

        // 1. normal 态校验：真实 emit_calendar_view 渲染选中空心强调环与 ADR-0042 局部裁剪节点
        let normal_state = make_base_state(false);
        let normal_out =
            emit_calendar_view(1, &desc, &normal_state, ControlState::default(), text_ctx);
        assert!(
            !normal_out.clips.is_empty(),
            "CalendarView must register an ADR-0042 local clip node"
        );
        assert!(
            normal_out.primitives.iter().any(|p| p.clip_idx == 0),
            "CalendarView internal primitives must bind to local clip node 0"
        );
        let normal_ring_count = normal_out
            .primitives
            .iter()
            .filter(|p| {
                p.prim_type() == PrimType::RoundedRect
                    && Token::new(p.fill_token_or_uv as u16) == Token::NONE
                    && Token::new(p.border_token_or_tex as u16) == Token::PRIMARY
            })
            .count();
        assert_eq!(
            normal_ring_count, 1,
            "normal CalendarView must render exactly 1 hollow primary ring for selected day"
        );

        // 2. disabled 态校验：抑制强调色填充与描边，全量字形变灰
        let disabled_ctrl = ControlState {
            disabled: true,
            ..ControlState::default()
        };
        let disabled_desc = DatePickerDesc::new(rect).with_z(100).disabled(true);
        let disabled_out =
            emit_calendar_view(1, &disabled_desc, &normal_state, disabled_ctrl, text_ctx);
        assert!(
            !disabled_out.primitives.iter().any(|p| {
                p.prim_type() == PrimType::RoundedRect
                    && (Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
                        || Token::new(p.border_token_or_tex as u16) == Token::PRIMARY)
            }),
            "disabled CalendarView must suppress primary fill and ring"
        );
        let glyph_tokens: Vec<Token> = disabled_out
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

        // 3. focused 态校验：外框焦点环可见（opacity == 1.0）
        let focused_ctrl = ControlState {
            focused: true,
            ..ControlState::default()
        };
        let focused_out = emit_calendar_view(1, &desc, &normal_state, focused_ctrl, text_ctx);
        assert!(
            focused_out
                .primitives
                .iter()
                .any(|p| p.flags.sub_element() == SubElement::FocusRing as u8 && p.opacity() > 0.9),
            "focused CalendarView must show visible focus ring"
        );

        // 4. today_highlighted 态校验：固定 today 下渲染实心强调圆与 ON_PRIMARY 字形
        let today_state = make_base_state(true);
        let today_out =
            emit_calendar_view(1, &desc, &today_state, ControlState::default(), text_ctx);
        assert!(
            today_out.primitives.iter().any(|p| {
                p.prim_type() == PrimType::RoundedRect
                    && Token::new(p.fill_token_or_uv as u16) == Token::PRIMARY
            }),
            "today_highlighted CalendarView must render solid primary circle for today"
        );
        assert!(
            today_out.primitives.iter().any(|p| {
                matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor)
                    && Token::new(p.fill_token_or_uv as u16) == Token::ON_PRIMARY
            }),
            "today_highlighted CalendarView must render ON_PRIMARY glyphs on today pill"
        );

        // 5. multiple_selected 态校验：同时选中 3 个确定性日期并渲染 3 个空心强调环
        let mut multi_state = CalendarState::new(fixed_date, CalendarSelectionMode::Multiple);
        multi_state.is_today_highlighted = false;
        for day in [10, 15, 20] {
            multi_state.select(NaiveDate::new(2026, 1, day).expect("valid multi-select date"));
        }
        let multi_out =
            emit_calendar_view(1, &desc, &multi_state, ControlState::default(), text_ctx);
        let multi_rings = multi_out
            .primitives
            .iter()
            .filter(|p| {
                p.prim_type() == PrimType::RoundedRect
                    && Token::new(p.fill_token_or_uv as u16) == Token::NONE
                    && Token::new(p.border_token_or_tex as u16) == Token::PRIMARY
            })
            .count();
        assert_eq!(
            multi_rings, 3,
            "multiple_selected CalendarView must render 3 hollow primary rings"
        );

        // 6. blackout 态校验：禁选日期（18、19）额外绘制 2 条斜线禁用曲线图元
        let mut blackout_state = make_base_state(false);
        blackout_state.blackout_dates = vec![
            NaiveDate::new(2026, 1, 18).expect("valid blackout date"),
            NaiveDate::new(2026, 1, 19).expect("valid blackout date"),
        ];
        let blackout_out =
            emit_calendar_view(1, &desc, &blackout_state, ControlState::default(), text_ctx);
        let normal_curves = normal_out
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        let blackout_curves = blackout_out
            .primitives
            .iter()
            .filter(|p| p.prim_type() == PrimType::Curve)
            .count();
        assert_eq!(
            blackout_curves,
            normal_curves + 2,
            "blackout CalendarView must emit 2 diagonal blackout curves for 2 blackout dates"
        );

        // 7. year_mode 态校验：展示 2026 年 12 个月（3 头部命中区 + 12 月格命中区 = 15）
        let mut year_state = make_base_state(false);
        year_state.display_mode = CalendarDisplayMode::Year;
        assert_eq!(year_state.header_title(), "2026");
        let year_out = emit_calendar_view(1, &desc, &year_state, ControlState::default(), text_ctx);
        assert!(!year_out.clips.is_empty());
        assert_eq!(
            year_out.sub_hits.len(),
            15,
            "year_mode CalendarView must expose 3 header + 12 month sub_hits"
        );

        // 8. decade_mode 态校验：展示 2020 - 2029 十年视图（3 头部命中区 + 12 年格命中区 = 15）
        let mut decade_state = make_base_state(false);
        decade_state.display_mode = CalendarDisplayMode::Decade;
        assert_eq!(decade_state.header_title(), "2020 - 2029");
        let decade_out =
            emit_calendar_view(1, &desc, &decade_state, ControlState::default(), text_ctx);
        assert!(!decade_out.clips.is_empty());
        assert_eq!(
            decade_out.sub_hits.len(),
            15,
            "decade_mode CalendarView must expose 3 header + 12 year sub_hits"
        );

        // 9. 紧凑高度零底部越界校验（300×200）
        let compact_rect = Rect::new(0.0, 0.0, 300.0, 200.0);
        let compact_desc = DatePickerDesc::new(compact_rect).with_z(100);
        assert!(
            compact_desc.cell_rect(41).max().y <= compact_rect.max().y,
            "last calendar cell bottom {} must stay within compact container {}",
            compact_desc.cell_rect(41).max().y,
            compact_rect.max().y
        );
        let compact_out = emit_calendar_view(
            1,
            &compact_desc,
            &normal_state,
            ControlState::default(),
            text_ctx,
        );
        assert!(
            !compact_out.clips.is_empty(),
            "compact CalendarView must attach local clip node"
        );
        for prim in compact_out
            .primitives
            .iter()
            .filter(|p| p.clip_idx != CLIP_NONE)
        {
            let bottom = prim.local_pos[1] + prim.size[1];
            assert!(
                bottom <= compact_rect.max().y + 0.5,
                "compact CalendarView primitive bottom {bottom} overflowed container {}",
                compact_rect.max().y
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
                Token::TEXT_PRIMARY,
                "WinUI 3 PersonPicture keeps initials bright (TEXT_PRIMARY) in disabled state"
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

        assert!(
            !fixture.output.primitives.is_empty(),
            "FontIcon must emit vector icon primitives"
        );

        // 2. 禁用态校验
        let disabled_fixture =
            emit_canonical_font_icon_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let icon_tokens: Vec<Token> = disabled_fixture
            .output
            .primitives
            .iter()
            .map(|p| Token::new(p.fill_token_or_uv as u16))
            .collect();
        assert!(!icon_tokens.is_empty());
        assert!(
            icon_tokens.contains(&Token::TEXT_DISABLED),
            "FontIcon must use TEXT_DISABLED in disabled state"
        );
    });
}

#[test]
fn font_icon_falls_back_to_all_apps_when_symbol_font_disabled() {
    with_test_ctx(|text_ctx| {
        text_ctx
            .engine
            .set_symbol_font_policy(unbound_text::SymbolFontPolicy::Disabled);
        assert!(!text_ctx.has_symbol_font());

        let rect = Rect::new(0.0, 0.0, CANONICAL_FONT_ICON_SIZE, CANONICAL_FONT_ICON_SIZE);
        let normal = emit_canonical_font_icon_fixture(rect, FixtureState::normal(), text_ctx, 100);
        let disabled =
            emit_canonical_font_icon_fixture(rect, FixtureState::disabled(), text_ctx, 100);

        let mut expected_prims = Vec::new();
        let mut expected_payloads = Vec::new();
        unbound_widgets::shell::emit_shell_icon_primitives(
            unbound_widgets::shell::ShellIcon::AllApps,
            rect,
            Token::TEXT_PRIMARY,
            Token::SURFACE,
            101,
            &mut expected_prims,
            &mut expected_payloads,
        );

        assert_eq!(normal.output.primitives.len(), expected_prims.len());
        assert_eq!(disabled.output.primitives.len(), expected_prims.len());
        assert!(
            normal
                .output
                .primitives
                .iter()
                .all(|p| p.prim_type() != PrimType::Glyph),
            "disabled symbol font must fall back to ShellIcon::AllApps vector geometry"
        );
        assert!(normal
            .output
            .primitives
            .iter()
            .all(|p| Token::new(p.fill_token_or_uv as u16) == Token::TEXT_PRIMARY));
        assert!(disabled
            .output
            .primitives
            .iter()
            .all(|p| Token::new(p.fill_token_or_uv as u16) == Token::TEXT_DISABLED));
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

        // 2. 禁用态校验：使用 sRGB 40% 预混的确定性图块色槽 333/334，不再二次衰减透明度
        let disabled_fixture =
            emit_canonical_image_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        let has_deterministic_fill = disabled_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.fill_token_or_uv as u16 == 333);
        let has_deterministic_border = disabled_fixture
            .output
            .primitives
            .iter()
            .any(|p| p.border_token_or_tex as u16 == 334);
        assert!(
            has_deterministic_fill && has_deterministic_border,
            "disabled Image must keep sRGB premixed fill/border tokens 333/334"
        );
        for p in &disabled_fixture.output.primitives {
            assert!(
                (p.opacity() - 1.0).abs() < 0.05,
                "disabled Image opacity {} must stay opaque via premixed tokens",
                p.opacity()
            );
        }
    });
}

#[test]
fn flyout_and_menu_flyout_normal_and_disabled_states() {
    with_test_ctx(|text_ctx| {
        let flyout_rect = Rect::new(0.0, 0.0, CANONICAL_FLYOUT_WIDTH, CANONICAL_FLYOUT_HEIGHT);

        // 1. Flyout 校验
        let flyout = emit_canonical_flyout_fixture(
            flyout_rect,
            FixtureState::normal(),
            false,
            text_ctx,
            100,
        );
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
        let menu_flyout = emit_canonical_menu_flyout_fixture(
            menu_rect,
            FixtureState::normal(),
            false,
            text_ctx,
            100,
        );
        assert_eq!(menu_flyout.total_size.x, CANONICAL_MENU_FLYOUT_WIDTH);
        assert_eq!(menu_flyout.total_size.y, 80.0);

        let menu_disabled = emit_canonical_menu_flyout_fixture(
            menu_rect,
            FixtureState::disabled(),
            false,
            text_ctx,
            100,
        );
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
        // WinUI 选中态是整块强调色实底（左右内缩 2px、上下内缩 6px），不是底部指示条。
        let has_checked_surface = checked_toggle.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == Token::PRIMARY.raw()
                && (p.size[1] - (CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT - 12.0)).abs() < 1e-2
        });
        assert!(
            has_checked_surface,
            "checked AppBarToggleButton must have solid PRIMARY surface"
        );

        // 3. 禁用态校验：抑制实底并变灰
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
fn slice_4_fixtures_disabled_state_keeps_full_opacity() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, CANONICAL_INFOBAR_WIDTH, CANONICAL_INFOBAR_HEIGHT);
        let disabled_fixture =
            emit_canonical_info_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);
        assert!(!disabled_fixture.output.primitives.is_empty());
        // 禁用态通过 TEXT_DISABLED 令牌表达，不再二次衰减透明度。
        for p in &disabled_fixture.output.primitives {
            assert!(
                (p.opacity() - 1.0).abs() < 0.05,
                "InfoBar primitive opacity {} must stay fully opaque",
                p.opacity()
            );
        }

        let cal_rect = Rect::new(0.0, 0.0, 300.0, 260.0);
        let cal_desc = DatePickerDesc::new(cal_rect).with_z(100).disabled(true);
        let fixed_date = NaiveDate::new(2026, 1, 15).expect("valid parity calendar date");
        let mut cal_state = CalendarState::new(fixed_date, CalendarSelectionMode::Single);
        cal_state.is_today_highlighted = false;
        cal_state.select(fixed_date);
        let disabled_cal = emit_calendar_view(
            1,
            &cal_desc,
            &cal_state,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            text_ctx,
        );
        assert!(!disabled_cal.primitives.is_empty());
        // CalendarView 禁用态已通过语义令牌衰减文字与箭头，可见图元（排除隐藏焦点环）不二次衰减透明度。
        for p in disabled_cal.primitives.iter().filter(|p| p.opacity() > 0.0) {
            assert!(
                (p.opacity() - 1.0).abs() < 0.05,
                "CalendarView visible primitive opacity {} must stay opaque when disabled via semantic tokens",
                p.opacity()
            );
        }
    });
}

#[test]
fn parity_20261120_win11_card_tokens_and_two_col_time_picker_regression() {
    assert_eq!(
        CANONICAL_TIME_COLUMNS,
        ["13", "30"],
        "24HourClock TimePicker must render 2 columns without seconds"
    );

    let light = unbound_theme::win11::light();
    let dark = unbound_theme::win11::dark();

    // 验证浅色与深色卡片描边与深色强调字色已对齐 WinUI 3 实测色值
    let dark_on_primary = dark.color(Token::ON_PRIMARY);
    assert_eq!(dark_on_primary.r, 0.0);
    assert_eq!(dark_on_primary.g, 0.0);
    assert_eq!(dark_on_primary.b, 0.0);
    assert_eq!(dark_on_primary.a, 1.0);

    let light_card_border = light.color(Token::CARD_BORDER);
    let dark_card_border = dark.color(Token::CARD_BORDER);
    assert!(light_card_border.a > 0.0);
    assert!(dark_card_border.a > 0.0);
}

#[test]
fn parity_20261121_visual_diagnosis_batch_regressions() {
    with_test_ctx(|text_ctx| {
        // 1. AppBarSeparator: 40px 槽位内上下各内缩 8px -> 24px 竖线
        let sep = unbound_widgets::AppBarSeparatorDesc::vertical(Rect::new(16.0, 16.0, 5.0, 40.0));
        let sep_line = sep.line_rect();
        assert_eq!(sep_line.origin.y, 24.0);
        assert_eq!(sep_line.height(), 24.0);

        // 2. RichEditBox: 水平内边距对齐 WinUI 实测 11px
        assert_eq!(unbound_widgets::RICH_EDIT_PADDING_X, 11.0);

        // 3. AnnotatedScrollBar: 44px 槽位内右对齐（滑块右沿紧贴槽位右沿）
        let asb = unbound_widgets::slot_fixture::emit_canonical_annotated_scroll_bar_fixture(
            Rect::new(16.0, 16.0, 44.0, 160.0),
            FixtureState::normal(),
            text_ctx,
            100,
        );
        assert!((asb.thumb_rect.max().x - 60.0).abs() < 1e-2);

        // 4. TabView: 选中标签页从 origin.y + 8.0 起排布
        let tabs = unbound_widgets::slot_fixture::emit_canonical_tab_view_fixture(
            Rect::new(16.0, 0.0, 360.0, 120.0),
            FixtureState::normal(),
            text_ctx,
            100,
        );
        assert_eq!(tabs.tab_rects[0].origin.y, 8.0);

        // 5. ListBox: 输出铺满容器矩形的底板（Token 331），而 ListView 不输出全尺寸底板
        let lb = unbound_widgets::slot_fixture::emit_canonical_listbox_rows_fixture(
            Rect::new(16.0, 16.0, 220.0, 120.0),
            FixtureState::disabled(),
            text_ctx,
            100,
        );
        assert!(lb.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && p.fill_token_or_uv as u16 == 331
                && (p.size[0] - 220.0).abs() < 1e-2
                && (p.size[1] - 120.0).abs() < 1e-2
        }));
        let lv = unbound_widgets::slot_fixture::emit_canonical_list_rows_fixture(
            Rect::new(16.0, 16.0, 220.0, 120.0),
            FixtureState::normal(),
            text_ctx,
            100,
        );
        assert!(!lv.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && (p.size[0] - 220.0).abs() < 1e-2
                && (p.size[1] - 120.0).abs() < 1e-2
        }));

        // 6. TimePicker: 24 小时制非对称分栏线位于 60.5%（200px 宽时在 x = origin.x + 121）
        let tp = emit_canonical_time_picker_fixture(
            Rect::new(16.0, 16.0, 200.0, 30.0),
            FixtureState::normal(),
            text_ctx,
            100,
        );
        assert!(tp.output.primitives.iter().any(|p| {
            p.prim_type() == PrimType::RoundedRect
                && (p.size[0] - 1.0).abs() < 1e-2
                && (p.local_pos[0] - 137.0).abs() < 1e-2
        }));

        // 7. Image: 正常态使用 328/329，禁用态使用 sRGB 预混 333/334，且直角无圆角
        let img = emit_canonical_image_fixture(
            Rect::new(16.0, 16.0, 64.0, 64.0),
            FixtureState::normal(),
            text_ctx,
            100,
        );
        assert!(img
            .output
            .primitives
            .iter()
            .any(|p| { p.fill_token_or_uv as u16 == 328 && p.radii()[0] == 0.0 }));
        let img_disabled = emit_canonical_image_fixture(
            Rect::new(16.0, 16.0, 64.0, 64.0),
            FixtureState::disabled(),
            text_ctx,
            100,
        );
        assert!(img_disabled
            .output
            .primitives
            .iter()
            .any(|p| { p.fill_token_or_uv as u16 == 333 && (p.opacity() - 1.0).abs() < 1e-4 }));
    });
}

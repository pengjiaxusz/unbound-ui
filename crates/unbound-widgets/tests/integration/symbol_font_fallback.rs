//! 命令栏、应用栏、日历选择器与评分控件的符号字体回退与 24 态契约测试。
//!
//! 通过 [`unbound_text::SymbolFontPolicy::Disabled`] 在任意无头宿主上强制走矢量回退路径，
//! 验证上述控件在无符号字体环境下零 panic、回退到校准后的 [`ShellIcon`] 轮廓，
//! 并且保持跨 24 种 [`ControlState`] 的图元数量恒定不变量。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::{GlyphCache, SymbolFontPolicy, TextEngine};
use unbound_theme::Token;
use unbound_widgets::shell::{emit_shell_icon_primitives, ShellIcon};
use unbound_widgets::slot_fixture::{
    emit_canonical_app_bar_button_fixture, emit_canonical_app_bar_toggle_button_fixture,
    emit_canonical_command_bar_fixture, FixtureState, CANONICAL_APP_BAR_BUTTON_HEIGHT,
    CANONICAL_APP_BAR_BUTTON_WIDTH, CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT,
    CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH,
};
use unbound_widgets::{
    emit_rating, emit_rating_with_text, CalendarDatePickerDesc, CalendarDatePickerState,
    ControlState, NaiveDate, PointerPhase, RatingDesc, TextCtx, UiWidget, WidgetOutput,
    CALENDAR_DATE_PICKER_HEIGHT,
};

/// 构造无头测试用的文本上下文。
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

/// 生成 24 种交互态组合（3 指针阶段 × 2 聚焦 × 2 禁用 × 2 选中）。
fn all_control_states() -> Vec<ControlState> {
    let mut states = Vec::with_capacity(24);
    for phase in [
        PointerPhase::Idle,
        PointerPhase::Hovered,
        PointerPhase::Pressed,
    ] {
        for focused in [false, true] {
            for disabled in [false, true] {
                for selected in [false, true] {
                    states.push(ControlState {
                        phase,
                        focused,
                        disabled,
                        selected,
                    });
                }
            }
        }
    }
    states
}

/// 校验所有引用载荷的图元（字形、图像、曲线）索引均未越界。
fn assert_payload_indices_in_bounds(out: &WidgetOutput) {
    for prim in &out.primitives {
        if matches!(
            prim.prim_type(),
            PrimType::Glyph | PrimType::GlyphColor | PrimType::Image | PrimType::Curve
        ) {
            assert!(
                (prim.extra_payload_idx as usize) < out.payloads.len(),
                "primitive {:?} payload index {} out of bounds (len {})",
                prim.prim_type(),
                prim.extra_payload_idx,
                out.payloads.len()
            );
        }
    }
}

/// 统计单枚 [`ShellIcon`] 发出的图元数量。
fn shell_icon_prim_count(icon: ShellIcon, rect: Rect) -> usize {
    let mut prims = Vec::new();
    let mut payloads = Vec::new();
    emit_shell_icon_primitives(
        icon,
        rect,
        Token::TEXT_PRIMARY,
        Token::SURFACE,
        100,
        &mut prims,
        &mut payloads,
    );
    prims.len()
}

#[test]
fn command_bar_falls_back_to_save_folder_open_ellipsis_when_symbol_font_disabled() {
    with_test_ctx(|text_ctx| {
        text_ctx
            .engine
            .set_symbol_font_policy(SymbolFontPolicy::Disabled);
        assert!(!text_ctx.has_symbol_font());

        let rect = Rect::new(0.0, 0.0, 320.0, 48.0);
        let normal =
            emit_canonical_command_bar_fixture(rect, FixtureState::normal(), text_ctx, 100);
        let disabled =
            emit_canonical_command_bar_fixture(rect, FixtureState::disabled(), text_ctx, 100);

        assert_payload_indices_in_bounds(&normal.output);
        assert_payload_indices_in_bounds(&disabled.output);

        // Save + FolderOpen + Ellipsis 三枚校准矢量图标，外加 1 枚竖向分隔线。
        let unit = Rect::new(0.0, 0.0, 68.0, 44.0);
        let expected_count = shell_icon_prim_count(ShellIcon::Save, unit)
            + shell_icon_prim_count(ShellIcon::FolderOpen, unit)
            + shell_icon_prim_count(ShellIcon::Ellipsis, unit)
            + 1;

        assert_eq!(normal.output.primitives.len(), expected_count);
        assert_eq!(disabled.output.primitives.len(), expected_count);
        assert!(
            normal
                .output
                .primitives
                .iter()
                .all(|p| p.prim_type() != PrimType::Glyph && p.prim_type() != PrimType::GlyphColor),
            "CommandBar must not emit symbol glyphs when symbol font is disabled"
        );
    });
}

#[test]
fn app_bar_button_and_toggle_fall_back_to_save_and_bold_when_symbol_font_disabled() {
    with_test_ctx(|text_ctx| {
        text_ctx
            .engine
            .set_symbol_font_policy(SymbolFontPolicy::Disabled);
        assert!(!text_ctx.has_symbol_font());

        let btn_rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_APP_BAR_BUTTON_WIDTH,
            CANONICAL_APP_BAR_BUTTON_HEIGHT,
        );
        let btn_normal =
            emit_canonical_app_bar_button_fixture(btn_rect, FixtureState::normal(), text_ctx, 100);
        let btn_disabled = emit_canonical_app_bar_button_fixture(
            btn_rect,
            FixtureState::disabled(),
            text_ctx,
            100,
        );
        assert_payload_indices_in_bounds(&btn_normal.output);
        assert_payload_indices_in_bounds(&btn_disabled.output);

        let save_non_glyph = 1 + shell_icon_prim_count(ShellIcon::Save, btn_rect);
        let actual_btn_non_glyph = btn_normal
            .output
            .primitives
            .iter()
            .filter(|p| p.prim_type() != PrimType::Glyph && p.prim_type() != PrimType::GlyphColor)
            .count();
        assert_eq!(
            actual_btn_non_glyph, save_non_glyph,
            "AppBarButton must fall back to ShellIcon::Save"
        );

        let toggle_rect = Rect::new(
            0.0,
            0.0,
            CANONICAL_APP_BAR_TOGGLE_BUTTON_WIDTH,
            CANONICAL_APP_BAR_TOGGLE_BUTTON_HEIGHT,
        );
        let toggle_normal = emit_canonical_app_bar_toggle_button_fixture(
            toggle_rect,
            FixtureState::normal(),
            text_ctx,
            100,
        );
        let toggle_checked = emit_canonical_app_bar_toggle_button_fixture(
            toggle_rect,
            FixtureState::checked(),
            text_ctx,
            100,
        );
        let toggle_disabled = emit_canonical_app_bar_toggle_button_fixture(
            toggle_rect,
            FixtureState::disabled(),
            text_ctx,
            100,
        );
        assert_payload_indices_in_bounds(&toggle_normal.output);
        assert_payload_indices_in_bounds(&toggle_checked.output);
        assert_payload_indices_in_bounds(&toggle_disabled.output);

        let bold_non_glyph = 1 + shell_icon_prim_count(ShellIcon::Bold, toggle_rect);
        for (name, out) in [
            ("normal", &toggle_normal.output),
            ("checked", &toggle_checked.output),
            ("disabled", &toggle_disabled.output),
        ] {
            let count = out
                .primitives
                .iter()
                .filter(|p| {
                    p.prim_type() != PrimType::Glyph && p.prim_type() != PrimType::GlyphColor
                })
                .count();
            assert_eq!(
                count, bold_non_glyph,
                "AppBarToggleButton ({name}) must fall back to ShellIcon::Bold"
            );
        }
    });
}

#[test]
fn calendar_date_picker_falls_back_to_calendar_icon_and_keeps_24_state_invariant() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(16.0, 16.0, 200.0, CALENDAR_DATE_PICKER_HEIGHT);
        let desc = CalendarDatePickerDesc::new(rect).with_placeholder("Select a date");
        let state = CalendarDatePickerState::with_date(
            NaiveDate::new(2024, 4, 15).expect("valid parity date"),
        );

        // 1. 禁用符号字体路径：验证回退 ShellIcon::Calendar 与 24 态恒定图元数。
        text_ctx
            .engine
            .set_symbol_font_policy(SymbolFontPolicy::Disabled);
        let expected_non_glyph = 5 + shell_icon_prim_count(ShellIcon::Calendar, desc.icon_rect());
        let mut disabled_policy_counts = Vec::with_capacity(24);
        for control in all_control_states() {
            let out = desc.emit_laid_out(1, rect, control, &state, text_ctx);
            assert_payload_indices_in_bounds(&out);
            let non_glyph = out
                .primitives
                .iter()
                .filter(|p| {
                    p.prim_type() != PrimType::Glyph && p.prim_type() != PrimType::GlyphColor
                })
                .count();
            assert_eq!(
                non_glyph, expected_non_glyph,
                "CalendarDatePicker must fall back to ShellIcon::Calendar when symbol font is disabled"
            );
            disabled_policy_counts.push(out.primitives.len());
        }
        assert!(
            disabled_policy_counts.windows(2).all(|w| w[0] == w[1]),
            "CalendarDatePicker 24-state primitive count drifted under Disabled policy: {disabled_policy_counts:?}"
        );

        // 2. 默认 Auto 策略路径：无论宿主是否安装符号字体，24 态图元数均必须恒定且载荷索引有效。
        text_ctx
            .engine
            .set_symbol_font_policy(SymbolFontPolicy::Auto);
        let mut auto_policy_counts = Vec::with_capacity(24);
        for control in all_control_states() {
            let out = desc.emit_laid_out(1, rect, control, &state, text_ctx);
            assert_payload_indices_in_bounds(&out);
            auto_policy_counts.push(out.primitives.len());
        }
        assert!(
            auto_policy_counts.windows(2).all(|w| w[0] == w[1]),
            "CalendarDatePicker 24-state primitive count drifted under Auto policy: {auto_policy_counts:?}"
        );
    });
}

#[test]
fn rating_emits_star_vectors_and_keeps_24_state_invariant() {
    with_test_ctx(|text_ctx| {
        text_ctx
            .engine
            .set_symbol_font_policy(SymbolFontPolicy::Disabled);

        let desc = RatingDesc::new(Rect::new(0.0, 0.0, 160.0, 32.0), 3.5);
        let unit = Rect::new(0.0, 0.0, 16.0, 16.0);
        // 3.5 分在 allow_half=true 下产生 4 枚实心星 + 1 枚空心星，外加底板与焦点环各 1 枚。
        let expected_vector_prims = 2
            + 4 * shell_icon_prim_count(ShellIcon::StarFilled, unit)
            + shell_icon_prim_count(ShellIcon::Star, unit);

        let mut direct_counts = Vec::with_capacity(24);
        let mut with_text_counts = Vec::with_capacity(24);
        for control in all_control_states() {
            let direct = emit_rating(1, &desc, control);
            let with_text = emit_rating_with_text(1, &desc, control, text_ctx);

            assert_payload_indices_in_bounds(&direct);
            assert_payload_indices_in_bounds(&with_text);
            assert_eq!(direct.primitives.len(), expected_vector_prims);
            assert_eq!(with_text.primitives.len(), expected_vector_prims);
            assert!(
                direct
                    .primitives
                    .iter()
                    .any(|p| p.prim_type() == PrimType::Curve),
                "Rating must render star vector contours instead of rounded-square placeholders"
            );
            assert!(
                with_text
                    .primitives
                    .iter()
                    .all(|p| p.prim_type() != PrimType::Glyph),
                "disabled symbol font must keep Rating on vector star fallback"
            );

            direct_counts.push(direct.primitives.len());
            with_text_counts.push(with_text.primitives.len());
        }
        assert!(direct_counts.windows(2).all(|w| w[0] == w[1]));
        assert!(with_text_counts.windows(2).all(|w| w[0] == w[1]));
    });
}

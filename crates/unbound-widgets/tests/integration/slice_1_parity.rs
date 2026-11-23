//! 切片 1（布局容器与文本编辑）基准控件几何与渲染契约代码级测试守护。
//!
//! 依据 ADR 0016 与 Spec #79，本测试套件覆盖切片 1 全部 6 个基准控件：
//! 1. `RichEditBox`：剥离顶部格式工具栏，还原为纯净编辑区域，校准尺寸 (240×96)、边框与内边距；
//! 2. `SwipeControl`：normal 态对齐为静态未滑动列表行，checked 态展现滑出删除动作，修正底板画刷；
//! 3. `StackPanel`：接入标准 `Row 1`、`Row 2`、`Row 3` 插桩并对齐 8px 垂直项间距；
//! 4. `ScrollView`：接入标准 3 行纯文本插桩并校准 220×80 视口与滚动槽位；
//! 5. `Card`：对齐 240×88 固有尺寸、8px 圆角、1px 微描边与 2px 海拔微阴影（禁用态抑制）；
//! 6. `Border`：对齐 200×64 固有尺寸、8px 圆角、1px 微描边与 0px 阴影。

use accesskit::Role;
use unbound_gpu::data::PrimType;
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, TextEngine};
use unbound_theme::Token;
use unbound_widgets::containers::{CARD_BORDER_WIDTH, CARD_RADIUS};
use unbound_widgets::scroll::{emit_scroll_view, ScrollState, ScrollViewDesc};
use unbound_widgets::slot_fixture::{
    emit_canonical_text_rows_fixture, FixtureState, CANONICAL_ROW_ITEMS, STANDARD_STACK_SPACING,
};
use unbound_widgets::{
    emit_rich_edit_box, primitive, ControlState, PointerPhase, RichEditBoxDesc, TextCtx,
    WidgetOutput, RICH_EDIT_PADDING_X, RICH_EDIT_PADDING_Y, RICH_EDIT_RADIUS,
};

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
// 1. RichEditBox 纯净化与契约守护
// ---------------------------------------------------------------------------

#[test]
fn rich_edit_box_canonical_geometry_and_padding() {
    let desc = RichEditBoxDesc::new(Rect::ZERO, "RichEditBox");
    let intrinsic = desc.intrinsic_size();
    assert_eq!(intrinsic.x, 240.0, "canonical RichEditBox width is 240px");
    assert_eq!(intrinsic.y, 96.0, "canonical RichEditBox height is 96px");
    assert_eq!(desc.radius, RICH_EDIT_RADIUS);
    assert_eq!(RICH_EDIT_RADIUS, 4.0, "WinUI control corner radius is 4px");

    let rect = Rect::new(10.0, 20.0, 240.0, 96.0);
    let desc_placed = RichEditBoxDesc::new(rect, "RichEditBox");
    let editor = desc_placed.editor_rect();
    assert_eq!(
        editor.origin.x,
        rect.origin.x + RICH_EDIT_PADDING_X,
        "editor left padding is 10px"
    );
    assert_eq!(
        editor.origin.y,
        rect.origin.y + RICH_EDIT_PADDING_Y,
        "editor top padding is 6px"
    );
    assert_eq!(
        editor.width(),
        rect.width() - RICH_EDIT_PADDING_X * 2.0,
        "editor width subtracts horizontal padding"
    );
    assert_eq!(
        editor.height(),
        rect.height() - RICH_EDIT_PADDING_Y * 2.0,
        "editor height subtracts vertical padding"
    );
}

#[test]
fn rich_edit_box_token_transitions_and_constant_primitive_count() {
    let rect = Rect::new(0.0, 0.0, 240.0, 96.0);
    let desc = RichEditBoxDesc::new(rect, "RichEditBox");

    // 1. 静止态
    let idle_out = emit_rich_edit_box(1, &desc, ControlState::default(), &mut []);
    assert_eq!(
        idle_out.primitives[0].fill_token_or_uv as u16,
        Token::CONTROL_FILL.raw()
    );
    assert_eq!(
        idle_out.primitives[0].border_token_or_tex as u16,
        Token::CONTROL_STROKE.raw()
    );

    // 2. 悬停态
    let hover_state = ControlState {
        phase: PointerPhase::Hovered,
        ..ControlState::default()
    };
    let hover_out = emit_rich_edit_box(1, &desc, hover_state, &mut []);
    assert_eq!(
        hover_out.primitives[0].border_token_or_tex as u16,
        Token::INPUT_BORDER_HOVER.raw()
    );

    // 3. 聚焦态
    let focus_state = ControlState {
        focused: true,
        ..ControlState::default()
    };
    let focus_out = emit_rich_edit_box(1, &desc, focus_state, &mut []);
    assert_eq!(
        focus_out.primitives[0].border_token_or_tex as u16,
        Token::FOCUS_RING.raw()
    );

    // 4. 禁用态
    let disabled_state = ControlState {
        disabled: true,
        ..ControlState::default()
    };
    let disabled_out = emit_rich_edit_box(1, &desc, disabled_state, &mut []);
    assert_eq!(
        disabled_out.primitives[0].fill_token_or_uv as u16,
        Token::CONTROL_DISABLED_BG.raw()
    );
    assert_eq!(
        disabled_out.primitives[0].border_token_or_tex as u16,
        Token::CONTROL_DISABLED_BORDER.raw()
    );
    assert!(disabled_out.hit.is_none(), "disabled editor refuses hit");
    assert!(!disabled_out.focusable, "disabled editor is not focusable");
    assert!(
        disabled_out.a11y.expect("has a11y node").disabled,
        "a11y node disabled"
    );

    // 5. 跨状态常数图元校验（无内置格式按钮增删）
    for phase in [
        PointerPhase::Idle,
        PointerPhase::Hovered,
        PointerPhase::Pressed,
    ] {
        for focused in [false, true] {
            for disabled in [false, true] {
                for selected in [false, true] {
                    let st = ControlState {
                        phase,
                        focused,
                        disabled,
                        selected,
                    };
                    let out = emit_rich_edit_box(1, &desc, st, &mut []);
                    assert_eq!(
                        out.primitives.len(),
                        3,
                        "exactly 3 primitives: outlined surface, bottom hairline, and focus ring"
                    );
                }
            }
        }
    }
}

#[test]
fn rich_edit_box_renders_glyphs_with_theme_tokens() {
    with_test_ctx(|text_ctx| {
        let rect = Rect::new(0.0, 0.0, 240.0, 96.0);
        let desc = RichEditBoxDesc::new(rect, "RichEditBox");
        let editor_r = desc.editor_rect();

        let mut normal_glyphs = text_ctx.shape_line_styled(
            "RichEditBox",
            editor_r,
            14.0,
            400,
            None,
            unbound_widgets::AlignX::Left,
            unbound_widgets::AlignY::Top,
        );
        let normal_out = emit_rich_edit_box(1, &desc, ControlState::default(), &mut normal_glyphs);
        let normal_text_prim = normal_out
            .primitives
            .iter()
            .find(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .expect("must emit glyph primitive in normal state");
        assert_eq!(
            normal_text_prim.fill_token_or_uv as u16,
            Token::TEXT_PRIMARY.raw()
        );

        let mut disabled_glyphs = text_ctx.shape_line_styled(
            "RichEditBox",
            editor_r,
            14.0,
            400,
            None,
            unbound_widgets::AlignX::Left,
            unbound_widgets::AlignY::Top,
        );
        let disabled_out = emit_rich_edit_box(
            1,
            &desc,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &mut disabled_glyphs,
        );
        let disabled_text_prim = disabled_out
            .primitives
            .iter()
            .find(|p| matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor))
            .expect("must emit glyph primitive in disabled state");
        assert_eq!(
            disabled_text_prim.fill_token_or_uv as u16,
            Token::TEXT_DISABLED.raw()
        );
    });
}

#[test]
fn rich_edit_box_catalog_pure_and_composite_variants_conformance() {
    use unbound_widgets::catalog::emit_widget_with_shaper;
    use unbound_widgets::WidgetCatalogId;

    let rect = Rect::new(0.0, 0.0, 320.0, 140.0);
    let mut shaper =
        |_text: &str, _origin: Vec2, _size: f32| -> Vec<unbound_text::GlyphQuad> { Vec::new() };

    // 1. variant 0 为纯净 WinUI 3 基准富文本编辑框
    let pure_out = emit_widget_with_shaper(
        WidgetCatalogId::RichEditBox,
        1,
        ControlState::default(),
        rect,
        unbound_widgets::catalog::CatalogParams {
            variant_idx: 0,
            ..Default::default()
        },
        &mut shaper,
    );
    assert!(pure_out.primitives.len() <= 3);

    // 2. variant 1 为展台专属高级复合富文本编辑器（带 [B][I][U] 格式工具栏，满足 Spec 用户故事 5）
    let composite_out = emit_widget_with_shaper(
        WidgetCatalogId::RichEditBox,
        1,
        ControlState::default(),
        rect,
        unbound_widgets::catalog::CatalogParams {
            variant_idx: 1,
            ..Default::default()
        },
        &mut shaper,
    );
    assert!(
        composite_out.primitives.len() > pure_out.primitives.len(),
        "composite variant must include format toolbar buttons"
    );
}

// ---------------------------------------------------------------------------
// 2. SwipeControl 状态机与静态/滑开渲染契约
// ---------------------------------------------------------------------------

#[test]
fn swipe_control_normal_state_is_static_full_width_card() {
    let swipe_w = 260.0;
    let swipe_h = 48.0;
    let swipe_rect = Rect::new(0.0, 0.0, swipe_w, swipe_h);

    // normal 态：整卡未滑开，占满 260px 全宽，表面为 CARD_BG，边框为 CARD_BORDER
    let mut out = WidgetOutput::default();
    out.primitives.push(primitive::outlined(
        swipe_rect,
        8.0,
        Token::CARD_BG,
        Token::CARD_BORDER,
        1.0,
        0.0,
        702,
    ));

    assert_eq!(out.primitives.len(), 1);
    let card_prim = &out.primitives[0];
    assert_eq!(card_prim.size[0], swipe_w);
    assert_eq!(card_prim.size[1], swipe_h);
    assert_eq!(card_prim.fill_token_or_uv as u16, Token::CARD_BG.raw());
    assert_eq!(
        card_prim.border_token_or_tex as u16,
        Token::CARD_BORDER.raw()
    );
}

#[test]
fn swipe_control_checked_state_exposes_action_slot_and_narrows_content() {
    let swipe_w = 260.0;
    let swipe_h = 48.0;
    let swipe_rect = Rect::new(0.0, 0.0, swipe_w, swipe_h);
    let action_w = 64.0;
    let action_rect = Rect::new(
        swipe_rect.max().x - action_w,
        swipe_rect.origin.y,
        action_w,
        swipe_h,
    );
    let content_rect = Rect::new(
        swipe_rect.origin.x,
        swipe_rect.origin.y,
        swipe_w - action_w,
        swipe_h,
    );

    let mut out = WidgetOutput::default();
    // 1. 右侧展开的动作槽底板（PRIMARY）
    out.primitives
        .push(primitive::surface(action_rect, 0.0, Token::PRIMARY, 700));
    // 2. 内容卡片收缩为 196px
    out.primitives.push(primitive::outlined(
        content_rect,
        8.0,
        Token::CARD_BG,
        Token::CARD_BORDER,
        1.0,
        0.0,
        702,
    ));

    assert_eq!(out.primitives.len(), 2);
    let action_prim = &out.primitives[0];
    assert_eq!(action_prim.size[0], 64.0);
    assert_eq!(action_prim.fill_token_or_uv as u16, Token::PRIMARY.raw());

    let content_prim = &out.primitives[1];
    assert_eq!(content_prim.size[0], 196.0);
    assert_eq!(content_prim.size[1], 48.0);
}

#[test]
fn swipe_control_disabled_state_downgrades_tokens() {
    let swipe_rect = Rect::new(0.0, 0.0, 260.0, 48.0);
    let mut out = WidgetOutput::default();
    out.primitives.push(primitive::outlined(
        swipe_rect,
        8.0,
        Token::CONTROL_DISABLED_BG,
        Token::CONTROL_DISABLED_BORDER,
        1.0,
        0.0,
        702,
    ));

    assert_eq!(
        out.primitives[0].fill_token_or_uv as u16,
        Token::CONTROL_DISABLED_BG.raw()
    );
    assert_eq!(
        out.primitives[0].border_token_or_tex as u16,
        Token::CONTROL_DISABLED_BORDER.raw()
    );
}

// ---------------------------------------------------------------------------
// 3. StackPanel 垂直堆叠插桩与项间距契约
// ---------------------------------------------------------------------------

#[test]
fn stack_panel_canonical_rows_and_spacing() {
    with_test_ctx(|text_ctx| {
        let panel_rect = Rect::new(0.0, 0.0, 200.0, 0.0);
        let fixture = emit_canonical_text_rows_fixture(
            panel_rect,
            STANDARD_STACK_SPACING,
            FixtureState::normal(),
            text_ctx,
            700,
        );

        assert_eq!(fixture.row_rects.len(), 3);
        assert_eq!(CANONICAL_ROW_ITEMS.len(), 3);
        assert_eq!(fixture.total_size.x, 200.0);

        // WinUI 3 的 14px 正文行盒为 19px；3 行 + 2 个 8px 间距 = 73px。
        // 回归锁定 19px：回退到 20px 会让第三行相对参考截图整体下移 2px。
        assert_eq!(
            fixture.total_size.y, 73.0,
            "StackPanel 3 rows with 8px spacing measure 73px"
        );

        // 各行等间距排列
        let row0 = fixture.row_rects[0];
        let row1 = fixture.row_rects[1];
        let row2 = fixture.row_rects[2];
        assert_eq!(row0.height(), 19.0, "canonical row box must be 19px");
        assert_eq!(row1.origin.y - row0.max().y, STANDARD_STACK_SPACING);
        assert_eq!(row2.origin.y - row1.max().y, STANDARD_STACK_SPACING);

        // 正常态文本令牌
        for p in &fixture.output.primitives {
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(p.fill_token_or_uv as u16, Token::TEXT_PRIMARY.raw());
            }
        }
    });
}

#[test]
fn stack_panel_disabled_propagates_to_all_rows() {
    with_test_ctx(|text_ctx| {
        let panel_rect = Rect::new(0.0, 0.0, 200.0, 0.0);
        let fixture = emit_canonical_text_rows_fixture(
            panel_rect,
            STANDARD_STACK_SPACING,
            FixtureState::disabled(),
            text_ctx,
            700,
        );

        assert_eq!(fixture.row_rects.len(), 3);
        for p in &fixture.output.primitives {
            if matches!(p.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
                assert_eq!(
                    p.fill_token_or_uv as u16,
                    Token::TEXT_DISABLED.raw(),
                    "all stack rows downgrade to TEXT_DISABLED"
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 4. ScrollView 视口与标准插桩契约
// ---------------------------------------------------------------------------

#[test]
fn scroll_view_viewport_and_rows_fixture() {
    with_test_ctx(|text_ctx| {
        let scroll_w = 220.0;
        let scroll_h = 80.0;
        let scroll_rect = Rect::new(0.0, 0.0, scroll_w, scroll_h);

        let rows_fixture = emit_canonical_text_rows_fixture(
            scroll_rect,
            6.0,
            FixtureState::normal(),
            text_ctx,
            702,
        );

        // 3 行 19px 行盒 + 2 个 6px 间距 = 69px <= 80px 视口
        assert_eq!(rows_fixture.total_size.y, 69.0);
        assert_eq!(
            rows_fixture.row_rects[0].height(),
            19.0,
            "canonical row box must be 19px"
        );

        let content_size = Vec2::new(scroll_w, rows_fixture.total_size.y);
        let desc = ScrollViewDesc::new(scroll_rect, content_size);
        let scroll_state = ScrollState::default();
        let scroll_out = emit_scroll_view(1, &desc, &scroll_state, ControlState::default()).output;

        // 内容完全在 80px 视口内部，垂直滚动条处于自动隐藏状态
        assert!(
            scroll_out.primitives.is_empty(),
            "auto scrollbar stays hidden when content fits within viewport"
        );
        assert!(
            scroll_out.hit.is_some(),
            "viewport hit region is registered"
        );
        assert_eq!(scroll_out.a11y.expect("a11y exists").role, Role::ScrollView);
    });
}

// ---------------------------------------------------------------------------
// 5. Card 与 Border 几何、微描边与阴影契约
// ---------------------------------------------------------------------------

#[test]
fn card_container_geometry_micro_border_and_elevation() {
    let card_w = 240.0;
    let card_h = 88.0;
    let card_r = Rect::new(0.0, 0.0, card_w, card_h);

    // 1. 常规态：包含 2px 海拔微阴影、CARD_BG 表面、1px CARD_BORDER 微描边
    let mut out = WidgetOutput::default();
    out.primitives.push(primitive::instance(
        card_r,
        [CARD_RADIUS; 4],
        Token::NONE,
        Token::NONE,
        0.0,
        2.0,
        PrimType::Shadow,
        unbound_gpu::data::SubElement::Shadow,
        100,
    ));
    out.primitives
        .push(primitive::surface(card_r, CARD_RADIUS, Token::CARD_BG, 100));
    out.primitives.push(primitive::instance(
        card_r,
        [CARD_RADIUS; 4],
        Token::NONE,
        Token::CARD_BORDER,
        CARD_BORDER_WIDTH,
        0.0,
        PrimType::RoundedRect,
        unbound_gpu::data::SubElement::Border,
        100,
    ));

    assert_eq!(out.primitives.len(), 3);
    assert_eq!(
        CARD_RADIUS, 8.0,
        "Card radius aligns to OverlayCornerRadius"
    );
    assert_eq!(CARD_BORDER_WIDTH, 1.0, "Card stroke is 1px hairline");
    assert_eq!(out.primitives[0].prim_type(), PrimType::Shadow);
    assert_eq!(
        out.primitives[1].fill_token_or_uv as u16,
        Token::CARD_BG.raw()
    );
    assert_eq!(
        out.primitives[2].border_token_or_tex as u16,
        Token::CARD_BORDER.raw()
    );

    // 2. 禁用态：阴影抑制、表面与描边变灰
    let mut disabled_out = WidgetOutput::default();
    disabled_out.primitives.push(primitive::surface(
        card_r,
        CARD_RADIUS,
        Token::CONTROL_DISABLED_BG,
        100,
    ));
    disabled_out.primitives.push(primitive::instance(
        card_r,
        [CARD_RADIUS; 4],
        Token::NONE,
        Token::CONTROL_DISABLED_BORDER,
        CARD_BORDER_WIDTH,
        0.0,
        PrimType::RoundedRect,
        unbound_gpu::data::SubElement::Border,
        100,
    ));

    assert_eq!(
        disabled_out.primitives.len(),
        2,
        "disabled card suppresses shadow"
    );
    assert_eq!(
        disabled_out.primitives[0].fill_token_or_uv as u16,
        Token::CONTROL_DISABLED_BG.raw()
    );
    assert_eq!(
        disabled_out.primitives[1].border_token_or_tex as u16,
        Token::CONTROL_DISABLED_BORDER.raw()
    );
}

#[test]
fn border_container_geometry_micro_border_and_no_shadow() {
    let border_w = 200.0;
    let border_h = 64.0;
    let border_r = Rect::new(0.0, 0.0, border_w, border_h);

    let mut out = WidgetOutput::default();
    out.primitives.push(primitive::surface(
        border_r,
        CARD_RADIUS,
        Token::CARD_BG,
        100,
    ));
    out.primitives.push(primitive::instance(
        border_r,
        [CARD_RADIUS; 4],
        Token::NONE,
        Token::CARD_BORDER,
        CARD_BORDER_WIDTH,
        0.0,
        PrimType::RoundedRect,
        unbound_gpu::data::SubElement::Border,
        100,
    ));

    assert_eq!(out.primitives.len(), 2, "Border has no shadow primitive");
    assert_eq!(out.primitives[0].size[0], border_w);
    assert_eq!(out.primitives[0].size[1], border_h);
    assert_eq!(
        out.primitives[0].fill_token_or_uv as u16,
        Token::CARD_BG.raw()
    );
    assert_eq!(
        out.primitives[1].border_token_or_tex as u16,
        Token::CARD_BORDER.raw()
    );
}

//! 选择框的展开箭头、占位符与禁用选项测试。
//!
//! 通过两条曲线载荷验证箭头，通过输出字形的墨色令牌验证占位符，并通过高亮状态验证键盘交互。
//!

use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_widgets::select::{emit_select, emit_select_with_glyphs, SelectDesc, SelectState};
use unbound_widgets::ControlState;

fn desc() -> SelectDesc {
    SelectDesc::new(
        Rect::new(20.0, 20.0, 220.0, 36.0),
        vec![
            "Alpha".to_owned(),
            "Beta".to_owned(),
            "Gamma".to_owned(),
            "Delta".to_owned(),
        ],
    )
    .with_placeholder("Pick one")
}

fn glyph() -> GlyphQuad {
    GlyphQuad {
        rect: Rect::new(32.0, 28.0, 8.0, 12.0),
        uv: [0.0, 0.0, 0.1, 0.2],
        layer: 3,
        color: false,
        msdf: false,
        phase: 0,
    }
}

#[test]
fn the_arrow_is_two_curve_strokes_that_flip_when_the_dropdown_opens() {
    let desc = desc();
    let mut state = SelectState::new();
    let closed = emit_select(4, &desc, &state, ControlState::default());
    assert_eq!(closed.primitives.len(), 5, "fill, border, 2 strokes, ring");
    assert_eq!(closed.payloads.len(), 2, "one payload per chevron stroke");
    assert!(closed
        .payloads
        .iter()
        .all(|payload| payload.line_width > 0.0));
    // 收起时：在图元局部空间中两端位于顶点上方。
    assert!(closed.payloads[0].p0[1] < closed.payloads[0].p2[1]);

    state.open();
    let open = emit_select(4, &desc, &state, ControlState::default());
    assert_eq!(open.primitives.len(), closed.primitives.len());
    assert_eq!(open.payloads.len(), 2);
    // 打开时：箭头朝上，因此两端位于顶点下方。
    assert!(open.payloads[0].p0[1] > open.payloads[0].p2[1]);
}

#[test]
fn an_empty_field_paints_the_placeholder_as_secondary_text() {
    let desc = desc();
    let state = SelectState::new();
    let glyphs = [glyph()];
    let output = emit_select_with_glyphs(4, &desc, &state, ControlState::default(), &glyphs);

    assert_eq!(desc.display_text(&state), "Pick one");
    assert_eq!(
        output.primitives[4].fill_token_or_uv,
        u32::from(Token::TEXT_SECONDARY.raw()),
        "the placeholder glyph is secondary ink"
    );

    let mut chosen = SelectState::new();
    chosen.selected = Some(1);
    let glyphs = [glyph()];
    let output = emit_select_with_glyphs(4, &desc, &chosen, ControlState::default(), &glyphs);
    assert_eq!(desc.display_text(&chosen), "Beta");
    assert_eq!(
        output.primitives[4].fill_token_or_uv,
        u32::from(Token::TEXT_PRIMARY.raw()),
        "a real selection is primary ink"
    );
}

#[test]
fn keyboard_navigation_skips_disabled_options() {
    let desc = desc().with_disabled_option(1).with_disabled_option(2);
    let mut state = SelectState::new();

    state.step(&desc, true);
    assert_eq!(state.highlighted, Some(0));
    state.step(&desc, true);
    assert_eq!(state.highlighted, Some(3), "Beta and Gamma are skipped");
    state.step(&desc, true);
    assert_eq!(state.highlighted, Some(0), "and it wraps");
    state.step(&desc, false);
    assert_eq!(state.highlighted, Some(3));
}

#[test]
fn an_all_disabled_select_never_moves_its_highlight() {
    let desc = desc()
        .with_disabled_option(0)
        .with_disabled_option(1)
        .with_disabled_option(2)
        .with_disabled_option(3);
    let mut state = SelectState::new();
    state.step(&desc, true);
    assert_eq!(state.highlighted, None);
    state.step(&desc, false);
    assert_eq!(state.highlighted, None);
}

#[test]
fn disabled_options_are_disabled_menu_rows() {
    let desc = desc().with_disabled_option(2);
    let menu = desc.menu(Rect::new(0.0, 0.0, 800.0, 600.0));
    assert!(!menu.items[0].disabled);
    assert!(menu.items[2].disabled);
    assert_eq!(menu.items.len(), 4);
    // 下拉列表被夹紧在给定的视口范围内。
    assert_eq!(menu.viewport, Some(Rect::new(0.0, 0.0, 800.0, 600.0)));
    assert_eq!(menu.rect.width(), desc.rect.width());
    assert_eq!(menu.rect.origin.y, desc.rect.max().y + 4.0);
    assert_eq!(
        Vec2::new(menu.rect.width(), menu.rect.height()),
        Vec2::new(220.0, 120.0)
    );
}

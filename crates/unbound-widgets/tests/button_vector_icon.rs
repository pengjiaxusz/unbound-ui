//! 按钮原生矢量图标槽（[`ShellIcon`]）单元测试。
//!
//! 验证带有 `ShellIcon::Play` 与 `ShellIcon::MoreHorizontal` 的按钮在
//! 默认、悬停、按下、聚焦与禁用态下图元数量保持恒定，前景色随状态切换，
//! 且所有图标与标签图元均严格落在按钮矩形内、互不重叠。

use unbound_gpu::data::{GpuPrimitiveInstance, PrimFlags, PrimType, SubElement};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_widgets::button::{
    emit_button, ButtonDesc, ButtonVariant, BUTTON_CONTENT_GAP, BUTTON_SHELL_ICON_SIZE,
};
use unbound_widgets::{ControlState, PointerPhase, ShellIcon, WidgetOutput};

fn sample_rect() -> Rect {
    Rect::new(20.0, 20.0, 160.0, 36.0)
}

fn sample_glyphs() -> Vec<GlyphQuad> {
    vec![
        GlyphQuad {
            rect: Rect::new(0.0, 0.0, 18.0, 14.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        },
        GlyphQuad {
            rect: Rect::new(20.0, 0.0, 22.0, 14.0),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            msdf: false,
            phase: 0,
        },
    ]
}

fn sub_element(primitive: &GpuPrimitiveInstance) -> u8 {
    PrimFlags((primitive.prim_type_and_flags >> 16) as u16).sub_element()
}

fn prim_rect(primitive: &GpuPrimitiveInstance) -> Rect {
    Rect::new(
        primitive.local_pos[0],
        primitive.local_pos[1],
        primitive.size[0],
        primitive.size[1],
    )
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    const EPS: f32 = 1e-3;
    inner.origin.x >= outer.origin.x - EPS
        && inner.origin.y >= outer.origin.y - EPS
        && inner.max().x <= outer.max().x + EPS
        && inner.max().y <= outer.max().y + EPS
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.origin.x < b.max().x
        && a.max().x > b.origin.x
        && a.origin.y < b.max().y
        && a.max().y > b.origin.y
}

fn is_glyph(primitive: &GpuPrimitiveInstance) -> bool {
    matches!(
        primitive.prim_type(),
        PrimType::Glyph | PrimType::GlyphColor
    )
}

fn content_icon_primitives(output: &WidgetOutput) -> Vec<&GpuPrimitiveInstance> {
    output
        .primitives
        .iter()
        .filter(|primitive| {
            sub_element(primitive) == SubElement::Content as u8 && !is_glyph(primitive)
        })
        .collect()
}

fn content_glyph_primitives(output: &WidgetOutput) -> Vec<&GpuPrimitiveInstance> {
    output
        .primitives
        .iter()
        .filter(|primitive| {
            sub_element(primitive) == SubElement::Content as u8 && is_glyph(primitive)
        })
        .collect()
}

fn states() -> [ControlState; 5] {
    [
        ControlState::default(),
        ControlState {
            phase: PointerPhase::Hovered,
            ..ControlState::default()
        },
        ControlState {
            phase: PointerPhase::Pressed,
            ..ControlState::default()
        },
        ControlState {
            focused: true,
            ..ControlState::default()
        },
        ControlState {
            disabled: true,
            ..ControlState::default()
        },
    ]
}

fn assert_constant_count_and_non_overlapping(desc: &ButtonDesc) {
    let tokens = desc.tokens();
    let mut expected_count: Option<usize> = None;

    for state in states() {
        let mut glyphs = sample_glyphs();
        let output = emit_button(1, desc, state, &mut glyphs);

        match expected_count {
            None => expected_count = Some(output.primitives.len()),
            Some(expected) => assert_eq!(
                output.primitives.len(),
                expected,
                "button primitive count must stay constant across states (state={state:?})"
            ),
        }

        let icon_prims = content_icon_primitives(&output);
        let glyph_prims = content_glyph_primitives(&output);
        assert!(
            !icon_prims.is_empty(),
            "shell_icon button must emit content icon primitives"
        );
        assert_eq!(
            glyph_prims.len(),
            2,
            "all label glyphs must be emitted in Content layer"
        );

        let expected_fg = if state.disabled {
            tokens.label_disabled.raw()
        } else {
            tokens.label.raw()
        };

        for icon_prim in &icon_prims {
            let icon_r = prim_rect(icon_prim);
            assert!(
                contains_rect(desc.rect, icon_r),
                "icon primitive {icon_r:?} must fall inside button rect {:?}",
                desc.rect
            );
            assert_eq!(
                icon_prim.fill_token_or_uv as u16, expected_fg,
                "icon primitive must use the button's state foreground token"
            );
        }

        for glyph_prim in &glyph_prims {
            let glyph_r = prim_rect(glyph_prim);
            assert!(
                contains_rect(desc.rect, glyph_r),
                "glyph primitive {glyph_r:?} must fall inside button rect {:?}",
                desc.rect
            );
            for icon_prim in &icon_prims {
                let icon_r = prim_rect(icon_prim);
                assert!(
                    !rects_overlap(icon_r, glyph_r),
                    "icon primitive {icon_r:?} must not overlap glyph primitive {glyph_r:?}"
                );
            }
        }
    }
}

#[test]
fn play_shell_icon_has_constant_primitives_and_clean_layout() {
    let desc = ButtonDesc::new(sample_rect(), "Run")
        .with_variant(ButtonVariant::Primary)
        .with_shell_icon(ShellIcon::Play);

    assert_constant_count_and_non_overlapping(&desc);

    let mut glyphs = sample_glyphs();
    let output = emit_button(1, &desc, ControlState::default(), &mut glyphs);
    let icon_max_x = content_icon_primitives(&output)
        .into_iter()
        .map(|p| prim_rect(p).max().x)
        .fold(f32::NEG_INFINITY, f32::max);
    let glyph_min_x = content_glyph_primitives(&output)
        .into_iter()
        .map(|p| prim_rect(p).origin.x)
        .fold(f32::INFINITY, f32::min);

    assert!(
        icon_max_x < glyph_min_x,
        "leading Play icon (max_x={icon_max_x}) must sit to the left of label (min_x={glyph_min_x})"
    );
}

#[test]
fn more_horizontal_shell_icon_has_constant_primitives_and_supports_trailing() {
    let leading = ButtonDesc::new(sample_rect(), "More")
        .with_variant(ButtonVariant::Secondary)
        .with_shell_icon(ShellIcon::MoreHorizontal);
    assert_constant_count_and_non_overlapping(&leading);

    let trailing = leading.clone().with_shell_icon_trailing(true);
    assert_constant_count_and_non_overlapping(&trailing);

    let mut glyphs = sample_glyphs();
    let output = emit_button(2, &trailing, ControlState::default(), &mut glyphs);
    let icon_prims = content_icon_primitives(&output);
    assert_eq!(
        icon_prims.len(),
        3,
        "MoreHorizontal must emit exactly 3 horizontal dots"
    );

    let icon_min_x = icon_prims
        .iter()
        .map(|p| prim_rect(p).origin.x)
        .fold(f32::INFINITY, f32::min);
    let glyph_max_x = content_glyph_primitives(&output)
        .into_iter()
        .map(|p| prim_rect(p).max().x)
        .fold(f32::NEG_INFINITY, f32::max);

    assert!(
        icon_min_x > glyph_max_x,
        "trailing MoreHorizontal icon (min_x={icon_min_x}) must sit to the right of label (max_x={glyph_max_x})"
    );
}

#[test]
fn icon_only_shell_icon_button_centers_within_rect() {
    let rect = Rect::new(10.0, 10.0, 32.0, 32.0);
    let desc = ButtonDesc::new(rect, "")
        .with_variant(ButtonVariant::Subtle)
        .with_shell_icon(ShellIcon::MoreHorizontal);

    let mut glyphs = Vec::new();
    let output = emit_button(3, &desc, ControlState::default(), &mut glyphs);
    let icon_prims = content_icon_primitives(&output);
    assert_eq!(icon_prims.len(), 3);

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for prim in &icon_prims {
        let r = prim_rect(prim);
        assert!(contains_rect(rect, r));
        min_x = min_x.min(r.origin.x);
        max_x = max_x.max(r.max().x);
    }
    let icon_center_x = (min_x + max_x) * 0.5;
    assert!(
        (icon_center_x - rect.center().x).abs() < 0.01,
        "icon-only button should center shell_icon horizontally (got {icon_center_x}, expected {})",
        rect.center().x
    );
    const { assert!(BUTTON_SHELL_ICON_SIZE > 0.0 && BUTTON_CONTENT_GAP > 0.0) };
}

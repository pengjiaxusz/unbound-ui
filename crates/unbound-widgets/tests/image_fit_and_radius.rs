//! 图像控件的独立圆角半径、边框与九宫格（nine-patch）变体测试。
//!
//! 无 GPU：测试直接检查输出图元的打包圆角、边框令牌以及九宫格的九个 UV 切分单元格。
//!

use unbound_gpu::data::PrimType;
use unbound_math::{f16, Rect, Vec2};
use unbound_theme::Token;
use unbound_widgets::image::{emit_image, emit_nine_patch, ImageDesc, NinePatchDesc};
use unbound_widgets::ControlState;

fn rect() -> Rect {
    Rect::new(10.0, 20.0, 200.0, 100.0)
}

#[test]
fn per_corner_radii_reach_the_primitive() {
    let desc = ImageDesc::new(rect(), Vec2::new(100.0, 100.0), 7).with_radii([1.0, 2.0, 3.0, 4.0]);
    let output = emit_image(1, &desc, ControlState::default());
    assert_eq!(output.primitives.len(), 1);
    assert_eq!(output.primitives[0].prim_type(), PrimType::Image);
    assert_eq!(
        output.primitives[0].corner_radii,
        f16::pack_radii([1.0, 2.0, 3.0, 4.0])
    );
}

#[test]
fn the_uniform_radius_helper_fills_all_four_corners() {
    let desc = ImageDesc::new(rect(), Vec2::new(100.0, 100.0), 7).with_radius(6.0);
    assert_eq!(desc.radius, [6.0; 4]);
    let output = emit_image(1, &desc, ControlState::default());
    assert_eq!(output.primitives[0].corner_radii, f16::pack_radii([6.0; 4]));
}

#[test]
fn a_border_is_a_second_stroke_with_the_same_radii() {
    let desc = ImageDesc::new(rect(), Vec2::new(100.0, 100.0), 7)
        .with_radius(8.0)
        .with_border(2.0, Token::OUTLINE);
    let output = emit_image(1, &desc, ControlState::default());
    assert_eq!(output.primitives.len(), 2, "texture + border stroke");

    let image = &output.primitives[0];
    assert_eq!(image.border_token_or_tex, 7, "the image keeps its texture");
    let border = &output.primitives[1];
    assert_eq!(border.prim_type(), PrimType::RoundedRect);
    assert_eq!(border.border_token_or_tex, u32::from(Token::OUTLINE.raw()));
    assert_eq!(border.border_width, f16::f32_to_f16(2.0));
    assert_eq!(border.corner_radii, f16::pack_radii([8.0; 4]));
}

#[test]
fn a_zero_width_border_paints_no_extra_primitive() {
    let desc = ImageDesc::new(rect(), Vec2::new(100.0, 100.0), 7).with_border(0.0, Token::OUTLINE);
    assert_eq!(
        emit_image(1, &desc, ControlState::default())
            .primitives
            .len(),
        1
    );
}

#[test]
fn a_nine_patch_emits_nine_uv_split_primitives() {
    let desc = NinePatchDesc::new(rect(), Vec2::new(60.0, 60.0), 5, [10.0, 12.0, 14.0, 16.0]);
    let output = emit_nine_patch(1, &desc, ControlState::default());

    assert_eq!(output.primitives.len(), 9);
    assert_eq!(output.payloads.len(), 9);
    assert!(output
        .primitives
        .iter()
        .all(|primitive| primitive.prim_type() == PrimType::Image));
    assert!(output
        .primitives
        .iter()
        .all(|primitive| primitive.border_token_or_tex == 5));
    for (index, primitive) in output.primitives.iter().enumerate() {
        assert_eq!(primitive.extra_payload_idx, index as u32);
    }

    // 四角保持自然内边距；中间区域拉伸。
    let cells = desc.cells();
    assert_eq!(cells[0], Rect::new(10.0, 20.0, 10.0, 12.0));
    assert_eq!(
        cells[4].size,
        Vec2::new(200.0 - 10.0 - 14.0, 100.0 - 12.0 - 16.0)
    );
    assert_eq!(cells[8].max(), rect().max());

    // UV 将纹理从 (0, 0) 平铺到 (1, 1)。
    assert_eq!(output.payloads[0].p0, [0.0, 0.0]);
    assert_eq!(output.payloads[0].p1, [10.0 / 60.0, 12.0 / 60.0]);
    assert_eq!(output.payloads[8].p1, [1.0, 1.0]);
}

#[test]
fn a_nine_patch_with_no_insets_still_splits_into_nine_cells() {
    let desc = NinePatchDesc::new(rect(), Vec2::new(60.0, 60.0), 5, [0.0; 4]);
    let output = emit_nine_patch(1, &desc, ControlState::default());
    assert_eq!(output.primitives.len(), 9);
    assert_eq!(desc.cells()[0].size, Vec2::ZERO);
    assert_eq!(desc.cells()[4], rect());
}

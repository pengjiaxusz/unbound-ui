//! 开关的弹簧位置与按下挤压形变几何测试。

use unbound_gpu::data::PrimType;
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_widgets::switch::{self, SwitchDesc, SwitchTokens};
use unbound_widgets::{ControlState, PointerPhase};

fn desc() -> SwitchDesc {
    SwitchDesc::new(Rect::new(0.0, 0.0, 200.0, 40.0), "Notifications")
}

fn glyph() -> GlyphQuad {
    GlyphQuad {
        rect: Rect::new(10.0, 14.0, 8.0, 12.0),
        uv: [0.0, 0.0, 1.0, 1.0],
        layer: 0,
        color: false,
        msdf: false,
        phase: 0,
    }
}

#[test]
fn thumb_t_interpolates_the_knob() {
    let off = desc();
    let on = desc().with_on(true);
    let halfway = desc().with_on(true).with_thumb_t(0.5);

    assert_eq!(off.knob_rect().origin.x, 160.0);
    assert_eq!(on.knob_rect().origin.x, 180.0);
    assert_eq!(halfway.knob_rect().origin.x, 170.0);
    assert_eq!(halfway.thumb_position(), 0.5);
}

#[test]
fn thumb_t_is_clamped_to_the_track() {
    assert_eq!(desc().with_thumb_t(-1.0).knob_rect().origin.x, 160.0);
    assert_eq!(desc().with_thumb_t(9.0).knob_rect().origin.x, 180.0);
}

#[test]
fn a_pressed_knob_stretches_into_a_capsule() {
    let off = desc();
    let idle = off.knob_rect_pressed(false);
    let pressed = off.knob_rect_pressed(true);
    assert_eq!(idle.size.x, 16.0);
    assert_eq!(pressed.size.x, 20.0);
    assert_eq!(pressed.origin.x, idle.origin.x, "an off knob grows right");
    assert_eq!(pressed.max().x, idle.max().x + 4.0);

    let on_idle = desc().with_on(true).knob_rect_pressed(false);
    let on_pressed = desc().with_on(true).knob_rect_pressed(true);
    assert_eq!(on_pressed.size.x, 20.0);
    assert_eq!(on_pressed.max().x, on_idle.max().x, "an on knob grows left");
    assert_eq!(on_pressed.origin.x, on_idle.origin.x - 4.0);
}

#[test]
fn the_pressed_state_uses_the_squished_knob() {
    let pressed = ControlState {
        phase: PointerPhase::Pressed,
        ..ControlState::default()
    };
    let output = switch::emit_switch(1, &desc(), pressed, &[], SwitchTokens::standard());
    assert_eq!(output.primitives[1].size[0], 20.0);
    let idle = switch::emit_switch(
        1,
        &desc(),
        ControlState::default(),
        &[],
        SwitchTokens::standard(),
    );
    assert_eq!(idle.primitives[1].size[0], 16.0);
}

#[test]
fn a_disabled_switch_uses_the_disabled_tokens() {
    let tokens = SwitchTokens::standard();
    let label = vec![glyph()];
    let output = switch::emit_switch(
        1,
        &desc().with_on(true),
        ControlState {
            disabled: true,
            ..ControlState::default()
        },
        &label,
        tokens,
    );

    assert_eq!(
        output.primitives[0].fill_token_or_uv,
        tokens.track_disabled.raw() as u32
    );
    assert_eq!(
        output.primitives[1].fill_token_or_uv,
        tokens.thumb_disabled.raw() as u32
    );
    let label_primitive = output
        .primitives
        .iter()
        .find(|primitive| primitive.prim_type() == PrimType::Glyph)
        .expect("the label is painted");
    assert_eq!(
        label_primitive.fill_token_or_uv,
        tokens.label_disabled.raw() as u32
    );
    assert!(output.hit.is_none());
}

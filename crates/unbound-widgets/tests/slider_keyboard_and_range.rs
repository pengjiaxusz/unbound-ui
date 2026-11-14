//! 滑块键盘步进、垂直几何布局与双滑块区间测试。

use unbound_math::{Rect, Vec2};
use unbound_widgets::slider::{
    emit_slider, reduce_slider_key, value_from_pointer, Orientation, SliderDesc, SliderKey,
    SliderTokens,
};
use unbound_widgets::ControlState;

fn desc() -> SliderDesc {
    SliderDesc::new(
        Rect::new(0.0, 0.0, 200.0, 32.0),
        "Density",
        50.0,
        0.0,
        100.0,
    )
    .with_step(5.0)
}

#[test]
fn the_arrow_keys_step_down_and_up() {
    let desc = desc();
    assert_eq!(reduce_slider_key(&desc, SliderKey::Left), 45.0);
    assert_eq!(reduce_slider_key(&desc, SliderKey::Down), 45.0);
    assert_eq!(reduce_slider_key(&desc, SliderKey::Right), 55.0);
    assert_eq!(reduce_slider_key(&desc, SliderKey::Up), 55.0);
}

#[test]
fn page_keys_move_ten_steps_and_home_end_jump_to_the_bounds() {
    let desc = desc();
    let top = SliderDesc::new(desc.rect, "Density", 90.0, 0.0, 100.0).with_step(5.0);
    assert_eq!(reduce_slider_key(&top, SliderKey::PageUp), 100.0);
    let bottom = SliderDesc::new(desc.rect, "Density", 10.0, 0.0, 100.0).with_step(5.0);
    assert_eq!(reduce_slider_key(&bottom, SliderKey::PageDown), 0.0);
    assert_eq!(reduce_slider_key(&desc, SliderKey::Home), 0.0);
    assert_eq!(reduce_slider_key(&desc, SliderKey::End), 100.0);
}

#[test]
fn a_continuous_slider_steps_by_one_percent_of_the_span() {
    let desc = SliderDesc::new(
        Rect::new(0.0, 0.0, 200.0, 32.0),
        "Density",
        50.0,
        0.0,
        100.0,
    );
    assert_eq!(reduce_slider_key(&desc, SliderKey::Left), 49.0);
    assert_eq!(reduce_slider_key(&desc, SliderKey::Right), 51.0);
}

#[test]
fn a_vertical_slider_runs_bottom_to_top() {
    let mut desc = desc().with_orientation(Orientation::Vertical);
    desc.rect = Rect::new(0.0, 0.0, 32.0, 200.0);
    let track = desc.track_rect();
    assert_eq!(track.origin.x, 14.0);
    assert_eq!(track.size.x, 4.0);
    assert_eq!(track.size.y, 200.0);

    desc.value = 0.0;
    assert_eq!(desc.thumb_rect().origin.y, 192.0);
    desc.value = 100.0;
    assert_eq!(desc.thumb_rect().origin.y, -8.0);
    desc.value = 50.0;
    assert_eq!(desc.thumb_rect().origin.y, 92.0);

    assert_eq!(value_from_pointer(Vec2::new(0.0, 0.0), &desc), 100.0);
    assert_eq!(value_from_pointer(Vec2::new(0.0, 100.0), &desc), 50.0);
    assert_eq!(value_from_pointer(Vec2::new(0.0, 200.0), &desc), 0.0);
}

#[test]
fn a_range_fills_between_the_two_thumbs() {
    let desc = SliderDesc::new(
        Rect::new(0.0, 0.0, 200.0, 32.0),
        "Density",
        75.0,
        0.0,
        100.0,
    )
    .with_step(5.0)
    .with_range_start(25.0);
    let fill = desc.fill_rect();
    assert_eq!(fill.origin.x, 50.0);
    assert_eq!(fill.size.x, 100.0);
    assert_eq!(
        desc.range_start_thumb_rect().map(|thumb| thumb.origin.x),
        Some(42.0)
    );
    assert_eq!(desc.thumb_rect().origin.x, 142.0);
}

#[test]
fn a_single_thumb_has_no_range_start() {
    assert!(desc().range_start_thumb_rect().is_none());
    assert_eq!(desc().fill_rect().origin.x, 0.0);
}

#[test]
fn ticks_and_the_tooltip_add_their_own_primitives() {
    let tokens = SliderTokens::standard();
    let plain = SliderDesc::new(desc().rect, "Density", 50.0, 0.0, 100.0)
        .with_step(25.0)
        .with_ticks(true);
    assert_eq!(plain.tick_ratios().len(), 5);
    let output = emit_slider(1, &plain, ControlState::default(), &[], tokens);
    // 轨道、填充、五个刻度、拇指、焦点环。
    assert_eq!(output.primitives.len(), 9);

    let with_tooltip = desc().with_value_tooltip(true);
    let tooltip = with_tooltip.tooltip_rect();
    assert_eq!(tooltip.size, Vec2::new(40.0, 20.0));
    assert!(
        tooltip.max().y <= with_tooltip.thumb_rect().origin.y,
        "the tooltip sits above the thumb"
    );
}

#[test]
fn a_disabled_slider_paints_the_disabled_slots() {
    let tokens = SliderTokens::standard();
    let output = emit_slider(
        1,
        &desc(),
        ControlState {
            disabled: true,
            ..ControlState::default()
        },
        &[],
        tokens,
    );
    assert_eq!(
        output.primitives[0].fill_token_or_uv,
        tokens.track_disabled.raw() as u32
    );
    assert_eq!(
        output.primitives[1].fill_token_or_uv,
        tokens.fill_disabled.raw() as u32
    );
    assert_eq!(
        output.primitives[2].fill_token_or_uv,
        tokens.thumb_disabled.raw() as u32
    );
    assert_eq!(
        output.primitives[2].border_token_or_tex,
        tokens.thumb_border_disabled.raw() as u32
    );
}

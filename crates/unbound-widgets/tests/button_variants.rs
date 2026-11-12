//! Size, icon, loading and keyboard-activation behaviour of the button.

use unbound_gpu::data::{GpuPrimitiveInstance, PrimFlags, PrimType, SubElement};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_widgets::button::{
    self, keyboard_activation, ButtonDesc, ButtonSize, ButtonVariant, BUTTON_LOADING_SIZE,
};
use unbound_widgets::image::IconDesc;
use unbound_widgets::{ControlState, PointerPhase};

fn desc() -> ButtonDesc {
    ButtonDesc::new(Rect::new(10.0, 10.0, 160.0, 36.0), "Save")
}

fn glyph() -> GlyphQuad {
    GlyphQuad {
        rect: Rect::new(80.0, 16.0, 8.0, 12.0),
        uv: [0.0, 0.0, 1.0, 1.0],
        layer: 0,
        color: false,
        msdf: false,
        phase: 0,
    }
}

fn sub_element(primitive: &GpuPrimitiveInstance) -> u8 {
    PrimFlags((primitive.prim_type_and_flags >> 16) as u16).sub_element()
}

fn background_fill(output: &unbound_widgets::WidgetOutput) -> u16 {
    output
        .primitives
        .iter()
        .find(|primitive| {
            primitive.prim_type() == PrimType::RoundedRect
                && sub_element(primitive) == SubElement::Background as u8
        })
        .map(|primitive| primitive.fill_token_or_uv as u16)
        .expect("a button always paints a background")
}

#[test]
fn sizes_map_to_the_standard_heights() {
    assert_eq!(ButtonSize::default(), ButtonSize::Md);
    assert_eq!(ButtonSize::Sm.height(), 28.0);
    assert_eq!(ButtonSize::Md.height(), 36.0);
    assert_eq!(ButtonSize::Lg.height(), 44.0);
    assert_eq!(desc().size, ButtonSize::Md);
    assert_eq!(desc().with_size(ButtonSize::Lg).size, ButtonSize::Lg);
}

#[test]
fn a_loading_button_refuses_clicks_and_paints_an_indicator() {
    let idle = emit(&desc());
    let loading = emit(&desc().with_loading(true));

    assert_eq!(loading.primitives.len(), idle.primitives.len() + 1);
    assert!(loading.hit.is_none(), "loading must not accept clicks");
    assert!(!loading.focusable);
    assert!(loading.a11y.as_ref().expect("still reachable").disabled);

    let indicator = loading
        .primitives
        .iter()
        .find(|primitive| {
            primitive.prim_type() == PrimType::Stroke
                && sub_element(primitive) == SubElement::Content as u8
        })
        .expect("the spinner must be a Content stroke");

    assert_eq!(indicator.size[0], BUTTON_LOADING_SIZE);
    assert_eq!(indicator.size[1], BUTTON_LOADING_SIZE);
    assert_eq!(indicator.local_pos[0], desc().rect.origin.x + 8.0);
}

#[test]
fn a_keyboard_press_paints_the_pressed_surface() {
    let tokens = ButtonVariant::Primary.tokens();
    let focused = ControlState {
        focused: true,
        ..ControlState::default()
    };

    let pressed = keyboard_activation(focused, true);
    assert_eq!(pressed.phase, PointerPhase::Pressed);

    let mut glyphs = Vec::new();
    let output = button::emit_button(1, &desc(), pressed, &mut glyphs);
    assert_eq!(background_fill(&output), tokens.fill_pressed.raw());
    assert_ne!(
        background_fill(&output),
        background_fill(&emit(&desc())),
        "a key press must not look idle"
    );

    // A key press without focus is not an activation.
    let unfocused = ControlState::default();
    assert_eq!(
        keyboard_activation(unfocused, true).phase,
        PointerPhase::Idle
    );
    // Releasing the key falls back to the router phase.
    let hovered = ControlState {
        phase: PointerPhase::Hovered,
        focused: true,
        ..ControlState::default()
    };
    assert_eq!(
        keyboard_activation(hovered, false).phase,
        PointerPhase::Hovered
    );
}

#[test]
fn an_icon_paints_its_atlas_layer() {
    let icon = IconDesc::new(Rect::ZERO, 7, Token::PRIMARY);
    let output = emit(&desc().with_icon(icon));

    let image = output
        .primitives
        .iter()
        .find(|primitive| primitive.prim_type() == PrimType::Image)
        .expect("an icon is a textured primitive");
    assert_eq!(image.border_token_or_tex, 7);
    assert!(
        !output.payloads.is_empty(),
        "the image needs its UV payload"
    );
}

#[test]
fn a_trailing_icon_sits_after_the_label() {
    let icon = IconDesc::new(Rect::ZERO, 3, Token::PRIMARY);
    let leading = emit_with_glyph(&desc().with_icon(icon.clone()));
    let trailing = emit_with_glyph(&desc().with_icon(icon).with_icon_trailing(true));

    let leading_image = image_x(&leading);
    let trailing_image = image_x(&trailing);
    let leading_text = glyph_x(&leading);
    let trailing_text = glyph_x(&trailing);

    assert!(leading_image < leading_text);
    assert!(trailing_image > trailing_text);
}

fn emit(desc: &ButtonDesc) -> unbound_widgets::WidgetOutput {
    let mut glyphs = Vec::new();
    button::emit_button(1, desc, ControlState::default(), &mut glyphs)
}

fn emit_with_glyph(desc: &ButtonDesc) -> unbound_widgets::WidgetOutput {
    let mut glyphs = vec![glyph()];
    button::emit_button(1, desc, ControlState::default(), &mut glyphs)
}

fn image_x(output: &unbound_widgets::WidgetOutput) -> f32 {
    output
        .primitives
        .iter()
        .find(|primitive| primitive.prim_type() == PrimType::Image)
        .expect("icon")
        .local_pos[0]
}

fn glyph_x(output: &unbound_widgets::WidgetOutput) -> f32 {
    output
        .primitives
        .iter()
        .find(|primitive| {
            matches!(
                primitive.prim_type(),
                PrimType::Glyph | PrimType::GlyphColor
            )
        })
        .expect("label")
        .local_pos[0]
}

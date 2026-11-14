//! 复选框的三态行为与矢量标记测试。

use accesskit::{Role, Toggled};
use unbound_gpu::data::{PrimFlags, PrimType, SubElement};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_widgets::checkbox::{self, CheckState, CheckboxDesc, CheckboxTokens};
use unbound_widgets::{ControlState, PointerPhase, WidgetOutput};

fn desc() -> CheckboxDesc {
    CheckboxDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Enable telemetry")
}

fn emit(desc: &CheckboxDesc, state: ControlState, label: &[GlyphQuad]) -> WidgetOutput {
    checkbox::emit_checkbox(1, desc, state, &[], label, CheckboxTokens::standard())
}

fn glyph() -> GlyphQuad {
    GlyphQuad {
        rect: Rect::new(30.0, 8.0, 8.0, 12.0),
        uv: [0.0, 0.0, 1.0, 1.0],
        layer: 0,
        color: false,
        msdf: false,
        phase: 0,
    }
}

fn sub_element(primitive: &unbound_gpu::data::GpuPrimitiveInstance) -> u8 {
    PrimFlags((primitive.prim_type_and_flags >> 16) as u16).sub_element()
}

#[test]
fn with_checked_keeps_the_boolean_api() {
    assert_eq!(desc().state, CheckState::Unchecked);
    assert_eq!(desc().with_checked(true).state, CheckState::Checked);
    assert_eq!(desc().with_checked(false).state, CheckState::Unchecked);
    let mixed = desc().with_state(CheckState::Indeterminate);
    assert!(mixed.state.is_indeterminate());
    assert!(!mixed.state.is_checked());
}

#[test]
fn an_unchecked_box_paints_no_mark() {
    let output = emit(&desc(), ControlState::default(), &[]);
    assert_eq!(output.primitives.len(), 2, "box plus the focus ring");
    assert!(
        !output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == PrimType::Curve),
        "an empty box has no mark geometry"
    );
}

#[test]
fn a_checked_box_paints_two_vector_strokes() {
    let output = emit(&desc().with_checked(true), ControlState::default(), &[]);
    let curves: Vec<_> = output
        .primitives
        .iter()
        .filter(|primitive| primitive.prim_type() == PrimType::Curve)
        .collect();
    assert_eq!(curves.len(), 2, "the check is two quadratic strokes");
    assert_eq!(
        output.payloads.len(),
        2,
        "each stroke carries its control points"
    );
    assert_eq!(output.primitives.len(), 4, "box, two strokes, ring");
    for curve in curves {
        assert_eq!(curve.fill_token_or_uv, Token::ON_PRIMARY.raw() as u32);
        assert_eq!(sub_element(curve), SubElement::Content as u8);
    }
}

#[test]
fn an_indeterminate_box_paints_a_dash_and_reports_mixed() {
    let output = emit(
        &desc().with_state(CheckState::Indeterminate),
        ControlState::default(),
        &[],
    );
    assert_eq!(output.primitives.len(), 3, "box, dash, ring");
    assert!(!output
        .primitives
        .iter()
        .any(|primitive| primitive.prim_type() == PrimType::Curve));

    let dash = output
        .primitives
        .iter()
        .find(|primitive| {
            primitive.prim_type() == PrimType::RoundedRect
                && primitive.fill_token_or_uv == Token::ON_PRIMARY.raw() as u32
        })
        .expect("the mixed state paints a rounded dash");
    assert!(
        dash.size[0] > dash.size[1],
        "a dash is wider than it is tall"
    );

    let a11y = output.a11y.expect("a checkbox is always reachable");
    assert_eq!(a11y.role, Role::CheckBox);
    assert!(!a11y.selected, "mixed is not checked");
    assert_eq!(a11y.value, "mixed");
    assert_eq!(CheckState::Indeterminate.toggled(), Toggled::Mixed);
    assert_eq!(CheckState::Checked.toggled(), Toggled::True);
    assert_eq!(CheckState::Unchecked.toggled(), Toggled::False);
}

#[test]
fn a_disabled_box_uses_the_disabled_tokens() {
    let tokens = CheckboxTokens::standard();
    let label = vec![glyph()];
    let disabled = ControlState {
        disabled: true,
        ..ControlState::default()
    };

    let unchecked = emit(&desc(), disabled, &label);
    let box_primitive = &unchecked.primitives[0];
    assert_eq!(
        box_primitive.fill_token_or_uv,
        tokens.box_disabled.raw() as u32
    );
    assert_eq!(
        box_primitive.border_token_or_tex,
        tokens.border_disabled.raw() as u32
    );
    let label_primitive = unchecked
        .primitives
        .iter()
        .find(|primitive| primitive.prim_type() == PrimType::Glyph)
        .expect("the label is painted");
    assert_eq!(
        label_primitive.fill_token_or_uv,
        tokens.label_disabled.raw() as u32
    );

    let checked = emit(&desc().with_checked(true), disabled, &[]);
    assert_eq!(
        checked.primitives[0].fill_token_or_uv,
        tokens.box_disabled.raw() as u32
    );
    let curve = checked
        .primitives
        .iter()
        .find(|primitive| primitive.prim_type() == PrimType::Curve)
        .expect("the mark is still emitted");
    assert_eq!(curve.fill_token_or_uv, tokens.mark_disabled.raw() as u32);
    assert!(checked.hit.is_none());
    assert!(!checked.focusable);
}

#[test]
fn the_checked_state_keeps_the_hover_feedback_of_an_empty_box() {
    let tokens = CheckboxTokens::standard();
    let hovered = emit(
        &desc(),
        ControlState {
            phase: PointerPhase::Hovered,
            ..ControlState::default()
        },
        &[],
    );
    assert_eq!(
        hovered.primitives[0].fill_token_or_uv,
        tokens.box_hover.raw() as u32
    );
}

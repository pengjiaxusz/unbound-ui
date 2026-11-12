//! Every standard control keeps a constant primitive count across states.
//!
//! The renderer can repair a *paint* change by patching the slices that
//! changed, but only while the scene's shape stays the same: a primitive that
//! appears or disappears renumbers every primitive after it, and the whole
//! pool has to be rewritten. Hovering a control is the most common thing a UI
//! does, so the controls promise the cheap path:
//!
//! > a control's state never changes how many primitives it emits.
//!
//! A state that needs a different appearance changes the *fields* of a
//! primitive — its colour slot, its border width, its opacity — never the
//! number of primitives.

use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_widgets::button::{self, ButtonDesc, ButtonVariant};
use unbound_widgets::checkbox::{self, CheckboxDesc};
use unbound_widgets::radio::{self, RadioDesc, RadioTokens};
use unbound_widgets::segmented::{self, SegmentedDesc, SegmentedTokens};
use unbound_widgets::slider::{self, SliderDesc, SliderTokens};
use unbound_widgets::switch::{self, SwitchDesc, SwitchTokens};
use unbound_widgets::{ControlState, PointerPhase};

/// The control's rectangle, comfortably above the minimum target size.
const RECT: Rect = Rect {
    origin: unbound_math::Vec2 { x: 40.0, y: 40.0 },
    size: unbound_math::Vec2 { x: 220.0, y: 36.0 },
};

/// A label long enough that the glyph run is not empty.
fn label() -> Vec<GlyphQuad> {
    (0..6)
        .map(|index| GlyphQuad {
            rect: Rect::new(
                RECT.origin.x + 60.0 + index as f32 * 8.0,
                RECT.origin.y + 10.0,
                7.0,
                13.0,
            ),
            uv: [0.0, 0.0, 1.0, 1.0],
            layer: 0,
            color: false,
            phase: 0,
        })
        .collect()
}

/// Every state a control can be in.
fn states() -> Vec<ControlState> {
    let mut states = Vec::new();
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

/// Every control, emitted into a list of `(name, primitives)`.
fn paint(state: ControlState) -> Vec<(&'static str, Vec<GpuPrimitiveInstance>)> {
    let label = label();
    let mut counts = Vec::new();

    let desc = ButtonDesc::new(RECT, "Save");
    for variant in [
        ButtonVariant::Primary,
        ButtonVariant::Secondary,
        ButtonVariant::Subtle,
        ButtonVariant::Icon,
        ButtonVariant::OnHero,
    ] {
        let mut label = label.clone();
        let mut desc = desc.clone();
        desc.variant = variant;
        let output = button::emit_button(1, &desc, state, &mut label);
        counts.push((variant_name(variant), output.primitives));
    }

    let output = switch::emit_switch(
        2,
        &SwitchDesc::new(RECT, "Animate"),
        state,
        &label,
        SwitchTokens::standard(),
    );
    counts.push(("switch", output.primitives));

    let desc = CheckboxDesc::new(RECT, "Snap to grid");
    let output = checkbox::emit_checkbox(
        3,
        &desc,
        state,
        &label[..2],
        &label,
        checkbox::CheckboxTokens::standard(),
    );
    counts.push(("checkbox", output.primitives));

    let desc = RadioDesc::new(RECT, "Comfortable");
    let output = radio::emit_radio(4, &desc, state, &label, RadioTokens::standard());
    counts.push(("radio", output.primitives));

    let desc = SliderDesc::new(RECT, "Gap", 18.0, 0.0, 36.0).with_step(1.0);
    let output = slider::emit_slider(5, &desc, state, &label, SliderTokens::standard());
    counts.push(("slider", output.primitives));

    let desc = SegmentedDesc::new(RECT, &["One", "Two", "Three"]);
    let output = segmented::emit_segmented_segment(
        6,
        &desc,
        1,
        state,
        &label[..3],
        SegmentedTokens::standard(),
    );
    counts.push(("segmented/idle", output.primitives));
    // The active slot paints a different pill than an idle one, so it has to
    // be probed separately: it is the state most likely to add a primitive.
    let desc = SegmentedDesc::new(RECT, &["One", "Two", "Three"]).with_selected(1);
    let output = segmented::emit_segmented_segment(
        7,
        &desc,
        1,
        state,
        &label[..3],
        SegmentedTokens::standard(),
    );
    counts.push(("segmented/active", output.primitives));

    counts
}

fn variant_name(variant: ButtonVariant) -> &'static str {
    match variant {
        ButtonVariant::Primary => "button/primary",
        ButtonVariant::Secondary => "button/secondary",
        ButtonVariant::Subtle => "button/subtle",
        ButtonVariant::Icon => "button/icon",
        ButtonVariant::OnHero => "button/on-hero",
    }
}

#[test]
fn no_control_changes_its_primitive_count_between_states() {
    let baseline = paint(ControlState::default());
    let mut failures = Vec::new();
    for state in states() {
        for ((name, primitives), (baseline_name, baseline_primitives)) in
            paint(state).into_iter().zip(baseline.iter())
        {
            debug_assert_eq!(name, *baseline_name);
            if primitives.len() != baseline_primitives.len() {
                failures.push(format!(
                    "{name} emits {} primitives in {state:?}, but {} when idle",
                    primitives.len(),
                    baseline_primitives.len()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} control(s) changed shape with their state:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The probe must not be vacuous: a hover has to change *something*.
///
/// If every state painted identically the shape test above would pass while
/// proving nothing, so this one states the other half of the contract: a
/// state change is a change of fields, which is exactly the patch the
/// renderer can apply.
#[test]
fn every_control_paints_differently_when_hovered() {
    // The active slot of a segmented control is deliberately inert: it *is*
    // the selection, and brightening it under the pointer would suggest a
    // state it does not have. Every other control must answer the pointer,
    // otherwise it looks dead until it is clicked.
    const INERT_BY_DESIGN: &[&str] = &["segmented/active"];

    let idle = paint(ControlState {
        phase: PointerPhase::Idle,
        ..ControlState::default()
    });
    let hovered = paint(ControlState {
        phase: PointerPhase::Hovered,
        ..ControlState::default()
    });
    let mut same = Vec::new();
    for ((name, idle), (_hovered_name, hovered)) in idle.iter().zip(hovered.iter()) {
        if idle == hovered && !INERT_BY_DESIGN.contains(name) {
            same.push(*name);
        }
    }
    assert!(
        same.is_empty(),
        "these controls paint the same idle and hovered: {same:?}"
    );
}

#[test]
fn the_probe_covers_every_control() {
    let probe = paint(ControlState::default());
    assert_eq!(
        probe.len(),
        11,
        "the probe must cover every emitter: {:?}",
        probe.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    );
}

//! The slider: a track, a filled part and a thumb.
//!
//! The slider is the only control with a *continuous* value, so it is also
//! the only one that needs a mapping from a pointer position to a number.
//! That mapping lives in [`value_from_pointer`] and nowhere else: the
//! application asks for a value, the control paints one.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default track height.
pub const SLIDER_TRACK: f32 = 4.0;

/// The default thumb size.
pub const SLIDER_THUMB: f32 = 16.0;

/// The colour slots a slider paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliderTokens {
    /// The track behind the thumb.
    pub track: Token,
    /// The filled part of the track.
    pub fill: Token,
    /// The thumb.
    pub thumb: Token,
    /// The border of the thumb.
    pub thumb_border: Token,
    /// The label.
    pub label: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl SliderTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            track: Token::CONTROL_TRACK,
            fill: Token::PRIMARY,
            thumb: Token::CONTROL_THUMB,
            thumb_border: Token::OUTLINE,
            label: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// The pure data descriptor of a slider.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderDesc {
    /// The row the slider occupies.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The current value.
    pub value: f32,
    /// The lowest value.
    pub min: f32,
    /// The highest value.
    pub max: f32,
    /// The step size; zero means continuous.
    pub step: f32,
    /// The track height.
    pub track_height: f32,
    /// The thumb size.
    pub thumb_size: f32,
    /// The draw order key.
    pub z: u16,
}

impl SliderDesc {
    /// A slider with the default metrics.
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>, value: f32, min: f32, max: f32) -> Self {
        Self {
            rect,
            label: label.into(),
            value: value.clamp(min.min(max), max.max(min)),
            min,
            max,
            step: 0.0,
            track_height: SLIDER_TRACK,
            thumb_size: SLIDER_THUMB,
            z: 700,
        }
    }

    /// The same slider with a step size.
    #[must_use]
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// The value normalised to `[0, 1]`.
    #[must_use]
    pub fn ratio(&self) -> f32 {
        let span = self.max - self.min;
        if span.abs() < f32::EPSILON {
            0.0
        } else {
            ((self.value - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// The track rectangle: full width, vertically centred in the row.
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + self.rect.size.y * 0.5 - self.track_height * 0.5,
            self.rect.size.x,
            self.track_height,
        )
    }

    /// The filled part of the track.
    #[must_use]
    pub fn fill_rect(&self) -> Rect {
        let track = self.track_rect();
        Rect::new(
            track.origin.x,
            track.origin.y,
            track.size.x * self.ratio(),
            track.size.y,
        )
    }

    /// The thumb rectangle.
    #[must_use]
    pub fn thumb_rect(&self) -> Rect {
        let track = self.track_rect();
        let half = self.thumb_size * 0.5;
        Rect::new(
            track.origin.x + track.size.x * self.ratio() - half,
            track.origin.y + track.size.y * 0.5 - half,
            self.thumb_size,
            self.thumb_size,
        )
    }
}

/// The value a pointer position corresponds to.
///
/// The pointer is clamped to the track, the raw value is snapped to the step
/// (when there is one) and the result is clamped to `[min, max]`. The step is
/// applied *relative to `min`*, so a step of `0.5` starting at `10` produces
/// `10.0, 10.5, 11.0 …` instead of drifting.
#[must_use]
pub fn value_from_pointer(pointer: Vec2, desc: &SliderDesc) -> f32 {
    let track = desc.track_rect();
    let ratio = if track.size.x.abs() < f32::EPSILON {
        0.0
    } else {
        ((pointer.x - track.origin.x) / track.size.x).clamp(0.0, 1.0)
    };
    let raw = desc.min + ratio * (desc.max - desc.min);
    let stepped = if desc.step > 0.0 {
        ((raw - desc.min) / desc.step).round() * desc.step + desc.min
    } else {
        raw
    };
    stepped.clamp(desc.min.min(desc.max), desc.max.max(desc.min))
}

/// Whether the slider is currently being dragged.
///
/// The router captures the pointer on press, so the answer stays true even
/// when the pointer leaves the track — exactly like a native slider.
#[must_use]
pub fn is_dragging(router: &unbound_input::Router, node: u64) -> bool {
    router.pointer().pressed() == Some(node)
}

/// Emit the slider: track, filled part, thumb and the focus ring.
#[must_use]
pub fn emit_slider(
    node: u64,
    desc: &SliderDesc,
    state: ControlState,
    label: &[GlyphQuad],
    tokens: SliderTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;

    primitives.push(surface(
        desc.track_rect(),
        desc.track_height * 0.5,
        tokens.track,
        z,
    ));
    primitives.push(surface(
        desc.fill_rect(),
        desc.track_height * 0.5,
        tokens.fill,
        z,
    ));

    let thumb = desc.thumb_rect();
    primitives.push(outlined(
        thumb,
        desc.thumb_size * 0.5,
        tokens.thumb,
        tokens.thumb_border,
        1.0,
        if state.phase == PointerPhase::Pressed {
            8.0
        } else {
            4.0
        },
        z,
    ));

    glyphs(label, tokens.label, z, &mut primitives, &mut payloads);

    if state.focused {
        primitives.push(focus_ring(
            thumb.inflate(3.0),
            desc.thumb_size * 0.5 + 3.0,
            tokens.focus_ring,
            2.0,
            z,
        ));
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded {
                radii: [desc.thumb_size * 0.5; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Slider, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(state.disabled);
    a11y.value = format!("{}", desc.value);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

/// The state of a slider that is being dragged.
#[must_use]
pub fn pressed_state(dragging: bool) -> ControlState {
    ControlState {
        phase: if dragging {
            PointerPhase::Pressed
        } else {
            PointerPhase::Idle
        },
        ..ControlState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> SliderDesc {
        SliderDesc::new(
            Rect::new(0.0, 0.0, 200.0, 32.0),
            "Density",
            50.0,
            0.0,
            100.0,
        )
    }

    #[test]
    fn the_thumb_follows_the_value() {
        let mut desc = desc();
        desc.value = 0.0;
        assert_eq!(desc.thumb_rect().origin.x, -8.0);
        desc.value = 100.0;
        assert_eq!(desc.thumb_rect().origin.x, 192.0);
        desc.value = 50.0;
        assert_eq!(desc.thumb_rect().origin.x, 92.0);
    }

    #[test]
    fn the_pointer_maps_onto_the_value_range() {
        let desc = desc();
        assert_eq!(value_from_pointer(Vec2::new(0.0, 0.0), &desc), 0.0);
        assert_eq!(value_from_pointer(Vec2::new(100.0, 0.0), &desc), 50.0);
        assert_eq!(value_from_pointer(Vec2::new(200.0, 0.0), &desc), 100.0);
    }

    #[test]
    fn the_pointer_is_clamped_to_the_track() {
        let desc = desc();
        assert_eq!(value_from_pointer(Vec2::new(-500.0, 0.0), &desc), 0.0);
        assert_eq!(value_from_pointer(Vec2::new(500.0, 0.0), &desc), 100.0);
    }

    #[test]
    fn a_step_snaps_relative_to_the_minimum() {
        let desc = SliderDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Size", 0.0, 10.0, 20.0)
            .with_step(0.5);
        assert_eq!(value_from_pointer(Vec2::new(50.0, 0.0), &desc), 12.5);
        assert_eq!(value_from_pointer(Vec2::new(52.0, 0.0), &desc), 12.5);
        assert_eq!(value_from_pointer(Vec2::new(56.0, 0.0), &desc), 13.0);
    }

    #[test]
    fn a_stepped_slider_never_leaves_the_range() {
        let desc = SliderDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Size", 0.0, 10.0, 20.0)
            .with_step(3.0);
        let value = value_from_pointer(Vec2::new(200.0, 0.0), &desc);
        assert!(value <= 20.0, "{value} 超出了上界");
        assert!(value >= 10.0);
    }

    #[test]
    fn the_fill_grows_with_the_value() {
        let mut desc = desc();
        desc.value = 25.0;
        assert_eq!(desc.fill_rect().size.x, 50.0);
        desc.value = 75.0;
        assert_eq!(desc.fill_rect().size.x, 150.0);
    }

    #[test]
    fn the_semantic_node_carries_the_value() {
        let tokens = SliderTokens::standard();
        let output = emit_slider(1, &desc(), ControlState::default(), &[], tokens);
        let a11y = output.a11y.expect("a slider is always reachable");
        assert_eq!(a11y.role, Role::Slider);
        assert_eq!(a11y.value, "50");
    }

    #[test]
    fn a_disabled_slider_leaves_the_hit_set() {
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
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }

    #[test]
    fn a_focused_slider_emits_a_focus_ring() {
        let tokens = SliderTokens::standard();
        let output = emit_slider(
            1,
            &desc(),
            ControlState {
                focused: true,
                ..ControlState::default()
            },
            &[],
            tokens,
        );
        assert!(output
            .primitives
            .iter()
            .any(|primitive| primitive.prim_type() == unbound_gpu::data::PrimType::Stroke));
    }
}

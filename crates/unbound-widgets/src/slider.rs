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

/// The direction a slider runs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// The track runs left to right.
    #[default]
    Horizontal,
    /// The track runs bottom to top.
    Vertical,
}

/// A key the slider understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderKey {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// The colour slots a slider paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliderTokens {
    /// The track behind the thumb.
    pub track: Token,
    /// The filled part of the track.
    pub fill: Token,
    /// The filled part while the pointer is over the slider.
    ///
    /// A slider has to answer the pointer before it is grabbed, otherwise the
    /// track looks inert until the drag starts. The state only changes which
    /// colour slot the fill resolves to, so the hover is one patched
    /// primitive, not a new one.
    pub fill_hover: Token,
    /// The filled part while the slider is being dragged.
    pub fill_pressed: Token,
    /// The thumb.
    pub thumb: Token,
    /// The border of the thumb.
    pub thumb_border: Token,
    /// The track of a disabled slider.
    pub track_disabled: Token,
    /// The filled part of a disabled slider.
    pub fill_disabled: Token,
    /// The thumb of a disabled slider.
    pub thumb_disabled: Token,
    /// The border of a disabled thumb.
    pub thumb_border_disabled: Token,
    /// The label.
    pub label: Token,
    /// The label of a disabled slider.
    pub label_disabled: Token,
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
            fill_hover: Token::PRIMARY_HOVER,
            fill_pressed: Token::PRIMARY_PRESSED,
            thumb: Token::CONTROL_THUMB,
            thumb_border: Token::OUTLINE,
            track_disabled: Token::CONTROL_DISABLED_BG,
            fill_disabled: Token::CONTROL_DISABLED_BG,
            thumb_disabled: Token::CONTROL_DISABLED_THUMB,
            thumb_border_disabled: Token::CONTROL_DISABLED_BORDER,
            label: Token::TEXT_SECONDARY,
            label_disabled: Token::TEXT_DISABLED,
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
    /// The track height (the track width when vertical).
    pub track_height: f32,
    /// The thumb size.
    pub thumb_size: f32,
    /// Whether the track runs horizontally or vertically.
    pub orientation: Orientation,
    /// The low end of a dual-thumb range, if any.
    pub range_start: Option<f32>,
    /// Whether the track paints a tick at every step.
    pub show_ticks: bool,
    /// Whether the value tooltip follows the thumb.
    pub show_value_tooltip: bool,
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
            orientation: Orientation::Horizontal,
            range_start: None,
            show_ticks: false,
            show_value_tooltip: false,
            z: 700,
        }
    }

    /// The same slider with a step size.
    #[must_use]
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// The same slider running in `orientation`.
    #[must_use]
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// The same slider with a second, lower thumb.
    #[must_use]
    pub fn with_range_start(mut self, range_start: f32) -> Self {
        self.range_start = Some(range_start);
        self
    }

    /// The same slider painting a tick at every step.
    #[must_use]
    pub fn with_ticks(mut self, show_ticks: bool) -> Self {
        self.show_ticks = show_ticks;
        self
    }

    /// The same slider with a value tooltip.
    #[must_use]
    pub fn with_value_tooltip(mut self, show_value_tooltip: bool) -> Self {
        self.show_value_tooltip = show_value_tooltip;
        self
    }

    /// A value normalised to `[0, 1]`.
    #[must_use]
    pub fn ratio_of(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span.abs() < f32::EPSILON {
            0.0
        } else {
            ((value - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// The current value normalised to `[0, 1]`.
    #[must_use]
    pub fn ratio(&self) -> f32 {
        self.ratio_of(self.value)
    }

    /// The low end of the dual-thumb range, clamped to `[min, value]`.
    #[must_use]
    pub fn range_start_ratio(&self) -> Option<f32> {
        self.range_start
            .map(|start| self.ratio_of(start.min(self.value)))
    }

    /// The track rectangle, centred in the row.
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                self.rect.origin.x,
                self.rect.origin.y + self.rect.size.y * 0.5 - self.track_height * 0.5,
                self.rect.size.x,
                self.track_height,
            ),
            Orientation::Vertical => Rect::new(
                self.rect.origin.x + self.rect.size.x * 0.5 - self.track_height * 0.5,
                self.rect.origin.y,
                self.track_height,
                self.rect.size.y,
            ),
        }
    }

    /// The filled part of the track.
    ///
    /// With a range start the fill spans both thumbs; otherwise it runs from
    /// the track's origin to the value.
    #[must_use]
    pub fn fill_rect(&self) -> Rect {
        let track = self.track_rect();
        let high = self.ratio();
        let low = self
            .range_start_ratio()
            .map_or(0.0, |start| start.min(high));
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                track.origin.x + track.size.x * low,
                track.origin.y,
                track.size.x * (high - low),
                track.size.y,
            ),
            Orientation::Vertical => Rect::new(
                track.origin.x,
                track.origin.y + track.size.y * (1.0 - high),
                track.size.x,
                track.size.y * (high - low),
            ),
        }
    }

    /// The rectangle of the thumb at `ratio`.
    #[must_use]
    pub fn thumb_rect_at(&self, ratio: f32) -> Rect {
        let track = self.track_rect();
        let half = self.thumb_size * 0.5;
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                track.origin.x + track.size.x * ratio - half,
                track.origin.y + track.size.y * 0.5 - half,
                self.thumb_size,
                self.thumb_size,
            ),
            Orientation::Vertical => Rect::new(
                track.origin.x + track.size.x * 0.5 - half,
                track.origin.y + track.size.y * (1.0 - ratio) - half,
                self.thumb_size,
                self.thumb_size,
            ),
        }
    }

    /// The thumb rectangle.
    #[must_use]
    pub fn thumb_rect(&self) -> Rect {
        self.thumb_rect_at(self.ratio())
    }

    /// The low thumb of a dual-thumb range.
    #[must_use]
    pub fn range_start_thumb_rect(&self) -> Option<Rect> {
        self.range_start_ratio()
            .map(|ratio| self.thumb_rect_at(ratio))
    }

    /// The ratios the tick marks sit at.
    ///
    /// A stepped slider ticks at every step (capped at 64 marks); a continuous
    /// one ticks at the quarter points.
    #[must_use]
    pub fn tick_ratios(&self) -> Vec<f32> {
        let span = (self.max - self.min).abs();
        if span < f32::EPSILON {
            return Vec::new();
        }
        if self.step > 0.0 {
            let count = (span / self.step).round() as usize;
            if (1..=64).contains(&count) {
                return (0..=count)
                    .map(|index| index as f32 / count as f32)
                    .collect();
            }
        }
        vec![0.25, 0.5, 0.75]
    }

    /// A tick mark at `ratio`.
    #[must_use]
    pub fn tick_rect(&self, ratio: f32) -> Rect {
        let track = self.track_rect();
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                track.origin.x + track.size.x * ratio - 1.0,
                track.origin.y + track.size.y * 0.5 - 4.0,
                2.0,
                8.0,
            ),
            Orientation::Vertical => Rect::new(
                track.origin.x + track.size.x * 0.5 - 4.0,
                track.origin.y + track.size.y * (1.0 - ratio) - 1.0,
                8.0,
                2.0,
            ),
        }
    }

    /// The tooltip bubble above (or beside) the thumb.
    #[must_use]
    pub fn tooltip_rect(&self) -> Rect {
        let thumb = self.thumb_rect();
        match self.orientation {
            Orientation::Horizontal => Rect::new(
                thumb.origin.x + thumb.size.x * 0.5 - 20.0,
                thumb.origin.y - 26.0,
                40.0,
                20.0,
            ),
            Orientation::Vertical => Rect::new(
                thumb.max().x + 8.0,
                thumb.origin.y + thumb.size.y * 0.5 - 10.0,
                40.0,
                20.0,
            ),
        }
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
    let ratio = match desc.orientation {
        Orientation::Horizontal => {
            if track.size.x.abs() < f32::EPSILON {
                0.0
            } else {
                ((pointer.x - track.origin.x) / track.size.x).clamp(0.0, 1.0)
            }
        }
        Orientation::Vertical => {
            if track.size.y.abs() < f32::EPSILON {
                0.0
            } else {
                (1.0 - (pointer.y - track.origin.y) / track.size.y).clamp(0.0, 1.0)
            }
        }
    };
    let raw = desc.min + ratio * (desc.max - desc.min);
    let stepped = if desc.step > 0.0 {
        ((raw - desc.min) / desc.step).round() * desc.step + desc.min
    } else {
        raw
    };
    stepped.clamp(desc.min.min(desc.max), desc.max.max(desc.min))
}

/// The value one keyboard event moves the slider to.
///
/// Left/Down decrease, Right/Up increase, PageUp/PageDown move by ten steps
/// and Home/End jump to the ends. A continuous slider moves by one percent of
/// its span.
#[must_use]
pub fn reduce_slider_key(desc: &SliderDesc, key: SliderKey) -> f32 {
    let span = desc.max - desc.min;
    let step = if desc.step > 0.0 {
        desc.step
    } else {
        span.abs() * 0.01
    };
    let value = match key {
        SliderKey::Left | SliderKey::Down => desc.value - step,
        SliderKey::Right | SliderKey::Up => desc.value + step,
        SliderKey::PageDown => desc.value - step * 10.0,
        SliderKey::PageUp => desc.value + step * 10.0,
        SliderKey::Home => desc.min.min(desc.max),
        SliderKey::End => desc.max.max(desc.min),
    };
    value.clamp(desc.min.min(desc.max), desc.max.max(desc.min))
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
    let mut primitives = Vec::with_capacity(8 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;

    primitives.push(surface(
        desc.track_rect(),
        desc.track_height * 0.5,
        if state.disabled {
            tokens.track_disabled
        } else {
            tokens.track
        },
        z,
    ));
    primitives.push(surface(
        desc.fill_rect(),
        desc.track_height * 0.5,
        fill_slot(state, tokens),
        z,
    ));

    if desc.show_ticks {
        for ratio in desc.tick_ratios() {
            primitives.push(surface(
                desc.tick_rect(ratio),
                1.0,
                if state.disabled {
                    tokens.label_disabled
                } else {
                    tokens.label
                },
                z,
            ));
        }
    }

    let thumb_fill = if state.disabled {
        tokens.thumb_disabled
    } else {
        tokens.thumb
    };
    let thumb_border = if state.disabled {
        tokens.thumb_border_disabled
    } else {
        tokens.thumb_border
    };

    if let Some(start) = desc.range_start_thumb_rect() {
        primitives.push(outlined(
            start,
            desc.thumb_size * 0.5,
            thumb_fill,
            thumb_border,
            1.0,
            4.0,
            z,
        ));
    }

    let thumb = desc.thumb_rect();
    primitives.push(outlined(
        thumb,
        desc.thumb_size * 0.5,
        thumb_fill,
        thumb_border,
        1.0,
        if state.phase == PointerPhase::Pressed {
            8.0
        } else {
            4.0
        },
        z,
    ));

    if desc.show_value_tooltip {
        primitives.push(outlined(
            desc.tooltip_rect(),
            6.0,
            thumb_fill,
            thumb_border,
            1.0,
            4.0,
            z,
        ));
    }

    glyphs(
        label,
        if state.disabled {
            tokens.label_disabled
        } else {
            tokens.label
        },
        z,
        &mut primitives,
        &mut payloads,
    );

    // The ring is always emitted: focus changes its opacity, not the
    // pool's shape, so focus is a paint change and never a re-upload.
    primitives.push(focus_ring(
        thumb.inflate(3.0),
        desc.thumb_size * 0.5 + 3.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

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

/// The colour slot the filled track paints with.
fn fill_slot(state: ControlState, tokens: SliderTokens) -> Token {
    if state.disabled {
        return tokens.fill_disabled;
    }
    match state.phase {
        PointerPhase::Idle => tokens.fill,
        PointerPhase::Hovered => tokens.fill_hover,
        PointerPhase::Pressed => tokens.fill_pressed,
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

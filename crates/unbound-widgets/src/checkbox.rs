//! The checkbox: a square that carries a check mark.

use accesskit::{Role, Toggled};
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// The default box size.
pub const CHECKBOX_BOX: f32 = 18.0;

/// The gap between the box and the label.
pub const CHECKBOX_GAP: f32 = 28.0;

/// The three positions of a checkbox.
///
/// `Indeterminate` is the "some of my children are checked" state: a tri-state
/// list uses it, and it must never be confused with "checked".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    /// The box is empty.
    #[default]
    Unchecked,
    /// The box carries a check mark.
    Checked,
    /// The box carries a dash.
    Indeterminate,
}

impl CheckState {
    /// Whether the checkbox is fully checked.
    #[must_use]
    pub const fn is_checked(self) -> bool {
        matches!(self, Self::Checked)
    }

    /// Whether the checkbox is in the mixed state.
    #[must_use]
    pub const fn is_indeterminate(self) -> bool {
        matches!(self, Self::Indeterminate)
    }

    /// The AccessKit `Toggled` value this state maps to.
    ///
    /// `Indeterminate` must reach assistive technology as `Mixed`, not as a
    /// second "off": a screen reader has to announce a tri-state value.
    #[must_use]
    pub const fn toggled(self) -> Toggled {
        match self {
            Self::Unchecked => Toggled::False,
            Self::Checked => Toggled::True,
            Self::Indeterminate => Toggled::Mixed,
        }
    }
}

/// The colour slots a checkbox paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckboxTokens {
    /// The box while unchecked.
    pub box_empty: Token,
    /// The box while hovered.
    pub box_hover: Token,
    /// The box while checked.
    pub box_checked: Token,
    /// The box while disabled.
    pub box_disabled: Token,
    /// The border of an unchecked box.
    pub border: Token,
    /// The border of a disabled, unchecked box.
    pub border_disabled: Token,
    /// The check mark.
    pub mark: Token,
    /// The check mark while disabled.
    pub mark_disabled: Token,
    /// The label.
    pub label: Token,
    /// The label while disabled.
    pub label_disabled: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl CheckboxTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            box_empty: Token::CARD_BG,
            box_hover: Token::CONTROL_HOVER,
            box_checked: Token::PRIMARY,
            box_disabled: Token::CONTROL_DISABLED_BG,
            border: Token::OUTLINE,
            border_disabled: Token::CONTROL_DISABLED_BORDER,
            mark: Token::ON_PRIMARY,
            mark_disabled: Token::CONTROL_DISABLED_THUMB,
            label: Token::TEXT_PRIMARY,
            label_disabled: Token::TEXT_DISABLED,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// The pure data descriptor of a checkbox.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckboxDesc {
    /// The row the checkbox occupies.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The position of the box.
    pub state: CheckState,
    /// The box size.
    pub box_size: f32,
    /// The distance between the box and the label.
    pub gap: f32,
    /// The draw order key.
    pub z: u16,
}

impl CheckboxDesc {
    /// A checkbox with the default metrics.
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            state: CheckState::Unchecked,
            box_size: CHECKBOX_BOX,
            gap: CHECKBOX_GAP,
            z: 700,
        }
    }

    /// The same checkbox in another position.
    ///
    /// Kept as the boolean shortcut so existing callers compile unchanged;
    /// tri-state callers use [`Self::with_state`].
    #[must_use]
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.state = if checked {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        };
        self
    }

    /// The same checkbox in an explicit position.
    #[must_use]
    pub fn with_state(mut self, state: CheckState) -> Self {
        self.state = state;
        self
    }

    /// The box rectangle: left-aligned, vertically centred in the row.
    #[must_use]
    pub fn box_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.y - self.box_size) * 0.5,
            self.box_size,
            self.box_size,
        )
    }

    /// The label rectangle.
    #[must_use]
    pub fn label_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + self.gap,
            self.rect.origin.y,
            (self.rect.size.x - self.gap).max(0.0),
            self.rect.size.y,
        )
    }
}

/// Emit the checkbox: box, check mark, label and the focus ring.
///
/// `mark` is kept for source compatibility; the mark is now drawn as vector
/// primitives from [`CheckboxDesc::state`], so the glyph run is ignored.
#[must_use]
pub fn emit_checkbox(
    node: u64,
    desc: &CheckboxDesc,
    state: ControlState,
    _mark: &[GlyphQuad],
    label: &[GlyphQuad],
    tokens: CheckboxTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label.len());
    let mut payloads = Vec::with_capacity(label.len());
    let z = desc.z;
    let box_rect = desc.box_rect();
    let disabled = state.disabled;

    let fill = if disabled {
        tokens.box_disabled
    } else {
        match desc.state {
            CheckState::Checked => tokens.box_checked,
            CheckState::Indeterminate => Token::INDETERMINATE_FILL,
            CheckState::Unchecked => {
                if state.phase == PointerPhase::Hovered {
                    tokens.box_hover
                } else {
                    tokens.box_empty
                }
            }
        }
    };
    let border = if desc.state == CheckState::Unchecked {
        if disabled {
            tokens.border_disabled
        } else {
            tokens.border
        }
    } else {
        Token::NONE
    };
    primitives.push(outlined(box_rect, 4.0, fill, border, 1.0, 0.0, z));

    // The mark is geometry, not a glyph: a font without `✓` still paints a
    // correct checkbox, and the stroke thickness scales with the box.
    let mark = if disabled {
        tokens.mark_disabled
    } else {
        tokens.mark
    };
    match desc.state {
        CheckState::Checked => check_mark(box_rect, mark, z, &mut primitives, &mut payloads),
        CheckState::Indeterminate => indeterminate_dash(box_rect, mark, z, &mut primitives),
        CheckState::Unchecked => {}
    }

    let label_color = if disabled {
        tokens.label_disabled
    } else {
        tokens.label
    };
    glyphs(label, label_color, z, &mut primitives, &mut payloads);

    // The ring is always emitted: focus changes its opacity, not the
    // pool's shape, so focus is a paint change and never a re-upload.
    primitives.push(focus_ring(
        box_rect.inflate(3.0),
        7.0,
        tokens.focus_ring,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), desc.rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::CheckBox, desc.label.clone(), desc.rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = desc.state.is_checked();
    if desc.state.is_indeterminate() {
        // `A11yNode` carries the toggled value through `value`; the numeric
        // mapping itself is exposed by [`CheckState::toggled`].
        a11y.value = "mixed".to_owned();
    }

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
    }
}

/// Two quadratic strokes forming a check mark.
fn check_mark(
    box_rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let x = box_rect.origin.x;
    let y = box_rect.origin.y;
    let w = box_rect.size.x;
    let h = box_rect.size.y;
    let width = (w * 0.16).max(1.5);
    let start = [x + w * 0.20, y + h * 0.52];
    let elbow = [x + w * 0.42, y + h * 0.72];
    let tip = [x + w * 0.79, y + h * 0.28];
    curve_stroke(
        [start, [x + w * 0.27, y + h * 0.63], elbow],
        width,
        color,
        z,
        primitives,
        payloads,
    );
    curve_stroke(
        [elbow, [x + w * 0.60, y + h * 0.78], tip],
        width,
        color,
        z,
        primitives,
        payloads,
    );
}

/// A single quadratic stroke, packed into its tight local box.
fn curve_stroke(
    points: [[f32; 2]; 3],
    width: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let min_x = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let min_y = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let max_x = points
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = points
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let local = points.map(|point| [point[0] - min_x, point[1] - min_y]);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(local[0], local[1], local[2], width));
    let mut primitive = instance(
        Rect::new(
            min_x,
            min_y,
            (max_x - min_x).max(0.5),
            (max_y - min_y).max(0.5),
        ),
        [0.0; 4],
        color,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Curve,
        SubElement::Content,
        z,
    );
    primitive.extra_payload_idx = payload_idx;
    primitives.push(primitive);
}

/// A rounded dash, the mixed-state mark.
fn indeterminate_dash(
    box_rect: Rect,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
) {
    let w = box_rect.size.x;
    let h = box_rect.size.y;
    let dash = Rect::new(
        box_rect.origin.x + w * 0.22,
        box_rect.origin.y + h * 0.5 - h * 0.07,
        w * 0.56,
        (h * 0.14).max(2.0),
    );
    primitives.push(outlined(
        dash,
        dash.size.y * 0.5,
        color,
        Token::NONE,
        0.0,
        0.0,
        z,
    ));
}

impl CheckboxDesc {
    /// The mixed-state accessibility value.
    #[must_use]
    pub const fn toggled(&self) -> Toggled {
        self.state.toggled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CheckboxDesc {
        CheckboxDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), "Enable telemetry")
    }

    #[test]
    fn the_box_is_left_aligned_and_centred() {
        let desc = desc();
        let rect = desc.box_rect();
        assert_eq!(rect.origin.x, 0.0);
        assert_eq!(rect.origin.y, 7.0);
        assert_eq!(rect.size.x, 18.0);
    }

    #[test]
    fn an_unchecked_box_keeps_its_border() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(1, &desc(), ControlState::default(), &[], &[], tokens);
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_empty.raw() as u32
        );
        assert_eq!(
            output.primitives[0].border_token_or_tex,
            tokens.border.raw() as u32
        );
    }

    #[test]
    fn a_checked_box_fills_and_drops_the_border() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc().with_checked(true),
            ControlState::default(),
            &[],
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_checked.raw() as u32
        );
        assert_eq!(output.primitives[0].border_token_or_tex, 0);
    }

    #[test]
    fn a_hovered_box_changes_its_fill() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &[],
            &[],
            tokens,
        );
        assert_eq!(
            output.primitives[0].fill_token_or_uv,
            tokens.box_hover.raw() as u32
        );
    }

    #[test]
    fn the_semantic_node_reports_the_checked_state() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc().with_checked(true),
            ControlState::default(),
            &[],
            &[],
            tokens,
        );
        let a11y = output.a11y.expect("a checkbox is always reachable");
        assert_eq!(a11y.role, Role::CheckBox);
        assert!(a11y.selected);
    }

    #[test]
    fn a_disabled_checkbox_leaves_the_hit_set() {
        let tokens = CheckboxTokens::standard();
        let output = emit_checkbox(
            1,
            &desc(),
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            &[],
            tokens,
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

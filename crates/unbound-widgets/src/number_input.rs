//! The numeric field: scrub to change, click to edit, Enter to evaluate.
//!
//! Two interaction modes share one state machine:
//!
//! * **Scrub** — dragging horizontally with the primary button changes the
//!   value by `delta_x * step`. `Shift` multiplies the step by ten and `Alt`
//!   divides it by ten, so coarse and fine adjustments need no extra widget.
//! * **Edit** — a click without movement (or a double click) swaps in a
//!   [`TextEditState`]. Committing evaluates a small arithmetic expression, so
//!   `120 + 45 * 2` becomes `210` instead of being rejected.
//!
//! The emitter never shapes text: the caller lays out [`NumberInputState::display_text`]
//! and hands the quads back, exactly like the text input does.

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{reduce_text_input, TextEditState, TextInputDesc, TextInputEvent};
use crate::{ControlState, WidgetOutput};

/// The width of the spinner column.
pub const SPIN_WIDTH: f32 = 18.0;
/// The padding between the field border and the text.
pub const NUMBER_PADDING: f32 = 10.0;
/// The corner radius of the field.
pub const NUMBER_RADIUS: f32 = 8.0;

/// Formatting policy and hard bounds for a numeric field.
#[derive(Clone, Debug, PartialEq)]
pub struct NumberInputDesc {
    /// The field rectangle.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The lowest accepted value.
    pub min: f64,
    /// The highest accepted value.
    pub max: f64,
    /// The scrub / spinner increment.
    pub step: f64,
    /// Digits kept after the decimal point.
    pub precision: u8,
    /// A unit suffix (`"px"`, `"%"`).
    pub unit: String,
    /// A prefix (`"$"`, `"≈"`).
    pub prefix: String,
    /// The field refuses interaction.
    pub disabled: bool,
    /// The draw order key.
    pub z: u16,
}

impl NumberInputDesc {
    /// A field bounded by `min` and `max`, stepping by one.
    #[must_use]
    pub fn new(rect: Rect, min: f64, max: f64) -> Self {
        Self {
            rect,
            label: String::new(),
            min,
            max,
            step: 1.0,
            precision: 0,
            unit: String::new(),
            prefix: String::new(),
            disabled: false,
            z: 730,
        }
    }
}

/// Which mode the field is in.
#[derive(Clone, Debug, PartialEq)]
pub enum NumberMode {
    /// Drag horizontally to change the value.
    Scrub,
    /// An inline text editor owns the keyboard.
    Edit(TextEditState),
}

/// The live state of one numeric field.
#[derive(Clone, Debug, PartialEq)]
pub struct NumberInputState {
    value: f64,
    mode: NumberMode,
    drag_origin_x: f32,
    drag_origin_value: f64,
    dragging: bool,
}

impl NumberInputState {
    /// A field holding `value`.
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self {
            value,
            mode: NumberMode::Scrub,
            drag_origin_x: 0.0,
            drag_origin_value: value,
            dragging: false,
        }
    }

    /// The current value.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    /// The current mode.
    #[must_use]
    pub fn mode(&self) -> &NumberMode {
        &self.mode
    }

    /// Whether an inline editor is open.
    #[must_use]
    pub fn is_editing(&self) -> bool {
        matches!(self.mode, NumberMode::Edit(_))
    }

    /// The inline editor, when one is open.
    #[must_use]
    pub fn edit(&self) -> Option<&TextEditState> {
        match &self.mode {
            NumberMode::Edit(edit) => Some(edit),
            NumberMode::Scrub => None,
        }
    }

    /// Set the value, clamped to `[min, max]` and rounded to `precision`.
    pub fn set_value(&mut self, value: f64, desc: &NumberInputDesc) -> bool {
        if !value.is_finite() {
            return false;
        }
        let snapped = round_to(value.clamp(desc.min, desc.max), desc.precision);
        let changed = (snapped - self.value).abs() > f64::EPSILON;
        self.value = snapped;
        changed
    }

    /// Step by whole increments (the spinner buttons and arrow keys).
    pub fn step_by(&mut self, steps: i32, desc: &NumberInputDesc) -> bool {
        self.set_value(self.value + desc.step * f64::from(steps), desc)
    }

    /// Begin a scrub gesture at `x`.
    pub fn begin_drag(&mut self, x: f32) {
        self.drag_origin_x = x;
        self.drag_origin_value = self.value;
        self.dragging = true;
    }

    /// Continue a scrub gesture.
    ///
    /// The new value is always derived from the gesture's origin, so a drag
    /// that moves back and forth never accumulates rounding drift.
    pub fn drag_to(&mut self, x: f32, shift: bool, alt: bool, desc: &NumberInputDesc) -> bool {
        if !self.dragging {
            return false;
        }
        let modifier = if shift {
            10.0
        } else if alt {
            0.1
        } else {
            1.0
        };
        let value =
            self.drag_origin_value + f64::from(x - self.drag_origin_x) * desc.step * modifier;
        self.set_value(value, desc)
    }

    /// Finish the scrub gesture.
    pub fn end_drag(&mut self) {
        self.dragging = false;
    }

    /// Whether a scrub gesture is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Open the inline editor, prefilled with the current value.
    pub fn begin_edit(&mut self, desc: &NumberInputDesc) {
        self.mode = NumberMode::Edit(TextEditState::with_text(self.format_value(desc)));
    }

    /// Route a text-input event to the inline editor.
    pub fn edit_event(
        &mut self,
        desc: &NumberInputDesc,
        event: &TextInputEvent,
    ) -> crate::text_input::TextInputOutcome {
        let editor_desc = edit_desc(desc);
        match &mut self.mode {
            NumberMode::Edit(edit) => reduce_text_input(&editor_desc, edit, event),
            NumberMode::Scrub => Default::default(),
        }
    }

    /// Commit the inline editor.
    ///
    /// A expression that cannot be evaluated leaves the editor open and the
    /// value untouched, so a typo never destroys the previous number.
    pub fn commit_edit(&mut self, desc: &NumberInputDesc) -> Option<f64> {
        let NumberMode::Edit(edit) = &self.mode else {
            return None;
        };
        let parsed = evaluate_expression(edit.text())?;
        self.set_value(parsed, desc);
        self.mode = NumberMode::Scrub;
        Some(self.value)
    }

    /// Abandon the inline editor.
    pub fn cancel_edit(&mut self) {
        self.mode = NumberMode::Scrub;
    }

    /// The text the caller should shape for display.
    #[must_use]
    pub fn display_text(&self, desc: &NumberInputDesc) -> String {
        match &self.mode {
            NumberMode::Edit(edit) => edit.text().to_owned(),
            NumberMode::Scrub => self.format_value(desc),
        }
    }

    /// `prefix + number + unit`.
    #[must_use]
    pub fn format_value(&self, desc: &NumberInputDesc) -> String {
        format!(
            "{}{}{}",
            desc.prefix,
            format_number(self.value, desc.precision),
            desc.unit
        )
    }
}

fn edit_desc(desc: &NumberInputDesc) -> TextInputDesc {
    let mut editor = TextInputDesc::new(desc.rect);
    editor.label = desc.label.clone();
    editor.disabled = desc.disabled;
    editor.max_bytes = 64;
    editor
}

/// Round to `precision` digits after the decimal point.
#[must_use]
pub fn round_to(value: f64, precision: u8) -> f64 {
    let factor = 10f64.powi(i32::from(precision.min(12)));
    (value * factor).round() / factor
}

/// Format with fixed decimals and no trailing zeros beyond `precision`.
#[must_use]
pub fn format_number(value: f64, precision: u8) -> String {
    let mut text = format!(
        "{value:.precision$}",
        precision = usize::from(precision.min(12))
    );
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".into();
    }
    text
}

/// Evaluate `+ - * /` with parentheses and unary minus.
///
/// The parser is deliberately tiny: it accepts the arithmetic a user types
/// into a numeric field, rejects everything else by returning `None`, and
/// never panics.
#[must_use]
pub fn evaluate_expression(input: &str) -> Option<f64> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        index: 0,
    };
    let value = parser.expression()?;
    parser.skip_space();
    if parser.index == parser.bytes.len() && value.is_finite() {
        Some(value)
    } else {
        None
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self
            .bytes
            .get(self.index)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.index += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_space();
        self.bytes.get(self.index).copied()
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.index += 1;
                    value += self.term()?;
                }
                Some(b'-') => {
                    self.index += 1;
                    value -= self.term()?;
                }
                _ => return Some(value),
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.factor()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.index += 1;
                    value *= self.factor()?;
                }
                Some(b'/') => {
                    self.index += 1;
                    let divisor = self.factor()?;
                    if divisor == 0.0 {
                        return None;
                    }
                    value /= divisor;
                }
                _ => return Some(value),
            }
        }
    }

    fn factor(&mut self) -> Option<f64> {
        match self.peek()? {
            b'-' => {
                self.index += 1;
                Some(-self.factor()?)
            }
            b'+' => {
                self.index += 1;
                self.factor()
            }
            b'(' => {
                self.index += 1;
                let value = self.expression()?;
                if self.peek()? != b')' {
                    return None;
                }
                self.index += 1;
                Some(value)
            }
            b'0'..=b'9' | b'.' => self.number(),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_space();
        let start = self.index;
        while self
            .bytes
            .get(self.index)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            self.index += 1;
        }
        if start == self.index {
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.index])
            .ok()?
            .parse()
            .ok()
    }
}

/// The spinner column splits into an upper and a lower button.
#[must_use]
pub fn spin_up_rect(rect: Rect) -> Rect {
    Rect::new(
        rect.max().x - SPIN_WIDTH,
        rect.origin.y + 1.0,
        SPIN_WIDTH,
        rect.size.y * 0.5 - 1.0,
    )
}

/// The lower spinner button.
#[must_use]
pub fn spin_down_rect(rect: Rect) -> Rect {
    let up = spin_up_rect(rect);
    Rect::new(
        up.origin.x,
        up.max().y,
        up.size.x,
        rect.max().y - up.max().y,
    )
}

/// The rectangle available to the formatted number.
#[must_use]
pub fn number_text_rect(rect: Rect) -> Rect {
    Rect::new(
        rect.origin.x + NUMBER_PADDING,
        rect.origin.y,
        (rect.size.x - NUMBER_PADDING * 2.0 - SPIN_WIDTH).max(0.0),
        rect.size.y,
    )
}

/// Emit the field chrome, spinner buttons and semantic node.
///
/// `glyphs` must already be positioned inside [`number_text_rect`]; the
/// emitter only decides colours.
#[must_use]
pub fn emit_number_input(
    node: u64,
    desc: &NumberInputDesc,
    state: &NumberInputState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(12 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::INPUT_BG
    };
    primitives.push(surface(rect, NUMBER_RADIUS, fill, desc.z));
    let border = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.focused || state.is_editing() {
        Token::OUTLINE_STRONG
    } else {
        Token::OUTLINE
    };
    primitives.push(outlined(
        rect,
        NUMBER_RADIUS,
        Token::NONE,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let ink = if control.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(glyphs, ink, desc.z, &mut primitives, &mut payloads);

    // The spinner column: a hairline separator and two slim buttons. The
    // buttons are painted whether or not they are hovered; only the fill
    // token changes, so the primitive count stays constant.
    let up = spin_up_rect(rect);
    let down = spin_down_rect(rect);
    primitives.push(instance(
        Rect::new(up.origin.x, rect.origin.y + 4.0, 1.0, rect.size.y - 8.0),
        [0.0; 4],
        Token::OUTLINE,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Background,
        desc.z,
    ));
    let spin_fill = |hot: bool| {
        if control.disabled {
            Token::CONTROL_DISABLED_BG
        } else if hot {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        }
    };
    let hot_up = control.phase == crate::PointerPhase::Hovered && control.selected;
    let hot_down = control.phase == crate::PointerPhase::Hovered && !control.selected;
    primitives.push(surface(up, 3.0, spin_fill(hot_up), desc.z));
    primitives.push(surface(down, 3.0, spin_fill(hot_down), desc.z));
    let mark = if control.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    primitives.push(instance(
        Rect::new(
            up.center().x - 3.0,
            up.origin.y + up.size.y * 0.5 - 0.75,
            6.0,
            1.5,
        ),
        [0.0; 4],
        mark,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Content,
        desc.z,
    ));
    primitives.push(instance(
        Rect::new(
            down.center().x - 3.0,
            down.origin.y + down.size.y * 0.5 - 0.75,
            6.0,
            1.5,
        ),
        [0.0; 4],
        mark,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Content,
        desc.z,
    ));

    primitives.push(focus_ring(
        rect.inflate(3.0),
        NUMBER_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [NUMBER_RADIUS; 4],
            })
            .with_cursor(if state.is_editing() {
                CursorKind::Text
            } else {
                CursorKind::ResizeEastWest
            })
    });
    let mut a11y = A11yNode::new(Role::SpinButton, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.format_value(desc);
    a11y.actions.insert(A11yAction::Increment);
    a11y.actions.insert(A11yAction::Decrement);
    a11y.actions.insert(A11yAction::SetValue);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> NumberInputDesc {
        let mut desc = NumberInputDesc::new(Rect::new(10.0, 10.0, 180.0, 34.0), 0.0, 100.0);
        desc.step = 2.0;
        desc.precision = 1;
        desc
    }

    #[test]
    fn values_are_clamped_and_rounded() {
        let desc = desc();
        let mut state = NumberInputState::new(10.0);
        assert!(state.set_value(500.0, &desc));
        assert_eq!(state.value(), 100.0);
        assert!(state.set_value(-5.0, &desc));
        assert_eq!(state.value(), 0.0);
        state.set_value(1.234, &desc);
        assert!((state.value() - 1.2).abs() < 1e-9);
        assert!(!state.set_value(f64::NAN, &desc));
    }

    #[test]
    fn scrubbing_uses_the_step_and_modifier_scale() {
        let desc = desc();
        let mut state = NumberInputState::new(10.0);
        state.begin_drag(100.0);
        assert!(state.drag_to(110.0, false, false, &desc));
        assert!((state.value() - 30.0).abs() < 1e-9, "10 px * step 2");

        state.begin_drag(100.0);
        state.drag_to(110.0, true, false, &desc);
        assert!(
            (state.value() - 100.0).abs() < 1e-9,
            "Shift x10 clamps at max"
        );

        state.set_value(10.0, &desc);
        state.begin_drag(100.0);
        state.drag_to(110.0, false, true, &desc);
        assert!((state.value() - 12.0).abs() < 1e-9, "Alt x0.1");
    }

    #[test]
    fn a_scrub_is_derived_from_its_origin() {
        let desc = desc();
        let mut state = NumberInputState::new(50.0);
        state.begin_drag(0.0);
        state.drag_to(5.0, false, false, &desc);
        state.drag_to(10.0, false, false, &desc);
        assert!((state.value() - 70.0).abs() < 1e-9);
        state.drag_to(0.0, false, false, &desc);
        assert!((state.value() - 50.0).abs() < 1e-9);
        state.end_drag();
        assert!(!state.is_dragging());
    }

    #[test]
    fn the_editor_is_prefilled_with_the_formatted_value() {
        let desc = desc();
        let mut state = NumberInputState::new(12.5);
        state.begin_edit(&desc);
        assert!(state.is_editing());
        assert_eq!(state.edit().map(TextEditState::text), Some("12.5"));
    }

    #[test]
    fn committing_evaluates_arithmetic() {
        let desc = desc();
        let mut state = NumberInputState::new(0.0);
        state.begin_edit(&desc);
        state.edit_event(&desc, &TextInputEvent::SelectAll);
        state.edit_event(&desc, &TextInputEvent::Insert("20 + 30 * 2".into()));
        assert_eq!(state.commit_edit(&desc), Some(80.0));
        assert!(!state.is_editing());
    }

    #[test]
    fn a_bad_expression_keeps_the_editor_and_the_value() {
        let desc = desc();
        let mut state = NumberInputState::new(7.0);
        state.begin_edit(&desc);
        state.edit_event(&desc, &TextInputEvent::SelectAll);
        state.edit_event(&desc, &TextInputEvent::Insert("1 + ".into()));
        assert_eq!(state.commit_edit(&desc), None);
        assert!(state.is_editing());
        assert_eq!(state.value(), 7.0);
    }

    #[test]
    fn expressions_respect_precedence_parens_and_unary_minus() {
        assert_eq!(evaluate_expression("120 + 45 * 2"), Some(210.0));
        assert_eq!(evaluate_expression("(1 + 2) * 3"), Some(9.0));
        assert_eq!(evaluate_expression("-(2 - 5)"), Some(3.0));
        assert_eq!(evaluate_expression("8 / 2 / 2"), Some(2.0));
        assert_eq!(evaluate_expression("1 / 0"), None);
        assert_eq!(evaluate_expression("1 +"), None);
        assert_eq!(evaluate_expression("abc"), None);
    }

    #[test]
    fn formatting_drops_trailing_zeros_but_keeps_precision() {
        assert_eq!(format_number(12.0, 0), "12");
        assert_eq!(format_number(12.50, 2), "12.5");
        assert_eq!(format_number(-0.0, 0), "0");
        assert_eq!(format_number(1.234, 2), "1.23");
    }

    #[test]
    fn the_spinner_buttons_tile_the_right_edge() {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);
        let up = spin_up_rect(rect);
        let down = spin_down_rect(rect);
        assert_eq!(up.size.x, SPIN_WIDTH);
        assert!((up.max().y - down.origin.y).abs() < 1e-6);
        assert!((down.max().y - rect.max().y).abs() < 1e-6);
        assert!(number_text_rect(rect).max().x <= up.origin.x);
    }

    #[test]
    fn the_field_reports_a_spin_button_with_increment_actions() {
        let state = NumberInputState::new(3.0);
        let mut widget = crate::WidgetHarness::new(
            NumberField {
                node: 9,
                desc: desc(),
                state,
            },
            Vec::new(),
        );
        widget.focus();
        let a11y = widget.output.a11y.as_ref().expect("reachable");
        assert_eq!(a11y.role, Role::SpinButton);
        assert!(a11y.actions.contains(A11yAction::Increment));
        assert!(a11y.actions.contains(A11yAction::Decrement));
        assert_eq!(
            widget.output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::ResizeEastWest))
        );
    }

    struct NumberField {
        node: u64,
        desc: NumberInputDesc,
        state: NumberInputState,
    }

    impl crate::WidgetDesc for NumberField {
        fn rect(&self) -> Rect {
            self.desc.rect
        }

        fn key(&self) -> u64 {
            self.node
        }

        fn disabled(&self) -> bool {
            self.desc.disabled
        }

        fn emit(&self, state: ControlState, glyphs: &mut [GlyphQuad]) -> WidgetOutput {
            emit_number_input(self.node, &self.desc, &self.state, state, glyphs)
        }
    }
}

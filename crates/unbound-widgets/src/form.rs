//! Form layout, validation aggregation and "jump to the first error".
//!
//! A form is not a control: it is the *composition* of labels, inputs and
//! messages. This module owns the three things that are genuinely shared:
//!
//! * where each label, input and message goes ([`layout_form`]),
//! * which field failed and what the message is ([`FormState`]), and
//! * scrolling the first failure into view and focusing it.
//!
//! Validation is circular-safe by construction: the reducer receives the
//! results, not the widgets, so a field that validates using another field's
//! value is just an application-level closure.

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{instance, surface};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::WidgetOutput;

/// The height of a standard input row.
pub const FORM_INPUT_HEIGHT: f32 = 36.0;
/// The height of one text line inside a label or message.
pub const FORM_LINE: f32 = 18.0;
/// The gap between a label and its input.
pub const FORM_LABEL_GAP: f32 = 6.0;
/// The gap between fields.
pub const FORM_FIELD_GAP: f32 = 18.0;
/// The gap between an input and its message.
pub const FORM_MESSAGE_GAP: f32 = 4.0;

/// Where a label sits relative to its input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormLayout {
    /// Label above the input.
    #[default]
    Vertical,
    /// Label to the left, in a fixed column.
    Horizontal,
}

/// The pure descriptor of one field.
#[derive(Clone, Debug, PartialEq)]
pub struct FormFieldDesc {
    /// The field label.
    pub label: String,
    /// The field must be filled.
    pub required: bool,
    /// A short help line shown while there is no error.
    pub help: String,
    /// The layout used by this field.
    pub layout: FormLayout,
    /// The height of the input control.
    pub input_height: f32,
}

impl FormFieldDesc {
    /// A vertical field with the standard height.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            required: false,
            help: String::new(),
            layout: FormLayout::Vertical,
            input_height: FORM_INPUT_HEIGHT,
        }
    }

    /// Mark the field as required.
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Attach a help line.
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }
}

/// A field and its stable identity.
#[derive(Clone, Debug, PartialEq)]
pub struct FormField {
    /// The identity the application uses for validation results.
    pub key: u64,
    /// The field's presentation.
    pub desc: FormFieldDesc,
}

impl FormField {
    /// A field with an identity.
    #[must_use]
    pub fn new(key: u64, desc: FormFieldDesc) -> Self {
        Self { key, desc }
    }
}

/// The pure descriptor of a form.
#[derive(Clone, Debug, PartialEq)]
pub struct FormDesc {
    /// The form's content rectangle.
    pub rect: Rect,
    /// The label column width in horizontal layouts.
    pub label_width: f32,
    /// The gap between fields.
    pub spacing: f32,
    /// The draw order key.
    pub z: u16,
}

impl FormDesc {
    /// A form with the default metrics.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label_width: 110.0,
            spacing: FORM_FIELD_GAP,
            z: 720,
        }
    }
}

/// The measured geometry of one field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormFieldLayout {
    /// The identity of the field.
    pub key: u64,
    /// The label rectangle.
    pub label: Rect,
    /// The reserved input rectangle.
    pub input: Rect,
    /// The message rectangle (help or error).
    pub message: Rect,
}

/// Layout every field top to bottom.
///
/// A field reserves a message line whenever it has help text; an error then
/// replaces the help line instead of pushing the rest of the form down, which
/// is what prevents a validation pass from making the layout jump.
#[must_use]
pub fn layout_form(desc: &FormDesc, fields: &[FormField]) -> Vec<FormFieldLayout> {
    let mut y = desc.rect.origin.y;
    let mut layouts = Vec::with_capacity(fields.len());
    for field in fields {
        let message_height = if field.desc.help.is_empty() {
            0.0
        } else {
            FORM_LINE + FORM_MESSAGE_GAP
        };
        let (label, input, message) = match field.desc.layout {
            FormLayout::Vertical => {
                let label = Rect::new(desc.rect.origin.x, y, desc.rect.size.x, FORM_LINE);
                let input = Rect::new(
                    desc.rect.origin.x,
                    label.max().y + FORM_LABEL_GAP,
                    desc.rect.size.x,
                    field.desc.input_height,
                );
                let message = Rect::new(
                    desc.rect.origin.x,
                    input.max().y + FORM_MESSAGE_GAP,
                    desc.rect.size.x,
                    FORM_LINE,
                );
                (label, input, message)
            }
            FormLayout::Horizontal => {
                let label = Rect::new(
                    desc.rect.origin.x,
                    y + (field.desc.input_height - FORM_LINE) * 0.5,
                    desc.label_width,
                    FORM_LINE,
                );
                let input = Rect::new(
                    desc.rect.origin.x + desc.label_width,
                    y,
                    (desc.rect.size.x - desc.label_width).max(0.0),
                    field.desc.input_height,
                );
                let message = Rect::new(
                    input.origin.x,
                    input.max().y + FORM_MESSAGE_GAP,
                    input.size.x,
                    FORM_LINE,
                );
                (label, input, message)
            }
        };
        y = input.max().y + message_height + desc.spacing;
        layouts.push(FormFieldLayout {
            key: field.key,
            label,
            input,
            message,
        });
    }
    layouts
}

/// The live validation state of a form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormState {
    errors: FxHashMap<u64, String>,
    focused: Option<u64>,
}

impl FormState {
    /// A form with no errors.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The message for a field, if it failed.
    #[must_use]
    pub fn error(&self, key: u64) -> Option<&str> {
        self.errors.get(&key).map(String::as_str)
    }

    /// The field that should own focus.
    #[must_use]
    pub fn focused(&self) -> Option<u64> {
        self.focused
    }

    /// Whether every field passed.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Record or clear one field's error.
    pub fn set_error(&mut self, key: u64, message: Option<String>) -> bool {
        match message.filter(|message| !message.is_empty()) {
            Some(message) => {
                let previous = self.errors.insert(key, message.clone());
                previous.as_deref() != Some(message.as_str())
            }
            None => self.errors.remove(&key).is_some(),
        }
    }

    /// Replace every error from a validation pass.
    ///
    /// Returns the first failing field in form order and focuses it. The form
    /// order is the caller's order, which is also visual order, so "first
    /// error" means "top-most error" and not "first field that happened to
    /// fail".
    pub fn validate<I>(&mut self, results: I) -> Option<u64>
    where
        I: IntoIterator<Item = (u64, Option<String>)>,
    {
        self.errors.clear();
        let mut first = None;
        for (key, message) in results {
            if let Some(message) = message.filter(|message| !message.is_empty()) {
                self.errors.insert(key, message);
                first.get_or_insert(key);
            }
        }
        self.focused = first;
        first
    }

    /// Focus a field.
    pub fn focus(&mut self, key: u64) {
        self.focused = Some(key);
    }

    /// The vertical offset needed to reveal `field` in `viewport`.
    #[must_use]
    pub fn reveal_offset(viewport: Rect, field: Rect) -> f32 {
        if field.origin.y < viewport.origin.y {
            field.origin.y - viewport.origin.y
        } else if field.max().y > viewport.max().y {
            field.max().y - viewport.max().y
        } else {
            0.0
        }
    }
}

/// Scroll `field` into view, returning whether the offset changed.
pub fn reveal_field(
    scroll: &mut ScrollState,
    viewport: Rect,
    field: Rect,
    content_height: f32,
) -> bool {
    let delta = FormState::reveal_offset(viewport, field);
    if delta.abs() <= f32::EPSILON {
        return false;
    }
    let desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
    let before = scroll.target().y;
    scroll.scroll_by(&desc, Vec2::new(0.0, delta));
    (scroll.target().y - before).abs() > f32::EPSILON
}

/// The required marker as a two-primitive dot: no glyphs needed.
#[must_use]
pub fn required_marker(rect: Rect, z: u16) -> Vec<GpuPrimitiveInstance> {
    vec![
        instance(
            Rect::new(
                rect.origin.x,
                rect.origin.y + rect.size.y * 0.5 - 2.0,
                4.0,
                4.0,
            ),
            [2.0; 4],
            Token::ERROR,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Content,
            z,
        ),
        instance(
            Rect::new(
                rect.origin.x + 2.0,
                rect.origin.y + rect.size.y - 1.0,
                2.0,
                1.0,
            ),
            [0.0; 4],
            Token::ERROR,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            z,
        ),
    ]
}

/// The semantic root of a form.
///
/// Screen readers announce the group once and then its labelled children, so
/// the form itself is a `Form` node with no hit region and no focus slot.
#[must_use]
pub fn emit_form_root(rect: Rect, label: impl Into<String>) -> WidgetOutput {
    WidgetOutput {
        a11y: Some(A11yNode::new(Role::Form, label.into(), rect)),
        ..WidgetOutput::default()
    }
}

/// Emit a field label with its required marker.
#[must_use]
pub fn emit_form_label(
    field: &FormField,
    layout: &FormFieldLayout,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::new();
    let mut payloads = Vec::with_capacity(glyphs.len());
    if field.desc.required {
        primitives.extend(required_marker(layout.label, 0));
    }
    if !glyphs.is_empty() {
        crate::primitive::glyphs(
            glyphs,
            Token::TEXT_SECONDARY,
            0,
            &mut primitives,
            &mut payloads,
        );
    }
    let mut a11y = A11yNode::new(Role::Label, field.desc.label.clone(), layout.label);
    a11y.description = field.desc.help.clone();
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
    }
}

/// Emit the message line for one field.
///
/// An error gets a filled surface and an error-coloured underline; help text
/// stays flat. The caller's glyphs are tinted accordingly.
#[must_use]
pub fn emit_form_message(
    field: &FormField,
    layout: &FormFieldLayout,
    error: Option<&str>,
    glyphs: &mut [GlyphQuad],
    z: u16,
) -> WidgetOutput {
    let mut primitives = Vec::new();
    let mut payloads = Vec::with_capacity(glyphs.len());
    let has_message = error.is_some() || !field.desc.help.is_empty();
    if has_message {
        primitives.push(surface(
            layout.message,
            0.0,
            if error.is_some() {
                Token::ERROR_SOFT
            } else {
                Token::NONE
            },
            z,
        ));
    }
    if error.is_some() {
        primitives.push(instance(
            Rect::new(
                layout.message.origin.x,
                layout.message.origin.y + 1.0,
                2.0,
                layout.message.size.y - 2.0,
            ),
            [1.0; 4],
            Token::ERROR,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Content,
            z,
        ));
    }
    crate::primitive::glyphs(
        glyphs,
        if error.is_some() {
            Token::ERROR
        } else {
            Token::TEXT_SECONDARY
        },
        z,
        &mut primitives,
        &mut payloads,
    );

    let mut a11y = A11yNode::new(
        Role::Label,
        error.unwrap_or(&field.desc.help).to_owned(),
        layout.message,
    );
    a11y.description = field.desc.label.clone();
    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<FormField> {
        vec![
            FormField::new(
                1,
                FormFieldDesc::new("Name").required(true).help("Full name"),
            ),
            FormField::new(2, FormFieldDesc::new("Email").required(true)),
        ]
    }

    #[test]
    fn vertical_fields_stack_labels_inputs_and_messages() {
        let desc = FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0));
        let layouts = layout_form(&desc, &fields());
        assert_eq!(layouts.len(), 2);
        assert!(layouts[0].label.max().y <= layouts[0].input.origin.y);
        assert!(layouts[0].message.origin.y > layouts[0].input.max().y);
        assert!(layouts[1].label.origin.y > layouts[0].message.origin.y);
    }

    #[test]
    fn horizontal_fields_put_the_label_in_a_column() {
        let desc = FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0));
        let mut field = FormFieldDesc::new("Weight");
        field.layout = FormLayout::Horizontal;
        let layouts = layout_form(&desc, &[FormField::new(1, field)]);
        assert_eq!(layouts[0].label.size.x, desc.label_width);
        assert_eq!(
            layouts[0].input.origin.x,
            desc.rect.origin.x + desc.label_width
        );
        assert!((layouts[0].input.size.x + desc.label_width - desc.rect.size.x).abs() < 1e-5);
    }

    #[test]
    fn validation_reports_and_focuses_the_first_failure() {
        let mut state = FormState::new();
        let first = state.validate([
            (1, None),
            (2, Some("required".into())),
            (3, Some("too long".into())),
        ]);
        assert_eq!(first, Some(2));
        assert_eq!(state.focused(), Some(2));
        assert_eq!(state.error(2), Some("required"));
        assert!(!state.is_valid());
        assert_eq!(state.error(1), None);
    }

    #[test]
    fn a_clean_pass_clears_previous_errors() {
        let mut state = FormState::new();
        state.validate([(1, Some("bad".into()))]);
        assert!(!state.is_valid());
        assert_eq!(state.validate([(1, None)]), None);
        assert!(state.is_valid());
        assert_eq!(state.focused(), None);
    }

    #[test]
    fn an_empty_message_counts_as_valid() {
        let mut state = FormState::new();
        assert_eq!(state.validate([(1, Some(String::new()))]), None);
        assert!(state.is_valid());
    }

    #[test]
    fn reveal_offsets_only_when_the_field_is_outside() {
        let viewport = Rect::new(0.0, 100.0, 300.0, 200.0);
        assert_eq!(
            FormState::reveal_offset(viewport, Rect::new(0.0, 120.0, 300.0, 40.0)),
            0.0
        );
        assert_eq!(
            FormState::reveal_offset(viewport, Rect::new(0.0, 40.0, 300.0, 40.0)),
            -60.0
        );
        assert_eq!(
            FormState::reveal_offset(viewport, Rect::new(0.0, 320.0, 300.0, 40.0)),
            60.0
        );
    }

    #[test]
    fn revealing_scrolls_a_spring_toward_the_field() {
        let viewport = Rect::new(0.0, 100.0, 300.0, 200.0);
        let mut scroll = ScrollState::new();
        assert!(reveal_field(
            &mut scroll,
            viewport,
            Rect::new(0.0, 400.0, 300.0, 40.0),
            1000.0
        ));
        assert!(scroll.target().y > 0.0);
        assert!(!reveal_field(
            &mut scroll,
            viewport,
            Rect::new(0.0, 150.0, 300.0, 40.0),
            1000.0
        ));
    }

    #[test]
    fn the_required_marker_has_two_primitives() {
        assert_eq!(
            required_marker(Rect::new(0.0, 0.0, 100.0, 18.0), 5).len(),
            2
        );
    }

    #[test]
    fn the_form_root_is_a_form_node() {
        let root = emit_form_root(Rect::new(0.0, 0.0, 320.0, 400.0), "Account");
        let a11y = root.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Form);
        assert_eq!(a11y.label, "Account");
        assert!(root.primitives.is_empty());
    }

    #[test]
    fn labels_and_messages_are_reachable() {
        let desc = FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0));
        let fields = fields();
        let layouts = layout_form(&desc, &fields);
        let label = emit_form_label(&fields[0], &layouts[0], &mut []);
        assert_eq!(label.a11y.expect("reachable").role, Role::Label);
        assert_eq!(label.primitives.len(), 2);

        let message = emit_form_message(&fields[0], &layouts[0], Some("Required"), &mut [], 0);
        let a11y = message.a11y.expect("reachable");
        assert_eq!(a11y.label, "Required");
        assert_eq!(message.primitives.len(), 2, "error surface and bar");
    }
}

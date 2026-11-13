//! The single-line text input: descriptor, edit state, reducer and emitter.
//!
//! The component is split along the same seams every other control uses:
//!
//! * [`TextInputDesc`] is pure data — geometry, policy and nothing mutable.
//! * [`TextEditState`] owns the UTF-8 buffer, the cursor, the selection and
//!   the undo/redo stacks. It never needs a font or a GPU.
//! * [`reduce_text_input`] is a pure state evolution over [`TextInputEvent`].
//!   It is where every UTF-8 boundary rule lives: the cursor can only land on
//!   a `char` boundary, so backspacing a CJK ideograph or an emoji removes the
//!   whole scalar value instead of panicking on a split code point.
//! * [`emit_text_input`] turns `(descriptor, state, edit state, measured text
//!   layout)` into primitives, a hit region and a semantic node.
//!
//! The emitter always writes the same primitive slots — background, border,
//! selection, caret, focus ring — whether or not there is a selection or
//! focus. Selection and caret use an opacity of `0.0` when absent, so typing
//! one character repaints one slot instead of reshaping the whole pool.
//!
//! The measured caret / selection rectangles come from the caller: the text
//! engine knows the shaped advance widths, this crate deliberately does not.

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement, CLIP_NONE};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{f16, Rect};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined, surface};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// The UI layer a text input's hit region lives in.
pub const TEXT_INPUT_LAYER: u8 = 1;
/// The default font size.
pub const TEXT_INPUT_FONT_SIZE: f32 = 14.0;
/// The default corner radius.
pub const TEXT_INPUT_RADIUS: f32 = 8.0;
/// Horizontal padding between the border and the text.
pub const TEXT_INPUT_PADDING: f32 = 10.0;
/// How far the focus ring grows outside the input.
const FOCUS_RING_GAP: f32 = 3.0;

/// A user action the text input understands.
///
/// Copy and cut are not performed by the widget: they return the text to copy
/// in [`TextInputOutcome`], and the application talks to the platform
/// clipboard. Paste is the mirror image: the application reads the clipboard
/// and sends [`TextInputEvent::Paste`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// Insert text at the cursor (typed characters, IME commit, paste).
    Insert(String),
    /// Delete the selection, or the code point before the cursor.
    Backspace,
    /// Delete the selection, or the code point after the cursor.
    Delete,
    /// Move one code point left.
    MoveLeft { extend: bool },
    /// Move one code point right.
    MoveRight { extend: bool },
    /// Move one word left (`Ctrl+Left`).
    MoveWordLeft { extend: bool },
    /// Move one word right (`Ctrl+Right`).
    MoveWordRight { extend: bool },
    /// Delete the word before the caret (`Ctrl+Backspace`).
    DeleteWordBack,
    /// Delete the word after the caret (`Ctrl+Delete`).
    DeleteWordForward,
    /// Move to the start of the text.
    Home { extend: bool },
    /// Move to the end of the text.
    End { extend: bool },
    /// `Ctrl+A`.
    SelectAll,
    /// `Ctrl+C`.
    Copy,
    /// `Ctrl+X`.
    Cut,
    /// `Ctrl+V`, after the application read the clipboard.
    Paste(String),
    /// `Ctrl+Z`.
    Undo,
    /// `Ctrl+Shift+Z` / `Ctrl+Y`.
    Redo,
    /// The pointer went down at a byte offset.
    PointerDown { offset: usize },
    /// The pointer moved while held, extending the selection.
    PointerDrag { offset: usize },
    /// The pointer was released.
    PointerUp,
    /// A double click selected a word around a byte offset.
    DoubleClick { offset: usize },
    /// A triple click selected the whole line (the whole buffer here).
    TripleClick,
    /// IME pre-edit text occupies `start..end` so it can be replaced on commit.
    ImePreedit { start: usize, end: usize },
    /// The composition ended; `text` replaces the pre-edit range.
    ImeCommit { text: String },
}

/// What one reduction produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInputOutcome {
    /// The edit buffer or selection changed.
    pub changed: bool,
    /// Text the application must put on the clipboard (`Ctrl+C`).
    pub copied: Option<String>,
    /// Text the application must put on the clipboard and forget (`Ctrl+X`).
    pub cut: Option<String>,
    /// The application should read the clipboard and send [`TextInputEvent::Paste`].
    pub paste_requested: bool,
}

impl TextInputOutcome {
    fn changed() -> Self {
        Self {
            changed: true,
            ..Self::default()
        }
    }
}

/// How the field's contents validate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValidationState {
    /// No validation message.
    #[default]
    None,
    /// The contents are invalid.
    Error,
    /// The contents deserve a warning but are accepted.
    Warning,
}

/// A single-line text field's pure descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInputDesc {
    /// The field rectangle.
    pub rect: Rect,
    /// The accessible label (falls back to the placeholder when empty).
    pub label: String,
    /// The placeholder shown while the buffer is empty.
    pub placeholder: String,
    /// The field refuses interaction and focus.
    pub disabled: bool,
    /// The text can be selected and copied but not edited.
    pub read_only: bool,
    /// The characters are masked and Copy/Cut are refused.
    pub password: bool,
    /// The validation state; it drives the border colour.
    pub validation: ValidationState,
    /// The field shows a clear affordance when it is not empty.
    pub clearable: bool,
    /// The maximum buffer length in bytes.
    pub max_bytes: usize,
    /// The draw order key.
    pub z: u16,
    /// The corner radius.
    pub radius: f32,
    /// The font size (used by the caller when shaping).
    pub font_size: f32,
}

impl TextInputDesc {
    /// A field with the default metrics.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            read_only: false,
            password: false,
            validation: ValidationState::None,
            clearable: false,
            max_bytes: 1 << 20,
            z: 720,
            radius: TEXT_INPUT_RADIUS,
            font_size: TEXT_INPUT_FONT_SIZE,
        }
    }

    /// The same field with a placeholder.
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The same field with an accessible label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The same field, disabled.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The same field, read-only.
    #[must_use]
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// The same field, masking its contents.
    #[must_use]
    pub fn password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    /// The same field in `validation`.
    #[must_use]
    pub fn with_validation(mut self, validation: ValidationState) -> Self {
        self.validation = validation;
        self
    }

    /// The same field with a clear affordance.
    #[must_use]
    pub fn clearable(mut self, clearable: bool) -> Self {
        self.clearable = clearable;
        self
    }

    /// The same field with a byte budget.
    #[must_use]
    pub fn max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// The text to shape and paint.
    ///
    /// A password field replaces every scalar value with `•`; the caller shapes
    /// the result instead of [`TextEditState::text`].
    #[must_use]
    pub fn display_text(&self, text: &str) -> String {
        if self.password {
            "\u{2022}".repeat(text.chars().count())
        } else {
            text.to_owned()
        }
    }

    /// The clear affordance, centred inside the trailing padding.
    #[must_use]
    pub fn clear_rect(&self) -> Rect {
        const SIZE: f32 = 16.0;
        Rect::new(
            self.rect.max().x - TEXT_INPUT_PADDING - SIZE,
            self.rect.origin.y + (self.rect.size.y - SIZE) * 0.5,
            SIZE,
            SIZE,
        )
    }

    fn editable(&self) -> bool {
        !self.disabled && !self.read_only
    }

    fn accessible_label(&self) -> String {
        if self.label.is_empty() {
            self.placeholder.clone()
        } else {
            self.label.clone()
        }
    }
}

/// One undo checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// The editable state of one text input.
///
/// Cursors and anchors are **byte offsets** into [`Self::text`], always on a
/// UTF-8 boundary. The pair `(anchor, cursor)` is the selection: equal values
/// mean "no selection".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextEditState {
    text: String,
    cursor: usize,
    anchor: usize,
    scroll_x: f32,
    preedit: Option<(usize, usize)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Consecutive typing merges into one undo step.
    coalescing: bool,
    /// The anchor a pointer drag extends from.
    drag_anchor: Option<usize>,
}

impl TextEditState {
    /// An empty field.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A field initialised with text; the caret sits at the end.
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            anchor: cursor,
            ..Self::default()
        }
    }

    /// The buffer.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The caret byte offset.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selection anchor byte offset.
    #[must_use]
    pub const fn anchor(&self) -> usize {
        self.anchor
    }

    /// The horizontal scroll offset, in logical pixels.
    #[must_use]
    pub const fn scroll_x(&self) -> f32 {
        self.scroll_x
    }

    /// The pre-edit range, if an IME composition is active.
    #[must_use]
    pub const fn preedit(&self) -> Option<(usize, usize)> {
        self.preedit
    }

    /// Whether the field has a non-empty selection.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    /// The selection as a sorted `(start, end)` byte range.
    #[must_use]
    pub fn selection(&self) -> Option<(usize, usize)> {
        if !self.has_selection() {
            return None;
        }
        Some((self.cursor.min(self.anchor), self.cursor.max(self.anchor)))
    }

    /// The selected text.
    #[must_use]
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection()?;
        self.text.get(start..end)
    }

    /// Replace the whole buffer and put the caret at the end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.normalize();
    }

    /// Set the horizontal scroll offset, clamped to non-negative values.
    pub fn set_scroll_x(&mut self, scroll_x: f32) {
        self.scroll_x = scroll_x.max(0.0);
    }

    /// Scroll just enough that the caret lies inside the visible window.
    ///
    /// `caret_x` is the caret's position in the text's own (unscrolled)
    /// coordinate space; the caret is kept inside
    /// `[scroll_x, scroll_x + viewport_width]`.
    pub fn ensure_caret_visible(&mut self, caret_x: f32, viewport_width: f32) {
        let viewport_width = viewport_width.max(0.0);
        if caret_x < self.scroll_x {
            self.scroll_x = caret_x.max(0.0);
        } else if caret_x > self.scroll_x + viewport_width {
            self.scroll_x = (caret_x - viewport_width).max(0.0);
        }
    }

    /// Move the caret to a byte offset, clamped to a character boundary.
    ///
    /// `extend` keeps the anchor, which grows or shrinks the selection.
    pub fn set_cursor(&mut self, offset: usize, extend: bool) {
        let cursor = floor_boundary(&self.text, offset);
        self.cursor = cursor;
        if !extend {
            self.anchor = cursor;
        }
    }

    /// Select everything.
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.coalescing = false;
    }

    /// Insert text at the cursor, replacing the selection.
    ///
    /// The buffer is clamped to `max_bytes` without splitting a code point.
    pub fn insert(&mut self, text: &str, max_bytes: usize) {
        self.insert_with(text, max_bytes, true);
    }

    fn insert_with(&mut self, text: &str, max_bytes: usize, coalesce: bool) {
        if text.is_empty() {
            return;
        }
        self.push_undo(coalesce);
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let mut next = String::with_capacity(self.text.len() + text.len());
        next.push_str(&self.text[..start]);
        next.push_str(text);
        next.push_str(&self.text[end..]);
        if next.len() > max_bytes {
            // Take as many whole code points of the insertion as fit.
            let room = max_bytes.saturating_sub(self.text.len() - (end - start));
            let mut accepted = 0usize;
            for (index, character) in text.char_indices() {
                let after = index + character.len_utf8();
                if after > room {
                    break;
                }
                accepted = after;
            }
            next.clear();
            next.push_str(&self.text[..start]);
            next.push_str(&text[..accepted]);
            next.push_str(&self.text[end..]);
        }
        self.text = next;
        self.cursor = floor_boundary(&self.text, start + text.len());
        self.anchor = self.cursor;
        self.normalize();
    }

    /// Delete the selection, or one code point before the caret.
    pub fn backspace(&mut self) {
        let (start, end) = self.selection().unwrap_or_else(|| {
            let start = prev_boundary(&self.text, self.cursor);
            (start, self.cursor)
        });
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// Delete the selection, or one code point after the caret.
    pub fn delete_forward(&mut self) {
        let (start, end) = self.selection().unwrap_or_else(|| {
            let end = next_boundary(&self.text, self.cursor);
            (self.cursor, end)
        });
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// Delete the selection, or the word before the caret.
    pub fn delete_word_backward(&mut self) {
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (prev_word_boundary(&self.text, self.cursor), self.cursor));
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// Delete the selection, or the word after the caret.
    pub fn delete_word_forward(&mut self) {
        let (start, end) = self
            .selection()
            .unwrap_or_else(|| (self.cursor, next_word_boundary(&self.text, self.cursor)));
        if start == end {
            return;
        }
        self.push_undo(false);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        self.normalize();
    }

    /// Undo one step. Returns whether anything was undone.
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        self.coalescing = false;
        true
    }

    /// Redo one step. Returns whether anything was redone.
    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        self.coalescing = false;
        true
    }

    /// Drop the undo history, e.g. after loading a different document.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.coalescing = false;
    }

    /// Replace the pre-edit range with committed text (IME commit).
    fn commit_preedit(&mut self, text: &str, max_bytes: usize) {
        if let Some((start, end)) = self.preedit.take() {
            let start = floor_boundary(&self.text, start);
            let end = floor_boundary(&self.text, end).max(start);
            self.cursor = start;
            self.anchor = end;
        }
        self.insert_with(text, max_bytes, false);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.normalize();
    }

    /// Merge a typing run into one undo step.
    fn push_undo(&mut self, coalesce: bool) {
        if !(coalesce && self.coalescing) {
            let snapshot = self.snapshot();
            self.undo.push(snapshot);
            // A long editing session must not grow without bound.
            if self.undo.len() > 256 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.coalescing = coalesce;
    }

    /// Keep the caret on a boundary and inside the buffer.
    fn normalize(&mut self) {
        self.cursor = floor_boundary(&self.text, self.cursor);
        self.anchor = floor_boundary(&self.text, self.anchor);
    }
}

/// Evolve a text field for one event.
pub fn reduce_text_input(
    desc: &TextInputDesc,
    edit: &mut TextEditState,
    event: &TextInputEvent,
) -> TextInputOutcome {
    if desc.disabled {
        return TextInputOutcome::default();
    }
    match event {
        TextInputEvent::Insert(text) => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.insert(text, desc.max_bytes);
            TextInputOutcome::changed()
        }
        TextInputEvent::Backspace => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.backspace();
            TextInputOutcome::changed()
        }
        TextInputEvent::Delete => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.delete_forward();
            TextInputOutcome::changed()
        }
        TextInputEvent::DeleteWordBack => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.delete_word_backward();
            TextInputOutcome::changed()
        }
        TextInputEvent::DeleteWordForward => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.delete_word_forward();
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveLeft { extend } => {
            let cursor = prev_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveRight { extend } => {
            let cursor = next_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveWordLeft { extend } => {
            let cursor = prev_word_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::MoveWordRight { extend } => {
            let cursor = next_word_boundary(edit.text(), edit.cursor());
            edit.set_cursor(cursor, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::Home { extend } => {
            edit.set_cursor(0, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::End { extend } => {
            let end = edit.text().len();
            edit.set_cursor(end, *extend);
            TextInputOutcome::changed()
        }
        TextInputEvent::SelectAll => {
            edit.select_all();
            TextInputOutcome::changed()
        }
        TextInputEvent::Copy => {
            if desc.password {
                return TextInputOutcome::default();
            }
            TextInputOutcome {
                copied: edit.selected_text().map(str::to_owned),
                ..TextInputOutcome::default()
            }
        }
        TextInputEvent::Cut => {
            if !desc.editable() || desc.password {
                return TextInputOutcome::default();
            }
            let Some(text) = edit.selected_text().map(str::to_owned) else {
                return TextInputOutcome::default();
            };
            edit.backspace();
            TextInputOutcome {
                changed: true,
                cut: Some(text),
                ..TextInputOutcome::default()
            }
        }
        TextInputEvent::Paste(text) => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.insert_with(text, desc.max_bytes, false);
            TextInputOutcome::changed()
        }
        TextInputEvent::Undo => TextInputOutcome {
            changed: edit.undo(),
            ..TextInputOutcome::default()
        },
        TextInputEvent::Redo => TextInputOutcome {
            changed: edit.redo(),
            ..TextInputOutcome::default()
        },
        TextInputEvent::PointerDown { offset } => {
            edit.drag_anchor = Some(*offset);
            edit.set_cursor(*offset, false);
            TextInputOutcome::changed()
        }
        TextInputEvent::PointerDrag { offset } => {
            let anchor = edit.drag_anchor.unwrap_or(edit.anchor());
            edit.anchor = floor_boundary(edit.text(), anchor);
            edit.set_cursor(*offset, true);
            TextInputOutcome::changed()
        }
        TextInputEvent::PointerUp => {
            edit.drag_anchor = None;
            TextInputOutcome::default()
        }
        TextInputEvent::DoubleClick { offset } => {
            if let Some((start, end)) = word_range(edit.text(), *offset) {
                edit.anchor = start;
                edit.cursor = end;
            } else {
                edit.set_cursor(*offset, false);
            }
            TextInputOutcome::changed()
        }
        TextInputEvent::TripleClick => {
            edit.select_all();
            TextInputOutcome::changed()
        }
        TextInputEvent::ImePreedit { start, end } => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            let start = floor_boundary(edit.text(), *start);
            let end = floor_boundary(edit.text(), *end).max(start);
            edit.preedit = Some((start, end));
            edit.cursor = end;
            edit.anchor = end;
            TextInputOutcome::changed()
        }
        TextInputEvent::ImeCommit { text } => {
            if !desc.editable() {
                return TextInputOutcome::default();
            }
            edit.commit_preedit(text, desc.max_bytes);
            TextInputOutcome::changed()
        }
    }
}

/// The measured text geometry the emitter draws.
///
/// The caller shapes the text (it owns the font, atlas and zoom) and hands
/// back the caret and selection rectangles in the same coordinate space as
/// [`TextInputDesc::rect`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextInputVisual {
    /// The caret rectangle.
    pub caret: Rect,
    /// The selection rectangle, if any text is selected.
    pub selection: Option<Rect>,
    /// The measured IME pre-edit rectangle, if a composition is active.
    pub preedit: Option<Rect>,
    /// Whether the caret is in its visible blink phase.
    pub caret_visible: bool,
}

impl Default for TextInputVisual {
    fn default() -> Self {
        Self {
            caret: Rect::ZERO,
            selection: None,
            preedit: None,
            caret_visible: true,
        }
    }
}

/// A text input bound to an identity, its edit state and its measured layout.
#[derive(Clone, Debug)]
pub struct TextInputWidget {
    /// The widget identity.
    pub node: u64,
    /// The descriptor.
    pub desc: TextInputDesc,
    /// The edit state.
    pub edit: TextEditState,
    /// The measured caret / selection geometry.
    pub visual: TextInputVisual,
}

impl WidgetDesc for TextInputWidget {
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
        emit_text_input(
            self.node,
            &self.desc,
            state,
            &self.edit,
            &self.visual,
            glyphs,
        )
    }
}

/// The pure emitter.
///
/// The primitive count is constant for a fixed glyph run: the selection and
/// caret slots are always written, and their opacity alone says whether they
/// are visible.
#[must_use]
pub fn emit_text_input(
    node: u64,
    desc: &TextInputDesc,
    state: ControlState,
    edit: &TextEditState,
    visual: &TextInputVisual,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let z = desc.z;
    let mut primitives = Vec::with_capacity(5 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if state.disabled {
        Token::DISABLED
    } else if state.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_RAISED
    };
    primitives.push(surface(rect, desc.radius, fill, z));

    let border = match desc.validation {
        ValidationState::Error => Token::INPUT_BORDER_ERROR,
        ValidationState::Warning => Token::INPUT_BORDER_WARNING,
        ValidationState::None => {
            if state.focused {
                Token::OUTLINE_STRONG
            } else {
                Token::OUTLINE
            }
        }
    };
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z,
    ));

    // The selection slot is always present; opacity carries its visibility.
    let mut selection = outlined(
        visual.selection.unwrap_or(Rect::ZERO),
        2.0,
        Token::NONE,
        Token::NONE,
        0.0,
        0.0,
        z,
    );
    selection.fill_token_or_uv = u32::from(Token::SELECTION.raw());
    selection.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::InnerShadow as u16,
    );
    set_opacity(
        &mut selection,
        if visual.selection.is_some() && !state.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(selection);

    // The caret slot: one pixel wide, hidden while the field is not focused.
    let mut caret = surface(visual.caret, 0.0, Token::PRIMARY, z);
    caret.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    set_opacity(
        &mut caret,
        if state.focused && !state.disabled && visual.caret_visible {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    // The IME pre-edit underline. Like the caret it is always in the pool;
    // the opacity says whether a composition is active.
    let mut preedit = surface(
        visual.preedit.unwrap_or(Rect::ZERO),
        0.0,
        Token::PREEDIT_UNDERLINE,
        z,
    );
    preedit.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
        PrimType::RoundedRect,
        Default::default(),
        SubElement::Content as u16,
    );
    set_opacity(
        &mut preedit,
        if edit.preedit().is_some() && visual.preedit.is_some() && !state.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(preedit);

    // The glyphs, positioned by the caller.
    let ink = if state.disabled {
        Token::TEXT_DISABLED
    } else if edit.text().is_empty() && !desc.placeholder.is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    for quad in glyphs.iter() {
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
        let mut glyph = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [quad.rect.origin.x, quad.rect.origin.y],
            size: [quad.rect.size.x, quad.rect.size.y],
            corner_radii: f16::pack_radii([0.0; 4]),
            border_width: f16::f32_to_f16(0.0),
            shadow_blur: f16::f32_to_f16(0.0),
            transform_idx: 0,
            clip_idx: CLIP_NONE,
            fill_token_or_uv: u32::from(ink.raw()),
            border_token_or_tex: quad.layer,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                if quad.color {
                    PrimType::GlyphColor
                } else {
                    PrimType::Glyph
                },
                Default::default(),
                SubElement::Content as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: payload_idx,
        };
        if quad.msdf {
            glyph.prim_type_and_flags |= u32::from(unbound_gpu::data::PrimFlags::MSDF.bits()) << 16;
        }
        primitives.push(glyph);
    }

    primitives.push(focus_ring(
        rect.inflate(FOCUS_RING_GAP),
        desc.radius + FOCUS_RING_GAP,
        Token::FOCUS_RING,
        2.0,
        z,
        state.focused,
    ));

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, TEXT_INPUT_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::TextInput, desc.accessible_label(), rect)
        .clickable()
        .disabled(state.disabled);
    a11y.value = desc.display_text(edit.text());
    a11y.actions.insert(A11yAction::SetValue);
    a11y.actions.insert(A11yAction::Focus);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

fn set_opacity(primitive: &mut GpuPrimitiveInstance, opacity: f32) {
    primitive.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(primitive.depth_u16(), opacity);
}

/// The nearest character boundary at or before `offset`.
#[must_use]
pub fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The previous character boundary.
#[must_use]
pub fn prev_boundary(text: &str, offset: usize) -> usize {
    let offset = floor_boundary(text, offset);
    if offset == 0 {
        return 0;
    }
    let mut index = offset - 1;
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The next character boundary.
#[must_use]
pub fn next_boundary(text: &str, offset: usize) -> usize {
    let offset = floor_boundary(text, offset);
    if offset >= text.len() {
        return text.len();
    }
    let mut index = offset + 1;
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index.min(text.len())
}

/// The previous word boundary.
///
/// `unicode_segmentation` is not a dependency of this crate, so a word is a
/// run of `char::is_alphanumeric` (or `_`) scalars and everything else —
/// spaces and punctuation — is skipped. This is the same definition
/// [`word_range`] uses for double-click selection.
#[must_use]
pub fn prev_word_boundary(text: &str, offset: usize) -> usize {
    let mut index = floor_boundary(text, offset);
    while index > 0 {
        let previous = prev_boundary(text, index);
        let Some(character) = text[previous..index].chars().next() else {
            break;
        };
        if is_word_char(character) {
            break;
        }
        index = previous;
    }
    while index > 0 {
        let previous = prev_boundary(text, index);
        let Some(character) = text[previous..index].chars().next() else {
            break;
        };
        if !is_word_char(character) {
            break;
        }
        index = previous;
    }
    index
}

/// The next word boundary, skipping separators first.
#[must_use]
pub fn next_word_boundary(text: &str, offset: usize) -> usize {
    let mut index = floor_boundary(text, offset);
    while index < text.len() {
        let next = next_boundary(text, index);
        let Some(character) = text[index..next].chars().next() else {
            break;
        };
        if is_word_char(character) {
            break;
        }
        index = next;
    }
    while index < text.len() {
        let next = next_boundary(text, index);
        let Some(character) = text[index..next].chars().next() else {
            break;
        };
        if !is_word_char(character) {
            break;
        }
        index = next;
    }
    index
}

/// The word range around a byte offset.
///
/// Word characters are Unicode alphanumerics and `_`; punctuation and
/// whitespace separate words. Scanning outwards from the offset (rather than
/// relying on a segmenter) keeps a run of CJK ideographs together, which is
/// what double-click selection is expected to do.
#[must_use]
pub fn word_range(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = floor_boundary(text, offset);
    let mut start = offset;
    while start > 0 {
        let previous = prev_boundary(text, start);
        let character = text[previous..start].chars().next()?;
        if !is_word_char(character) {
            break;
        }
        start = previous;
    }
    let mut end = offset;
    while end < text.len() {
        let next = next_boundary(text, end);
        let character = text[end..next].chars().next()?;
        if !is_word_char(character) {
            break;
        }
        end = next;
    }
    if start == end {
        return None;
    }
    Some((start, end))
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WidgetHarness;

    fn desc() -> TextInputDesc {
        TextInputDesc::new(Rect::new(20.0, 20.0, 240.0, 36.0)).with_placeholder("type here")
    }

    fn reduce(edit: &mut TextEditState, event: TextInputEvent) -> TextInputOutcome {
        reduce_text_input(&desc(), edit, &event)
    }

    #[test]
    fn typing_and_backspacing_respect_char_boundaries() {
        let mut edit = TextEditState::new();
        reduce(&mut edit, TextInputEvent::Insert("你好".into()));
        assert_eq!(edit.text(), "你好");
        assert_eq!(edit.cursor(), 6);

        // One backspace removes one whole ideograph, not one byte.
        reduce(&mut edit, TextInputEvent::Backspace);
        assert_eq!(edit.text(), "你");
        assert!(edit.text().is_char_boundary(edit.cursor()));

        reduce(&mut edit, TextInputEvent::Insert("a".into()));
        assert_eq!(edit.text(), "你a");
        reduce(&mut edit, TextInputEvent::Backspace);
        assert_eq!(edit.text(), "你");
    }

    #[test]
    fn an_emoji_is_deleted_as_one_scalar_value() {
        let mut edit = TextEditState::with_text("a😀b");
        edit.set_cursor(5, false);
        assert!(edit.text().is_char_boundary(5));
        reduce(&mut edit, TextInputEvent::Backspace);
        assert_eq!(edit.text(), "ab");
        assert_eq!(edit.cursor(), 1);
    }

    #[test]
    fn the_cursor_never_lands_inside_a_multibyte_character() {
        let mut edit = TextEditState::with_text("你好");
        edit.set_cursor(1, false);
        assert_eq!(edit.cursor(), 0);
        edit.set_cursor(34, false);
        assert_eq!(edit.cursor(), 6);
    }

    #[test]
    fn shift_arrows_extend_the_selection_and_typing_replaces_it() {
        let mut edit = TextEditState::with_text("hello world");
        reduce(&mut edit, TextInputEvent::Home { extend: false });
        for _ in 0..5 {
            reduce(&mut edit, TextInputEvent::MoveRight { extend: true });
        }
        assert_eq!(edit.selected_text(), Some("hello"));
        reduce(&mut edit, TextInputEvent::Insert(",".into()));
        assert_eq!(edit.text(), ", world");
        assert_eq!(edit.cursor(), 1);
    }

    #[test]
    fn double_click_selects_a_word_including_cjk() {
        let mut edit = TextEditState::with_text("hello 世界 world");
        reduce(&mut edit, TextInputEvent::DoubleClick { offset: 8 });
        assert_eq!(edit.selected_text(), Some("世界"));

        let mut edit = TextEditState::with_text("hello 世界 world");
        reduce(&mut edit, TextInputEvent::DoubleClick { offset: 1 });
        assert_eq!(edit.selected_text(), Some("hello"));
    }

    #[test]
    fn dragging_selects_between_two_offsets() {
        let mut edit = TextEditState::with_text("abcdef");
        reduce(&mut edit, TextInputEvent::PointerDown { offset: 1 });
        reduce(&mut edit, TextInputEvent::PointerDrag { offset: 4 });
        assert_eq!(edit.selected_text(), Some("bcd"));
        reduce(&mut edit, TextInputEvent::PointerUp);
        assert!(edit.drag_anchor.is_none());
    }

    #[test]
    fn copy_and_cut_report_the_clipboard_payload() {
        let mut edit = TextEditState::with_text("abcdef");
        edit.anchor = 0;
        edit.cursor = 3;

        let outcome = reduce(&mut edit, TextInputEvent::Copy);
        assert_eq!(outcome.copied.as_deref(), Some("abc"));
        assert!(!outcome.changed, "copy must not edit");

        let outcome = reduce(&mut edit, TextInputEvent::Cut);
        assert_eq!(outcome.cut.as_deref(), Some("abc"));
        assert_eq!(edit.text(), "def");
    }

    #[test]
    fn undo_merges_a_typing_run_but_not_paste() {
        let mut edit = TextEditState::new();
        for character in ["a", "b", "c"] {
            reduce(&mut edit, TextInputEvent::Insert(character.into()));
        }
        assert_eq!(edit.text(), "abc");
        assert!(edit.undo());
        assert_eq!(edit.text(), "", "the whole typing run is one step");

        reduce(&mut edit, TextInputEvent::Paste("xyz".into()));
        reduce(&mut edit, TextInputEvent::Insert("!".into()));
        assert!(edit.undo());
        assert_eq!(edit.text(), "xyz");
        assert!(edit.undo());
        assert_eq!(edit.text(), "");
    }

    #[test]
    fn redo_restores_an_undone_edit() {
        let mut edit = TextEditState::new();
        reduce(&mut edit, TextInputEvent::Insert("a".into()));
        assert!(edit.undo());
        assert_eq!(edit.text(), "");
        assert!(edit.redo());
        assert_eq!(edit.text(), "a");
    }

    #[test]
    fn ime_preedit_is_replaced_by_the_commit() {
        let mut edit = TextEditState::with_text("ab");
        reduce(&mut edit, TextInputEvent::ImePreedit { start: 2, end: 2 });
        assert_eq!(edit.preedit(), Some((2, 2)));

        reduce(&mut edit, TextInputEvent::ImeCommit { text: "你".into() });
        assert_eq!(edit.text(), "ab你");
        assert_eq!(edit.preedit(), None);
    }

    #[test]
    fn the_byte_budget_never_splits_a_character() {
        let desc = desc().max_bytes(5);
        let mut edit = TextEditState::new();
        reduce_text_input(&desc, &mut edit, &TextInputEvent::Insert("你好".into()));
        assert_eq!(edit.text(), "你", "only the three-byte character fits");
    }

    #[test]
    fn a_disabled_or_read_only_field_rejects_edits_but_allows_navigation() {
        let mut edit = TextEditState::with_text("locked");
        let mut disabled = desc();
        disabled.disabled = true;
        assert!(
            !reduce_text_input(&disabled, &mut edit, &TextInputEvent::Insert("x".into())).changed
        );
        assert_eq!(edit.text(), "locked");

        let mut read_only = desc();
        read_only.read_only = true;
        assert!(!reduce_text_input(&read_only, &mut edit, &TextInputEvent::Backspace).changed);
        assert!(reduce_text_input(&read_only, &mut edit, &TextInputEvent::SelectAll).changed);
        assert_eq!(edit.selected_text(), Some("locked"));
    }

    #[test]
    fn the_emitter_reports_a_text_input_role_and_value() {
        let mut edit = TextEditState::with_text("abc");
        edit.anchor = 1;
        let widget = TextInputWidget {
            node: 5,
            desc: desc(),
            edit,
            visual: TextInputVisual::default(),
        };
        let mut harness = WidgetHarness::new(widget, Vec::new());
        harness.focus();
        let node = harness.output.a11y.as_ref().expect("a11y node");
        assert_eq!(node.role, Role::TextInput);
        assert_eq!(node.value, "abc");
        assert!(node.actions.contains(A11yAction::SetValue));
        assert_eq!(
            harness.output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let edit = TextEditState::with_text("abc");
        let glyphs: Vec<GlyphQuad> = Vec::new();
        let mut counts = Vec::new();
        for caret_visible in [false, true] {
            let visual = TextInputVisual {
                caret: Rect::new(30.0, 24.0, 1.0, 20.0),
                selection: Some(Rect::new(24.0, 24.0, 20.0, 20.0)),
                caret_visible,
                ..TextInputVisual::default()
            };
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let state = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output = emit_text_input(
                                9,
                                &desc(),
                                state,
                                &edit,
                                &visual,
                                &mut glyphs.clone(),
                            );
                            counts.push(output.primitives.len());
                        }
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

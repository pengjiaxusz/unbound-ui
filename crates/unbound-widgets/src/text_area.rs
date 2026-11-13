//! The multi-line editor: vertical caret movement over wrapped lines.
//!
//! [`TextEditState`] owns the buffer and every UTF-8 rule; this module adds
//! only what a *multi-line* field needs:
//!
//! * a newline insertion reducer,
//! * vertical movement over a caller-supplied list of visual lines with a
//!   remembered target column, so moving through a paragraph does not make the
//!   caret drift left, and
//! * a vertical scroll spring.
//!
//! The caller still owns shaping. It passes the wrapped lines as byte ranges
//! ([`AreaLine`]) and the merged selection slots, which keeps this module free
//! of font dependencies while producing the same rounded selection geometry
//! the rest of the text stack draws.

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::selection::RoundedSlot;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{
    floor_boundary, reduce_text_input, TextEditState, TextInputDesc, TextInputEvent,
};
use crate::{ControlState, WidgetOutput};

/// Horizontal padding between the border and the text.
pub const AREA_PADDING: f32 = 10.0;
/// The default corner radius.
pub const AREA_RADIUS: f32 = 8.0;

/// One visual (already wrapped) line as a byte range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaLine {
    /// The first byte of the line.
    pub start: usize,
    /// One past the last byte of the line.
    pub end: usize,
}

impl AreaLine {
    /// A line covering `start..end`.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// Split `text` on `\n`, ignoring `\r`.
///
/// The result always has at least one line, so `"a\n"` ends with an empty
/// line — the caret can sit there, which is what an editor is expected to do.
#[must_use]
pub fn logical_lines(text: &str) -> Vec<AreaLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            let end = if index > start && text.as_bytes()[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            lines.push(AreaLine::new(start, end));
            start = index + 1;
        }
    }
    lines.push(AreaLine::new(start, text.len()));
    lines
}

/// The pure descriptor of a multi-line field.
#[derive(Clone, Debug, PartialEq)]
pub struct TextAreaDesc {
    /// The field rectangle.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The placeholder shown while the buffer is empty.
    pub placeholder: String,
    /// The field refuses interaction.
    pub disabled: bool,
    /// The text can be selected but not edited.
    pub read_only: bool,
    /// The maximum buffer length in bytes.
    pub max_bytes: usize,
    /// The draw order key.
    pub z: u16,
    /// The corner radius.
    pub radius: f32,
    /// The line height used by the caller's shaping.
    pub line_height: f32,
}

impl TextAreaDesc {
    /// A field with the default metrics.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            read_only: false,
            max_bytes: 1 << 20,
            z: 725,
            radius: AREA_RADIUS,
            line_height: 20.0,
        }
    }

    /// The rectangle the text is drawn into.
    #[must_use]
    pub fn text_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + AREA_PADDING,
            self.rect.origin.y + AREA_PADDING * 0.5,
            (self.rect.size.x - AREA_PADDING * 2.0).max(0.0),
            (self.rect.size.y - AREA_PADDING).max(0.0),
        )
    }

    fn editable(&self) -> bool {
        !self.disabled && !self.read_only
    }
}

/// The live state of a multi-line field.
#[derive(Clone, Debug)]
pub struct TextAreaState {
    /// The shared text buffer, caret and selection.
    pub edit: TextEditState,
    /// The vertical column vertical movement tries to keep.
    preferred_column: Option<usize>,
    /// The vertical scroll offset.
    pub scroll: ScrollState,
}

impl TextAreaState {
    /// An empty field.
    #[must_use]
    pub fn new() -> Self {
        Self {
            edit: TextEditState::new(),
            preferred_column: None,
            scroll: ScrollState::new(),
        }
    }

    /// A field holding `text`.
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            edit: TextEditState::with_text(text),
            preferred_column: None,
            scroll: ScrollState::new(),
        }
    }

    /// The buffer.
    #[must_use]
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    /// The remembered target column, if vertical movement is in progress.
    #[must_use]
    pub fn preferred_column(&self) -> Option<usize> {
        self.preferred_column
    }

    /// Forget the target column; a horizontal edit moves the anchor.
    pub fn clear_preferred(&mut self) {
        self.preferred_column = None;
    }

    /// Insert a newline at the caret.
    pub fn insert_newline(&mut self, desc: &TextAreaDesc) -> bool {
        if !desc.editable() {
            return false;
        }
        self.preferred_column = None;
        self.edit.insert("\n", desc.max_bytes);
        true
    }

    /// Route a text event to the buffer, clearing the target column for any
    /// event that moves the caret horizontally.
    pub fn handle_event(
        &mut self,
        desc: &TextAreaDesc,
        event: &TextInputEvent,
    ) -> crate::text_input::TextInputOutcome {
        if matches!(
            event,
            TextInputEvent::MoveLeft { .. }
                | TextInputEvent::MoveRight { .. }
                | TextInputEvent::MoveWordLeft { .. }
                | TextInputEvent::MoveWordRight { .. }
                | TextInputEvent::Home { .. }
                | TextInputEvent::End { .. }
        ) {
            self.preferred_column = None;
        }
        let mut editor = TextInputDesc::new(desc.rect);
        editor.label = desc.label.clone();
        editor.disabled = desc.disabled;
        editor.read_only = desc.read_only;
        editor.max_bytes = desc.max_bytes;
        reduce_text_input(&editor, &mut self.edit, event)
    }

    /// The index of the line containing `cursor`.
    #[must_use]
    pub fn line_index(lines: &[AreaLine], cursor: usize) -> Option<usize> {
        if lines.is_empty() {
            return None;
        }
        lines
            .iter()
            .position(|line| cursor >= line.start && cursor <= line.end)
            .or(Some(lines.len() - 1))
    }

    /// The caret offset.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.edit.cursor()
    }

    /// Move the caret up or down one visual line.
    ///
    /// The byte column is remembered (`preferred_column`) so a caret that
    /// leaves a long line for a short one and comes back lands where it
    /// started instead of at the short line's end.
    pub fn move_vertical(&mut self, lines: &[AreaLine], delta: isize, extend: bool) -> bool {
        if lines.is_empty() {
            return false;
        }
        let cursor = self.caret();
        let Some(current) = Self::line_index(lines, cursor) else {
            return false;
        };
        let line = lines[current];
        let column = self
            .preferred_column
            .unwrap_or_else(|| cursor.saturating_sub(line.start));
        self.preferred_column = Some(column);
        let target = (current as isize + delta).clamp(0, lines.len() as isize - 1) as usize;
        if target == current {
            return false;
        }
        let target_line = lines[target];
        let offset = floor_boundary(
            self.edit.text(),
            (target_line.start + column).min(target_line.end),
        );
        self.edit.set_cursor(offset, extend);
        true
    }

    /// Move by a whole logical paragraph (`Ctrl+Up` / `Ctrl+Down`).
    pub fn move_paragraph(&mut self, delta: isize, extend: bool) -> bool {
        let lines = logical_lines(self.edit.text());
        self.move_vertical(&lines, delta, extend)
    }

    /// Scroll by a pixel delta, clamped to the content.
    pub fn scroll_by(&mut self, desc: &TextAreaDesc, dy: f32, content_height: f32) -> bool {
        let viewport = desc.text_rect();
        let scroll_desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
        let before = self.scroll.target().y;
        self.scroll.scroll_by(&scroll_desc, Vec2::new(0.0, dy));
        (self.scroll.target().y - before).abs() > f32::EPSILON
    }

    /// Advance the scroll spring.
    pub fn step(&mut self, desc: &TextAreaDesc, dt: f32, content_height: f32) -> bool {
        let viewport = desc.text_rect();
        let scroll_desc = ScrollViewDesc::new(viewport, Vec2::new(viewport.size.x, content_height));
        self.scroll.step(&scroll_desc, dt)
    }
}

impl Default for TextAreaState {
    fn default() -> Self {
        Self::new()
    }
}

/// The measured geometry the emitter draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextAreaVisual {
    /// The caret rectangle.
    pub caret: Rect,
    /// The merged selection slots, in draw order.
    pub selection: Vec<RoundedSlot>,
    /// Whether the caret is in its visible blink phase.
    pub caret_visible: bool,
}

/// Emit the field chrome, selection, caret and semantic node.
#[must_use]
pub fn emit_text_area(
    node: u64,
    desc: &TextAreaDesc,
    state: &TextAreaState,
    control: ControlState,
    visual: &TextAreaVisual,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(6 + visual.selection.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::INPUT_BG
    };
    primitives.push(surface(rect, desc.radius, fill, desc.z));
    let border = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.focused {
        Token::OUTLINE_STRONG
    } else {
        Token::OUTLINE
    };
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    for slot in &visual.selection {
        primitives.push(instance(
            slot.rect,
            slot.radii,
            Token::SELECTION,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::InnerShadow,
            desc.z,
        ));
    }

    // The caret slot is always present; opacity alone carries its visibility,
    // mirroring `emit_text_input`, so focus/blink never reshapes the pool.
    let mut caret = surface(visual.caret, 0.0, Token::PRIMARY, desc.z);
    caret.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        caret.depth_u16(),
        if control.focused && !control.disabled && visual.caret_visible {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    let ink = if control.disabled {
        Token::TEXT_DISABLED
    } else if state.edit.text().is_empty() && !desc.placeholder.is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(glyphs, ink, desc.z, &mut primitives, &mut payloads);

    primitives.push(focus_ring(
        rect.inflate(3.0),
        desc.radius + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [desc.radius; 4],
            })
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::MultilineTextInput, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.edit.text().to_owned();
    a11y.description = desc.placeholder.clone();
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

    fn desc() -> TextAreaDesc {
        TextAreaDesc::new(Rect::new(10.0, 10.0, 260.0, 120.0))
    }

    #[test]
    fn logical_lines_end_with_an_empty_line_after_a_newline() {
        let lines = logical_lines("ab\ncd\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], AreaLine::new(3, 5));
        assert_eq!(lines[2], AreaLine::new(6, 6));
        assert_eq!(logical_lines("").len(), 1);
    }

    #[test]
    fn inserting_a_newline_splits_the_buffer() {
        let desc = desc();
        let mut state = TextAreaState::with_text("ab");
        state.edit.set_cursor(1, false);
        assert!(state.insert_newline(&desc));
        assert_eq!(state.text(), "a\nb");
        assert_eq!(state.caret(), 2);
    }

    #[test]
    fn vertical_movement_keeps_the_byte_column() {
        let mut state = TextAreaState::with_text("abcdef\nxy\nabcdef");
        let lines = logical_lines(state.text());
        state.edit.set_cursor(5, false);
        assert!(state.move_vertical(&lines, 1, false));
        assert_eq!(state.caret(), 9, "the short line clamps to its end");
        assert!(state.move_vertical(&lines, 1, false));
        assert_eq!(state.caret(), 15, "the column is restored");
        assert!(state.move_vertical(&lines, -1, false));
        assert_eq!(state.caret(), 9);
        assert!(state.move_vertical(&lines, -3, false));
        assert_eq!(state.caret(), 5);
    }

    #[test]
    fn shifting_extends_the_selection_vertically() {
        let mut state = TextAreaState::with_text("aa\nbb\ncc");
        let lines = logical_lines(state.text());
        state.edit.set_cursor(1, false);
        assert!(state.move_vertical(&lines, 1, true));
        assert_eq!(state.edit.selected_text(), Some("a\nb"));
    }

    #[test]
    fn horizontal_events_clear_the_target_column() {
        let desc = desc();
        let mut state = TextAreaState::with_text("aa\nbb");
        let lines = logical_lines(state.text());
        state.edit.set_cursor(1, false);
        state.move_vertical(&lines, 1, false);
        assert!(state.preferred_column().is_some());
        state.handle_event(&desc, &TextInputEvent::MoveLeft { extend: false });
        assert!(state.preferred_column().is_none());
    }

    #[test]
    fn a_read_only_field_rejects_newlines() {
        let mut desc = desc();
        desc.read_only = true;
        let mut state = TextAreaState::with_text("a");
        assert!(!state.insert_newline(&desc));
        assert_eq!(state.text(), "a");
    }

    #[test]
    fn scrolling_clamps_to_the_content_height() {
        let desc = desc();
        let mut state = TextAreaState::new();
        let content = 1000.0;
        assert!(state.scroll_by(&desc, 5000.0, content));
        assert!(state.scroll.target().y > 0.0);
        assert!(state.scroll.target().y <= content);
        for _ in 0..120 {
            state.step(&desc, 1.0 / 60.0, content);
        }
        assert!(state.scroll.offset().y <= content);
        assert!(state.scroll_by(&desc, -5000.0, content));
        assert_eq!(state.scroll.target().y, 0.0);
    }

    #[test]
    fn the_emitter_reports_a_multiline_role_and_value() {
        let state = TextAreaState::with_text("one\ntwo");
        let visual = TextAreaVisual {
            caret: Rect::new(20.0, 20.0, 1.0, 18.0),
            selection: vec![RoundedSlot {
                rect: Rect::new(20.0, 20.0, 40.0, 18.0),
                radii: [3.0; 4],
            }],
            caret_visible: true,
        };
        let output = emit_text_area(
            5,
            &desc(),
            &state,
            ControlState::default(),
            &visual,
            &mut [],
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::MultilineTextInput);
        assert_eq!(a11y.value, "one\ntwo");
        // Surface, border, focus ring and the always-present caret slot.
        assert_eq!(output.primitives.len(), 4 + visual.selection.len());
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let state = TextAreaState::with_text("one\ntwo");
        let mut counts = Vec::new();
        for caret_visible in [false, true] {
            let visual = TextAreaVisual {
                caret: Rect::new(20.0, 20.0, 1.0, 18.0),
                selection: vec![RoundedSlot {
                    rect: Rect::new(20.0, 20.0, 40.0, 18.0),
                    radii: [3.0; 4],
                }],
                caret_visible,
            };
            for phase in [
                crate::PointerPhase::Idle,
                crate::PointerPhase::Hovered,
                crate::PointerPhase::Pressed,
            ] {
                for focused in [false, true] {
                    for disabled in [false, true] {
                        for selected in [false, true] {
                            let control = ControlState {
                                phase,
                                focused,
                                disabled,
                                selected,
                            };
                            let output =
                                emit_text_area(5, &desc, &state, control, &visual, &mut []);
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

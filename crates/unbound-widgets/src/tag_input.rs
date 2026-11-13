//! The tag field: chips, a trailing editor and comma / Enter commitment.
//!
//! A tag field is two controls in one box: committed chips and a live editor.
//! The state machine keeps them separate so deleting a chip never touches the
//! text buffer, and typing `,` (or pasting `"a, b"`) commits every segment in
//! one reduction instead of leaving a comma in the buffer.
//!
//! Layout is caller-measured: the widget cannot shape text, so
//! [`layout_tags`] takes a `measure` closure. That keeps wrapping exact for the
//! font actually in use and still makes the geometry unit-testable with a
//! deterministic stub.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{reduce_text_input, TextEditState, TextInputDesc, TextInputEvent};
use crate::{ControlState, WidgetOutput};

/// The height of one chip.
pub const TAG_HEIGHT: f32 = 24.0;
/// The gap between chips.
pub const TAG_GAP: f32 = 6.0;
/// The horizontal padding inside a chip.
pub const TAG_PADDING: f32 = 8.0;
/// The width reserved for a chip's close affordance.
pub const TAG_CLOSE: f32 = 14.0;
/// The inset between the border and the chips.
pub const TAG_INSET: f32 = 8.0;
/// The corner radius of the field and its chips.
pub const TAG_RADIUS: f32 = 8.0;

/// The pure descriptor of a tag field.
#[derive(Clone, Debug, PartialEq)]
pub struct TagInputDesc {
    /// The field rectangle.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The placeholder shown while there are no chips and no text.
    pub placeholder: String,
    /// The maximum number of chips.
    pub max_tags: usize,
    /// The maximum byte length of one chip's text.
    pub max_tag_bytes: usize,
    /// The field refuses interaction.
    pub disabled: bool,
    /// The draw order key.
    pub z: u16,
}

impl TagInputDesc {
    /// An empty tag field.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            max_tags: usize::MAX,
            max_tag_bytes: 64,
            disabled: false,
            z: 738,
        }
    }
}

/// The keys the tag field consumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagKey {
    /// Commit the editor's contents.
    Enter,
    /// Delete the last chip when the editor is empty.
    Backspace,
    /// Select the previous chip.
    Previous,
    /// Select the next chip.
    Next,
}

/// What one reduction produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagInputOutcome {
    /// The chips or the editor changed.
    pub changed: bool,
    /// The chips committed this reduction.
    pub committed: Vec<String>,
}

/// The live state of a tag field.
#[derive(Clone, Debug, PartialEq)]
pub struct TagInputState {
    tags: Vec<String>,
    input: TextEditState,
    selected: Option<usize>,
}

impl TagInputState {
    /// An empty field.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tags: Vec::new(),
            input: TextEditState::new(),
            selected: None,
        }
    }

    /// A field pre-filled with `tags`.
    #[must_use]
    pub fn with_tags(tags: impl IntoIterator<Item = String>) -> Self {
        Self {
            tags: tags.into_iter().collect(),
            input: TextEditState::new(),
            selected: None,
        }
    }

    /// The committed chips.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// The trailing editor.
    #[must_use]
    pub fn input(&self) -> &TextEditState {
        &self.input
    }

    /// The selected chip.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Make the trailing editor hold `text`.
    pub fn set_input(&mut self, text: impl Into<String>) {
        self.input = TextEditState::with_text(text);
    }

    /// Add one chip.
    ///
    /// Surrounding whitespace is trimmed. Empty, duplicate and over-long tags
    /// are rejected, as is a chip beyond `max_tags`.
    pub fn insert_tag(&mut self, text: &str, desc: &TagInputDesc) -> bool {
        let tag = text.trim();
        if tag.is_empty()
            || tag.len() > desc.max_tag_bytes
            || self.tags.iter().any(|existing| existing == tag)
            || self.tags.len() >= desc.max_tags
        {
            return false;
        }
        self.tags.push(tag.to_owned());
        true
    }

    /// Commit the editor's contents as one chip.
    pub fn commit_input(&mut self, desc: &TagInputDesc) -> bool {
        let text = self.input.text().to_owned();
        if !self.insert_tag(&text, desc) {
            return false;
        }
        self.input = TextEditState::new();
        true
    }

    /// Remove one chip.
    pub fn delete_tag(&mut self, index: usize) -> bool {
        if index >= self.tags.len() {
            return false;
        }
        self.tags.remove(index);
        self.selected = match self.selected {
            Some(selected) if selected > index => Some(selected - 1),
            Some(selected) if selected == index => {
                if self.tags.is_empty() {
                    None
                } else {
                    Some(index.min(self.tags.len() - 1))
                }
            }
            other => other,
        };
        true
    }

    /// Move the chip selection by whole chips, clamping at the ends.
    pub fn select_step(&mut self, delta: isize) -> bool {
        if self.tags.is_empty() {
            return false;
        }
        let current = self.selected.map_or(0, |index| index as isize);
        let next = (current + delta).clamp(0, self.tags.len() as isize - 1);
        let changed = self.selected != Some(next as usize);
        self.selected = Some(next as usize);
        changed
    }

    /// Route a text event, committing on a delimiter or on Enter.
    pub fn on_text_input(
        &mut self,
        desc: &TagInputDesc,
        event: &TextInputEvent,
    ) -> TagInputOutcome {
        if desc.disabled {
            return TagInputOutcome::default();
        }
        let mut editor = TextInputDesc::new(desc.rect);
        editor.label = desc.label.clone();
        editor.max_bytes = desc.max_tag_bytes;
        let outcome = reduce_text_input(&editor, &mut self.input, event);
        let mut committed = Vec::new();
        let mut changed = outcome.changed;

        if let TextInputEvent::Insert(text) = event {
            if text.contains(',') || text.contains('\n') {
                let buffer = self.input.text().to_owned();
                for segment in buffer.split([',', '\n']) {
                    if self.insert_tag(segment, desc) {
                        committed.push(segment.trim().to_owned());
                        changed = true;
                    }
                }
                self.input = TextEditState::new();
            }
        }
        TagInputOutcome { changed, committed }
    }

    /// Reduce a key.
    pub fn key(&mut self, desc: &TagInputDesc, key: TagKey) -> TagInputOutcome {
        match key {
            TagKey::Enter => {
                let before = self.tags.len();
                let changed = self.commit_input(desc);
                let committed = if self.tags.len() > before {
                    self.tags.last().cloned().into_iter().collect()
                } else {
                    Vec::new()
                };
                TagInputOutcome { changed, committed }
            }
            TagKey::Backspace => {
                if self.input.text().is_empty() && self.tags.pop().is_some() {
                    self.selected = None;
                    return TagInputOutcome {
                        changed: true,
                        committed: Vec::new(),
                    };
                }
                TagInputOutcome::default()
            }
            TagKey::Previous => TagInputOutcome {
                changed: self.select_step(-1),
                committed: Vec::new(),
            },
            TagKey::Next => TagInputOutcome {
                changed: self.select_step(1),
                committed: Vec::new(),
            },
        }
    }
}

impl Default for TagInputState {
    fn default() -> Self {
        Self::new()
    }
}

/// The measured chip geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagLayout {
    /// The chip rectangles, in model order.
    pub chips: Vec<Rect>,
    /// The close affordance of each chip.
    pub close: Vec<Rect>,
    /// The trailing editor.
    pub input: Rect,
    /// How many rows the chips occupy.
    pub lines: usize,
}

/// Wrap chips and place the trailing editor.
///
/// `measure` returns the text width of a chip's label. A chip's total width is
/// that plus its padding and close affordance.
#[must_use]
pub fn layout_tags(
    desc: &TagInputDesc,
    state: &TagInputState,
    measure: impl Fn(&str) -> f32,
) -> TagLayout {
    let content = desc.rect.inflate(-TAG_INSET);
    let mut x = content.origin.x;
    let mut y = content.origin.y;
    let mut chips = Vec::with_capacity(state.tags.len());
    let mut close = Vec::with_capacity(state.tags.len());
    let mut lines = 1usize;
    let row_end = content.max().x;

    for tag in &state.tags {
        let width = (measure(tag) + TAG_PADDING * 2.0 + TAG_CLOSE).max(TAG_HEIGHT);
        if x > content.origin.x && x + width > row_end {
            x = content.origin.x;
            y += TAG_HEIGHT + TAG_GAP;
            lines += 1;
        }
        let chip = Rect::new(x, y, width, TAG_HEIGHT);
        chips.push(chip);
        close.push(Rect::new(
            chip.max().x - TAG_CLOSE,
            chip.origin.y,
            TAG_CLOSE,
            chip.size.y,
        ));
        x = chip.max().x + TAG_GAP;
    }

    let remaining = (row_end - x).max(80.0);
    let input = Rect::new(x, y, remaining, TAG_HEIGHT);
    TagLayout {
        chips,
        close,
        input,
        lines,
    }
}

/// Emit the field chrome, chips and semantic node.
#[must_use]
pub fn emit_tag_input(
    node: u64,
    desc: &TagInputDesc,
    state: &TagInputState,
    control: ControlState,
    layout: &TagLayout,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(6 + layout.chips.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::INPUT_BG
    };
    primitives.push(surface(rect, TAG_RADIUS, fill, desc.z));
    primitives.push(outlined(
        rect,
        TAG_RADIUS,
        Token::NONE,
        if control.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if control.focused {
            Token::OUTLINE_STRONG
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        desc.z,
    ));

    for (index, chip) in layout.chips.iter().enumerate() {
        let selected = state.selected == Some(index);
        primitives.push(surface(
            *chip,
            TAG_HEIGHT * 0.5,
            Token::PRIMARY_SOFT,
            desc.z,
        ));
        if selected {
            primitives.push(outlined(
                *chip,
                TAG_HEIGHT * 0.5,
                Token::NONE,
                Token::PRIMARY,
                2.0,
                0.0,
                desc.z,
            ));
        }
        if let Some(close) = layout.close.get(index) {
            primitives.push(instance(
                Rect::new(close.center().x - 3.0, close.center().y - 0.75, 6.0, 1.5),
                [0.0; 4],
                Token::TEXT_SECONDARY,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Content,
                desc.z,
            ));
        }
    }

    // The empty-editor caret keeps a fixed slot: focus only changes its
    // opacity, so the primitive count is invariant across states.
    let mut caret = surface(
        Rect::new(
            layout.input.origin.x,
            layout.input.origin.y + 4.0,
            1.5,
            TAG_HEIGHT - 8.0,
        ),
        0.0,
        Token::PRIMARY,
        desc.z,
    );
    caret.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        caret.depth_u16(),
        if state.input.text().is_empty() && control.focused && !control.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    let ink = if control.disabled {
        Token::TEXT_DISABLED
    } else if state.tags.is_empty() && state.input.text().is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(glyphs, ink, desc.z, &mut primitives, &mut payloads);

    primitives.push(focus_ring(
        rect.inflate(3.0),
        TAG_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [TAG_RADIUS; 4],
            })
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.tags.join(", ");
    a11y.description = desc.placeholder.clone();

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

    fn desc() -> TagInputDesc {
        TagInputDesc::new(Rect::new(10.0, 10.0, 300.0, 80.0))
    }

    #[test]
    fn chips_are_trimmed_and_deduplicated() {
        let desc = desc();
        let mut state = TagInputState::new();
        assert!(state.insert_tag("  rust ", &desc));
        assert_eq!(state.tags(), ["rust"]);
        assert!(!state.insert_tag("rust", &desc));
        assert!(!state.insert_tag("   ", &desc));
        assert_eq!(state.tags().len(), 1);
    }

    #[test]
    fn limits_are_enforced() {
        let mut desc = desc();
        desc.max_tags = 3;
        desc.max_tag_bytes = 4;
        let mut state = TagInputState::new();
        assert!(state.insert_tag("one", &desc));
        assert!(state.insert_tag("two", &desc));
        assert!(state.insert_tag("four", &desc));
        assert!(!state.insert_tag("five", &desc), "max tags");
        assert!(!state.insert_tag("long", &desc));
        assert_eq!(state.tags().len(), 3);
    }

    #[test]
    fn committing_clears_the_editor() {
        let desc = desc();
        let mut state = TagInputState::new();
        state.set_input("hello");
        assert!(state.commit_input(&desc));
        assert_eq!(state.tags(), ["hello"]);
        assert!(state.input().text().is_empty());
    }

    #[test]
    fn a_comma_commits_every_segment() {
        let desc = desc();
        let mut state = TagInputState::new();
        let outcome = state.on_text_input(&desc, &TextInputEvent::Insert("a, b,c".into()));
        assert_eq!(outcome.committed, ["a", "b", "c"]);
        assert_eq!(state.tags(), ["a", "b", "c"]);
        assert!(state.input().text().is_empty());
    }

    #[test]
    fn backspace_on_an_empty_editor_pops_the_last_chip() {
        let desc = desc();
        let mut state = TagInputState::with_tags(["a".to_owned(), "b".to_owned()]);
        let outcome = state.key(&desc, TagKey::Backspace);
        assert!(outcome.changed);
        assert_eq!(state.tags(), ["a"]);
        assert!(state.key(&desc, TagKey::Backspace).changed);
        assert!(!state.key(&desc, TagKey::Backspace).changed);
    }

    #[test]
    fn deleting_a_chip_keeps_the_selection_valid() {
        let mut state = TagInputState::with_tags(["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        state.select_step(1);
        state.select_step(1);
        assert_eq!(state.selected(), Some(2));
        assert!(state.delete_tag(1));
        assert_eq!(state.selected(), Some(1));
        assert!(!state.delete_tag(9));
    }

    #[test]
    fn chips_wrap_and_the_editor_takes_the_rest_of_the_row() {
        let desc = TagInputDesc::new(Rect::new(0.0, 0.0, 120.0, 100.0));
        let state =
            TagInputState::with_tags(["one".to_owned(), "two".to_owned(), "three".to_owned()]);
        let layout = layout_tags(&desc, &state, |tag| tag.len() as f32 * 8.0);
        assert_eq!(layout.chips.len(), 3);
        assert!(layout.lines >= 2);
        assert!(layout.chips[2].origin.y > layout.chips[0].origin.y);
        assert!(layout.input.origin.x >= 0.0);
        assert!(layout.input.size.x >= 80.0);
    }

    #[test]
    fn the_emitter_reports_a_group_with_the_chip_values() {
        let desc = desc();
        let state = TagInputState::with_tags(["rust".to_owned(), "gpu".to_owned()]);
        let layout = layout_tags(&desc, &state, |tag| tag.len() as f32 * 8.0);
        let output = emit_tag_input(11, &desc, &state, ControlState::default(), &layout, &mut []);
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.value, "rust, gpu");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let state = TagInputState::new();
        let layout = layout_tags(&desc, &state, |tag| tag.len() as f32 * 8.0);
        let mut counts = Vec::new();
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
                        let output = emit_tag_input(11, &desc, &state, control, &layout, &mut []);
                        counts.push(output.primitives.len());
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

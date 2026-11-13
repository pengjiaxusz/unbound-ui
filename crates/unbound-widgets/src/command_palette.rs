//! The command palette: a search field over a ranked command list.
//!
//! Ranking is a pure function of `(query, command, recency)`, so the same
//! query always produces the same order and a test can assert it. The score
//! is deliberately coarse — prefix beats substring beats subsequence — and
//! recency only breaks ties, never promotes a non-match.

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The default row height.
pub const COMMAND_ROW: f32 = 32.0;

/// The search field height.
pub const COMMAND_INPUT: f32 = 44.0;

/// One palette command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// Stable identity.
    pub id: u64,
    /// The label shown in the list.
    pub title: String,
    /// The group heading.
    pub group: String,
    /// Extra words that match but are not shown.
    pub keywords: Vec<String>,
    /// The shortcut hint shown on the right.
    pub shortcut: Option<String>,
}

impl Command {
    /// A command with a title and group.
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, group: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            group: group.into(),
            keywords: Vec::new(),
            shortcut: None,
        }
    }

    /// Add hidden search keywords.
    #[must_use]
    pub fn with_keywords(mut self, keywords: &[&str]) -> Self {
        self.keywords = keywords.iter().map(|word| (*word).to_owned()).collect();
        self
    }

    /// Add a shortcut hint.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

/// The pure descriptor of a command palette.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandPaletteDesc {
    /// The palette rectangle.
    pub rect: Rect,
    /// The placeholder shown while the query is empty.
    pub placeholder: String,
    /// The row height.
    pub row_height: f32,
    /// The maximum number of rows shown at once.
    pub max_rows: usize,
    /// The draw order key.
    pub z: u16,
}

impl CommandPaletteDesc {
    /// A palette of the given size.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            placeholder: "Type a command…".to_owned(),
            row_height: COMMAND_ROW,
            max_rows: 8,
            z: 900,
        }
    }

    /// The search field.
    #[must_use]
    pub fn input_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            COMMAND_INPUT.min(self.rect.size.y),
        )
    }

    /// The list below the field.
    #[must_use]
    pub fn list_rect(&self) -> Rect {
        let input = self.input_rect();
        Rect::new(
            self.rect.origin.x,
            input.max().y,
            self.rect.size.x,
            (self.rect.size.y - input.size.y).max(0.0),
        )
    }

    /// The rectangle of the nth visible result.
    #[must_use]
    pub fn row_rect(&self, index: usize) -> Rect {
        let list = self.list_rect();
        Rect::new(
            list.origin.x,
            list.origin.y + index as f32 * self.row_height,
            list.size.x,
            self.row_height
                .min((list.max().y - list.origin.y - index as f32 * self.row_height).max(0.0)),
        )
    }
}

/// How well a command matches a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// No match.
    None,
    /// The letters appear in order, with gaps.
    Subsequence,
    /// The query appears somewhere in the title or a keyword.
    Substring,
    /// The title or a keyword starts with the query.
    Prefix,
}

/// Score one command against a query.
///
/// Returns `None` when nothing matches. The recency rank is a tie-breaker
/// worth at most 90 points, so it can never lift a subsequence above a
/// substring match.
#[must_use]
pub fn score_command(query: &str, command: &Command, recency: u32) -> Option<i32> {
    let query = query.trim().to_lowercase();
    let bonus = (recency.min(9) as i32) * 10;
    if query.is_empty() {
        return Some(bonus);
    }
    let title = command.title.to_lowercase();

    let mut best = MatchKind::None;
    let mut best_score = 0;
    for candidate in
        std::iter::once(title.as_str()).chain(command.keywords.iter().map(String::as_str))
    {
        let candidate = candidate.to_lowercase();
        let (kind, score) = if candidate.starts_with(&query) {
            (MatchKind::Prefix, 1000 - candidate.len() as i32)
        } else if let Some(position) = candidate.find(&query) {
            (MatchKind::Substring, 500 - position as i32)
        } else if let Some(gaps) = subsequence_gaps(&candidate, &query) {
            (MatchKind::Subsequence, 200 - gaps as i32)
        } else {
            (MatchKind::None, i32::MIN)
        };
        if kind > best {
            best = kind;
            best_score = score;
        } else if kind == best && kind != MatchKind::None {
            best_score = best_score.max(score);
        }
    }
    (best != MatchKind::None).then_some(best_score + bonus)
}

/// The number of skipped characters when `query` is a subsequence of `text`.
fn subsequence_gaps(text: &str, query: &str) -> Option<usize> {
    let mut chars = text.chars();
    let mut gaps = 0;
    for wanted in query.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(actual) if actual == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(gaps)
}

/// The live state of a command palette.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandPaletteState {
    /// Whether the palette is visible.
    pub open: bool,
    /// The current query.
    pub query: String,
    /// The index of the highlighted result.
    pub selected: usize,
    /// Every command known to the palette.
    pub commands: Vec<Command>,
    /// Usage counters; higher means more recent.
    recency: FxHashMap<u64, u32>,
    /// A monotonically increasing use counter.
    tick: u32,
}

impl CommandPaletteState {
    /// A closed palette over the given commands.
    #[must_use]
    pub fn new(commands: Vec<Command>) -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            commands,
            recency: FxHashMap::default(),
            tick: 0,
        }
    }

    /// Show the palette with a fresh query.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
    }

    /// Hide the palette.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Replace the query and reset the highlight.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.selected = 0;
    }

    /// The result indices, best first.
    #[must_use]
    pub fn results(&self) -> Vec<usize> {
        let mut scored: Vec<(i32, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let recency = self.recency.get(&command.id).copied().unwrap_or(0);
                score_command(&self.query, command, recency).map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, index)| index).collect()
    }

    /// The highlighted command, if any.
    #[must_use]
    pub fn selected_command(&self) -> Option<&Command> {
        let results = self.results();
        results
            .get(self.selected)
            .and_then(|index| self.commands.get(*index))
    }

    /// Move the highlight, wrapping around the result list.
    pub fn move_selection(&mut self, delta: isize) -> bool {
        let count = self.results().len();
        if count == 0 {
            return false;
        }
        let next = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        let changed = next != self.selected;
        self.selected = next;
        changed
    }

    /// Record that a command was invoked: it becomes the most recent.
    pub fn record_use(&mut self, id: u64) {
        self.tick = self.tick.saturating_add(1);
        self.recency.insert(id, self.tick);
    }

    /// The recency rank of a command.
    #[must_use]
    pub fn recency(&self, id: u64) -> u32 {
        self.recency.get(&id).copied().unwrap_or(0)
    }

    /// The results that fit in the list, in order.
    #[must_use]
    pub fn visible_results(&self, desc: &CommandPaletteDesc) -> Vec<usize> {
        let mut results = self.results();
        results.truncate(desc.max_rows);
        results
    }
}

/// The colour slots a command palette paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandPaletteTokens {
    /// The palette surface.
    pub surface: Token,
    /// The palette border.
    pub border: Token,
    /// The search field.
    pub input: Token,
    /// An idle row.
    pub row: Token,
    /// The highlighted row.
    pub row_selected: Token,
    /// The title text.
    pub text: Token,
    /// The title of the highlighted row.
    pub text_selected: Token,
    /// The group and shortcut hints.
    pub hint: Token,
    /// The keyboard focus ring.
    pub focus_ring: Token,
}

impl CommandPaletteTokens {
    /// The standard set.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            surface: Token::OVERLAY_BG,
            border: Token::OUTLINE,
            input: Token::SURFACE_SUNKEN,
            row: Token::NONE,
            row_selected: Token::PRIMARY_SOFT,
            text: Token::TEXT_PRIMARY,
            text_selected: Token::PRIMARY,
            hint: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// Emit the palette shell: surface, border, search field and list backdrop.
#[must_use]
pub fn emit_command_palette(
    desc: &CommandPaletteDesc,
    state: &CommandPaletteState,
    tokens: CommandPaletteTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4);
    primitives.push(outlined(
        desc.rect,
        12.0,
        tokens.surface,
        tokens.border,
        1.0,
        24.0,
        desc.z,
    ));
    primitives.push(surface(desc.input_rect(), 8.0, tokens.input, desc.z));
    if !state.open || state.results().is_empty() {
        primitives.push(surface(desc.list_rect(), 0.0, tokens.surface, desc.z));
    }
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
    }
}

/// Emit one result row.
#[must_use]
pub fn emit_command_row(
    desc: &CommandPaletteDesc,
    state: &CommandPaletteState,
    visible_index: usize,
    control: ControlState,
    label: &[GlyphQuad],
    shortcut: &[GlyphQuad],
    tokens: CommandPaletteTokens,
) -> WidgetOutput {
    let results = state.visible_results(desc);
    let Some(command_index) = results.get(visible_index).copied() else {
        return WidgetOutput::default();
    };
    let Some(command) = state.commands.get(command_index) else {
        return WidgetOutput::default();
    };
    let rect = desc.row_rect(visible_index);
    if rect.size.y <= 0.0 {
        return WidgetOutput::default();
    }
    let highlighted = visible_index == state.selected.min(results.len() - 1);
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(3 + label.len() + shortcut.len());
    let mut payloads = Vec::with_capacity(label.len() + shortcut.len());

    let fill = if highlighted {
        tokens.row_selected
    } else {
        tokens.row
    };
    primitives.push(outlined(rect, 6.0, fill, Token::NONE, 0.0, 0.0, desc.z));
    if highlighted {
        primitives.push(instance(
            Rect::new(rect.origin.x, rect.origin.y, 3.0, rect.size.y),
            [0.0; 4],
            tokens.text_selected,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }
    glyphs(
        label,
        if highlighted {
            tokens.text_selected
        } else {
            tokens.text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    glyphs(
        shortcut,
        tokens.hint,
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        7.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(command.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [6.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, command.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = highlighted;
    a11y.description = command.group.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<Command> {
        vec![
            Command::new(1, "Open File", "File").with_shortcut("Ctrl+O"),
            Command::new(2, "Close File", "File"),
            Command::new(3, "Open Folder", "File").with_keywords(&["directory", "workspace"]),
            Command::new(4, "Toggle Theme", "View").with_shortcut("Ctrl+T"),
            Command::new(5, "Rebuild", "Build"),
        ]
    }

    fn state() -> CommandPaletteState {
        CommandPaletteState::new(commands())
    }

    #[test]
    fn an_empty_query_lists_every_command() {
        let state = state();
        assert_eq!(state.results().len(), 5);
    }

    #[test]
    fn a_prefix_outranks_a_substring() {
        let mut state = state();
        state.set_query("open");
        let results = state.results();
        assert_eq!(results[0], 0, "Open File starts with the query");
        assert_eq!(results[1], 2, "Open Folder starts with the query");
        assert!(!results.contains(&1), "Close File is only a substring...");
    }

    #[test]
    fn hidden_keywords_participate_in_matching() {
        let mut state = state();
        state.set_query("workspace");
        assert_eq!(state.results(), vec![2]);
    }

    #[test]
    fn a_subsequence_matches_with_gaps() {
        let mut state = state();
        state.set_query("rbld");
        assert_eq!(state.results(), vec![4]);
    }

    #[test]
    fn recency_breaks_ties_but_cannot_invent_a_match() {
        let mut state = state();
        state.set_query("open");
        let before = state.results();
        state.record_use(2);
        assert!(
            !state.results().contains(&1),
            "Close File still does not match"
        );
        assert_eq!(state.results(), before);
        state.set_query("file");
        state.record_use(2);
        let after = state.results();
        assert_eq!(after[0], 1, "the most recent tie wins");
    }

    #[test]
    fn selection_wraps_and_moves() {
        let mut state = state();
        assert!(state.move_selection(1));
        assert_eq!(state.selected, 1);
        assert!(state.move_selection(-1));
        assert_eq!(state.selected, 0);
        assert!(state.move_selection(-1));
        assert_eq!(state.selected, 4);
        state.set_query("zzz");
        assert!(!state.move_selection(1));
    }

    #[test]
    fn the_selected_command_follows_the_query() {
        let mut state = state();
        state.set_query("toggle");
        assert_eq!(state.selected_command().map(|command| command.id), Some(4));
    }

    #[test]
    fn opening_resets_the_query() {
        let mut state = state();
        state.set_query("open");
        state.selected = 1;
        state.close();
        state.open();
        assert!(state.open);
        assert!(state.query.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn only_max_rows_are_visible() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        let state = state();
        assert_eq!(state.visible_results(&desc).len(), 5);
        let mut narrow = desc.clone();
        narrow.max_rows = 2;
        assert_eq!(state.visible_results(&narrow).len(), 2);
    }

    #[test]
    fn rows_stack_below_the_search_field() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        assert_eq!(desc.input_rect().size.y, COMMAND_INPUT);
        assert_eq!(desc.list_rect().origin.y, COMMAND_INPUT);
        assert_eq!(desc.row_rect(1).origin.y, COMMAND_INPUT + COMMAND_ROW);
    }

    #[test]
    fn the_highlighted_row_is_selected_and_pickable() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        let mut state = state();
        state.open();
        state.set_query("open");
        let tokens = CommandPaletteTokens::standard();
        let output = emit_command_row(&desc, &state, 0, ControlState::default(), &[], &[], tokens);
        let a11y = output.a11y.expect("a row is reachable");
        assert!(a11y.selected);
        assert_eq!(a11y.role, Role::Button);
        let hit = output.hit.expect("a row is pickable");
        assert_eq!(hit.node, 1);
    }

    #[test]
    fn an_out_of_range_row_emits_nothing() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        let state = state();
        let output = emit_command_row(
            &desc,
            &state,
            99,
            ControlState::default(),
            &[],
            &[],
            CommandPaletteTokens::standard(),
        );
        assert!(output.primitives.is_empty());
    }
}

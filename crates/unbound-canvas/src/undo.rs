//! Transactional undo/redo with copy-on-write snapshots (`docs` §5.3).
//!
//! The doc's shape is: `BeginTransaction` on pointer-down, a stream of cheap
//! `UpdateTransaction` calls while dragging, `CommitTransaction` on pointer-up,
//! and exactly **one** undo step at the end. That is what this module implements,
//! and it implements it as a *patch log* rather than a state snapshot:
//!
//! * a drag of three nodes on a 1 000 000 node canvas records three rectangles,
//!   not a million — the memory delta per step is a few dozen bytes, which is
//!   the property the doc's "single step ≤ a few KB" requirement is really
//!   about,
//! * the log is also the thing the reducer applies, so undo/redo cannot drift
//!   away from the state that produced it.
//!
//! Text is special: consecutive keystrokes must merge into one undo step, but a
//! word boundary or a 500 ms pause must start a new one. [`UndoStack::begin`]
//! decides that by looking at the still-open transaction, so the caller never
//! has to.

#![forbid(unsafe_op_in_unsafe_fn)]

use rustc_hash::FxHashMap;
use unbound_math::Rect;

/// How long two text edits may be apart and still merge, in seconds.
pub const TEXT_COALESCE_WINDOW: f64 = 0.5;

/// What kind of interaction opened the transaction.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TransactionKind {
    /// Anything that should never merge (a button press, a menu command).
    Discrete,
    /// A drag or resize; per-entity updates overwrite in place.
    Drag,
    /// Typing; merges within [`TEXT_COALESCE_WINDOW`] unless a word boundary is
    /// crossed.
    Text,
}

/// One reversible change.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Geometry of one entity.
    Geometry {
        id: u32,
        before: Rect,
        after: Rect,
    },
    /// A text buffer edit.
    Text { before: String, after: String },
    /// Creation or destruction of one entity.
    Exist { id: u32, existed: bool },
}

impl Edit {
    /// The same edit, pointed the other way.
    #[must_use]
    pub fn inverted(&self) -> Self {
        match self {
            Self::Geometry { id, before, after } => Self::Geometry {
                id: *id,
                before: *after,
                after: *before,
            },
            Self::Text { before, after } => Self::Text {
                before: after.clone(),
                after: before.clone(),
            },
            Self::Exist { id, existed } => Self::Exist {
                id: *id,
                existed: !existed,
            },
        }
    }

    /// Payload bytes, used by the KPI assertions.
    #[must_use]
    pub fn bytes(&self) -> usize {
        core::mem::size_of::<Self>()
            + match self {
                Self::Text { before, after } => before.len() + after.len(),
                _ => 0,
            }
    }

    /// Entity this edit touches, if it touches one.
    #[must_use]
    pub fn entity(&self) -> Option<u32> {
        match self {
            Self::Geometry { id, .. } | Self::Exist { id, .. } => Some(*id),
            Self::Text { .. } => None,
        }
    }
}

/// A committed (or in-progress) group of edits.
#[derive(Clone, Debug)]
pub struct Transaction {
    label: &'static str,
    kind: TransactionKind,
    edits: Vec<Edit>,
    /// Index of the geometry edit for each entity, so a drag overwrites instead
    /// of appending thousands of entries.
    geometry_slot: FxHashMap<u32, usize>,
    started: f64,
    last_edit: f64,
}

impl Transaction {
    fn new(label: &'static str, kind: TransactionKind, now: f64) -> Self {
        Self {
            label,
            kind,
            edits: Vec::new(),
            geometry_slot: FxHashMap::default(),
            started: now,
            last_edit: now,
        }
    }

    /// Human-readable label, e.g. `"drag 3 items"`.
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Kind that opened the transaction.
    #[must_use]
    pub fn kind(&self) -> TransactionKind {
        self.kind
    }

    /// Edits in application order.
    #[must_use]
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// True when nothing changed and the transaction should not be recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Logical time the transaction opened.
    #[must_use]
    pub fn started(&self) -> f64 {
        self.started
    }

    /// Logical time of the last recorded edit.
    #[must_use]
    pub fn last_edit(&self) -> f64 {
        self.last_edit
    }

    /// Payload bytes, for the KPI assertions.
    #[must_use]
    pub fn bytes(&self) -> usize {
        core::mem::size_of::<Self>() + self.edits.iter().map(Edit::bytes).sum::<usize>()
    }

    /// True when the last recorded text ends on a word boundary, which means
    /// the next keystroke must open a fresh undo step.
    fn ends_on_boundary(&self) -> bool {
        match self.edits.last() {
            Some(Edit::Text { after, .. }) => {
                after.chars().last().is_some_and(is_word_boundary)
            }
            _ => false,
        }
    }

    /// Iterate the inverse edits, newest first: exactly what undo must apply.
    pub fn inverted(&self) -> impl Iterator<Item = Edit> + '_ {
        self.edits.iter().rev().map(Edit::inverted)
    }
}

/// Undo/redo history.
#[derive(Debug)]
pub struct UndoStack {
    past: Vec<Transaction>,
    future: Vec<Transaction>,
    active: Option<Transaction>,
    limit: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(256)
    }
}

impl UndoStack {
    /// Create a stack that keeps at most `limit` committed steps.
    ///
    /// # Panics
    /// Panics when `limit` is zero.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        assert!(limit > 0, "history limit must be positive");
        Self {
            past: Vec::new(),
            future: Vec::new(),
            active: None,
            limit,
        }
    }

    /// Number of undoable steps.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.past.len()
    }

    /// Number of redoable steps.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.future.len()
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// True while a transaction is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// The open transaction, if any.
    #[must_use]
    pub fn active(&self) -> Option<&Transaction> {
        self.active.as_ref()
    }

    /// Open a transaction.
    ///
    /// An open text transaction is *kept* when the new one is also text and the
    /// gap is within [`TEXT_COALESCE_WINDOW`] — that is the merge rule. Anything
    /// else commits the previous transaction first, so a drag that is interrupted
    /// by a keyboard shortcut still lands in history.
    pub fn begin(&mut self, kind: TransactionKind, label: &'static str, now: f64) {
        let mergeable = self.active.as_ref().is_some_and(|active| {
            active.kind == TransactionKind::Text
                && kind == TransactionKind::Text
                && now - active.last_edit <= TEXT_COALESCE_WINDOW
                && !active.ends_on_boundary()
        });
        if mergeable {
            return;
        }
        self.commit();
        self.active = Some(Transaction::new(label, kind, now));
    }

    /// Record a geometry change, returning `false` when nothing moved.
    ///
    /// Repeated calls for the same entity during a drag overwrite the recorded
    /// `after` rect and keep the original `before`, so a 120 Hz drag of a second
    /// still produces a single entry.
    pub fn record_geometry(&mut self, id: u32, before: Rect, after: Rect, now: f64) -> bool {
        if before == after {
            return false;
        }
        let Some(active) = &mut self.active else {
            return false;
        };
        active.last_edit = now;
        if let Some(slot) = active.geometry_slot.get(&id) {
            if let Edit::Geometry { after: stored, .. } = &mut active.edits[*slot] {
                *stored = after;
            }
            return true;
        }
        let slot = active.edits.len();
        active.edits.push(Edit::Geometry { id, before, after });
        active.geometry_slot.insert(id, slot);
        true
    }

    /// Record a text change, merging with the previous keystroke when the doc
    /// allows it.
    ///
    /// Merging requires the new buffer to be an extension of the previous one
    /// (typing at the end of what was just typed) and the appended character to
    /// be a normal word character. Space, punctuation and newlines end the run,
    /// which is what makes "undo" step back a word at a time.
    pub fn record_text(&mut self, before: &str, after: &str, now: f64) -> bool {
        if before == after {
            return false;
        }
        let Some(active) = &mut self.active else {
            return false;
        };
        active.last_edit = now;
        if active.kind != TransactionKind::Text {
            active.edits.push(Edit::Text {
                before: before.to_owned(),
                after: after.to_owned(),
            });
            return true;
        }
        if let Some(Edit::Text {
            after: previous, ..
        }) = active.edits.last_mut()
        {
            let extends = after.starts_with(previous.as_str()) && after.len() > previous.len();
            let appended_is_word = after[previous.len()..]
                .chars()
                .next()
                .is_some_and(|c| !is_word_boundary(c));
            if extends && appended_is_word {
                *previous = after.to_owned();
                return true;
            }
        }
        active.edits.push(Edit::Text {
            before: before.to_owned(),
            after: after.to_owned(),
        });
        true
    }

    /// Record that an entity appeared or disappeared.
    pub fn record_exist(&mut self, id: u32, existed: bool, now: f64) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        active.last_edit = now;
        active.edits.push(Edit::Exist { id, existed });
        true
    }

    /// Commit the open transaction. Empty transactions are dropped, so a click
    /// without movement never pollutes the history.
    pub fn commit(&mut self) -> Option<&Transaction> {
        let active = self.active.take()?;
        if active.is_empty() {
            return None;
        }
        self.past.push(active);
        self.future.clear();
        if self.past.len() > self.limit {
            let excess = self.past.len() - self.limit;
            self.past.drain(0..excess);
        }
        self.past.last()
    }

    /// Throw the open transaction away (Escape during a drag).
    pub fn abort(&mut self) {
        self.active = None;
    }

    /// Pop the newest committed transaction and hand it to the caller, which is
    /// expected to apply [`Transaction::inverted`].
    pub fn undo(&mut self) -> Option<&Transaction> {
        if self.active.is_some() {
            self.commit();
        }
        let entry = self.past.pop()?;
        self.future.push(entry);
        self.future.last()
    }

    /// Pop the newest undone transaction and hand it to the caller, which is
    /// expected to apply [`Transaction::edits`] in order.
    pub fn redo(&mut self) -> Option<&Transaction> {
        let entry = self.future.pop()?;
        self.past.push(entry);
        self.past.last()
    }

    /// Total payload bytes held by the history, for the KPI assertions.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.past.iter().map(Transaction::bytes).sum::<usize>()
            + self.future.iter().map(Transaction::bytes).sum::<usize>()
            + self.active.as_ref().map_or(0, Transaction::bytes)
    }

    /// Label of the step [`Self::undo`] would revert.
    #[must_use]
    pub fn undo_label(&self) -> Option<&'static str> {
        self.past.last().map(Transaction::label)
    }

    /// Label of the step [`Self::redo`] would re-apply.
    #[must_use]
    pub fn redo_label(&self) -> Option<&'static str> {
        self.future.last().map(Transaction::label)
    }
}

/// Characters that end a text-coalescing run.
///
/// Covers ASCII whitespace and punctuation plus the CJK punctuation that a
/// Chinese IME commits, because for those languages the "word" boundary is a
/// full-width character rather than a space.
#[must_use]
pub fn is_word_boundary(c: char) -> bool {
    c.is_whitespace() || c.is_ascii_punctuation() || is_cjk_punctuation(c)
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{303F}'   // CJK symbols and punctuation
        | '\u{FF00}'..='\u{FF65}' // full-width forms
        | '\u{2018}'..='\u{201F}' // curly quotes
        | '\u{2026}' | '\u{2014}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_drag_of_many_frames_is_one_undo_step() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        let start = rect(0.0, 0.0, 10.0, 10.0);
        for frame in 1..=120u32 {
            let now = rect(f32::from(frame as u16), 0.0, 10.0, 10.0);
            stack.record_geometry(7, start, now, f64::from(frame) / 120.0);
        }
        stack.commit();
        assert_eq!(stack.depth(), 1);
        let txn = &stack.past[0];
        assert_eq!(txn.edits().len(), 1, "one entity, one entry");
        match &txn.edits()[0] {
            Edit::Geometry { before, after, .. } => {
                assert_eq!(*before, start);
                assert_eq!(*after, rect(120.0, 0.0, 10.0, 10.0));
            }
            other => panic!("unexpected edit {other:?}"),
        }
    }

    #[test]
    fn a_click_without_movement_records_nothing() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        assert!(!stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(0.0, 0.0, 1.0, 1.0), 0.1));
        assert!(stack.commit().is_none());
        assert_eq!(stack.depth(), 0);
        assert!(!stack.can_undo());
    }

    #[test]
    fn abort_discards_the_open_transaction() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(5.0, 5.0, 1.0, 1.0), 0.1);
        stack.abort();
        assert!(!stack.is_open());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "move a", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(10.0, 0.0, 1.0, 1.0), 0.1);
        stack.commit();
        stack.begin(TransactionKind::Drag, "move b", 0.2);
        stack.record_geometry(2, rect(0.0, 0.0, 1.0, 1.0), rect(20.0, 0.0, 1.0, 1.0), 0.3);
        stack.commit();

        let undo = stack.undo().unwrap();
        assert_eq!(undo.label(), "move b");
        let inverse: Vec<_> = undo.inverted().collect();
        assert_eq!(
            inverse[0],
            Edit::Geometry {
                id: 2,
                before: rect(20.0, 0.0, 1.0, 1.0),
                after: rect(0.0, 0.0, 1.0, 1.0)
            }
        );
        assert_eq!(stack.redo_depth(), 1);

        stack.undo();
        assert!(!stack.can_undo());
        assert_eq!(stack.redo_depth(), 2);

        let redo = stack.redo().unwrap();
        assert_eq!(redo.label(), "move a");
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.redo_depth(), 1);
    }

    #[test]
    fn a_new_commit_clears_the_redo_stack() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Discrete, "a", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(1.0, 0.0, 1.0, 1.0), 0.1);
        stack.commit();
        stack.undo();
        assert_eq!(stack.redo_depth(), 1);
        stack.begin(TransactionKind::Discrete, "b", 0.2);
        stack.record_geometry(1, rect(1.0, 0.0, 1.0, 1.0), rect(2.0, 0.0, 1.0, 1.0), 0.3);
        stack.commit();
        assert_eq!(stack.redo_depth(), 0, "branching must drop the redo tail");
    }

    #[test]
    fn typing_merges_into_one_step() {
        let mut stack = UndoStack::new(16);
        let mut now = 0.0;
        let mut text = String::new();
        for c in "hello".chars() {
            stack.begin(TransactionKind::Text, "type", now);
            let before = text.clone();
            text.push(c);
            assert!(stack.record_text(&before, &text, now));
            now += 0.1;
        }
        stack.commit();
        assert_eq!(stack.depth(), 1, "five keystrokes must be one step");
        match &stack.past[0].edits()[0] {
            Edit::Text { before, after } => {
                assert_eq!(before, "");
                assert_eq!(after, "hello");
            }
            other => panic!("unexpected edit {other:?}"),
        }
    }

    #[test]
    fn a_space_starts_a_new_undo_step() {
        let mut stack = UndoStack::new(16);
        let mut now = 0.0;
        let mut text = String::new();
        for c in "ab cd".chars() {
            stack.begin(TransactionKind::Text, "type", now);
            let before = text.clone();
            text.push(c);
            stack.record_text(&before, &text, now);
            now += 0.1;
        }
        stack.commit();
        // "ab " is one step, "cd" is another.
        assert_eq!(stack.depth(), 2, "{:?}", stack.past);
    }

    #[test]
    fn a_long_pause_starts_a_new_undo_step() {
        let mut stack = UndoStack::new(16);
        let mut text = String::new();
        stack.begin(TransactionKind::Text, "type", 0.0);
        let before = text.clone();
        text.push('a');
        stack.record_text(&before, &text, 0.0);
        stack.commit();

        stack.begin(TransactionKind::Text, "type", 10.0);
        let before = text.clone();
        text.push('b');
        stack.record_text(&before, &text, 10.0);
        stack.commit();
        assert_eq!(stack.depth(), 2);
    }

    #[test]
    fn a_non_extension_starts_a_new_edit_within_the_transaction() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Text, "type", 0.0);
        stack.record_text("ab", "abc", 0.1);
        // The user moved the caret and typed elsewhere: not an extension.
        stack.record_text("abc", "Xabc", 0.2);
        stack.commit();
        assert_eq!(stack.past[0].edits().len(), 2);
    }

    #[test]
    fn history_respects_its_limit() {
        let mut stack = UndoStack::new(3);
        for i in 0..10u32 {
            stack.begin(TransactionKind::Discrete, "step", f64::from(i));
            stack.record_geometry(i, rect(0.0, 0.0, 1.0, 1.0), rect(1.0, 0.0, 1.0, 1.0), f64::from(i));
            stack.commit();
        }
        assert_eq!(stack.depth(), 3);
    }

    #[test]
    fn a_drag_on_a_million_node_canvas_costs_bytes_not_megabytes() {
        let mut stack = UndoStack::new(128);
        // Only three of the million nodes are selected.
        stack.begin(TransactionKind::Drag, "drag 3", 0.0);
        for frame in 0..600u32 {
            for id in [11u32, 22, 33] {
                let x = frame as f32;
                stack.record_geometry(
                    id,
                    rect(0.0, id as f32, 10.0, 10.0),
                    rect(x, id as f32, 10.0, 10.0),
                    f64::from(frame) / 120.0,
                );
            }
        }
        stack.commit();
        assert_eq!(stack.past[0].edits().len(), 3);
        assert!(
            stack.bytes() < 1024,
            "history grew to {} bytes",
            stack.bytes()
        );
    }

    #[test]
    fn opening_a_second_transaction_commits_the_first() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(3.0, 0.0, 1.0, 1.0), 0.1);
        stack.begin(TransactionKind::Discrete, "delete", 0.2);
        assert_eq!(stack.depth(), 1, "the interrupted drag must be preserved");
        assert!(stack.is_open());
    }

    #[test]
    fn undo_commits_an_open_transaction_first() {
        let mut stack = UndoStack::new(16);
        stack.begin(TransactionKind::Drag, "drag", 0.0);
        stack.record_geometry(1, rect(0.0, 0.0, 1.0, 1.0), rect(3.0, 0.0, 1.0, 1.0), 0.1);
        let undone = stack.undo().unwrap();
        assert_eq!(undone.label(), "drag");
        assert!(!stack.is_open());
    }

    #[test]
    fn existence_edits_invert_cleanly() {
        let edit = Edit::Exist {
            id: 4,
            existed: true,
        };
        assert_eq!(
            edit.inverted(),
            Edit::Exist {
                id: 4,
                existed: false
            }
        );
        assert_eq!(edit.entity(), Some(4));
    }

    #[test]
    fn word_boundaries_cover_ascii_and_cjk() {
        assert!(is_word_boundary(' '));
        assert!(is_word_boundary('\n'));
        assert!(is_word_boundary(','));
        assert!(is_word_boundary('，'));
        assert!(is_word_boundary('。'));
        assert!(!is_word_boundary('a'));
        assert!(!is_word_boundary('中'));
    }
}

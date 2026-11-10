//! The deterministic cursor state stack (`docs` §6.2).
//!
//! Cursor resolution is a *priority* problem, not a "last writer wins" problem:
//! a pointer captured by a scrollbar thumb must keep showing `grabbing` even
//! while it flies over a text field, and a resize handle must beat the canvas
//! tool behind it. The doc fixes the order:
//!
//! ```text
//! Active Pointer Capture > Overlay > Resize Handle > Canvas Tool > Text > Default
//! ```
//!
//! Each *source* contributes at most one entry, so pushing the same source every
//! frame cannot grow the stack, and ties inside one source are broken by an
//! insertion sequence number — which makes the resolved cursor a pure function
//! of the frame's inputs.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_canvas::Handle;

/// What the OS pointer should look like.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CursorKind {
    #[default]
    Default,
    /// Hand over something clickable.
    Pointer,
    /// I-beam over selectable text.
    Text,
    /// Open hand: content can be panned.
    Grab,
    /// Closed hand: content is being panned.
    Grabbing,
    /// Precise target, e.g. the rotation grip.
    Crosshair,
    /// Four-way move.
    Move,
    ResizeNorthSouth,
    ResizeEastWest,
    ResizeNorthWestSouthEast,
    ResizeNorthEastSouthWest,
    NotAllowed,
    /// Platform-defined cursor id.
    Custom(u16),
}

impl CursorKind {
    /// Map a transform handle to its cursor.
    #[must_use]
    pub fn for_handle(handle: Handle) -> Self {
        match handle {
            Handle::North | Handle::South => Self::ResizeNorthSouth,
            Handle::East | Handle::West => Self::ResizeEastWest,
            Handle::NorthWest | Handle::SouthEast => Self::ResizeNorthWestSouthEast,
            Handle::NorthEast | Handle::SouthWest => Self::ResizeNorthEastSouthWest,
            Handle::Rotate => Self::Crosshair,
        }
    }

    /// Which stack slot this cursor competes for.
    #[must_use]
    pub fn source(self) -> CursorSource {
        match self {
            Self::ResizeNorthSouth
            | Self::ResizeEastWest
            | Self::ResizeNorthWestSouthEast
            | Self::ResizeNorthEastSouthWest => CursorSource::Handle,
            Self::Text => CursorSource::Text,
            Self::Grab | Self::Grabbing | Self::Crosshair | Self::Move => CursorSource::CanvasTool,
            _ => CursorSource::Default,
        }
    }
}

/// Who is asking for the cursor. The numeric value *is* the priority.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CursorSource {
    Default = 0,
    Text = 1,
    CanvasTool = 2,
    Handle = 3,
    Overlay = 4,
    Capture = 5,
}

#[derive(Copy, Clone, Debug)]
struct Entry {
    source: CursorSource,
    kind: CursorKind,
    seq: u64,
}

/// Priority-ordered cursor requests for one frame.
#[derive(Clone, Debug, Default)]
pub struct CursorStack {
    entries: Vec<Entry>,
    seq: u64,
}

impl CursorStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a frame: drop every request from the previous one.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Request a cursor for `source`, replacing any earlier request from it.
    pub fn push(&mut self, source: CursorSource, kind: CursorKind) {
        self.seq += 1;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.source == source) {
            entry.kind = kind;
            entry.seq = self.seq;
            return;
        }
        self.entries.push(Entry {
            source,
            kind,
            seq: self.seq,
        });
    }

    /// Drop the request from `source`.
    pub fn pop(&mut self, source: CursorSource) {
        self.entries.retain(|e| e.source != source);
    }

    /// Highest-priority request, or [`CursorKind::Default`].
    #[must_use]
    pub fn resolve(&self) -> CursorKind {
        self.entries
            .iter()
            .max_by_key(|e| (e.source, e.seq))
            .map_or(CursorKind::Default, |e| e.kind)
    }

    /// Number of sources currently requesting a cursor.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_beats_everything() {
        let mut stack = CursorStack::new();
        stack.push(CursorSource::Text, CursorKind::Text);
        stack.push(CursorSource::Handle, CursorKind::ResizeEastWest);
        stack.push(CursorSource::Overlay, CursorKind::Pointer);
        stack.push(CursorSource::Capture, CursorKind::Grabbing);
        assert_eq!(stack.resolve(), CursorKind::Grabbing);
    }

    #[test]
    fn releasing_the_capture_falls_back_in_priority_order() {
        let mut stack = CursorStack::new();
        stack.push(CursorSource::Text, CursorKind::Text);
        stack.push(CursorSource::Handle, CursorKind::ResizeNorthSouth);
        stack.push(CursorSource::Capture, CursorKind::Grabbing);
        stack.pop(CursorSource::Capture);
        assert_eq!(stack.resolve(), CursorKind::ResizeNorthSouth);
        stack.pop(CursorSource::Handle);
        assert_eq!(stack.resolve(), CursorKind::Text);
    }

    #[test]
    fn one_slot_per_source() {
        let mut stack = CursorStack::new();
        for _ in 0..10 {
            stack.push(CursorSource::CanvasTool, CursorKind::Grab);
        }
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn clears_between_frames() {
        let mut stack = CursorStack::new();
        stack.push(CursorSource::Handle, CursorKind::ResizeEastWest);
        stack.clear();
        assert_eq!(stack.resolve(), CursorKind::Default);
        assert!(stack.is_empty());
    }

    #[test]
    fn handles_map_to_the_right_resize_cursors() {
        assert_eq!(
            CursorKind::for_handle(Handle::North),
            CursorKind::ResizeNorthSouth
        );
        assert_eq!(
            CursorKind::for_handle(Handle::West),
            CursorKind::ResizeEastWest
        );
        assert_eq!(
            CursorKind::for_handle(Handle::NorthWest),
            CursorKind::ResizeNorthWestSouthEast
        );
        assert_eq!(
            CursorKind::for_handle(Handle::SouthWest),
            CursorKind::ResizeNorthEastSouthWest
        );
    }
}

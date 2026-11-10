//! OS 原生拖拽穿透（`docs` §6.1）。
//!
//! When the user drags a file from Explorer into the window, the OS owns the
//! drag. The engine's job is to turn that stream into the *same* kind of
//! transient `DragOver` action the internal drag gestures produce, so the
//! existing double-layer pick test highlights the drop target and the reducer
//! never learns where the file came from.
//!
//! File paths do not fit in the 24-byte action payload, so they live in a small
//! arena and the action carries an index. The arena is transient by design: it
//! is cleared when the drag leaves, which is what keeps a 100 000-file drop from
//! growing the undo stack.

#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::PathBuf;

use unbound_abi::action::{ActionKind, RawAction};
use unbound_abi::desc::RawNodeId;
use unbound_math::Vec2;

/// What the OS is carrying.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragPayload {
    pub paths: Vec<PathBuf>,
    /// Plain-text drags (from a browser or an editor) arrive without paths.
    pub text: Option<String>,
}

impl DragPayload {
    /// A file drag.
    #[must_use]
    pub fn files(paths: Vec<PathBuf>) -> Self {
        Self { paths, text: None }
    }

    /// A text drag.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            paths: Vec::new(),
            text: Some(text.into()),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.text.is_none()
    }

    /// Number of files, for the event log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.paths.len()
    }
}

/// Arena of live drag payloads.
#[derive(Clone, Debug, Default)]
pub struct PayloadArena {
    entries: Vec<Option<DragPayload>>,
}

impl PayloadArena {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a payload and return its id.
    pub fn insert(&mut self, payload: DragPayload) -> u64 {
        if let Some((index, slot)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(payload);
            return index as u64 + 1;
        }
        self.entries.push(Some(payload));
        self.entries.len() as u64
    }

    /// Payload for `id`.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&DragPayload> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.entries.get(index)?.as_ref()
    }

    /// Drop a payload. Ids are never reused while any other payload is alive,
    /// but the slot is recycled, so a long drag session does not grow.
    pub fn remove(&mut self, id: u64) -> Option<DragPayload> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.entries.get_mut(index)?.take()
    }

    /// Forget everything (drag left the window).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of live payloads.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|slot| slot.is_some()).count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One frame of an OS drag.
#[derive(Clone, Debug)]
pub struct DragOver {
    pub position: Vec2,
    /// Hit-test target under the pointer, if the window accepted the drag.
    pub target: Option<RawNodeId>,
    pub payload_id: u64,
}

impl DragOver {
    /// Build the transient action the reducer and the overlay layer consume.
    ///
    /// The action is *not* transactional: it never enters the undo stack
    /// (`docs` §5.3 keeps ephemeral state out of history).
    #[must_use]
    pub fn to_action(&self, kind: ActionKind) -> RawAction {
        let mut action = RawAction::new(
            kind,
            self.target.map_or(RawNodeId::ROOT.0, |target| target.0),
        );
        action.payload[0] = self.payload_id;
        // Pack the position as two `f32` bit patterns so the 40-byte record
        // stays `Eq + Hash` (required by the action journal).
        action.payload[1] = u64::from(self.position.x.to_bits());
        action.payload[2] = u64::from(self.position.y.to_bits());
        action
    }

    /// Decode a position previously packed by [`Self::to_action`].
    #[must_use]
    pub fn position_from(action: &RawAction) -> Vec2 {
        Vec2::new(
            f32::from_bits(action.payload[1] as u32),
            f32::from_bits(action.payload[2] as u32),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_round_trip_through_the_arena() {
        let mut arena = PayloadArena::new();
        let first = arena.insert(DragPayload::files(vec![PathBuf::from("a.png")]));
        let second = arena.insert(DragPayload::text("hello"));
        assert_eq!(arena.get(first).unwrap().paths.len(), 1);
        assert_eq!(arena.get(second).unwrap().text.as_deref(), Some("hello"));
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn removed_slots_are_recycled() {
        let mut arena = PayloadArena::new();
        let first = arena.insert(DragPayload::text("a"));
        arena.insert(DragPayload::text("b"));
        arena.remove(first);
        let reused = arena.insert(DragPayload::text("c"));
        assert_eq!(reused, first, "the freed slot must be reused");
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn an_unknown_id_is_none() {
        let arena = PayloadArena::new();
        assert!(arena.get(0).is_none());
        assert!(arena.get(99).is_none());
    }

    #[test]
    fn the_action_carries_target_id_and_position() {
        let drag = DragOver {
            position: Vec2::new(12.5, -3.25),
            target: Some(RawNodeId(7)),
            payload_id: 42,
        };
        let action = drag.to_action(ActionKind::DragOver);
        assert_eq!(action.kind(), ActionKind::DragOver);
        assert_eq!(action.target, 7);
        assert_eq!(action.payload[0], 42);
        assert_eq!(DragOver::position_from(&action), drag.position);
    }

    #[test]
    fn a_drag_outside_any_target_targets_the_root() {
        let drag = DragOver {
            position: Vec2::ZERO,
            target: None,
            payload_id: 1,
        };
        let action = drag.to_action(ActionKind::Drop);
        assert_eq!(action.kind(), ActionKind::Drop);
        assert_eq!(action.target, RawNodeId::ROOT.0);
    }

    #[test]
    fn clearing_drops_everything() {
        let mut arena = PayloadArena::new();
        arena.insert(DragPayload::text("x"));
        arena.clear();
        assert!(arena.is_empty());
    }

    #[test]
    fn an_empty_payload_is_recognised() {
        assert!(DragPayload::default().is_empty());
        assert!(!DragPayload::text("x").is_empty());
        assert!(!DragPayload::files(vec![PathBuf::from("a")]).is_empty());
        assert_eq!(DragPayload::files(vec![PathBuf::from("a")]).len(), 1);
    }
}

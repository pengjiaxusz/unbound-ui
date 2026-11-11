//! Serializable action as *data*.
//!
//! `docs` §4.1: view nodes may not hold `Box<dyn Fn>` closures. Every
//! interaction binds a strongly typed, `Copy` action record, which makes undo,
//! replay, network sync and agent-driven fuzzing trivial.

use bytemuck::{Pod, Zeroable};

/// Semantic action kind. Kept as a `u32` discriminant so guests and hosts with
/// different builds can still route actions by number.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ActionKind {
    #[default]
    None = 0,
    /// Activate a control (click, Enter, Space, accessibility invoke).
    Press = 1,
    PointerDown = 2,
    PointerUp = 3,
    PointerMove = 4,
    PointerCancel = 5,
    /// Wheel / trackpad; payload = (dx, dy) as `f32` bits.
    Scroll = 6,
    /// Text editing: insert string from the action arena.
    TextInsert = 7,
    TextBackspace = 8,
    TextDelete = 9,
    /// Canvas gestures.
    CanvasPan = 10,
    CanvasZoom = 11,
    CanvasSelect = 12,
    CanvasMoveSelection = 13,
    CanvasDelete = 14,
    CanvasDuplicate = 15,
    CanvasGroup = 16,
    CanvasUngroup = 17,
    /// Undo/redo scope control (`docs` §5.3).
    BeginTransaction = 18,
    UpdateTransaction = 19,
    CommitTransaction = 20,
    CancelTransaction = 21,
    Undo = 22,
    Redo = 23,
    /// Theme & viewport.
    SetTheme = 24,
    Resize = 25,
    Focus = 26,
    Blur = 27,
    /// OS file drag entered the window (`docs` §6.1).
    DragEnter = 28,
    /// OS file drag moved over a drop target.
    DragOver = 29,
    /// OS file drag released.
    Drop = 30,
    /// Escape hatch for application-specific actions.
    Custom = 0xFFFF,
}

impl ActionKind {
    #[must_use]
    pub const fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Press,
            2 => Self::PointerDown,
            3 => Self::PointerUp,
            4 => Self::PointerMove,
            5 => Self::PointerCancel,
            6 => Self::Scroll,
            7 => Self::TextInsert,
            8 => Self::TextBackspace,
            9 => Self::TextDelete,
            10 => Self::CanvasPan,
            11 => Self::CanvasZoom,
            12 => Self::CanvasSelect,
            13 => Self::CanvasMoveSelection,
            14 => Self::CanvasDelete,
            15 => Self::CanvasDuplicate,
            16 => Self::CanvasGroup,
            17 => Self::CanvasUngroup,
            18 => Self::BeginTransaction,
            19 => Self::UpdateTransaction,
            20 => Self::CommitTransaction,
            21 => Self::CancelTransaction,
            22 => Self::Undo,
            23 => Self::Redo,
            24 => Self::SetTheme,
            28 => Self::DragEnter,
            29 => Self::DragOver,
            30 => Self::Drop,
            25 => Self::Resize,
            26 => Self::Focus,
            27 => Self::Blur,
            0xFFFF => Self::Custom,
            _ => Self::None,
        }
    }

    /// Actions that mutate document state and therefore belong in the undo
    /// stack. Transient pointer traffic explicitly does not.
    #[must_use]
    pub const fn is_transactional(self) -> bool {
        matches!(
            self,
            Self::CanvasMoveSelection
                | Self::CanvasDelete
                | Self::CanvasDuplicate
                | Self::CanvasGroup
                | Self::CanvasUngroup
                | Self::TextInsert
                | Self::TextBackspace
                | Self::TextDelete
        )
    }

    /// Actions that must never be coalesced even when they arrive inside the
    /// text-merge time window.
    #[must_use]
    pub const fn is_boundary(self) -> bool {
        matches!(
            self,
            Self::CommitTransaction
                | Self::CancelTransaction
                | Self::Undo
                | Self::Redo
                | Self::CanvasDelete
        )
    }
}

/// A 40-byte POD action record (4B kind + 4B target + 24B payload + 8B time).
///
/// `payload` is intentionally untyped: the reducer knows how to interpret it
/// per [`ActionKind`]. Floats are stored as their bit patterns so the record is
/// `Eq` + `Hash` (needed for coalescing and for the action journal).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct RawAction {
    pub kind: u32,
    /// Target node, or `NO_NODE` for global actions.
    pub target: u32,
    /// `u64` payload slots: indices, token ids, packed coordinate pairs, …
    pub payload: [u64; 3],
    /// Monotonic input timestamp in microseconds (used for text coalescing).
    pub timestamp_us: u64,
}

impl RawAction {
    pub const NONE: Self = Self {
        kind: 0,
        target: 0,
        payload: [0; 3],
        timestamp_us: 0,
    };

    #[must_use]
    pub const fn new(kind: ActionKind, target: u32) -> Self {
        Self {
            kind: kind as u32,
            target,
            payload: [0; 3],
            timestamp_us: 0,
        }
    }

    #[must_use]
    pub const fn with_payload(mut self, index: usize, value: u64) -> Self {
        if index < 3 {
            self.payload[index] = value;
        }
        self
    }

    #[must_use]
    pub const fn with_time(mut self, us: u64) -> Self {
        self.timestamp_us = us;
        self
    }

    #[inline]
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        ActionKind::from_u32(self.kind)
    }

    #[inline]
    #[must_use]
    pub const fn f32_at(&self, index: usize) -> f32 {
        if index < 3 {
            f32::from_bits(self.payload[index] as u32)
        } else {
            0.0
        }
    }

    #[inline]
    #[must_use]
    pub const fn f32_pair(&self, a: usize, b: usize) -> (f32, f32) {
        (self.f32_at(a), self.f32_at(b))
    }

    /// Pack two `f32` coordinates into one payload slot (exact round-trip).
    #[must_use]
    pub const fn pack_vec2(x: f32, y: f32) -> u64 {
        (x.to_bits() as u64) | ((y.to_bits() as u64) << 32)
    }

    #[must_use]
    pub const fn unpack_vec2(v: u64) -> (f32, f32) {
        (f32::from_bits(v as u32), f32::from_bits((v >> 32) as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_round_trips_kind_and_payload() {
        let a = RawAction::new(ActionKind::CanvasZoom, 7)
            .with_payload(0, RawAction::pack_vec2(12.5, -3.25))
            .with_time(1234);
        assert_eq!(a.kind(), ActionKind::CanvasZoom);
        assert_eq!(a.target, 7);
        assert_eq!(a.timestamp_us, 1234);
        let (x, y) = RawAction::unpack_vec2(a.payload[0]);
        assert_eq!((x, y), (12.5, -3.25));
    }

    #[test]
    fn float_payload_is_exact_through_bits() {
        let v = 0.1f32 + 0.2f32;
        let a = RawAction::new(ActionKind::PointerMove, 1).with_payload(1, v.to_bits() as u64);
        assert_eq!(a.f32_at(1), v);
    }

    #[test]
    fn action_size_is_40_bytes() {
        assert_eq!(core::mem::size_of::<RawAction>(), 40);
        assert_eq!(core::mem::align_of::<RawAction>(), 8);
    }

    #[test]
    fn unknown_kind_maps_to_none() {
        assert_eq!(ActionKind::from_u32(999), ActionKind::None);
        let a = RawAction {
            kind: 999,
            ..Default::default()
        };
        assert_eq!(a.kind(), ActionKind::None);
    }

    #[test]
    fn transactional_classification() {
        assert!(ActionKind::CanvasMoveSelection.is_transactional());
        assert!(ActionKind::TextInsert.is_transactional());
        assert!(!ActionKind::PointerMove.is_transactional());
        assert!(!ActionKind::Undo.is_transactional());
        assert!(ActionKind::Undo.is_boundary());
        assert!(!ActionKind::TextInsert.is_boundary());
    }

    #[test]
    fn out_of_range_payload_is_ignored() {
        let a = RawAction::new(ActionKind::Press, 0).with_payload(9, 5);
        assert_eq!(a.payload, [0; 3]);
        assert_eq!(a.f32_at(9), 0.0);
    }
}

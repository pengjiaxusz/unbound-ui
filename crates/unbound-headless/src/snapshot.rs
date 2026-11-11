//! Zero-copy time-travel snapshots (`docs` §7.2).
//!
//! When a frame fails, the whole point is to be able to *look at it again*:
//! replay the exact geometry, step through it, diff it against a later frame.
//! `rkyv` is the format that makes this cheap — the archived bytes can be
//! read in place, with no deserialisation pass and no allocation.
//!
//! Two formats live here on purpose:
//!
//! * [`TimeTravel`] — the `rkyv` archive. Read it in place, or restore it into
//!   a live [`Frame`]. This is the format a debugger keeps.
//! * [`capture`] / [`restore`] — `postcard`, the compact serde format. Useful
//!   for shipping a frame over a wire or storing thousands of them.
//!
//! The DTO layer exists because [`crate::frame::NodeSnapshot`] carries a
//! `&'static str` source location. Serialising a string and leaking it back on
//! load is fine for a debugging artefact restored once per investigation, and it
//! keeps the hot path free of `String` churn.

#![forbid(unsafe_op_in_unsafe_fn)]

use serde::{Deserialize, Serialize};
use unbound_math::Rect;

use crate::frame::{Frame, NodeKind, NodeSnapshot, SourceLoc};

/// Stable wire format for a frame.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct FrameSnapshot {
    pub viewport: [f32; 4],
    pub nodes: Vec<SnapNode>,
}

/// Stable wire format for one node. Field order is part of the format.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct SnapNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub path: String,
    pub rect: [f32; 4],
    pub clip: Option<[f32; 4]>,
    /// `0` container, `1` text, `2` interactive, `3` portal.
    pub kind: u8,
    pub ink: [f32; 4],
    pub ellipsis: bool,
    pub occluded: bool,
    pub min_size: [f32; 2],
    pub overlays_siblings: bool,
    pub scrollable: bool,
    /// Text foreground and background, `r,g,b,r,g,b`.
    pub text_colors: Option<[u8; 6]>,
}

fn rect_to_array(rect: Rect) -> [f32; 4] {
    [rect.origin.x, rect.origin.y, rect.size.x, rect.size.y]
}

fn rect_from_array(array: [f32; 4]) -> Rect {
    Rect::new(array[0], array[1], array[2], array[3])
}

impl FrameSnapshot {
    /// Capture a frame.
    #[must_use]
    pub fn capture(frame: &Frame) -> Self {
        Self {
            viewport: rect_to_array(frame.viewport),
            nodes: frame
                .nodes
                .iter()
                .map(|node| {
                    let (kind, ink, ellipsis, occluded, min_size) = match &node.kind {
                        NodeKind::Container => (0u8, Rect::ZERO, false, false, [0.0, 0.0]),
                        NodeKind::Text { ink, ellipsis } => {
                            (1u8, *ink, *ellipsis, false, [0.0, 0.0])
                        }
                        NodeKind::Interactive { min_size, occluded } => {
                            (2u8, Rect::ZERO, false, *occluded, [min_size.x, min_size.y])
                        }
                        NodeKind::Portal => (3u8, Rect::ZERO, false, false, [0.0, 0.0]),
                    };
                    SnapNode {
                        id: node.id,
                        parent: node.parent,
                        children: node.children.clone(),
                        file: node.loc.file.to_owned(),
                        line: node.loc.line,
                        column: node.loc.column,
                        path: node.path.clone(),
                        rect: rect_to_array(node.rect),
                        clip: node.clip.map(rect_to_array),
                        kind,
                        ink: rect_to_array(ink),
                        ellipsis,
                        occluded,
                        min_size,
                        overlays_siblings: node.overlays_siblings,
                        scrollable: node.scrollable,
                        text_colors: node.text_colors.map(|colors| {
                            [
                                colors.foreground[0],
                                colors.foreground[1],
                                colors.foreground[2],
                                colors.background[0],
                                colors.background[1],
                                colors.background[2],
                            ]
                        }),
                    }
                })
                .collect(),
        }
    }

    /// Rebuild a frame. Source locations are interned for the process lifetime.
    #[must_use]
    pub fn restore(&self) -> Frame {
        Frame::from_nodes(
            rect_from_array(self.viewport),
            self.nodes
                .iter()
                .map(|node| {
                    let kind = match node.kind {
                        1 => NodeKind::Text {
                            ink: rect_from_array(node.ink),
                            ellipsis: node.ellipsis,
                        },
                        2 => NodeKind::Interactive {
                            min_size: unbound_math::Vec2::new(node.min_size[0], node.min_size[1]),
                            occluded: node.occluded,
                        },
                        3 => NodeKind::Portal,
                        _ => NodeKind::Container,
                    };
                    NodeSnapshot {
                        id: node.id,
                        parent: node.parent,
                        children: node.children.clone(),
                        loc: SourceLoc {
                            file: intern(&node.file),
                            line: node.line,
                            column: node.column,
                        },
                        path: node.path.clone(),
                        rect: rect_from_array(node.rect),
                        clip: node.clip.map(rect_from_array),
                        kind,
                        overlays_siblings: node.overlays_siblings,
                        scrollable: node.scrollable,
                        text_colors: node.text_colors.map(|colors| crate::frame::TextColors {
                            foreground: [colors[0], colors[1], colors[2]],
                            background: [colors[3], colors[4], colors[5]],
                        }),
                    }
                })
                .collect(),
        )
    }
}

/// Intern a file path for the lifetime of the process.
fn intern(file: &str) -> &'static str {
    // A snapshot is restored at most a handful of times per investigation, and
    // `Location` needs a `&'static str`, so leaking is the correct trade here.
    Box::leak(file.to_owned().into_boxed_str())
}

/// Serialise a frame to bytes.
///
/// # Errors
/// Returns the underlying postcard error when the frame does not fit its
/// declared sizes (which would indicate a corrupt `Frame`).
pub fn capture(frame: &Frame) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(&FrameSnapshot::capture(frame))
}

/// Deserialise a frame from bytes.
///
/// # Errors
/// Returns the underlying postcard error for truncated or corrupt input.
pub fn restore(bytes: &[u8]) -> Result<Frame, postcard::Error> {
    let snapshot: FrameSnapshot = postcard::from_bytes(bytes)?;
    Ok(snapshot.restore())
}

/// A zero-copy time-travel snapshot.
///
/// The bytes are the archive; every read is a pointer chase into the buffer
/// instead of a copy. A debugger can keep thousands of these in memory and
/// still step through them instantly.
#[derive(Clone, Debug)]
pub struct TimeTravel {
    bytes: rkyv::util::AlignedVec,
}

impl TimeTravel {
    /// Archive a frame.
    ///
    /// # Errors
    /// Returns the archive error when the frame cannot be serialised.
    pub fn capture(frame: &Frame) -> Result<Self, rkyv::rancor::Error> {
        let snapshot = FrameSnapshot::capture(frame);
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&snapshot)?;
        Ok(Self { bytes })
    }

    /// Read the archived frame in place.
    ///
    /// # Errors
    /// Returns the validation error when the buffer is corrupt or was written
    /// by a different version of the format.
    pub fn access(&self) -> Result<&rkyv::Archived<FrameSnapshot>, rkyv::rancor::Error> {
        rkyv::access::<rkyv::Archived<FrameSnapshot>, rkyv::rancor::Error>(&self.bytes)
    }

    /// The archived bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Number of bytes the snapshot occupies.
    #[must_use]
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    /// Restore a live frame from the archive.
    ///
    /// # Errors
    /// Returns the archive error when the buffer is corrupt.
    pub fn restore(&self) -> Result<Frame, rkyv::rancor::Error> {
        let archived = self.access()?;
        let restored = rkyv::deserialize::<FrameSnapshot, rkyv::rancor::Error>(archived)?;
        Ok(restored.restore())
    }
}

/// Open an archived snapshot that was written elsewhere.
///
/// # Errors
/// Returns the validation error when the buffer is corrupt.
pub fn open(bytes: &[u8]) -> Result<TimeTravel, rkyv::rancor::Error> {
    // Validate before taking ownership: a corrupt buffer must never reach the
    // rest of the debugger as if it were a frame.
    rkyv::access::<rkyv::Archived<FrameSnapshot>, rkyv::rancor::Error>(bytes)?;
    let mut owned = rkyv::util::AlignedVec::new();
    owned.extend_from_slice(bytes);
    Ok(TimeTravel { bytes: owned })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameBuilder;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn sample() -> Frame {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 800.0, 600.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 800.0, 600.0), "Root");
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(10.0, 10.0, 80.0, 30.0),
            "Button",
        );
        b.close();
        b.open(
            NodeKind::Text {
                ink: rect(0.0, 0.0, 12.5, 8.0),
                ellipsis: true,
            },
            rect(0.0, 0.0, 12.5, 8.0),
            "Label",
        );
        b.close();
        b.open(NodeKind::Portal, rect(5.0, 5.0, 100.0, 50.0), "Menu");
        b.close();
        b.close();
        b.build()
    }

    #[test]
    fn a_frame_round_trips_through_postcard() {
        let frame = sample();
        let bytes = capture(&frame).unwrap();
        let restored = restore(&bytes).unwrap();
        assert_eq!(restored.viewport, frame.viewport);
        assert_eq!(restored.nodes.len(), frame.nodes.len());
        for (a, b) in restored.nodes.iter().zip(&frame.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.parent, b.parent);
            assert_eq!(a.children, b.children);
            assert_eq!(a.rect, b.rect);
            assert_eq!(a.clip, b.clip);
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.path, b.path);
            assert_eq!(a.loc.line, b.loc.line);
        }
    }

    #[test]
    fn a_time_travel_snapshot_round_trips_through_rkyv() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let restored = snapshot.restore().unwrap();
        assert_eq!(restored.viewport, frame.viewport);
        assert_eq!(restored.nodes.len(), frame.nodes.len());
        for (a, b) in restored.nodes.iter().zip(&frame.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.rect, b.rect);
            assert_eq!(a.path, b.path);
        }
    }

    #[test]
    fn the_archive_can_be_read_in_place() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let archived = snapshot.access().unwrap();
        // The archived viewport is read straight out of the buffer.
        assert_eq!(archived.viewport[0], 0.0);
        assert_eq!(archived.viewport[2], 800.0);
        assert_eq!(archived.nodes.len(), frame.nodes.len());
        // The source path survives as a string slice into the archive.
        assert!(archived.nodes[0].path.as_str().contains("Root"));
    }

    #[test]
    fn a_snapshot_survives_a_trip_through_a_buffer() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let reopened = open(snapshot.bytes()).unwrap();
        let restored = reopened.restore().unwrap();
        assert_eq!(restored.nodes.len(), frame.nodes.len());
        assert_eq!(restored.nodes[0].path, frame.nodes[0].path);
    }

    #[test]
    fn a_corrupt_buffer_is_rejected_instead_of_read() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        let mut bytes = snapshot.bytes().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(open(&bytes).is_err());
    }

    #[test]
    fn the_archive_is_compact() {
        let frame = sample();
        let snapshot = TimeTravel::capture(&frame).unwrap();
        // Ten kilobytes of geometry must not explode into megabytes.
        assert!(
            snapshot.size() < 64 * 1024,
            "archive is {} bytes",
            snapshot.size()
        );
    }

    /// A snapshot is meant to travel over a wire, so its ids are whatever the
    /// sender put there. Id 0 has no slot in the dense index: the old code
    /// subtracted one from zero and panicked instead of restoring the frame.
    #[test]
    fn a_snapshot_carrying_a_zero_id_is_restored_instead_of_panicking() {
        let frame = sample();
        let mut snapshot = FrameSnapshot::capture(&frame);
        snapshot.nodes[0].id = 0;
        let bytes = postcard::to_allocvec(&snapshot).unwrap();
        let restored = restore(&bytes).expect("a malformed id must not abort");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert!(restored.get(0).is_some());
    }

    /// `u64::MAX` used to size the id index by the id itself, which asked the
    /// allocator for `usize::MAX` entries.
    #[test]
    fn a_snapshot_carrying_an_absurd_id_is_restored_instead_of_panicking() {
        let frame = sample();
        let mut snapshot = FrameSnapshot::capture(&frame);
        snapshot.nodes[0].id = u64::MAX;
        let bytes = postcard::to_allocvec(&snapshot).unwrap();
        let restored = restore(&bytes).expect("a malformed id must not abort");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert!(restored.get(u64::MAX).is_some());
    }

    /// The same goes through the rkyv path: `open` validates the *archive*,
    /// but the ids inside it are still the sender's business.
    #[test]
    fn an_archive_carrying_a_zero_id_is_restored_instead_of_panicking() {
        let frame = sample();
        let mut snapshot = FrameSnapshot::capture(&frame);
        snapshot.nodes[0].id = 0;
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&snapshot).unwrap();
        let reopened = open(&bytes).expect("the archive itself is well formed");
        let restored = reopened.restore().expect("a malformed id must not abort");
        assert_eq!(restored.nodes.len(), snapshot.nodes.len());
        assert!(restored.get(0).is_some());
    }
}

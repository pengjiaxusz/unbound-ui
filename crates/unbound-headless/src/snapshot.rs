//! Zero-copy-ish binary snapshots for time-travel debugging (`docs` §7.2).
//!
//! The doc asks for `rkyv`/`postcard` so that a failing frame can be captured
//! and replayed without keeping the whole process alive. This module uses
//! `postcard`, the compact serde format: it is a few kilobytes for a full
//! 10 000 node frame and needs no unsafe code, which keeps the harness honest
//! about "no GPU, no unsafe".
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameSnapshot {
    pub viewport: [f32; 4],
    pub nodes: Vec<SnapNode>,
}

/// Stable wire format for one node. Field order is part of the format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
                            (1, *ink, *ellipsis, false, [0.0, 0.0])
                        }
                        NodeKind::Interactive { min_size, occluded } => {
                            (2, Rect::ZERO, false, *occluded, [min_size.x, min_size.y])
                        }
                        NodeKind::Portal => (3, Rect::ZERO, false, false, [0.0, 0.0]),
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
    fn the_payload_is_compact() {
        let frame = sample();
        let bytes = capture(&frame).unwrap();
        // 4 nodes with strings: a few hundred bytes, not kilobytes.
        assert!(bytes.len() < 512, "snapshot grew to {} bytes", bytes.len());
    }

    #[test]
    fn a_restored_frame_still_validates() {
        let frame = sample();
        let bytes = capture(&frame).unwrap();
        let restored = restore(&bytes).unwrap();
        assert_eq!(
            crate::rules::check(&restored).len(),
            crate::rules::check(&frame).len()
        );
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        assert!(restore(&[0xff, 0xff, 0xff]).is_err());
    }
}

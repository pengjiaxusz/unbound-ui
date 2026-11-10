//! 无头验证器的几何模型（`docs` §7.2）。
//!
//! A [`Frame`] is the *result* of one headless step: the node tree after the
//! reducer ran and Taffy solved it. It carries exactly the data the five layout
//! sanitizers need, and nothing that would require a GPU, a window handle or a
//! font file.

#![forbid(unsafe_op_in_unsafe_fn)]

use core::panic::Location;
use unbound_math::{Rect, Vec2};

/// Where a node was built. Captured with `#[track_caller]` so a sanitizer can
/// point at the line that created the node instead of at the node id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct SourceLoc {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
}

impl SourceLoc {
    /// Capture the caller's location.
    #[must_use]
    #[track_caller]
    pub fn caller() -> Self {
        Self::from_location(Location::caller())
    }

    #[must_use]
    pub fn from_location(location: &'static Location<'static>) -> Self {
        Self {
            file: location.file(),
            line: location.line(),
            column: location.column(),
        }
    }
}

impl core::fmt::Display for SourceLoc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// What a node is, from the sanitizers' point of view.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Plain layout container.
    Container,
    /// Text: carries the measured ink box so clipping can be detected.
    Text {
        /// Real ink bounds of the shaped text, in the parent's coordinate space.
        ink: Rect,
        /// `true` when the component declares `text-overflow: ellipsis`, which
        /// makes a clipped ink box intentional.
        ellipsis: bool,
    },
    /// Something clickable: carries the minimum target size and whether a
    /// sibling painted above it swallows the centre point.
    Interactive {
        /// Minimum acceptable clipped size; `24 × 24` per the doc's default.
        min_size: Vec2,
        /// Set by the hit tester when the node's centre is fully covered.
        occluded: bool,
    },
    /// A Portal overlay: must stay inside the physical viewport.
    Portal,
}

impl NodeKind {
    /// The doc's default minimum interactive target.
    pub const DEFAULT_MIN_SIZE: Vec2 = Vec2 { x: 24.0, y: 24.0 };

    #[must_use]
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::Interactive { .. })
    }

    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    #[must_use]
    pub fn is_portal(&self) -> bool {
        matches!(self, Self::Portal)
    }
}

/// One node in a validated frame.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSnapshot {
    pub id: u64,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub loc: SourceLoc,
    /// Semantic path, e.g. `Root/Toolbar/Button "Save"`.
    pub path: String,
    /// Border box after layout, in viewport space.
    pub rect: Rect,
    /// Effective clip (already intersected with every ancestor), if any.
    pub clip: Option<Rect>,
    pub kind: NodeKind,
    /// Absolutely positioned overlays are allowed to overlap their siblings.
    pub overlays_siblings: bool,
    /// Inside a scroll container: clipped text there is expected.
    pub scrollable: bool,
}

impl NodeSnapshot {
    /// Rect after applying the effective clip.
    #[must_use]
    pub fn visible_rect(&self) -> Rect {
        match self.clip {
            Some(clip) => self.rect.intersect(&clip),
            None => self.rect,
        }
    }

    /// True when this node has a scrollable ancestor (including itself).
    #[must_use]
    pub fn in_scroll_container(&self) -> bool {
        self.scrollable
    }
}

/// A solved frame, ready for the sanitizers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub viewport: Rect,
    pub nodes: Vec<NodeSnapshot>,
    /// `index[id - 1]` is the position of node `id` in `nodes`.
    ///
    /// Ids are dense and assigned by the builder, so a plain `Vec` beats a hash
    /// map here: `get` is two loads, which matters because the sanitizers call
    /// it millions of times on a large frame.
    index: Vec<usize>,
}

impl Frame {
    #[must_use]
    pub fn new(viewport: Rect) -> Self {
        Self {
            viewport,
            nodes: Vec::new(),
            index: Vec::new(),
        }
    }

    /// Build a frame from an existing node list, rebuilding the id index.
    #[must_use]
    pub fn from_nodes(viewport: Rect, nodes: Vec<NodeSnapshot>) -> Self {
        let mut frame = Self {
            viewport,
            nodes,
            index: Vec::new(),
        };
        frame.rebuild_index();
        frame
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for (position, node) in self.nodes.iter().enumerate() {
            let slot = node.id as usize;
            if self.index.len() < slot {
                self.index.resize(slot, usize::MAX);
            }
            self.index[slot - 1] = position;
        }
    }

    fn position_of(&self, id: u64) -> Option<usize> {
        let slot = usize::try_from(id).ok()?.checked_sub(1)?;
        let position = *self.index.get(slot)?;
        if position == usize::MAX || position >= self.nodes.len() {
            None
        } else {
            Some(position)
        }
    }

    #[must_use]
    pub fn get(&self, id: u64) -> Option<&NodeSnapshot> {
        self.position_of(id).map(|position| &self.nodes[position])
    }

    /// Parent chain of `id`, root first.
    #[must_use]
    pub fn chain(&self, id: u64) -> Vec<u64> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(node) = current {
            chain.push(node);
            current = self.get(node).and_then(|n| n.parent);
        }
        chain.reverse();
        chain
    }
}

/// Stack-based frame builder.
///
/// Every `push` records the calling source line, so the diagnostics point at
/// component code rather than at a numeric id.
#[derive(Debug)]
pub struct FrameBuilder {
    frame: Frame,
    /// Open nodes, with a flag telling whether they establish a scroll container.
    stack: Vec<(u64, bool)>,
    next_id: u64,
    labels: Vec<String>,
    scroll_depth: u32,
}

impl FrameBuilder {
    #[must_use]
    pub fn new(viewport: Rect) -> Self {
        Self {
            frame: Frame::new(viewport),
            stack: Vec::new(),
            next_id: 1,
            labels: Vec::new(),
            scroll_depth: 0,
        }
    }

    /// Open a node as a child of the current one.
    #[track_caller]
    pub fn open(&mut self, kind: NodeKind, rect: Rect, label: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let parent = self.stack.last().map(|(id, _)| *id);
        let path = match parent.and_then(|p| self.labels.get(p as usize - 1)) {
            Some(parent_path) if !label.is_empty() => format!("{parent_path}/{label}"),
            Some(parent_path) => parent_path.clone(),
            None => label.to_owned(),
        };
        if let Some(parent_id) = parent {
            if let Some(node) = self.frame.get_mut(parent_id) {
                node.children.push(id);
            }
        }
        // Inherit the parent's clip, intersected with the parent's own bounds
        // when the parent clips its overflow.
        let clip = match parent.and_then(|p| self.frame.get(p)) {
            Some(parent_node) => parent_node.clip,
            None => None,
        };
        let scrollable = self.scroll_depth > 0;
        self.frame.index.resize(id as usize, usize::MAX);
        self.frame.index[id as usize - 1] = self.frame.nodes.len();
        self.frame.nodes.push(NodeSnapshot {
            id,
            parent,
            children: Vec::new(),
            loc: SourceLoc::caller(),
            path: path.clone(),
            rect,
            clip,
            kind,
            overlays_siblings: false,
            scrollable,
        });
        self.labels.push(path);
        self.stack.push((id, false));
        id
    }

    /// Close the current node.
    pub fn close(&mut self) {
        if let Some((_, guard)) = self.stack.pop() {
            if guard {
                self.scroll_depth = self.scroll_depth.saturating_sub(1);
            }
        }
    }

    /// Mark the current node as an absolutely positioned overlay.
    pub fn set_overlays_siblings(&mut self, value: bool) {
        if let Some((id, _)) = self.stack.last().copied() {
            if let Some(node) = self.frame.get_mut(id) {
                node.overlays_siblings = value;
            }
        }
    }

    /// Mark the current node as a scroll container: its descendants may clip
    /// their content without that counting as a layout bug.
    pub fn set_scrollable(&mut self, value: bool) {
        if let Some(entry) = self.stack.last_mut() {
            if value && !entry.1 {
                entry.1 = true;
                self.scroll_depth += 1;
                if let Some(node) = self.frame.get_mut(entry.0) {
                    node.scrollable = true;
                }
            }
        }
    }

    /// Clip the current node (and its descendants) to `rect`.
    pub fn set_clip(&mut self, rect: Rect) {
        if let Some(id) = self.stack.last().map(|(id, _)| *id) {
            let inherited = self.frame.get(id).and_then(|n| n.clip);
            let clip = match inherited {
                Some(existing) => existing.intersect(&rect),
                None => rect,
            };
            if let Some(node) = self.frame.get_mut(id) {
                node.clip = Some(clip);
            }
        }
    }

    /// The node currently open.
    #[must_use]
    pub fn current(&self) -> Option<u64> {
        self.stack.last().map(|(id, _)| *id)
    }

    /// Finish and hand back the frame.
    #[must_use]
    pub fn build(self) -> Frame {
        self.frame
    }
}

impl Frame {
    fn get_mut(&mut self, id: u64) -> Option<&mut NodeSnapshot> {
        let position = self.position_of(id)?;
        self.nodes.get_mut(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn the_builder_nests_and_names_nodes() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 800.0, 600.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 800.0, 600.0), "Root");
        b.open(NodeKind::Container, rect(0.0, 0.0, 400.0, 40.0), "Toolbar");
        b.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect(10.0, 5.0, 80.0, 30.0),
            "Button \"Save\"",
        );
        b.close();
        b.close();
        b.close();
        let frame = b.build();
        assert_eq!(frame.nodes.len(), 3);
        assert_eq!(frame.nodes[2].path, "Root/Toolbar/Button \"Save\"");
        assert_eq!(frame.chain(3), vec![1, 2, 3]);
        assert_eq!(frame.nodes[0].children, vec![2]);
    }

    #[test]
    fn every_push_records_the_calling_line() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        let frame = b.build();
        assert!(frame.nodes[0].loc.file.ends_with("frame.rs"));
        assert!(frame.nodes[0].loc.line > 0);
        assert!(frame.nodes[0].loc.column > 0);
    }

    #[test]
    fn clips_are_inherited_and_intersected() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.set_clip(rect(0.0, 0.0, 50.0, 50.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Inner");
        b.set_clip(rect(25.0, 25.0, 100.0, 100.0));
        let id = b.current().unwrap();
        b.close();
        b.close();
        let frame = b.build();
        let inner = frame.get(id).unwrap();
        assert_eq!(inner.clip, Some(rect(25.0, 25.0, 25.0, 25.0)));
    }

    #[test]
    fn the_visible_rect_is_the_clipped_rect() {
        let mut b = FrameBuilder::new(rect(0.0, 0.0, 100.0, 100.0));
        b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Root");
        b.set_clip(rect(0.0, 0.0, 30.0, 30.0));
        let id = b.open(NodeKind::Container, rect(0.0, 0.0, 100.0, 100.0), "Big");
        b.close();
        b.close();
        let frame = b.build();
        let node = frame.get(id).unwrap();
        assert_eq!(node.visible_rect(), rect(0.0, 0.0, 30.0, 30.0));
    }
}

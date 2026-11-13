//! The file drop zone: OS drag feedback and path delivery.
//!
//! The engine never learns where a file came from. The OS drag arrives as a
//! [`DragOver`] carrying a payload id into the [`PayloadArena`]; this widget
//! only answers two questions — "is the pointer over me" and "which paths did
//! I receive" — so a file dropped from Explorer takes the same path as one
//! dragged inside the app.
//!
//! Paths cross the widget boundary as `Vec<PathBuf>`, never as an action
//! payload: a hundred-file drop therefore costs no undo history.

use std::path::PathBuf;

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, DragOver, HitRegion, PayloadArena, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// The inset of the dashed outline.
pub const DROP_INSET: f32 = 8.0;
/// The length of one dash.
pub const DROP_DASH: f32 = 10.0;
/// The gap between dashes.
pub const DROP_GAP: f32 = 7.0;
/// The corner radius of the zone.
pub const DROP_RADIUS: f32 = 10.0;

/// The pure descriptor of a drop zone.
#[derive(Clone, Debug, PartialEq)]
pub struct FileDropDesc {
    /// The zone rectangle.
    pub rect: Rect,
    /// The accessible label.
    pub label: String,
    /// The hint shown inside the zone.
    pub hint: String,
    /// Whether a drop may deliver more than one file.
    pub accept_multiple: bool,
    /// The zone refuses interaction.
    pub disabled: bool,
    /// The draw order key.
    pub z: u16,
}

impl FileDropDesc {
    /// A zone that accepts several files.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            hint: String::new(),
            accept_multiple: true,
            disabled: false,
            z: 745,
        }
    }
}

/// The live state of a drop zone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDropState {
    hovered: bool,
    payload: Option<u64>,
}

impl FileDropState {
    /// An idle zone.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a compatible drag is over the zone.
    #[must_use]
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// The payload currently hovering, if any.
    #[must_use]
    pub fn payload(&self) -> Option<u64> {
        self.payload
    }

    /// Update the hover state from an OS drag.
    ///
    /// Returns whether the zone's visual state changed.
    pub fn drag_over(
        &mut self,
        node: u64,
        desc: &FileDropDesc,
        over: &DragOver,
        arena: &PayloadArena,
    ) -> bool {
        let over_us = !desc.disabled
            && over
                .target
                .map(|target| u64::from(target.0))
                .is_some_and(|target| target == node)
            && desc.rect.contains_point(over.position);
        let compatible = arena
            .get(over.payload_id)
            .is_some_and(|payload| !payload.is_empty());
        let hovered = over_us && compatible;
        let payload = hovered.then_some(over.payload_id);
        let changed = self.hovered != hovered || self.payload != payload;
        self.hovered = hovered;
        self.payload = payload;
        changed
    }

    /// Finish a drag, returning the files to deliver.
    ///
    /// A single-file zone truncates the list instead of rejecting the drop:
    /// the first path is the one the user grabbed first, which is the least
    /// surprising choice.
    pub fn drop_files(
        &mut self,
        desc: &FileDropDesc,
        over: &DragOver,
        arena: &PayloadArena,
    ) -> Option<Vec<PathBuf>> {
        let accepted = self.hovered && self.payload == Some(over.payload_id);
        self.leave();
        if !accepted {
            return None;
        }
        let mut paths = arena.get(over.payload_id)?.paths.clone();
        if paths.is_empty() {
            return None;
        }
        if !desc.accept_multiple {
            paths.truncate(1);
        }
        Some(paths)
    }

    /// The drag left the zone or the window.
    pub fn leave(&mut self) -> bool {
        let changed = self.hovered || self.payload.is_some();
        self.hovered = false;
        self.payload = None;
        changed
    }
}

/// A dashed rectangle drawn with line primitives.
fn dash_rect(rect: Rect, mut out: impl FnMut(Rect, PrimType)) {
    let step = DROP_DASH + DROP_GAP;
    let mut x = rect.origin.x;
    while x < rect.max().x {
        let width = DROP_DASH.min(rect.max().x - x);
        out(Rect::new(x, rect.origin.y, width, 1.0), PrimType::Line);
        out(Rect::new(x, rect.max().y - 1.0, width, 1.0), PrimType::Line);
        x += step;
    }
    let mut y = rect.origin.y;
    while y < rect.max().y {
        let height = DROP_DASH.min(rect.max().y - y);
        out(Rect::new(rect.origin.x, y, 1.0, height), PrimType::Line);
        out(
            Rect::new(rect.max().x - 1.0, y, 1.0, height),
            PrimType::Line,
        );
        y += step;
    }
}

/// Emit the zone chrome, dashed outline and semantic node.
#[must_use]
pub fn emit_file_drop(
    node: u64,
    desc: &FileDropDesc,
    state: &FileDropState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::new();
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.hovered || control.phase == crate::PointerPhase::Hovered {
        Token::PRIMARY_SOFT
    } else {
        Token::SURFACE_SUNKEN
    };
    primitives.push(surface(rect, DROP_RADIUS, fill, desc.z));

    let inner = rect.inflate(-DROP_INSET);
    let mark = if control.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if state.hovered || control.focused {
        Token::PRIMARY
    } else {
        Token::OUTLINE_STRONG
    };
    dash_rect(inner, |dash, kind| {
        primitives.push(instance(
            dash,
            [0.0; 4],
            mark,
            Token::NONE,
            0.0,
            0.0,
            kind,
            SubElement::Content,
            desc.z,
        ));
    });

    // A simple "tray" glyph: two strokes and a dot. No font needed.
    let cx = rect.center().x;
    let cy = rect.center().y - 6.0;
    for (dx, width) in [(-8.0, 16.0), (-5.0, 1.0)] {
        primitives.push(instance(
            Rect::new(cx + dx, cy, width, 1.5),
            [0.0; 4],
            mark,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }
    primitives.push(surface(
        Rect::new(cx - 5.0, cy + 5.0, 10.0, 8.0),
        2.0,
        mark,
        desc.z,
    ));

    emit_glyphs(
        glyphs,
        if control.disabled {
            Token::TEXT_DISABLED
        } else if state.hovered {
            Token::PRIMARY
        } else {
            Token::TEXT_SECONDARY
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );

    primitives.push(focus_ring(
        rect.inflate(3.0),
        DROP_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [DROP_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.description = desc.hint.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
    }
}

/// The point a drag is hovering, for hit-testing in tests and applications.
#[must_use]
pub fn drop_point(rect: Rect) -> Vec2 {
    rect.center()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_abi::desc::RawNodeId;
    use unbound_input::DragPayload;

    fn desc() -> FileDropDesc {
        FileDropDesc::new(Rect::new(0.0, 0.0, 200.0, 100.0))
    }

    fn over(node: u64, payload_id: u64, position: Vec2) -> DragOver {
        DragOver {
            position,
            target: Some(RawNodeId(u32::try_from(node).expect("test node fits"))),
            payload_id,
        }
    }

    #[test]
    fn hovering_requires_the_zone_to_be_the_hit_target() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();

        assert!(!state.drag_over(1, &desc, &over(2, id, Vec2::new(50.0, 50.0)), &arena));
        assert!(!state.is_hovered());
        assert!(state.drag_over(1, &desc, &over(1, id, Vec2::new(50.0, 50.0)), &arena));
        assert!(state.is_hovered());
        assert_eq!(state.payload(), Some(id));
    }

    #[test]
    fn a_position_outside_the_rect_is_rejected() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();
        assert!(!state.drag_over(1, &desc, &over(1, id, Vec2::new(500.0, 50.0)), &arena));
        assert!(!state.is_hovered());
    }

    #[test]
    fn an_empty_payload_never_lights_the_zone() {
        let desc = desc();
        let arena = PayloadArena::new();
        let mut state = FileDropState::new();
        assert!(!state.drag_over(1, &desc, &over(1, 99, Vec2::new(10.0, 10.0)), &arena));
    }

    #[test]
    fn dropping_delivers_the_paths_and_clears_the_hover() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![
            PathBuf::from("a.txt"),
            PathBuf::from("b.txt"),
        ]));
        let mut state = FileDropState::new();
        let drag = over(1, id, Vec2::new(20.0, 20.0));
        state.drag_over(1, &desc, &drag, &arena);
        assert_eq!(
            state.drop_files(&desc, &drag, &arena),
            Some(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")])
        );
        assert!(!state.is_hovered());
    }

    #[test]
    fn a_single_file_zone_truncates_the_drop() {
        let mut desc = desc();
        desc.accept_multiple = false;
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![
            PathBuf::from("a.txt"),
            PathBuf::from("b.txt"),
        ]));
        let mut state = FileDropState::new();
        let drag = over(1, id, Vec2::new(20.0, 20.0));
        state.drag_over(1, &desc, &drag, &arena);
        assert_eq!(
            state.drop_files(&desc, &drag, &arena),
            Some(vec![PathBuf::from("a.txt")])
        );
    }

    #[test]
    fn dropping_without_a_hover_delivers_nothing() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();
        assert_eq!(
            state.drop_files(&desc, &over(1, id, Vec2::new(20.0, 20.0)), &arena),
            None
        );
    }

    #[test]
    fn leaving_clears_the_state_once() {
        let desc = desc();
        let mut arena = PayloadArena::new();
        let id = arena.insert(DragPayload::files(vec![PathBuf::from("a.txt")]));
        let mut state = FileDropState::new();
        state.drag_over(1, &desc, &over(1, id, Vec2::new(20.0, 20.0)), &arena);
        assert!(state.leave());
        assert!(!state.leave());
    }

    #[test]
    fn the_emitter_reports_a_pointer_cursor_and_the_hint() {
        let desc = desc();
        let state = FileDropState::new();
        let output = emit_file_drop(4, &desc, &state, ControlState::default(), &mut []);
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
        assert!(output.primitives.len() > 20, "dashed outline is emitted");
    }
}

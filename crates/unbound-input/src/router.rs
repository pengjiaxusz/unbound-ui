//! Input routing: hit test → pointer capture → cursor (`docs` §6.1, §6.2).
//!
//! [`Router`] is the piece that turns "the OS said the pointer moved" into "node
//! 42 is being dragged". It owns the three stateful things that must agree with
//! each other:
//!
//! * the pointer state machine (press / drag / release / double click),
//! * the hover target, derived from the same pick the press would use,
//! * the cursor stack, refreshed from the pick path every frame.
//!
//! Keeping them in one place is what makes the cursor *deterministic*: it is
//! recomputed from the same ordered pick path that decides the click, so a
//! resize handle can never show an I-beam while it is the click target.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;

use crate::cursor::{CursorKind, CursorSource, CursorStack};
use crate::hit::{pick, HitRegion, PORTAL_LAYER};
use crate::pointer::{PointerButton, PointerConfig, PointerEvent, PointerState};

/// Routes pointer input to hit regions.
#[derive(Clone, Debug)]
pub struct Router {
    regions: Vec<HitRegion>,
    pointer: PointerState,
    cursors: CursorStack,
    hover: Option<u64>,
    tool_cursor: CursorKind,
    /// The last canvas position the pointer was observed at.
    ///
    /// `PointerState::position` only moves when the state machine receives an
    /// event; a pan, a zoom or a scroll leaves it stale. Hover and cursor must
    /// be resolved from the freshest point, so every observation — an OS
    /// event or a camera-driven [`Router::refresh_at`] — is recorded here.
    ///
    /// `None` means the pointer never entered: picking at the state machine's
    /// origin placeholder would invent a hover target out of nothing.
    last_position: Option<Vec2>,
}

impl Default for Router {
    fn default() -> Self {
        Self::new(PointerConfig::default())
    }
}

impl Router {
    #[must_use]
    pub fn new(config: PointerConfig) -> Self {
        Self {
            regions: Vec::new(),
            pointer: PointerState::new(config),
            cursors: CursorStack::new(),
            hover: None,
            tool_cursor: CursorKind::Default,
            last_position: None,
        }
    }

    /// Replace the pickable set for this frame.
    ///
    /// The vector is reused, so a steady frame does not allocate. Hover and
    /// cursor are re-picked because the new set may no longer contain the node
    /// that was under the pointer.
    pub fn set_regions(&mut self, regions: Vec<HitRegion>) {
        self.regions = regions;
        match self.last_position {
            Some(position) => self.refresh_at(position),
            None => self.refresh_cursor(),
        }
    }

    /// Pickable regions currently registered.
    #[must_use]
    pub fn regions(&self) -> &[HitRegion] {
        &self.regions
    }

    /// Node under the pointer, if any.
    #[must_use]
    pub fn hover(&self) -> Option<u64> {
        self.hover
    }

    /// Cursor to show, resolved from the current frame.
    #[must_use]
    pub fn cursor(&self) -> CursorKind {
        self.cursors.resolve()
    }

    /// Cursor the active canvas tool asks for.
    pub fn set_tool_cursor(&mut self, cursor: CursorKind) {
        self.tool_cursor = cursor;
        self.refresh_cursor();
    }

    /// Pointer state, for gestures that need the raw capture/drag flags.
    #[must_use]
    pub fn pointer(&self) -> &PointerState {
        &self.pointer
    }

    /// Topmost region at `point`.
    #[must_use]
    pub fn pick(&self, point: Vec2) -> Option<&HitRegion> {
        pick(&self.regions, point)
    }

    /// Button press.
    pub fn pointer_down(&mut self, position: Vec2, now: f64) -> Option<PointerEvent> {
        self.last_position = Some(position);
        let target = pick(&self.regions, position).map(|r| r.node);
        self.hover = target;
        let event = self
            .pointer
            .on_down(target, position, PointerButton::Left, now);
        self.refresh_cursor();
        event
    }

    /// Pointer movement. The hit test is skipped while a capture is active —
    /// that is the whole point of the capture.
    pub fn pointer_move(&mut self, position: Vec2) -> Option<PointerEvent> {
        self.last_position = Some(position);
        let event = self.pointer.on_move(position);
        if self.pointer.capture().is_none() {
            self.hover = pick(&self.regions, position).map(|r| r.node);
        }
        self.refresh_cursor();
        event
    }

    /// Button release.
    ///
    /// A release that lands outside the region it was pressed on does not
    /// produce a click, even when the pointer never travelled far enough to
    /// count as a drag: a button must not activate when the user lets go over
    /// something else.
    pub fn pointer_up(&mut self, position: Vec2, now: f64) -> Vec<PointerEvent> {
        self.last_position = Some(position);
        let pressed = self.pointer.pressed();
        let mut events = self.pointer.on_up(position, PointerButton::Left, now);
        let released_on = pick(&self.regions, position).map(|r| r.node);
        self.hover = released_on;
        if pressed.is_some() {
            let clicked = events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. }));
            if released_on != pressed {
                events.retain(|event| {
                    !matches!(
                        event,
                        PointerEvent::Click { .. } | PointerEvent::DoubleClick { .. }
                    )
                });
                // The click was withheld, so the next press starts a new
                // click sequence instead of pairing with this one.
                self.pointer.reset_click_sequence();
            } else if !clicked {
                // A drag that ends on the target delivers no click either;
                // it must not pair with the next press as a double click.
                self.pointer.reset_click_sequence();
            }
        }
        self.refresh_cursor();
        events
    }

    /// Abort the current interaction.
    pub fn cancel(&mut self) -> Option<PointerEvent> {
        let event = self.pointer.cancel();
        match self.last_position {
            // Hover was frozen while the capture was active; the pointer may
            // now be anywhere, so it must be re-picked.
            Some(position) => self.refresh_at(position),
            None => self.refresh_cursor(),
        }
        event
    }

    /// Re-pick hover and cursor at `position` without moving the state machine.
    ///
    /// A pan, a zoom or a scene rebuild moves the geometry under a stationary
    /// pointer. Regions live in canvas space, so the hover target and cursor
    /// have to be resolved at the new canvas point rather than waiting for the
    /// next OS cursor event.
    pub fn refresh_at(&mut self, position: Vec2) {
        self.last_position = Some(position);
        if self.pointer.capture().is_none() {
            self.hover = pick(&self.regions, position).map(|r| r.node);
        }
        self.refresh_cursor_at(position);
    }

    /// Recompute the cursor stack from the current pick path.
    fn refresh_cursor(&mut self) {
        self.refresh_cursor_at(self.last_position.unwrap_or(Vec2::ZERO));
    }

    /// Recompute the cursor stack from the pick path at `position`.
    ///
    /// One pass per source, topmost region wins inside a source. That keeps the
    /// result independent of the region order and needs no temporary vector, so
    /// a mouse move allocates nothing.
    fn refresh_cursor_at(&mut self, position: Vec2) {
        self.cursors.clear();
        if self.pointer.capture().is_some() {
            let kind = if self.pointer.is_dragging() {
                CursorKind::Grabbing
            } else {
                CursorKind::Pointer
            };
            self.cursors.push(CursorSource::Capture, kind);
        }
        const SOURCES: [CursorSource; 5] = [
            CursorSource::Overlay,
            CursorSource::Handle,
            CursorSource::CanvasTool,
            CursorSource::Text,
            CursorSource::Default,
        ];
        for source in SOURCES {
            let mut best: Option<((u8, u32, u64), CursorKind)> = None;
            for region in &self.regions {
                let Some(kind) = region.cursor else { continue };
                let region_source = if region.layer >= PORTAL_LAYER {
                    CursorSource::Overlay
                } else {
                    kind.source()
                };
                if region_source != source || !region.hit(position) {
                    continue;
                }
                let rank = (region.layer, region.z, region.node);
                if best.is_none_or(|(current, _)| rank > current) {
                    best = Some((rank, kind));
                }
            }
            if let Some((_, kind)) = best {
                self.cursors.push(source, kind);
            }
        }
        // Only claim the tool slot when the tool actually wants a cursor:
        // pushing `Default` here would outrank a plain hovered widget.
        if self.tool_cursor != CursorKind::Default {
            self.cursors
                .push(CursorSource::CanvasTool, self.tool_cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hit::Shape;
    use unbound_math::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn region(node: u64, bounds: Rect) -> HitRegion {
        HitRegion::new(node, 1, 0, bounds).with_cursor(CursorKind::Pointer)
    }

    fn router() -> Router {
        Router::default()
    }

    #[test]
    fn a_press_targets_the_topmost_region() {
        let mut r = router();
        r.set_regions(vec![
            region(1, rect(0.0, 0.0, 100.0, 100.0)),
            region(2, rect(50.0, 50.0, 100.0, 100.0)),
        ]);
        let event = r.pointer_down(Vec2::new(60.0, 60.0), 0.0).unwrap();
        assert_eq!(event.target(), Some(2));
        assert_eq!(r.hover(), Some(2));
    }

    #[test]
    fn moving_without_a_press_updates_hover_only() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.hover(), Some(1));
        r.pointer_move(Vec2::new(500.0, 500.0));
        assert_eq!(r.hover(), None);
    }

    #[test]
    fn a_hovered_resize_handle_sets_the_resize_cursor() {
        let mut r = router();
        let handle = HitRegion::new(9, 1, 10, rect(90.0, 40.0, 20.0, 20.0))
            .with_cursor(CursorKind::ResizeEastWest);
        r.set_regions(vec![region(1, rect(0.0, 0.0, 200.0, 200.0)), handle]);
        r.pointer_move(Vec2::new(95.0, 45.0));
        assert_eq!(r.cursor(), CursorKind::ResizeEastWest);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.cursor(), CursorKind::Pointer);
    }

    #[test]
    fn a_portal_overlay_wins_the_cursor_over_the_ui() {
        let mut r = router();
        let portal = HitRegion::new(2, PORTAL_LAYER, 0, rect(0.0, 0.0, 50.0, 50.0))
            .with_cursor(CursorKind::Text);
        r.set_regions(vec![region(1, rect(0.0, 0.0, 200.0, 200.0)), portal]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.cursor(), CursorKind::Text);
    }

    #[test]
    fn a_capture_keeps_the_grabbing_cursor_over_other_regions() {
        let mut r = router();
        r.set_regions(vec![
            region(1, rect(0.0, 0.0, 100.0, 100.0)),
            HitRegion::new(2, 1, 0, rect(200.0, 200.0, 100.0, 100.0)).with_cursor(CursorKind::Text),
        ]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(30.0, 10.0));
        assert!(r.pointer().is_dragging());
        assert_eq!(r.cursor(), CursorKind::Grabbing);
        // Fly over the text region: the capture still owns the cursor.
        r.pointer_move(Vec2::new(250.0, 250.0));
        assert_eq!(r.cursor(), CursorKind::Grabbing);
        r.pointer_up(Vec2::new(250.0, 250.0), 0.5);
        assert_eq!(r.cursor(), CursorKind::Text);
    }

    #[test]
    fn a_full_drag_cycle_emits_start_drag_and_up() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        let down = r.pointer_down(Vec2::new(10.0, 10.0), 0.0).unwrap();
        assert!(matches!(down, PointerEvent::Down { .. }));
        let start = r.pointer_move(Vec2::new(20.0, 10.0)).unwrap();
        assert!(matches!(start, PointerEvent::DragStart { target: 1, .. }));
        let drag = r.pointer_move(Vec2::new(40.0, 10.0)).unwrap();
        assert!(matches!(drag, PointerEvent::Drag { target: 1, .. }));
        let up = r.pointer_up(Vec2::new(40.0, 10.0), 0.3);
        assert_eq!(up.len(), 1, "a drag must not emit a click");
    }

    #[test]
    fn a_click_on_a_rounded_button_ignores_the_corner() {
        let mut r = router();
        r.set_regions(vec![HitRegion::new(5, 1, 0, rect(0.0, 0.0, 100.0, 100.0))
            .with_shape(Shape::Rounded { radii: [30.0; 4] })]);
        let corner = r.pointer_down(Vec2::new(2.0, 2.0), 0.0).unwrap();
        assert_eq!(corner.target(), None);
        let centre = r.pointer_down(Vec2::new(50.0, 50.0), 0.1).unwrap();
        assert_eq!(centre.target(), Some(5));
    }

    #[test]
    fn cancel_releases_the_capture() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(50.0, 10.0));
        let event = r.cancel().unwrap();
        assert_eq!(event, PointerEvent::CaptureLost { target: 1 });
        assert_eq!(
            r.cursor(),
            CursorKind::Pointer,
            "the capture slot must be gone and the hovered widget takes over"
        );
    }

    #[test]
    fn the_tool_cursor_is_the_floor() {
        let mut r = router();
        r.set_regions(vec![]);
        r.set_tool_cursor(CursorKind::Crosshair);
        r.pointer_move(Vec2::new(1.0, 1.0));
        assert_eq!(r.cursor(), CursorKind::Crosshair);
    }

    #[test]
    fn replacing_the_regions_repicks_the_hover_at_the_same_pointer() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.hover(), Some(1));

        r.set_regions(vec![region(2, rect(0.0, 0.0, 100.0, 100.0))]);
        assert_eq!(r.hover(), Some(2), "the pointer stayed, the scene changed");

        r.set_regions(Vec::new());
        assert_eq!(r.hover(), None);
    }

    #[test]
    fn regions_alone_do_not_invent_a_hover() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        assert_eq!(r.hover(), None, "the pointer never entered the window");
    }

    #[test]
    fn a_release_outside_the_press_target_is_not_a_click() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(99.0, 50.0), 0.0);
        // Two pixels outside: below the three-pixel drag slop, so the state
        // machine still considers this a click unless the router checks where
        // the release landed.
        let events = r.pointer_up(Vec2::new(101.0, 50.0), 0.1);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. })),
            "releasing outside the target must not activate it: {events:?}"
        );
        assert_eq!(r.hover(), None);

        // The withheld click must not poison the next click sequence.
        let down = r.pointer_down(Vec2::new(50.0, 50.0), 0.2).unwrap();
        assert!(matches!(down, PointerEvent::Down { click_count: 1, .. }));
        let events = r.pointer_up(Vec2::new(50.0, 50.0), 0.3);
        assert!(matches!(
            events.last(),
            Some(PointerEvent::Click { count: 1, .. })
        ));
    }

    #[test]
    fn a_drag_does_not_pair_with_the_next_press_as_a_double_click() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(60.0, 60.0));
        let events = r.pointer_up(Vec2::new(60.0, 60.0), 0.1);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. })),
            "a drag must not click even when it ends on the target"
        );

        let down = r.pointer_down(Vec2::new(10.0, 10.0), 0.2).unwrap();
        assert!(
            matches!(down, PointerEvent::Down { click_count: 1, .. }),
            "the drag must reset the click sequence, got {down:?}"
        );
    }

    #[test]
    fn cancelling_repicks_the_hover_after_a_drag() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        r.pointer_move(Vec2::new(300.0, 300.0));
        assert_eq!(r.hover(), Some(1), "hover is frozen while captured");

        r.cancel();
        assert_eq!(r.hover(), None, "the pointer is no longer over the button");
    }

    #[test]
    fn a_scene_rebuild_after_a_camera_move_does_not_resurrect_the_old_hover() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_move(Vec2::new(10.0, 10.0));
        assert_eq!(r.hover(), Some(1));

        // The camera pans: the same window point now sits at canvas (500, 500).
        r.refresh_at(Vec2::new(500.0, 500.0));
        assert_eq!(r.hover(), None);

        // A rebuild must re-pick at the refreshed point, not at the stale
        // state-machine position where the pointer used to be.
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        assert_eq!(r.hover(), None, "a stale hover must not come back");
        assert_eq!(r.cursor(), CursorKind::Default);
    }

    #[test]
    fn cancelling_after_a_camera_move_uses_the_refreshed_position() {
        let mut r = router();
        r.set_regions(vec![region(1, rect(0.0, 0.0, 100.0, 100.0))]);
        r.pointer_down(Vec2::new(10.0, 10.0), 0.0);
        assert_eq!(r.hover(), Some(1));

        // The canvas moves under the captured pointer.
        r.refresh_at(Vec2::new(500.0, 500.0));
        assert_eq!(r.hover(), Some(1), "hover stays frozen while captured");

        r.cancel();
        assert_eq!(
            r.hover(),
            None,
            "cancel must use the refreshed point, not the stale press point"
        );
    }
}

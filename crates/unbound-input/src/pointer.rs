//! Pointer capture and the click/drag state machine (`docs` §6.1).
//!
//! Once a drag starts the pointer is *captured*: every subsequent move and the
//! release are routed to the captured node, even when the cursor leaves the
//! window or flies over unrelated widgets. That single rule is what makes
//! scrollbar thumbs, marquee selection and canvas panning behave under fast
//! motion, and it is why this module owns the drag decision instead of leaving
//! it to every widget.
//!
//! Time is passed in as `f64` seconds rather than read from the clock, so the
//! whole state machine is deterministic and replayable in the headless harness.

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;

/// Which button produced an event.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// Buttons currently held.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl Buttons {
    fn set(&mut self, button: PointerButton, pressed: bool) {
        match button {
            PointerButton::Left => self.left = pressed,
            PointerButton::Right => self.right = pressed,
            PointerButton::Middle => self.middle = pressed,
        }
    }
}

/// Everything the input layer emits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PointerEvent {
    Down {
        target: Option<u64>,
        position: Vec2,
        button: PointerButton,
        click_count: u8,
    },
    Move {
        position: Vec2,
        delta: Vec2,
        target: Option<u64>,
    },
    DragStart {
        target: u64,
        position: Vec2,
    },
    Drag {
        target: u64,
        position: Vec2,
        delta: Vec2,
    },
    Up {
        target: Option<u64>,
        position: Vec2,
        button: PointerButton,
    },
    Click {
        target: u64,
        position: Vec2,
        count: u8,
    },
    DoubleClick {
        target: u64,
        position: Vec2,
    },
    /// The capture was taken away (window lost focus, Escape, target removed).
    CaptureLost {
        target: u64,
    },
}

impl PointerEvent {
    /// Target node, when the event has one.
    #[must_use]
    pub fn target(&self) -> Option<u64> {
        match self {
            Self::Down { target, .. } | Self::Move { target, .. } | Self::Up { target, .. } => {
                *target
            }
            Self::DragStart { target, .. }
            | Self::Drag { target, .. }
            | Self::Click { target, .. }
            | Self::DoubleClick { target, .. }
            | Self::CaptureLost { target } => Some(*target),
        }
    }
}

/// Timing and distance thresholds.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointerConfig {
    /// Distance the pointer must travel before a press becomes a drag.
    pub drag_slop: f32,
    /// Maximum gap between two clicks of a double click, in seconds.
    pub double_click_window: f64,
    /// Maximum movement between two clicks of a double click.
    pub double_click_slop: f32,
}

impl Default for PointerConfig {
    fn default() -> Self {
        Self {
            drag_slop: 3.0,
            double_click_window: 0.5,
            double_click_slop: 4.0,
        }
    }
}

/// Per-pointer state machine.
#[derive(Clone, Debug)]
pub struct PointerState {
    config: PointerConfig,
    position: Vec2,
    down_position: Vec2,
    pressed: Option<u64>,
    capture: Option<u64>,
    dragging: bool,
    buttons: Buttons,
    click_count: u8,
    last_click_time: f64,
    last_click_position: Vec2,
    last_click_target: Option<u64>,
}

impl PointerState {
    #[must_use]
    pub fn new(config: PointerConfig) -> Self {
        Self {
            config,
            position: Vec2::ZERO,
            down_position: Vec2::ZERO,
            pressed: None,
            capture: None,
            dragging: false,
            buttons: Buttons::default(),
            click_count: 0,
            last_click_time: f64::NEG_INFINITY,
            last_click_position: Vec2::ZERO,
            last_click_target: None,
        }
    }

    #[must_use]
    pub fn position(&self) -> Vec2 {
        self.position
    }

    #[must_use]
    pub fn capture(&self) -> Option<u64> {
        self.capture
    }

    #[must_use]
    pub fn pressed(&self) -> Option<u64> {
        self.pressed
    }

    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    #[must_use]
    pub fn buttons(&self) -> Buttons {
        self.buttons
    }

    /// Clicks counted for the current press.
    #[must_use]
    pub fn click_count(&self) -> u8 {
        self.click_count
    }

    /// Press on `target`.
    pub fn on_down(
        &mut self,
        target: Option<u64>,
        position: Vec2,
        button: PointerButton,
        now: f64,
    ) -> Option<PointerEvent> {
        self.position = position;
        self.buttons.set(button, true);
        if button != PointerButton::Left {
            // Right/middle presses are reported but never start a drag.
            return Some(PointerEvent::Down {
                target,
                position,
                button,
                click_count: 1,
            });
        }
        let repeat = target.is_some()
            && target == self.last_click_target
            && now - self.last_click_time <= self.config.double_click_window
            && (position - self.last_click_position).length() <= self.config.double_click_slop;
        self.click_count = if repeat {
            (self.click_count + 1).min(3)
        } else {
            1
        };
        self.last_click_time = now;
        self.last_click_position = position;
        self.last_click_target = target;
        self.down_position = position;
        self.pressed = target;
        self.capture = target;
        self.dragging = false;
        Some(PointerEvent::Down {
            target,
            position,
            button,
            click_count: self.click_count,
        })
    }

    /// Movement. Returns at most one event: the transition frame reports
    /// `DragStart` instead of `Drag`, so a consumer never sees both.
    pub fn on_move(&mut self, position: Vec2) -> Option<PointerEvent> {
        let delta = position - self.position;
        self.position = position;
        if let Some(target) = self.pressed {
            if !self.dragging {
                if (position - self.down_position).length() >= self.config.drag_slop {
                    self.dragging = true;
                    return Some(PointerEvent::DragStart { target, position });
                }
                return Some(PointerEvent::Move {
                    position,
                    delta,
                    target: Some(target),
                });
            }
            if self.buttons.left {
                return Some(PointerEvent::Drag {
                    target,
                    position,
                    delta,
                });
            }
        }
        Some(PointerEvent::Move {
            position,
            delta,
            target: self.capture,
        })
    }

    /// Release.
    ///
    /// Returns the `Up` event followed, when the press never turned into a drag,
    /// by a `Click` (and a `DoubleClick` for the second click of a pair).
    pub fn on_up(
        &mut self,
        position: Vec2,
        button: PointerButton,
        now: f64,
    ) -> Vec<PointerEvent> {
        self.position = position;
        self.buttons.set(button, false);
        let target = self.pressed;
        let dragged = self.dragging;
        self.pressed = None;
        self.capture = None;
        self.dragging = false;

        let mut events = vec![PointerEvent::Up {
            target,
            position,
            button,
        }];
        if button == PointerButton::Left && !dragged {
            if let Some(target) = target {
                events.push(PointerEvent::Click {
                    target,
                    position,
                    count: self.click_count,
                });
                if self.click_count == 2 {
                    events.push(PointerEvent::DoubleClick { target, position });
                }
            }
        } else if !dragged {
            self.last_click_time = now;
        }
        events
    }

    /// Abort the current interaction (focus loss, Escape, target removed).
    pub fn cancel(&mut self) -> Option<PointerEvent> {
        let target = self.capture.or(self.pressed)?;
        self.pressed = None;
        self.capture = None;
        self.dragging = false;
        self.buttons = Buttons::default();
        Some(PointerEvent::CaptureLost { target })
    }

    /// Take the capture without a press, e.g. when a scrollbar thumb grabs.
    pub fn grab(&mut self, target: u64) {
        self.capture = Some(target);
    }

    /// Drop the capture but keep the button state.
    pub fn release_capture(&mut self) {
        self.capture = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PointerState {
        PointerState::new(PointerConfig::default())
    }

    #[test]
    fn a_press_and_release_without_movement_is_a_click() {
        let mut p = state();
        let down = p.on_down(Some(7), Vec2::new(10.0, 10.0), PointerButton::Left, 0.0);
        assert!(matches!(
            down,
            Some(PointerEvent::Down {
                target: Some(7),
                click_count: 1,
                ..
            })
        ));
        let events = p.on_up(Vec2::new(10.0, 10.0), PointerButton::Left, 0.1);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], PointerEvent::Up { .. }));
        assert!(matches!(
            events[1],
            PointerEvent::Click {
                target: 7,
                count: 1,
                ..
            }
        ));
        assert_eq!(p.capture(), None);
    }

    #[test]
    fn movement_past_the_slop_starts_a_drag_and_suppresses_the_click() {
        let mut p = state();
        p.on_down(Some(3), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        let start = p.on_move(Vec2::new(5.0, 0.0)).unwrap();
        assert!(matches!(start, PointerEvent::DragStart { target: 3, .. }));
        let drag = p.on_move(Vec2::new(9.0, 0.0)).unwrap();
        assert!(matches!(
            drag,
            PointerEvent::Drag {
                target: 3,
                delta,
                ..
            } if (delta.x - 4.0).abs() < 1e-6
        ));
        let events = p.on_up(Vec2::new(9.0, 0.0), PointerButton::Left, 0.2);
        assert_eq!(events.len(), 1, "a drag must not synthesise a click");
        assert!(matches!(events[0], PointerEvent::Up { .. }));
    }

    #[test]
    fn sub_slop_movement_stays_a_move() {
        let mut p = state();
        p.on_down(Some(3), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        let event = p.on_move(Vec2::new(1.0, 0.0)).unwrap();
        assert!(matches!(event, PointerEvent::Move { target: Some(3), .. }));
        assert!(!p.is_dragging());
    }

    #[test]
    fn the_capture_keeps_routing_to_the_original_target() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        p.on_move(Vec2::new(50.0, 0.0));
        assert_eq!(p.capture(), Some(1));
        let event = p.on_move(Vec2::new(500.0, 500.0)).unwrap();
        assert_eq!(event.target(), Some(1), "capture must survive leaving the node");
    }

    #[test]
    fn a_double_click_is_detected_within_the_window() {
        let mut p = state();
        p.on_down(Some(4), Vec2::new(20.0, 20.0), PointerButton::Left, 0.0);
        p.on_up(Vec2::new(20.0, 20.0), PointerButton::Left, 0.1);
        let down = p
            .on_down(Some(4), Vec2::new(21.0, 20.0), PointerButton::Left, 0.2)
            .unwrap();
        assert!(matches!(
            down,
            PointerEvent::Down {
                click_count: 2,
                ..
            }
        ));
        let events = p.on_up(Vec2::new(21.0, 20.0), PointerButton::Left, 0.25);
        assert!(events
            .iter()
            .any(|e| matches!(e, PointerEvent::DoubleClick { target: 4, .. })));
    }

    #[test]
    fn a_slow_second_click_is_a_fresh_single_click() {
        let mut p = state();
        p.on_down(Some(4), Vec2::new(20.0, 20.0), PointerButton::Left, 0.0);
        p.on_up(Vec2::new(20.0, 20.0), PointerButton::Left, 0.05);
        let down = p
            .on_down(Some(4), Vec2::new(20.0, 20.0), PointerButton::Left, 2.0)
            .unwrap();
        assert!(matches!(
            down,
            PointerEvent::Down {
                click_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn clicking_a_different_target_resets_the_count() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(20.0, 20.0), PointerButton::Left, 0.0);
        p.on_up(Vec2::new(20.0, 20.0), PointerButton::Left, 0.05);
        let down = p
            .on_down(Some(2), Vec2::new(20.0, 20.0), PointerButton::Left, 0.1)
            .unwrap();
        assert!(matches!(
            down,
            PointerEvent::Down {
                click_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn the_right_button_never_starts_a_drag() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(0.0, 0.0), PointerButton::Right, 0.0);
        let event = p.on_move(Vec2::new(100.0, 0.0)).unwrap();
        assert!(matches!(event, PointerEvent::Move { .. }));
        assert!(!p.is_dragging());
    }

    #[test]
    fn cancel_reports_the_capture_and_clears_state() {
        let mut p = state();
        p.on_down(Some(5), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        p.on_move(Vec2::new(20.0, 0.0));
        let event = p.cancel().unwrap();
        assert_eq!(event, PointerEvent::CaptureLost { target: 5 });
        assert_eq!(p.capture(), None);
        assert!(!p.is_dragging());
        assert!(p.cancel().is_none(), "second cancel is a no-op");
    }

    #[test]
    fn a_click_outside_any_target_still_reports_up() {
        let mut p = state();
        p.on_down(None, Vec2::new(1.0, 1.0), PointerButton::Left, 0.0);
        let events = p.on_up(Vec2::new(1.0, 1.0), PointerButton::Left, 0.1);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], PointerEvent::Up { target: None, .. }));
    }
}

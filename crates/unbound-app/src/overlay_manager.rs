//! Runtime overlay stack & focus-trap manager (`docs` §6.2, S5.2).
//!
//! Wraps [`OverlayStack`] and coordinates [`FocusManager`] traps, outside-click
//! light dismissal, `Escape` dismissal, and focus return to the trigger element.

use unbound_input::{FocusCause, FocusManager};
use unbound_math::Vec2;
use unbound_widgets::overlay::{OverlayEntry, OverlayStack};

/// Coordinates open overlays, modal focus traps, and focus restoration.
#[derive(Debug, Default)]
pub struct OverlayManager {
    stack: OverlayStack,
    return_focus: Vec<(u64, Option<u64>)>,
}

impl OverlayManager {
    /// Create an empty overlay manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the underlying [`OverlayStack`].
    #[must_use]
    pub fn stack(&self) -> &OverlayStack {
        &self.stack
    }

    /// Mutably borrow the underlying [`OverlayStack`].
    pub fn stack_mut(&mut self) -> &mut OverlayStack {
        &mut self.stack
    }

    /// Whether any modal overlay is currently trapping focus and locking background scroll.
    #[must_use]
    pub fn is_modal_active(&self) -> bool {
        self.stack.focus_scope().is_some()
    }

    /// Open an overlay, remembering current focus for later restoration and
    /// installing a focus trap if the overlay stack now has an active modal scope.
    pub fn open(&mut self, entry: OverlayEntry, focus: &mut FocusManager) -> u64 {
        let node = entry.node;
        let prev = focus.focused();
        if !self.stack.contains(node) {
            self.return_focus.push((node, prev));
        }
        let key = self.stack.push(entry);
        self.sync_focus_trap(focus);
        key
    }

    /// Close a specific overlay by `node`, restoring its trigger focus and updating traps.
    pub fn close(&mut self, node: u64, focus: &mut FocusManager) -> bool {
        if !self.stack.close(node) {
            return false;
        }
        self.restore_and_sync(node, focus);
        true
    }

    /// Handle a primary pointer-down at `point`, dismissing the topmost light-dismissable
    /// overlay if the click fell outside its bounds.
    pub fn handle_pointer_down(&mut self, point: Vec2, focus: &mut FocusManager) -> Option<u64> {
        let closed = self.stack.close_outside(point)?;
        self.restore_and_sync(closed, focus);
        Some(closed)
    }

    /// Handle `Escape`, dismissing the topmost escapable overlay and restoring focus.
    pub fn handle_escape(&mut self, focus: &mut FocusManager) -> Option<u64> {
        let closed = self.stack.close_on_escape()?;
        self.restore_and_sync(closed, focus);
        Some(closed)
    }

    /// Prune overlays whose anchor no longer exists and update focus traps.
    pub fn prune_anchors(
        &mut self,
        valid: impl FnMut(u64) -> bool,
        focus: &mut FocusManager,
    ) -> usize {
        let removed = self.stack.prune_anchors(valid);
        if removed > 0 {
            self.return_focus
                .retain(|(node, _)| self.stack.contains(*node));
            self.sync_focus_trap(focus);
        }
        removed
    }

    /// Synchronize [`FocusManager`] trap with the topmost modal in [`OverlayStack`].
    pub fn sync_focus_trap(&self, focus: &mut FocusManager) {
        if let Some(scope) = self.stack.focus_scope() {
            focus.set_trap(scope.iter().copied());
        } else {
            focus.clear_trap();
        }
    }

    fn restore_and_sync(&mut self, closed: u64, focus: &mut FocusManager) {
        self.sync_focus_trap(focus);
        if let Some(pos) = self.return_focus.iter().rposition(|(n, _)| *n == closed) {
            let (_, prev) = self.return_focus.remove(pos);
            if let Some(target) = prev {
                if focus.is_trapped(target) {
                    focus.focus(target, FocusCause::Programmatic);
                }
            }
        }
    }
}

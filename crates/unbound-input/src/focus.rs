//! Framework-level keyboard focus navigation (`docs` §6.2).
//!
//! The focus slot itself lives in the router / semantic tree; this module owns
//! the *policy*: which node receives focus next, whether the focus ring is
//! allowed to appear, and which nodes a modal dialog traps focus inside.
//!
//! The manager deliberately stores only `u64` identities. The caller installs
//! the tab order (usually `SceneCollector::focus_order`) once per frame, and
//! the manager prunes a focus that points at a node the new scene no longer
//! contains. Keeping the type free of widget or scene dependencies is what
//! lets `unbound-input` stay below `unbound-widgets` in the dependency graph.
//!
//! Two rules make the ring feel native:
//!
//! * A pointer click focuses a control but, by default, does **not** paint the
//!   ring; only keyboard navigation (or a text field, which needs its caret)
//!   reveals it.
//! * A focus trap never lets `Tab` escape, even when the trap set is empty;
//!   an empty trap therefore means "focus stays where it is".

/// A keyboard key the focus manager understands.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusKey {
    /// `Tab`: move to the next focusable node.
    Tab,
    /// `Shift+Tab`: move to the previous focusable node.
    ShiftTab,
    /// `Escape`: drop focus (or leave the trap).
    Escape,
}

/// Why a node received focus.
///
/// The cause decides whether the focus ring is painted: a pointer click must
/// not flash a ring, a keyboard traversal must, and a text field always shows
/// its caret.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusCause {
    /// The pointer selected the node.
    Pointer,
    /// Keyboard traversal selected the node.
    Keyboard,
    /// A text field was activated (click or keyboard).
    TextEntry,
    /// Code called [`FocusManager::focus`].
    Programmatic,
}

impl FocusCause {
    /// Whether this cause reveals the focus ring.
    #[must_use]
    pub const fn reveals_ring(self) -> bool {
        matches!(self, Self::Keyboard | Self::TextEntry)
    }
}

/// Configuration for the focus policy.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusConfig {
    /// Paint the focus ring after a pointer click as well.
    pub ring_on_pointer: bool,
}

/// The keyboard focus state machine.
#[derive(Debug)]
pub struct FocusManager {
    order: Vec<u64>,
    focused: Option<u64>,
    visible: bool,
    trap: Option<Vec<u64>>,
    config: FocusConfig,
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new(FocusConfig::default())
    }
}

impl FocusManager {
    #[must_use]
    pub fn new(config: FocusConfig) -> Self {
        Self {
            order: Vec::new(),
            focused: None,
            visible: false,
            trap: None,
            config,
        }
    }

    /// Replace the tab order.
    ///
    /// Duplicates are removed, keeping the first occurrence. A focus that is
    /// no longer in the order is dropped — a node that vanished from the scene
    /// must not keep the keyboard.
    ///
    /// Returns whether the focus changed.
    pub fn set_order(&mut self, order: impl IntoIterator<Item = u64>) -> bool {
        self.order.clear();
        for key in order {
            if !self.order.contains(&key) {
                self.order.push(key);
            }
        }
        if let Some(focused) = self.focused {
            if !self.is_reachable(focused) {
                self.focus_internal(None, false);
                return true;
            }
        }
        false
    }

    /// The current tab order.
    #[must_use]
    pub fn order(&self) -> &[u64] {
        &self.order
    }

    /// The focused node, if any.
    #[must_use]
    pub const fn focused(&self) -> Option<u64> {
        self.focused
    }

    /// Whether the focus ring should be painted this frame.
    #[must_use]
    pub const fn focus_visible(&self) -> bool {
        self.visible
    }

    /// Focus a node explicitly.
    ///
    /// Returns whether the focus changed.
    pub fn focus(&mut self, key: u64, cause: FocusCause) -> bool {
        let visible =
            cause.reveals_ring() || (cause == FocusCause::Pointer && self.config.ring_on_pointer);
        self.focus_internal(Some(key), visible)
    }

    /// Drop focus. Returns whether anything changed.
    pub fn blur(&mut self) -> bool {
        self.focus_internal(None, false)
    }

    /// Move to the next (or previous) node, wrapping around.
    ///
    /// A focus trap restricts the cycle to the trap set. When nothing is
    /// focused yet, the first candidate wins.
    pub fn step(&mut self, backwards: bool) -> Option<u64> {
        let candidates = self.candidates();
        if candidates.is_empty() {
            return self.focused;
        }
        let current = self
            .focused
            .and_then(|key| candidates.iter().position(|candidate| *candidate == key));
        let next = match current {
            Some(index) if backwards => (index + candidates.len() - 1) % candidates.len(),
            Some(index) => (index + 1) % candidates.len(),
            None if backwards => candidates.len() - 1,
            None => 0,
        };
        let key = candidates[next];
        self.focus_internal(Some(key), true);
        self.focused
    }

    /// Handle a navigation key.
    ///
    /// Returns the newly focused node (`None` after `Escape`), or the current
    /// focus when the key is not a navigation key.
    pub fn handle_key(&mut self, key: FocusKey) -> Option<u64> {
        match key {
            FocusKey::Tab => self.step(false),
            FocusKey::ShiftTab => self.step(true),
            FocusKey::Escape => {
                self.blur();
                None
            }
        }
    }

    /// Restrict keyboard traversal to a set of nodes.
    ///
    /// This is what a modal dialog installs: the nodes behind the scrim must
    /// not be reachable with `Tab`. If the current focus is outside the trap,
    /// the first trapped node receives it.
    pub fn set_trap(&mut self, nodes: impl IntoIterator<Item = u64>) {
        let mut trap: Vec<u64> = Vec::new();
        for node in nodes {
            if !trap.contains(&node) {
                trap.push(node);
            }
        }
        self.trap = Some(trap);
        if let Some(focused) = self.focused {
            if !trap_is_empty(self.trap.as_deref()) && !self.is_reachable(focused) {
                let first = self.trap.as_ref().and_then(|trap| trap.first().copied());
                self.focus_internal(first, true);
            }
        }
    }

    /// Remove the focus trap. Focus is left where it is.
    pub fn clear_trap(&mut self) {
        self.trap = None;
    }

    /// The active trap set, if any.
    #[must_use]
    pub fn trap(&self) -> Option<&[u64]> {
        self.trap.as_deref()
    }

    /// Whether `key` is inside the active trap (or no trap is installed).
    #[must_use]
    pub fn is_trapped(&self, key: u64) -> bool {
        self.trap.as_ref().is_none_or(|trap| trap.contains(&key))
    }

    /// The candidate list a `Tab` traversal walks.
    fn candidates(&self) -> &[u64] {
        match &self.trap {
            Some(trap) => trap,
            None => &self.order,
        }
    }

    /// Whether a node is reachable under the current trap.
    fn is_reachable(&self, key: u64) -> bool {
        match &self.trap {
            Some(trap) => trap.contains(&key),
            None => self.order.contains(&key),
        }
    }

    fn focus_internal(&mut self, key: Option<u64>, visible: bool) -> bool {
        if self.focused == key && self.visible == visible {
            return false;
        }
        let changed = self.focused != key;
        self.focused = key;
        self.visible = key.is_some() && visible;
        changed
    }
}

fn trap_is_empty(trap: Option<&[u64]>) -> bool {
    trap.is_some_and(<[u64]>::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> FocusManager {
        let mut manager = FocusManager::default();
        manager.set_order([10, 20, 30]);
        manager
    }

    #[test]
    fn tab_cycles_forward_and_wraps() {
        let mut manager = manager();
        assert_eq!(manager.handle_key(FocusKey::Tab), Some(10));
        assert_eq!(manager.handle_key(FocusKey::Tab), Some(20));
        assert_eq!(manager.handle_key(FocusKey::Tab), Some(30));
        assert_eq!(manager.handle_key(FocusKey::Tab), Some(10));
    }

    #[test]
    fn shift_tab_cycles_backwards_and_wraps() {
        let mut manager = manager();
        assert_eq!(manager.handle_key(FocusKey::ShiftTab), Some(30));
        assert_eq!(manager.handle_key(FocusKey::ShiftTab), Some(20));
        assert_eq!(manager.handle_key(FocusKey::ShiftTab), Some(10));
    }

    #[test]
    fn escape_drops_focus() {
        let mut manager = manager();
        manager.focus(20, FocusCause::Keyboard);
        assert_eq!(manager.handle_key(FocusKey::Escape), None);
        assert_eq!(manager.focused(), None);
        assert!(!manager.focus_visible());
    }

    #[test]
    fn a_pointer_click_does_not_reveal_the_ring() {
        let mut manager = manager();
        manager.focus(10, FocusCause::Pointer);
        assert_eq!(manager.focused(), Some(10));
        assert!(!manager.focus_visible());
    }

    #[test]
    fn keyboard_navigation_and_text_entry_reveal_the_ring() {
        let mut navigated = manager();
        navigated.handle_key(FocusKey::Tab);
        assert!(navigated.focus_visible());

        let mut entry = manager();
        entry.focus(20, FocusCause::TextEntry);
        assert!(entry.focus_visible());

        let mut ringed = FocusManager::new(FocusConfig {
            ring_on_pointer: true,
        });
        ringed.set_order([10]);
        ringed.focus(10, FocusCause::Pointer);
        assert!(ringed.focus_visible());
    }

    #[test]
    fn removing_the_focused_node_drops_the_focus() {
        let mut manager = manager();
        manager.focus(20, FocusCause::Keyboard);
        assert!(manager.set_order([10, 30]));
        assert_eq!(manager.focused(), None);
    }

    #[test]
    fn a_trap_confines_tab_and_shift_tab() {
        let mut manager = manager();
        manager.set_trap([100, 200]);
        assert_eq!(manager.focused(), None);
        for _ in 0..10 {
            let focused = manager
                .handle_key(FocusKey::Tab)
                .expect("trap is not empty");
            assert!([100, 200].contains(&focused), "escaped to {focused}");
        }
        for _ in 0..10 {
            let focused = manager
                .handle_key(FocusKey::ShiftTab)
                .expect("trap is not empty");
            assert!([100, 200].contains(&focused), "escaped to {focused}");
        }
    }

    #[test]
    fn installing_a_trap_moves_an_outside_focus_inside() {
        let mut manager = manager();
        manager.focus(30, FocusCause::Keyboard);
        manager.set_trap([100]);
        assert_eq!(manager.focused(), Some(100));
        assert!(manager.is_trapped(100));
        assert!(!manager.is_trapped(30));
        manager.clear_trap();
        assert!(manager.is_trapped(30));
    }

    #[test]
    fn an_empty_trap_never_moves_focus() {
        let mut manager = manager();
        manager.focus(30, FocusCause::Keyboard);
        manager.set_trap(std::iter::empty());
        assert_eq!(manager.focused(), Some(30));
        assert_eq!(manager.handle_key(FocusKey::Tab), Some(30));
    }

    #[test]
    fn duplicates_in_the_order_are_removed() {
        let mut manager = FocusManager::default();
        manager.set_order([1, 2, 1, 3, 2]);
        assert_eq!(manager.order(), &[1, 2, 3]);
    }
}

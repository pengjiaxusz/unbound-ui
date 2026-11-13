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

/// Orientation for linear roving tabindex groups.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RovingOrientation {
    /// Left/Right arrows move across items.
    Horizontal,
    /// Up/Down arrows move across items.
    Vertical,
    /// Both Left/Right and Up/Down move linearly.
    Both,
}

/// Navigation topology for a [`FocusGroup`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusGroupMode {
    /// 1D strip (tabs, radio group, segmented control, toolbar, menu).
    Linear(RovingOrientation),
    /// 2D grid (calendar, color swatches, icon picker) with `columns` per row.
    Grid { columns: usize },
    /// Hierarchical tree (tree view, outline) where Up/Down/Home/End step items.
    Tree,
}

/// Directional key for roving tabindex navigation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RovingKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// A roving-tabindex focus group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusGroup {
    /// Stable group identity.
    pub id: u64,
    /// Navigation mode (`Linear`, `Grid`, `Tree`).
    pub mode: FocusGroupMode,
    /// Ordered member node IDs.
    pub items: Vec<u64>,
    /// Currently active index inside `items`.
    pub active_index: usize,
    /// Whether stepping past ends wraps around.
    pub wrap: bool,
}

impl FocusGroup {
    /// Create a new roving focus group.
    #[must_use]
    pub fn new(id: u64, mode: FocusGroupMode, items: Vec<u64>) -> Self {
        Self {
            id,
            mode,
            items,
            active_index: 0,
            wrap: true,
        }
    }

    /// Builder: set wrap-around behavior.
    #[must_use]
    pub fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Currently active node in this group.
    #[must_use]
    pub fn active_node(&self) -> Option<u64> {
        self.items.get(self.active_index).copied()
    }

    /// Sync active index to `node` if present.
    pub fn set_active_node(&mut self, node: u64) -> bool {
        if let Some(pos) = self.items.iter().position(|&id| id == node) {
            self.active_index = pos;
            true
        } else {
            false
        }
    }

    /// Navigate according to `key` and return the newly focused node ID, if moved.
    pub fn navigate(&mut self, key: RovingKey) -> Option<u64> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        let cur = self.active_index.min(len - 1);
        let next = match self.mode {
            FocusGroupMode::Linear(orient) => match (orient, key) {
                (_, RovingKey::Home) => Some(0),
                (_, RovingKey::End) => Some(len - 1),
                (RovingOrientation::Horizontal | RovingOrientation::Both, RovingKey::Left)
                | (RovingOrientation::Vertical | RovingOrientation::Both, RovingKey::Up) => {
                    if cur > 0 {
                        Some(cur - 1)
                    } else if self.wrap {
                        Some(len - 1)
                    } else {
                        None
                    }
                }
                (RovingOrientation::Horizontal | RovingOrientation::Both, RovingKey::Right)
                | (RovingOrientation::Vertical | RovingOrientation::Both, RovingKey::Down) => {
                    if cur + 1 < len {
                        Some(cur + 1)
                    } else if self.wrap {
                        Some(0)
                    } else {
                        None
                    }
                }
                _ => None,
            },
            FocusGroupMode::Grid { columns } => {
                let cols = columns.max(1);
                match key {
                    RovingKey::Home => Some(0),
                    RovingKey::End => Some(len - 1),
                    RovingKey::Left => {
                        if cur > 0 {
                            Some(cur - 1)
                        } else if self.wrap {
                            Some(len - 1)
                        } else {
                            None
                        }
                    }
                    RovingKey::Right => {
                        if cur + 1 < len {
                            Some(cur + 1)
                        } else if self.wrap {
                            Some(0)
                        } else {
                            None
                        }
                    }
                    RovingKey::Up => {
                        if cur >= cols {
                            Some(cur - cols)
                        } else {
                            None
                        }
                    }
                    RovingKey::Down => {
                        if cur + cols < len {
                            Some(cur + cols)
                        } else {
                            None
                        }
                    }
                }
            }
            FocusGroupMode::Tree => match key {
                RovingKey::Home => Some(0),
                RovingKey::End => Some(len - 1),
                RovingKey::Up => {
                    if cur > 0 {
                        Some(cur - 1)
                    } else if self.wrap {
                        Some(len - 1)
                    } else {
                        None
                    }
                }
                RovingKey::Down => {
                    if cur + 1 < len {
                        Some(cur + 1)
                    } else if self.wrap {
                        Some(0)
                    } else {
                        None
                    }
                }
                RovingKey::Left | RovingKey::Right => None,
            },
        }?;
        self.active_index = next;
        self.items.get(next).copied()
    }
}

/// Registry of roving-tabindex [`FocusGroup`]s active for the current frame.
#[derive(Debug, Default)]
pub struct RovingRegistry {
    groups: Vec<FocusGroup>,
}

impl RovingRegistry {
    /// Register or replace a focus group.
    pub fn register(&mut self, group: FocusGroup) {
        if let Some(existing) = self.groups.iter_mut().find(|g| g.id == group.id) {
            *existing = group;
        } else {
            self.groups.push(group);
        }
    }

    /// Remove all registered groups.
    pub fn clear(&mut self) {
        self.groups.clear();
    }

    /// All registered groups.
    #[must_use]
    pub fn groups(&self) -> &[FocusGroup] {
        &self.groups
    }

    /// Find the group containing `node`.
    #[must_use]
    pub fn group_for_node(&self, node: u64) -> Option<&FocusGroup> {
        self.groups.iter().find(|g| g.items.contains(&node))
    }

    /// Handle a directional key for `focused`, returning the next node ID if handled.
    pub fn handle_key(&mut self, focused: Option<u64>, key: RovingKey) -> Option<u64> {
        let node = focused?;
        let group = self.groups.iter_mut().find(|g| g.items.contains(&node))?;
        group.set_active_node(node);
        group.navigate(key)
    }
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
    roving: RovingRegistry,
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
            roving: RovingRegistry::default(),
        }
    }

    /// Register a roving-tabindex focus group for the current frame.
    pub fn register_group(&mut self, group: FocusGroup) {
        self.roving.register(group);
    }

    /// Clear all registered roving focus groups.
    pub fn clear_groups(&mut self) {
        self.roving.clear();
    }

    /// Handle a roving-tabindex directional key (`Arrow*` / `Home` / `End`).
    ///
    /// Returns the newly focused node ID if the currently focused node belongs
    /// to a registered [`FocusGroup`] and moved.
    pub fn handle_roving_key(&mut self, key: RovingKey) -> Option<u64> {
        let next = self.roving.handle_key(self.focused, key)?;
        if self.is_trapped(next) {
            self.focus_internal(Some(next), true);
            Some(next)
        } else {
            None
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

    #[test]
    fn roving_focus_group_supports_linear_grid_and_tree() {
        let mut manager = FocusManager::default();
        manager.set_order([1, 2, 3, 10, 11, 12, 13]);
        manager.register_group(FocusGroup::new(
            900,
            FocusGroupMode::Linear(RovingOrientation::Horizontal),
            vec![1, 2, 3],
        ));
        manager.register_group(
            FocusGroup::new(
                901,
                FocusGroupMode::Grid { columns: 2 },
                vec![10, 11, 12, 13],
            )
            .with_wrap(false),
        );

        manager.focus(1, FocusCause::Keyboard);
        assert_eq!(manager.handle_roving_key(RovingKey::Right), Some(2));
        assert_eq!(manager.handle_roving_key(RovingKey::End), Some(3));
        assert_eq!(manager.handle_roving_key(RovingKey::Right), Some(1));
        assert_eq!(manager.handle_roving_key(RovingKey::Left), Some(3));

        manager.focus(10, FocusCause::Keyboard);
        assert_eq!(manager.handle_roving_key(RovingKey::Down), Some(12));
        assert_eq!(manager.handle_roving_key(RovingKey::Right), Some(13));
        assert_eq!(manager.handle_roving_key(RovingKey::Up), Some(11));
        assert_eq!(manager.handle_roving_key(RovingKey::Right), Some(12));
        assert_eq!(manager.handle_roving_key(RovingKey::Home), Some(10));
        assert!(manager.focus_visible());
    }
}

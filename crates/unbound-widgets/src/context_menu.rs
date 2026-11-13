//! The context menu: an anchored [`MenuDesc`] plus the reason it closed.
//!
//! Rendering is deliberately delegated to [`unbound_widgets::menu`]. This
//! module only owns the things that make a context menu different from a menu
//! bar popup: the anchor, the target node, focus restoration and a close
//! reason the caller can turn into an announcement.

use unbound_math::{Rect, Vec2};

use crate::menu::{MenuDesc, MenuState};
use crate::overlay::{place_anchored_popup, Placement};

/// Why a context menu closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextClose {
    /// Escape or a back gesture.
    Escape,
    /// A click outside the menu.
    ClickOutside,
    /// An item was activated.
    Selection,
    /// The surface under the menu scrolled.
    Scroll,
}

/// The keys a context menu understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextKey {
    /// Highlight the previous selectable item.
    Up,
    /// Highlight the next selectable item.
    Down,
    /// Open the highlighted submenu.
    Right,
    /// Close the current submenu layer, or the menu.
    Left,
    /// Activate the highlighted item.
    Enter,
    /// Close the menu.
    Escape,
}

/// The live state of a context menu.
#[derive(Clone, Debug)]
pub struct ContextMenuState {
    /// Whether the menu is visible.
    pub open: bool,
    /// The pointer position that opened it.
    pub anchor: Vec2,
    /// The node the menu applies to.
    pub target: Option<u64>,
    /// The focused node before the menu opened.
    pub return_focus: Option<u64>,
    /// The nested menu state, reused unchanged from the menu widget.
    pub menu: MenuState,
    /// Why the menu closed, consumed by the caller.
    pub close_reason: Option<ContextClose>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenuState {
    /// A closed context menu.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: false,
            anchor: Vec2::ZERO,
            target: None,
            return_focus: None,
            menu: MenuState::new(),
            close_reason: None,
        }
    }

    /// Open at a pointer position, remembering the node and focus owner.
    pub fn open_at(&mut self, anchor: Vec2, target: u64, return_focus: Option<u64>) {
        self.open = true;
        self.anchor = anchor;
        self.target = Some(target);
        self.return_focus = return_focus;
        self.menu = MenuState::new();
        self.close_reason = None;
    }

    /// Close and record why.
    pub fn close(&mut self, reason: ContextClose) {
        if !self.open {
            return;
        }
        self.open = false;
        self.close_reason = Some(reason);
        self.menu = MenuState::new();
    }

    /// Read the close reason once.
    pub fn take_close_reason(&mut self) -> Option<ContextClose> {
        self.close_reason.take()
    }

    /// Whether the menu is visible.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Place a menu of a given size next to the anchor.
    ///
    /// The menu flips above or left of the pointer when it would leave the
    /// viewport, which is the difference between a usable context menu on a
    /// screen edge and one that renders half off-screen.
    #[must_use]
    pub fn place(&self, size: Vec2, viewport: Rect) -> Rect {
        let max = viewport.max();
        let mut x = self.anchor.x;
        let mut y = self.anchor.y;
        if x + size.x > max.x {
            x = (self.anchor.x - size.x).max(viewport.origin.x);
        }
        if y + size.y > max.y {
            y = (self.anchor.y - size.y).max(viewport.origin.y);
        }
        Rect::new(
            x.clamp(viewport.origin.x, (max.x - size.x).max(viewport.origin.x)),
            y.clamp(viewport.origin.y, (max.y - size.y).max(viewport.origin.y)),
            size.x.min(viewport.size.x),
            size.y.min(viewport.size.y),
        )
    }

    /// Fill a menu descriptor with the anchored rectangle.
    #[must_use]
    pub fn desc_at(&self, mut desc: MenuDesc, viewport: Rect) -> MenuDesc {
        let size = Vec2::new(desc.rect.size.x, desc.content_height());
        desc.rect = self.place(size, viewport);
        desc
    }

    /// The same placement through the generic overlay helper.
    #[must_use]
    pub fn placed(&self, size: Vec2, viewport: Rect) -> Rect {
        place_anchored_popup(
            Rect::new(self.anchor.x, self.anchor.y, 0.0, 0.0),
            size,
            viewport,
            Placement::BottomStart,
        )
    }

    /// Reduce one key press.
    ///
    /// Returns the activated item index when Enter chose one. Escape closes
    /// the menu; the caller observes [`Self::is_open`] or
    /// [`Self::take_close_reason`].
    pub fn key(&mut self, desc: &MenuDesc, key: ContextKey) -> Option<usize> {
        match key {
            ContextKey::Up => {
                self.menu.move_highlight(desc, false);
                None
            }
            ContextKey::Down => {
                self.menu.move_highlight(desc, true);
                None
            }
            ContextKey::Right => {
                self.menu.expand(desc);
                None
            }
            ContextKey::Left => {
                if !self.menu.collapse() {
                    self.close(ContextClose::Escape);
                }
                None
            }
            ContextKey::Enter => {
                let activated = self.menu.activate(desc);
                if activated.is_some() {
                    self.close(ContextClose::Selection);
                }
                activated
            }
            ContextKey::Escape => {
                if !self.menu.collapse() {
                    self.close(ContextClose::Escape);
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::MenuItem;

    fn desc() -> MenuDesc {
        MenuDesc::new(
            Rect::new(0.0, 0.0, 200.0, 0.0),
            vec![
                MenuItem::action("Cut").with_shortcut("Ctrl+X"),
                MenuItem::separator(),
                MenuItem::action("Copy").with_shortcut("Ctrl+C"),
            ],
        )
    }

    #[test]
    fn opening_records_the_anchor_target_and_focus() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::new(40.0, 50.0), 9, Some(3));
        assert!(state.is_open());
        assert_eq!(state.target, Some(9));
        assert_eq!(state.return_focus, Some(3));
        assert!(state.close_reason.is_none());
    }

    #[test]
    fn closing_records_the_reason_once() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::ZERO, 1, None);
        state.close(ContextClose::ClickOutside);
        assert!(!state.is_open());
        assert_eq!(state.take_close_reason(), Some(ContextClose::ClickOutside));
        assert_eq!(state.take_close_reason(), None);
    }

    #[test]
    fn a_menu_near_the_bottom_right_flips_inside_the_viewport() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::new(780.0, 700.0), 1, None);
        let placed = state.place(Vec2::new(200.0, 160.0), Rect::new(0.0, 0.0, 800.0, 600.0));
        assert!(placed.max().x <= 800.0);
        assert!(placed.max().y <= 600.0);
        assert!(placed.origin.x >= 0.0 && placed.origin.y >= 0.0);
        assert_eq!(placed.origin.x, 580.0, "the menu flips left of the pointer");
        assert_eq!(placed.origin.y, 440.0, "the menu flips above the pointer");
    }

    #[test]
    fn the_descriptor_gets_the_placed_rectangle_and_its_content_height() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::new(10.0, 20.0), 1, None);
        let placed = state.desc_at(desc(), Rect::new(0.0, 0.0, 800.0, 600.0));
        assert_eq!(placed.rect.origin, Vec2::new(10.0, 20.0));
        assert!(placed.rect.size.y >= 60.0);
    }

    #[test]
    fn escape_closes_and_restores_nothing_but_the_reason() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::ZERO, 1, Some(4));
        let desc = desc();
        assert_eq!(state.key(&desc, ContextKey::Escape), None);
        assert!(!state.is_open());
        assert_eq!(state.take_close_reason(), Some(ContextClose::Escape));
    }

    #[test]
    fn enter_activates_the_highlighted_item_and_closes() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::ZERO, 1, None);
        let desc = desc();
        state.key(&desc, ContextKey::Down);
        let activated = state.key(&desc, ContextKey::Enter);
        assert!(activated.is_some());
        assert!(!state.is_open());
        assert_eq!(state.take_close_reason(), Some(ContextClose::Selection));
    }

    #[test]
    fn up_and_down_skip_separators() {
        let mut state = ContextMenuState::new();
        state.open_at(Vec2::ZERO, 1, None);
        let desc = desc();
        state.key(&desc, ContextKey::Down);
        state.key(&desc, ContextKey::Down);
        // Two steps from nothing land on "Copy" (index 2); the separator in
        // between is not selectable.
        assert_eq!(state.menu.highlighted, Some(2));
    }
}

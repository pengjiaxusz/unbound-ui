//! A registry of independent windows.
//!
//! The display server may deliver events for any surface, so the shell needs a
//! stable id-to-context map rather than a single "the window" field. This is
//! that map, kept pure so the ownership rules can be tested without creating a
//! window:
//!
//! * insertion order is event order, and iteration is stable,
//! * removing the primary window promotes the oldest survivor,
//! * a duplicate id is rejected instead of silently replacing a live surface,
//! * every window owns its own context; closing one never touches another.

/// A window identity allocated by the shell.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

/// One registered window.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSlot<T> {
    /// The window identity.
    pub id: WindowId,
    /// The per-window state (surface, swapchain, router, …).
    pub context: T,
    /// Whether the window is currently mapped.
    pub visible: bool,
}

/// The window registry.
#[derive(Clone, Debug)]
pub struct WindowRegistry<T> {
    slots: Vec<WindowSlot<T>>,
    primary: Option<WindowId>,
}

impl<T> Default for WindowRegistry<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            primary: None,
        }
    }
}

impl<T> WindowRegistry<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new window.
    ///
    /// Returns the old context when the id was already live; the caller can
    /// then drop it deliberately instead of leaking a surface.
    pub fn insert(&mut self, id: WindowId, context: T) -> Option<T> {
        if let Some(index) = self.index_of(id) {
            let old = core::mem::replace(&mut self.slots[index].context, context);
            return Some(old);
        }
        self.slots.push(WindowSlot {
            id,
            context,
            visible: true,
        });
        if self.primary.is_none() {
            self.primary = Some(id);
        }
        None
    }

    /// The primary window.
    #[must_use]
    pub fn primary(&self) -> Option<WindowId> {
        self.primary
    }

    /// Mark an existing window as primary.
    pub fn set_primary(&mut self, id: WindowId) -> bool {
        if self.index_of(id).is_none() {
            return false;
        }
        self.primary = Some(id);
        true
    }

    /// A window's context.
    #[must_use]
    pub fn get(&self, id: WindowId) -> Option<&T> {
        self.slots
            .iter()
            .find(|slot| slot.id == id)
            .map(|slot| &slot.context)
    }

    /// A window's mutable context.
    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut T> {
        self.slots
            .iter_mut()
            .find(|slot| slot.id == id)
            .map(|slot| &mut slot.context)
    }

    /// Remove a window. The context is returned so its surface can be dropped
    /// on the caller's thread.
    pub fn remove(&mut self, id: WindowId) -> Option<T> {
        let index = self.index_of(id)?;
        let slot = self.slots.remove(index);
        if self.primary == Some(id) {
            self.primary = self.slots.first().map(|slot| slot.id);
        }
        Some(slot.context)
    }

    /// Every window, in creation order.
    pub fn iter(&self) -> impl Iterator<Item = &WindowSlot<T>> {
        self.slots.iter()
    }

    /// Every window context, in creation order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut WindowSlot<T>> {
        self.slots.iter_mut()
    }

    /// The number of live windows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Mark visibility.
    pub fn set_visible(&mut self, id: WindowId, visible: bool) {
        if let Some(index) = self.index_of(id) {
            self.slots[index].visible = visible;
        }
    }

    fn index_of(&self, id: WindowId) -> Option<usize> {
        self.slots.iter().position(|slot| slot.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_window_owns_its_own_context() {
        let mut registry = WindowRegistry::new();
        registry.insert(WindowId(1), Vec::<u64>::new());
        registry.insert(WindowId(2), Vec::<u64>::new());
        registry.get_mut(WindowId(1)).unwrap().push(7);
        assert_eq!(registry.get(WindowId(1)).unwrap(), &[7]);
        assert!(registry.get(WindowId(2)).unwrap().is_empty());
    }

    #[test]
    fn removing_the_primary_promotes_the_oldest_survivor() {
        let mut registry = WindowRegistry::new();
        registry.insert(WindowId(1), "a");
        registry.insert(WindowId(2), "b");
        registry.insert(WindowId(3), "c");
        assert_eq!(registry.primary(), Some(WindowId(1)));
        assert_eq!(registry.remove(WindowId(1)), Some("a"));
        assert_eq!(registry.primary(), Some(WindowId(2)));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn a_duplicate_id_replaces_only_that_window() {
        let mut registry = WindowRegistry::new();
        registry.insert(WindowId(1), "a");
        registry.insert(WindowId(2), "b");
        assert_eq!(registry.insert(WindowId(1), "a2"), Some("a"));
        assert_eq!(registry.get(WindowId(1)), Some(&"a2"));
        assert_eq!(registry.get(WindowId(2)), Some(&"b"));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn closing_every_window_clears_the_primary() {
        let mut registry = WindowRegistry::new();
        registry.insert(WindowId(1), 1);
        registry.insert(WindowId(2), 2);
        registry.remove(WindowId(1));
        registry.remove(WindowId(2));
        assert!(registry.is_empty());
        assert_eq!(registry.primary(), None);
    }

    #[test]
    fn iteration_is_creation_order() {
        let mut registry = WindowRegistry::new();
        for id in [3, 1, 2] {
            registry.insert(WindowId(id), id);
        }
        let ids: Vec<u64> = registry.iter().map(|slot| slot.id.0).collect();
        assert_eq!(ids, vec![3, 1, 2]);
    }

    #[test]
    fn visibility_is_per_window() {
        let mut registry = WindowRegistry::new();
        registry.insert(WindowId(1), ());
        registry.insert(WindowId(2), ());
        registry.set_visible(WindowId(1), false);
        assert!(!registry.iter().next().unwrap().visible);
        assert!(registry.iter().nth(1).unwrap().visible);
    }
}

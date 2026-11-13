//! Per-node widget state, keyed by the stable node key (`docs` §5.4).
//!
//! A widget that owns local state — a text cursor, a scroll offset, a menu's
//! open flag — used to have to live in the application's own state and be
//! dispatched by node id by hand. [`WidgetStateStore`] gives the runtime a
//! single arena for that: the view asks for `use_state::<T>(key)` and gets the
//! same value back on every frame, and a key that disappears from the view is
//! *unmounted* and freed at the end of the frame.
//!
//! The store is frame-scoped only in its bookkeeping: [`WidgetStateStore::begin_frame`]
//! starts recording which keys the view touched, and [`WidgetStateStore::end_frame`]
//! reports the difference against the previous frame so the application can run
//! its mount / unmount hooks.

#![forbid(unsafe_op_in_unsafe_fn)]

use std::any::{Any, TypeId};

use rustc_hash::{FxHashMap, FxHashSet};

/// A lifecycle transition reported for one node key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleEvent {
    /// The key was rendered this frame and was not rendered last frame.
    Mounted(u64),
    /// The key was rendered last frame and is gone this frame. Its state has
    /// already been reclaimed.
    Unmounted(u64),
    /// The keyboard focus moved to this key.
    Focused(u64),
    /// The keyboard focus left this key.
    Blurred(u64),
}

impl LifecycleEvent {
    /// The node key the event is about.
    #[must_use]
    pub const fn key(self) -> u64 {
        match self {
            Self::Mounted(key) | Self::Unmounted(key) | Self::Focused(key) | Self::Blurred(key) => {
                key
            }
        }
    }
}

/// The mount / unmount sets computed at the end of a frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleChanges {
    /// Keys that appeared this frame, sorted.
    pub mounted: Vec<u64>,
    /// Keys that disappeared this frame, sorted and already reclaimed.
    pub unmounted: Vec<u64>,
}

impl LifecycleChanges {
    /// Whether the frame produced no lifecycle transition.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mounted.is_empty() && self.unmounted.is_empty()
    }
}

/// The runtime-owned arena of local widget state.
///
/// One value per `(node key, type)` pair: a node may keep a `ScrollState` and
/// a `Spring` at the same time without one evicting the other.
#[derive(Default)]
pub struct WidgetStateStore {
    entries: FxHashMap<(u64, TypeId), Box<dyn Any + Send + Sync>>,
    visited: FxHashSet<u64>,
    mounted_last_frame: FxHashSet<u64>,
}

impl core::fmt::Debug for WidgetStateStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WidgetStateStore")
            .field("entries", &self.entries.len())
            .field("visited", &self.visited.len())
            .field("mounted_last_frame", &self.mounted_last_frame.len())
            .finish()
    }
}

impl WidgetStateStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The state for `key`, created with `T::default()` on first use and
    /// marked as visited so the frame keeps it mounted.
    pub fn use_state<T: Default + Send + Sync + 'static>(&mut self, key: u64) -> &mut T {
        self.visited.insert(key);
        self.entry(key)
    }

    /// The state for `key`, created with `init()` on first use and marked as
    /// visited so the frame keeps it mounted.
    pub fn use_state_or_init<T: Send + Sync + 'static>(
        &mut self,
        key: u64,
        init: impl FnOnce() -> T,
    ) -> &mut T {
        self.visited.insert(key);
        self.entries
            .entry((key, TypeId::of::<T>()))
            .or_insert_with(|| Box::new(init()))
            .downcast_mut::<T>()
            .expect("a state entry is always stored under its own TypeId")
    }

    /// Read state for `(key, T)` if present.
    #[must_use]
    pub fn get<T: 'static>(&self, key: u64) -> Option<&T> {
        self.entries
            .get(&(key, TypeId::of::<T>()))
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// Mutably access state for `(key, T)` if present.
    pub fn get_mut<T: 'static>(&mut self, key: u64) -> Option<&mut T> {
        self.entries
            .get_mut(&(key, TypeId::of::<T>()))
            .and_then(|boxed| boxed.downcast_mut::<T>())
    }

    /// Insert or replace state for `(key, T)`.
    pub fn insert_state<T: Send + Sync + 'static>(&mut self, key: u64, value: T) {
        self.visited.insert(key);
        self.entries
            .insert((key, TypeId::of::<T>()), Box::new(value));
    }

    /// The state for `key` without marking it as visited.
    ///
    /// Event reduction runs between frames; a key touched there must not
    /// count as rendered, or a removed widget would never be unmounted.
    pub fn entry<T: Default + Send + Sync + 'static>(&mut self, key: u64) -> &mut T {
        self.entries
            .entry((key, TypeId::of::<T>()))
            .or_insert_with(|| Box::<T>::default())
            .downcast_mut::<T>()
            .expect("a state entry is always stored under its own TypeId")
    }

    /// Start recording which keys the frame touched.
    pub fn begin_frame(&mut self) {
        self.visited.clear();
    }

    /// Finish the frame: report the mount / unmount delta and reclaim the
    /// state of the keys that disappeared.
    pub fn end_frame(&mut self) -> LifecycleChanges {
        let mut mounted: Vec<u64> = self
            .visited
            .difference(&self.mounted_last_frame)
            .copied()
            .collect();
        let mut unmounted: Vec<u64> = self
            .mounted_last_frame
            .difference(&self.visited)
            .copied()
            .collect();
        mounted.sort_unstable();
        unmounted.sort_unstable();

        if !unmounted.is_empty() {
            self.entries.retain(|(key, _), _| !unmounted.contains(key));
        }
        self.mounted_last_frame.clone_from(&self.visited);
        LifecycleChanges { mounted, unmounted }
    }

    /// Drop every value stored for `key`.
    pub fn remove(&mut self, key: u64) {
        self.entries.retain(|(entry_key, _), _| *entry_key != key);
        self.visited.remove(&key);
        self.mounted_last_frame.remove(&key);
    }

    /// How many `(key, type)` values are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, Eq)]
    struct ScrollState {
        offset: i32,
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    struct Spring {
        value: i32,
    }

    #[test]
    fn state_survives_across_frames_for_the_same_key() {
        let mut store = WidgetStateStore::new();
        store.begin_frame();
        store.use_state::<ScrollState>(7).offset = 40;
        assert_eq!(store.end_frame().mounted, vec![7]);

        store.begin_frame();
        assert_eq!(store.use_state::<ScrollState>(7).offset, 40);
        let changes = store.end_frame();
        assert!(changes.is_empty(), "the key stayed mounted");
    }

    #[test]
    fn a_key_may_hold_two_different_types() {
        let mut store = WidgetStateStore::new();
        store.use_state::<ScrollState>(1).offset = 3;
        store.use_state::<Spring>(1).value = 9;
        assert_eq!(store.len(), 2);
        assert_eq!(store.use_state::<ScrollState>(1).offset, 3);
        assert_eq!(store.use_state::<Spring>(1).value, 9);
    }

    #[test]
    fn mounting_and_unmounting_are_reported_once() {
        let mut store = WidgetStateStore::new();
        store.begin_frame();
        store.use_state::<ScrollState>(1);
        store.use_state::<ScrollState>(2);
        let changes = store.end_frame();
        assert_eq!(changes.mounted, vec![1, 2]);
        assert!(changes.unmounted.is_empty());

        store.begin_frame();
        store.use_state::<ScrollState>(1);
        let changes = store.end_frame();
        assert!(changes.mounted.is_empty());
        assert_eq!(changes.unmounted, vec![2]);
        assert_eq!(store.len(), 1, "the unmounted state was reclaimed");
    }

    #[test]
    fn entry_does_not_mark_the_key_as_visited() {
        let mut store = WidgetStateStore::new();
        store.begin_frame();
        store.entry::<ScrollState>(5);
        let changes = store.end_frame();
        assert!(
            changes.mounted.is_empty(),
            "an unvisited key is not mounted"
        );
    }

    #[test]
    fn a_frame_with_no_view_unmounts_nothing_when_skipped() {
        let mut store = WidgetStateStore::new();
        store.begin_frame();
        store.use_state::<ScrollState>(3);
        store.end_frame();
        // A skipped frame does not call begin/end at all, so the key stays.
        assert_eq!(store.len(), 1);
    }
}

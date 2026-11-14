//! 按稳定节点键索引的逐节点控件状态（`docs` §5.4）。
//!
//! 拥有局部状态的控件——文本光标、滚动偏移、菜单展开标志——过去不得不
//! 存放在应用自身状态中并按节点 ID 手动分发。[`WidgetStateStore`] 为运行时
//! 提供了统一的状态池：视图只需调用 `use_state::<T>(key)` 即可每帧取回同一值，
//! 而从视图中消失的键会在帧末被*卸载*并释放。
//!
//!
//!
//! 该存储仅在账本记录上以帧为作用域：[`WidgetStateStore::begin_frame`] 开始记录
//! 本帧视图触及了哪些键，[`WidgetStateStore::end_frame`] 则对比上一帧报告差集，
//! 以便应用运行挂载 / 卸载钩子。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

use std::any::{Any, TypeId};

use rustc_hash::{FxHashMap, FxHashSet};

/// 针对单个节点键报告的生命周期转换。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleEvent {
    /// 该键本帧被渲染且上一帧未渲染。
    Mounted(u64),
    /// 该键上一帧被渲染而本帧已消失。其状态已被回收。
    ///
    Unmounted(u64),
    /// 键盘焦点移至该键。
    Focused(u64),
    /// 键盘焦点离开该键。
    Blurred(u64),
}

impl LifecycleEvent {
    /// 该事件关联的节点键。
    #[must_use]
    pub const fn key(self) -> u64 {
        match self {
            Self::Mounted(key) | Self::Unmounted(key) | Self::Focused(key) | Self::Blurred(key) => {
                key
            }
        }
    }
}

/// 帧末计算出的挂载 / 卸载集合。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleChanges {
    /// 本帧新出现的键（已排序）。
    pub mounted: Vec<u64>,
    /// 本帧消失并已回收的键（已排序）。
    pub unmounted: Vec<u64>,
}

impl LifecycleChanges {
    /// 本帧是否未产生任何生命周期转换。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mounted.is_empty() && self.unmounted.is_empty()
    }
}

/// 运行时持有的局部控件状态池。
///
/// 每个 `(node key, type)` 对对应一个值：同一节点可同时保存 `ScrollState` 与
/// `Spring` 而互不覆盖。
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
    /// 创建空存储。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取 `key` 的状态，首次使用时通过 `T::default()` 创建并标记为已访问，
    /// 以保持本帧挂载状态。
    pub fn use_state<T: Default + Send + Sync + 'static>(&mut self, key: u64) -> &mut T {
        self.visited.insert(key);
        self.entry(key)
    }

    /// 获取 `key` 的状态，首次使用时通过 `init()` 创建并标记为已访问，
    /// 以保持本帧挂载状态。
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

    /// 若存在，则读取 `(key, T)` 的状态。
    #[must_use]
    pub fn get<T: 'static>(&self, key: u64) -> Option<&T> {
        self.entries
            .get(&(key, TypeId::of::<T>()))
            .and_then(|boxed| boxed.downcast_ref::<T>())
    }

    /// 若存在，则可变访问 `(key, T)` 的状态。
    pub fn get_mut<T: 'static>(&mut self, key: u64) -> Option<&mut T> {
        self.entries
            .get_mut(&(key, TypeId::of::<T>()))
            .and_then(|boxed| boxed.downcast_mut::<T>())
    }

    /// 插入或替换 `(key, T)` 的状态。
    pub fn insert_state<T: Send + Sync + 'static>(&mut self, key: u64, value: T) {
        self.visited.insert(key);
        self.entries
            .insert((key, TypeId::of::<T>()), Box::new(value));
    }

    /// 获取 `key` 的状态但不将其标记为已访问。
    ///
    /// 事件归约在帧间运行；此处触及的键不应计为已渲染，
    /// 否则已移除的控件将永远不会被卸载。
    pub fn entry<T: Default + Send + Sync + 'static>(&mut self, key: u64) -> &mut T {
        self.entries
            .entry((key, TypeId::of::<T>()))
            .or_insert_with(|| Box::<T>::default())
            .downcast_mut::<T>()
            .expect("a state entry is always stored under its own TypeId")
    }

    /// 开始记录本帧触及的键。
    pub fn begin_frame(&mut self) {
        self.visited.clear();
    }

    /// 结束本帧：报告挂载 / 卸载差集并回收已消失键的状态。
    ///
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

    /// 丢弃为 `key` 存储的所有值。
    pub fn remove(&mut self, key: u64) {
        self.entries.retain(|(entry_key, _), _| *entry_key != key);
        self.visited.remove(&key);
        self.mounted_last_frame.remove(&key);
    }

    /// 当前持有的 `(key, type)` 值数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 存储是否为空。
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
        // 被跳过的帧根本不会调用 begin/end，因此该键得以保留。
        assert_eq!(store.len(), 1);
    }
}

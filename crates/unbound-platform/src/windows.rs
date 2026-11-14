//! 独立窗口的注册表。
//!
//! 显示服务器可能针对任意表面派发事件，因此外壳需要稳定的 ID 到上下文映射表，
//! 而非单一的“当前窗口”字段。该映射表保持纯逻辑实现，无需创建真实窗口即可
//! 测试所有权规则：
//!
//!
//! * 插入顺序即事件顺序，迭代顺序稳定；
//! * 移除主窗口时自动将最老的存活窗口提升为主窗口；
//! * 拒绝重复 ID，绝不静默替换存活的表面；
//! * 每个窗口拥有各自独立的上下文；关闭一个窗口绝不影响其他窗口。

/// 由外壳分配的窗口标识。
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

/// 单个已注册的窗口。
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSlot<T> {
    /// 窗口标识。
    pub id: WindowId,
    /// 逐窗口状态（表面、交换链、路由器等）。
    pub context: T,
    /// 窗口当前是否已映射显示。
    pub visible: bool,
}

/// 窗口注册表。
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

    /// 注册一个新窗口。
    ///
    /// 若该 ID 已存在，则返回旧上下文；调用方可显式释放它，避免表面泄漏。
    ///
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

    /// 返回主窗口。
    #[must_use]
    pub fn primary(&self) -> Option<WindowId> {
        self.primary
    }

    /// 将指定已存在窗口设为主窗口。
    pub fn set_primary(&mut self, id: WindowId) -> bool {
        if self.index_of(id).is_none() {
            return false;
        }
        self.primary = Some(id);
        true
    }

    /// 获取指定窗口的上下文引用。
    #[must_use]
    pub fn get(&self, id: WindowId) -> Option<&T> {
        self.slots
            .iter()
            .find(|slot| slot.id == id)
            .map(|slot| &slot.context)
    }

    /// 获取指定窗口的可变上下文引用。
    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut T> {
        self.slots
            .iter_mut()
            .find(|slot| slot.id == id)
            .map(|slot| &mut slot.context)
    }

    /// 移除一个窗口。返回其上下文，以便在调用方线程上释放其表面。
    ///
    pub fn remove(&mut self, id: WindowId) -> Option<T> {
        let index = self.index_of(id)?;
        let slot = self.slots.remove(index);
        if self.primary == Some(id) {
            self.primary = self.slots.first().map(|slot| slot.id);
        }
        Some(slot.context)
    }

    /// 按创建顺序遍历所有窗口。
    pub fn iter(&self) -> impl Iterator<Item = &WindowSlot<T>> {
        self.slots.iter()
    }

    /// 按创建顺序遍历所有窗口上下文。
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut WindowSlot<T>> {
        self.slots.iter_mut()
    }

    /// 当前存活的窗口数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// 更新窗口的可见性标记。
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

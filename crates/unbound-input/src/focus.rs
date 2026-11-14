//! 框架级键盘焦点导航（`docs` §6.2）。
//!
//! 焦点槽位本身位于路由器/语义树中；本模块负责**策略**：
//! 下一个获得焦点的节点、是否允许显示焦点环，以及模态对话框将焦点
//! 限制在哪些节点内部。
//!
//! 管理器刻意只存储 `u64` 节点标识。调用方每帧安装一次 Tab 顺序
//! （通常为 `SceneCollector::focus_order`），管理器会清理指向新场景中
//! 已不存在节点的焦点。不引入控件或场景依赖，使得 `unbound-input`
//! 在依赖图中保持位于 `unbound-widgets` 之下。
//!
//!
//! 两条规则让焦点环呈现原生手感：
//!
//! * 指针点击会使控件聚焦，但默认**不**绘制焦点环；仅键盘导航
//!   （或需要显示插入符的文本输入框）才会显露焦点环。
//! * 焦点陷阱绝不允许 `Tab` 逃逸，即使陷阱集合为空亦然；
//!   因此空陷阱意味着“焦点停留在原地”。
//!

/// 焦点管理器可识别的键盘按键。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusKey {
    /// `Tab`：移动到下一个可聚焦节点。
    Tab,
    /// `Shift+Tab`：移动到上一个可聚焦节点。
    ShiftTab,
    /// `Escape`：清除焦点（或退出焦点陷阱）。
    Escape,
}

/// 线性漫游 tabindex 分组的方向。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RovingOrientation {
    /// 左/右方向键在条目间移动。
    Horizontal,
    /// 上/下方向键在条目间移动。
    Vertical,
    /// 左/右与上/下方向键均按线性顺序移动。
    Both,
}

/// [`FocusGroup`] 的导航拓扑结构。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusGroupMode {
    /// 一维条带（选项卡、单选组、分段控件、工具栏、菜单）。
    Linear(RovingOrientation),
    /// 二维网格（日历、色板、图标选择器），每行 `columns` 列。
    Grid { columns: usize },
    /// 层级树（树形视图、大纲），通过 Up/Down/Home/End 步进条目。
    Tree,
}

/// 用于漫游 tabindex 导航的方向键。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RovingKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// 漫游 tabindex（roving-tabindex）焦点分组。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusGroup {
    /// 稳定的分组标识。
    pub id: u64,
    /// 导航模式（`Linear`、`Grid`、`Tree`）。
    pub mode: FocusGroupMode,
    /// 有序的成员节点 ID 列表。
    pub items: Vec<u64>,
    /// 当前在 `items` 中激活的索引。
    pub active_index: usize,
    /// 越过首尾边界时是否循环回绕。
    pub wrap: bool,
}

impl FocusGroup {
    /// 创建新的漫游焦点分组。
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

    /// 构建器方法：设置首尾循环行为。
    #[must_use]
    pub fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// 该分组中当前激活的节点。
    #[must_use]
    pub fn active_node(&self) -> Option<u64> {
        self.items.get(self.active_index).copied()
    }

    /// 若 `node` 存在于分组中，则将激活索引同步到该节点。
    pub fn set_active_node(&mut self, node: u64) -> bool {
        if let Some(pos) = self.items.iter().position(|&id| id == node) {
            self.active_index = pos;
            true
        } else {
            false
        }
    }

    /// 根据 `key` 执行导航，若焦点发生移动则返回新聚焦的节点 ID。
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

/// 当前帧活跃的漫游 tabindex [`FocusGroup`] 注册表。
#[derive(Debug, Default)]
pub struct RovingRegistry {
    groups: Vec<FocusGroup>,
}

impl RovingRegistry {
    /// 注册或替换一个焦点分组。
    pub fn register(&mut self, group: FocusGroup) {
        if let Some(existing) = self.groups.iter_mut().find(|g| g.id == group.id) {
            *existing = group;
        } else {
            self.groups.push(group);
        }
    }

    /// 移除所有已注册的分组。
    pub fn clear(&mut self) {
        self.groups.clear();
    }

    /// 返回所有已注册的分组。
    #[must_use]
    pub fn groups(&self) -> &[FocusGroup] {
        &self.groups
    }

    /// 查找包含 `node` 的分组。
    #[must_use]
    pub fn group_for_node(&self, node: u64) -> Option<&FocusGroup> {
        self.groups.iter().find(|g| g.items.contains(&node))
    }

    /// 为 `focused` 处理方向键，若已处理则返回下一个节点 ID。
    pub fn handle_key(&mut self, focused: Option<u64>, key: RovingKey) -> Option<u64> {
        let node = focused?;
        let group = self.groups.iter_mut().find(|g| g.items.contains(&node))?;
        group.set_active_node(node);
        group.navigate(key)
    }
}

/// 节点获得焦点的原因。
///
/// 该原因决定是否绘制焦点环：指针点击不应闪烁焦点环，
/// 键盘遍历必须显示焦点环，文本输入框则始终显示插入符。
///
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusCause {
    /// 由指针选中节点。
    Pointer,
    /// 由键盘遍历选中节点。
    Keyboard,
    /// 文本输入框被激活（点击或键盘）。
    TextEntry,
    /// 代码显式调用了 [`FocusManager::focus`]。
    Programmatic,
}

impl FocusCause {
    /// 该焦点原因是否应显示焦点环。
    #[must_use]
    pub const fn reveals_ring(self) -> bool {
        matches!(self, Self::Keyboard | Self::TextEntry)
    }
}

/// 焦点策略配置。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusConfig {
    /// 在指针点击后也绘制焦点环。
    pub ring_on_pointer: bool,
}

/// 键盘焦点状态机。
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

    /// 为当前帧注册一个漫游 tabindex 焦点分组。
    pub fn register_group(&mut self, group: FocusGroup) {
        self.roving.register(group);
    }

    /// 清空所有已注册的漫游焦点分组。
    pub fn clear_groups(&mut self) {
        self.roving.clear();
    }

    /// 处理漫游 tabindex 方向键（`Arrow*` / `Home` / `End`）。
    ///
    /// 若当前聚焦节点属于某个已注册的 [`FocusGroup`] 且发生了移动，
    /// 则返回新聚焦的节点 ID。
    pub fn handle_roving_key(&mut self, key: RovingKey) -> Option<u64> {
        let next = self.roving.handle_key(self.focused, key)?;
        if self.is_trapped(next) {
            self.focus_internal(Some(next), true);
            Some(next)
        } else {
            None
        }
    }

    /// 替换 Tab 顺序表。
    ///
    /// 重复项会被去重，仅保留首次出现的位置。若当前焦点已不在新顺序中，
    /// 则清除焦点——从场景中消失的节点不应继续占用键盘输入。
    ///
    ///
    /// 返回焦点是否发生变化。
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

    /// 当前的 Tab 顺序表。
    #[must_use]
    pub fn order(&self) -> &[u64] {
        &self.order
    }

    /// 当前聚焦的节点（若有）。
    #[must_use]
    pub const fn focused(&self) -> Option<u64> {
        self.focused
    }

    /// 本帧是否应绘制焦点环。
    #[must_use]
    pub const fn focus_visible(&self) -> bool {
        self.visible
    }

    /// 显式聚焦指定节点。
    ///
    /// 返回焦点是否发生变化。
    pub fn focus(&mut self, key: u64, cause: FocusCause) -> bool {
        let visible =
            cause.reveals_ring() || (cause == FocusCause::Pointer && self.config.ring_on_pointer);
        self.focus_internal(Some(key), visible)
    }

    /// 清除当前焦点。返回状态是否发生变化。
    pub fn blur(&mut self) -> bool {
        self.focus_internal(None, false)
    }

    /// 移动到下一个（或上一个）节点，支持首尾回绕。
    ///
    /// 焦点陷阱会将循环限制在陷阱集合内部。当尚未聚焦任何节点时，
    /// 第一个候选节点胜出。
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

    /// 处理焦点导航按键。
    ///
    /// 返回新聚焦的节点（`Escape` 后返回 `None`），若按键非导航键
    /// 则返回当前焦点。
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

    /// 将键盘遍历限制在指定节点集合内。
    ///
    /// 模态对话框通过此机制确保遮罩层后方的节点无法通过 `Tab` 访问。
    /// 若当前焦点位于陷阱之外，则自动将焦点移至陷阱内的第一个节点。
    ///
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

    /// 移除焦点陷阱，当前焦点保持不变。
    pub fn clear_trap(&mut self) {
        self.trap = None;
    }

    /// 当前生效的焦点陷阱集合（若有）。
    #[must_use]
    pub fn trap(&self) -> Option<&[u64]> {
        self.trap.as_deref()
    }

    /// `key` 是否位于当前陷阱内部（或尚未安装陷阱）。
    #[must_use]
    pub fn is_trapped(&self, key: u64) -> bool {
        self.trap.as_ref().is_none_or(|trap| trap.contains(&key))
    }

    /// `Tab` 遍历所走的候选节点列表。
    fn candidates(&self) -> &[u64] {
        match &self.trap {
            Some(trap) => trap,
            None => &self.order,
        }
    }

    /// 在当前焦点陷阱下某节点是否可达。
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

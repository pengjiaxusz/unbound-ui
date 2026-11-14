//! 上下文菜单（ContextMenu）：带锚点定位的 [`MenuDesc`] 及关闭原因。
//!
//! 渲染刻意委托给 [`unbound_widgets::menu`]。本模块仅负责上下文菜单与菜单栏弹出菜单的差异部分：
//! 锚点位置、目标节点、焦点恢复以及供调用方播报的关闭原因。

use unbound_math::{Rect, Vec2};

use crate::menu::{MenuDesc, MenuState};
use crate::overlay::{place_anchored_popup, Placement};

/// 上下文菜单关闭的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextClose {
    /// 按下 Escape 或触发返回手势。
    Escape,
    /// 点击了菜单外部区域。
    ClickOutside,
    /// 某个菜单项被激活。
    Selection,
    /// 菜单下方的表面发生了滚动。
    Scroll,
}

/// 上下文菜单识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextKey {
    /// 高亮上一个可选菜单项。
    Up,
    /// 高亮下一个可选菜单项。
    Down,
    /// 打开高亮项的子菜单。
    Right,
    /// 关闭当前子菜单层级或整个菜单。
    Left,
    /// 激活当前高亮项。
    Enter,
    /// 关闭菜单。
    Escape,
}

/// 上下文菜单的实时状态。
#[derive(Clone, Debug)]
pub struct ContextMenuState {
    /// 菜单是否可见。
    pub open: bool,
    /// 打开菜单时的指针位置。
    pub anchor: Vec2,
    /// 菜单所作用的节点。
    pub target: Option<u64>,
    /// 菜单打开前持有焦点的节点。
    pub return_focus: Option<u64>,
    /// 嵌套菜单状态，直接复用自 menu 组件。
    pub menu: MenuState,
    /// 菜单关闭原因，供调用方消费。
    pub close_reason: Option<ContextClose>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenuState {
    /// 创建关闭状态的上下文菜单。
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

    /// 在指针位置打开菜单，并记录目标节点与原焦点所有者。
    pub fn open_at(&mut self, anchor: Vec2, target: u64, return_focus: Option<u64>) {
        self.open = true;
        self.anchor = anchor;
        self.target = Some(target);
        self.return_focus = return_focus;
        self.menu = MenuState::new();
        self.close_reason = None;
    }

    /// 关闭菜单并记录关闭原因。
    pub fn close(&mut self, reason: ContextClose) {
        if !self.open {
            return;
        }
        self.open = false;
        self.close_reason = Some(reason);
        self.menu = MenuState::new();
    }

    /// 一次性读取并消费关闭原因。
    pub fn take_close_reason(&mut self) -> Option<ContextClose> {
        self.close_reason.take()
    }

    /// 菜单当前是否可见。
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 将指定尺寸的菜单放置在锚点旁。
    ///
    /// 当菜单会超出视口时自动向上或向左翻转，从而避免屏幕边缘菜单被截断。
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

    /// 用锚定后的矩形填充菜单描述符。
    #[must_use]
    pub fn desc_at(&self, mut desc: MenuDesc, viewport: Rect) -> MenuDesc {
        let size = Vec2::new(desc.rect.size.x, desc.content_height());
        desc.rect = self.place(size, viewport);
        desc
    }

    /// 通过通用浮层定位助手执行相同的放置逻辑。
    #[must_use]
    pub fn placed(&self, size: Vec2, viewport: Rect) -> Rect {
        place_anchored_popup(
            Rect::new(self.anchor.x, self.anchor.y, 0.0, 0.0),
            size,
            viewport,
            Placement::BottomStart,
        )
    }

    /// 归约单次按键。
    ///
    /// 当 Enter 选中某项时返回被激活项的索引。Escape 会关闭菜单；
    /// 调用方可通过 [`Self::is_open`] 或 [`Self::take_close_reason`] 观察状态。
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
        // 从未选中状态步进两步会落在“Copy”（索引 2）；中间的分隔符不可选中。
        assert_eq!(state.menu.highlighted, Some(2));
    }
}

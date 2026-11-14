//! 菜单栏（MenuBar）：由顶层菜单组成的水平行。
//!
//! 菜单栏仅持有当前打开的索引，绝不持有弹出菜单本身。一旦某个菜单打开，
//! 悬停到同级菜单项无需点击即可切换弹出菜单——这是所有桌面应用通用的“菜单栏模式”——
//! 左右方向键在菜单栏间移动，而 [`unbound_widgets::menu`] 中的方向键处理器则遍历弹出菜单。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认菜单栏高度。
pub const MENUBAR_HEIGHT: f32 = 30.0;

/// 单个顶层菜单。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuBarItem {
    /// 稳定标识，同时用作菜单项节点。
    pub id: u64,
    /// 可见标题。
    pub title: String,
}

impl MenuBarItem {
    /// 创建标题为 `title` 的菜单项。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
        }
    }
}

/// 菜单栏的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuBarDesc {
    /// 菜单栏矩形。
    pub rect: Rect,
    /// 从左到右排列的菜单列表。
    pub items: Vec<MenuBarItem>,
    /// 单个菜单项内部的水平内边距。
    pub item_padding: f32,
    /// 绘制顺序键。
    pub z: u16,
}

impl MenuBarDesc {
    /// 使用默认度量的菜单栏。
    #[must_use]
    pub fn new(rect: Rect, items: Vec<MenuBarItem>) -> Self {
        Self {
            rect,
            items,
            item_padding: 12.0,
            z: 700,
        }
    }

    /// 根据测量标签或标题长度计算单个菜单项的宽度。
    #[must_use]
    pub fn item_width(&self, index: usize, measured: Option<f32>) -> f32 {
        measured.unwrap_or_else(|| {
            let chars = self
                .items
                .get(index)
                .map_or(4, |item| item.title.chars().count());
            chars as f32 * 8.0 + self.item_padding * 2.0
        })
    }

    /// 单个菜单项的矩形。
    #[must_use]
    pub fn item_rect(&self, _index: usize, width: f32, offset: f32) -> Rect {
        Rect::new(
            self.rect.origin.x + offset,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }

    /// 布局所有菜单项，返回 `(rectangle, item index)`。
    #[must_use]
    pub fn layout(&self, widths: &[f32]) -> Vec<(Rect, usize)> {
        let mut out = Vec::with_capacity(self.items.len());
        let mut x = 0.0;
        for index in 0..self.items.len() {
            let width = self.item_width(index, widths.get(index).copied());
            out.push((self.item_rect(index, width, x), index));
            x += width;
        }
        out
    }

    /// 弹出菜单应当锚定的坐标点。
    #[must_use]
    pub fn popup_anchor(&self, index: usize, widths: &[f32]) -> unbound_math::Vec2 {
        let layout = self.layout(widths);
        let rect = layout
            .get(index)
            .map(|(rect, _)| *rect)
            .unwrap_or(self.rect);
        unbound_math::Vec2::new(rect.origin.x, rect.max().y)
    }
}

/// 菜单栏的实时状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuBarState {
    /// 当前打开的菜单索引（若有）。
    pub open: Option<usize>,
    /// 指针下方的菜单项索引。
    pub hovered: Option<usize>,
}

impl MenuBarState {
    /// 创建空状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否有菜单处于打开状态。
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// 切换指定菜单的打开/关闭状态。
    pub fn toggle(&mut self, index: usize) -> bool {
        let next = if self.open == Some(index) {
            None
        } else {
            Some(index)
        };
        let changed = next != self.open;
        self.open = next;
        changed
    }

    /// 显式打开指定菜单。
    pub fn open(&mut self, index: usize) -> bool {
        if self.open == Some(index) {
            return false;
        }
        self.open = Some(index);
        true
    }

    /// 关闭所有菜单。
    pub fn close(&mut self) -> bool {
        self.open.take().is_some()
    }

    /// 将指针移动到某个菜单项上方。
    ///
    /// 当已有菜单打开时，悬停同级菜单项会立即打开它；当菜单栏关闭时，悬停仅记录高亮。
    pub fn hover(&mut self, index: usize) -> bool {
        if self.hovered == Some(index) {
            return false;
        }
        self.hovered = Some(index);
        if self.open.is_some() && self.open != Some(index) {
            self.open = Some(index);
            return true;
        }
        true
    }

    /// 清除悬停高亮。
    pub fn leave(&mut self) {
        self.hovered = None;
    }
}

/// 聚焦状态下的菜单栏识别的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuBarKey {
    /// 移至上一个菜单。
    Left,
    /// 移至下一个菜单。
    Right,
    /// 打开或激活当前聚焦的菜单。
    Enter,
    /// 关闭当前打开的菜单。
    Escape,
}

/// 归约单次按键。
///
/// 返回打开的菜单是否发生变化。Left/Right 仅在菜单打开时生效；
/// 菜单栏关闭时方向键交由应用的焦点顺序处理。
#[must_use]
pub fn reduce_menubar_key(desc: &MenuBarDesc, state: &mut MenuBarState, key: MenuBarKey) -> bool {
    let count = desc.items.len();
    if count == 0 {
        return false;
    }
    match key {
        MenuBarKey::Left | MenuBarKey::Right => {
            if let Some(open) = state.open {
                let next = if key == MenuBarKey::Left {
                    (open + count - 1) % count
                } else {
                    (open + 1) % count
                };
                state.open = Some(next);
                true
            } else {
                false
            }
        }
        MenuBarKey::Enter => match state.open {
            Some(_) => false,
            None => {
                let index = state.hovered.unwrap_or(0);
                state.open = Some(index.min(count - 1));
                true
            }
        },
        MenuBarKey::Escape => state.close(),
    }
}

/// 菜单栏绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuBarTokens {
    /// 菜单栏背景色。
    pub bar: Token,
    /// 闲置菜单项背景色。
    pub item: Token,
    /// 悬停菜单项背景色。
    pub item_hover: Token,
    /// 打开状态菜单项背景色。
    pub item_open: Token,
    /// 菜单项文字颜色。
    pub text: Token,
    /// 打开状态菜单项文字颜色。
    pub text_open: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

impl MenuBarTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            bar: Token::NAV_BG,
            item: Token::NONE,
            item_hover: Token::NAV_HOVER,
            item_open: Token::NAV_SELECTED,
            text: Token::NAV_TEXT,
            text_open: Token::NAV_TEXT,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 发射菜单栏背景。
#[must_use]
pub fn emit_menubar(desc: &MenuBarDesc, tokens: MenuBarTokens) -> WidgetOutput {
    WidgetOutput {
        primitives: vec![surface(desc.rect, 0.0, tokens.bar, desc.z)],
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 发射单个顶层菜单标题。
#[must_use]
pub fn emit_menubar_item(
    desc: &MenuBarDesc,
    state: &MenuBarState,
    index: usize,
    rect: Rect,
    control: ControlState,
    label: &[GlyphQuad],
    tokens: MenuBarTokens,
) -> WidgetOutput {
    let Some(item) = desc.items.get(index) else {
        return WidgetOutput::default();
    };
    let disabled = control.disabled;
    let open = state.open == Some(index);
    let mut primitives = Vec::with_capacity(3 + label.len());
    let mut payloads = Vec::with_capacity(label.len());

    let fill = if open {
        tokens.item_open
    } else if control.phase == PointerPhase::Hovered {
        tokens.item_hover
    } else {
        tokens.item
    };
    primitives.push(outlined(rect, 4.0, fill, Token::NONE, 0.0, 0.0, desc.z));
    glyphs(
        label,
        if open { tokens.text_open } else { tokens.text },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        5.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(item.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [4.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Menu, item.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.expanded = Some(open);
    a11y.selected = open;

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> MenuBarDesc {
        MenuBarDesc::new(
            Rect::new(0.0, 0.0, 400.0, MENUBAR_HEIGHT),
            vec![
                MenuBarItem::new(1, "File"),
                MenuBarItem::new(2, "Edit"),
                MenuBarItem::new(3, "View"),
            ],
        )
    }

    #[test]
    fn items_are_laid_out_left_to_right() {
        let desc = desc();
        let layout = desc.layout(&[]);
        assert_eq!(layout.len(), 3);
        assert_eq!(layout[0].0.origin.x, 0.0);
        assert!(layout[1].0.origin.x >= layout[0].0.max().x);
        assert_eq!(layout[2].0.origin.x, layout[1].0.max().x);
    }

    #[test]
    fn tapping_the_open_menu_closes_it() {
        let mut state = MenuBarState::new();
        assert!(state.toggle(0));
        assert!(state.is_open());
        assert!(state.toggle(0));
        assert!(!state.is_open());
    }

    #[test]
    fn hovering_a_sibling_switches_the_open_menu() {
        let mut state = MenuBarState::new();
        state.open(1);
        assert!(state.hover(2));
        assert_eq!(state.open, Some(2));
        state.close();
        assert!(state.hover(0));
        assert!(!state.is_open(), "a closed bar does not open on hover");
    }

    #[test]
    fn arrow_keys_walk_the_bar_only_while_open() {
        let desc = desc();
        let mut state = MenuBarState::new();
        assert!(!reduce_menubar_key(&desc, &mut state, MenuBarKey::Right));
        state.open(2);
        assert!(reduce_menubar_key(&desc, &mut state, MenuBarKey::Right));
        assert_eq!(state.open, Some(0), "wraps to the first menu");
        assert!(reduce_menubar_key(&desc, &mut state, MenuBarKey::Left));
        assert_eq!(state.open, Some(2));
    }

    #[test]
    fn enter_opens_the_hovered_menu_and_escape_closes_it() {
        let desc = desc();
        let mut state = MenuBarState::new();
        state.hover(1);
        assert!(reduce_menubar_key(&desc, &mut state, MenuBarKey::Enter));
        assert_eq!(state.open, Some(1));
        assert!(reduce_menubar_key(&desc, &mut state, MenuBarKey::Escape));
        assert!(!state.is_open());
        assert!(!reduce_menubar_key(&desc, &mut state, MenuBarKey::Escape));
    }

    #[test]
    fn the_popup_anchors_under_the_item() {
        let desc = desc();
        let anchor = desc.popup_anchor(1, &[]);
        let layout = desc.layout(&[]);
        assert_eq!(anchor.x, layout[1].0.origin.x);
        assert_eq!(anchor.y, MENUBAR_HEIGHT);
    }

    #[test]
    fn an_open_item_is_selected_and_pickable() {
        let desc = desc();
        let mut state = MenuBarState::new();
        state.open(0);
        let rect = desc.layout(&[])[0].0;
        let output = emit_menubar_item(
            &desc,
            &state,
            0,
            rect,
            ControlState::default(),
            &[],
            MenuBarTokens::standard(),
        );
        let a11y = output.a11y.expect("an item is reachable");
        assert_eq!(a11y.role, Role::Menu);
        assert_eq!(a11y.expanded, Some(true));
        assert!(a11y.selected);
        let hit = output.hit.expect("an item is pickable");
        assert_eq!(hit.node, 1);
    }

    #[test]
    fn a_disabled_item_is_not_pickable() {
        let desc = desc();
        let state = MenuBarState::new();
        let rect = desc.layout(&[])[0].0;
        let output = emit_menubar_item(
            &desc,
            &state,
            0,
            rect,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            &[],
            MenuBarTokens::standard(),
        );
        assert!(output.hit.is_none());
        assert!(!output.focusable);
    }
}

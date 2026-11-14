//! 下拉菜单与上下文菜单（Menu）。
//!
//! 菜单即数据：包含菜单项、禁用状态与快捷键。状态机（[`MenuState`]）仅持有当前高亮索引；
//! `Up`/`Down` 会跳过分隔符与禁用项，`Enter` 上报选中的菜单项。
//! 发射器绘制一个背景、每个菜单项一个高亮槽位以及每条分隔符一条线——
//! 同样为固定描述符保持恒定的图元数量。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{HitRegion, PORTAL_LAYER};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::image::IconDesc;
use crate::overlay::OverlayEntry;
use crate::primitive::{instance, surface};
use crate::scroll::{ScrollState, ScrollViewDesc};
use crate::{ControlState, WidgetDesc, WidgetOutput};

/// 菜单的命中测试层级。
pub const MENU_LAYER: u8 = PORTAL_LAYER;
/// 默认行高。
pub const MENU_ITEM_HEIGHT: f32 = 30.0;
const MENU_SHADOW: f32 = 18.0;

/// 菜单中的单行条目。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// 可见标签。
    pub label: String,
    /// 绘制在右侧的快捷键提示（若有）。
    pub shortcut: String,
    /// 该行是否不可选择。
    pub disabled: bool,
    /// 该行是分隔符而非操作项。
    pub separator: bool,
    /// 前置图标（若有）。
    pub icon: Option<IconDesc>,
    /// 可勾选行的勾选状态（`Some` 表示可勾选）。
    pub checked: Option<bool>,
    /// 按 `ArrowRight` 打开的嵌套子菜单（若有）。
    pub submenu: Option<Vec<MenuItem>>,
}

impl MenuItem {
    /// 创建可选择的操作项。
    #[must_use]
    pub fn action(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            disabled: false,
            separator: false,
            icon: None,
            checked: None,
            submenu: None,
        }
    }

    /// 创建水平分隔符。
    #[must_use]
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: String::new(),
            disabled: true,
            separator: true,
            icon: None,
            checked: None,
            submenu: None,
        }
    }

    /// 附加快捷键提示。
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    /// 附加前置图标。
    #[must_use]
    pub fn with_icon(mut self, icon: IconDesc) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 设为可勾选行（已勾选或未勾选）。
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// 附加嵌套子菜单。
    #[must_use]
    pub fn with_submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = Some(items);
        self
    }

    /// 将菜单项设为禁用。
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// 判断菜单项是否可被选中。
    #[must_use]
    pub fn selectable(&self) -> bool {
        !self.disabled && !self.separator
    }

    /// 判断菜单项是否会打开嵌套子菜单。
    #[must_use]
    pub fn has_submenu(&self) -> bool {
        self.submenu.as_ref().is_some_and(|items| !items.is_empty())
    }
}

/// 菜单的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuDesc {
    /// 菜单矩形。
    pub rect: Rect,
    /// 从上到下排列的菜单行列表。
    pub items: Vec<MenuItem>,
    /// 行高。
    pub item_height: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 已知时菜单必须保持在内的视口。
    pub viewport: Option<Rect>,
}

impl MenuDesc {
    #[must_use]
    pub fn new(rect: Rect, items: Vec<MenuItem>) -> Self {
        Self {
            rect,
            items,
            item_height: MENU_ITEM_HEIGHT,
            radius: 8.0,
            z: 905,
            viewport: None,
        }
    }

    /// 所有行所需的总高度。
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.items.len() as f32 * self.item_height
    }

    /// 将菜单高度约束到视口范围内。
    ///
    /// 返回行是否溢出（即是否需要滚动）。
    /// 菜单保持顶边不变并向下收缩，符合锚定在输入框下方的下拉列表行为。
    pub fn clamp_height(&mut self) -> bool {
        let overflow = self.content_height() > self.rect.height() + f32::EPSILON;
        let Some(viewport) = self.viewport else {
            return overflow;
        };
        let available = (viewport.max().y - self.rect.origin.y)
            .max(self.item_height.min(self.content_height()));
        if self.content_height() > available + f32::EPSILON {
            self.rect = Rect::new(
                self.rect.origin.x,
                self.rect.origin.y,
                self.rect.width(),
                available,
            );
            true
        } else {
            false
        }
    }

    /// 沿着从根开始的 `path` 获取嵌套层级的行切片。
    #[must_use]
    pub fn items_at(&self, path: &[usize]) -> Option<&[MenuItem]> {
        let mut level: &[MenuItem] = &self.items;
        for &index in path {
            level = level.get(index)?.submenu.as_deref()?;
        }
        Some(level)
    }

    /// 单行的矩形。
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + index as f32 * self.item_height,
            self.rect.width(),
            self.item_height,
        )
    }

    /// 获取指定坐标点下的行（若有）。
    #[must_use]
    pub fn item_at(&self, point: unbound_math::Vec2) -> Option<usize> {
        self.item_at_offset(point, unbound_math::Vec2::ZERO)
    }

    /// 考虑滚动内容偏移量后，获取指定坐标点下的行。
    #[must_use]
    pub fn item_at_offset(
        &self,
        point: unbound_math::Vec2,
        offset: unbound_math::Vec2,
    ) -> Option<usize> {
        if !self.rect.contains_point(point) {
            return None;
        }
        let index = ((point.y - self.rect.origin.y + offset.y) / self.item_height).floor();
        if index < 0.0 {
            return None;
        }
        let index = index as usize;
        (index < self.items.len()).then_some(index)
    }

    /// 为持有 `Semantics` 的调用方构造单行的无障碍节点。
    #[must_use]
    pub fn item_a11y(&self, index: usize) -> Option<A11yNode> {
        let item = self.items.get(index)?;
        if item.separator {
            return Some(A11yNode::new(Role::Unknown, "", self.item_rect(index)));
        }
        Some(
            A11yNode::new(Role::MenuItem, item.label.clone(), self.item_rect(index))
                .clickable()
                .disabled(item.disabled),
        )
    }
}

/// 菜单的高亮行状态。
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// 当前高亮索引。
    pub highlighted: Option<usize>,
    /// 已展开的子菜单索引路径，最外层在前。
    pub open_path: Vec<usize>,
    /// 当菜单高于视口时的滚动状态。
    ///
    /// 行未溢出时为 `None`。由 [`Self::ensure_scroll`] 安装；滚动偏移量刻意不参与 `PartialEq`，
    /// 因为它是实时指针状态，而非菜单选区的一部分。
    pub scroll: Option<ScrollState>,
}

impl PartialEq for MenuState {
    fn eq(&self, other: &Self) -> bool {
        self.highlighted == other.highlighted && self.open_path == other.open_path
    }
}

impl Eq for MenuState {}

impl MenuState {
    /// 创建无高亮行的菜单状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取当前打开层级的行切片。
    #[must_use]
    pub fn open_items<'a>(&self, desc: &'a MenuDesc) -> &'a [MenuItem] {
        desc.items_at(&self.open_path).unwrap_or(&desc.items)
    }

    /// 移动高亮，自动跳过分隔符与禁用行。
    ///
    /// 返回新的高亮索引。
    pub fn move_highlight(&mut self, desc: &MenuDesc, forward: bool) -> Option<usize> {
        let items = self.open_items(desc);
        if items.is_empty() {
            return None;
        }
        let count = items.len();
        let start = self
            .highlighted
            .unwrap_or(if forward { count - 1 } else { 0 });
        for step in 1..=count {
            let index = if forward {
                (start + step) % count
            } else {
                (start + count - step) % count
            };
            if items[index].selectable() {
                self.highlighted = Some(index);
                return Some(index);
            }
        }
        None
    }

    /// 高亮指针下方的行，并在需要时展开子菜单。
    pub fn hover(&mut self, desc: &MenuDesc, point: unbound_math::Vec2) -> Option<usize> {
        let index = desc.item_at_offset(point, self.scroll_offset())?;
        let item = desc.items.get(index)?;
        if !item.selectable() {
            return self.highlighted;
        }
        if item.has_submenu() {
            self.open_path = vec![index];
            self.highlighted = None;
        } else if self.open_path.is_empty() {
            self.highlighted = Some(index);
        } else {
            // 在根层级悬停叶子项时会关闭已打开的子菜单。
            self.open_path.clear();
            self.highlighted = Some(index);
        }
        self.highlighted
    }

    /// `ArrowRight`：展开高亮行的子菜单。
    ///
    /// 返回是否打开了子菜单。
    pub fn expand(&mut self, desc: &MenuDesc) -> bool {
        let Some(index) = self.highlighted else {
            return false;
        };
        let opens = self
            .open_items(desc)
            .get(index)
            .is_some_and(MenuItem::has_submenu);
        if opens {
            self.open_path.push(index);
            self.highlighted = None;
        }
        opens
    }

    /// `ArrowLeft`：折叠最内层已打开的子菜单。
    ///
    /// 返回是否关闭了子菜单。
    pub fn collapse(&mut self) -> bool {
        let Some(parent) = self.open_path.pop() else {
            return false;
        };
        self.highlighted = Some(parent);
        true
    }

    /// 高亮下一个标签以 `ch` 开头的行。
    ///
    /// 搜索大小写不敏感，且从当前高亮之后开始，因此重复输入同一字母会在匹配项间循环。
    /// 返回新的高亮索引。
    pub fn typeahead(&mut self, desc: &MenuDesc, ch: char) -> Option<usize> {
        let items = self.open_items(desc);
        if items.is_empty() || ch.is_control() {
            return self.highlighted;
        }
        let needle = ch.to_lowercase().next()?;
        let start = self.highlighted.map_or(0, |index| index + 1);
        for step in 0..items.len() {
            let index = (start + step) % items.len();
            let item = &items[index];
            if item.selectable()
                && item
                    .label
                    .chars()
                    .next()
                    .is_some_and(|first| first.to_lowercase().next() == Some(needle))
            {
                self.highlighted = Some(index);
                return Some(index);
            }
        }
        self.highlighted
    }

    /// 提交当前高亮行。
    pub fn activate(&self, desc: &MenuDesc) -> Option<usize> {
        let index = self.highlighted?;
        self.open_items(desc)
            .get(index)
            .filter(|item| item.selectable())
            .map(|_| index)
    }

    /// 安装（或移除）内嵌滚动状态。
    ///
    /// 返回行是否超出裁剪后的菜单高度。
    pub fn ensure_scroll(&mut self, desc: &MenuDesc) -> bool {
        let overflow = desc.content_height() > desc.rect.height() + f32::EPSILON;
        if overflow {
            self.scroll.get_or_insert_with(ScrollState::new);
        } else {
            self.scroll = None;
        }
        overflow
    }

    /// 当前滚动偏移量。
    #[must_use]
    pub fn scroll_offset(&self) -> unbound_math::Vec2 {
        self.scroll
            .as_ref()
            .map_or(unbound_math::Vec2::ZERO, ScrollState::offset)
    }

    /// 按 `delta` 滚动打开的菜单，并限制在内容范围内。
    pub fn scroll_by(&mut self, desc: &MenuDesc, delta: unbound_math::Vec2) {
        self.ensure_scroll(desc);
        let view = ScrollViewDesc::new(
            desc.rect,
            unbound_math::Vec2::new(desc.rect.width(), desc.content_height()),
        );
        if let Some(scroll) = self.scroll.as_mut() {
            scroll.scroll_by(&view, delta);
            scroll.jump_to(&view, scroll.target());
        }
    }
}

/// 绑定了身份标识的菜单描述符。
#[derive(Clone, Debug)]
pub struct MenuWidget {
    /// 菜单节点。
    pub node: u64,
    /// 触发器节点。
    pub anchor_node: u64,
    /// 菜单描述符。
    pub desc: MenuDesc,
    /// 当前高亮行。
    pub state: MenuState,
}

impl WidgetDesc for MenuWidget {
    fn rect(&self) -> Rect {
        self.desc.rect
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_menu(self.node, self.anchor_node, &self.desc, &self.state, state).0
    }
}

/// 发射菜单、其命中区域与浮层栈条目。
#[must_use]
pub fn emit_menu(
    node: u64,
    anchor_node: u64,
    desc: &MenuDesc,
    state: &MenuState,
    control: ControlState,
) -> (WidgetOutput, OverlayEntry) {
    let rect = desc.rect;
    let radii = [desc.radius; 4];
    let offset = state.scroll_offset();
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::with_capacity(3 + desc.items.len() * 2);

    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        Token::NONE,
        0.0,
        MENU_SHADOW,
        PrimType::Shadow,
        SubElement::Shadow,
        desc.z,
    ));
    primitives.push(surface(
        rect,
        desc.radius,
        if control.disabled {
            Token::SURFACE_SUNKEN
        } else {
            Token::OVERLAY_BG
        },
        desc.z,
    ));

    for (index, item) in desc.items.iter().enumerate() {
        let row = desc
            .item_rect(index)
            .translate(unbound_math::Vec2::new(0.0, -offset.y));
        if item.separator {
            let line = Rect::new(row.origin.x + 8.0, row.center().y, row.width() - 16.0, 1.0);
            primitives.push(surface(line, 0.0, Token::DIVIDER, desc.z));
            // 保持行数恒定：为高亮保留一个不可见槽位。
            let mut placeholder = surface(Rect::ZERO, 0.0, Token::NONE, desc.z);
            placeholder.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(desc.z, 0.0);
            primitives.push(placeholder);
            continue;
        }
        let highlighted = state.highlighted == Some(index) && item.selectable();
        let mut slot = surface(
            row,
            4.0,
            if highlighted {
                Token::CONTROL_HOVER
            } else {
                Token::NONE
            },
            desc.z,
        );
        if !highlighted {
            slot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(desc.z, 0.0);
        }
        primitives.push(slot);
        primitives.push(instance(
            row,
            [0.0; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Line,
            SubElement::Content,
            desc.z,
        ));
    }

    primitives.push(instance(
        rect,
        radii,
        Token::NONE,
        Token::OUTLINE,
        1.0,
        0.0,
        PrimType::RoundedRect,
        SubElement::Border,
        desc.z,
    ));

    let hit = Some(HitRegion::new(node, MENU_LAYER, u32::from(desc.z), rect));
    let a11y = A11yNode::new(Role::Menu, "", rect).disabled(control.disabled);

    (
        WidgetOutput {
            primitives,
            payloads: Vec::new(),
            hit,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        OverlayEntry::menu(node, anchor_node, rect),
    )
}

/// 菜单中的分隔符行。
#[must_use]
pub fn separator_indices(desc: &MenuDesc) -> Vec<usize> {
    desc.items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.separator.then_some(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> MenuDesc {
        MenuDesc::new(
            Rect::new(100.0, 100.0, 220.0, 150.0),
            vec![
                MenuItem::action("Open").with_shortcut("Ctrl+O"),
                MenuItem::separator(),
                MenuItem::action("Disabled").disabled(),
                MenuItem::action("Quit"),
            ],
        )
    }

    #[test]
    fn arrow_navigation_skips_separators_and_disabled_rows() {
        let desc = desc();
        let mut state = MenuState::new();
        assert_eq!(state.move_highlight(&desc, true), Some(0));
        assert_eq!(state.move_highlight(&desc, true), Some(3));
        assert_eq!(state.move_highlight(&desc, true), Some(0));
        assert_eq!(state.move_highlight(&desc, false), Some(3));
    }

    #[test]
    fn enter_returns_the_highlighted_action_only() {
        let desc = desc();
        let mut state = MenuState::new();
        assert_eq!(state.activate(&desc), None);
        state.highlighted = Some(2);
        assert_eq!(
            state.activate(&desc),
            None,
            "a disabled row cannot activate"
        );
        state.highlighted = Some(3);
        assert_eq!(state.activate(&desc), Some(3));
    }

    #[test]
    fn hovering_a_disabled_row_does_not_highlight_it() {
        let desc = desc();
        let mut state = MenuState::new();
        assert_eq!(
            state.hover(&desc, unbound_math::Vec2::new(120.0, 175.0)),
            None
        );
        assert_eq!(
            state.hover(&desc, unbound_math::Vec2::new(120.0, 115.0)),
            Some(0)
        );
    }

    #[test]
    fn the_primitive_count_is_constant_for_a_fixed_menu() {
        let desc = desc();
        let state = MenuState::new();
        let control = ControlState::default();
        let a = emit_menu(1, 2, &desc, &state, control).0.primitives.len();
        let mut highlighted = MenuState::new();
        highlighted.highlighted = Some(0);
        let b = emit_menu(1, 2, &desc, &highlighted, control)
            .0
            .primitives
            .len();
        assert_eq!(a, b);
    }

    #[test]
    fn a_menu_exposes_menu_item_semantics() {
        let desc = desc();
        let item = desc.item_a11y(0).unwrap();
        assert_eq!(item.role, Role::MenuItem);
        assert_eq!(item.label, "Open");
        assert!(desc.item_a11y(1).unwrap().label.is_empty());
        assert_eq!(separator_indices(&desc), vec![1]);
    }
}

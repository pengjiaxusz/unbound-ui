//! Dropdown and context menus.
//!
//! A menu is data: items, their disabled state and their shortcuts. The state
//! machine ([`MenuState`]) owns only the highlighted index; `Up`/`Down` skip
//! separators and disabled entries, and `Enter` reports the chosen item. The
//! emitter paints one background, one highlight slot per item and one line per
//! separator — again a constant primitive count for a fixed descriptor.

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

/// The hit-test layer of a menu.
pub const MENU_LAYER: u8 = PORTAL_LAYER;
/// The default row height.
pub const MENU_ITEM_HEIGHT: f32 = 30.0;
const MENU_SHADOW: f32 = 18.0;

/// One row of a menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// The visible label.
    pub label: String,
    /// The shortcut hint drawn on the right, if any.
    pub shortcut: String,
    /// The row is not selectable.
    pub disabled: bool,
    /// The row is a separator rather than an action.
    pub separator: bool,
    /// The leading icon, if any.
    pub icon: Option<IconDesc>,
    /// `Some` for a checkable row: the check state.
    pub checked: Option<bool>,
    /// The nested menu opened with `ArrowRight`, if any.
    pub submenu: Option<Vec<MenuItem>>,
}

impl MenuItem {
    /// A selectable action.
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

    /// A horizontal separator.
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

    /// The same item with a shortcut hint.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    /// The same item with a leading icon.
    #[must_use]
    pub fn with_icon(mut self, icon: IconDesc) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The same item as a checked (or unchecked) checkable row.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// The same item with a nested submenu.
    #[must_use]
    pub fn with_submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = Some(items);
        self
    }

    /// The same item, disabled.
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// Whether the item can be selected.
    #[must_use]
    pub fn selectable(&self) -> bool {
        !self.disabled && !self.separator
    }

    /// Whether the item opens a nested menu.
    #[must_use]
    pub fn has_submenu(&self) -> bool {
        self.submenu.as_ref().is_some_and(|items| !items.is_empty())
    }
}

/// The pure descriptor of a menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuDesc {
    /// The menu rectangle.
    pub rect: Rect,
    /// The rows, top to bottom.
    pub items: Vec<MenuItem>,
    /// The row height.
    pub item_height: f32,
    /// The corner radius.
    pub radius: f32,
    /// The draw order key.
    pub z: u16,
    /// The viewport the menu must stay inside, when known.
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

    /// The total height the rows need.
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.items.len() as f32 * self.item_height
    }

    /// Clamp the menu height to its viewport.
    ///
    /// Returns whether the rows overflow, i.e. whether a scroll is needed.
    /// The menu keeps its top edge and shrinks downwards, which is the
    /// behaviour of a dropdown anchored under a field.
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

    /// The rows of a nested level, following `path` from the root.
    #[must_use]
    pub fn items_at(&self, path: &[usize]) -> Option<&[MenuItem]> {
        let mut level: &[MenuItem] = &self.items;
        for &index in path {
            level = level.get(index)?.submenu.as_deref()?;
        }
        Some(level)
    }

    /// The rectangle of one row.
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + index as f32 * self.item_height,
            self.rect.width(),
            self.item_height,
        )
    }

    /// The row under a point, if any.
    #[must_use]
    pub fn item_at(&self, point: unbound_math::Vec2) -> Option<usize> {
        self.item_at_offset(point, unbound_math::Vec2::ZERO)
    }

    /// The row under a point, accounting for a scrolled content offset.
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

    /// The accessible node of one row, for a caller that owns a `Semantics`.
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

/// The highlighted row of a menu.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// The highlighted index.
    pub highlighted: Option<usize>,
    /// The indices of the expanded submenus, outermost first.
    pub open_path: Vec<usize>,
    /// The scroll state of a menu taller than its viewport.
    ///
    /// `None` while the rows fit. [`Self::ensure_scroll`] installs it; the
    /// scroll offset is deliberately excluded from `PartialEq`, because it is
    /// live pointer state rather than part of the menu's selection.
    pub scroll: Option<ScrollState>,
}

impl PartialEq for MenuState {
    fn eq(&self, other: &Self) -> bool {
        self.highlighted == other.highlighted && self.open_path == other.open_path
    }
}

impl Eq for MenuState {}

impl MenuState {
    /// A menu with no row highlighted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows of the currently open level.
    #[must_use]
    pub fn open_items<'a>(&self, desc: &'a MenuDesc) -> &'a [MenuItem] {
        desc.items_at(&self.open_path).unwrap_or(&desc.items)
    }

    /// Move the highlight, skipping separators and disabled rows.
    ///
    /// Returns the new index.
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

    /// Highlight the row under the pointer, expanding a submenu.
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
            // Hovering a leaf at the root level closes the open submenu.
            self.open_path.clear();
            self.highlighted = Some(index);
        }
        self.highlighted
    }

    /// `ArrowRight`: expand the highlighted row's submenu.
    ///
    /// Returns whether a submenu opened.
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

    /// `ArrowLeft`: collapse the innermost open submenu.
    ///
    /// Returns whether a submenu closed.
    pub fn collapse(&mut self) -> bool {
        let Some(parent) = self.open_path.pop() else {
            return false;
        };
        self.highlighted = Some(parent);
        true
    }

    /// Highlight the next row whose label starts with `ch`.
    ///
    /// The search is case-insensitive and starts after the current highlight,
    /// so repeatedly typing the same letter cycles through the matches.
    /// Returns the highlighted index.
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

    /// Commit the highlighted row.
    pub fn activate(&self, desc: &MenuDesc) -> Option<usize> {
        let index = self.highlighted?;
        self.open_items(desc)
            .get(index)
            .filter(|item| item.selectable())
            .map(|_| index)
    }

    /// Install (or drop) the embedded scroll state.
    ///
    /// Returns whether the rows overflow the clamped menu height.
    pub fn ensure_scroll(&mut self, desc: &MenuDesc) -> bool {
        let overflow = desc.content_height() > desc.rect.height() + f32::EPSILON;
        if overflow {
            self.scroll.get_or_insert_with(ScrollState::new);
        } else {
            self.scroll = None;
        }
        overflow
    }

    /// The current scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> unbound_math::Vec2 {
        self.scroll
            .as_ref()
            .map_or(unbound_math::Vec2::ZERO, ScrollState::offset)
    }

    /// Scroll the open menu by `delta`, clamped to its content.
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

/// A menu descriptor bound to an identity.
#[derive(Clone, Debug)]
pub struct MenuWidget {
    /// The menu node.
    pub node: u64,
    /// The trigger node.
    pub anchor_node: u64,
    /// The descriptor.
    pub desc: MenuDesc,
    /// The highlighted row.
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

/// Emit a menu, its hit region and its stack entry.
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
            // Keep the row count constant: an invisible slot for the highlight.
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

/// The separator rows of a menu.
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

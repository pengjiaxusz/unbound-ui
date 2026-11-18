//! ItemsView：统一列表 / 网格布局的集合视图。
//!
//! 与 `GridView` 的区别在于布局可切换：`List` 单列堆叠，`Grid` 按列换行。
//! 两种布局下每个条目始终输出「填充 + 选中描边」两个图元。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 条目间距。
pub const ITEMS_VIEW_GAP: f32 = 6.0;
/// 条目圆角。
pub const ITEMS_VIEW_RADIUS: f32 = 4.0;

/// 集合布局方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemsLayout {
    /// 单列列表。
    #[default]
    List,
    /// 多列网格。
    Grid,
}

/// 统一集合视图描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ItemsViewDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 列数；`List` 布局下恒为 1。
    pub columns: usize,
    /// 条目总数。
    pub items: usize,
    /// 布局方式。
    pub layout: ItemsLayout,
    /// 选中条目索引。
    pub selected: Option<usize>,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl ItemsViewDesc {
    /// 创建 `items` 个条目的集合视图。
    #[must_use]
    pub fn new(rect: Rect, items: usize) -> Self {
        Self {
            rect,
            columns: 3,
            items: items.max(1),
            layout: ItemsLayout::List,
            selected: None,
            disabled: false,
            z: 718,
        }
    }

    /// 切换为网格布局并指定列数。
    #[must_use]
    pub fn grid(mut self, columns: usize) -> Self {
        self.layout = ItemsLayout::Grid;
        self.columns = columns.max(1);
        self
    }

    /// 设置选中条目。
    #[must_use]
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 有效列数：列表布局恒为 1。
    #[must_use]
    pub fn effective_columns(&self) -> usize {
        match self.layout {
            ItemsLayout::List => 1,
            ItemsLayout::Grid => self.columns.max(1),
        }
    }

    /// 行数。
    #[must_use]
    pub fn rows(&self) -> usize {
        let columns = self.effective_columns();
        (self.items + columns - 1) / columns
    }

    /// 第 `index` 个条目的矩形。
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.items {
            return None;
        }
        let columns = self.effective_columns();
        let rows = self.rows().max(1);
        let col = index % columns;
        let row = index / columns;
        let width = (self.rect.width() - ITEMS_VIEW_GAP * (columns.saturating_sub(1)) as f32)
            / columns as f32;
        let height =
            (self.rect.height() - ITEMS_VIEW_GAP * (rows.saturating_sub(1)) as f32) / rows as f32;
        Some(Rect::new(
            self.rect.origin.x + col as f32 * (width + ITEMS_VIEW_GAP),
            self.rect.origin.y + row as f32 * (height + ITEMS_VIEW_GAP),
            width.max(0.0),
            height.max(0.0),
        ))
    }
}

/// 输出统一集合视图的图元、命中区域与语义节点。
#[must_use]
pub fn emit_items_view(
    node: u64,
    desc: &ItemsViewDesc,
    control: ControlState,
    item_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(3 + desc.items * 2 + item_glyphs.len());
    let mut payloads = Vec::with_capacity(item_glyphs.len());

    primitives.push(outlined(
        rect,
        8.0,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::SURFACE
        },
        if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if hovered {
            Token::INPUT_BORDER_HOVER
        } else {
            Token::CONTROL_STROKE
        },
        1.0,
        0.0,
        z,
    ));

    for index in 0..desc.items {
        let Some(item) = desc.item_rect(index) else {
            continue;
        };
        let selected = desc.selected == Some(index);
        primitives.push(surface(
            item,
            ITEMS_VIEW_RADIUS,
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else if selected {
                Token::PRIMARY_SOFT
            } else if hovered {
                Token::CONTROL_HOVER
            } else {
                Token::CARD_BG
            },
            z + 1,
        ));
        primitives.push(outlined(
            item,
            ITEMS_VIEW_RADIUS,
            Token::NONE,
            if disabled {
                Token::NONE
            } else if selected {
                Token::PRIMARY
            } else {
                Token::CARD_BORDER
            },
            if selected { 2.0 } else { 1.0 },
            0.0,
            z + 2,
        ));
    }

    emit_glyphs(
        item_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded { radii: [8.0; 4] })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Grid, "Items view", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        for index in 0..desc.items {
            if let Some(item) = desc.item_rect(index) {
                let key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x900 + index as u64);
                out.with_sub_control(
                    key,
                    HitRegion::new(key, CONTROL_LAYER, u32::from(z) + 1, item)
                        .with_cursor(CursorKind::Pointer),
                    A11yNode::new(Role::ListItem, format!("Item {}", index + 1), item).clickable(),
                    true,
                );
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ItemsViewDesc {
        ItemsViewDesc::new(Rect::new(0.0, 0.0, 300.0, 120.0), 6)
    }

    #[test]
    fn the_list_layout_stacks_one_column() {
        let d = desc();
        assert_eq!(d.effective_columns(), 1);
        assert_eq!(d.rows(), 6);
        let first = d.item_rect(0).expect("item 0");
        let second = d.item_rect(1).expect("item 1");
        assert!((first.origin.x - second.origin.x).abs() < 1e-4);
        assert!(first.max().y <= second.origin.y);
    }

    #[test]
    fn the_grid_layout_wraps_into_columns() {
        let d = desc().grid(3);
        assert_eq!(d.effective_columns(), 3);
        assert_eq!(d.rows(), 2);
        let first = d.item_rect(0).expect("item 0");
        let second = d.item_rect(1).expect("item 1");
        assert!(first.max().x <= second.origin.x);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().grid(3).selected(2);
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let state = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        counts.push(emit_items_view(1, &d, state, &mut []).primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

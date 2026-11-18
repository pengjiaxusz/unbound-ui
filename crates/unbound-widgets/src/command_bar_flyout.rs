//! CommandBarFlyout：文本选中或右键时弹出的迷你图标工具条。
//!
//! 浮层由一串等宽图标槽位、一条竖向分隔线与右侧「更多」按钮组成。
//! 槽位数量由描述符固定，悬停与按下只切换填充令牌。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 浮层圆角。
pub const COMMAND_BAR_FLYOUT_RADIUS: f32 = 8.0;
/// 图标槽位边长。
pub const COMMAND_BAR_FLYOUT_ITEM: f32 = 32.0;

/// 迷你工具条浮层描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CommandBarFlyoutDesc {
    /// 浮层矩形。
    pub rect: Rect,
    /// 左侧图标槽位数量（至少 1）。
    pub items: usize,
    /// 是否绘制右侧「更多」按钮。
    pub show_more: bool,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl CommandBarFlyoutDesc {
    /// 创建包含四个图标槽位的浮层。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            items: 4,
            show_more: true,
            radius: COMMAND_BAR_FLYOUT_RADIUS,
            disabled: false,
            z: 760,
        }
    }

    /// 设置图标槽位数量。
    #[must_use]
    pub fn with_items(mut self, items: usize) -> Self {
        self.items = items.max(1);
        self
    }

    /// 隐藏「更多」按钮。
    #[must_use]
    pub fn without_more(mut self) -> Self {
        self.show_more = false;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 第 `index` 个图标槽位的矩形。
    #[must_use]
    pub fn item_rect(&self, index: usize) -> Rect {
        let pad = 4.0;
        let height = (self.rect.height() - pad * 2.0).max(0.0);
        Rect::new(
            self.rect.origin.x + pad + index as f32 * (COMMAND_BAR_FLYOUT_ITEM + 2.0),
            self.rect.origin.y + pad,
            COMMAND_BAR_FLYOUT_ITEM.min(height.max(0.0)),
            height,
        )
    }

    /// 右侧「更多」按钮矩形。
    #[must_use]
    pub fn more_rect(&self) -> Rect {
        let pad = 4.0;
        let size = (self.rect.height() - pad * 2.0).max(0.0);
        Rect::new(
            self.rect.max().x - size - pad,
            self.rect.origin.y + pad,
            size,
            size,
        )
    }

    /// 竖向分隔线矩形。
    #[must_use]
    pub fn divider_rect(&self) -> Rect {
        let more = self.more_rect();
        Rect::new(
            more.origin.x - 5.0,
            self.rect.origin.y + 8.0,
            1.0,
            (self.rect.height() - 16.0).max(0.0),
        )
    }
}

/// 输出迷你工具条浮层的图元、命中区域与语义节点。
#[must_use]
pub fn emit_command_bar_flyout(
    node: u64,
    desc: &CommandBarFlyoutDesc,
    control: ControlState,
    icon_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;
    let pressed = !disabled && control.phase == PointerPhase::Pressed;

    let item_fill = if disabled {
        Token::NONE
    } else if pressed {
        Token::SURFACE_SUNKEN
    } else if hovered {
        Token::CONTROL_HOVER
    } else {
        Token::NONE
    };

    let mut primitives = Vec::with_capacity(desc.items + 4 + icon_glyphs.len());
    let mut payloads = Vec::with_capacity(icon_glyphs.len());

    primitives.push(outlined(
        rect,
        desc.radius,
        Token::OVERLAY_BG,
        if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::CARD_BORDER
        },
        1.0,
        4.0,
        z,
    ));

    for index in 0..desc.items {
        let item = desc.item_rect(index);
        primitives.push(outlined(item, 4.0, item_fill, Token::NONE, 0.0, 0.0, z + 1));
    }

    primitives.push(surface(desc.divider_rect(), 0.0, Token::DIVIDER, z + 2));
    primitives.push(outlined(
        desc.more_rect(),
        4.0,
        item_fill,
        Token::NONE,
        0.0,
        0.0,
        z + 3,
    ));

    emit_glyphs(
        icon_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 4,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Toolbar, "Command bar flyout", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CommandBarFlyoutDesc {
        CommandBarFlyoutDesc::new(Rect::new(0.0, 0.0, 220.0, 44.0))
    }

    #[test]
    fn items_stay_inside_the_flyout() {
        let d = desc();
        for index in 0..d.items {
            let item = d.item_rect(index);
            assert!(d.rect.contains_rect(&item), "{index}: {item:?}");
        }
        assert!(d.rect.contains_rect(&d.more_rect()));
    }

    #[test]
    fn the_divider_sits_before_the_more_button() {
        let d = desc();
        assert!(d.divider_rect().max().x <= d.more_rect().origin.x);
    }

    #[test]
    fn hovering_lights_up_the_item_slots() {
        let d = desc();
        let idle = emit_command_bar_flyout(1, &d, ControlState::default(), &mut []);
        let hovered = emit_command_bar_flyout(
            1,
            &d,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_eq!(
            idle.primitives[1].fill_token_or_uv as u16,
            Token::NONE.raw()
        );
        assert_eq!(
            hovered.primitives[1].fill_token_or_uv as u16,
            Token::CONTROL_HOVER.raw()
        );
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc();
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
                        counts.push(
                            emit_command_bar_flyout(1, &d, state, &mut [])
                                .primitives
                                .len(),
                        );
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

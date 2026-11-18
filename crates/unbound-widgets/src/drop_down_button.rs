//! DropDownButton：右侧固定 Chevron 的菜单触发器按钮。
//!
//! 与 [`crate::split_button::SplitButton`] 的区别在于语义：分裂按钮左侧是
//! 「执行主操作」、右侧才是下拉；本控件整块都可点击，点击只负责打开菜单，
//! 因此只有单一命中区域，展开状态通过 `selected` 语义与箭头配色表达。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const DROP_DOWN_BUTTON_RADIUS: f32 = 4.0;
/// 右侧下拉箭头距控件右沿的距离。
pub const DROP_DOWN_BUTTON_CHEVRON_INSET: f32 = 16.0;

/// 下拉按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DropDownButtonDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签与显示文本。
    pub label: String,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl DropDownButtonDesc {
    /// 使用默认度量创建下拉按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            radius: DROP_DOWN_BUTTON_RADIUS,
            disabled: false,
            z: 707,
        }
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 下拉按钮状态：菜单是否展开。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropDownButtonState {
    /// 菜单是否展开。
    pub open: bool,
}

impl DropDownButtonState {
    /// 创建收起的菜单状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换展开状态。
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

/// 输出下拉按钮的图元、命中区域与语义节点。
#[must_use]
pub fn emit_drop_down_button(
    node: u64,
    desc: &DropDownButtonDesc,
    state: &DropDownButtonState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let radii = [desc.radius; 4];
    let disabled = control.disabled || desc.disabled;
    let open = state.open || control.selected;

    // DropDownButtonBackground：静息 `ControlFillColorDefault`，
    // 悬停 `ControlFillColorSecondary`，按下/展开 `ControlFillColorTertiary`。
    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == PointerPhase::Pressed || open {
        Token::SURFACE_SUNKEN
    } else if control.phase == PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::CONTROL_FILL
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::CONTROL_STROKE
    };
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let chevron_ink = if disabled {
        Token::TEXT_DISABLED
    } else if open {
        Token::PRIMARY
    } else {
        Token::TEXT_SECONDARY
    };

    let mut primitives = Vec::with_capacity(5 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len() + 2);

    primitives.push(surface(rect, desc.radius, fill, z));
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z + 1,
    ));

    // 固定右侧 Chevron：两条下行描边。
    let cx = rect.max().x - DROP_DOWN_BUTTON_CHEVRON_INSET;
    let cy = rect.center().y;
    curve_stroke(
        [cx - 4.0, cy - 2.0],
        [cx, cy + 2.0],
        1.5,
        chevron_ink,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [cx, cy + 2.0],
        [cx + 4.0, cy - 2.0],
        1.5,
        chevron_ink,
        z + 2,
        &mut primitives,
        &mut payloads,
    );

    emit_glyphs(glyphs, ink, z + 2, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !disabled,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });

    // 整块按钮只有一个命中区域：点击任意位置都打开/关闭菜单。
    let a11y = A11yNode::new(Role::Button, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled)
        .expanded(open);
    let mut a11y = a11y;
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

    fn desc() -> DropDownButtonDesc {
        DropDownButtonDesc::new(Rect::new(0.0, 0.0, 150.0, 32.0), "Options")
    }

    #[test]
    fn opening_the_menu_changes_the_background_slot() {
        let closed = emit_drop_down_button(
            1,
            &desc(),
            &DropDownButtonState::new(),
            ControlState::default(),
            &mut [],
        );
        let open = emit_drop_down_button(
            1,
            &desc(),
            &DropDownButtonState { open: true },
            ControlState::default(),
            &mut [],
        );
        assert_ne!(closed.primitives[0], open.primitives[0]);
        assert_eq!(
            open.primitives[0].fill_token_or_uv as u16,
            Token::SURFACE_SUNKEN.raw()
        );
        let node = open.a11y.expect("reachable");
        assert_eq!(node.role, Role::Button);
        assert_eq!(node.expanded, Some(true));
        assert!(node.selected);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
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
                            emit_drop_down_button(
                                1,
                                &desc(),
                                &DropDownButtonState::new(),
                                state,
                                &mut [],
                            )
                            .primitives
                            .len(),
                        );
                    }
                }
            }
        }
        assert!(counts.windows(2).all(|pair| pair[0] == pair[1]), "{counts:?}");
    }

    #[test]
    fn toggling_flips_the_open_flag() {
        let mut state = DropDownButtonState::new();
        assert!(!state.open);
        state.toggle();
        assert!(state.open);
        state.toggle();
        assert!(!state.open);
    }
}

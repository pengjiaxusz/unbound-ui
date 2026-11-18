//! ToggleSplitButton：左侧主按钮带选中态、右侧为下拉区的组合按钮。
//!
//! 左侧是「保持选中」的动作按钮，右侧只负责打开菜单；两个区域各自拥有
//! 命中区域与语义节点。选中态用强调色填充左侧，右侧保持中性表面。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 右侧下拉区的固定宽度。对齐 WinUI `SplitButtonSecondaryButtonSize = 32`。
pub const TOGGLE_SPLIT_DROPDOWN_WIDTH: f32 = 32.0;
/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const TOGGLE_SPLIT_RADIUS: f32 = 4.0;

/// 切换分裂按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ToggleSplitButtonDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签与主按钮文本。
    pub label: String,
    /// 圆角半径。
    pub radius: f32,
    /// 右侧下拉区宽度。
    pub split_width: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl ToggleSplitButtonDesc {
    /// 使用默认度量创建切换分裂按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            radius: TOGGLE_SPLIT_RADIUS,
            split_width: TOGGLE_SPLIT_DROPDOWN_WIDTH,
            disabled: false,
            z: 708,
        }
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 左侧主操作区矩形。
    #[must_use]
    pub fn main_rect(&self) -> Rect {
        let split = self.split_width.min(self.rect.width() * 0.4);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            (self.rect.size.x - split).max(0.0),
            self.rect.size.y,
        )
    }

    /// 右侧下拉区矩形。
    #[must_use]
    pub fn dropdown_rect(&self) -> Rect {
        let main = self.main_rect();
        Rect::new(
            main.max().x,
            self.rect.origin.y,
            (self.rect.size.x - main.size.x).max(0.0),
            self.rect.size.y,
        )
    }
}

/// 切换分裂按钮状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToggleSplitButtonState {
    /// 左侧主按钮是否处于选中态。
    pub checked: bool,
    /// 右侧菜单是否展开。
    pub menu_open: bool,
}

impl ToggleSplitButtonState {
    /// 创建未选中、菜单收起的状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 切换左侧选中态。
    pub fn toggle(&mut self) {
        self.checked = !self.checked;
    }
}

/// 输出切换分裂按钮的图元、两个命中区域与语义节点。
#[must_use]
pub fn emit_toggle_split_button(
    node: u64,
    desc: &ToggleSplitButtonDesc,
    state: &ToggleSplitButtonState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let radius = desc.radius;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let checked = state.checked || control.selected;
    let main_rect = desc.main_rect();
    let arrow_rect = desc.dropdown_rect();
    let divider_rect = Rect::new(
        main_rect.max().x - 0.5,
        rect.origin.y,
        1.0,
        rect.height(),
    );

    // 左侧选中态：`AccentFillColorDefault` / `Secondary` / `Tertiary`；
    // 未选中沿用 SplitButton 的中性表面。
    let main_fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if checked {
        match control.phase {
            PointerPhase::Pressed => Token::PRIMARY_PRESSED,
            PointerPhase::Hovered => Token::PRIMARY_HOVER,
            PointerPhase::Idle => Token::PRIMARY,
        }
    } else {
        match control.phase {
            PointerPhase::Pressed => Token::SURFACE_SUNKEN,
            PointerPhase::Hovered => Token::CONTROL_HOVER,
            PointerPhase::Idle => Token::CONTROL_FILL,
        }
    };
    let arrow_fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if state.menu_open {
        if checked {
            Token::PRIMARY_PRESSED
        } else {
            Token::SURFACE_SUNKEN
        }
    } else {
        main_fill
    };
    let divider = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if checked {
        Token::PRIMARY_PRESSED
    } else {
        Token::CONTROL_STROKE
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if checked {
        Token::PRIMARY
    } else {
        Token::CONTROL_STROKE
    };
    let main_ink = if disabled {
        Token::TEXT_DISABLED
    } else if checked {
        Token::ON_PRIMARY
    } else {
        Token::TEXT_PRIMARY
    };
    let arrow_ink = if disabled {
        Token::TEXT_DISABLED
    } else if checked {
        Token::ON_PRIMARY
    } else {
        Token::TEXT_SECONDARY
    };

    let mut primitives = Vec::with_capacity(7 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len() + 2);

    primitives.push(instance_rounded(
        main_rect,
        [radius, 0.0, 0.0, radius],
        main_fill,
        z,
    ));
    primitives.push(instance_rounded(
        arrow_rect,
        [0.0, radius, radius, 0.0],
        arrow_fill,
        z + 1,
    ));
    primitives.push(surface(divider_rect, 0.0, divider, z + 2));
    primitives.push(outlined(
        rect,
        radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z + 3,
    ));

    let acx = arrow_rect.max().x - 18.0;
    let acy = arrow_rect.center().y;
    curve_stroke(
        [acx - 4.0, acy - 2.0],
        [acx, acy + 2.0],
        1.5,
        arrow_ink,
        z + 4,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [acx, acy + 2.0],
        [acx + 4.0, acy - 2.0],
        1.5,
        arrow_ink,
        z + 4,
        &mut primitives,
        &mut payloads,
    );
    emit_glyphs(glyphs, main_ink, z + 4, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        radius,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    let main_node = node ^ 0xFE;
    let arrow_node = node ^ 0xFF;

    let mut out = WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    };
    if !disabled {
        out.hit = Some(
            HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded { radii: [radius; 4] })
                .with_cursor(CursorKind::Pointer),
        );
        out.sub_hits.push(
            HitRegion::new(main_node, BUTTON_LAYER, u32::from(z + 4), main_rect)
                .with_shape(Shape::Rounded {
                    radii: [radius, 0.0, 0.0, radius],
                })
                .with_cursor(CursorKind::Pointer),
        );
        out.sub_hits.push(
            HitRegion::new(arrow_node, BUTTON_LAYER, u32::from(z + 4), arrow_rect)
                .with_shape(Shape::Rounded {
                    radii: [0.0, radius, radius, 0.0],
                })
                .with_cursor(CursorKind::Pointer),
        );
    }

    let mut root = A11yNode::new(Role::Button, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled);
    root.selected = checked;
    out.a11y = Some(root);
    let mut main_a11y = A11yNode::new(Role::Button, desc.label.clone(), main_rect)
        .clickable()
        .disabled(disabled);
    main_a11y.selected = checked;
    out.sub_a11y.push((main_node, main_a11y));
    out.sub_a11y.push((
        arrow_node,
        A11yNode::new(Role::Button, "More options", arrow_rect)
            .clickable()
            .disabled(disabled)
            .expanded(state.menu_open),
    ));
    out.sub_focus_order.push(main_node);
    out.sub_focus_order.push(arrow_node);
    out.focusable = !disabled;
    out
}

fn instance_rounded(
    rect: Rect,
    radii: [f32; 4],
    fill: Token,
    z: u16,
) -> unbound_gpu::data::GpuPrimitiveInstance {
    crate::primitive::instance(
        rect,
        radii,
        fill,
        Token::NONE,
        0.0,
        0.0,
        unbound_gpu::data::PrimType::RoundedRect,
        unbound_gpu::data::SubElement::Background,
        z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> ToggleSplitButtonDesc {
        ToggleSplitButtonDesc::new(Rect::new(0.0, 0.0, 180.0, 32.0), "Bold")
    }

    #[test]
    fn the_two_halves_have_separate_hit_regions() {
        let out = emit_toggle_split_button(
            1,
            &desc(),
            &ToggleSplitButtonState::new(),
            ControlState::default(),
            &mut [],
        );
        assert_eq!(out.sub_hits.len(), 2);
        assert!(out.sub_hits[0].bounds.max().x <= out.sub_hits[1].bounds.origin.x + 0.5);
        assert_eq!(out.sub_hits[0].node, 1 ^ 0xFE);
        assert_eq!(out.sub_hits[1].node, 1 ^ 0xFF);
        assert_eq!(out.sub_focus_order.len(), 2);
    }

    #[test]
    fn checking_the_main_half_fills_it_with_the_accent_slot() {
        let out = emit_toggle_split_button(
            1,
            &desc(),
            &ToggleSplitButtonState {
                checked: true,
                menu_open: false,
            },
            ControlState::default(),
            &mut [],
        );
        assert_eq!(
            out.primitives[0].fill_token_or_uv as u16,
            Token::PRIMARY.raw()
        );
        let node = out.sub_a11y[0].1.clone();
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
                        let split = ToggleSplitButtonState {
                            checked: selected,
                            menu_open: focused,
                        };
                        counts.push(
                            emit_toggle_split_button(1, &desc(), &split, state, &mut [])
                                .primitives
                                .len(),
                        );
                    }
                }
            }
        }
        assert!(counts.windows(2).all(|pair| pair[0] == pair[1]), "{counts:?}");
    }
}

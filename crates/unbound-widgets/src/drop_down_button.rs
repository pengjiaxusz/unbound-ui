//! DropDownButton：右侧固定 Chevron 的菜单触发器按钮。
//!
//! 与 [`crate::split_button::SplitButton`] 的区别在于语义：分裂按钮左侧是
//! 「执行主操作」、右侧才是下拉；本控件整块都可点击，点击只负责打开菜单，
//! 因此只有单一命中区域，展开状态通过 `selected` 与 `expanded` 语义及箭头配色表达。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const DROP_DOWN_BUTTON_RADIUS: f32 = 4.0;
/// 标准控件高度。对齐 WinUI `DropDownButton` 的 32px。
pub const DROP_DOWN_BUTTON_HEIGHT: f32 = 32.0;
/// 右侧下拉箭头距控件右沿的距离。
pub const DROP_DOWN_BUTTON_CHEVRON_INSET: f32 = 16.0;

/// 下拉按钮支持的键盘按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropDownButtonKey {
    /// Enter 键：切换菜单展开状态。
    Enter,
    /// Space 键：切换菜单展开状态。
    Space,
    /// 下箭头（或 Alt+Down）：展开菜单。
    ArrowDown,
    /// Escape 键：收起关闭菜单。
    Escape,
}

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

    /// 使用标准高度便捷构造。
    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::new(0.0, 0.0, 120.0, DROP_DOWN_BUTTON_HEIGHT), label)
    }

    /// 设置圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 设置绘制图层顺序键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
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

    /// 展开菜单。
    pub fn open(&mut self) {
        self.open = true;
    }

    /// 收起关闭菜单。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// 处理键盘按键交互。若状态发生改变则返回 `true`。
    pub fn handle_key(&mut self, key: DropDownButtonKey) -> bool {
        match key {
            DropDownButtonKey::Enter | DropDownButtonKey::Space => {
                self.toggle();
                true
            }
            DropDownButtonKey::ArrowDown => {
                if !self.open {
                    self.open = true;
                    true
                } else {
                    false
                }
            }
            DropDownButtonKey::Escape => {
                if self.open {
                    self.open = false;
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// 将键盘激活折叠进视觉状态。
#[must_use]
pub fn keyboard_activation(state: ControlState, space_or_enter_down: bool) -> ControlState {
    if space_or_enter_down && state.focused && !state.disabled {
        ControlState {
            phase: PointerPhase::Pressed,
            ..state
        }
    } else {
        state
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
    let mut a11y = A11yNode::new(Role::Button, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled)
        .expanded(open);
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

impl UiWidget for DropDownButtonDesc {
    type State = DropDownButtonState;

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, 14.0, 500);
        let w = (measured.x + 39.0).max(self.rect.width()).max(80.0);
        let h = DROP_DOWN_BUTTON_HEIGHT.max(self.rect.height());
        Vec2::new(w, h)
    }

    fn reduce(&self, state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { .. } => {
                state.toggle();
                true
            }
            WidgetInputEvent::KeyDown(chord) => {
                let key = match chord.key {
                    KeyName::Named("Enter") => Some(DropDownButtonKey::Enter),
                    KeyName::Named("Space") => Some(DropDownButtonKey::Space),
                    KeyName::Named("ArrowDown") => Some(DropDownButtonKey::ArrowDown),
                    KeyName::Named("Escape") => Some(DropDownButtonKey::Escape),
                    _ => None,
                };
                key.is_some_and(|k| state.handle_key(k))
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        // WinUI 为右侧 chevron 预留 40px 内容列（32px 图标列 + 8px 内边距），
        // 标签只在前 80px 内居中并被裁切；参考截图因此显示为 `DropDownB`。
        let content_r = Rect::new(
            rect.origin.x + 13.0,
            rect.origin.y,
            (rect.width() - 53.0).max(0.0),
            rect.height(),
        );
        let mut glyphs = text_ctx.shape_line(
            &desc.label,
            content_r,
            14.0,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        // WinUI 用一个独立的内容列裁掉超出部分；这里按字形起点过滤即可
        // 复现参考截图中 `DropDownB` 的截断外观。
        glyphs.retain(|quad| quad.rect.origin.x < content_r.max().x - 1.0);
        emit_drop_down_button(node, &desc, state, control, &mut glyphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> DropDownButtonDesc {
        DropDownButtonDesc::new(
            Rect::new(0.0, 0.0, 150.0, DROP_DOWN_BUTTON_HEIGHT),
            "Options",
        )
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
    fn keyboard_activation_collapses_to_pressed() {
        let focused = ControlState {
            focused: true,
            ..ControlState::default()
        };
        let active = keyboard_activation(focused, true);
        assert_eq!(active.phase, PointerPhase::Pressed);

        let inactive = keyboard_activation(focused, false);
        assert_eq!(inactive.phase, PointerPhase::Idle);
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
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }

    #[test]
    fn toggling_and_key_handling_drive_open_state() {
        let mut state = DropDownButtonState::new();
        assert!(!state.open);
        state.toggle();
        assert!(state.open);
        state.close();
        assert!(!state.open);
        state.open();
        assert!(state.open);

        // 键盘按键驱动
        assert!(state.handle_key(DropDownButtonKey::Escape));
        assert!(!state.open);
        assert!(state.handle_key(DropDownButtonKey::ArrowDown));
        assert!(state.open);
        assert!(state.handle_key(DropDownButtonKey::Space));
        assert!(!state.open);
        assert!(state.handle_key(DropDownButtonKey::Enter));
        assert!(state.open);
    }
}

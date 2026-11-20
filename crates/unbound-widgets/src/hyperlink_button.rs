//! HyperlinkButton：无边框、透明底的强调色文字链接按钮。
//!
//! 与普通按钮的区别只在配色：WinUI `HyperlinkButton` 不使用控件表面
//! `ControlFillColorDefault`，文字直接取强调色，悬停与按下仅叠加一层浅底。
//! 因此本控件没有 `ButtonVariant`，描述符与发射器都保持独立。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
use crate::primitive::{focus_ring, glyphs as emit_glyphs, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 链接按钮的标准控件高度。对齐 WinUI `HyperlinkButton` 的 32px。
pub const HYPERLINK_BUTTON_HEIGHT: f32 = 32.0;

/// 链接按钮的圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const HYPERLINK_RADIUS: f32 = 4.0;

/// 链接按钮描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct HyperlinkButtonDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签与显示文本。
    pub label: String,
    /// 目标链接地址（WinUI `NavigateUri`）。
    pub uri: Option<String>,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl HyperlinkButtonDesc {
    /// 使用默认度量创建链接按钮。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            uri: None,
            disabled: false,
            z: 705,
        }
    }

    /// 使用标准高度便捷构造。
    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self::new(Rect::new(0.0, 0.0, 120.0, HYPERLINK_BUTTON_HEIGHT), label)
    }

    /// 设置目标链接地址。
    #[must_use]
    pub fn with_uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
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

/// 将键盘激活（Enter / Space）折叠进视觉状态。
///
/// 与普通按钮一致，当控件聚焦且按住 Enter 或 Space 时，视觉表现等同于指针按下态。
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

/// 输出链接按钮的图元、命中区域与语义节点。
///
/// 无边框，底色只在悬停/按下时出现；图元数量与状态无关。
#[must_use]
pub fn emit_hyperlink_button(
    node: u64,
    desc: &HyperlinkButtonDesc,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;

    // HyperlinkButtonBackground：静息透明，悬停 `ControlFillColorSecondary`，
    // 按下 `ControlFillColorTertiary`，禁用透明。
    let fill = if disabled {
        Token::NONE
    } else {
        match control.phase {
            PointerPhase::Pressed => Token::SURFACE_SUNKEN,
            PointerPhase::Hovered => Token::CONTROL_HOVER,
            PointerPhase::Idle => Token::NONE,
        }
    };
    // HyperlinkButtonForeground：强调色文字，禁用取 `TextFillColorDisabled`。
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        match control.phase {
            PointerPhase::Pressed => Token::PRIMARY_PRESSED,
            PointerPhase::Hovered | PointerPhase::Idle => Token::PRIMARY,
        }
    };

    let mut primitives = Vec::with_capacity(3 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    // 槽位始终写入，仅令牌随状态变化，确保图元数量恒定。
    primitives.push(surface(rect, HYPERLINK_RADIUS, fill, z));
    emit_glyphs(glyphs, ink, z + 1, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        HYPERLINK_RADIUS,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [HYPERLINK_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });

    let mut a11y = A11yNode::new(Role::Link, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled);
    if let Some(ref uri) = desc.uri {
        a11y = a11y.with_value(uri);
    }

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

impl UiWidget for HyperlinkButtonDesc {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let measured = text_ctx.measure_line(&self.label, 14.0, 500);
        let w = (measured.x + 22.0).max(self.rect.width()).max(64.0);
        let h = HYPERLINK_BUTTON_HEIGHT.max(self.rect.height());
        Vec2::new(w, h)
    }

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        match event {
            WidgetInputEvent::PointerDown { .. } => true,
            WidgetInputEvent::KeyDown(chord) => {
                matches!(chord.key, KeyName::Named("Enter") | KeyName::Named("Space"))
            }
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let clip_right = rect.max().x - 12.0;
        let content_r = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y,
            (rect.width() - 24.0).max(0.0),
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
        glyphs.retain_mut(|quad| {
            if quad.rect.origin.x >= clip_right {
                false
            } else {
                if quad.rect.max().x > clip_right {
                    let w = quad.rect.width();
                    let new_w = clip_right - quad.rect.origin.x;
                    if w > 0.0 && new_w > 0.0 {
                        let fraction = new_w / w;
                        quad.uv[2] = quad.uv[0] + (quad.uv[2] - quad.uv[0]) * fraction;
                        quad.rect.size.x = new_w;
                    }
                }
                true
            }
        });
        emit_hyperlink_button(node, &desc, control, &mut glyphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> HyperlinkButtonDesc {
        HyperlinkButtonDesc::new(Rect::new(10.0, 10.0, 120.0, 24.0), "Learn more")
    }

    fn background(output: &WidgetOutput) -> u16 {
        output.primitives[0].fill_token_or_uv as u16
    }

    #[test]
    fn hover_and_press_change_the_background_slot() {
        let idle = emit_hyperlink_button(1, &desc(), ControlState::default(), &mut []);
        let hovered = emit_hyperlink_button(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        let pressed = emit_hyperlink_button(
            1,
            &desc(),
            ControlState {
                phase: PointerPhase::Pressed,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_eq!(background(&idle), Token::NONE.raw());
        assert_eq!(background(&hovered), Token::CONTROL_HOVER.raw());
        assert_eq!(background(&pressed), Token::SURFACE_SUNKEN.raw());
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
                            emit_hyperlink_button(1, &desc(), state, &mut [])
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
    fn a_disabled_link_is_not_clickable() {
        let output =
            emit_hyperlink_button(1, &desc().disabled(true), ControlState::default(), &mut []);
        assert!(output.hit.is_none());
        assert!(!output.focusable);
        assert_eq!(output.a11y.expect("reachable").role, Role::Link);
    }

    #[test]
    fn uri_is_propagated_to_a11y_node() {
        let d = desc().with_uri("https://example.com");
        let output = emit_hyperlink_button(1, &d, ControlState::default(), &mut []);
        let node = output.a11y.expect("reachable");
        assert_eq!(node.value, "https://example.com");
    }
}

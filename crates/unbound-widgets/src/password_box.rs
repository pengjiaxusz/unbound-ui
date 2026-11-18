//! PasswordBox：掩码显示的密码输入框。
//!
//! 视觉复用 [`crate::text_input`] 的 TextBox 规律：32px 高、4px 圆角、
//! 聚焦态底部 2px 强调线。右侧内嵌「眼睛」按钮，按住时显示明文。
//! 明文与掩码之间只差要塑形的字符串，因此由状态提供
//! [`PasswordBoxState::masked_text`] 与 [`PasswordBoxState::display_text`]。

use accesskit::Role;
use unbound_a11y::{A11yAction, A11yNode};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::text_input::{focus_underline, TEXT_INPUT_LAYER};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 掩码字符（U+2022 圆点）。以转义书写，避免在源码里引入非 ASCII 字面量。
pub const MASK_CHAR: char = '\u{2022}';
/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const PASSWORD_BOX_RADIUS: f32 = 4.0;
/// 标准高度。对齐 WinUI `TextControlThemeMinHeight = 32`。
pub const PASSWORD_BOX_HEIGHT: f32 = 32.0;
/// 默认字号。对齐 WinUI `ControlContentThemeFontSize = 14`。
pub const PASSWORD_BOX_FONT_SIZE: f32 = 14.0;
/// 内嵌眼睛按钮宽度。对齐 WinUI TextBox 模板的 30px 内嵌按钮。
pub const PASSWORD_BOX_INNER_BUTTON_WIDTH: f32 = 30.0;
/// 内嵌按钮外边距。对齐 WinUI `TextBoxInnerButtonMargin = 0,4,4,4`。
pub const PASSWORD_BOX_INNER_BUTTON_MARGIN: f32 = 4.0;

/// 密码框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PasswordBoxDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 空内容时显示的占位文本。
    pub placeholder: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
    /// 圆角半径。
    pub radius: f32,
    /// 字号（供调用方塑形时使用）。
    pub font_size: f32,
}

impl PasswordBoxDesc {
    /// 使用默认度量创建密码框。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            z: 709,
            radius: PASSWORD_BOX_RADIUS,
            font_size: PASSWORD_BOX_FONT_SIZE,
        }
    }

    /// 设置占位文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 右侧「显示明文」眼睛按钮矩形。
    #[must_use]
    pub fn reveal_rect(&self) -> Rect {
        let height =
            (self.rect.size.y - PASSWORD_BOX_INNER_BUTTON_MARGIN * 2.0).max(16.0);
        Rect::new(
            self.rect.max().x - PASSWORD_BOX_INNER_BUTTON_MARGIN - PASSWORD_BOX_INNER_BUTTON_WIDTH,
            self.rect.origin.y + PASSWORD_BOX_INNER_BUTTON_MARGIN,
            PASSWORD_BOX_INNER_BUTTON_WIDTH,
            height,
        )
    }

    /// 无障碍标签：为空时回退到占位文本。
    #[must_use]
    pub fn accessible_label(&self) -> String {
        if self.label.is_empty() {
            self.placeholder.clone()
        } else {
            self.label.clone()
        }
    }
}

/// 密码框状态：明文缓冲区与是否按住显示明文。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasswordBoxState {
    /// 明文缓冲区。
    pub text: String,
    /// 是否显示明文（按住眼睛按钮）。
    pub reveal: bool,
}

impl PasswordBoxState {
    /// 创建空密码框状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建预填明文的密码框状态。
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reveal: false,
        }
    }

    /// 掩码文本：每个字符替换为一个 `MASK_CHAR`。
    #[must_use]
    pub fn masked_text(&self) -> String {
        let count = self.text.chars().count();
        let mut masked = String::with_capacity(count * MASK_CHAR.len_utf8());
        for _ in 0..count {
            masked.push(MASK_CHAR);
        }
        masked
    }

    /// 调用方应实际塑形并绘制的文本。
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.reveal {
            self.text.clone()
        } else {
            self.masked_text()
        }
    }

    /// 切换明文显示。
    pub fn toggle_reveal(&mut self) {
        self.reveal = !self.reveal;
    }
}

/// 眼睛按钮的确定性子节点键。
#[must_use]
pub const fn reveal_button_node(node: u64) -> u64 {
    node ^ 0x0050_4245_5945
}

/// 输出密码框的图元、命中区域与语义节点。
#[must_use]
pub fn emit_password_box(
    node: u64,
    desc: &PasswordBoxDesc,
    state: &PasswordBoxState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let radii = [desc.radius; 4];
    let disabled = control.disabled || desc.disabled;
    let eye = desc.reveal_rect();

    // TextBox 的三态填充：`ControlFillColorDefault` / `Secondary` / `Disabled`。
    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
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
    } else if state.text.is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    let eye_fill = if disabled {
        Token::NONE
    } else if control.phase == PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::NONE
    };
    let eye_ink = if disabled {
        Token::TEXT_DISABLED
    } else if state.reveal {
        Token::PRIMARY
    } else {
        Token::TEXT_SECONDARY
    };

    let mut primitives = Vec::with_capacity(8 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

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
    // 聚焦态底部 2px 强调线；与 TextBox 使用同一实现。
    primitives.push(focus_underline(
        rect,
        desc.radius,
        z + 1,
        if control.focused && !disabled { 1.0 } else { 0.0 },
    ));

    // 眼睛按钮：悬停浅底 + 一个圆环与瞳孔。始终写入，确保图元数量恒定。
    primitives.push(surface(eye, 4.0, eye_fill, z + 1));
    let icon = 14.0;
    let ring = Rect::new(
        eye.center().x - icon * 0.5,
        eye.center().y - icon * 0.5,
        icon,
        icon,
    );
    primitives.push(outlined(
        ring,
        icon * 0.5,
        Token::NONE,
        eye_ink,
        1.5,
        0.0,
        z + 2,
    ));
    primitives.push(surface(
        Rect::new(
            eye.center().x - 2.0,
            eye.center().y - 2.0,
            4.0,
            4.0,
        ),
        2.0,
        eye_ink,
        z + 2,
    ));

    emit_glyphs(glyphs, ink, z + 2, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(node, TEXT_INPUT_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Text)
    });

    let mut a11y = A11yNode::new(Role::TextInput, desc.accessible_label(), rect)
        .clickable()
        .disabled(disabled);
    a11y.value = state.display_text();
    a11y.actions.insert(A11yAction::SetValue);
    a11y.actions.insert(A11yAction::Focus);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        let eye_key = reveal_button_node(node);
        let eye_label = if state.reveal {
            "Hide password"
        } else {
            "Show password"
        };
        out.with_sub_control(
            eye_key,
            HitRegion::new(eye_key, TEXT_INPUT_LAYER, u32::from(z) + 1, eye)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, eye_label, eye).clickable(),
            true,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> PasswordBoxDesc {
        PasswordBoxDesc::new(Rect::new(0.0, 0.0, 220.0, PASSWORD_BOX_HEIGHT))
            .with_placeholder("Password")
    }

    #[test]
    fn masked_text_replaces_every_scalar_with_the_mask_character() {
        let state = PasswordBoxState::with_text("hunter2");
        assert_eq!(state.masked_text(), MASK_CHAR.to_string().repeat(7));
        assert_eq!(state.display_text(), state.masked_text());
        assert_eq!(
            state.masked_text().chars().count(),
            state.text.chars().count()
        );
    }

    #[test]
    fn revealing_shows_the_plain_buffer_and_toggles_back() {
        let mut state = PasswordBoxState::with_text("s3cret");
        assert_eq!(state.display_text(), MASK_CHAR.to_string().repeat(6));
        state.toggle_reveal();
        assert_eq!(state.display_text(), "s3cret");
        state.toggle_reveal();
        assert_eq!(state.display_text(), MASK_CHAR.to_string().repeat(6));
    }

    #[test]
    fn the_eye_button_is_a_separate_sub_control() {
        let out = emit_password_box(
            7,
            &desc(),
            &PasswordBoxState::with_text("abc"),
            ControlState::default(),
            &mut [],
        );
        assert_eq!(out.sub_hits.len(), 1);
        assert_eq!(out.sub_hits[0].node, reveal_button_node(7));
        assert!(out.sub_hits[0].bounds.max().x <= desc().rect.max().x);
        assert_eq!(out.sub_a11y.len(), 1);
        assert_eq!(out.sub_a11y[0].1.role, Role::Button);
        assert!(out.sub_focus_order.contains(&reveal_button_node(7)));
        let node = out.a11y.expect("reachable");
        assert_eq!(node.role, Role::TextInput);
        assert_eq!(node.value, MASK_CHAR.to_string().repeat(3));
    }

    #[test]
    fn hover_changes_the_field() {
        let idle = emit_password_box(
            1,
            &desc(),
            &PasswordBoxState::with_text("abc"),
            ControlState::default(),
            &mut [],
        );
        let hovered = emit_password_box(
            1,
            &desc(),
            &PasswordBoxState::with_text("abc"),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_ne!(idle.primitives, hovered.primitives);
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
                            emit_password_box(
                                1,
                                &desc(),
                                &PasswordBoxState::with_text("abc"),
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
}

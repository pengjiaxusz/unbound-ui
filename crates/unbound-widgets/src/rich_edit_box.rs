//! RichEditBox：带格式工具条的可编辑富文本输入框。
//!
//! 控件由三段组成：顶部格式工具条（B / I / U）、细分隔线与下方可编辑文本区。
//! 三个格式按钮各自上报独立命中区域，但图元数量与交互状态无关——悬停、聚焦与
//! 禁用只切换令牌槽位，绝不增删图元。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 格式工具条高度。
pub const RICH_EDIT_TOOLBAR_HEIGHT: f32 = 28.0;
/// 格式按钮边长。
pub const RICH_EDIT_FORMAT_BUTTON: f32 = 20.0;
/// 控件圆角。对齐 WinUI `ControlCornerRadius = 4`。
pub const RICH_EDIT_RADIUS: f32 = 4.0;

/// 富文本编辑框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RichEditBoxDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 加粗标记是否激活。
    pub bold: bool,
    /// 斜体标记是否激活。
    pub italic: bool,
    /// 下划线标记是否激活。
    pub underline: bool,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl RichEditBoxDesc {
    /// 创建格式标记全部关闭的富文本编辑框。
    #[must_use]
    pub fn new(rect: Rect, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            bold: false,
            italic: false,
            underline: false,
            radius: RICH_EDIT_RADIUS,
            disabled: false,
            z: 720,
        }
    }

    /// 设置三个格式标记的激活状态。
    #[must_use]
    pub fn with_marks(mut self, bold: bool, italic: bool, underline: bool) -> Self {
        self.bold = bold;
        self.italic = italic;
        self.underline = underline;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 工具条区域矩形。
    #[must_use]
    pub fn toolbar_rect(&self) -> Rect {
        let height = RICH_EDIT_TOOLBAR_HEIGHT.min(self.rect.height() * 0.5);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.width(),
            height.max(0.0),
        )
    }

    /// 第 `index` 个格式按钮（0=B、1=I、2=U）的矩形。
    #[must_use]
    pub fn format_button_rect(&self, index: usize) -> Rect {
        let toolbar = self.toolbar_rect();
        let size = RICH_EDIT_FORMAT_BUTTON
            .min((toolbar.height() - 4.0).max(8.0))
            .max(8.0);
        let x = toolbar.origin.x + 6.0 + index as f32 * (size + 4.0);
        Rect::new(
            x,
            toolbar.origin.y + (toolbar.height() - size) * 0.5,
            size,
            size,
        )
    }

    /// 文本编辑区矩形。
    #[must_use]
    pub fn editor_rect(&self) -> Rect {
        let toolbar = self.toolbar_rect();
        Rect::new(
            self.rect.origin.x,
            toolbar.max().y,
            self.rect.width(),
            (self.rect.height() - toolbar.height()).max(0.0),
        )
    }
}

/// 输出富文本编辑框的图元、命中区域与语义节点。
///
/// 图元顺序固定：外框、工具条、分隔线、三个格式按钮、编辑区、焦点环。
#[must_use]
pub fn emit_rich_edit_box(
    node: u64,
    desc: &RichEditBoxDesc,
    control: ControlState,
    text_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;
    let pressed = !disabled && control.phase == PointerPhase::Pressed;

    let bg = if disabled {
        Token::CONTROL_DISABLED_BG
    } else {
        Token::INPUT_BG
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if control.focused {
        Token::FOCUS_RING
    } else if hovered {
        Token::INPUT_BORDER_HOVER
    } else {
        Token::CONTROL_STROKE
    };
    let toolbar_fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if pressed {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_SUNKEN
    };

    let toolbar = desc.toolbar_rect();
    let editor = desc.editor_rect();

    let mut primitives = Vec::with_capacity(8 + text_glyphs.len());
    let mut payloads = Vec::with_capacity(text_glyphs.len());

    primitives.push(outlined(rect, desc.radius, bg, border, 1.0, 0.0, z));
    primitives.push(surface(toolbar, 0.0, toolbar_fill, z + 1));
    // 工具条与编辑区之间的细分隔线。
    primitives.push(surface(
        Rect::new(
            toolbar.origin.x,
            toolbar.max().y - 1.0,
            toolbar.width(),
            1.0,
        ),
        0.0,
        Token::DIVIDER,
        z + 2,
    ));

    let marks = [desc.bold, desc.italic, desc.underline];
    for (index, active) in marks.into_iter().enumerate() {
        let button = desc.format_button_rect(index);
        let fill = if disabled {
            Token::CONTROL_DISABLED_BG
        } else if active {
            Token::PRIMARY_SOFT
        } else if hovered {
            Token::CONTROL_HOVER
        } else {
            Token::CONTROL_FILL
        };
        primitives.push(outlined(button, 3.0, fill, Token::NONE, 0.0, 0.0, z + 3));
    }

    primitives.push(surface(editor, 0.0, Token::SURFACE, z + 4));
    emit_glyphs(
        text_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 5,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 6,
        control.focused && !disabled,
    ));

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, BUTTON_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Text)
        }),
        a11y: Some(
            A11yNode::new(Role::TextInput, desc.label.clone(), rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        for (index, name) in ["Bold", "Italic", "Underline"].into_iter().enumerate() {
            let key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x500 + index as u64);
            let button = desc.format_button_rect(index);
            out.with_sub_control(
                key,
                HitRegion::new(key, BUTTON_LAYER, u32::from(z) + 3, button)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, name, button).clickable(),
                true,
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> RichEditBoxDesc {
        RichEditBoxDesc::new(Rect::new(0.0, 0.0, 260.0, 120.0), "Document")
    }

    #[test]
    fn the_toolbar_sits_above_the_editor() {
        let d = desc();
        let toolbar = d.toolbar_rect();
        let editor = d.editor_rect();
        assert_eq!(toolbar.origin.y, d.rect.origin.y);
        assert!((toolbar.max().y - editor.origin.y).abs() < 1e-4);
        assert!((toolbar.height() + editor.height() - d.rect.height()).abs() < 1e-4);
    }

    #[test]
    fn format_buttons_stay_inside_the_toolbar() {
        let d = desc();
        for index in 0..3 {
            let button = d.format_button_rect(index);
            assert!(
                d.toolbar_rect().contains_rect(&button),
                "{index}: {button:?}"
            );
            assert!(button.size.x > 0.0 && button.size.y > 0.0);
        }
    }

    #[test]
    fn hovering_switches_the_border_token() {
        let d = desc();
        let idle = emit_rich_edit_box(1, &d, ControlState::default(), &mut []);
        let hovered = emit_rich_edit_box(
            1,
            &d,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_eq!(
            idle.primitives[0].border_token_or_tex as u16,
            Token::CONTROL_STROKE.raw()
        );
        assert_eq!(
            hovered.primitives[0].border_token_or_tex as u16,
            Token::INPUT_BORDER_HOVER.raw()
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
                        counts.push(emit_rich_edit_box(1, &d, state, &mut []).primitives.len());
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
    fn a_disabled_editor_refuses_the_pointer() {
        let output =
            emit_rich_edit_box(1, &desc().disabled(true), ControlState::default(), &mut []);
        assert!(output.hit.is_none());
        assert!(!output.focusable);
        assert_eq!(output.a11y.expect("reachable").role, Role::TextInput);
    }
}

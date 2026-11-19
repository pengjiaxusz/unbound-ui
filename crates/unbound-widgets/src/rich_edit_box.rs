//! RichEditBox：对齐 WinUI 3 官方标准的原生富文本编辑框。
//!
//! 依据 ADR 0016 与 Spec #79，RichEditBox 还原为 WinUI 3 官方纯净形态（不内置非标格式工具栏）：
//! 纯净编辑表面、1px 边框与标准内边距（TextControlThemePadding），支持悬停、聚焦与禁用状态切换。
//! 图元数量严格保持常数，悬停/聚焦/禁用只切换令牌槽位，绝不增删图元。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 控件圆角。对齐 WinUI `ControlCornerRadius = 4`。
pub const RICH_EDIT_RADIUS: f32 = 4.0;

/// 水平内边距。对齐 WinUI `TextControlThemePadding` 左侧 10px。
pub const RICH_EDIT_PADDING_X: f32 = 10.0;

/// 垂直内边距。对齐 WinUI `TextControlThemePadding` 顶部 6px。
pub const RICH_EDIT_PADDING_Y: f32 = 6.0;

/// 富文本编辑框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RichEditBoxDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 加粗标记是否激活（保留供富文本格式状态绑定）。
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
    /// 创建格式标记全部关闭的纯净富文本编辑框。
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

    /// 设置格式标记的激活状态。
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

    /// 设置圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 设置绘制顺序键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 文本编辑区矩形（内边距：左 10px、上 6px、右 10px、下 6px）。
    #[must_use]
    pub fn editor_rect(&self) -> Rect {
        let pad_x = RICH_EDIT_PADDING_X;
        let pad_y = RICH_EDIT_PADDING_Y;
        Rect::new(
            self.rect.origin.x + pad_x,
            self.rect.origin.y + pad_y,
            (self.rect.width() - pad_x * 2.0).max(0.0),
            (self.rect.height() - pad_y * 2.0).max(0.0),
        )
    }

    /// 固有尺寸：WinUI 3 标准宽度 240px、高度 96px（或当前矩形尺寸）。
    #[must_use]
    pub fn intrinsic_size(&self) -> Vec2 {
        let w = if self.rect.width() > 0.0 {
            self.rect.width()
        } else {
            240.0
        };
        let h = if self.rect.height() > 0.0 {
            self.rect.height()
        } else {
            96.0
        };
        Vec2::new(w, h)
    }
}

/// 输出纯净富文本编辑框的图元、命中区域与语义节点。
///
/// 图元顺序固定：外框（背景+描边）、文本字形、焦点环。
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

    let mut primitives = Vec::with_capacity(2 + text_glyphs.len());
    let mut payloads = Vec::with_capacity(text_glyphs.len());

    // 1. 外框与背景
    primitives.push(outlined(rect, desc.radius, bg, border, 1.0, 0.0, z));

    // 2. 文本字形
    emit_glyphs(
        text_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 2,
        &mut primitives,
        &mut payloads,
    );

    // 3. 焦点环
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 3,
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
                .with_cursor(CursorKind::Text)
        }),
        a11y: Some(
            A11yNode::new(Role::TextInput, desc.label.clone(), rect)
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

    fn desc() -> RichEditBoxDesc {
        RichEditBoxDesc::new(Rect::new(0.0, 0.0, 240.0, 96.0), "Document")
    }

    #[test]
    fn editor_rect_applies_standard_padding() {
        let d = desc();
        let editor = d.editor_rect();
        assert_eq!(editor.origin.x, 10.0);
        assert_eq!(editor.origin.y, 6.0);
        assert_eq!(editor.width(), 220.0);
        assert_eq!(editor.height(), 84.0);
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
        assert_eq!(
            counts[0], 2,
            "2 primitives: outlined surface and focus ring"
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

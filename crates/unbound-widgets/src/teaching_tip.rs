//! TeachingTip：带 Tail 尖角的引导气泡。
//!
//! 完整形态包含：圆角卡片、指向目标的小尖角、标题与正文、主 / 次操作按钮与右上角
//! 关闭按钮。所有部件始终输出，关闭与禁用只通过令牌与命中区域表达，不改变图元数量。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::button::BUTTON_LAYER;
use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 卡片圆角。对齐 WinUI `OverlayCornerRadius = 8`（TeachingTip 的 CornerRadius）。
pub const TEACHING_TIP_RADIUS: f32 = 8.0;
/// 尖角尺寸。
pub const TEACHING_TIP_TAIL: f32 = 10.0;
/// 操作按钮高度。
pub const TEACHING_TIP_BUTTON_HEIGHT: f32 = 28.0;
/// 关闭按钮边长。
pub const TEACHING_TIP_CLOSE: f32 = 16.0;
/// 引导气泡阴影模糊半径。对齐 WinUI 3 `FlyoutElevation`（24px 解析阴影）。
pub const TEACHING_TIP_SHADOW: f32 = 24.0;
/// 引导气泡外描边厚度。对齐 WinUI 3 `SurfaceStroke`（1px 微描边）。
pub const TEACHING_TIP_BORDER_WIDTH: f32 = 1.0;

/// 尖角所在的边。
///
/// 尖角表示气泡指向目标的方向，因此 `Top` 意味着尖角位于卡片上沿、指向更上方。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TeachingTipTail {
    /// 尖角居中于卡片上沿。
    #[default]
    Top,
    /// 尖角居中于卡片下沿。
    Bottom,
    /// 尖角居中于卡片左沿。
    Left,
    /// 尖角居中于卡片右沿。
    Right,
}

/// 引导气泡描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TeachingTipDesc {
    /// 卡片矩形（不含尖角）。
    pub rect: Rect,
    /// 尖角所在的边。
    pub tail: TeachingTipTail,
    /// 标题文本。
    pub title: String,
    /// 正文文本。
    pub body: String,
    /// 主操作按钮文本。
    pub primary_label: String,
    /// 次操作按钮文本。
    pub secondary_label: Option<String>,
    /// 是否显示关闭按钮。
    pub closable: bool,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl TeachingTipDesc {
    /// 创建带标题与正文的引导气泡。
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            rect,
            tail: TeachingTipTail::Top,
            title: title.into(),
            body: body.into(),
            primary_label: "Got it".to_owned(),
            secondary_label: None,
            closable: true,
            radius: TEACHING_TIP_RADIUS,
            disabled: false,
            z: 780,
        }
    }

    /// 指定尖角方向。
    #[must_use]
    pub fn tail(mut self, tail: TeachingTipTail) -> Self {
        self.tail = tail;
        self
    }

    /// 设置主 / 次操作按钮文本。
    #[must_use]
    pub fn with_actions(
        mut self,
        primary: impl Into<String>,
        secondary: impl Into<String>,
    ) -> Self {
        self.primary_label = primary.into();
        self.secondary_label = Some(secondary.into());
        self
    }

    /// 隐藏关闭按钮。
    #[must_use]
    pub fn without_close(mut self) -> Self {
        self.closable = false;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 尖角矩形。
    #[must_use]
    pub fn tail_rect(&self) -> Rect {
        let half = TEACHING_TIP_TAIL * 0.5;
        match self.tail {
            TeachingTipTail::Top => Rect::new(
                self.rect.center().x - half,
                self.rect.origin.y - half.max(0.0),
                TEACHING_TIP_TAIL,
                half,
            ),
            TeachingTipTail::Bottom => Rect::new(
                self.rect.center().x - half,
                self.rect.max().y,
                TEACHING_TIP_TAIL,
                half,
            ),
            TeachingTipTail::Left => Rect::new(
                self.rect.origin.x - half,
                self.rect.center().y - half,
                half,
                TEACHING_TIP_TAIL,
            ),
            TeachingTipTail::Right => Rect::new(
                self.rect.max().x,
                self.rect.center().y - half,
                half,
                TEACHING_TIP_TAIL,
            ),
        }
    }

    /// 关闭按钮矩形。
    #[must_use]
    pub fn close_rect(&self) -> Rect {
        Rect::new(
            self.rect.max().x - TEACHING_TIP_CLOSE - 8.0,
            self.rect.origin.y + 8.0,
            TEACHING_TIP_CLOSE,
            TEACHING_TIP_CLOSE,
        )
    }

    /// 主操作按钮矩形。
    #[must_use]
    pub fn primary_rect(&self) -> Rect {
        let width = 84.0_f32.min(self.rect.width() * 0.5);
        Rect::new(
            self.rect.max().x - width - 12.0,
            self.rect.max().y - TEACHING_TIP_BUTTON_HEIGHT - 12.0,
            width.max(0.0),
            TEACHING_TIP_BUTTON_HEIGHT,
        )
    }

    /// 次操作按钮矩形；没有次操作时返回零尺寸矩形。
    #[must_use]
    pub fn secondary_rect(&self) -> Rect {
        let primary = self.primary_rect();
        let width = 76.0_f32.min(self.rect.width() * 0.5);
        Rect::new(
            primary.origin.x - width - 8.0,
            primary.origin.y,
            if self.secondary_label.is_some() {
                width.max(0.0)
            } else {
                0.0
            },
            TEACHING_TIP_BUTTON_HEIGHT,
        )
    }
}

/// 输出引导气泡的图元、命中区域与语义节点。
#[must_use]
pub fn emit_teaching_tip(
    node: u64,
    desc: &TeachingTipDesc,
    control: ControlState,
    text_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let card_border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if hovered {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };

    let mut primitives = Vec::with_capacity(8 + text_glyphs.len());
    let mut payloads = Vec::with_capacity(text_glyphs.len());

    // 1. 引导气泡解析阴影（Elevation 阴影：24px）
    primitives.push(instance(
        rect,
        [desc.radius; 4],
        Token::NONE,
        Token::NONE,
        0.0,
        TEACHING_TIP_SHADOW,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));
    // 2. 气泡卡片表面与 1px 微描边（SurfaceStroke）
    primitives.push(outlined(
        rect,
        desc.radius,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::OVERLAY_BG
        },
        card_border,
        TEACHING_TIP_BORDER_WIDTH,
        0.0,
        z,
    ));
    primitives.push(surface(desc.tail_rect(), 2.0, Token::OVERLAY_BG, z + 1));
    primitives.push(outlined(
        desc.primary_rect(),
        4.0,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::PRIMARY
        },
        Token::NONE,
        0.0,
        0.0,
        z + 2,
    ));
    primitives.push(outlined(
        desc.secondary_rect(),
        4.0,
        Token::NONE,
        if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::CONTROL_STROKE
        },
        if desc.secondary_label.is_some() {
            1.0
        } else {
            0.0
        },
        0.0,
        z + 3,
    ));
    primitives.push(outlined(
        desc.close_rect(),
        3.0,
        Token::NONE,
        Token::NONE,
        0.0,
        0.0,
        z + 4,
    ));
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
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Group, desc.title.clone(), rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        let primary_key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x800;
        out.with_sub_control(
            primary_key,
            HitRegion::new(
                primary_key,
                BUTTON_LAYER,
                u32::from(z) + 2,
                desc.primary_rect(),
            )
            .with_cursor(CursorKind::Pointer),
            A11yNode::new(
                Role::Button,
                desc.primary_label.clone(),
                desc.primary_rect(),
            )
            .clickable(),
            true,
        );
        if let Some(secondary) = &desc.secondary_label {
            let key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x801;
            out.with_sub_control(
                key,
                HitRegion::new(key, BUTTON_LAYER, u32::from(z) + 3, desc.secondary_rect())
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, secondary.clone(), desc.secondary_rect()).clickable(),
                true,
            );
        }
        if desc.closable {
            let key = node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x802;
            out.with_sub_control(
                key,
                HitRegion::new(key, BUTTON_LAYER, u32::from(z) + 4, desc.close_rect())
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, "Close", desc.close_rect()).clickable(),
                true,
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TeachingTipDesc {
        TeachingTipDesc::new(
            Rect::new(40.0, 40.0, 300.0, 160.0),
            "New feature",
            "Frame pacing now adapts to display refresh.",
        )
        .with_actions("Try it", "Later")
    }

    #[test]
    fn the_tail_sticks_out_of_the_card() {
        let d = desc();
        let tail = d.tail_rect();
        assert!(tail.max().y <= d.rect.origin.y);
        assert!((tail.center().x - d.rect.center().x).abs() < 1e-4);
    }

    #[test]
    fn the_buttons_sit_inside_the_card() {
        let d = desc();
        assert!(d.rect.contains_rect(&d.primary_rect()));
        assert!(d.rect.contains_rect(&d.secondary_rect()));
        assert!(d.secondary_rect().max().x <= d.primary_rect().origin.x);
    }

    #[test]
    fn a_tip_without_secondary_has_no_secondary_hit() {
        let d = TeachingTipDesc::new(Rect::new(0.0, 0.0, 300.0, 160.0), "Title", "Body");
        let output = emit_teaching_tip(1, &d, ControlState::default(), &mut []);
        // 主按钮与关闭按钮各一个子命中区域。
        assert_eq!(output.sub_hits.len(), 2);
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
                        counts.push(emit_teaching_tip(1, &d, state, &mut []).primitives.len());
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

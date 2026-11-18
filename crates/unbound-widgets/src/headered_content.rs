//! HeaderedContentControl：统一的「上方标题 + 下方内容」包装器。
//!
//! WinUI 的 `HeaderedContentControl` 把标题排版抽成公共能力，供 `ToggleSwitch`、
//! `Slider`、`TextBox` 等控件复用。本模块只负责容器外观：填充分隔线把标题条
//! 与内容区分开，内容本身由调用方绘制。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 标题条默认高度。
pub const HEADERED_HEADER_HEIGHT: f32 = 28.0;
/// 容器圆角。
pub const HEADERED_RADIUS: f32 = 6.0;

/// 带标题的内容包装器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderedContentDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 标题文本。
    pub header: String,
    /// 标题条高度。
    pub header_height: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl HeaderedContentDesc {
    /// 创建带标题的容器。
    #[must_use]
    pub fn new(rect: Rect, header: impl Into<String>) -> Self {
        Self {
            rect,
            header: header.into(),
            header_height: HEADERED_HEADER_HEIGHT,
            radius: HEADERED_RADIUS,
            disabled: false,
            z: 726,
        }
    }

    /// 设置标题条高度。
    #[must_use]
    pub fn with_header_height(mut self, height: f32) -> Self {
        self.header_height = height.max(0.0);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 标题条矩形。
    #[must_use]
    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.width(),
            self.header_height.min(self.rect.height()),
        )
    }

    /// 内容区矩形。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        let header = self.header_rect();
        Rect::new(
            self.rect.origin.x,
            header.max().y,
            self.rect.width(),
            (self.rect.height() - header.height()).max(0.0),
        )
    }
}

/// 输出带标题容器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_headered_content(
    node: u64,
    desc: &HeaderedContentDesc,
    control: ControlState,
    header_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(4 + header_glyphs.len());
    let mut payloads = Vec::with_capacity(header_glyphs.len());

    primitives.push(outlined(
        rect,
        desc.radius,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if hovered {
            Token::INPUT_BORDER_HOVER
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        z,
    ));
    let header = desc.header_rect();
    primitives.push(surface(
        header,
        desc.radius,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::SURFACE_SUNKEN
        },
        z + 1,
    ));
    primitives.push(surface(
        Rect::new(header.origin.x, header.max().y - 1.0, header.width(), 1.0),
        0.0,
        Token::DIVIDER,
        z + 2,
    ));
    emit_glyphs(
        header_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_SECONDARY
        },
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Group, desc.header.clone(), rect)
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

    fn desc() -> HeaderedContentDesc {
        HeaderedContentDesc::new(Rect::new(0.0, 0.0, 240.0, 160.0), "Appearance")
    }

    #[test]
    fn the_content_starts_after_the_header() {
        let d = desc();
        assert_eq!(d.header_rect().height(), HEADERED_HEADER_HEIGHT);
        assert_eq!(d.content_rect().origin.y, d.header_rect().max().y);
        assert_eq!(
            d.header_rect().height() + d.content_rect().height(),
            d.rect.height()
        );
    }

    #[test]
    fn hovering_changes_the_container_border() {
        let d = desc();
        let idle = emit_headered_content(1, &d, ControlState::default(), &mut []);
        let hovered = emit_headered_content(
            1,
            &d,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_ne!(
            idle.primitives[0].border_token_or_tex,
            hovered.primitives[0].border_token_or_tex
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
                            emit_headered_content(1, &d, state, &mut [])
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

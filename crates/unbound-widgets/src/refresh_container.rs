//! RefreshContainer：下拉刷新容器。
//!
//! 顶部指示器始终存在：下拉进度控制其直径与填充，刷新中则取强调色。
//! 图元数量固定为「内容外框、指示器轨道、指示器填充、内容分隔线、焦点环」五项。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 指示器最大直径。
pub const REFRESH_INDICATOR_SIZE: f32 = 24.0;
/// 内容圆角。
pub const REFRESH_RADIUS: f32 = 8.0;

/// 下拉刷新容器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RefreshContainerDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 下拉进度 `0.0..=1.0`。
    pub pull: f32,
    /// 是否正在刷新。
    pub refreshing: bool,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl RefreshContainerDesc {
    /// 创建未下拉的刷新容器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            pull: 0.0,
            refreshing: false,
            radius: REFRESH_RADIUS,
            disabled: false,
            z: 708,
        }
    }

    /// 设置下拉进度，自动夹紧到 `0.0..=1.0`。
    #[must_use]
    pub fn pull(mut self, pull: f32) -> Self {
        self.pull = pull.clamp(0.0, 1.0);
        self
    }

    /// 设置刷新中状态。
    #[must_use]
    pub fn refreshing(mut self, refreshing: bool) -> Self {
        self.refreshing = refreshing;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 指示器直径：随下拉进度放大，刷新中固定为最大直径。
    #[must_use]
    pub fn indicator_size(&self) -> f32 {
        if self.refreshing {
            REFRESH_INDICATOR_SIZE
        } else {
            (REFRESH_INDICATOR_SIZE * self.pull.clamp(0.0, 1.0)).max(0.0)
        }
    }

    /// 指示器矩形，水平居中于容器顶部。
    #[must_use]
    pub fn indicator_rect(&self) -> Rect {
        let size = self.indicator_size();
        Rect::new(
            self.rect.origin.x + (self.rect.width() - size) * 0.5,
            self.rect.origin.y + 6.0,
            size,
            size,
        )
    }

    /// 内容区矩形。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + REFRESH_INDICATOR_SIZE + 12.0,
            self.rect.width(),
            (self.rect.height() - REFRESH_INDICATOR_SIZE - 12.0).max(0.0),
        )
    }
}

/// 输出下拉刷新容器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_refresh_container(
    node: u64,
    desc: &RefreshContainerDesc,
    control: ControlState,
    content_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(6 + content_glyphs.len());
    let mut payloads = Vec::with_capacity(content_glyphs.len());

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

    let track = Rect::new(
        desc.rect.origin.x + (desc.rect.width() - REFRESH_INDICATOR_SIZE) * 0.5,
        desc.rect.origin.y + 6.0,
        REFRESH_INDICATOR_SIZE,
        REFRESH_INDICATOR_SIZE,
    );
    primitives.push(surface(
        track,
        REFRESH_INDICATOR_SIZE * 0.5,
        Token::SURFACE_SUNKEN,
        z + 1,
    ));
    let indicator = desc.indicator_rect();
    primitives.push(surface(
        indicator,
        indicator.size.x * 0.5,
        if disabled {
            Token::CONTROL_DISABLED_THUMB
        } else if desc.refreshing || desc.pull >= 1.0 {
            Token::PRIMARY
        } else {
            Token::CONTROL_THUMB
        },
        z + 2,
    ));
    primitives.push(surface(
        Rect::new(
            desc.rect.origin.x + 8.0,
            desc.content_rect().origin.y - 6.0,
            (desc.rect.width() - 16.0).max(0.0),
            1.0,
        ),
        0.0,
        Token::DIVIDER,
        z + 3,
    ));
    emit_glyphs(
        content_glyphs,
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
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Group, "Pull to refresh", rect)
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

    fn desc() -> RefreshContainerDesc {
        RefreshContainerDesc::new(Rect::new(0.0, 0.0, 280.0, 180.0))
    }

    #[test]
    fn the_indicator_grows_with_the_pull() {
        let closed = desc();
        let half = desc().pull(0.5);
        let full = desc().pull(2.0);
        assert_eq!(closed.indicator_size(), 0.0);
        assert_eq!(half.indicator_size(), REFRESH_INDICATOR_SIZE * 0.5);
        assert_eq!(full.indicator_size(), REFRESH_INDICATOR_SIZE);
    }

    #[test]
    fn refreshing_forces_the_full_indicator() {
        let d = desc().refreshing(true);
        assert_eq!(d.indicator_size(), REFRESH_INDICATOR_SIZE);
    }

    #[test]
    fn the_content_sits_below_the_indicator() {
        let d = desc();
        assert!(d.content_rect().origin.y >= d.indicator_rect().max().y);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().pull(0.7);
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
                            emit_refresh_container(1, &d, state, &mut [])
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

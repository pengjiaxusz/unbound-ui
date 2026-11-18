//! SwipeControl：列表项左右滑动露出操作按钮的容器。
//!
//! 内容行覆盖在左右两个操作槽之上；槽位始终输出，仅以令牌区分「已露出 / 未露出」。
//! 因此滑动过程中图元数量恒定，渲染器只需修补填充色。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 滑动操作槽宽度上限。
pub const SWIPE_ACTION_WIDTH: f32 = 96.0;
/// 内容行圆角。
pub const SWIPE_RADIUS: f32 = 8.0;

/// 当前露出的操作方向。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwipeSide {
    /// 左右操作槽都未露出。
    #[default]
    Closed,
    /// 左侧操作槽露出。
    Left,
    /// 右侧操作槽露出。
    Right,
}

/// 滑动容器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SwipeControlDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 左侧操作标签；`None` 表示没有左侧操作。
    pub left_label: Option<String>,
    /// 右侧操作标签；`None` 表示没有右侧操作。
    pub right_label: Option<String>,
    /// 当前露出的方向。
    pub side: SwipeSide,
    /// 圆角半径。
    pub radius: f32,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl SwipeControlDesc {
    /// 创建左右各有一个操作槽的滑动容器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            left_label: Some("Delete".to_owned()),
            right_label: Some("Archive".to_owned()),
            side: SwipeSide::Closed,
            radius: SWIPE_RADIUS,
            disabled: false,
            z: 712,
        }
    }

    /// 只保留左侧操作。
    #[must_use]
    pub fn with_left(mut self, label: impl Into<String>) -> Self {
        self.left_label = Some(label.into());
        self
    }

    /// 只保留右侧操作。
    #[must_use]
    pub fn with_right(mut self, label: impl Into<String>) -> Self {
        self.right_label = Some(label.into());
        self
    }

    /// 设置当前露出的方向。
    #[must_use]
    pub fn side(mut self, side: SwipeSide) -> Self {
        self.side = side;
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 左侧操作槽矩形。
    #[must_use]
    pub fn left_rect(&self) -> Rect {
        let width = SWIPE_ACTION_WIDTH.min(self.rect.width() * 0.5);
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            width.max(0.0),
            self.rect.height(),
        )
    }

    /// 右侧操作槽矩形。
    #[must_use]
    pub fn right_rect(&self) -> Rect {
        let width = SWIPE_ACTION_WIDTH.min(self.rect.width() * 0.5);
        Rect::new(
            self.rect.max().x - width.max(0.0),
            self.rect.origin.y,
            width.max(0.0),
            self.rect.height(),
        )
    }

    /// 内容行矩形：按露出方向向反方向平移半个槽位。
    #[must_use]
    pub fn content_rect(&self) -> Rect {
        let offset = match self.side {
            SwipeSide::Closed => 0.0,
            SwipeSide::Left => -self.left_rect().width(),
            SwipeSide::Right => self.right_rect().width(),
        };
        Rect::new(
            self.rect.origin.x + offset,
            self.rect.origin.y,
            self.rect.width(),
            self.rect.height(),
        )
    }
}

/// 输出滑动容器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_swipe_control(
    node: u64,
    desc: &SwipeControlDesc,
    control: ControlState,
    content_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(4 + content_glyphs.len());
    let mut payloads = Vec::with_capacity(content_glyphs.len());

    let action_fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if hovered {
        Token::CONTROL_HOVER
    } else {
        Token::SURFACE_SUNKEN
    };

    primitives.push(surface(desc.left_rect(), 0.0, action_fill, z));
    primitives.push(surface(desc.right_rect(), 0.0, action_fill, z));
    primitives.push(outlined(
        desc.content_rect(),
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
        2.0,
        z + 1,
    ));
    emit_glyphs(
        content_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 2,
        &mut primitives,
        &mut payloads,
    );
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
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [desc.radius; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Group, "Swipe control", rect)
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

    fn desc() -> SwipeControlDesc {
        SwipeControlDesc::new(Rect::new(0.0, 0.0, 320.0, 48.0))
    }

    #[test]
    fn action_slots_sit_on_both_edges() {
        let d = desc();
        assert_eq!(d.left_rect().origin.x, d.rect.origin.x);
        assert_eq!(d.right_rect().max().x, d.rect.max().x);
        assert!(d.left_rect().max().x <= d.right_rect().origin.x);
    }

    #[test]
    fn revealing_left_shifts_the_content_left() {
        let d = desc().side(SwipeSide::Left);
        assert_eq!(
            d.content_rect().origin.x,
            d.rect.origin.x - d.left_rect().width()
        );
    }

    #[test]
    fn hovering_changes_the_content_border() {
        let d = desc();
        let idle = emit_swipe_control(1, &d, ControlState::default(), &mut []);
        let hovered = emit_swipe_control(
            1,
            &d,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_ne!(
            idle.primitives[2].border_token_or_tex,
            hovered.primitives[2].border_token_or_tex
        );
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().side(SwipeSide::Right);
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
                        counts.push(emit_swipe_control(1, &d, state, &mut []).primitives.len());
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

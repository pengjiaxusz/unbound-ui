//! RadialGauge：环形刻度仪表。
//!
//! 表盘由背景圆、固定数量的刻度胶囊、指针、中心轴与焦点环组成。
//! 指针角度只随数值变化；悬停与禁用只切换背景与指针令牌。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, focus_ring, glyphs as emit_glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 刻度数量。
pub const RADIAL_GAUGE_SEGMENTS: usize = 12;
/// 中心轴半径。
pub const RADIAL_GAUGE_HUB: f32 = 6.0;
/// 指针长度占半径的比例。
pub const RADIAL_GAUGE_NEEDLE: f32 = 0.72;

/// 环形仪表描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGaugeDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 归一化数值 `0.0..=1.0`。
    pub value_norm: f32,
    /// 刻度数量。
    pub segments: usize,
    /// 无障碍标签。
    pub label: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl RadialGaugeDesc {
    /// 创建数值为 0 的环形仪表。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            value_norm: 0.0,
            segments: RADIAL_GAUGE_SEGMENTS,
            label: "Gauge".to_owned(),
            disabled: false,
            z: 720,
        }
    }

    /// 设置归一化数值，自动夹紧。
    #[must_use]
    pub fn value_norm(mut self, value: f32) -> Self {
        self.value_norm = value.clamp(0.0, 1.0);
        self
    }

    /// 设置刻度数量（至少 3）。
    #[must_use]
    pub fn with_segments(mut self, segments: usize) -> Self {
        self.segments = segments.max(3);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 圆心。
    #[must_use]
    pub fn center(&self) -> unbound_math::Vec2 {
        self.rect.center()
    }

    /// 半径。
    #[must_use]
    pub fn radius(&self) -> f32 {
        self.rect.width().min(self.rect.height()) * 0.5
    }

    /// 指针方向角（弧度），从 135° 扫到 405°。
    #[must_use]
    pub fn needle_angle(&self) -> f32 {
        std::f32::consts::PI * 0.75 + self.value_norm.clamp(0.0, 1.0) * std::f32::consts::PI * 1.5
    }

    /// 指针末端坐标。
    #[must_use]
    pub fn needle_tip(&self) -> unbound_math::Vec2 {
        let center = self.center();
        let length = self.radius() * RADIAL_GAUGE_NEEDLE;
        let angle = self.needle_angle();
        unbound_math::Vec2::new(
            center.x + angle.cos() * length,
            center.y + angle.sin() * length,
        )
    }

    /// 第 `index` 个刻度的矩形。
    #[must_use]
    pub fn tick_rect(&self, index: usize) -> Rect {
        let count = self.segments.max(1);
        let fraction = index as f32 / count as f32;
        let angle = std::f32::consts::PI * 0.75 + fraction * std::f32::consts::PI * 1.5;
        let center = self.center();
        let distance = self.radius() * 0.88;
        Rect::new(
            center.x + angle.cos() * distance - 1.5,
            center.y + angle.sin() * distance - 1.5,
            3.0,
            3.0,
        )
    }
}

/// 输出环形仪表的图元、命中区域与语义节点。
#[must_use]
pub fn emit_radial_gauge(
    node: u64,
    desc: &RadialGaugeDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;
    let center = desc.center();
    let radius = desc.radius();

    let mut primitives = Vec::with_capacity(desc.segments + 5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(1 + label_glyphs.len());

    primitives.push(surface(
        Rect::new(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        ),
        radius,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else if hovered {
            Token::CONTROL_HOVER
        } else {
            Token::SURFACE_SUNKEN
        },
        z,
    ));
    for index in 0..desc.segments {
        primitives.push(surface(
            desc.tick_rect(index),
            1.5,
            if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            },
            z + 1,
        ));
    }
    let tip = desc.needle_tip();
    curve_stroke(
        [center.x, center.y],
        [tip.x, tip.y],
        2.0,
        if disabled {
            Token::CONTROL_DISABLED_THUMB
        } else {
            Token::PRIMARY
        },
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(surface(
        Rect::new(
            center.x - RADIAL_GAUGE_HUB * 0.5,
            center.y - RADIAL_GAUGE_HUB * 0.5,
            RADIAL_GAUGE_HUB,
            RADIAL_GAUGE_HUB,
        ),
        RADIAL_GAUGE_HUB * 0.5,
        if disabled {
            Token::CONTROL_DISABLED_THUMB
        } else {
            Token::TEXT_PRIMARY
        },
        z + 3,
    ));
    emit_glyphs(
        label_glyphs,
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
        radius,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect).with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::ProgressIndicator, desc.label.clone(), rect)
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

    fn desc() -> RadialGaugeDesc {
        RadialGaugeDesc::new(Rect::new(0.0, 0.0, 160.0, 160.0))
    }

    #[test]
    fn the_needle_sweeps_with_the_value() {
        let low = desc().value_norm(0.0);
        let high = desc().value_norm(1.0);
        assert!((high.needle_angle() - low.needle_angle()).abs() > 1.0);
        assert_ne!(low.needle_tip().x.round(), high.needle_tip().x.round());
    }

    #[test]
    fn ticks_sit_on_the_dial_ring() {
        let d = desc();
        let center = d.center();
        for index in 0..d.segments {
            let tick = d.tick_rect(index);
            let distance = (tick.center().x - center.x).hypot(tick.center().y - center.y);
            assert!((distance - d.radius() * 0.88).abs() < 0.5);
        }
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().value_norm(0.6);
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
                        counts.push(emit_radial_gauge(1, &d, state, &mut []).primitives.len());
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

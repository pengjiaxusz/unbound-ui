//! RangeSelector：双拇指区间滑块。
//!
//! 轨道、已选区间与两个拇指构成固定四件套，外加焦点环。拇指位置由低频 / 高频
//! 两个端点归一化得到，端点顺序在构造时即被夹紧。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 轨道厚度。
pub const RANGE_TRACK_HEIGHT: f32 = 4.0;
/// 拇指边长。
pub const RANGE_THUMB_SIZE: f32 = 16.0;

/// 区间选择器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RangeSelectorDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 最小值。
    pub min: f32,
    /// 最大值。
    pub max: f32,
    /// 低端点。
    pub low: f32,
    /// 高端点。
    pub high: f32,
    /// 无障碍标签。
    pub label: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl RangeSelectorDesc {
    /// 创建覆盖整个取值范围的区间选择器。
    #[must_use]
    pub fn new(rect: Rect, min: f32, max: f32) -> Self {
        let min = min.min(max);
        let max = max.max(min);
        Self {
            rect,
            min,
            max,
            low: min,
            high: max,
            label: "Range".to_owned(),
            disabled: false,
            z: 700,
        }
    }

    /// 设置两个端点，顺序颠倒时自动交换并夹紧。
    #[must_use]
    pub fn with_range(mut self, low: f32, high: f32) -> Self {
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        self.low = low.clamp(self.min, self.max);
        self.high = high.clamp(self.min, self.max);
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 取值跨度，保证非零。
    #[must_use]
    fn span(&self) -> f32 {
        (self.max - self.min).abs().max(f32::EPSILON)
    }

    /// 数值的归一化位置。
    #[must_use]
    pub fn fraction_of(&self, value: f32) -> f32 {
        ((value - self.min) / self.span()).clamp(0.0, 1.0)
    }

    /// 轨道矩形。
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.height() - RANGE_TRACK_HEIGHT) * 0.5,
            self.rect.width(),
            RANGE_TRACK_HEIGHT,
        )
    }

    /// 已选区间矩形。
    #[must_use]
    pub fn active_rect(&self) -> Rect {
        let track = self.track_rect();
        let travel = (track.width() - RANGE_THUMB_SIZE).max(0.0);
        let low = self.fraction_of(self.low);
        let high = self.fraction_of(self.high);
        Rect::new(
            track.origin.x + RANGE_THUMB_SIZE * 0.5 + travel * low,
            track.origin.y,
            (travel * (high - low)).max(0.0),
            track.height(),
        )
    }

    /// 端点 `index`（0=低、1=高）的拇指矩形。
    #[must_use]
    pub fn thumb_rect(&self, index: usize) -> Rect {
        let track = self.track_rect();
        let travel = (track.width() - RANGE_THUMB_SIZE).max(0.0);
        let fraction = self.fraction_of(if index == 0 { self.low } else { self.high });
        Rect::new(
            track.origin.x + travel * fraction,
            track.origin.y + (track.height() - RANGE_THUMB_SIZE) * 0.5,
            RANGE_THUMB_SIZE,
            RANGE_THUMB_SIZE,
        )
    }
}

/// 输出区间选择器的图元、命中区域与语义节点。
#[must_use]
pub fn emit_range_selector(
    node: u64,
    desc: &RangeSelectorDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(6 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    primitives.push(surface(desc.track_rect(), 2.0, Token::CONTROL_TRACK, z));
    primitives.push(surface(
        desc.active_rect(),
        2.0,
        if disabled {
            Token::SLIDER_FILL_DISABLED
        } else {
            Token::PRIMARY
        },
        z + 1,
    ));
    for index in 0..2 {
        primitives.push(surface(
            desc.thumb_rect(index),
            RANGE_THUMB_SIZE * 0.5,
            if disabled {
                Token::SLIDER_THUMB_DISABLED
            } else if hovered {
                Token::PRIMARY_HOVER
            } else {
                Token::CONTROL_THUMB
            },
            z + 2,
        ));
    }
    emit_glyphs(
        label_glyphs,
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
        4.0,
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
                .with_shape(Shape::Rounded { radii: [4.0; 4] })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Slider, desc.label.clone(), rect)
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

    fn desc() -> RangeSelectorDesc {
        RangeSelectorDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), 0.0, 100.0).with_range(25.0, 75.0)
    }

    #[test]
    fn endpoints_are_normalized_between_zero_and_one() {
        let d = desc();
        assert!((d.fraction_of(25.0) - 0.25).abs() < 1e-4);
        assert!((d.fraction_of(75.0) - 0.75).abs() < 1e-4);
        assert_eq!(d.fraction_of(-10.0), 0.0);
        assert_eq!(d.fraction_of(140.0), 1.0);
    }

    #[test]
    fn reversed_endpoints_are_swapped() {
        let d = RangeSelectorDesc::new(Rect::new(0.0, 0.0, 200.0, 32.0), 0.0, 100.0)
            .with_range(80.0, 20.0);
        assert_eq!(d.low, 20.0);
        assert_eq!(d.high, 80.0);
    }

    #[test]
    fn the_low_thumb_precedes_the_high_thumb() {
        let d = desc();
        assert!(d.thumb_rect(0).origin.x <= d.thumb_rect(1).origin.x);
        assert!(d.active_rect().width() > 0.0);
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
                        counts.push(emit_range_selector(1, &d, state, &mut []).primitives.len());
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

//! BladeView：水平展开的多级刀片面板（Azure 门户风格）。
//!
//! 每片刀片由一个面板底与一条标题条组成，刀片之间以细分隔线隔开。
//! 活动刀片用强调浅色标题条标记；图元数量只取决于刀片数量。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 刀片间距。
pub const BLADE_GAP: f32 = 6.0;
/// 标题条高度。
pub const BLADE_HEADER_HEIGHT: f32 = 30.0;
/// 刀片圆角。
pub const BLADE_RADIUS: f32 = 6.0;

/// 多级刀片面板描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct BladeViewDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 刀片数量（至少 1）。
    pub blades: usize,
    /// 活动刀片索引。
    pub active: usize,
    /// 无障碍标签。
    pub label: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl BladeViewDesc {
    /// 创建三片刀片的面板。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            blades: 3,
            active: 0,
            label: "Blades".to_owned(),
            disabled: false,
            z: 724,
        }
    }

    /// 设置刀片数量。
    #[must_use]
    pub fn with_blades(mut self, blades: usize) -> Self {
        self.blades = blades.max(1);
        self.active = self.active.min(self.blades - 1);
        self
    }

    /// 设置活动刀片。
    #[must_use]
    pub fn active(mut self, index: usize) -> Self {
        self.active = index.min(self.blades.saturating_sub(1));
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 第 `index` 片刀片的矩形。
    #[must_use]
    pub fn blade_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.blades {
            return None;
        }
        let count = self.blades.max(1) as f32;
        let width = (self.rect.width() - BLADE_GAP * (count - 1.0)) / count;
        Some(Rect::new(
            self.rect.origin.x + index as f32 * (width + BLADE_GAP),
            self.rect.origin.y,
            width.max(0.0),
            self.rect.height(),
        ))
    }

    /// 第 `index` 片刀片的标题条矩形。
    #[must_use]
    pub fn header_rect(&self, index: usize) -> Option<Rect> {
        self.blade_rect(index).map(|blade| {
            Rect::new(
                blade.origin.x,
                blade.origin.y,
                blade.width(),
                BLADE_HEADER_HEIGHT.min(blade.height()),
            )
        })
    }
}

/// 输出多级刀片面板的图元、命中区域与语义节点。
#[must_use]
pub fn emit_blade_view(
    node: u64,
    desc: &BladeViewDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;

    let mut primitives = Vec::with_capacity(desc.blades * 2 + 2 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    for index in 0..desc.blades {
        let Some(blade) = desc.blade_rect(index) else {
            continue;
        };
        let active = index == desc.active;
        primitives.push(outlined(
            blade,
            BLADE_RADIUS,
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else {
                Token::CARD_BG
            },
            if disabled {
                Token::CONTROL_DISABLED_BORDER
            } else {
                Token::CARD_BORDER
            },
            1.0,
            0.0,
            z,
        ));
        if let Some(header) = desc.header_rect(index) {
            primitives.push(surface(
                header,
                BLADE_RADIUS,
                if disabled {
                    Token::CONTROL_DISABLED_BG
                } else if active {
                    Token::PRIMARY_SOFT
                } else if hovered {
                    Token::CONTROL_HOVER
                } else {
                    Token::SURFACE_SUNKEN
                },
                z + 1,
            ));
        }
    }

    // 刀片之间的细分隔线始终到达：数量固定为 blade - 1。
    for index in 0..desc.blades.saturating_sub(1) {
        if let Some(blade) = desc.blade_rect(index) {
            primitives.push(surface(
                Rect::new(
                    blade.max().x + BLADE_GAP * 0.5,
                    desc.rect.origin.y + 6.0,
                    1.0,
                    (desc.rect.height() - 12.0).max(0.0),
                ),
                0.0,
                Token::DIVIDER,
                z + 2,
            ));
        }
    }

    emit_glyphs(
        label_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        0.0,
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
                    radii: [BLADE_RADIUS; 4],
                })
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Group, desc.label.clone(), rect)
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

    fn desc() -> BladeViewDesc {
        BladeViewDesc::new(Rect::new(0.0, 0.0, 360.0, 220.0))
    }

    #[test]
    fn blades_split_the_width_evenly() {
        let d = desc();
        let first = d.blade_rect(0).expect("blade 0");
        let second = d.blade_rect(1).expect("blade 1");
        assert!((first.width() - second.width()).abs() < 1e-4);
        assert!(first.max().x <= second.origin.x);
        assert!(d.blade_rect(3).is_none());
    }

    #[test]
    fn headers_cap_each_blade() {
        let d = desc();
        let header = d.header_rect(0).expect("header");
        assert_eq!(header.height(), BLADE_HEADER_HEIGHT);
        assert_eq!(header.width(), d.blade_rect(0).expect("blade").width());
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().active(1);
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
                        counts.push(emit_blade_view(1, &d, state, &mut []).primitives.len());
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

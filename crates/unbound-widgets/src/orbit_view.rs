//! OrbitView：环绕式布局视图。
//!
//! 中心轴周围均匀分布固定数量的节点，节点落在一条描边圆环上。
//! 选中节点只改变填充令牌，不影响节点数量与位置。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 节点直径。
pub const ORBIT_NODE_SIZE: f32 = 28.0;
/// 节点距中心的比例。
pub const ORBIT_RADIUS_RATIO: f32 = 0.78;

/// 环绕布局描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct OrbitViewDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 环绕节点数量。
    pub nodes: usize,
    /// 当前选中节点。
    pub selected: Option<usize>,
    /// 无障碍标签。
    pub label: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl OrbitViewDesc {
    /// 创建六个环绕节点。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            nodes: 6,
            selected: None,
            label: "Orbit view".to_owned(),
            disabled: false,
            z: 722,
        }
    }

    /// 设置节点数量（至少 1）。
    #[must_use]
    pub fn with_nodes(mut self, nodes: usize) -> Self {
        self.nodes = nodes.max(1);
        self
    }

    /// 设置选中节点。
    #[must_use]
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 环绕半径。
    #[must_use]
    pub fn orbit_radius(&self) -> f32 {
        self.rect.width().min(self.rect.height()) * 0.5 * ORBIT_RADIUS_RATIO
    }

    /// 第 `index` 个节点的圆心。
    #[must_use]
    pub fn node_center(&self, index: usize) -> unbound_math::Vec2 {
        let count = self.nodes.max(1);
        let angle = std::f32::consts::TAU * index as f32 / count as f32;
        let center = self.rect.center();
        unbound_math::Vec2::new(
            center.x + angle.cos() * self.orbit_radius(),
            center.y + angle.sin() * self.orbit_radius(),
        )
    }

    /// 第 `index` 个节点的矩形。
    #[must_use]
    pub fn node_rect(&self, index: usize) -> Rect {
        let center = self.node_center(index);
        Rect::new(
            center.x - ORBIT_NODE_SIZE * 0.5,
            center.y - ORBIT_NODE_SIZE * 0.5,
            ORBIT_NODE_SIZE,
            ORBIT_NODE_SIZE,
        )
    }
}

/// 输出环绕布局的图元、命中区域与语义节点。
#[must_use]
pub fn emit_orbit_view(
    node: u64,
    desc: &OrbitViewDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;
    let center = rect.center();
    let radius = desc.orbit_radius();

    let mut primitives = Vec::with_capacity(desc.nodes + 4 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    primitives.push(outlined(
        Rect::new(
            center.x - radius,
            center.y - radius,
            radius * 2.0,
            radius * 2.0,
        ),
        radius,
        Token::NONE,
        if disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if hovered {
            Token::PRIMARY
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        z,
    ));
    for index in 0..desc.nodes {
        let selected = desc.selected == Some(index);
        primitives.push(surface(
            desc.node_rect(index),
            ORBIT_NODE_SIZE * 0.5,
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else if selected {
                Token::PRIMARY
            } else {
                Token::SURFACE_RAISED
            },
            z + 1,
        ));
    }
    primitives.push(surface(
        Rect::new(center.x - 22.0, center.y - 22.0, 44.0, 44.0),
        22.0,
        if disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        z + 2,
    ));
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
        radius,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect).with_cursor(CursorKind::Pointer)
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

    fn desc() -> OrbitViewDesc {
        OrbitViewDesc::new(Rect::new(0.0, 0.0, 200.0, 200.0))
    }

    #[test]
    fn nodes_sit_on_the_orbit_ring() {
        let d = desc();
        let center = d.rect.center();
        for index in 0..d.nodes {
            let node = d.node_rect(index).center();
            let distance = (node.x - center.x).hypot(node.y - center.y);
            assert!((distance - d.orbit_radius()).abs() < 0.5);
        }
    }

    #[test]
    fn the_first_node_starts_at_the_right_edge() {
        let d = desc();
        let first = d.node_center(0);
        assert!((first.y - d.rect.center().y).abs() < 1e-3);
        assert!(first.x > d.rect.center().x);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().selected(2);
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
                        counts.push(emit_orbit_view(1, &d, state, &mut []).primitives.len());
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

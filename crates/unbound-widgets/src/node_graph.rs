//! 节点图编辑器图元。
//!
//! 节点图是带有两种额外交互能力（可拖拽卡片与类型化端口间连线）的画布。
//! 几何信息驻留于此；模型（存在哪些节点、连线含义）由调用方维护。
//! 状态机仅跟踪*交互*：选区、拖拽中的连线，以及在 12 px 内
//! 将连线端点吸附到兼容端口的磁力机制。
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_canvas::Selection;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;
use unbound_vector::cubic_to_quads;

use crate::primitive::{glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::WidgetOutput;

/// 端口圆点半径。
pub const PIN_RADIUS: f32 = 5.0;
/// 拖拽连线吸附到端口的距离。
pub const PIN_SNAP: f32 = 12.0;
/// 节点卡片标题栏高度。
pub const NODE_HEADER: f32 = 28.0;
/// 端口之间的垂直间距。
pub const PIN_SPACING: f32 = 22.0;
/// 曲线扁平化容差（逻辑像素）。
pub const WIRE_TOLERANCE: f32 = 0.25;
/// 连线宽度。
pub const WIRE_WIDTH: f32 = 2.0;

/// 端口类型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PinKind {
    /// 传输数据。
    #[default]
    Data,
    /// 传输执行流。
    Flow,
}

impl PinKind {
    /// 端口颜色。
    #[must_use]
    pub fn token(self) -> Token {
        match self {
            Self::Data => Token::PRIMARY,
            Self::Flow => Token::SECONDARY,
        }
    }
}

/// 单个输入或输出端口。
#[derive(Clone, Debug, PartialEq)]
pub struct PinSpec {
    /// 圆点旁显示的标签。
    pub label: String,
    /// 端口类型。
    pub kind: PinKind,
}

impl PinSpec {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: PinKind::Data,
        }
    }

    /// 构造执行流类型的同名端口。
    #[must_use]
    pub fn flow(mut self) -> Self {
        self.kind = PinKind::Flow;
        self
    }
}

/// 图中的单个卡片。
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNodeDesc {
    /// 稳定 ID。
    pub id: u32,
    /// 卡片原点（图空间）。
    pub pos: Vec2,
    /// 卡片尺寸。
    pub size: Vec2,
    /// 标题栏文本。
    pub title: String,
    /// 输入端口列表（从上到下）。
    pub inputs: Vec<PinSpec>,
    /// 输出端口列表（从上到下）。
    pub outputs: Vec<PinSpec>,
    /// 标题栏填充色。
    pub header_token: Token,
}

impl GraphNodeDesc {
    /// 创建带标题的卡片。
    #[must_use]
    pub fn new(id: u32, pos: Vec2, size: Vec2, title: impl Into<String>) -> Self {
        Self {
            id,
            pos,
            size,
            title: title.into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            header_token: Token::HEADER_BG,
        }
    }

    /// 为卡片设置输入输出端口。
    #[must_use]
    pub fn with_ports(mut self, inputs: Vec<PinSpec>, outputs: Vec<PinSpec>) -> Self {
        self.inputs = inputs;
        self.outputs = outputs;
        self
    }

    /// 卡片矩形。
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, self.size.x, self.size.y)
    }

    /// 标题栏矩形。
    #[must_use]
    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.pos.x,
            self.pos.y,
            self.size.x,
            NODE_HEADER.min(self.size.y),
        )
    }

    /// 输入端口中心点。
    #[must_use]
    pub fn input_center(&self, index: usize) -> Vec2 {
        Vec2::new(
            self.pos.x - PIN_RADIUS,
            self.pos.y + NODE_HEADER + 16.0 + index as f32 * PIN_SPACING,
        )
    }

    /// 输出端口中心点。
    #[must_use]
    pub fn output_center(&self, index: usize) -> Vec2 {
        Vec2::new(
            self.pos.x + self.size.x + PIN_RADIUS,
            self.pos.y + NODE_HEADER + 16.0 + index as f32 * PIN_SPACING,
        )
    }

    /// 按侧边与索引获取端口中心点。
    #[must_use]
    pub fn pin_center(&self, output: bool, index: usize) -> Option<Vec2> {
        let count = if output {
            self.outputs.len()
        } else {
            self.inputs.len()
        };
        (index < count).then(|| {
            if output {
                self.output_center(index)
            } else {
                self.input_center(index)
            }
        })
    }

    /// 查找 `point` 周围 `radius` 范围内的端口。
    #[must_use]
    pub fn pin_at(&self, point: Vec2, radius: f32) -> Option<(bool, u8)> {
        for output in [false, true] {
            let count = if output {
                self.outputs.len()
            } else {
                self.inputs.len()
            };
            for index in 0..count {
                let centre = self.pin_center(output, index)?;
                if (centre - point).length_squared() <= radius * radius {
                    return Some((output, index as u8));
                }
            }
        }
        None
    }
}

/// 两个端口之间的连接。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphWire {
    /// 源节点。
    pub from_node: u32,
    /// 源输出端口索引。
    pub from_pin: u8,
    /// 目标节点。
    pub to_node: u32,
    /// 目标输入端口索引。
    pub to_pin: u8,
}

/// 当前正在拖拽的连线。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveWire {
    /// 源节点。
    pub from_node: u32,
    /// 源输出端口索引。
    pub from_pin: u8,
    /// 指针位置（图空间）。
    pub cursor: Vec2,
    /// 靠近时连线将吸附到的目标端口。
    pub snap: Option<(u32, u8)>,
}

/// 单个节点图的交互状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeGraphState {
    /// 已选中的节点。
    pub selection: Selection,
    /// 拖拽中的连线。
    pub dragging: Option<ActiveWire>,
    /// 视口平移量。
    pub pan: Vec2,
    /// 视口缩放比例。
    pub zoom: f32,
}

impl NodeGraphState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            ..Self::default()
        }
    }

    /// 单选节点，替换当前选区。
    pub fn select(&mut self, id: u32) {
        self.selection.set(&[id]);
    }

    /// 切换单个节点的选中状态。
    pub fn toggle(&mut self, id: u32) {
        self.selection.toggle(id);
    }

    /// 从输出端口开始拖拽连线。
    pub fn begin_wire(&mut self, from_node: u32, from_pin: u8, cursor: Vec2) {
        self.dragging = Some(ActiveWire {
            from_node,
            from_pin,
            cursor,
            snap: None,
        });
    }

    /// 移动拖拽中的连线并查找兼容的目标端口。
    pub fn drag_wire(&mut self, cursor: Vec2, nodes: &[GraphNodeDesc]) {
        if let Some(wire) = &mut self.dragging {
            wire.cursor = cursor;
            wire.snap =
                nearest_input(nodes, cursor, PIN_SNAP).filter(|(node, _)| *node != wire.from_node);
        }
    }

    /// 结束拖拽；若落在端口上则返回新建的连线。
    pub fn end_wire(&mut self) -> Option<GraphWire> {
        let wire = self.dragging.take()?;
        let (to_node, to_pin) = wire.snap?;
        Some(GraphWire {
            from_node: wire.from_node,
            from_pin: wire.from_pin,
            to_node,
            to_pin,
        })
    }

    /// 放弃当前拖拽中的连线。
    pub fn cancel_wire(&mut self) {
        self.dragging = None;
    }
}

/// 查找 `radius` 范围内最近的输入端口。
#[must_use]
pub fn nearest_input(nodes: &[GraphNodeDesc], point: Vec2, radius: f32) -> Option<(u32, u8)> {
    let mut best: Option<(f32, u32, u8)> = None;
    for node in nodes {
        for index in 0..node.inputs.len() {
            let Some(centre) = node.pin_center(false, index) else {
                continue;
            };
            let distance = (centre - point).length_squared();
            if distance <= radius * radius && best.is_none_or(|(d, _, _)| distance < d) {
                best = Some((distance, node.id, index as u8));
            }
        }
    }
    best.map(|(_, node, pin)| (node, pin))
}

/// 单个端口命中区域。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinHit {
    /// 所属节点。
    pub node: u32,
    /// 端口索引。
    pub pin: u8,
    /// 是否为输出端口。
    pub output: bool,
    /// 圆点周围的正方形命中框。
    pub rect: Rect,
}

/// 节点图的输出内容。
#[derive(Clone, Debug)]
pub struct NodeGraphOutput {
    /// 卡片与连线图元。
    pub output: WidgetOutput,
    /// 端口命中区域。
    pub pins: Vec<PinHit>,
    /// 卡片命中区域。
    pub nodes: Vec<HitRegion>,
}

/// 输出节点图。`node_glyphs` 必须与 `nodes` 一一对应。
#[must_use]
pub fn emit_node_graph(
    _node: u64,
    nodes: &[GraphNodeDesc],
    wires: &[GraphWire],
    state: &NodeGraphState,
    node_glyphs: &[Vec<GlyphQuad>],
) -> NodeGraphOutput {
    let mut primitives: Vec<GpuPrimitiveInstance> = Vec::new();
    let mut payloads: Vec<GpuPayload> = Vec::new();
    let mut pins = Vec::new();
    let mut hits = Vec::new();
    let z = 580u16;

    for wire in wires {
        let (Some(from), Some(to)) = (
            nodes
                .iter()
                .find(|node| node.id == wire.from_node)
                .and_then(|node| node.pin_center(true, usize::from(wire.from_pin))),
            nodes
                .iter()
                .find(|node| node.id == wire.to_node)
                .and_then(|node| node.pin_center(false, usize::from(wire.to_pin))),
        ) else {
            continue;
        };
        push_wire(
            &mut primitives,
            &mut payloads,
            from,
            to,
            Token::SECONDARY,
            581,
        );
    }

    if let Some(active) = state.dragging {
        if let Some(from) = nodes
            .iter()
            .find(|node| node.id == active.from_node)
            .and_then(|node| node.pin_center(true, usize::from(active.from_pin)))
        {
            let end = active
                .snap
                .and_then(|(node, pin)| {
                    nodes
                        .iter()
                        .find(|item| item.id == node)
                        .and_then(|item| item.pin_center(false, usize::from(pin)))
                })
                .unwrap_or(active.cursor);
            push_wire(
                &mut primitives,
                &mut payloads,
                from,
                end,
                Token::PRIMARY,
                581,
            );
        }
    }

    for (index, node) in nodes.iter().enumerate() {
        let selected = state.selection.contains(node.id);
        let rect = node.rect();
        primitives.push(outlined(
            rect,
            8.0,
            if selected {
                Token::CANVAS_NODE_SELECTED
            } else {
                Token::CANVAS_NODE
            },
            if selected {
                Token::PRIMARY
            } else {
                Token::OUTLINE
            },
            1.0,
            6.0,
            580,
        ));
        primitives.push(outlined(
            node.header_rect(),
            8.0,
            node.header_token,
            Token::NONE,
            0.0,
            0.0,
            581,
        ));
        if let Some(run) = node_glyphs.get(index) {
            glyphs(
                run,
                Token::TEXT_PRIMARY,
                582,
                &mut primitives,
                &mut payloads,
            );
        }
        emit_pins(node, &mut primitives, &mut pins);
        hits.push(HitRegion::new(
            u64::from(node.id),
            CONTROL_LAYER,
            u32::from(z),
            rect,
        ));
    }

    let mut a11y = A11yNode::new(Role::Group, "节点图", Rect::new(0.0, 0.0, 0.0, 0.0));
    a11y.value = format!("{} 个节点", nodes.len());

    NodeGraphOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: None,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        pins,
        nodes: hits,
    }
}

fn emit_pins(
    node: &GraphNodeDesc,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    pins: &mut Vec<PinHit>,
) {
    for output in [false, true] {
        let count = if output {
            node.outputs.len()
        } else {
            node.inputs.len()
        };
        for index in 0..count {
            let Some(centre) = node.pin_center(output, index) else {
                continue;
            };
            let spec = if output {
                &node.outputs[index]
            } else {
                &node.inputs[index]
            };
            let rect = Rect::new(
                centre.x - PIN_RADIUS,
                centre.y - PIN_RADIUS,
                PIN_RADIUS * 2.0,
                PIN_RADIUS * 2.0,
            );
            primitives.push(surface(rect, PIN_RADIUS, spec.kind.token(), 583));
            primitives.push(surface(
                rect.inflate(1.5),
                PIN_RADIUS + 1.5,
                Token::CANVAS_NODE,
                582,
            ));
            pins.push(PinHit {
                node: node.id,
                pin: index as u8,
                output,
                rect,
            });
        }
    }
}

fn push_wire(
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
    from: Vec2,
    to: Vec2,
    color: Token,
    z: u16,
) {
    let dx = (to.x - from.x).abs().max(40.0) * 0.5;
    let c1 = Vec2::new(from.x + dx, from.y);
    let c2 = Vec2::new(to.x - dx, to.y);
    // 控件层没有相机，因此使用固定屏幕容差进行扁平化。
    for quad in cubic_to_quads(from, c1, c2, to, WIRE_TOLERANCE) {
        let min = quad
            .p0
            .min(quad.p1)
            .min(quad.p2)
            .min(quad.p0.min(quad.p1).min(quad.p2));
        let max = quad
            .p0
            .max(quad.p1)
            .max(quad.p2)
            .max(quad.p0.max(quad.p1).max(quad.p2));
        let origin = Rect::new(
            min.x,
            min.y,
            (max.x - min.x).max(1.0),
            (max.y - min.y).max(1.0),
        );
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload::curve(
            [quad.p0.x - origin.origin.x, quad.p0.y - origin.origin.y],
            [quad.p1.x - origin.origin.x, quad.p1.y - origin.origin.y],
            [quad.p2.x - origin.origin.x, quad.p2.y - origin.origin.y],
            WIRE_WIDTH,
        ));
        let mut primitive = instance(
            origin,
            [0.0; 4],
            color,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Curve,
            SubElement::Content,
            z,
        );
        primitive.extra_payload_idx = payload_idx;
        primitives.push(primitive);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, x: f32) -> GraphNodeDesc {
        GraphNodeDesc::new(id, Vec2::new(x, 40.0), Vec2::new(160.0, 120.0), "Node")
            .with_ports(vec![PinSpec::new("in")], vec![PinSpec::new("out")])
    }

    #[test]
    fn ports_sit_on_the_card_edges() {
        let card = node(1, 100.0);
        let input = card.input_center(0);
        let output = card.output_center(0);
        assert!(input.x < card.rect().origin.x);
        assert!(output.x > card.rect().max().x);
        assert_eq!(input.y, output.y);
    }

    #[test]
    fn a_dragged_wire_snaps_to_the_nearest_input() {
        let nodes = vec![node(1, 0.0), node(2, 300.0)];
        let mut state = NodeGraphState::new();
        state.begin_wire(1, 0, state.pan);
        let target = nodes[1].input_center(0);
        state.drag_wire(target + Vec2::new(6.0, 0.0), &nodes);
        assert_eq!(state.dragging.unwrap().snap, Some((2, 0)));
        let wire = state.end_wire().expect("wire");
        assert_eq!(wire.from_node, 1);
        assert_eq!(wire.to_node, 2);
    }

    #[test]
    fn a_wire_beyond_the_magnet_is_discarded() {
        let nodes = vec![node(1, 0.0), node(2, 300.0)];
        let mut state = NodeGraphState::new();
        state.begin_wire(1, 0, Vec2::ZERO);
        state.drag_wire(
            nodes[1].input_center(0) + Vec2::new(PIN_SNAP + 4.0, 0.0),
            &nodes,
        );
        assert!(state.end_wire().is_none());
    }

    #[test]
    fn selecting_replaces_the_selection_and_toggling_extends_it() {
        let mut state = NodeGraphState::new();
        state.select(1);
        state.toggle(2);
        assert_eq!(state.selection.ids(), &[1, 2]);
        state.select(3);
        assert_eq!(state.selection.ids(), &[3]);
    }

    #[test]
    fn the_emitter_draws_cards_pins_and_wires() {
        let nodes = vec![node(1, 0.0), node(2, 300.0)];
        let wires = [GraphWire {
            from_node: 1,
            from_pin: 0,
            to_node: 2,
            to_pin: 0,
        }];
        let state = NodeGraphState::new();
        let output = emit_node_graph(0, &nodes, &wires, &state, &[Vec::new(), Vec::new()]);
        assert_eq!(output.nodes.len(), 2);
        assert_eq!(output.pins.len(), 4);
        assert!(!output.output.primitives.is_empty());
        assert!(!output.output.payloads.is_empty());
    }
}

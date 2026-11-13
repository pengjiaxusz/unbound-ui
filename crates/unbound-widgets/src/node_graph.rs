//! Node graph editor primitives.
//!
//! A node graph is a canvas with two extra affordances: draggable cards and
//! wires between typed ports. The geometry lives here; the model (which nodes
//! exist, what a wire means) stays with the caller. The state machine only
//! tracks the *interaction*: the selection, an in-flight wire and the magnet
//! that snaps a wire end onto a compatible port within 12 px.

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

/// The radius of a port dot.
pub const PIN_RADIUS: f32 = 5.0;
/// The distance at which a dragged wire snaps onto a port.
pub const PIN_SNAP: f32 = 12.0;
/// The node card header height.
pub const NODE_HEADER: f32 = 28.0;
/// The vertical spacing between ports.
pub const PIN_SPACING: f32 = 22.0;
/// The curve flattening tolerance, in logical pixels.
pub const WIRE_TOLERANCE: f32 = 0.25;
/// The wire width.
pub const WIRE_WIDTH: f32 = 2.0;

/// The kind of a port.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PinKind {
    /// Carries data.
    #[default]
    Data,
    /// Carries execution flow.
    Flow,
}

impl PinKind {
    /// The port colour.
    #[must_use]
    pub fn token(self) -> Token {
        match self {
            Self::Data => Token::PRIMARY,
            Self::Flow => Token::SECONDARY,
        }
    }
}

/// One input or output port.
#[derive(Clone, Debug, PartialEq)]
pub struct PinSpec {
    /// The label shown next to the dot.
    pub label: String,
    /// The port kind.
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

    /// The same port as execution flow.
    #[must_use]
    pub fn flow(mut self) -> Self {
        self.kind = PinKind::Flow;
        self
    }
}

/// One card in the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNodeDesc {
    /// The stable id.
    pub id: u32,
    /// The card origin, in graph space.
    pub pos: Vec2,
    /// The card size.
    pub size: Vec2,
    /// The header title.
    pub title: String,
    /// The input ports, top to bottom.
    pub inputs: Vec<PinSpec>,
    /// The output ports, top to bottom.
    pub outputs: Vec<PinSpec>,
    /// The header fill.
    pub header_token: Token,
}

impl GraphNodeDesc {
    /// A card with a title.
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

    /// The same card with ports.
    #[must_use]
    pub fn with_ports(mut self, inputs: Vec<PinSpec>, outputs: Vec<PinSpec>) -> Self {
        self.inputs = inputs;
        self.outputs = outputs;
        self
    }

    /// The card rectangle.
    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, self.size.x, self.size.y)
    }

    /// The header rectangle.
    #[must_use]
    pub fn header_rect(&self) -> Rect {
        Rect::new(
            self.pos.x,
            self.pos.y,
            self.size.x,
            NODE_HEADER.min(self.size.y),
        )
    }

    /// The centre of an input port.
    #[must_use]
    pub fn input_center(&self, index: usize) -> Vec2 {
        Vec2::new(
            self.pos.x - PIN_RADIUS,
            self.pos.y + NODE_HEADER + 16.0 + index as f32 * PIN_SPACING,
        )
    }

    /// The centre of an output port.
    #[must_use]
    pub fn output_center(&self, index: usize) -> Vec2 {
        Vec2::new(
            self.pos.x + self.size.x + PIN_RADIUS,
            self.pos.y + NODE_HEADER + 16.0 + index as f32 * PIN_SPACING,
        )
    }

    /// A port centre, by side and index.
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

    /// The port under `point`, within `radius`.
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

/// A connection between two ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphWire {
    /// The source node.
    pub from_node: u32,
    /// The source output index.
    pub from_pin: u8,
    /// The target node.
    pub to_node: u32,
    /// The target input index.
    pub to_pin: u8,
}

/// The wire currently being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveWire {
    /// The source node.
    pub from_node: u32,
    /// The source output index.
    pub from_pin: u8,
    /// The pointer, in graph space.
    pub cursor: Vec2,
    /// The port the wire will snap onto, when it is near one.
    pub snap: Option<(u32, u8)>,
}

/// The interaction state of one graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeGraphState {
    /// The selected nodes.
    pub selection: Selection,
    /// The in-flight wire.
    pub dragging: Option<ActiveWire>,
    /// The viewport pan.
    pub pan: Vec2,
    /// The viewport zoom.
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

    /// Select one node, replacing the selection.
    pub fn select(&mut self, id: u32) {
        self.selection.set(&[id]);
    }

    /// Toggle one node.
    pub fn toggle(&mut self, id: u32) {
        self.selection.toggle(id);
    }

    /// Start dragging a wire from an output port.
    pub fn begin_wire(&mut self, from_node: u32, from_pin: u8, cursor: Vec2) {
        self.dragging = Some(ActiveWire {
            from_node,
            from_pin,
            cursor,
            snap: None,
        });
    }

    /// Move the in-flight wire and look for a compatible target port.
    pub fn drag_wire(&mut self, cursor: Vec2, nodes: &[GraphNodeDesc]) {
        if let Some(wire) = &mut self.dragging {
            wire.cursor = cursor;
            wire.snap =
                nearest_input(nodes, cursor, PIN_SNAP).filter(|(node, _)| *node != wire.from_node);
        }
    }

    /// Finish the drag, returning the wire when it landed on a port.
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

    /// Abandon the in-flight wire.
    pub fn cancel_wire(&mut self) {
        self.dragging = None;
    }
}

/// The closest input port within `radius`.
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

/// One port hit region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinHit {
    /// The owning node.
    pub node: u32,
    /// The port index.
    pub pin: u8,
    /// Whether it is an output.
    pub output: bool,
    /// The square hit box around the dot.
    pub rect: Rect,
}

/// What one graph emits.
#[derive(Clone, Debug)]
pub struct NodeGraphOutput {
    /// The cards and wires.
    pub output: WidgetOutput,
    /// The port hit regions.
    pub pins: Vec<PinHit>,
    /// The card hit regions.
    pub nodes: Vec<HitRegion>,
}

/// Emit a node graph. `node_glyphs` must be parallel to `nodes`.
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
    // The widget layer has no camera, so flatten at a fixed screen tolerance.
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

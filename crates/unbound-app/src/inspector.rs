//! DevTools inspector and the runtime half of the debug loop (`docs` §10).
//!
//! The inspector is deliberately *not* a widget: it paints straight into the
//! scene collector on the debug pass (the highest `z`, drawn last) and reads
//! the same [`UiWorld`] and hit regions the runtime uses. That keeps it out of
//! the layout tree, so toggling it never re-solves a layout.
//!
//! When debug sanitizers are on, every frame is also mirrored into a
//! [`unbound_headless::Frame`] and run through the five sanitizers. Their
//! diagnostics become blinking error boxes plus text lines in the panel.

use std::collections::HashMap;

use unbound_abi::NodeFlags;
use unbound_gpu::data::{
    GpuPayload, GpuPrimitiveInstance, PassLayer, PrimType, SubElement, CLIP_NONE,
};
use unbound_headless::{Diagnostic, Frame, NodeKind, NodeSnapshot, SourceLoc};
use unbound_input::{pick_stack, HitRegion};
use unbound_math::{f16, Rect, Vec2};
use unbound_theme::Token;
use unbound_world::{NodeId, UiWorld};

/// Top of the primitive `z` range: the debug pass is painted above everything.
pub const INSPECTOR_Z: u16 = u16::MAX;

/// Width of the right-hand panel, in logical pixels.
pub const PANEL_WIDTH: f32 = 300.0;
/// Header strip height.
pub const PANEL_HEADER: f32 = 26.0;
/// How fast the violation boxes blink, in radians per second.
const BLINK_HZ: f32 = 2.0;

/// One frame's box model, resolved from the world.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorNode {
    pub id: NodeId,
    /// Stable semantic key (`RawNodeDesc::key`).
    pub key: u64,
    /// Where the node was built.
    pub source: SourceLoc,
    /// Margin box (border box expanded by the declared margins).
    pub margin: Rect,
    /// Border box — the node's own `world_rect`.
    pub border: Rect,
    /// Border box minus the border width.
    pub padding: Rect,
    /// Padding box minus the padding.
    pub content: Rect,
}

/// A sanitizer failure, ready to paint.
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    pub node: u64,
    pub rect: Rect,
    pub rule: &'static str,
    pub message: String,
}

/// What the inspector contributes to one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorScene {
    /// Always [`PassLayer::Debug`]: the caller appends this after the scene.
    pub layer: PassLayer,
    pub primitives: Vec<GpuPrimitiveInstance>,
    pub payloads: Vec<GpuPayload>,
    /// The picked node, when the pointer is over something.
    pub node: Option<InspectorNode>,
    pub violations: Vec<Violation>,
    /// Text the host draws inside the panel (it owns the font).
    pub lines: Vec<String>,
}

impl InspectorScene {
    /// True when there is nothing to append.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
    }
}

/// The inspector state: enabled flag, picked node, violations and blink phase.
#[derive(Clone, Debug, Default)]
pub struct Inspector {
    enabled: bool,
    pick: Option<InspectorNode>,
    violations: Vec<Violation>,
    blink: f32,
}

impl Inspector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle on/off; returns the new state.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Advance the blink phase.
    pub fn tick(&mut self, dt: f32) {
        self.blink = (self.blink + dt * BLINK_HZ * core::f32::consts::TAU)
            .rem_euclid(core::f32::consts::TAU);
    }

    /// Blink opacity in `0.35..=1.0`.
    #[must_use]
    pub fn blink_opacity(&self) -> f32 {
        0.35 + 0.325 * (self.blink.sin() + 1.0)
    }

    /// Repick the node under `pointer`.
    ///
    /// The hit stack wins because it is what a click would do; when the pointer
    /// is over a non-interactive gap the deepest containing node is used so the
    /// box model still works on plain containers.
    pub fn update(&mut self, world: &UiWorld, regions: &[HitRegion], pointer: Vec2) {
        let hit = pick_stack(regions, pointer)
            .first()
            .map(|region| region.node);
        let id = hit
            .and_then(|key| world.find_key(key))
            .or_else(|| deepest_at(world, pointer));
        self.pick = id.map(|id| describe(world, id));
    }

    /// Replace the violation list from a sanitizer report.
    pub fn set_diagnostics(&mut self, diagnostics: &[Diagnostic]) {
        self.violations.clear();
        self.violations
            .extend(diagnostics.iter().map(|diagnostic| Violation {
                node: diagnostic.node,
                rect: diagnostic.rect,
                rule: diagnostic.rule.name(),
                message: diagnostic.message.clone(),
            }));
    }

    /// Drop the violation list, e.g. when sanitizers are disabled.
    pub fn clear_diagnostics(&mut self) {
        self.violations.clear();
    }

    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    #[must_use]
    pub fn picked(&self) -> Option<&InspectorNode> {
        self.pick.as_ref()
    }

    /// Build this frame's overlay.
    #[must_use]
    pub fn emit(&self, viewport: Vec2) -> InspectorScene {
        let mut scene = InspectorScene {
            layer: PassLayer::Debug,
            violations: self.violations.clone(),
            ..InspectorScene::default()
        };
        if !self.enabled {
            return scene;
        }

        // Violations first so the picked box model paints over them.
        let opacity = self.blink_opacity();
        for violation in &self.violations {
            push_ring(
                &mut scene.primitives,
                violation.rect,
                violation.rect,
                2.0,
                Token::ERROR,
                INSPECTOR_Z - 8,
                opacity,
            );
        }

        if let Some(node) = &self.pick {
            scene.node = Some(node.clone());
            push_box_model(&mut scene.primitives, node);
            scene.lines.push(format!(
                "{}  key={}  rect={:?}",
                node.source, node.key, node.border
            ));
        } else {
            scene.lines.push("Inspector: hover a node".to_owned());
        }

        scene
            .lines
            .push(format!("violations: {}", self.violations.len()));
        for violation in &self.violations {
            scene.lines.push(format!(
                "[{}] node {} {}",
                violation.rule, violation.node, violation.message
            ));
        }

        push_panel(&mut scene.primitives, viewport, &scene.lines);
        scene
    }
}

/// Resolve one node's box model from the world.
#[must_use]
pub fn describe(world: &UiWorld, id: NodeId) -> InspectorNode {
    let data = world.node(id);
    let border = data.world_rect;
    let bw = data.desc.border_width.max(0.0);
    let padding = inset(
        border,
        data.desc.padding.left + bw,
        data.desc.padding.top + bw,
        data.desc.padding.right + bw,
        data.desc.padding.bottom + bw,
    );
    let content = inset(
        padding,
        data.desc.padding.left + bw,
        data.desc.padding.top + bw,
        data.desc.padding.right + bw,
        data.desc.padding.bottom + bw,
    );
    let margin = outset(
        border,
        data.desc.margin.left,
        data.desc.margin.top,
        data.desc.margin.right,
        data.desc.margin.bottom,
    );
    InspectorNode {
        id,
        key: data.desc.key,
        source: SourceLoc::from_location(data.source),
        margin,
        border,
        padding,
        content,
    }
}

/// Mirror the live world into the headless analyzer's frame format.
///
/// Ids are the semantic keys, so a diagnostic's `node` maps straight back to a
/// widget without a side table. A zero key gets a private high id: zero is the
/// sanitizers' "no node" sentinel.
#[must_use]
pub fn capture_frame(world: &UiWorld, viewport: Vec2) -> Frame {
    let root = world.root();
    let ids: Vec<NodeId> = world.walk(root).collect();

    // First pass: assign ids. A node without a key gets a private high id
    // because zero is the sanitizers' "no node" sentinel.
    let mut keys: HashMap<NodeId, u64> = HashMap::with_capacity(ids.len());
    let mut synthetic = 1u64 << 63;
    for id in &ids {
        let key = world.node(*id).desc.key;
        let key = if key == 0 {
            synthetic += 1;
            synthetic
        } else {
            key
        };
        keys.insert(*id, key);
    }

    // Second pass: build snapshots. `walk` is pre-order, so a parent's path is
    // always ready before its children ask for it.
    let mut paths: HashMap<NodeId, String> = HashMap::with_capacity(ids.len());
    let mut nodes = Vec::with_capacity(ids.len());
    for id in ids {
        let data = world.node(id);
        let key = keys[&id];
        let parent_key = if id == root {
            None
        } else {
            keys.get(&data.parent).copied()
        };
        let parent_path = paths.get(&data.parent).cloned().unwrap_or_default();
        let path = format!("{parent_path}/{:?}#{key}", data.tag());
        paths.insert(id, path.clone());

        let kind = if world.text(id).is_some() {
            NodeKind::Text {
                ink: data.world_rect,
                ellipsis: data.desc.has(NodeFlags::TEXT_ELLIPSIS),
            }
        } else if matches!(data.tag(), unbound_abi::NodeTag::Portal) {
            NodeKind::Portal
        } else if data.desc.has(NodeFlags::INTERACTIVE) {
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            }
        } else {
            NodeKind::Container
        };

        nodes.push(NodeSnapshot {
            id: key,
            parent: parent_key,
            children: world
                .children(id)
                .filter_map(|child| keys.get(&child).copied())
                .collect(),
            loc: SourceLoc::from_location(data.source),
            path,
            rect: data.world_rect,
            clip: data.clip,
            kind,
            overlays_siblings: false,
            scrollable: data.desc.has(NodeFlags::SCROLL_X) || data.desc.has(NodeFlags::SCROLL_Y),
            text_colors: None,
        });
    }

    Frame::from_nodes(Rect::new(0.0, 0.0, viewport.x, viewport.y), nodes)
}

fn deepest_at(world: &UiWorld, point: Vec2) -> Option<NodeId> {
    let mut best: Option<(u16, NodeId)> = None;
    for id in world.walk(world.root()) {
        let data = world.node(id);
        if data.world_rect.contains_point(point) && best.is_none_or(|(depth, _)| data.depth > depth)
        {
            best = Some((data.depth, id));
        }
    }
    best.map(|(_, id)| id)
}

fn inset(rect: Rect, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    let width = (rect.size.x - left - right).max(0.0);
    let height = (rect.size.y - top - bottom).max(0.0);
    Rect::new(rect.origin.x + left, rect.origin.y + top, width, height)
}

fn outset(rect: Rect, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::new(
        rect.origin.x - left,
        rect.origin.y - top,
        rect.size.x + left + right,
        rect.size.y + top + bottom,
    )
}

fn zone(rect: Rect, token: Token, z: u16, opacity: f32) -> GpuPrimitiveInstance {
    GpuPrimitiveInstance {
        chunk_id: [0, 0],
        local_pos: [rect.origin.x, rect.origin.y],
        size: [rect.size.x, rect.size.y],
        corner_radii: f16::pack_radii([0.0; 4]),
        border_width: f16::f32_to_f16(0.0),
        shadow_blur: f16::f32_to_f16(0.0),
        transform_idx: 0,
        clip_idx: CLIP_NONE,
        fill_token_or_uv: u32::from(token.raw()),
        border_token_or_tex: 0,
        prim_type_and_flags: GpuPrimitiveInstance::pack_type(
            PrimType::RoundedRect,
            unbound_gpu::data::BlendMode::Normal,
            SubElement::Background as u16,
        ),
        z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, opacity),
        extra_payload_idx: 0,
    }
}

/// Four bands forming the ring between `outer` and `inner`.
fn push_ring(
    out: &mut Vec<GpuPrimitiveInstance>,
    outer: Rect,
    inner: Rect,
    thickness: f32,
    token: Token,
    z: u16,
    opacity: f32,
) {
    let t = thickness.max(1.0);
    let top = Rect::new(outer.origin.x, outer.origin.y, outer.size.x, t);
    let bottom = Rect::new(
        outer.origin.x,
        outer.origin.y + outer.size.y - t,
        outer.size.x,
        t,
    );
    let left = Rect::new(
        outer.origin.x,
        inner.origin.y.min(outer.max().y),
        t,
        inner.size.y,
    );
    let right = Rect::new(
        outer.origin.x + outer.size.x - t,
        inner.origin.y.min(outer.max().y),
        t,
        inner.size.y,
    );
    out.extend_from_slice(&[
        zone(top, token, z, opacity),
        zone(bottom, token, z, opacity),
        zone(left, token, z, opacity),
        zone(right, token, z, opacity),
    ]);
}

fn push_box_model(out: &mut Vec<GpuPrimitiveInstance>, node: &InspectorNode) {
    let z = INSPECTOR_Z - 4;
    push_ring(out, node.margin, node.border, 1.0, Token::WARNING, z, 1.0);
    push_ring(
        out,
        node.border,
        node.padding,
        1.0,
        Token::ACCENT,
        z + 1,
        1.0,
    );
    push_ring(
        out,
        node.padding,
        node.content,
        1.0,
        Token::SUCCESS,
        z + 2,
        1.0,
    );
    push_ring(
        out,
        node.content,
        node.content,
        1.0,
        Token::PRIMARY,
        z + 3,
        1.0,
    );
}

fn push_panel(out: &mut Vec<GpuPrimitiveInstance>, viewport: Vec2, lines: &[String]) {
    let width = PANEL_WIDTH.min(viewport.x * 0.45);
    if width < 80.0 || viewport.y <= 0.0 {
        return;
    }
    let x = viewport.x - width;
    let panel = Rect::new(x, 0.0, width, viewport.y);
    out.push(zone(panel, Token::SURFACE_RAISED, INSPECTOR_Z - 20, 1.0));
    out.push(zone(
        Rect::new(x, 0.0, width, PANEL_HEADER),
        Token::ACCENT,
        INSPECTOR_Z - 19,
        1.0,
    ));
    // One stripe per line, so the panel visibly reflects the report even
    // though the host owns the font that draws the text.
    let row = 18.0;
    for (index, _) in lines.iter().enumerate().take(16) {
        let y = PANEL_HEADER + 8.0 + index as f32 * row;
        if y + row > viewport.y {
            break;
        }
        let token = if index == lines.len() - 1 {
            Token::ON_SURFACE
        } else {
            Token::ON_SURFACE_MUTED
        };
        out.push(zone(
            Rect::new(x + 10.0, y + row * 0.5 - 1.0, width - 20.0, 2.0),
            token,
            INSPECTOR_Z - 18,
            0.6,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::panic::Location;
    use rustc_hash::FxHashMap;
    use unbound_abi::{Edges, RawNodeDesc};
    use unbound_math::Rect;

    fn loc() -> &'static Location<'static> {
        Location::caller()
    }

    fn world_with_button() -> (UiWorld, NodeId, NodeId) {
        let mut world = UiWorld::new();
        let root = world.root();
        if let Some(data) = world.get_mut(root) {
            data.desc = RawNodeDesc::container().size(800.0, 600.0).key(1);
        }
        let button = {
            let mut desc = RawNodeDesc::container()
                .size(120.0, 32.0)
                .padding(Edges::all(8.0))
                .border(0, 1.0)
                .margin(Edges::all(4.0))
                .key(42);
            desc.flags |= NodeFlags::INTERACTIVE.bits();
            world.insert(root, desc, loc())
        };
        let mut rects = FxHashMap::default();
        rects.insert(root, Rect::new(0.0, 0.0, 800.0, 600.0));
        rects.insert(button, Rect::new(100.0, 100.0, 120.0, 32.0));
        world.apply_rects(&rects);
        (world, root, button)
    }

    #[test]
    fn picking_resolves_the_hit_stack_to_a_world_node() {
        let (world, _, button) = world_with_button();
        let mut inspector = Inspector::new();
        inspector.set_enabled(true);
        let region = HitRegion::new(42, 1, 1, Rect::new(100.0, 100.0, 120.0, 32.0));
        inspector.update(&world, &[region], Vec2::new(110.0, 110.0));
        let picked = inspector.picked().expect("button picked");
        assert_eq!(picked.id, button);
        assert_eq!(picked.key, 42);
        assert_eq!(
            picked.padding,
            Rect::new(109.0, 109.0, 102.0, 14.0),
            "padding box = border box inset by border + padding"
        );
    }

    #[test]
    fn the_debug_pass_is_emitted_above_the_scene() {
        let (world, _, _) = world_with_button();
        let mut inspector = Inspector::new();
        inspector.set_enabled(true);
        let region = HitRegion::new(42, 1, 1, Rect::new(100.0, 100.0, 120.0, 32.0));
        inspector.update(&world, &[region], Vec2::new(110.0, 110.0));
        let scene = inspector.emit(Vec2::new(800.0, 600.0));
        assert_eq!(scene.layer, PassLayer::Debug);
        assert!(!scene.is_empty());
        assert!(scene
            .primitives
            .iter()
            .all(|p| p.depth_u16() >= INSPECTOR_Z - 20));
        assert!(scene.lines.iter().any(|line| line.contains("key=42")));
    }

    #[test]
    fn diagnostics_become_blinking_error_boxes() {
        let mut inspector = Inspector::new();
        inspector.set_enabled(true);
        inspector.set_diagnostics(&[Diagnostic {
            rule: unbound_headless::Rule::NoNaNOrInf,
            node: 42,
            loc: SourceLoc::default(),
            path: "Root/Button#42".to_owned(),
            rect: Rect::new(10.0, 10.0, 20.0, 20.0),
            chain: vec![1, 42],
            message: "NaN width".to_owned(),
        }]);
        let before = inspector.blink_opacity();
        inspector.tick(0.1);
        assert_ne!(before, inspector.blink_opacity());

        let scene = inspector.emit(Vec2::new(800.0, 600.0));
        assert_eq!(scene.violations.len(), 1);
        assert!(scene.lines.iter().any(|line| line.contains("NoNaNOrInf")));
    }

    #[test]
    fn the_headless_frame_mirror_uses_semantic_keys() {
        let (world, root, button) = world_with_button();
        let frame = capture_frame(&world, Vec2::new(800.0, 600.0));
        assert_eq!(frame.nodes.len(), 2);
        let root_key = world.node(root).desc.key;
        let button_key = world.node(button).desc.key;
        assert!(frame.nodes.iter().any(|node| node.id == root_key));
        let node = frame
            .nodes
            .iter()
            .find(|node| node.id == button_key)
            .expect("button mirrored");
        assert!(node.kind.is_interactive());
        assert_eq!(node.parent, Some(root_key));
    }
}

//! DevTools 检查器及调试循环的运行时侧实现（`docs` §10）。
//!
//! 检查器刻意*不是*一个控件：它直接向调试通道（最高 `z` 值，最后绘制）
//! 的场景收集器绘制，并读取运行时使用的同一份 [`UiWorld`] 与命中区域。
//! 这使其脱离布局树，开关检查器绝不会触发重新布局。
//!
//!
//! 当开启调试检查器时，每帧还会被镜像为 [`unbound_headless::Frame`]
//! 并运行五项检查器。违规项会表现为闪烁的错误框与面板中的文本行。
//!

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

/// 图元 `z` 范围顶部：调试通道绘制在所有内容之上。
pub const INSPECTOR_Z: u16 = u16::MAX;

/// 右侧面板宽度（逻辑像素）。
pub const PANEL_WIDTH: f32 = 300.0;
/// 标题条高度。
pub const PANEL_HEADER: f32 = 26.0;
/// 违规边框闪烁速度（弧度/秒）。
const BLINK_HZ: f32 = 2.0;

/// 从 world 解析出的单帧盒模型。
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorNode {
    pub id: NodeId,
    /// 稳定语义键（`RawNodeDesc::key`）。
    pub key: u64,
    /// 节点构建源码位置。
    pub source: SourceLoc,
    /// 外边距盒（边框盒向外扩展声明的外边距）。
    pub margin: Rect,
    /// 边框盒——节点自身的 `world_rect`。
    pub border: Rect,
    /// 边框盒减去边框宽度。
    pub padding: Rect,
    /// 内边距盒减去内边距。
    pub content: Rect,
}

/// 待绘制的单条检查器违规项。
#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    pub node: u64,
    pub rect: Rect,
    pub rule: &'static str,
    pub message: String,
}

/// 检查器向单帧贡献的调试覆盖层。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorScene {
    /// 始终为 [`PassLayer::Debug`]：调用方在主场景之后追加。
    pub layer: PassLayer,
    pub primitives: Vec<GpuPrimitiveInstance>,
    pub payloads: Vec<GpuPayload>,
    /// 指针悬停时拾取到的节点。
    pub node: Option<InspectorNode>,
    pub violations: Vec<Violation>,
    /// 由 host 绘制在面板内的文本（host 拥有字体）。
    pub lines: Vec<String>,
}

impl InspectorScene {
    /// 无任何调试内容可追加时为 true。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty()
    }
}

/// 检查器状态：启用标志、拾取节点、违规列表与闪烁相位。
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

    /// 切换开关状态；返回新状态。
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

    /// 推进闪烁相位。
    pub fn tick(&mut self, dt: f32) {
        self.blink = (self.blink + dt * BLINK_HZ * core::f32::consts::TAU)
            .rem_euclid(core::f32::consts::TAU);
    }

    /// 闪烁不透明度，范围 `0.35..=1.0`。
    #[must_use]
    pub fn blink_opacity(&self) -> f32 {
        0.35 + 0.325 * (self.blink.sin() + 1.0)
    }

    /// 重新拾取 `pointer` 下的节点。
    ///
    /// 命中栈优先，因为这与点击行为一致；当指针位于非交互间隙时，
    /// 回退取最深层包含节点，使普通容器也能查看盒模型。
    ///
    pub fn update(&mut self, world: &UiWorld, regions: &[HitRegion], pointer: Vec2) {
        let hit = pick_stack(regions, pointer)
            .first()
            .map(|region| region.node);
        let id = hit
            .and_then(|key| world.find_key(key))
            .or_else(|| deepest_at(world, pointer));
        self.pick = id.map(|id| describe(world, id));
    }

    /// 从检查器报告更新违规列表。
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

    /// 清空违规列表（例如关闭检查器时）。
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

    /// 构建本帧的检查器覆盖层。
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

        // 先绘制违规框，使拾取到的盒模型覆盖其上。
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

/// 从 world 解析单个节点的盒模型。
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

/// 将实时 world 镜像为无头分析器的帧格式。
///
/// ID 直接采用语义键，使诊断中的 `node` 无需侧表即可直接映射回控件。
/// 零键会被分配一个私有高位 ID：0 是检查器的“无节点”哨兵。
///
#[must_use]
pub fn capture_frame(world: &UiWorld, viewport: Vec2) -> Frame {
    let root = world.root();
    let ids: Vec<NodeId> = world.walk(root).collect();

    // 第一遍：分配 ID。无键节点分配私有高位 ID，
    // 因为 0 是检查器的“无节点”哨兵。
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

    // 第二遍：构建快照。`walk` 为前序遍历，因此父节点路径在子节点请求前必然就绪。
    //
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

/// 构成 `outer` 与 `inner` 之间环形区域的四个条带。
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
    // 每行一条色带，使面板在视觉上直观反映报告，即便字体绘制由 host 负责。
    //
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

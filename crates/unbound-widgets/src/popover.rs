//! 锚定气泡框与下拉容器。
//!
//! 气泡框由表面（surface）与 [`OverlayEntry`] 组成：几何位置由
//! [`place_anchored_popup`] 计算，确保不会超出窗口；条目则交由
//! [`OverlayStack`] 负责关闭、排序并随触发器一同清理。
//!
//! 内容由调用方输出；本模块负责绘制容器、边框、阴影及（可选的）背景模糊请求。
//!

use unbound_gpu::data::{GpuClipNode, GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{HitRegion, PORTAL_LAYER};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::overlay::{OverlayEntry, PlacedPopup, Placement};
use crate::primitive::{instance, surface};
use crate::{ControlState, SceneCollector, WidgetDesc, WidgetOutput};

/// 气泡框的命中测试层：Portal 层级。
pub const POPOVER_LAYER: u8 = PORTAL_LAYER;
/// 气泡框阴影模糊半径。
const POPOVER_SHADOW: f32 = 24.0;
/// 箭头尖端深度与底边半宽。
const ARROW_SIZE: f32 = 8.0;
/// 箭头描边宽度。
const ARROW_STROKE: f32 = 2.0;

/// 锚定到触发器的气泡框描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PopoverDesc {
    /// 触发器矩形。
    pub anchor: Rect,
    /// 气泡框必须保持在内的视口矩形。
    pub viewport: Rect,
    /// 气泡框的首选尺寸。
    pub size: Vec2,
    /// 首选放置方向。
    pub placement: Placement,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制顺序键。
    pub z: u16,
    /// 气泡框背后的背景模糊半径；`0.0` 表示禁用该 Pass。
    pub backdrop_blur: f32,
    /// 是否绘制指向锚点的箭头。
    pub show_arrow: bool,
}

impl PopoverDesc {
    #[must_use]
    pub fn new(anchor: Rect, viewport: Rect, size: Vec2) -> Self {
        Self {
            anchor,
            viewport,
            size,
            placement: Placement::BottomStart,
            radius: 10.0,
            z: 900,
            backdrop_blur: 0.0,
            show_arrow: false,
        }
    }

    /// 已放置的弹窗信息，包含选定方向与箭头尖端。
    #[must_use]
    pub fn placed(&self) -> PlacedPopup {
        crate::overlay::place_anchored_popup_detailed(
            self.anchor,
            self.size,
            self.viewport,
            self.placement,
        )
    }

    /// 已放置的气泡框矩形。
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.placed().rect
    }
}

/// 绑定了节点标识的气泡框描述符。
#[derive(Clone, Debug)]
pub struct PopoverWidget {
    /// 气泡框自身的节点 ID。
    pub node: u64,
    /// 触发器的节点 ID。
    pub anchor_node: u64,
    /// 气泡框描述符。
    pub desc: PopoverDesc,
}

impl WidgetDesc for PopoverWidget {
    fn rect(&self) -> Rect {
        self.desc.rect()
    }

    fn key(&self) -> u64 {
        self.node
    }

    fn emit(&self, state: ControlState, _glyphs: &mut [unbound_text::GlyphQuad]) -> WidgetOutput {
        emit_popover(self.node, self.anchor_node, &self.desc, state).output
    }
}

/// 单个气泡框的输出结果。
#[derive(Clone, Debug)]
pub struct PopoverOutput {
    /// 绘制的容器及其命中区域。
    pub output: WidgetOutput,
    /// 调用方需推入栈的浮层条目。
    pub entry: OverlayEntry,
    /// 当 `backdrop_blur > 0` 时的模糊请求。
    pub backdrop: Option<crate::overlay::BackdropRequest>,
}

/// 输出气泡框容器。
#[must_use]
pub fn emit_popover(
    node: u64,
    anchor_node: u64,
    desc: &PopoverDesc,
    state: ControlState,
) -> PopoverOutput {
    let placed = desc.placed();
    let rect = placed.rect;
    let radii = [desc.radius; 4];
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![
        instance(
            rect,
            radii,
            Token::NONE,
            Token::NONE,
            0.0,
            POPOVER_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            desc.z,
        ),
        surface(rect, desc.radius, Token::OVERLAY_BG, desc.z),
        instance(
            rect,
            radii,
            Token::NONE,
            Token::OUTLINE,
            1.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Border,
            desc.z,
        ),
    ];
    let mut payloads = Vec::new();
    if desc.show_arrow {
        arrow_edges(
            placed.chosen_placement,
            placed.arrow_tip,
            desc.z,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, POPOVER_LAYER, u32::from(desc.z), rect)
            .with_shape(unbound_input::Shape::Rounded { radii })
    });
    let a11y = unbound_a11y::A11yNode::new(accesskit::Role::GenericContainer, "", rect)
        .disabled(state.disabled);

    PopoverOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        entry: OverlayEntry::popover(node, anchor_node, rect),
        backdrop: crate::overlay::BackdropRequest::new(node, rect, desc.backdrop_blur),
    }
}

/// 在同一作用域内输出气泡框及其子元素。
///
/// 先绘制容器，再在气泡框裁剪区域内收集子元素并提升至 [`POPOVER_LAYER`]，
/// 使其在命中测试中优先于 Portal 下方的界面。
pub fn emit_popover_scope<I>(
    collector: &mut SceneCollector,
    node: u64,
    anchor_node: u64,
    desc: &PopoverDesc,
    state: ControlState,
    children: I,
) -> PopoverOutput
where
    I: IntoIterator<Item = (u64, WidgetOutput)>,
{
    let output = emit_popover(node, anchor_node, desc, state);
    if let Some(backdrop) = output.backdrop {
        collector.backdrops.push(backdrop);
    }
    let rect = output.entry.bounds;
    collector.push(node, output.output.clone());

    let clip = GpuClipNode::from_aabb([rect.min().x, rect.min().y, rect.max().x, rect.max().y]);
    collector.push_clip(clip, rect);
    for (key, mut child) in children {
        if let Some(hit) = child.hit.as_mut() {
            hit.layer = POPOVER_LAYER;
        }
        collector.push(key, child);
    }
    collector.pop_clip();
    output
}

/// 构成指向锚点箭头的两条二次曲线描边。
fn arrow_edges(
    placement: Placement,
    tip: Vec2,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    // 从弹窗边缘指向弹窗内部的方向。
    let inward = match placement {
        Placement::BottomStart | Placement::BottomEnd | Placement::Bottom => Vec2::new(0.0, 1.0),
        Placement::TopStart | Placement::TopEnd | Placement::Top => Vec2::new(0.0, -1.0),
        Placement::RightStart | Placement::RightEnd | Placement::Right => Vec2::new(1.0, 0.0),
        Placement::LeftStart | Placement::LeftEnd | Placement::Left => Vec2::new(-1.0, 0.0),
    };
    let base = tip + inward * ARROW_SIZE;
    let half = Vec2::new(-inward.y, inward.x) * ARROW_SIZE;
    for corner in [base + half, base - half] {
        curve_stroke(
            [tip, (tip + corner) * 0.5, corner],
            ARROW_STROKE,
            Token::OUTLINE,
            z,
            primitives,
            payloads,
        );
    }
}

/// 打包到紧凑局部包围盒中的单条二次曲线描边。
fn curve_stroke(
    points: [Vec2; 3],
    width: f32,
    color: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let local = points.map(|point| [point.x - min_x, point.y - min_y]);
    let payload_idx = payloads.len() as u32;
    payloads.push(GpuPayload::curve(local[0], local[1], local[2], width));
    let mut primitive = instance(
        Rect::new(
            min_x,
            min_y,
            (max_x - min_x).max(0.5),
            (max_y - min_y).max(0.5),
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_popover_stays_on_screen_and_layers_above_the_ui() {
        let desc = PopoverDesc::new(
            Rect::new(1200.0, 680.0, 40.0, 30.0),
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            Vec2::new(240.0, 200.0),
        );
        let output = emit_popover(1, 2, &desc, ControlState::default());
        assert!(desc.viewport.contains_rect(&output.entry.bounds));
        assert_eq!(output.output.hit.as_ref().unwrap().layer, POPOVER_LAYER);
        assert_eq!(output.output.primitives.len(), 3);
    }
}

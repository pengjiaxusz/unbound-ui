//! 变换操纵器（TransformGizmo）：八个缩放手柄、一个旋转手柄与实时尺寸气泡。
//!
//! 该操纵器按契约处于*屏幕空间*：无论画布如何缩放，其边框与手柄始终保持固定像素大小，
//! 因此绘制前需先按调用方的缩放倍率转换边界。每个手柄均上报各自带有对应缩放光标的命中区域，
//! 从而可从任意手柄发起拖拽。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_canvas::{Handle, SnapGuide};
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 手柄正方形尺寸。
pub const GRIP_SIZE: f32 = 8.0;
/// 旋转手柄直径。
pub const ROTATE_GRIP: f32 = 10.0;
/// 顶边与旋转手柄之间的间距。
pub const ROTATE_OFFSET: f32 = 22.0;
/// 尺寸气泡高度。
pub const SIZE_BUBBLE: f32 = 20.0;

/// 操纵器描述符。`bounds` 位于**屏幕**空间。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoDesc {
    /// 选区边界，已投影至屏幕空间。
    pub bounds: Rect,
    /// 画布缩放倍率，用于保持描边为 1 像素宽。
    pub zoom: f32,
    /// 旋转手柄偏移距离。
    pub rotate_offset: f32,
    /// 是否绘制尺寸气泡。
    pub show_size: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl GizmoDesc {
    #[must_use]
    pub fn new(bounds: Rect, zoom: f32) -> Self {
        Self {
            bounds,
            zoom: if zoom > 0.0 { zoom } else { 1.0 },
            rotate_offset: ROTATE_OFFSET,
            show_size: true,
            z: 900,
        }
    }

    /// 隐藏尺寸气泡的同款操纵器。
    #[must_use]
    pub fn without_size(mut self) -> Self {
        self.show_size = false;
        self
    }

    /// 屏幕像素下恒定的细线宽度。
    #[must_use]
    pub fn line_width(&self) -> f32 {
        (1.0 / self.zoom).clamp(0.5, 2.0)
    }

    /// 静息状态下单个手柄的矩形区域。
    #[must_use]
    pub fn grip_rect(&self, handle: Handle) -> Rect {
        grip_rect_for(self.bounds, self.rotate_offset, handle)
    }

    /// 在 `radius` 范围内查找 `point` 命中的手柄。
    #[must_use]
    pub fn grip_at(&self, point: Vec2, radius: f32) -> Option<Handle> {
        Handle::hit(self.bounds, point, radius, self.rotate_offset)
    }

    /// 尺寸气泡矩形。
    #[must_use]
    pub fn size_bubble_rect(&self, width: f32) -> Rect {
        Rect::new(
            self.bounds.center().x - width * 0.5,
            self.bounds.max().y + 12.0,
            width,
            SIZE_BUBBLE,
        )
    }
}

/// `bounds` 周围 `handle` 对应的手柄矩形。
#[must_use]
pub fn grip_rect_for(bounds: Rect, rotate_offset: f32, handle: Handle) -> Rect {
    let centre = handle.position(bounds, rotate_offset);
    let size = if handle == Handle::Rotate {
        ROTATE_GRIP
    } else {
        GRIP_SIZE
    };
    Rect::new(centre.x - size * 0.5, centre.y - size * 0.5, size, size)
}

/// 单个操纵器的发射结果。
#[derive(Clone, Debug)]
pub struct GizmoOutput {
    /// 轮廓线、手柄、辅助线与气泡。
    pub output: WidgetOutput,
    /// 按 [`Handle::ALL`] 顺序排列的各手柄命中区域。
    pub hits: Vec<(Handle, HitRegion)>,
    /// 尺寸气泡（若绘制）。
    pub bubble: Option<Rect>,
}

/// 发射操纵器、当前拖拽预览与尺寸气泡。
///
/// `preview` 为来自调用方 [`unbound_canvas::HandleDrag`] 的 `(handle, bounds)`；
/// 存在时轮廓线跟随预览变化，且被拖拽的手柄高亮显示。
#[must_use]
pub fn emit_gizmo(
    node: u64,
    desc: &GizmoDesc,
    preview: Option<(Handle, Rect)>,
    guides: &[SnapGuide],
    size_glyphs: &[GlyphQuad],
    control: ControlState,
) -> GizmoOutput {
    let width = desc.line_width();
    let bounds = preview.map_or(desc.bounds, |(_, bounds)| bounds);
    let mut primitives: Vec<GpuPrimitiveInstance> = vec![instance(
        bounds,
        [0.0; 4],
        Token::NONE,
        Token::PRIMARY,
        width,
        0.0,
        PrimType::Stroke,
        SubElement::FocusRing,
        desc.z,
    )];

    for guide in guides {
        let rect = match guide.axis {
            unbound_canvas::Axis::X => Rect::new(
                guide.coordinate - 0.5,
                guide.start,
                1.0,
                (guide.end - guide.start).max(1.0),
            ),
            unbound_canvas::Axis::Y => Rect::new(
                guide.start,
                guide.coordinate - 0.5,
                (guide.end - guide.start).max(1.0),
                1.0,
            ),
        };
        primitives.push(surface(rect, 0.0, Token::SNAP_GUIDE, desc.z + 1));
    }

    let mut hits = Vec::with_capacity(Handle::ALL.len());
    for handle in Handle::ALL {
        let grip = grip_rect_for(bounds, desc.rotate_offset, handle);
        let radius = grip.size.x * 0.5;
        primitives.push(surface(grip, radius, Token::SURFACE_RAISED, desc.z + 1));
        let ink = if preview.is_some_and(|(active, _)| active == handle) {
            Token::PRIMARY
        } else {
            Token::OUTLINE_STRONG
        };
        primitives.push(surface(grip.inflate(width), radius + width, ink, desc.z));
        // 东南角手柄始终预留焦点环槽位；聚焦仅改变其不透明度，使图元数量与状态无关。
        if handle == Handle::SouthEast {
            primitives.push(focus_ring(
                grip.inflate(3.0),
                6.0,
                Token::FOCUS_RING,
                2.0,
                desc.z + 2,
                control.focused,
            ));
        }
        hits.push((
            handle,
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), grip.inflate(2.0))
                .with_cursor(CursorKind::for_handle(handle)),
        ));
    }

    let mut payloads = Vec::new();
    let bubble = desc.show_size.then(|| {
        let rect = desc.size_bubble_rect(96.0);
        primitives.push(surface(rect, 6.0, Token::HUD_BG, desc.z + 2));
        glyphs(
            size_glyphs,
            Token::HUD_TEXT,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
        rect
    });

    let mut a11y = A11yNode::new(Role::Group, "变换手柄", bounds).clickable();
    a11y.value = format!("{:.0} × {:.0}", bounds.size.x, bounds.size.y);

    GizmoOutput {
        output: WidgetOutput {
            primitives,
            payloads,
            hit: None,
            a11y: Some(a11y),
            focusable: false,
            ..Default::default()
        },
        hits,
        bubble,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_canvas::{Axis, HandleDrag, Modifiers, SnapKind};

    fn desc() -> GizmoDesc {
        GizmoDesc::new(Rect::new(100.0, 100.0, 200.0, 100.0), 2.0)
    }

    #[test]
    fn grips_sit_on_the_eight_edges_and_the_rotation_grip_above() {
        let desc = desc();
        assert_eq!(
            desc.grip_rect(Handle::NorthWest).origin,
            Vec2::new(96.0, 96.0)
        );
        assert_eq!(
            desc.grip_rect(Handle::SouthEast).origin,
            Vec2::new(296.0, 196.0)
        );
        let rotate = desc.grip_rect(Handle::Rotate);
        assert!(rotate.center().y < desc.bounds.origin.y);
        assert_eq!(rotate.center().x, desc.bounds.center().x);
    }

    #[test]
    fn the_hairline_is_screen_constant() {
        assert_eq!(desc().line_width(), 0.5);
        assert_eq!(
            GizmoDesc::new(Rect::new(0.0, 0.0, 1.0, 1.0), 1.0).line_width(),
            1.0
        );
    }

    #[test]
    fn a_grip_is_hit_within_its_radius() {
        let desc = desc();
        let centre = Handle::East.position(desc.bounds, desc.rotate_offset);
        assert_eq!(
            desc.grip_at(centre + Vec2::new(2.0, 0.0), 8.0),
            Some(Handle::East)
        );
        assert!(desc.grip_at(centre + Vec2::new(40.0, 40.0), 8.0).is_none());
    }

    #[test]
    fn a_drag_preview_moves_the_outline_and_the_bubble() {
        let desc = desc();
        let drag = HandleDrag::new(Handle::SouthEast, desc.bounds, desc.bounds.max(), 0.0);
        let preview = drag.preview(desc.bounds.max() + Vec2::new(20.0, 10.0), Modifiers::NONE);
        let guides = [SnapGuide {
            axis: Axis::X,
            coordinate: 320.0,
            start: 0.0,
            end: 500.0,
            kind: SnapKind::Right,
        }];
        let output = emit_gizmo(
            4,
            &desc,
            Some((Handle::SouthEast, preview)),
            &guides,
            &[],
            ControlState::default(),
        );
        assert_eq!(output.hits.len(), Handle::ALL.len());
        assert!(output.bubble.is_some());
        let outline = &output.output.primitives[0];
        assert_eq!(outline.size, [preview.size.x, preview.size.y]);
    }

    #[test]
    fn every_grip_carries_the_right_cursor() {
        let output = emit_gizmo(4, &desc(), None, &[], &[], ControlState::default());
        for (handle, hit) in &output.hits {
            assert_eq!(hit.cursor, Some(CursorKind::for_handle(*handle)));
        }
    }

    #[test]
    fn the_gizmo_announces_the_selection_size() {
        let output = emit_gizmo(4, &desc(), None, &[], &[], ControlState::default());
        let a11y = output.output.a11y.expect("gizmo");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.value, "200 × 100");
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        let output = emit_gizmo(4, &desc, None, &[], &[], control);
                        counts.push(output.output.primitives.len());
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

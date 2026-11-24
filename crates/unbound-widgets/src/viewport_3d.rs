//! 3D 视口控件：面向离线或实时 3D 渲染目标的纹理承载、交互手势追踪与状态机发射器。
//!
//! 本模块提供：
//! - [`Viewport3DDesc`]：声明 3D 视口尺寸、圆角、边框、渲染缩放与绑定纹理槽位；
//! - [`emit_viewport_3d`]：在所有 24 种 [`ControlState`] 组合下发射恒定 3 个图元
//!   （内容表面/占位表面、边框描边、焦点环），确保悬停与聚焦仅触发局部字段更新；
//! - [`Viewport3DPointerTracker`]：将左键轨道旋转（Orbit）、右键/Shift+左键平移（Pan）
//!   与滚轮缩放（Zoom）统一归一化为逻辑像素增量事件 [`Viewport3DGestureEvent`]。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
pub use unbound_gpu::{
    clamp_render_scale, compute_viewport_physical_extent, ViewportCamera3D,
    ViewportCameraConstraints, ViewportId,
};
use unbound_input::{CursorKind, HitRegion, PointerButton, Shape};
use unbound_math::{f16, Rect, Vec2};
use unbound_theme::Token;

use crate::primitive::{focus_ring, instance};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 3D 视口默认圆角半径（对齐 WinUI `OverlayCornerRadius = 8px`）。
pub const VIEWPORT_3D_RADIUS: f32 = 8.0;

/// 3D 视口默认绘制顺序键（与 [`crate::image::ImageDesc`] 对齐）。
pub const VIEWPORT_3D_DEFAULT_Z: u16 = 690;

/// 3D 视口交互手势增量（已按 DPI 缩放归一化到逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Viewport3DGesture {
    /// 轨道相机旋转增量（逻辑像素）。
    OrbitDelta { dx: f32, dy: f32 },
    /// 视口平面平移增量（逻辑像素）。
    PanDelta { dx: f32, dy: f32 },
    /// 滚轮缩放增量（逻辑像素）。
    ZoomDelta { delta: f32 },
}

/// 绑定了目标 [`ViewportId`] 的 3D 视口手势事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport3DGestureEvent {
    /// 目标视口标识。
    pub viewport_id: ViewportId,
    /// 本次交互产生的手势增量。
    pub gesture: Viewport3DGesture,
}

/// 3D 视口当前激活的指针拖拽模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewport3DDragMode {
    /// 轨道旋转模式（左键拖拽）。
    Orbit,
    /// 平移模式（右键拖拽或 Shift+左键拖拽）。
    Pan,
}

/// 正在进行中的指针拖拽捕获状态。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ActiveDrag {
    viewport_id: ViewportId,
    button: PointerButton,
    mode: Viewport3DDragMode,
    last_pos: Vec2,
}

/// 将 `dpi_scale` 规范化为安全正有限值，非法输入回退到 `1.0`。
#[inline]
fn normalize_dpi_scale(dpi_scale: f32) -> f32 {
    if dpi_scale.is_finite() && dpi_scale > 0.0 {
        dpi_scale
    } else {
        1.0
    }
}

/// 3D 视口指针捕获与手势增量追踪器。
#[derive(Debug, Clone, Default)]
pub struct Viewport3DPointerTracker {
    active: Option<ActiveDrag>,
}

impl Viewport3DPointerTracker {
    /// 创建空闲状态的手势追踪器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 处理指针按下事件并尝试捕获目标视口。
    ///
    /// - 左键且未按住 Shift：进入 [`Viewport3DDragMode::Orbit`]；
    /// - 右键，或左键且按住 Shift：进入 [`Viewport3DDragMode::Pan`]；
    /// - 其他按键不触发捕获并返回 `false`。
    pub fn on_pointer_down(
        &mut self,
        viewport_id: ViewportId,
        button: PointerButton,
        shift_held: bool,
        pos: Vec2,
    ) -> bool {
        let mode = match button {
            PointerButton::Left if !shift_held => Viewport3DDragMode::Orbit,
            PointerButton::Left | PointerButton::Right => Viewport3DDragMode::Pan,
            _ => return false,
        };
        self.active = Some(ActiveDrag {
            viewport_id,
            button,
            mode,
            last_pos: pos,
        });
        true
    }

    /// 处理指针移动事件，将位移按 `dpi_scale` 归一化后输出手势增量事件。
    ///
    /// 未处于捕获态、位移为零或包含非有限值时返回 `None`。
    pub fn on_pointer_move(&mut self, pos: Vec2, dpi_scale: f32) -> Option<Viewport3DGestureEvent> {
        let active = self.active.as_mut()?;
        let scale = normalize_dpi_scale(dpi_scale);
        let dx = (pos.x - active.last_pos.x) / scale;
        let dy = (pos.y - active.last_pos.y) / scale;
        active.last_pos = pos;
        if !dx.is_finite() || !dy.is_finite() || (dx == 0.0 && dy == 0.0) {
            return None;
        }
        let gesture = match active.mode {
            Viewport3DDragMode::Orbit => Viewport3DGesture::OrbitDelta { dx, dy },
            Viewport3DDragMode::Pan => Viewport3DGesture::PanDelta { dx, dy },
        };
        Some(Viewport3DGestureEvent {
            viewport_id: active.viewport_id,
            gesture,
        })
    }

    /// 处理指针抬起事件；仅当按键与当前捕获按键一致时释放捕获并返回 `true`。
    pub fn on_pointer_up(&mut self, button: PointerButton) -> bool {
        if self.active.is_some_and(|drag| drag.button == button) {
            self.active = None;
            true
        } else {
            false
        }
    }

    /// 指针捕获被系统或窗口失焦打断时清空当前拖拽状态。
    pub fn on_capture_lost(&mut self) {
        self.active = None;
    }

    /// 将滚轮垂直增量归一化为 [`Viewport3DGesture::ZoomDelta`] 事件。
    ///
    /// 增量为零或非有限值时返回 `None`。
    #[must_use]
    pub fn on_wheel(
        viewport_id: ViewportId,
        delta_y: f32,
        dpi_scale: f32,
    ) -> Option<Viewport3DGestureEvent> {
        let scale = normalize_dpi_scale(dpi_scale);
        let delta = delta_y / scale;
        if !delta.is_finite() || delta == 0.0 {
            return None;
        }
        Some(Viewport3DGestureEvent {
            viewport_id,
            gesture: Viewport3DGesture::ZoomDelta { delta },
        })
    }

    /// 返回当前被指针捕获的视口标识（如有）。
    #[must_use]
    pub fn active_viewport(&self) -> Option<ViewportId> {
        self.active.map(|drag| drag.viewport_id)
    }

    /// 返回当前激活的拖拽模式（如有）。
    #[must_use]
    pub fn active_mode(&self) -> Option<Viewport3DDragMode> {
        self.active.map(|drag| drag.mode)
    }
}

/// 3D 视口控件描述符。
#[derive(Debug, Clone, PartialEq)]
pub struct Viewport3DDesc<'a> {
    /// 视口唯一标识，用于关联外部渲染目标与相机手势。
    pub viewport_id: ViewportId,
    /// 期望布局尺寸（逻辑像素）。
    pub size: Vec2,
    /// 四角独立圆角半径：左上、右上、右下、左下。
    pub radius: [f32; 4],
    /// 外框描边宽度（像素）。
    pub border_width: f32,
    /// 空闲态边框颜色令牌。
    pub border_color: Token,
    /// 绑定纹理时的着色令牌；[`Token::NONE`] 表示保持原色。
    pub tint: Token,
    /// 整体不透明度（`0.0..=1.0`）。
    pub opacity: f32,
    /// 离屏渲染缩放倍率。
    pub render_scale: f32,
    /// 未绑定纹理槽位时的占位背景令牌。
    pub placeholder_color: Token,
    /// 可选外部纹理句柄回退标识。
    pub fallback_external_texture: Option<u64>,
    /// 已注册到无绑定纹理表的槽位索引；`None` 时渲染占位表面。
    pub texture_slot: Option<u32>,
    /// 是否接受指针交互与悬停高亮。
    pub interactive: bool,
    /// 无障碍替代描述文本。
    pub alt: &'a str,
    /// 基础绘制顺序键。
    pub z: u16,
}

impl Default for Viewport3DDesc<'_> {
    fn default() -> Self {
        Self::new(ViewportId::new(1))
    }
}

impl<'a> Viewport3DDesc<'a> {
    /// 使用默认视觉参数创建指定 [`ViewportId`] 的 3D 视口描述符。
    #[must_use]
    pub fn new(viewport_id: ViewportId) -> Self {
        Self {
            viewport_id,
            size: Vec2::new(320.0, 200.0),
            radius: [VIEWPORT_3D_RADIUS; 4],
            border_width: 1.0,
            border_color: Token::OUTLINE,
            tint: Token::NONE,
            opacity: 1.0,
            render_scale: 1.0,
            placeholder_color: Token::SURFACE_SUNKEN,
            fallback_external_texture: None,
            texture_slot: None,
            interactive: true,
            alt: "",
            z: VIEWPORT_3D_DEFAULT_Z,
        }
    }

    /// 设置期望视口尺寸。
    #[must_use]
    pub fn with_size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    /// 设置四角圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: [f32; 4]) -> Self {
        self.radius = radius;
        self
    }

    /// 设置边框宽度与颜色令牌。
    #[must_use]
    pub fn with_border(mut self, width: f32, color: Token) -> Self {
        self.border_width = width;
        self.border_color = color;
        self
    }

    /// 设置纹理着色令牌。
    #[must_use]
    pub fn with_tint(mut self, tint: Token) -> Self {
        self.tint = tint;
        self
    }

    /// 设置不透明度（钳制到 `0.0..=1.0`）。
    #[must_use]
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self
    }

    /// 设置离屏渲染缩放倍率（经 [`clamp_render_scale`] 钳制）。
    #[must_use]
    pub fn with_render_scale(mut self, render_scale: f32) -> Self {
        self.render_scale = clamp_render_scale(render_scale);
        self
    }

    /// 设置未绑定纹理时的占位背景颜色令牌。
    #[must_use]
    pub fn with_placeholder_color(mut self, color: Token) -> Self {
        self.placeholder_color = color;
        self
    }

    /// 设置可选外部纹理句柄回退值。
    #[must_use]
    pub fn with_fallback_external_texture(mut self, texture: Option<u64>) -> Self {
        self.fallback_external_texture = texture;
        self
    }

    /// 绑定无绑定纹理表槽位索引。
    #[must_use]
    pub fn with_texture_slot(mut self, slot: Option<u32>) -> Self {
        self.texture_slot = slot;
        self
    }

    /// 设置是否开启指针交互与悬停反馈。
    #[must_use]
    pub fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// 设置无障碍替代描述文本。
    #[must_use]
    pub fn with_alt(mut self, alt: &'a str) -> Self {
        self.alt = alt;
        self
    }

    /// 设置基础绘制顺序键。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }
}

/// 发射 3D 视口控件的图元、命中区域与无障碍节点。
///
/// 退化矩形（宽高 `<= 0.0` 或包含非有限值）直接返回空输出；
/// 其余情形在全部 24 种 [`ControlState`] 组合下恒定输出 3 个图元：
/// 1. 绑定纹理图像或占位圆角背景；
/// 2. 响应悬停/按下/禁用状态的边框描边；
/// 3. 键盘焦点环（通过不透明度显隐）。
#[must_use]
pub fn emit_viewport_3d(
    node: u64,
    rect: Rect,
    desc: &Viewport3DDesc<'_>,
    state: ControlState,
) -> WidgetOutput {
    let valid_rect = rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.x.is_finite()
        && rect.size.y.is_finite()
        && rect.size.x > 0.0
        && rect.size.y > 0.0;
    if !valid_rect {
        return WidgetOutput::default();
    }

    let opacity = if desc.opacity.is_finite() {
        desc.opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let z_content = desc.z;
    let z_border = desc.z.saturating_add(1);
    let z_focus = desc.z.saturating_add(2);

    let mut primitives = Vec::with_capacity(3);
    let mut payloads = Vec::with_capacity(1);

    // 1. 内容层：已绑定纹理槽位时发射 `PrimType::Image`，否则发射占位圆角表面。
    if let Some(slot) = desc.texture_slot {
        let mut content = instance(
            rect,
            desc.radius,
            desc.tint,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Image,
            SubElement::Content,
            z_content,
        );
        content.border_token_or_tex = slot;
        content.extra_payload_idx = 0;
        content.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z_content, opacity);
        primitives.push(content);
        payloads.push(GpuPayload {
            p0: [0.0, 0.0],
            p1: [1.0, 1.0],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
    } else {
        let mut placeholder = instance(
            rect,
            desc.radius,
            desc.placeholder_color,
            Token::NONE,
            0.0,
            0.0,
            PrimType::RoundedRect,
            SubElement::Background,
            z_content,
        );
        placeholder.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z_content, opacity);
        primitives.push(placeholder);
        payloads.push(GpuPayload {
            p0: [0.0; 2],
            p1: [0.0; 2],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        });
    }

    // 2. 边框层：交互启用时根据指针阶段切换令牌，保证悬停态可观测且图元数恒定。
    let border_token = if state.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else if desc.interactive {
        match state.phase {
            PointerPhase::Hovered => Token::OUTLINE_STRONG,
            PointerPhase::Pressed => Token::ACCENT,
            PointerPhase::Idle => desc.border_color,
        }
    } else {
        desc.border_color
    };
    let mut border = instance(
        rect,
        desc.radius,
        Token::NONE,
        border_token,
        desc.border_width.max(0.0),
        0.0,
        PrimType::RoundedRect,
        SubElement::Border,
        z_border,
    );
    border.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(z_border, opacity);
    primitives.push(border);

    // 3. 焦点环：始终发射，仅按聚焦状态切换不透明度。
    let mut ring = focus_ring(
        rect,
        desc.radius[0],
        Token::FOCUS_RING,
        2.0,
        z_focus,
        state.focused && !state.disabled,
    );
    ring.corner_radii = f16::pack_radii(desc.radius);
    primitives.push(ring);

    let hit = (desc.interactive && !state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: desc.radius })
            .with_cursor(CursorKind::Grab)
    });

    let mut a11y = A11yNode::new(Role::Image, desc.alt.to_owned(), rect).disabled(state.disabled);
    if desc.interactive {
        a11y = a11y.clickable();
    }

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: desc.interactive && !state.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_drag_emits_orbit_delta_with_dpi_normalization() {
        let vp = ViewportId::new(7);
        let mut tracker = Viewport3DPointerTracker::new();
        assert!(tracker.on_pointer_down(vp, PointerButton::Left, false, Vec2::new(10.0, 20.0)));
        assert_eq!(tracker.active_viewport(), Some(vp));
        assert_eq!(tracker.active_mode(), Some(Viewport3DDragMode::Orbit));

        let ev = tracker
            .on_pointer_move(Vec2::new(30.0, 50.0), 2.0)
            .expect("left drag must emit orbit delta");
        assert_eq!(
            ev,
            Viewport3DGestureEvent {
                viewport_id: vp,
                gesture: Viewport3DGesture::OrbitDelta { dx: 10.0, dy: 15.0 },
            }
        );

        // 位移为零时不发射冗余事件。
        assert!(tracker.on_pointer_move(Vec2::new(30.0, 50.0), 2.0).is_none());
    }

    #[test]
    fn right_drag_and_shift_left_drag_emit_pan_delta() {
        let vp = ViewportId::new(9);
        let mut tracker = Viewport3DPointerTracker::new();

        assert!(tracker.on_pointer_down(vp, PointerButton::Right, false, Vec2::new(100.0, 100.0)));
        assert_eq!(tracker.active_mode(), Some(Viewport3DDragMode::Pan));
        let ev = tracker
            .on_pointer_move(Vec2::new(112.0, 88.0), 1.5)
            .expect("right drag must emit pan delta");
        assert_eq!(
            ev,
            Viewport3DGestureEvent {
                viewport_id: vp,
                gesture: Viewport3DGesture::PanDelta { dx: 8.0, dy: -8.0 },
            }
        );
        assert!(tracker.on_pointer_up(PointerButton::Right));
        assert_eq!(tracker.active_viewport(), None);

        assert!(tracker.on_pointer_down(vp, PointerButton::Left, true, Vec2::new(0.0, 0.0)));
        assert_eq!(tracker.active_mode(), Some(Viewport3DDragMode::Pan));
        let ev = tracker
            .on_pointer_move(Vec2::new(-4.0, 6.0), 1.0)
            .expect("shift+left drag must emit pan delta");
        assert_eq!(
            ev,
            Viewport3DGestureEvent {
                viewport_id: vp,
                gesture: Viewport3DGesture::PanDelta { dx: -4.0, dy: 6.0 },
            }
        );
    }

    #[test]
    fn wheel_and_capture_lifecycle_handle_edge_cases() {
        let vp = ViewportId::new(3);
        let mut tracker = Viewport3DPointerTracker::new();

        // 中键不触发捕获。
        assert!(!tracker.on_pointer_down(vp, PointerButton::Middle, false, Vec2::ZERO));
        assert_eq!(tracker.active_viewport(), None);

        assert!(tracker.on_pointer_down(vp, PointerButton::Left, false, Vec2::new(5.0, 5.0)));
        // 非匹配按键抬起不得释放捕获。
        assert!(!tracker.on_pointer_up(PointerButton::Right));
        assert_eq!(tracker.active_viewport(), Some(vp));

        // 捕获丢失必须清空状态。
        tracker.on_capture_lost();
        assert_eq!(tracker.active_viewport(), None);
        assert_eq!(tracker.active_mode(), None);
        assert!(tracker.on_pointer_move(Vec2::new(20.0, 20.0), 1.0).is_none());

        // 滚轮按 DPI 缩放归一化，零值与非有限值返回 None。
        let zoom = Viewport3DPointerTracker::on_wheel(vp, -120.0, 2.0)
            .expect("non-zero wheel delta must emit zoom event");
        assert_eq!(
            zoom,
            Viewport3DGestureEvent {
                viewport_id: vp,
                gesture: Viewport3DGesture::ZoomDelta { delta: -60.0 },
            }
        );
        assert!(Viewport3DPointerTracker::on_wheel(vp, 0.0, 1.0).is_none());
        assert!(Viewport3DPointerTracker::on_wheel(vp, f32::NAN, 1.0).is_none());
        let fallback_scale = Viewport3DPointerTracker::on_wheel(vp, 24.0, 0.0)
            .expect("non-positive dpi scale must fall back to 1.0");
        assert_eq!(
            fallback_scale.gesture,
            Viewport3DGesture::ZoomDelta { delta: 24.0 }
        );
    }

    #[test]
    fn degenerate_rect_emits_no_primitives_or_hit_region() {
        let desc = Viewport3DDesc::new(ViewportId::new(1));
        for rect in [
            Rect::new(0.0, 0.0, 0.0, 100.0),
            Rect::new(0.0, 0.0, 100.0, 0.0),
            Rect::new(0.0, 0.0, -10.0, 100.0),
            Rect::new(0.0, 0.0, f32::NAN, 100.0),
        ] {
            let out = emit_viewport_3d(1, rect, &desc, ControlState::default());
            assert!(out.primitives.is_empty());
            assert!(out.hit.is_none());
            assert!(out.sub_hits.is_empty());
        }
    }

    #[test]
    fn placeholder_and_bound_texture_keep_constant_shape_and_hover_border_transition() {
        let rect = Rect::new(16.0, 24.0, 320.0, 200.0);
        let placeholder_desc = Viewport3DDesc::new(ViewportId::new(2)).with_alt("Scene preview");
        let textured_desc = placeholder_desc.clone().with_texture_slot(Some(42));

        let idle_state = ControlState::default();
        let hover_state = ControlState {
            phase: PointerPhase::Hovered,
            ..ControlState::default()
        };
        let pressed_state = ControlState {
            phase: PointerPhase::Pressed,
            ..ControlState::default()
        };

        let p_idle = emit_viewport_3d(10, rect, &placeholder_desc, idle_state);
        let p_hover = emit_viewport_3d(10, rect, &placeholder_desc, hover_state);
        let p_pressed = emit_viewport_3d(10, rect, &placeholder_desc, pressed_state);
        let t_idle = emit_viewport_3d(10, rect, &textured_desc, idle_state);

        assert_eq!(p_idle.primitives.len(), 3);
        assert_eq!(p_hover.primitives.len(), 3);
        assert_eq!(p_pressed.primitives.len(), 3);
        assert_eq!(t_idle.primitives.len(), 3);

        // 占位与纹理图元类型区分清晰，且绑定槽位写入 border_token_or_tex。
        assert_eq!(p_idle.primitives[0].prim_type(), PrimType::RoundedRect);
        assert_eq!(t_idle.primitives[0].prim_type(), PrimType::Image);
        assert_eq!(t_idle.primitives[0].border_token_or_tex, 42);
        assert_eq!(t_idle.payloads[0].p0, [0.0, 0.0]);
        assert_eq!(t_idle.payloads[0].p1, [1.0, 1.0]);

        // 悬停与按下态必须切换边框令牌。
        assert_ne!(
            p_idle.primitives[1].border_token_or_tex,
            p_hover.primitives[1].border_token_or_tex
        );
        assert_ne!(
            p_hover.primitives[1].border_token_or_tex,
            p_pressed.primitives[1].border_token_or_tex
        );

        // 交互启用时注册 Grab 光标命中区；关闭交互时移除命中区。
        let hit = p_idle.hit.expect("interactive viewport must register hit");
        assert_eq!(hit.cursor, Some(CursorKind::Grab));

        let non_interactive = placeholder_desc.with_interactive(false);
        let ni_out = emit_viewport_3d(10, rect, &non_interactive, idle_state);
        assert_eq!(ni_out.primitives.len(), 3);
        assert!(ni_out.hit.is_none());
    }
}

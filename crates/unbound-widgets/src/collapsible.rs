//! 单分区折叠面板（Collapsible / Disclosure），支持弹簧展开进度。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// [`CollapsibleDesc`] 的交互状态。
#[derive(Clone, Debug, PartialEq)]
pub struct CollapsibleState {
    /// 目标展开状态。
    pub open: bool,
    /// 归一化 `[0.0, 1.0]` 弹簧展开进度。
    pub progress: f32,
}

impl Default for CollapsibleState {
    fn default() -> Self {
        Self {
            open: false,
            progress: 0.0,
        }
    }
}

impl CollapsibleState {
    /// 切换展开/折叠状态。
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// 朝目标状态 `open` 步进动画。
    pub fn tick(&mut self, dt: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        let diff = target - self.progress;
        if diff.abs() <= 0.005 {
            self.progress = target;
            false
        } else {
            self.progress += diff * (dt * 12.0).clamp(0.0, 1.0);
            true
        }
    }
}

/// 单分区 [`Collapsible`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CollapsibleDesc {
    /// 外层边界矩形。
    pub rect: Rect,
    /// 标题栏触发文本。
    pub title: String,
    /// 是否处于展开状态（`progress` 位于 `0.0..=1.0`）。
    pub open: bool,
    /// 归一化展开进度 `[0.0, 1.0]`。
    pub progress: f32,
    /// 基础 z-index。
    pub z: u16,
}

impl CollapsibleDesc {
    /// 创建折叠面板描述符。
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>, open: bool) -> Self {
        Self {
            rect,
            title: title.into(),
            open,
            progress: if open { 1.0 } else { 0.0 },
            z: 600,
        }
    }
}

/// 发射固定形状的折叠面板（1 外框 + 1 标题栏 + 1 箭头 + 1 内容区 + 1 焦点环 = 5 个图元）。
#[must_use]
pub fn emit_collapsible(
    node: u64,
    desc: &CollapsibleDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let border = if hovered || state.focused {
        Token::FOCUS_RING
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(
        desc.rect,
        6.0,
        Token::SURFACE,
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let header_h = 32.0_f32.min(desc.rect.height());
    let header_r = Rect::new(
        desc.rect.origin.x,
        desc.rect.origin.y,
        desc.rect.width(),
        header_h,
    );
    primitives.push(surface(
        header_r,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        desc.z + 1,
    ));

    let chev_r = Rect::new(
        header_r.max().x - 20.0,
        header_r.origin.y + (header_h - 8.0) * 0.5,
        8.0,
        8.0,
    );
    primitives.push(surface(
        chev_r,
        2.0,
        if desc.open {
            Token::PRIMARY
        } else {
            Token::TEXT_SECONDARY
        },
        desc.z + 2,
    ));

    let body_h = ((desc.rect.height() - header_h) * desc.progress.clamp(0.0, 1.0)).max(0.0);
    let body_r = Rect::new(
        desc.rect.origin.x + 2.0,
        desc.rect.origin.y + header_h,
        (desc.rect.width() - 4.0).max(1.0),
        body_h.max(1.0),
    );
    let mut body = surface(body_r, 4.0, Token::SURFACE_SUNKEN, desc.z + 1);
    if desc.progress <= 0.01 {
        body.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(body.depth_u16(), 0.0);
    }
    primitives.push(body);

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z + 1), header_r)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::DisclosureTriangle, desc.title.clone(), header_r)
        .disabled(state.disabled);
    a11y.expanded = Some(desc.open);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

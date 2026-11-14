//! Spinner 与 LoadingOverlay 局部加载指示器及锁定遮罩。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{outlined, surface};
use crate::{ControlState, WidgetOutput};

/// [`Spinner`] 加载环的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct SpinnerDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 归一化动画相位 `[0.0, 1.0]`。
    pub phase: f32,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl SpinnerDesc {
    /// 创建加载环描述符。
    #[must_use]
    pub fn new(rect: Rect, phase: f32) -> Self {
        Self {
            rect,
            phase: phase.rem_euclid(1.0),
            label: "Loading".to_owned(),
            z: 750,
        }
    }
}

/// 覆盖任意目标矩形的 [`LoadingOverlay`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct LoadingOverlayDesc {
    /// 需要遮罩的目标区域。
    pub rect: Rect,
    /// 加载是否处于激活状态。
    pub active: bool,
    /// 加载环相位 `[0.0, 1.0]`。
    pub phase: f32,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

/// 输出结构稳定的加载环（1 个外环 + 1 个旋转指示圆点 = 2 个图元）。
#[must_use]
pub fn emit_spinner(desc: &SpinnerDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2);
    let sz = desc.rect.width().min(desc.rect.height()).max(16.0);
    let cx = desc.rect.origin.x + desc.rect.width() * 0.5;
    let cy = desc.rect.origin.y + desc.rect.height() * 0.5;
    let ring_r = Rect::new(cx - sz * 0.5, cy - sz * 0.5, sz, sz);

    primitives.push(outlined(
        ring_r,
        sz * 0.5,
        Token::SURFACE_SUNKEN,
        Token::CARD_BORDER,
        2.0,
        0.0,
        desc.z,
    ));

    let angle = desc.phase * core::f32::consts::TAU;
    let orbit = (sz * 0.5 - 4.0).max(2.0);
    let dot_sz = 6.0_f32.min(sz * 0.35);
    let dot_r = Rect::new(
        cx + angle.cos() * orbit - dot_sz * 0.5,
        cy + angle.sin() * orbit - dot_sz * 0.5,
        dot_sz,
        dot_sz,
    );
    primitives.push(surface(
        dot_r,
        dot_sz * 0.5,
        if state.disabled {
            Token::DISABLED
        } else {
            Token::PRIMARY
        },
        desc.z + 1,
    ));

    let a11y = A11yNode::new(Role::ProgressIndicator, desc.label.clone(), desc.rect)
        .disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

/// 输出结构稳定的加载遮罩（1 个半透明遮罩 + 2 个加载环图元 = 3 个图元）。
#[must_use]
pub fn emit_loading_overlay(desc: &LoadingOverlayDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3);
    let mut scrim = surface(desc.rect, 6.0, Token::SURFACE_SUNKEN, desc.z);
    scrim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        scrim.depth_u16(),
        if desc.active { 0.65 } else { 0.0 },
    );
    primitives.push(scrim);

    let spin_sz = desc.rect.width().min(desc.rect.height()).clamp(16.0, 44.0);
    let spin_rect = Rect::new(
        desc.rect.origin.x + (desc.rect.width() - spin_sz) * 0.5,
        desc.rect.origin.y + (desc.rect.height() - spin_sz) * 0.5,
        spin_sz,
        spin_sz,
    );
    let spin = emit_spinner(&SpinnerDesc::new(spin_rect, desc.phase), state);
    for mut p in spin.primitives {
        if !desc.active {
            p.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(p.depth_u16(), 0.0);
        }
        primitives.push(p);
    }

    let a11y = A11yNode::new(Role::ProgressIndicator, desc.label.clone(), desc.rect)
        .disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

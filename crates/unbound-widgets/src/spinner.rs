//! Spinner & LoadingOverlay local loading indicator and lock scrim.

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{outlined, surface};
use crate::{ControlState, WidgetOutput};

/// Descriptor for a [`Spinner`] ring.
#[derive(Clone, Debug, PartialEq)]
pub struct SpinnerDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Normalized animation phase `[0.0, 1.0]`.
    pub phase: f32,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl SpinnerDesc {
    /// Create a spinner descriptor.
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

/// Descriptor for a [`LoadingOverlay`] covering an arbitrary target rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadingOverlayDesc {
    /// Target area to mask.
    pub rect: Rect,
    /// Whether loading is active.
    pub active: bool,
    /// Spinner phase `[0.0, 1.0]`.
    pub phase: f32,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

/// Emit a stable-shape spinner (1 outer ring + 1 rotating indicator dot = 2 primitives).
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
    }
}

/// Emit a stable-shape loading overlay (1 scrim + 2 spinner primitives = 3 primitives).
#[must_use]
pub fn emit_loading_overlay(desc: &LoadingOverlayDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3);
    let mut scrim = surface(desc.rect, 6.0, Token::SURFACE_SUNKEN, desc.z);
    scrim.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        scrim.depth_u16(),
        if desc.active { 0.65 } else { 0.0 },
    );
    primitives.push(scrim);

    let spin = emit_spinner(&SpinnerDesc::new(desc.rect, desc.phase), state);
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
    }
}

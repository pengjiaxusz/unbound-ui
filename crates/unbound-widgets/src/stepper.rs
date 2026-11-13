//! Stepper / Wizard progress indicator with horizontal and vertical layouts.

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

const MAX_STEP_SLOTS: usize = 5;

/// Completion status of a step.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum StepStatus {
    /// Not yet reached.
    #[default]
    Pending,
    /// Currently active step.
    Active,
    /// Successfully completed.
    Completed,
    /// Validation or execution error.
    Error,
}

/// Layout orientation of a stepper.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum StepperOrientation {
    /// Steps arranged left to right.
    #[default]
    Horizontal,
    /// Steps arranged top to bottom.
    Vertical,
}

/// Single step descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepItem {
    /// Step title.
    pub title: String,
    /// Step status.
    pub status: StepStatus,
}

impl StepItem {
    /// Create a step item.
    #[must_use]
    pub fn new(title: impl Into<String>, status: StepStatus) -> Self {
        Self {
            title: title.into(),
            status,
        }
    }
}

/// Descriptor for a [`Stepper`].
#[derive(Clone, Debug, PartialEq)]
pub struct StepperDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Steps (up to 5 rendered slots).
    pub steps: Vec<StepItem>,
    /// Orientation.
    pub orientation: StepperOrientation,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl StepperDesc {
    /// Create a horizontal stepper descriptor.
    #[must_use]
    pub fn new(rect: Rect, steps: Vec<StepItem>) -> Self {
        Self {
            rect,
            steps,
            orientation: StepperOrientation::Horizontal,
            label: "Stepper".to_owned(),
            z: 600,
        }
    }

    /// Set orientation.
    #[must_use]
    pub fn with_orientation(mut self, orientation: StepperOrientation) -> Self {
        self.orientation = orientation;
        self
    }
}

/// Emit a stable-shape stepper (1 bg + 4 connectors + 5 step nodes + 1 focus ring = 11 primitives).
#[must_use]
pub fn emit_stepper(
    node: u64,
    desc: &StepperDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(11 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let count = desc.steps.len().clamp(1, MAX_STEP_SLOTS);
    let badge_sz = 22.0_f32.min(desc.rect.height() - 8.0).max(12.0);
    let horizontal = desc.orientation == StepperOrientation::Horizontal;

    let step_center = |idx: usize| -> (f32, f32) {
        if horizontal {
            let span = (desc.rect.width() - badge_sz - 16.0).max(1.0);
            let t = if count <= 1 {
                0.0
            } else {
                idx as f32 / (count - 1) as f32
            };
            (
                desc.rect.origin.x + 8.0 + badge_sz * 0.5 + span * t,
                desc.rect.origin.y + desc.rect.height() * 0.5,
            )
        } else {
            let span = (desc.rect.height() - badge_sz - 16.0).max(1.0);
            let t = if count <= 1 {
                0.0
            } else {
                idx as f32 / (count - 1) as f32
            };
            (
                desc.rect.origin.x + 16.0 + badge_sz * 0.5,
                desc.rect.origin.y + 8.0 + badge_sz * 0.5 + span * t,
            )
        }
    };

    for i in 0..(MAX_STEP_SLOTS - 1) {
        let visible = i + 1 < desc.steps.len();
        let (cx0, cy0) = step_center(i.min(count - 1));
        let (cx1, cy1) = step_center((i + 1).min(count - 1));
        let conn_rect = if horizontal {
            Rect::new(
                cx0 + badge_sz * 0.5 + 2.0,
                cy0 - 1.5,
                (cx1 - cx0 - badge_sz - 4.0).max(2.0),
                3.0,
            )
        } else {
            Rect::new(
                cx0 - 1.5,
                cy0 + badge_sz * 0.5 + 2.0,
                3.0,
                (cy1 - cy0 - badge_sz - 4.0).max(2.0),
            )
        };
        let completed = desc
            .steps
            .get(i)
            .is_some_and(|s| s.status == StepStatus::Completed);
        let mut inst = surface(
            conn_rect,
            1.5,
            if completed {
                Token::PRIMARY
            } else {
                Token::CARD_BORDER
            },
            desc.z + 1,
        );
        if !visible {
            inst.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(inst.depth_u16(), 0.0);
        }
        primitives.push(inst);
    }

    for i in 0..MAX_STEP_SLOTS {
        let step = desc.steps.get(i);
        let (cx, cy) = step_center(i.min(count - 1));
        let r = Rect::new(cx - badge_sz * 0.5, cy - badge_sz * 0.5, badge_sz, badge_sz);
        let status = step.map(|s| s.status).unwrap_or(StepStatus::Pending);
        let (fill, border) = match status {
            StepStatus::Completed | StepStatus::Active => (Token::PRIMARY, Token::PRIMARY),
            StepStatus::Error => (Token::ERROR, Token::ERROR),
            StepStatus::Pending => (Token::SURFACE, Token::CARD_BORDER),
        };
        let mut inst = outlined(r, badge_sz * 0.5, fill, border, 1.5, 0.0, desc.z + 2);
        if step.is_none() {
            inst.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(inst.depth_u16(), 0.0);
        }
        primitives.push(inst);
    }

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
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::List, desc.label.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

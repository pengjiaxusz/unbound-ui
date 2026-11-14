//! Stepper / 向导步骤进度指示器，支持水平与垂直布局。

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

/// 单个步骤的完成状态。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum StepStatus {
    /// 尚未到达。
    #[default]
    Pending,
    /// 当前激活步骤。
    Active,
    /// 已成功完成。
    Completed,
    /// 校验或执行出错。
    Error,
}

/// 步骤条的布局方向。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum StepperOrientation {
    /// 步骤从左到右排列。
    #[default]
    Horizontal,
    /// 步骤从上到下排列。
    Vertical,
}

/// 单个步骤描述符。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepItem {
    /// 步骤标题。
    pub title: String,
    /// 步骤状态。
    pub status: StepStatus,
}

impl StepItem {
    /// 创建步骤项。
    #[must_use]
    pub fn new(title: impl Into<String>, status: StepStatus) -> Self {
        Self {
            title: title.into(),
            status,
        }
    }
}

/// [`Stepper`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct StepperDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 步骤列表（最多渲染 5 个槽位）。
    pub steps: Vec<StepItem>,
    /// 排列方向。
    pub orientation: StepperOrientation,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl StepperDesc {
    /// 创建水平步骤条描述符。
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

    /// 设置排列方向。
    #[must_use]
    pub fn with_orientation(mut self, orientation: StepperOrientation) -> Self {
        self.orientation = orientation;
        self
    }
}

/// 步骤节点的确定性子节点 ID（`0..5`）。
#[must_use]
pub const fn stepper_step_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0xD00 + index as u64)
}

/// 输出结构稳定的步骤条（1 个背景 + 4 条连接线 + 5 个步骤节点 + 1 个焦点环 = 11 个图元）。
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

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for (i, step) in desc.steps.iter().take(MAX_STEP_SLOTS).enumerate() {
            let (cx, cy) = step_center(i.min(count - 1));
            let r = Rect::new(cx - badge_sz * 0.5, cy - badge_sz * 0.5, badge_sz, badge_sz);
            let key = stepper_step_node(node, i);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 2, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, step.title.clone(), r).clickable(),
                true,
            );
        }
    }

    out
}

//! 后台任务进度浮窗与活动队列栈（`TaskHud`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 单个后台任务进度项。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskActivityItem {
    pub id: u64,
    pub title: String,
    /// 进度范围 `0.0..=1.0`。
    pub progress: f32,
    pub cancellable: bool,
}

impl TaskActivityItem {
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, progress: f32) -> Self {
        Self {
            id,
            title: title.into(),
            progress: progress.clamp(0.0, 1.0),
            cancellable: true,
        }
    }
}

/// 后台任务 HUD 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskHudDesc {
    pub rect: Rect,
    pub tasks: Vec<TaskActivityItem>,
    pub z: u16,
}

impl TaskHudDesc {
    #[must_use]
    pub fn new(rect: Rect, tasks: Vec<TaskActivityItem>) -> Self {
        Self { rect, tasks, z: 40 }
    }
}

/// 发射后台任务 HUD（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_task_hud(
    node: u64,
    desc: &TaskHudDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.tasks.len() * 3 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(r, 10.0, Token::HUD_BG, border, 1.0, 12.0, z));

    for (idx, task) in desc.tasks.iter().enumerate() {
        let y = r.origin.y + 12.0 + idx as f32 * 38.0;
        let track_r = Rect::new(
            r.origin.x + 12.0,
            y + 20.0,
            (r.size.x - 24.0).max(20.0),
            6.0,
        );
        primitives.push(surface(track_r, 3.0, Token::CONTROL_TRACK, z + 1));
        let fill_r = Rect::new(
            track_r.origin.x,
            track_r.origin.y,
            track_r.size.x * task.progress.clamp(0.0, 1.0),
            track_r.size.y,
        );
        primitives.push(surface(fill_r, 3.0, Token::PRIMARY, z + 2));
        let cancel_r = Rect::new(r.max().x - 26.0, y, 14.0, 14.0);
        primitives.push(surface(cancel_r, 3.0, Token::SURFACE_RAISED, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::HUD_TEXT,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        12.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Status, "Active Tasks", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

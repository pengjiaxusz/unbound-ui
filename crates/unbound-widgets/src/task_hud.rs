//! 后台任务进度浮窗与活动队列栈（`TaskHud`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

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

    // 浮层圆角对齐 WinUI `OverlayCornerRadius`（8），描边用 1px `ControlStrokeColorDefault`。
    // Win11 的 Flyout 没有悬停态，因此这里不再按悬停更换描边色。
    primitives.push(outlined(
        r,
        8.0,
        Token::HUD_BG,
        Token::CONTROL_STROKE,
        1.0,
        12.0,
        z,
    ));

    for (idx, task) in desc.tasks.iter().enumerate() {
        // 行距对齐 WinUI `ListViewItemMinHeight`（40）。
        let y = r.origin.y + 12.0 + idx as f32 * 40.0;
        let track_r = Rect::new(
            r.origin.x + 12.0,
            y + 20.0,
            (r.size.x - 24.0).max(20.0),
            3.0,
        );
        // 进度条按 WinUI ProgressBar 规格：3px 高、1.5px 圆角（`ProgressBarMinHeight` /
        // `ProgressBarCornerRadius`）。
        primitives.push(surface(track_r, 1.5, Token::CONTROL_TRACK, z + 1));
        let fill_r = Rect::new(
            track_r.origin.x,
            track_r.origin.y,
            track_r.size.x * task.progress.clamp(0.0, 1.0),
            track_r.size.y,
        );
        primitives.push(surface(fill_r, 1.5, Token::PRIMARY, z + 2));
        // 取消按钮按控件级 4px 圆角（`ControlCornerRadius`）绘制；
        // 悬停/按压沿用 Win11 按钮的 `SubtleFillColorSecondary/Tertiary` 语义。
        let cancel_fill = if control.disabled {
            Token::CONTROL_DISABLED_BG
        } else if control.phase == crate::PointerPhase::Pressed {
            Token::PRESSED
        } else if control.phase == crate::PointerPhase::Hovered {
            Token::HOVER
        } else {
            Token::SURFACE_RAISED
        };
        let cancel_r = Rect::new(r.max().x - 26.0, y, 14.0, 14.0);
        primitives.push(surface(cancel_r, 4.0, cancel_fill, z + 2));
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
        10.0,
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

//! 后台任务进度浮窗与活动队列栈（`TaskHud`）。
//!
//! 提供多任务队列状态机（[`TaskHudState`]，支持任务新增/更新进度/完成/失败/取消回调、
//! 折叠为单行摘要胶囊与展开清单切换、清除已完成项）与 24 态恒定图元数发射器（[`emit_task_hud`]）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

const LAYER: u8 = 1;

/// 后台任务生命周期状态。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    /// 运行中。
    #[default]
    Running,
    /// 已完成。
    Completed,
    /// 执行失败。
    Failed,
    /// 已取消。
    Cancelled,
}

impl TaskStatus {
    /// 是否属于终态（可被一键清除）。
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// 单个后台任务进度项。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskActivityItem {
    pub id: u64,
    pub title: String,
    /// 进度范围 `0.0..=1.0`。
    pub progress: f32,
    pub cancellable: bool,
    /// 当前任务状态。
    pub status: TaskStatus,
    /// 可选失败或状态说明信息。
    pub message: Option<String>,
}

impl TaskActivityItem {
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, progress: f32) -> Self {
        Self {
            id,
            title: title.into(),
            progress: progress.clamp(0.0, 1.0),
            cancellable: true,
            status: TaskStatus::Running,
            message: None,
        }
    }

    /// 设置是否允许取消。
    #[must_use]
    pub const fn with_cancellable(mut self, cancellable: bool) -> Self {
        self.cancellable = cancellable;
        self
    }

    /// 设置任务状态与可选说明。
    #[must_use]
    pub fn with_status(mut self, status: TaskStatus, message: Option<String>) -> Self {
        self.status = status;
        self.message = message;
        self
    }
}

/// 后台任务队列交互状态机。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskHudState {
    /// 任务队列。
    pub tasks: Vec<TaskActivityItem>,
    /// 是否折叠为单行摘要胶囊。
    pub collapsed: bool,
    /// 等待宿主消费的取消请求任务 ID 队列。
    pending_cancels: Vec<u64>,
}

impl TaskHudState {
    /// 创建空任务队列状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 新增或按 `id` 更新任务项。
    pub fn upsert_task(&mut self, item: TaskActivityItem) {
        if let Some(existing) = self.tasks.iter_mut().find(|t| t.id == item.id) {
            *existing = item;
        } else {
            self.tasks.push(item);
        }
    }

    /// 更新指定任务的进度（自动钳制到 `0.0..=1.0`）；任务不存在时返回 `false`。
    pub fn update_progress(&mut self, id: u64, progress: f32) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        task.progress = progress.clamp(0.0, 1.0);
        if task.status == TaskStatus::Running && task.progress >= 1.0 {
            task.status = TaskStatus::Completed;
            task.cancellable = false;
        }
        true
    }

    /// 将指定任务标记为已完成。
    pub fn mark_completed(&mut self, id: u64) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        task.progress = 1.0;
        task.status = TaskStatus::Completed;
        task.cancellable = false;
        true
    }

    /// 将指定任务标记为失败并附带原因说明。
    pub fn mark_failed(&mut self, id: u64, reason: impl Into<String>) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        task.status = TaskStatus::Failed;
        task.cancellable = false;
        task.message = Some(reason.into());
        true
    }

    /// 请求取消指定任务（触发回调并记录到 `pending_cancels`）；若不可取消或不存在则返回 `false`。
    pub fn request_cancel<F>(&mut self, id: u64, on_cancel: F) -> bool
    where
        F: FnOnce(u64),
    {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        if !task.cancellable || task.status.is_terminal() {
            return false;
        }
        task.status = TaskStatus::Cancelled;
        task.cancellable = false;
        self.pending_cancels.push(id);
        on_cancel(id);
        true
    }

    /// 取出并清空待宿主处理的取消请求 ID 列表。
    pub fn take_cancel_requests(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.pending_cancels)
    }

    /// 切换折叠（单行摘要胶囊）/ 展开（完整任务清单）模式。
    pub fn toggle_collapsed(&mut self) -> bool {
        self.collapsed = !self.collapsed;
        self.collapsed
    }

    /// 显式设置折叠态。
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// 清除所有已进入终态（Completed / Failed / Cancelled）的任务，返回清除数量。
    pub fn clear_completed(&mut self) -> usize {
        let before = self.tasks.len();
        self.tasks.retain(|t| !t.status.is_terminal());
        before - self.tasks.len()
    }

    /// 当前仍在运行中的任务数。
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Running)
            .count()
    }

    /// 折叠胶囊单行摘要文案。
    #[must_use]
    pub fn summary_text(&self) -> String {
        let active = self.active_count();
        if self.tasks.is_empty() {
            "No active tasks".to_owned()
        } else if active == 0 {
            format!("{} tasks finished", self.tasks.len())
        } else if active == 1 {
            "1 task running".to_owned()
        } else {
            format!("{active} tasks running")
        }
    }

    /// 基于当前队列与折叠态构造用于发射的 [`TaskHudDesc`]。
    #[must_use]
    pub fn to_desc(&self, rect: Rect) -> TaskHudDesc {
        let visible_tasks = if self.collapsed {
            self.tasks.iter().take(1).cloned().collect()
        } else {
            self.tasks.clone()
        };
        TaskHudDesc {
            rect,
            tasks: visible_tasks,
            collapsed: self.collapsed,
            summary_text: self.summary_text(),
            z: 40,
        }
    }
}

/// 后台任务 HUD 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TaskHudDesc {
    pub rect: Rect,
    pub tasks: Vec<TaskActivityItem>,
    /// 是否处于单行摘要胶囊折叠态。
    pub collapsed: bool,
    /// 折叠态单行摘要与无障碍状态值。
    pub summary_text: String,
    pub z: u16,
}

impl TaskHudDesc {
    #[must_use]
    pub fn new(rect: Rect, tasks: Vec<TaskActivityItem>) -> Self {
        let active = tasks.iter().filter(|t| t.status == TaskStatus::Running).count();
        let summary_text = if tasks.is_empty() {
            "No active tasks".to_owned()
        } else {
            format!("{active} tasks running")
        };
        Self {
            rect,
            tasks,
            collapsed: false,
            summary_text,
            z: 40,
        }
    }

    /// 设置折叠态。
    #[must_use]
    pub const fn with_collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
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
        let fill_tok = match task.status {
            TaskStatus::Running => Token::PRIMARY,
            TaskStatus::Completed => Token::SUCCESS,
            TaskStatus::Failed => Token::ERROR,
            TaskStatus::Cancelled => Token::TEXT_DISABLED,
        };
        primitives.push(surface(fill_r, 1.5, fill_tok, z + 2));
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
        a11y: Some(
            A11yNode::new(Role::Status, "Active Tasks", r)
                .with_value(desc.summary_text.clone())
                .expanded(!desc.collapsed)
                .disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn task_hud_state_queue_lifecycle_cancel_and_collapse() {
        let mut st = TaskHudState::new();
        assert_eq!(st.active_count(), 0);
        assert_eq!(st.summary_text(), "No active tasks");

        // 并发写入多个任务并推进进度。
        st.upsert_task(TaskActivityItem::new(1, "Capture Frame", 0.25));
        st.upsert_task(TaskActivityItem::new(2, "Build Shaders", 0.50));
        st.upsert_task(TaskActivityItem::new(3, "Export Trace", 0.10).with_cancellable(false));
        assert_eq!(st.active_count(), 3);
        assert_eq!(st.summary_text(), "3 tasks running");

        assert!(st.update_progress(1, 1.0));
        assert_eq!(st.tasks[0].status, TaskStatus::Completed);
        assert_eq!(st.active_count(), 2);

        // 不可取消的任务拒绝取消。
        let mut cancelled_ids = Vec::new();
        assert!(!st.request_cancel(3, |id| cancelled_ids.push(id)));
        assert!(cancelled_ids.is_empty());

        // 可取消的任务触发取消回调并记录请求。
        assert!(st.request_cancel(2, |id| cancelled_ids.push(id)));
        assert_eq!(cancelled_ids, vec![2]);
        assert_eq!(st.take_cancel_requests(), vec![2]);

        // 标记任务 3 失败后清除所有终态任务。
        assert!(st.mark_failed(3, "Disk full"));
        assert_eq!(st.active_count(), 0);
        assert_eq!(st.clear_completed(), 3);
        assert!(st.tasks.is_empty());

        // 折叠态切换。
        st.upsert_task(TaskActivityItem::new(4, "Upload", 0.4));
        st.upsert_task(TaskActivityItem::new(5, "Index", 0.6));
        assert!(st.toggle_collapsed());
        let collapsed_desc = st.to_desc(Rect::new(20.0, 20.0, 280.0, 48.0));
        assert!(collapsed_desc.collapsed);
        assert_eq!(collapsed_desc.tasks.len(), 1);
    }

    #[test]
    fn task_hud_empty_queue_and_headless_geometry() {
        let rect = Rect::new(20.0, 20.0, 280.0, 56.0);
        let st = TaskHudState::new();
        let desc = st.to_desc(rect);
        let out = emit_task_hud(1, &desc, ControlState::default(), &[]);
        assert_eq!(out.primitives.len(), 2);

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "TaskHud",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

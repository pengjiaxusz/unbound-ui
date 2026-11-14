//! 运行时覆盖层栈与焦点陷阱管理器（`docs` §6.2，S5.2）。
//!
//! 封装 [`OverlayStack`]，协调 [`FocusManager`] 焦点陷阱、点击外部轻关闭、
//! `Escape` 关闭以及将焦点归还至触发元素。

use unbound_input::{FocusCause, FocusManager};
use unbound_math::Vec2;
use unbound_widgets::overlay::{OverlayEntry, OverlayStack};

/// 协调已打开的覆盖层、模态焦点陷阱与焦点恢复。
#[derive(Debug, Default)]
pub struct OverlayManager {
    stack: OverlayStack,
    return_focus: Vec<(u64, Option<u64>)>,
}

impl OverlayManager {
    /// 创建空的覆盖层管理器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 借用底层的 [`OverlayStack`]。
    #[must_use]
    pub fn stack(&self) -> &OverlayStack {
        &self.stack
    }

    /// 可变借用底层的 [`OverlayStack`]。
    pub fn stack_mut(&mut self) -> &mut OverlayStack {
        &mut self.stack
    }

    /// 当前是否存在模态覆盖层正在捕获焦点并锁定背景滚动。
    #[must_use]
    pub fn is_modal_active(&self) -> bool {
        self.stack.focus_scope().is_some()
    }

    /// 打开覆盖层，记录当前焦点以便后续恢复，并在存在活跃模态作用域时安装焦点陷阱。
    ///
    pub fn open(&mut self, entry: OverlayEntry, focus: &mut FocusManager) -> u64 {
        let node = entry.node;
        let prev = focus.focused();
        if !self.stack.contains(node) {
            self.return_focus.push((node, prev));
        }
        let key = self.stack.push(entry);
        self.sync_focus_trap(focus);
        key
    }

    /// 按 `node` 关闭指定覆盖层，恢复其触发元素焦点并更新陷阱。
    pub fn close(&mut self, node: u64, focus: &mut FocusManager) -> bool {
        if !self.stack.close(node) {
            return false;
        }
        self.restore_and_sync(node, focus);
        true
    }

    /// 处理 `point` 处的主指针按下事件，若点击落在最顶层可轻关闭覆盖层之外则将其关闭。
    ///
    pub fn handle_pointer_down(&mut self, point: Vec2, focus: &mut FocusManager) -> Option<u64> {
        let closed = self.stack.close_outside(point)?;
        self.restore_and_sync(closed, focus);
        Some(closed)
    }

    /// 处理 `Escape` 键，关闭最顶层可按 Esc 关闭的覆盖层并恢复焦点。
    pub fn handle_escape(&mut self, focus: &mut FocusManager) -> Option<u64> {
        let closed = self.stack.close_on_escape()?;
        self.restore_and_sync(closed, focus);
        Some(closed)
    }

    /// 清理锚点已不存在的覆盖层并更新焦点陷阱。
    pub fn prune_anchors(
        &mut self,
        valid: impl FnMut(u64) -> bool,
        focus: &mut FocusManager,
    ) -> usize {
        let removed = self.stack.prune_anchors(valid);
        if removed > 0 {
            self.return_focus
                .retain(|(node, _)| self.stack.contains(*node));
            self.sync_focus_trap(focus);
        }
        removed
    }

    /// 将 [`FocusManager`] 焦点陷阱与 [`OverlayStack`] 中最顶层模态同步。
    pub fn sync_focus_trap(&self, focus: &mut FocusManager) {
        if let Some(scope) = self.stack.focus_scope() {
            focus.set_trap(scope.iter().copied());
        } else {
            focus.clear_trap();
        }
    }

    fn restore_and_sync(&mut self, closed: u64, focus: &mut FocusManager) {
        self.sync_focus_trap(focus);
        if let Some(pos) = self.return_focus.iter().rposition(|(n, _)| *n == closed) {
            let (_, prev) = self.return_focus.remove(pos);
            if let Some(target) = prev {
                if focus.is_trapped(target) {
                    focus.focus(target, FocusCause::Programmatic);
                }
            }
        }
    }
}

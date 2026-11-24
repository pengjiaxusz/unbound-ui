//! 跨平台无障碍适配器统一封装（收口 Windows UIA 与 Linux AT-SPI）。

use std::sync::mpsc::Sender;
use std::sync::Arc;

use accesskit::{ActionRequest, TreeUpdate};
use unbound_a11y::Semantics;

use crate::a11y_bridge::A11yRequest;
use crate::a11y_linux::{A11yWindowBounds, LinuxA11y};
use crate::a11y_win32::Win32A11y;
use crate::backend::{Flow, PlatformApp};
use crate::native_window::NativeWindow;
use crate::window::PlatformState;

/// 跨平台无障碍适配器：统一收口 Windows [`Win32A11y`] 与 Linux [`LinuxA11y`]。
pub enum PlatformA11y {
    /// Windows UI Automation 适配器。
    Win32(Win32A11y),
    /// Linux AT-SPI（`accesskit_unix`）适配器。
    Linux(LinuxA11y),
}

impl PlatformA11y {
    /// 为指定 Windows 窗口句柄创建适配器。
    #[must_use]
    pub fn for_win32(hwnd: isize) -> Self {
        Self::Win32(Win32A11y::new(hwnd))
    }

    /// 为指定 Linux 顶层窗口创建独立的 AT-SPI 适配器（不用于非激活弹出窗）。
    #[must_use]
    pub fn for_linux(window_id: u64, wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        Self::Linux(LinuxA11y::new(window_id, wake))
    }

    /// 获取动作请求注入发送端。
    #[must_use]
    pub fn request_sender(&self) -> Sender<ActionRequest> {
        match self {
            Self::Win32(a) => a.request_sender(),
            Self::Linux(a) => a.request_sender(),
        }
    }

    /// 启用或停用无障碍更新发布。
    pub fn set_active(&mut self, active: bool) {
        match self {
            Self::Win32(a) => a.set_active(active),
            Self::Linux(a) => a.set_active(active),
        }
    }

    /// 同步顶层窗口焦点状态。
    pub fn set_focus(&mut self, focused: bool) {
        if let Self::Linux(a) = self {
            a.set_focus(focused);
        }
    }

    /// 同步顶层窗口几何边界。
    pub fn set_window_bounds(&mut self, bounds: A11yWindowBounds) {
        if let Self::Linux(a) = self {
            a.set_window_bounds(bounds);
        }
    }

    /// 发布语义树增量（惰性门禁：未激活时不消费）。
    pub fn publish(&mut self, semantics: &mut Semantics) -> bool {
        match self {
            Self::Win32(a) => a.publish(semantics),
            Self::Linux(a) => a.publish(semantics),
        }
    }

    /// 通过惰性闭包发布语义树增量。
    pub fn publish_with(&mut self, update: impl FnOnce() -> Option<TreeUpdate>) -> bool {
        match self {
            Self::Win32(a) => a.publish_with(update),
            Self::Linux(a) => a.publish_with(update),
        }
    }

    /// 排空来自辅助技术的动作请求。
    #[must_use]
    pub fn drain_requests(&mut self) -> Vec<A11yRequest> {
        match self {
            Self::Win32(a) => a.drain_requests(),
            Self::Linux(a) => a.drain_requests(),
        }
    }

    /// 已跨越边界发布的更新总次数。
    #[must_use]
    pub fn published(&self) -> u64 {
        match self {
            Self::Win32(a) => a.published(),
            Self::Linux(a) => a.published(),
        }
    }
}

/// 统一排空单个顶层窗口的入站无障碍动作并惰性发布出站语义增量。
pub(crate) fn pump_platform_a11y<A: PlatformApp>(
    app: &mut A,
    a11y: &mut PlatformA11y,
    state: &mut PlatformState,
    window: &NativeWindow,
) -> Flow {
    for request in a11y.drain_requests() {
        if app.accessibility_action(state, window, request) == Flow::Exit {
            return Flow::Exit;
        }
    }
    a11y.publish_with(|| app.accessibility_update_for_window(window));
    Flow::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y_bridge::action_request;
    use accesskit::Action;
    use unbound_a11y::{A11yAction, A11yNode, Semantics};
    use unbound_math::Rect;

    fn sample_semantics(label: &str) -> Semantics {
        let mut s = Semantics::new(1);
        s.upsert(
            10,
            A11yNode::new(
                accesskit::Role::Button,
                label,
                Rect::new(0.0, 0.0, 100.0, 32.0),
            )
            .clickable(),
        );
        s.set_focus(10);
        s
    }

    #[test]
    fn linux_a11y_lazy_activation_does_not_consume_initial_snapshot_before_atspi_connects() {
        let mut a11y = PlatformA11y::for_linux(1, None);
        let mut semantics = sample_semantics("Confirm");
        let pending_before = semantics.pending();
        assert!(pending_before > 0);

        // 辅助技术未激活前，publish 与 publish_with 均不得消费首次快照
        assert!(!a11y.publish(&mut semantics));
        assert_eq!(semantics.pending(), pending_before);

        let mut closure_called = false;
        assert!(!a11y.publish_with(|| {
            closure_called = true;
            semantics.take_update()
        }));
        assert!(!closure_called);
        assert_eq!(a11y.published(), 0);
    }

    #[test]
    fn multi_window_a11y_adapters_isolate_requests_and_lifecycle() {
        let mut main_win = PlatformA11y::for_linux(1, None);
        let mut second_win = PlatformA11y::for_linux(2, None);

        let main_tx = main_win.request_sender();
        let second_tx = second_win.request_sender();

        main_tx
            .send(action_request(Action::Click, 10, None))
            .expect("send to main");
        second_tx
            .send(action_request(Action::SetValue, 20, Some("hello")))
            .expect("send to second");

        let main_reqs = main_win.drain_requests();
        assert_eq!(main_reqs.len(), 1);
        assert_eq!(main_reqs[0].node, 10);
        assert_eq!(main_reqs[0].action, A11yAction::Click);

        let second_reqs = second_win.drain_requests();
        assert_eq!(second_reqs.len(), 1);
        assert_eq!(second_reqs[0].node, 20);
        assert_eq!(second_reqs[0].action, A11yAction::SetValue);
        assert_eq!(second_reqs[0].value.as_deref(), Some("hello"));

        // 关闭副窗释放其适配器不影响主窗继续接收动作
        drop(second_win);
        main_win.set_focus(true);
        main_win.set_window_bounds(A11yWindowBounds::new(0.0, 0.0, 1280.0, 800.0));
        main_tx
            .send(action_request(Action::Focus, 10, None))
            .expect("main still alive");
        assert_eq!(main_win.drain_requests().len(), 1);
    }
}

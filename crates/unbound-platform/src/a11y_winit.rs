//! 真实平台无障碍适配器（AccessKit + winit）。
//!
//! [`super::a11y_bridge`] 定义引擎侧契约；本模块负责将其送达操作系统。
//! 适配器必须在窗口显示**之前**创建（AccessKit 的硬性要求），每个脏帧发布
//! 一条增量 `TreeUpdate`，并将传入的 `ActionRequest` 排入通道由事件循环排空——
//! 回调可能在任意线程上被调用，因此不能直接触碰应用状态。
//!
//!
//!
//! 当没有辅助技术监听时，`update_if_active` 绝不会调用更新回调：
//! 静止帧零系统调用开销。

use std::sync::mpsc::{channel, Receiver, Sender};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use unbound_a11y::Semantics;

use super::a11y_bridge::{engine_action, A11yRequest};

/// 入站通道半端：接收来自操作系统的请求。
///
/// `std::sync::mpsc::Sender` 满足 `Send` 但非 `Sync`；适配器仅要求 `Send`，
/// 因此将其封装可在不改变通道语义的前提下满足 trait 约束。
///
struct RequestSender(Sender<ActionRequest>);

impl ActionHandler for RequestSender {
    fn do_action(&mut self, request: ActionRequest) {
        // 通道已关闭意味着窗口已销毁；丢弃该请求是唯一合理的处理方式。
        //
        let _ = self.0.send(request);
    }
}

struct Activation;

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // 应用在首帧通过 `publish` 发布完整语义树；此处返回 `None` 让平台在
        // 首帧落地前先使用占位树。
        //
        None
    }
}

struct Deactivation;

impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {}
}

/// 窗口的无障碍适配器。
pub struct WinitA11y {
    adapter: accesskit_winit::Adapter,
    requests: Receiver<ActionRequest>,
    published: u64,
}

impl WinitA11y {
    /// 为尚处于隐藏状态的窗口创建适配器。
    ///
    /// # Panics
    /// 若窗口已显示，AccessKit 会 panic；因此后端先创建隐藏窗口，随后再显示。
    ///
    #[must_use]
    pub fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        window: &winit::window::Window,
    ) -> Self {
        let (sender, requests) = channel();
        let adapter = accesskit_winit::Adapter::with_direct_handlers(
            event_loop,
            window,
            Activation,
            RequestSender(sender),
            Deactivation,
        );
        Self {
            adapter,
            requests,
            published: 0,
        }
    }

    /// 向适配器送入一条窗口事件。
    ///
    /// 必须在应用处理该事件前调用，确保适配器看到与 UI 完全相同的事件流。
    ///
    pub fn process_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        self.adapter.process_event(window, event);
    }

    /// 若有辅助技术正在监听，则发布待处理的语义增量。
    ///
    /// 返回是否有更新跨越边界。仅当语义树确实存在待处理变更时才调用 `take_update`，
    /// 因此静止帧零堆分配。
    ///
    ///
    pub fn publish(&mut self, semantics: &mut Semantics) -> bool {
        if semantics.pending() == 0 {
            return false;
        }
        let Some(update) = semantics.take_update() else {
            return false;
        };
        self.adapter.update_if_active(move || update);
        self.published += 1;
        true
    }

    /// 通过闭包发布，使调用方可在 `update_if_active` 内部惰性构建更新
    /// （无客户端监听时不会消费语义树）。
    ///
    pub fn publish_with(&mut self, update: impl FnOnce() -> Option<TreeUpdate>) -> bool {
        let Some(update) = update() else {
            return false;
        };
        self.adapter.update_if_active(move || update);
        self.published += 1;
        true
    }

    /// 排空自上一帧以来操作系统排入队列的动作请求。
    #[must_use]
    pub fn drain_requests(&mut self) -> Vec<A11yRequest> {
        let mut out = Vec::new();
        while let Ok(request) = self.requests.try_recv() {
            if let Some(action) = engine_action(request.action) {
                let value = match &request.data {
                    Some(accesskit::ActionData::Value(value)) => Some(value.as_ref().to_owned()),
                    _ => None,
                };
                out.push(A11yRequest {
                    node: request.target_node.0,
                    action,
                    value,
                });
            }
        }
        out
    }

    /// 跨越边界的更新总次数。
    #[must_use]
    pub fn published(&self) -> u64 {
        self.published
    }
}

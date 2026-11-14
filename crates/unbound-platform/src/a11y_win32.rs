//! 原生 Win32 无障碍适配器（AccessKit + UI Automation 桥接）。
//!
//! [`super::a11y_bridge`] 定义引擎侧契约；本模块负责将其与 Win32 窗口生命周期对接。
//! 适配器在窗口显示前创建，每个脏帧发布一条增量 `TreeUpdate`，并将入站
//! `ActionRequest` 排入通道由事件循环排空。

use std::sync::mpsc::{channel, Receiver, Sender};

use accesskit::{ActionRequest, TreeUpdate};
use unbound_a11y::Semantics;

use super::a11y_bridge::{engine_action, A11yBridge, A11yRequest, A11yTransport};

/// Win32 侧无障碍更新传输缓冲。
#[derive(Default)]
struct Win32A11yTransport {
    last_update: Option<TreeUpdate>,
}

impl A11yTransport for Win32A11yTransport {
    fn send(&mut self, update: TreeUpdate) {
        self.last_update = Some(update);
    }
}

/// 原生 Win32 窗口的无障碍适配器。
pub struct Win32A11y {
    /// 关联的原生窗口句柄。单独保存一份，使只读查询无需可变借用传输层。
    hwnd: isize,
    bridge: A11yBridge<Win32A11yTransport>,
    sender: Sender<ActionRequest>,
    requests: Receiver<ActionRequest>,
    published: u64,
}

impl Win32A11y {
    /// 为指定 `hwnd` 创建 Win32 无障碍适配器。
    #[must_use]
    pub fn new(hwnd: isize) -> Self {
        let (sender, requests) = channel();
        let mut bridge = A11yBridge::new(Win32A11yTransport { last_update: None });
        bridge.set_enabled(true);
        Self {
            hwnd,
            bridge,
            sender,
            requests,
            published: 0,
        }
    }

    /// 返回关联的原生窗口句柄。
    #[must_use]
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 获取用于从系统回调注入动作请求的发送端。
    #[must_use]
    pub fn request_sender(&self) -> Sender<ActionRequest> {
        self.sender.clone()
    }

    /// 启用或停用无障碍更新发布。
    pub fn set_active(&mut self, active: bool) {
        self.bridge.set_enabled(active);
    }

    /// 若有辅助技术正在监听，则发布待处理的语义增量。
    pub fn publish(&mut self, semantics: &mut Semantics) -> bool {
        if semantics.pending() == 0 {
            return false;
        }
        if self.bridge.publish(semantics) {
            self.published += 1;
            true
        } else {
            false
        }
    }

    /// 通过闭包惰性构建并发布语义树增量。
    pub fn publish_with(&mut self, update: impl FnOnce() -> Option<TreeUpdate>) -> bool {
        if !self.bridge.enabled() {
            return false;
        }
        let Some(update) = update() else {
            return false;
        };
        self.bridge.transport_mut().send(update);
        self.published += 1;
        true
    }

    /// 排空自上一帧以来入队的动作请求。
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

//! 原生 Win32 无障碍适配器（AccessKit + UI Automation 桥接）。
//!
//! [`super::a11y_bridge`] 定义引擎侧契约；本模块负责将其与 Win32 窗口生命周期对接。
//! 适配器在窗口显示前创建，每个脏帧发布一条增量 `TreeUpdate`，并将入站
//! `ActionRequest` 排入通道由事件循环排空。

use std::sync::mpsc::{channel, Receiver, Sender};

use accesskit::{ActionRequest, TreeUpdate};
use unbound_a11y::Semantics;

use super::a11y_bridge::{engine_action, A11yBridge, A11yRequest, A11yTransport};

#[cfg(windows)]
use super::uia_win32::UiaProvider;

/// Win32 侧无障碍更新传输缓冲。
///
/// 传输的终点就是系统的 UI Automation 出口：语义增量在这里跨越边界。
/// 增量只有一条通道——`Semantics::take_update` 每次只产出一份，
/// 两个出口各拿一份的话，先拿的那个会把另一个饿死。
#[derive(Default)]
struct Win32A11yTransport {
    /// 系统的 UI Automation 出口。
    #[cfg(windows)]
    uia: Option<UiaProvider>,
}

impl Win32A11yTransport {
    /// 建立传输缓冲，并把系统动作请求接到同一条通道上。
    fn new(hwnd: isize, sender: &Sender<ActionRequest>) -> Self {
        #[cfg(windows)]
        {
            use std::ffi::c_void;
            // 句柄由调用方（窗口创建处）保证有效；`HWND` 只是它的薄包装。
            let handle = accesskit_windows::HWND(hwnd as *mut c_void);
            Self {
                uia: UiaProvider::new(handle, sender.clone()),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (hwnd, sender);
            Self {}
        }
    }
}

impl A11yTransport for Win32A11yTransport {
    fn send(&mut self, update: TreeUpdate) {
        #[cfg(windows)]
        if let Some(uia) = self.uia.as_mut() {
            uia.update(update);
        }
        #[cfg(not(windows))]
        let _ = update;
    }

    fn wants_update(&self) -> bool {
        #[cfg(windows)]
        {
            // 辅助技术接入之前不要动语义树：那时的更新会被 Windows 丢弃，
            // 而取走它就再也拿不回来了——等接入时本该给出完整树。
            self.uia.as_ref().is_some_and(UiaProvider::is_activated)
        }
        #[cfg(not(windows))]
        {
            true
        }
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
    ///
    /// 必须在窗口**显示之前**调用：UIA 出口靠窗口子类化接住 `WM_GETOBJECT`，
    /// 错过这个时机就再也装不上了。
    #[must_use]
    pub fn new(hwnd: isize) -> Self {
        let (sender, requests) = channel();
        let mut bridge = A11yBridge::new(Win32A11yTransport::new(hwnd, &sender));
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
    ///
    /// 闭包是**惰性**的：传输层说不要，就根本不会被调用。这正是
    /// UI Automation 需要的——Windows 在辅助技术接入之前不消费任何
    /// 更新，那时就构造更新等于白费力气，还会把「完整树」提前摘走。
    pub fn publish_with(&mut self, update: impl FnOnce() -> Option<TreeUpdate>) -> bool {
        if !self.bridge.enabled() {
            return false;
        }
        if !self.bridge.transport_mut().wants_update() {
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

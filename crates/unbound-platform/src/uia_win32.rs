//! UI Automation 提供者：把同一棵语义树暴露给系统。
//!
//! 这一层**不是第二套语义**，而是「一棵语义树、两个出口」：
//!
//! * 角色定义、节点数据、动作模型与 Agent 侧**完全共用**
//!   ——同一棵 [`unbound_a11y::Semantics`]、同一套 `accesskit::Role`、
//!   同一套动作语义。
//! * 差别**只在出口通道**：Agent 走框架内部的语义树（更快、更全、
//!   合成输入不抢真实焦点），UIA 走系统的无障碍栈。
//!
//! 有了它，屏幕阅读器、Inspect.exe 与第三方自动化工具就能读懂并操作界面——
//! 这是「Win11 原生感」的无障碍维度，与 Agent 能力互不替代。
//!
//! 本模块属于平台层的**可选适配器**：任何用 unbound-ui 的程序都能一键开启，
//! 业务代码里不需要出现一行 UIA 专用逻辑。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_windows::{SubclassingAdapter, HWND};
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

/// UIA 的激活处理器。
///
/// 窗口刚创建时语义树还不存在，因此这里**不**凭空造一棵占位树：
/// AccessKit 允许首次激活返回 `None`，随后由第一次 `update_if_active`
/// 送出**完整树**。我们的语义树导出正是这么做的——首帧带 `tree`，
/// 之后才是增量。
///
/// 这里还兼职做一件关键的事：**记录辅助技术接入的时刻**。
/// 接入之前，适配器不消费任何更新；那时的发布会被丢弃，还会把
/// 「完整树」从语义树上摘走，等接入时只剩一截残缺的增量。
struct UiaActivation {
    /// 辅助技术是否已经接入。
    activated: Arc<AtomicBool>,
}

impl ActivationHandler for UiaActivation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.activated.store(true, Ordering::Release);
        None
    }
}

/// UIA 的动作处理器：把系统发来的动作请求排进通道，交给事件循环排空。
struct UiaActionHandler {
    sender: Sender<ActionRequest>,
}

impl ActionHandler for UiaActionHandler {
    fn do_action(&mut self, request: ActionRequest) {
        // 发送失败只说明接收端已经退场：动作无处可去，但不值得为此崩掉。
        let _ = self.sender.send(request);
    }
}

/// 原生窗口的 UI Automation 提供者。
///
/// 它通过 Windows 的窗口子类化接住 `WM_GETOBJECT`——那是 UIA Provider
/// 的激活入口。类化之后，屏幕阅读器与 Inspect.exe 就能拿到我们的窗口。
pub struct UiaProvider {
    adapter: SubclassingAdapter,
    /// 辅助技术是否已经接入。与 [`UiaActivation`] 共享同一份状态。
    activated: Arc<AtomicBool>,
}

impl UiaProvider {
    /// 为指定窗口创建提供者。
    ///
    /// 返回 `None` 表示**装不上**：Windows 的窗口子类化必须赶在窗口首次
    /// 显示之前，错过这个时机 `WM_GETOBJECT` 就再也送不到我们手里。
    /// 这时程序照常运行，只是没有 UIA 出口——比崩掉好。
    #[must_use]
    pub fn new(hwnd: HWND, sender: Sender<ActionRequest>) -> Option<Self> {
        // SAFETY: 只读取窗口可见性，不触碰窗口状态。
        if unsafe { IsWindowVisible(hwnd) }.into() {
            return None;
        }
        let activated = Arc::new(AtomicBool::new(false));
        let adapter = SubclassingAdapter::new(
            hwnd,
            UiaActivation {
                activated: Arc::clone(&activated),
            },
            UiaActionHandler { sender },
        );
        Some(Self { adapter, activated })
    }

    /// 辅助技术是否已经接入。
    ///
    /// 接入之前适配器不消费任何更新：那时的发布会被丢弃。更要紧的是，
    /// 那时若把「完整树」从语义树上摘走，等辅助技术真正接入时，
    /// 拿到的就只是一截残缺的增量——屏幕阅读器会看到一个空壳。
    #[must_use]
    pub fn is_activated(&self) -> bool {
        self.activated.load(Ordering::Acquire)
    }

    /// 发布语义树更新。
    ///
    /// 更新只有在 UIA **真正被激活**时才会被应用：没人在听的时候，
    /// 一棵几百节点的树不值得每帧构造。
    pub fn update(&mut self, update: TreeUpdate) {
        if let Some(events) = self.adapter.update_if_active(move || update) {
            events.raise();
        }
    }
}

//! Linux AT-SPI 无障碍适配器（基于纯 Rust `accesskit_unix`，零 `libatk-dev` / `libdbus-1-dev`）。
//!
//! 遵循二期 Spec（#213）：
//! - 每个顶层窗口在首次显示前拥有独立的 `accesskit_unix::Adapter` 与语义桥接；
//! - 采用惰性激活门禁：在辅助技术触发 `request_initial_tree` 前，`wants_update()` 恒返回 `false`，
//!   绝不提前消费 `Semantics::take_update()` 首次完整快照；
//! - 当环境缺失 AT-SPI / D-Bus 会话总线时自动降级为空传输层并仅记录一次降级日志，绝不阻塞或崩溃。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Once};

use accesskit::{ActionRequest, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};
use unbound_a11y::Semantics;

use crate::a11y_bridge::{engine_action, A11yBridge, A11yRequest, A11yTransport};

/// 跨平台窗口几何矩形（屏幕逻辑/物理边界，供 AT-SPI 同步窗口位置与尺寸）。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct A11yWindowBounds {
    /// 左上角 X 坐标。
    pub x: f64,
    /// 左上角 Y 坐标。
    pub y: f64,
    /// 宽度。
    pub width: f64,
    /// 高度。
    pub height: f64,
}

impl A11yWindowBounds {
    /// 构造无障碍窗口边界矩形。
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod unix_impl {
    use super::*;
    use accesskit::{ActionHandler, ActivationHandler, DeactivationHandler};

    static LOG_NO_BUS_ONCE: Once = Once::new();

    fn has_atspi_or_session_bus() -> bool {
        std::env::var_os("AT_SPI_BUS_ADDRESS").is_some_and(|v| !v.is_empty())
            || std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty())
            || std::env::var_os("XDG_RUNTIME_DIR")
                .map(|dir| std::path::PathBuf::from(dir).join("bus").exists())
                .unwrap_or(false)
    }

    struct UnixActivationHandler {
        activated: Arc<AtomicBool>,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    }

    impl ActivationHandler for UnixActivationHandler {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            self.activated.store(true, Ordering::Release);
            if let Some(wake) = &self.wake {
                wake();
            }
            let root_id = NodeId(0);
            let root = Node::new(Role::Window);
            Some(TreeUpdate {
                nodes: vec![(root_id, root)],
                tree: Some(TreeInfo::new(root_id)),
                tree_id: TreeId::ROOT,
                focus: root_id,
            })
        }
    }

    struct UnixActionHandler {
        sender: Sender<ActionRequest>,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    }

    impl ActionHandler for UnixActionHandler {
        fn do_action(&mut self, request: ActionRequest) {
            let _ = self.sender.send(request);
            if let Some(wake) = &self.wake {
                wake();
            }
        }
    }

    struct UnixDeactivationHandler {
        activated: Arc<AtomicBool>,
    }

    impl DeactivationHandler for UnixDeactivationHandler {
        fn deactivate_accessibility(&mut self) {
            self.activated.store(false, Ordering::Release);
        }
    }

    pub(super) struct InnerAdapter {
        adapter: Option<accesskit_unix::Adapter>,
        activated: Arc<AtomicBool>,
    }

    impl InnerAdapter {
        pub(super) fn new(
            sender: Sender<ActionRequest>,
            wake: Option<Arc<dyn Fn() + Send + Sync>>,
            activated: Arc<AtomicBool>,
        ) -> Self {
            if !has_atspi_or_session_bus() {
                LOG_NO_BUS_ONCE.call_once(|| {
                    log::debug!(
                        "AT-SPI / D-Bus session bus unavailable; Linux accessibility degraded to no-op"
                    );
                });
                return Self {
                    adapter: None,
                    activated,
                };
            }
            let adapter = accesskit_unix::Adapter::new(
                UnixActivationHandler {
                    activated: Arc::clone(&activated),
                    wake: wake.clone(),
                },
                UnixActionHandler { sender, wake },
                UnixDeactivationHandler {
                    activated: Arc::clone(&activated),
                },
            );
            Self {
                adapter: Some(adapter),
                activated,
            }
        }

        pub(super) fn send(&mut self, update: TreeUpdate) {
            if let Some(adapter) = &mut self.adapter {
                adapter.update_if_active(|| update);
            }
        }

        pub(super) fn wants_update(&self) -> bool {
            self.adapter.is_some() && self.activated.load(Ordering::Acquire)
        }

        pub(super) fn update_focus(&mut self, focused: bool) {
            if let Some(adapter) = &mut self.adapter {
                adapter.update_window_focus_state(focused);
            }
        }

        pub(super) fn update_bounds(&mut self, bounds: A11yWindowBounds) {
            if let Some(adapter) = &mut self.adapter {
                let rect = Rect {
                    x0: bounds.x,
                    y0: bounds.y,
                    x1: bounds.x + bounds.width.max(0.0),
                    y1: bounds.y + bounds.height.max(0.0),
                };
                adapter.set_root_window_bounds(rect, rect);
            }
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"), not(target_os = "android"))))]
mod unix_impl {
    use super::*;

    pub(super) struct InnerAdapter {
        activated: Arc<AtomicBool>,
    }

    impl InnerAdapter {
        pub(super) fn new(
            _sender: Sender<ActionRequest>,
            _wake: Option<Arc<dyn Fn() + Send + Sync>>,
            activated: Arc<AtomicBool>,
        ) -> Self {
            Self { activated }
        }

        pub(super) fn send(&mut self, _update: TreeUpdate) {}

        pub(super) fn wants_update(&self) -> bool {
            self.activated.load(Ordering::Acquire)
        }

        pub(super) fn update_focus(&mut self, _focused: bool) {}

        pub(super) fn update_bounds(&mut self, _bounds: A11yWindowBounds) {}
    }
}

/// Linux AT-SPI 无障碍更新传输层。
struct LinuxA11yTransport {
    inner: unix_impl::InnerAdapter,
}

impl A11yTransport for LinuxA11yTransport {
    fn send(&mut self, update: TreeUpdate) {
        self.inner.send(update);
    }

    fn wants_update(&self) -> bool {
        self.inner.wants_update()
    }
}

/// 单个 Linux 顶层窗口的独立无障碍适配器。
pub struct LinuxA11y {
    window_id: u64,
    bridge: A11yBridge<LinuxA11yTransport>,
    activated: Arc<AtomicBool>,
    sender: Sender<ActionRequest>,
    requests: Receiver<ActionRequest>,
    published: u64,
}

impl LinuxA11y {
    /// 为指定顶层窗口创建独立的 Linux AT-SPI 适配器（在窗口首次显示前调用）。
    #[must_use]
    pub fn new(window_id: u64, wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let (sender, requests) = channel();
        let activated = Arc::new(AtomicBool::new(false));
        let transport = LinuxA11yTransport {
            inner: unix_impl::InnerAdapter::new(sender.clone(), wake, Arc::clone(&activated)),
        };
        let mut bridge = A11yBridge::new(transport);
        bridge.set_enabled(true);
        Self {
            window_id,
            bridge,
            activated,
            sender,
            requests,
            published: 0,
        }
    }

    /// 关联的顶层窗口 ID。
    #[must_use]
    pub const fn window_id(&self) -> u64 {
        self.window_id
    }

    /// 获取入站动作请求发送端（供测试或外部桥接注入）。
    #[must_use]
    pub fn request_sender(&self) -> Sender<ActionRequest> {
        self.sender.clone()
    }

    /// 当前是否已有辅助技术完成惰性激活。
    #[must_use]
    pub fn is_activated(&self) -> bool {
        self.activated.load(Ordering::Acquire)
    }

    /// 启用或停用无障碍更新发布。
    pub fn set_active(&mut self, active: bool) {
        self.bridge.set_enabled(active);
    }

    /// 同步顶层窗口键盘焦点状态至 AT-SPI。
    pub fn set_focus(&mut self, focused: bool) {
        self.bridge.transport_mut().inner.update_focus(focused);
    }

    /// 同步顶层窗口几何边界至 AT-SPI。
    pub fn set_window_bounds(&mut self, bounds: A11yWindowBounds) {
        self.bridge.transport_mut().inner.update_bounds(bounds);
    }

    /// 若辅助技术已激活，则消费并发布语义树增量。
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

    /// 通过惰性闭包发布语义树增量：未激活时不调用闭包，保护首次完整快照。
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

    /// 排空来自辅助技术的待处理动作请求。
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

    /// 已跨越边界发布的更新次数。
    #[must_use]
    pub const fn published(&self) -> u64 {
        self.published
    }
}

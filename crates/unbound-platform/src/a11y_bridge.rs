//! 语义树与无障碍适配器之间的桥接层。
//!
//! [`Semantics`] 已负责生成增量 `TreeUpdate`；本模块负责其两侧的双向通道：
//!
//! *
//! * **输出方向（out）**—— 将更新交给传输层（在 Windows 上由
//!   `accesskit_winit::Adapter` 推送至 UI Automation）；
//! * **输入方向（in）**—— 将来自辅助技术的 `ActionRequest` 翻译回引擎的
//!   [`A11yAction`] 及目标节点 ID，使应用 reducer 能像处理指针或键盘事件一样
//!   处理它。
//!
//! 将传输层抽象为 trait 使双向往返可在无头机器上测试：测试可记录跨边界的
//! 更新内容，并向回注入合成的动作请求。
//!

use accesskit::{Action, ActionData, ActionRequest, NodeId, TreeUpdate};
use unbound_a11y::{A11yAction, Semantics};

/// 无障碍更新的输出目标。
pub trait A11yTransport {
    /// 向平台适配器发送一次更新。
    fn send(&mut self, update: TreeUpdate);
}

/// 引擎词汇表中的单条动作请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A11yRequest {
    /// 目标节点 ID。
    pub node: u64,
    /// 请求执行的动作。
    pub action: A11yAction,
    /// `SetValue` 动作携带的新值。
    pub value: Option<String>,
}

/// 双向无障碍桥接器。
pub struct A11yBridge<T: A11yTransport> {
    transport: T,
    enabled: bool,
    updates: u64,
    requests: u64,
}

impl<T: A11yTransport> A11yBridge<T> {
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            enabled: false,
            updates: 0,
            requests: 0,
        }
    }

    /// 在平台报告有辅助技术正在监听之前，无障碍功能处于关闭状态。
    /// 关闭状态下调用发布不会产生任何开销。
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 获取内部传输层，供需要直达平台适配器的外壳使用。
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// 发布待处理的语义增量（若有）。
    ///
    /// 返回是否有更新跨越边界。[`Semantics::reset`] 后的首次调用为完整快照，
    /// 后续调用均为增量更新。
    pub fn publish(&mut self, semantics: &mut Semantics) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(update) = semantics.take_update() else {
            return false;
        };
        self.transport.send(update);
        self.updates += 1;
        true
    }

    /// 翻译一条传入的动作请求。
    ///
    /// 当无障碍未启用或该动作在引擎中无对应项时返回 `None`。
    ///
    pub fn translate(&mut self, request: &ActionRequest) -> Option<A11yRequest> {
        if !self.enabled {
            return None;
        }
        let action = engine_action(request.action)?;
        let value = match &request.data {
            Some(ActionData::Value(value)) => Some(value.as_ref().to_owned()),
            _ if action == A11yAction::SetValue => None,
            _ => None,
        };
        self.requests += 1;
        Some(A11yRequest {
            node: request.target_node.0,
            action,
            value,
        })
    }

    /// 跨越边界的更新总次数。
    #[must_use]
    pub fn updates(&self) -> u64 {
        self.updates
    }

    /// 跨越边界的动作请求总次数。
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.requests
    }
}

/// 将 AccessKit 动作映射为引擎动作集。
#[must_use]
pub fn engine_action(action: Action) -> Option<A11yAction> {
    Some(match action {
        Action::Click => A11yAction::Click,
        Action::Focus => A11yAction::Focus,
        Action::Blur => A11yAction::Blur,
        Action::SetValue => A11yAction::SetValue,
        Action::Increment => A11yAction::Increment,
        Action::Decrement => A11yAction::Decrement,
        Action::Expand => A11yAction::Expand,
        Action::Collapse => A11yAction::Collapse,
        Action::ScrollIntoView => A11yAction::ScrollIntoView,
        Action::ShowContextMenu => A11yAction::ShowContextMenu,
        _ => return None,
    })
}

/// 构建合成动作请求（供测试及暴露自定义协议的适配器使用）。
///
#[must_use]
pub fn action_request(action: Action, node: u64, value: Option<&str>) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: accesskit::TreeId::ROOT,
        target_node: NodeId(node),
        data: value.map(|value| ActionData::Value(value.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_a11y::Semantics;
    use unbound_math::Rect;

    #[derive(Default)]
    struct Recorder {
        updates: Vec<TreeUpdate>,
    }

    impl A11yTransport for Recorder {
        fn send(&mut self, update: TreeUpdate) {
            self.updates.push(update);
        }
    }

    fn tree() -> Semantics {
        let mut semantics = Semantics::new(1);
        semantics.upsert(
            5,
            unbound_a11y::A11yNode::new(
                accesskit::Role::Button,
                "Save",
                Rect::new(0.0, 0.0, 80.0, 30.0),
            )
            .clickable(),
        );
        semantics.set_focus(5);
        semantics
    }

    #[test]
    fn publishing_forwards_one_update_per_dirty_tree() {
        let mut bridge = A11yBridge::new(Recorder::default());
        let mut semantics = tree();

        assert!(
            !bridge.publish(&mut semantics),
            "disabled publishes nothing"
        );

        bridge.set_enabled(true);
        assert!(bridge.publish(&mut semantics));
        assert!(!bridge.publish(&mut semantics), "nothing new to send");
        assert_eq!(bridge.transport_mut().updates.len(), 1);

        let update = &bridge.transport_mut().updates[0];
        assert!(update
            .nodes
            .iter()
            .any(|(id, node)| { id.0 == 5 && node.label() == Some("Save") }));
    }

    #[test]
    fn a_click_request_round_trips_to_the_engine_action() {
        let mut bridge = A11yBridge::new(Recorder::default());
        bridge.set_enabled(true);
        let request = action_request(Action::Click, 5, None);
        let translated = bridge.translate(&request).expect("click is supported");
        assert_eq!(translated.node, 5);
        assert_eq!(translated.action, A11yAction::Click);
        assert_eq!(translated.value, None);
    }

    #[test]
    fn a_set_value_request_carries_the_new_text() {
        let mut bridge = A11yBridge::new(Recorder::default());
        bridge.set_enabled(true);
        let request = action_request(Action::SetValue, 9, Some("hello"));
        let translated = bridge.translate(&request).expect("set-value is supported");
        assert_eq!(translated.action, A11yAction::SetValue);
        assert_eq!(translated.value.as_deref(), Some("hello"));
    }

    #[test]
    fn unknown_and_disabled_requests_are_dropped() {
        let mut bridge = A11yBridge::new(Recorder::default());
        bridge.set_enabled(false);
        assert!(bridge
            .translate(&action_request(Action::Click, 5, None))
            .is_none());
        bridge.set_enabled(true);
        assert!(bridge
            .translate(&action_request(Action::ScrollLeft, 5, None))
            .is_none());
    }
}

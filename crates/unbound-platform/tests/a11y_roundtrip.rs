//! 通过公共 API 测试无障碍桥接器的双向往返。

use accesskit::{Action, TreeUpdate};
use unbound_a11y::{A11yAction, Semantics};
use unbound_math::Rect;
use unbound_platform::{action_request, A11yBridge, A11yTransport};

#[derive(Default)]
struct Recorder {
    updates: Vec<TreeUpdate>,
}

impl A11yTransport for Recorder {
    fn send(&mut self, update: TreeUpdate) {
        self.updates.push(update);
    }
}

fn semantics() -> Semantics {
    let mut semantics = Semantics::new(1);
    semantics.upsert(
        50,
        unbound_a11y::A11yNode::new(
            accesskit::Role::Button,
            "Publish",
            Rect::new(10.0, 10.0, 90.0, 32.0),
        )
        .clickable(),
    );
    semantics.set_focus(50);
    semantics
}

#[test]
fn an_update_reaches_the_transport_only_when_enabled() {
    let mut bridge = A11yBridge::new(Recorder::default());
    let mut tree = semantics();

    assert!(!bridge.publish(&mut tree));
    bridge.set_enabled(true);
    assert!(bridge.publish(&mut tree));
    assert_eq!(bridge.transport_mut().updates.len(), 1);
    assert!(!bridge.publish(&mut tree), "the delta is consumed");
}

#[test]
fn a_platform_click_becomes_an_engine_request() {
    let mut bridge = A11yBridge::new(Recorder::default());
    bridge.set_enabled(true);

    let translated = bridge
        .translate(&action_request(Action::Click, 50, None))
        .expect("click maps to A11yAction::Click");
    assert_eq!(translated.node, 50);
    assert_eq!(translated.action, A11yAction::Click);
    assert_eq!(bridge.requests(), 1);
}

#[test]
fn set_value_carries_the_text_and_unknown_actions_are_dropped() {
    let mut bridge = A11yBridge::new(Recorder::default());
    bridge.set_enabled(true);

    let translated = bridge
        .translate(&action_request(Action::SetValue, 7, Some("hello")))
        .expect("set-value maps");
    assert_eq!(translated.action, A11yAction::SetValue);
    assert_eq!(translated.value.as_deref(), Some("hello"));

    assert!(bridge
        .translate(&action_request(Action::ScrollLeft, 7, None))
        .is_none());
}

/// 模拟 UI Automation：辅助技术接入之前不消费任何更新。
struct DeafTransport {
    /// 是否已接入。
    active: bool,
    updates: Vec<TreeUpdate>,
}

impl A11yTransport for DeafTransport {
    fn send(&mut self, update: TreeUpdate) {
        self.updates.push(update);
    }

    fn wants_update(&self) -> bool {
        self.active
    }
}

#[test]
fn a_deaf_transport_leaves_the_semantic_tree_untouched() {
    let mut bridge = A11yBridge::new(DeafTransport {
        active: false,
        updates: Vec::new(),
    });
    bridge.set_enabled(true);
    let mut tree = semantics();

    // 没人听的时候，一次发布都不该发生——更重要的是，语义树不能被摘走。
    assert!(!bridge.publish(&mut tree));
    assert_eq!(bridge.transport_mut().updates.len(), 0);

    // 接入之后，第一次发布必须是一棵**完整**的树。
    bridge.transport_mut().active = true;
    assert!(bridge.publish(&mut tree));
    let update = &bridge.transport_mut().updates[0];
    assert!(
        update.tree.is_some(),
        "the first update after activation must carry the full tree"
    );
    assert!(
        update.nodes.iter().any(|(id, _)| id.0 == 50),
        "the full tree must carry every node, not just the changed ones"
    );
}

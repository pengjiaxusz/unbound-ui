//! The bridge between the semantic tree and an accessibility adapter.
//!
//! [`Semantics`] already produces incremental `TreeUpdate`s; this module owns
//! the two directions around it:
//!
//! * **out** — hand the update to a transport (on Windows, an
//!   `accesskit_winit::Adapter` pushes it into UI Automation),
//! * **in** — translate an `ActionRequest` from assistive technology back into
//!   the engine's [`A11yAction`] plus the target node id, so the application's
//!   reducer can handle it exactly like a pointer or keyboard event.
//!
//! Keeping the transport a trait is what makes the round trip testable on a
//! headless machine: the test records what would have crossed the boundary and
//! feeds a synthetic request back in.

use accesskit::{Action, ActionData, ActionRequest, NodeId, TreeUpdate};
use unbound_a11y::{A11yAction, Semantics};

/// Where accessibility updates go.
pub trait A11yTransport {
    /// Send one update to the platform adapter.
    fn send(&mut self, update: TreeUpdate);
}

/// One action request, in the engine's vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A11yRequest {
    /// The target node id.
    pub node: u64,
    /// The requested action.
    pub action: A11yAction,
    /// The new value for `SetValue`.
    pub value: Option<String>,
}

/// The two-way accessibility bridge.
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

    /// Accessibility is off until the platform says an assistive technology
    /// is listening. Publishing while disabled costs nothing.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The transport, for a shell that needs to reach the platform adapter.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Publish the pending semantic delta, if any.
    ///
    /// Returns whether an update crossed the boundary. The first call after a
    /// [`Semantics::reset`] is a full snapshot; later calls are deltas.
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

    /// Translate an incoming action request.
    ///
    /// Returns `None` when accessibility is disabled or the action has no
    /// engine equivalent.
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

    /// How many updates crossed the boundary.
    #[must_use]
    pub fn updates(&self) -> u64 {
        self.updates
    }

    /// How many action requests crossed the boundary.
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.requests
    }
}

/// Map an AccessKit action to the engine's action set.
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

/// Build a synthetic request (used by tests and by adapters that expose a
/// custom protocol).
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

//! The real platform accessibility adapter (AccessKit + winit).
//!
//! [`super::a11y_bridge`] owns the engine-side contract; this module is the
//! transport that makes it reach the OS. The adapter is created *before* the
//! window is shown (AccessKit's requirement), publishes one incremental
//! `TreeUpdate` per dirty frame, and queues incoming `ActionRequest`s on a
//! channel that the event loop drains — handlers may be called on any thread,
//! so they cannot touch the application state directly.
//!
//! When no assistive technology is listening, `update_if_active` never calls
//! the updater: a static frame costs no system call.

use std::sync::mpsc::{channel, Receiver, Sender};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use unbound_a11y::Semantics;

use super::a11y_bridge::{engine_action, A11yRequest};

/// The inbound half: requests coming from the OS.
///
/// `std::sync::mpsc::Sender` is `Send` but not `Sync`; the adapter only asks
/// for `Send`, so wrapping it keeps the bound satisfied without changing the
/// channel semantics.
struct RequestSender(Sender<ActionRequest>);

impl ActionHandler for RequestSender {
    fn do_action(&mut self, request: ActionRequest) {
        // A closed channel means the window is gone; dropping the request is
        // the only sane response.
        let _ = self.0.send(request);
    }
}

struct Activation;

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // The application publishes a full tree on the first frame through
        // `publish`; returning `None` lets the platform use a placeholder
        // until that frame lands.
        None
    }
}

struct Deactivation;

impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {}
}

/// The window's accessibility adapter.
pub struct WinitA11y {
    adapter: accesskit_winit::Adapter,
    requests: Receiver<ActionRequest>,
    published: u64,
}

impl WinitA11y {
    /// Create the adapter for a window that is still invisible.
    ///
    /// # Panics
    /// AccessKit panics when the window is already visible; the backend
    /// therefore creates the window hidden and shows it afterwards.
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

    /// Feed one window event to the adapter.
    ///
    /// Must be called before the event is handled by the application, so the
    /// adapter sees the same event stream the UI does.
    pub fn process_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        self.adapter.process_event(window, event);
    }

    /// Publish the pending semantic delta, if an assistive technology is
    /// listening.
    ///
    /// Returns whether an update crossed the boundary. `take_update` is only
    /// called when the tree actually has pending changes, so a static frame
    /// allocates nothing.
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

    /// Publish through a closure, so the caller can build the update lazily
    /// inside `update_if_active` (the tree is not consumed when no client is
    /// listening).
    pub fn publish_with(&mut self, update: impl FnOnce() -> Option<TreeUpdate>) -> bool {
        let Some(update) = update() else {
            return false;
        };
        self.adapter.update_if_active(move || update);
        self.published += 1;
        true
    }

    /// Drain the action requests the OS queued since the last frame.
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

    /// How many updates crossed the boundary.
    #[must_use]
    pub fn published(&self) -> u64 {
        self.published
    }
}

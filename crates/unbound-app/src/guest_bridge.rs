//! Hosting a UBMF guest as a regular [`Application`].
//!
//! The guest owns its state behind the C ABI; the host owns the node arena the
//! guest builds every view. This adapter is the bridge: it asks the guest for a
//! tree, translates each node into the declarative widgets the runtime knows,
//! and sends bound actions back through [`GuestHost::reduce`] when those
//! widgets are clicked.
//!
//! The translation is intentionally mechanical. A node's [`NodeTag`] decides
//! the widget: a `Text` leaf with a bound action becomes a button (that is how
//! the guest expresses "this row is clickable"), any other `Text` leaf becomes
//! a paragraph, and everything with children becomes a container that inherits
//! the guest's own layout descriptor. `Scroll`, `Portal` and `Canvas` keep
//! their container shape until the host grows dedicated widgets for them.

use rustc_hash::FxHashMap;

use unbound_abi::action::RawAction;
use unbound_abi::desc::{NodeTag, RawNodeDesc, RawNodeId};
use unbound_host::{GuestHost, NodeArena};
use unbound_math::Rect;
use unbound_text::TextStyle;
use unbound_widgets::{ButtonDesc, ButtonVariant};

use crate::{AppContext, AppEvent, Application, DirtyFlags, EventOutcome, ViewBuilder, WidgetSpec};

/// The application wrapper around a mounted guest.
pub struct GuestAppAdapter {
    host: GuestHost,
    /// The action each widget key is bound to, rebuilt with every view.
    actions: FxHashMap<u64, RawAction>,
}

impl GuestAppAdapter {
    /// Wrap an already-mounted host.
    ///
    /// The host may be empty; [`Application::view`] then paints a short
    /// diagnostic instead of failing.
    #[must_use]
    pub fn new(host: GuestHost) -> Self {
        Self {
            host,
            actions: FxHashMap::default(),
        }
    }

    /// The mounted host.
    #[must_use]
    pub fn host(&self) -> &GuestHost {
        &self.host
    }

    /// Mutable access, for mounting or hot-reloading a vtable.
    pub fn host_mut(&mut self) -> &mut GuestHost {
        &mut self.host
    }

    /// The action bound to one widget key, if the last view bound one.
    #[must_use]
    pub fn action_for(&self, key: u64) -> Option<&RawAction> {
        self.actions.get(&key)
    }
}

impl Application for GuestAppAdapter {
    /// The guest owns the state; the host-side state is unit.
    type State = ();

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {}

    fn reduce(
        &mut self,
        _state: &mut Self::State,
        event: AppEvent,
        _ctx: &mut AppContext,
    ) -> EventOutcome {
        let AppEvent::Click { node, .. } = event else {
            return EventOutcome::default();
        };
        let Some(action) = self.actions.get(&node).copied() else {
            return EventOutcome::default();
        };
        // A guest that refuses an action keeps its old view; the next frame
        // simply repaints it unchanged.
        if self.host.reduce(&action).is_err() {
            return EventOutcome::default();
        }
        EventOutcome::new(DirtyFlags::PAINT | DirtyFlags::A11Y)
    }

    fn view(&mut self, _state: &Self::State, view: &mut ViewBuilder<'_>) {
        // The guest rebuilds its arena from its (possibly newly reduced) state.
        let built = self.host.build().is_ok();
        self.actions.clear();
        let arena = self.host.arena();
        if !built || arena.is_empty() {
            view.text(1, "guest not mounted", TextStyle::sized(16.0));
            return;
        }
        if let Some(root) = arena
            .finished_root()
            .or_else(|| arena.roots().first().copied())
        {
            emit_node(arena, root, view, &mut self.actions);
        }
    }
}

/// Translate one arena node and, recursively, its children.
fn emit_node(
    arena: &NodeArena,
    id: RawNodeId,
    view: &mut ViewBuilder<'_>,
    actions: &mut FxHashMap<u64, RawAction>,
) {
    let Some(node) = arena.get(id) else {
        return;
    };
    // Ids start at 1, so 0 stays free for "no node".
    let key = u64::from(node.id.0);
    if let Some(action) = node.action {
        actions.insert(key, action);
    }
    let desc: RawNodeDesc = node.desc;
    let tag = NodeTag::from_u8(desc.tag);
    if tag == NodeTag::Text && node.children.is_empty() {
        if node.action.is_some() {
            let mut button = ButtonDesc::new(Rect::new(0.0, 0.0, 320.0, 28.0), &node.text);
            button.variant = ButtonVariant::Secondary;
            view.widget(key, desc, WidgetSpec::Button(button));
        } else {
            let size = if desc.font_size > 0.0 {
                desc.font_size
            } else {
                14.0
            };
            view.text(key, node.text.clone(), TextStyle::sized(size));
        }
        return;
    }
    // Containers keep the guest's direction, gaps and sizing verbatim: the
    // layout engine is the same one the host UI uses.
    let children = node.children.clone();
    view.container(key, desc, |inner| {
        for child in children {
            emit_node(arena, child, inner, actions);
        }
    });
}

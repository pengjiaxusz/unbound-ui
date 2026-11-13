//! Type-safe view router stack & breadcrumb synchronization (S5.4).

use unbound_widgets::breadcrumb::Crumb;

/// Direction of the active route transition animation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TransitionDirection {
    /// No transition currently playing.
    #[default]
    None,
    /// Pushing or navigating forward.
    Forward,
    /// Popping or navigating backward.
    Backward,
}

/// A single entry on the [`RouterStack`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteEntry {
    /// Stable route identity.
    pub id: u64,
    /// Route name / path segment (e.g. `"settings"` or `"users/:id"`).
    pub name: String,
    /// Human-readable label shown in breadcrumbs.
    pub label: String,
    /// Key-value route parameters.
    pub params: Vec<(String, String)>,
}

impl RouteEntry {
    /// Create a new route entry.
    #[must_use]
    pub fn new(id: u64, name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            label: label.into(),
            params: Vec::new(),
        }
    }

    /// Attach a route parameter.
    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    /// Look up a route parameter by key.
    #[must_use]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// History-backed view navigation stack with forward/back, transition progress,
/// and automatic [`BreadcrumbItem`] generation.
#[derive(Clone, Debug)]
pub struct RouterStack {
    back_stack: Vec<RouteEntry>,
    forward_stack: Vec<RouteEntry>,
    transition_direction: TransitionDirection,
    transition_progress: f32,
    transition_duration: f32,
}

impl RouterStack {
    /// Create a router seeded with an initial root route.
    #[must_use]
    pub fn new(root: RouteEntry) -> Self {
        Self {
            back_stack: vec![root],
            forward_stack: Vec::new(),
            transition_direction: TransitionDirection::None,
            transition_progress: 1.0,
            transition_duration: 0.22,
        }
    }

    /// Current active route (top of `back_stack`).
    #[must_use]
    pub fn current(&self) -> &RouteEntry {
        self.back_stack
            .last()
            .expect("RouterStack always contains at least the root route")
    }

    /// Full active breadcrumb trail from root to current route.
    #[must_use]
    pub fn history(&self) -> &[RouteEntry] {
        &self.back_stack
    }

    /// Routes available for [`Self::forward`].
    #[must_use]
    pub fn forward_history(&self) -> &[RouteEntry] {
        &self.forward_stack
    }

    /// Push a new route onto the stack, clearing the forward stack.
    pub fn push(&mut self, entry: RouteEntry) {
        self.back_stack.push(entry);
        self.forward_stack.clear();
        self.transition_direction = TransitionDirection::Forward;
        self.transition_progress = 0.0;
    }

    /// Replace the current top route without growing history.
    pub fn replace(&mut self, entry: RouteEntry) {
        if let Some(last) = self.back_stack.last_mut() {
            *last = entry;
        } else {
            self.back_stack.push(entry);
        }
        self.transition_direction = TransitionDirection::Forward;
        self.transition_progress = 0.0;
    }

    /// Pop the current route onto the forward stack (never pops the root).
    pub fn pop(&mut self) -> bool {
        if self.back_stack.len() <= 1 {
            return false;
        }
        if let Some(popped) = self.back_stack.pop() {
            self.forward_stack.push(popped);
            self.transition_direction = TransitionDirection::Backward;
            self.transition_progress = 0.0;
            true
        } else {
            false
        }
    }

    /// Navigate forward to the most recently popped route.
    pub fn forward(&mut self) -> bool {
        if let Some(next) = self.forward_stack.pop() {
            self.back_stack.push(next);
            self.transition_direction = TransitionDirection::Forward;
            self.transition_progress = 0.0;
            true
        } else {
            false
        }
    }

    /// Pop back to a specific route index in the history trail (e.g. clicking a breadcrumb).
    pub fn pop_to_index(&mut self, index: usize) -> bool {
        if index >= self.back_stack.len().saturating_sub(1) {
            return false;
        }
        while self.back_stack.len() - 1 > index {
            if let Some(popped) = self.back_stack.pop() {
                self.forward_stack.push(popped);
            }
        }
        self.transition_direction = TransitionDirection::Backward;
        self.transition_progress = 0.0;
        true
    }

    /// Whether [`Self::pop`] can go back.
    #[must_use]
    pub fn can_pop(&self) -> bool {
        self.back_stack.len() > 1
    }

    /// Whether [`Self::forward`] can advance.
    #[must_use]
    pub fn can_forward(&self) -> bool {
        !self.forward_stack.is_empty()
    }

    /// Current transition direction and normalized `[0.0, 1.0]` progress.
    #[must_use]
    pub fn transition(&self) -> (TransitionDirection, f32) {
        (self.transition_direction, self.transition_progress)
    }

    /// Advance the route transition animation by `dt` seconds.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.transition_direction == TransitionDirection::None {
            return false;
        }
        let dur = self.transition_duration.max(0.001);
        self.transition_progress = (self.transition_progress + dt.max(0.0) / dur).min(1.0);
        if self.transition_progress >= 1.0 {
            self.transition_direction = TransitionDirection::None;
        }
        true
    }

    /// Generate [`Crumb`]s synchronized with the current route stack.
    #[must_use]
    pub fn breadcrumb_items(&self) -> Vec<Crumb> {
        self.back_stack
            .iter()
            .map(|entry| Crumb::new(entry.id, entry.label.clone()))
            .collect()
    }
}

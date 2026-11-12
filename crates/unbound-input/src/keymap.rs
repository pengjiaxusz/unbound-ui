//! Context-aware keyboard shortcuts (`docs` §6.4).
//!
//! A key press is not a shortcut until the modifiers and the focused *scope*
//! agree. [`Keymap`] keeps the bindings and answers one question per press:
//! "which command does this chord run here, if any?".
//!
//! Two shapes are supported:
//!
//! * a single chord, e.g. `Ctrl+Shift+P`;
//! * a two-chord sequence, e.g. `Ctrl+K, Ctrl+S`, where the first chord only
//!   arms the sequence and the second one fires it within
//!   [`Keymap::sequence_window`].
//!
//! The context is a plain string the application owns (`"TextInput"`,
//! `"Canvas"`, `"Modal"`). A binding registered *without* a context is global;
//! a binding registered for the active context wins over it, which is how a
//! modal can shadow the application's own shortcut without unregistering it.

#![forbid(unsafe_op_in_unsafe_fn)]

/// One physical key in a chord.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyName {
    /// A character key, already mapped from the keyboard layout.
    Char(char),
    /// A named key such as `"Enter"` or `"ArrowLeft"`.
    Named(&'static str),
    /// A function key, `F1`..`F24`.
    Function(u8),
}

/// A key plus the modifiers that must be held with it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// The key itself.
    pub key: KeyName,
    /// `Ctrl` / `Cmd` style modifier (the platform decides which).
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Windows / Super / Command.
    pub meta: bool,
}

impl KeyChord {
    /// A chord with no modifiers.
    #[must_use]
    pub const fn new(key: KeyName) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        }
    }

    /// A character chord with no modifiers.
    #[must_use]
    pub const fn character(character: char) -> Self {
        Self::new(KeyName::Char(character))
    }

    /// A named chord with no modifiers.
    #[must_use]
    pub const fn named(name: &'static str) -> Self {
        Self::new(KeyName::Named(name))
    }

    /// The same chord with `Ctrl` required.
    #[must_use]
    pub const fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// The same chord with `Shift` required.
    #[must_use]
    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// The same chord with `Alt` required.
    #[must_use]
    pub const fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// The same chord with `Meta` required.
    #[must_use]
    pub const fn with_meta(mut self) -> Self {
        self.meta = true;
        self
    }
}

/// One registered shortcut.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    /// The chord that fires the action.
    pub chord: KeyChord,
    /// The chord that arms a sequence, when this binding is two-chord.
    pub prefix: Option<KeyChord>,
    /// The scope the binding belongs to; `None` is global.
    pub context: Option<&'static str>,
    /// The application's command number.
    pub action: u64,
}

/// The pending state of a two-chord sequence.
#[derive(Copy, Clone, Debug, PartialEq)]
struct PendingSequence {
    prefix: KeyChord,
    started: f64,
}

/// The shortcut table.
#[derive(Debug)]
pub struct Keymap {
    bindings: Vec<KeyBinding>,
    pending: Option<PendingSequence>,
    sequence_window: f64,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

impl Keymap {
    /// How long a sequence prefix stays armed, in seconds.
    pub const DEFAULT_SEQUENCE_WINDOW: f64 = 1.0;

    /// An empty keymap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            pending: None,
            sequence_window: Self::DEFAULT_SEQUENCE_WINDOW,
        }
    }

    /// Override how long a sequence prefix stays armed.
    #[must_use]
    pub fn with_sequence_window(mut self, seconds: f64) -> Self {
        self.sequence_window = seconds.max(0.0);
        self
    }

    /// Register a single-chord shortcut.
    pub fn bind(&mut self, chord: KeyChord, context: Option<&'static str>, action: u64) {
        self.bindings.push(KeyBinding {
            chord,
            prefix: None,
            context,
            action,
        });
    }

    /// Register a two-chord shortcut.
    pub fn bind_sequence(
        &mut self,
        prefix: KeyChord,
        chord: KeyChord,
        context: Option<&'static str>,
        action: u64,
    ) {
        self.bindings.push(KeyBinding {
            chord,
            prefix: Some(prefix),
            context,
            action,
        });
    }

    /// The bindings, for a help overlay.
    #[must_use]
    pub fn bindings(&self) -> &[KeyBinding] {
        &self.bindings
    }

    /// The armed sequence prefix, if the user is halfway through one.
    #[must_use]
    pub fn pending(&self) -> Option<KeyChord> {
        self.pending.map(|pending| pending.prefix)
    }

    /// Drop an armed prefix (focus change, Escape, a click elsewhere).
    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// Feed one press and return the command it fires, if any.
    ///
    /// `now` is the runtime clock in seconds; it only matters for the sequence
    /// window, so a test can pass `0.0` for every press.
    pub fn feed(&mut self, chord: KeyChord, context: Option<&str>, now: f64) -> Option<u64> {
        if let Some(pending) = self.pending.take() {
            if now - pending.started <= self.sequence_window {
                let hit = self
                    .bindings
                    .iter()
                    .rev()
                    .find(|binding| {
                        binding.prefix == Some(pending.prefix)
                            && binding.chord == chord
                            && Self::context_matches(binding.context, context)
                    })
                    .map(|binding| binding.action);
                if hit.is_some() {
                    return hit;
                }
                // The second chord was not part of the sequence. It may still
                // start or fire a binding of its own, so fall through.
            }
        }

        if let Some(action) = self.match_single(chord, context) {
            return Some(action);
        }

        let arms_sequence = self.bindings.iter().any(|binding| {
            binding.prefix == Some(chord) && Self::context_matches(binding.context, context)
        });
        if arms_sequence {
            self.pending = Some(PendingSequence {
                prefix: chord,
                started: now,
            });
        }
        None
    }

    /// A single-chord match. An exact context wins over a global binding;
    /// within the same scope the most recently registered binding wins.
    fn match_single(&self, chord: KeyChord, context: Option<&str>) -> Option<u64> {
        let exact = self.bindings.iter().rev().find(|binding| {
            binding.prefix.is_none()
                && binding.chord == chord
                && binding.context.is_some()
                && Self::context_matches(binding.context, context)
        });
        let global = self.bindings.iter().rev().find(|binding| {
            binding.prefix.is_none() && binding.chord == chord && binding.context.is_none()
        });
        exact.or(global).map(|binding| binding.action)
    }

    fn context_matches(binding: Option<&'static str>, active: Option<&str>) -> bool {
        match (binding, active) {
            (None, _) => true,
            (Some(binding), Some(active)) => binding == active,
            (Some(_), None) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_shift_p() -> KeyChord {
        KeyChord::character('p').with_ctrl().with_shift()
    }

    #[test]
    fn a_single_chord_fires_its_action() {
        let mut keymap = Keymap::new();
        keymap.bind(ctrl_shift_p(), None, 7);
        assert_eq!(keymap.feed(ctrl_shift_p(), None, 0.0), Some(7));
    }

    #[test]
    fn a_chord_with_the_wrong_modifiers_does_not_match() {
        let mut keymap = Keymap::new();
        keymap.bind(ctrl_shift_p(), None, 7);
        assert_eq!(
            keymap.feed(KeyChord::character('p').with_ctrl(), None, 0.0),
            None
        );
    }

    #[test]
    fn a_two_chord_sequence_fires_on_the_second_chord() {
        let mut keymap = Keymap::new();
        let prefix = KeyChord::character('k').with_ctrl();
        let chord = KeyChord::character('s').with_ctrl();
        keymap.bind_sequence(prefix, chord, None, 42);
        assert_eq!(keymap.feed(prefix, None, 0.0), None);
        assert_eq!(keymap.pending(), Some(prefix));
        assert_eq!(keymap.feed(chord, None, 0.2), Some(42));
        assert_eq!(keymap.pending(), None);
    }

    #[test]
    fn an_expired_prefix_does_not_fire_the_sequence() {
        let mut keymap = Keymap::new();
        let prefix = KeyChord::character('k').with_ctrl();
        let chord = KeyChord::character('s').with_ctrl();
        keymap.bind_sequence(prefix, chord, None, 42);
        assert_eq!(keymap.feed(prefix, None, 0.0), None);
        assert_eq!(keymap.feed(chord, None, 5.0), None);
    }

    #[test]
    fn an_exact_context_binding_wins_over_the_global_one() {
        let mut keymap = Keymap::new();
        keymap.bind(KeyChord::named("Escape"), None, 1);
        keymap.bind(KeyChord::named("Escape"), Some("Modal"), 2);
        assert_eq!(keymap.feed(KeyChord::named("Escape"), None, 0.0), Some(1));
        assert_eq!(
            keymap.feed(KeyChord::named("Escape"), Some("Modal"), 0.0),
            Some(2)
        );
        assert_eq!(
            keymap.feed(KeyChord::named("Escape"), Some("Canvas"), 0.0),
            Some(1),
            "an unrelated scope falls back to the global binding"
        );
    }

    #[test]
    fn a_scoped_sequence_needs_its_scope_to_arm() {
        let mut keymap = Keymap::new();
        let prefix = KeyChord::character('k').with_ctrl();
        let chord = KeyChord::character('s').with_ctrl();
        keymap.bind_sequence(prefix, chord, Some("Canvas"), 42);
        assert_eq!(keymap.feed(prefix, Some("TextInput"), 0.0), None);
        assert_eq!(keymap.pending(), None);
    }

    #[test]
    fn the_last_registered_binding_wins_inside_a_scope() {
        let mut keymap = Keymap::new();
        keymap.bind(ctrl_shift_p(), None, 1);
        keymap.bind(ctrl_shift_p(), None, 2);
        assert_eq!(keymap.feed(ctrl_shift_p(), None, 0.0), Some(2));
    }

    #[test]
    fn a_missed_sequence_lets_the_second_chord_act_on_its_own() {
        let mut keymap = Keymap::new();
        let prefix = KeyChord::character('k').with_ctrl();
        let other = KeyChord::character('x').with_ctrl();
        keymap.bind_sequence(prefix, KeyChord::character('s').with_ctrl(), None, 1);
        keymap.bind(other, None, 9);
        assert_eq!(keymap.feed(prefix, None, 0.0), None);
        assert_eq!(keymap.feed(other, None, 0.1), Some(9));
    }
}

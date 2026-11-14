//! 上下文感知键盘快捷键（`docs` §6.4）。
//!
//! 仅当修饰键与当前聚焦的**作用域（scope）**同时匹配时，按键才构成快捷键。
//! [`Keymap`] 维护所有绑定并在每次按键时回答同一个问题：
//! “该组合键在此处触发哪条命令（若有）？”。
//!
//! 支持两种按键形态：
//!
//! * 单组和弦键，例如 `Ctrl+Shift+P`；
//! * 双组序列键，例如 `Ctrl+K, Ctrl+S`，其中第一组和弦仅预备序列，
//!   第二组和弦在 [`Keymap::sequence_window`] 时间窗内触发命令。
//!
//!
//! 上下文是由应用持有的普通字符串（`"TextInput"`、`"Canvas"`、`"Modal"`）。
//! 未指定上下文的绑定为全局绑定；为当前活跃上下文注册的绑定优先级高于
//! 全局绑定，模态框借此无需注销即可遮蔽应用自身的快捷键。
//!

#![forbid(unsafe_op_in_unsafe_fn)]

/// 和弦中的单个物理按键。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyName {
    /// 已按键盘布局映射后的字符键。
    Char(char),
    /// 具名按键，如 `"Enter"` 或 `"ArrowLeft"`。
    Named(&'static str),
    /// 功能键，`F1`..`F24`。
    Function(u8),
}

/// 按键及必须同时按住的修饰键。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// 按键本身。
    pub key: KeyName,
    /// `Ctrl` / `Cmd` 风格修饰键（由平台决定具体映射）。
    pub ctrl: bool,
    /// Shift 键。
    pub shift: bool,
    /// Alt / Option 键。
    pub alt: bool,
    /// Windows / Super / Command 键。
    pub meta: bool,
}

impl KeyChord {
    /// 不带任何修饰键的和弦。
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

    /// 不带修饰键的字符和弦。
    #[must_use]
    pub const fn character(character: char) -> Self {
        Self::new(KeyName::Char(character))
    }

    /// 不带修饰键的具名按键和弦。
    #[must_use]
    pub const fn named(name: &'static str) -> Self {
        Self::new(KeyName::Named(name))
    }

    /// 要求同时按住 `Ctrl` 的同一和弦。
    #[must_use]
    pub const fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// 要求同时按住 `Shift` 的同一和弦。
    #[must_use]
    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// 要求同时按住 `Alt` 的同一和弦。
    #[must_use]
    pub const fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// 要求同时按住 `Meta` 的同一和弦。
    #[must_use]
    pub const fn with_meta(mut self) -> Self {
        self.meta = true;
        self
    }
}

/// 单条已注册的快捷键绑定。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    /// 触发动作的和弦。
    pub chord: KeyChord,
    /// 当该绑定为双组序列键时，用于预备序列的前缀和弦。
    pub prefix: Option<KeyChord>,
    /// 绑定所属的作用域；`None` 表示全局。
    pub context: Option<&'static str>,
    /// 应用侧的命令编号。
    pub action: u64,
}

/// 双组序列键的挂起等待状态。
#[derive(Copy, Clone, Debug, PartialEq)]
struct PendingSequence {
    prefix: KeyChord,
    started: f64,
}

/// 快捷键映射表。
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
    /// 序列前缀保持预备状态的时长（单位：秒）。
    pub const DEFAULT_SEQUENCE_WINDOW: f64 = 1.0;

    /// 创建一个空快捷键映射表。
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            pending: None,
            sequence_window: Self::DEFAULT_SEQUENCE_WINDOW,
        }
    }

    /// 覆盖序列前缀保持预备状态的时长。
    #[must_use]
    pub fn with_sequence_window(mut self, seconds: f64) -> Self {
        self.sequence_window = seconds.max(0.0);
        self
    }

    /// 注册单组和弦快捷键。
    pub fn bind(&mut self, chord: KeyChord, context: Option<&'static str>, action: u64) {
        self.bindings.push(KeyBinding {
            chord,
            prefix: None,
            context,
            action,
        });
    }

    /// 注册双组序列快捷键。
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

    /// 返回所有绑定列表，供快捷键帮助浮层展示。
    #[must_use]
    pub fn bindings(&self) -> &[KeyBinding] {
        &self.bindings
    }

    /// 若用户正处于序列按键中途，返回当前已预备的序列前缀。
    #[must_use]
    pub fn pending(&self) -> Option<KeyChord> {
        self.pending.map(|pending| pending.prefix)
    }

    /// 丢弃已预备的前缀（焦点变化、按下 Escape 或点击别处时调用）。
    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// 送入一次按键并返回其触发的命令（若有）。
    ///
    /// `now` 为运行时时钟（单位：秒）；仅用于序列时间窗判断，
    /// 因此测试中每次按键均可传入 `0.0`。
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
                // 第二组和弦不属于该序列，但它本身仍可能开启或触发另一条绑定，
                // 因此继续向下匹配。
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

    /// 单组和弦匹配：精确上下文优先于全局绑定；
    /// 同一作用域内，最近注册的绑定优先。
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

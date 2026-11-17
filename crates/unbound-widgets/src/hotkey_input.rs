//! 快捷键输入框（HotkeyInput / ShortcutRecorder）：捕获组合键并检测冲突。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion, KeyChord};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 控件圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
const HOTKEY_RADIUS: f32 = 4.0;

/// [`HotkeyInputDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotkeyInputState {
    /// 当前绑定的快捷键组合。
    pub chord: Option<KeyChord>,
    /// 是否正在积极监听下一次按键。
    pub recording: bool,
    /// 捕获的组合键是否与其他绑定冲突。
    pub conflict: bool,
}

/// 快捷键清除/录制指示按钮的确定性子节点 ID。
#[must_use]
pub const fn hotkey_clear_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x900
}

/// 将组合键格式化为标准的人类可读快捷键字符串（如 `"Ctrl+Shift+K"`、`"F12"`）。
#[must_use]
pub fn format_key_chord(chord: &KeyChord) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(5);
    if chord.ctrl {
        parts.push("Ctrl".to_owned());
    }
    if chord.alt {
        parts.push("Alt".to_owned());
    }
    if chord.shift {
        parts.push("Shift".to_owned());
    }
    if chord.meta {
        parts.push("Win".to_owned());
    }
    match chord.key {
        unbound_input::KeyName::Char(c) => {
            parts.push(c.to_ascii_uppercase().to_string());
        }
        unbound_input::KeyName::Named(name) => {
            parts.push(name.to_owned());
        }
        unbound_input::KeyName::Function(n) => {
            parts.push(format!("F{n}"));
        }
    }
    parts.join("+")
}

impl HotkeyInputState {
    /// 若已捕获快捷键，返回其格式化文本。
    #[must_use]
    pub fn formatted_chord(&self) -> Option<String> {
        self.chord.as_ref().map(format_key_chord)
    }
    /// 开始监听组合键。
    pub fn start_recording(&mut self) {
        self.recording = true;
        self.conflict = false;
    }

    /// 清除已绑定的快捷键并停止录制。
    pub fn clear(&mut self) {
        self.chord = None;
        self.recording = false;
        self.conflict = false;
    }

    /// 若 `recording` 为 true 则捕获 `chord`，并检查 `existing_bindings` 是否存在冲突。
    /// 若组合键无冲突并被接受则返回 `true`。
    pub fn handle_key(&mut self, chord: KeyChord, existing_bindings: &[KeyChord]) -> bool {
        if !self.recording {
            return false;
        }
        self.conflict = existing_bindings.contains(&chord);
        self.chord = Some(chord);
        self.recording = false;
        !self.conflict
    }
}

/// [`HotkeyInput`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct HotkeyInputDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 显示的组合键文本（例如 `"Ctrl+K"`）。
    pub shortcut_text: String,
    /// 是否正在积极录制。
    pub recording: bool,
    /// 是否激活冲突警告。
    pub conflict: bool,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl HotkeyInputDesc {
    /// 创建快捷键录制器描述符。
    #[must_use]
    pub fn new(rect: Rect, shortcut_text: impl Into<String>) -> Self {
        Self {
            rect,
            shortcut_text: shortcut_text.into(),
            recording: false,
            conflict: false,
            label: "Shortcut Input".to_owned(),
            z: 600,
        }
    }
}

/// 发射固定形状的快捷键录制器（1 背景 + 1 录制圆点 + 1 焦点环 = 3 个图元）。
#[must_use]
pub fn emit_hotkey_input(
    node: u64,
    desc: &HotkeyInputDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(3 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    // 描边对齐 WinUI 文本控件：常态 `ControlStrokeColorDefault`（1px），
    // 悬停 `ControlStrokeColorSecondary`，录制/聚焦用强调色，冲突用错误色。
    let border = if desc.conflict {
        Token::ERROR
    } else if desc.recording || state.focused {
        Token::PRIMARY
    } else if hovered {
        Token::INPUT_BORDER_HOVER
    } else {
        Token::CONTROL_STROKE
    };
    primitives.push(outlined(
        desc.rect,
        HOTKEY_RADIUS,
        // 填充 `ControlFillColorDefault`，悬停 `ControlFillColorSecondary`。
        if hovered {
            Token::CONTROL_HOVER
        } else {
            Token::CONTROL_FILL
        },
        border,
        1.0,
        0.0,
        desc.z,
    ));

    let capsule_r = Rect::new(
        desc.rect.origin.x + 6.0,
        desc.rect.origin.y + 4.0,
        (desc.rect.width() - 32.0).max(16.0),
        (desc.rect.height() - 8.0).max(12.0),
    );
    primitives.push(outlined(
        capsule_r,
        4.0,
        // 键帽底色比外框浅一档，读取为可按键位。
        if desc.shortcut_text.is_empty() {
            Token::CONTROL_FILL
        } else {
            Token::CONTROL_HOVER
        },
        Token::CONTROL_STROKE,
        1.0,
        0.0,
        desc.z + 1,
    ));

    let dot_r = Rect::new(
        desc.rect.max().x - 16.0,
        desc.rect.origin.y + (desc.rect.height() - 8.0) * 0.5,
        8.0,
        8.0,
    );
    primitives.push(surface(
        dot_r,
        4.0,
        if desc.conflict {
            Token::ERROR
        } else if desc.recording {
            Token::PRIMARY
        } else {
            Token::TEXT_SECONDARY
        },
        desc.z + 1,
    ));

    primitives.push(focus_ring(
        desc.rect,
        HOTKEY_RADIUS,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = desc.shortcut_text.clone();

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let clear_key = hotkey_clear_node(node);
        let clear_r = Rect::new(dot_r.origin.x - 4.0, dot_r.origin.y - 4.0, 16.0, 16.0);
        out.with_sub_control(
            clear_key,
            HitRegion::new(clear_key, CONTROL_LAYER, u32::from(desc.z) + 1, clear_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, "Clear shortcut", clear_r).clickable(),
            true,
        );
    }

    out
}

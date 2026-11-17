//! 底部上下文快捷键提示栏（`ShortcutBar`）与快捷键冲突检测状态机（`KeybindingManagerState`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 提示栏圆角。对齐 WinUI `ControlCornerRadius = 4`。
const BAR_RADIUS: f32 = 4.0;
/// 键帽圆角。对齐 WinUI `ControlCornerRadius = 4`。
const KEYCAP_RADIUS: f32 = 4.0;
/// 提示栏左右内边距。沿用既有 10px，与 `StatusBarDesc::padding` 同档。
const BAR_PADDING: f32 = 10.0;
/// 键帽与栏体上下边缘的间距（键帽内边距 4）。
const KEYCAP_INSET: f32 = 4.0;

/// 单个快捷键绑定条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub action_id: String,
    pub label: String,
    pub chord: String,
}

impl ShortcutEntry {
    #[must_use]
    pub fn new(
        action_id: impl Into<String>,
        label: impl Into<String>,
        chord: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            label: label.into(),
            chord: chord.into(),
        }
    }
}

/// 快捷键管理器状态（支持键位冲突实时检测）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeybindingManagerState {
    pub bindings: Vec<ShortcutEntry>,
}

impl KeybindingManagerState {
    #[must_use]
    pub fn new(bindings: Vec<ShortcutEntry>) -> Self {
        Self { bindings }
    }

    /// 检测共享同一组合键字符串的所有冲突动作对 `(action_a, action_b, chord)`。
    #[must_use]
    pub fn detect_conflicts(&self) -> Vec<(String, String, String)> {
        let mut conflicts = Vec::new();
        for i in 0..self.bindings.len() {
            for j in (i + 1)..self.bindings.len() {
                if !self.bindings[i].chord.is_empty()
                    && self.bindings[i]
                        .chord
                        .eq_ignore_ascii_case(&self.bindings[j].chord)
                {
                    conflicts.push((
                        self.bindings[i].action_id.clone(),
                        self.bindings[j].action_id.clone(),
                        self.bindings[i].chord.clone(),
                    ));
                }
            }
        }
        conflicts
    }
}

/// 底部上下文快捷键提示栏描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutBarDesc {
    pub rect: Rect,
    pub entries: Vec<ShortcutEntry>,
    pub z: u16,
}

impl ShortcutBarDesc {
    #[must_use]
    pub fn new(rect: Rect, entries: Vec<ShortcutEntry>) -> Self {
        Self {
            rect,
            entries,
            z: 12,
        }
    }
}

/// 发射底部快捷键提示栏（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_shortcut_bar(
    node: u64,
    desc: &ShortcutBarDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.entries.len() + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::DIVIDER
    };
    // 栏体圆角 `ControlCornerRadius = 4`，1px 描边对齐 `ControlStrokeColorDefault` 档。
    primitives.push(outlined(
        r,
        BAR_RADIUS,
        Token::HEADER_BG,
        border,
        1.0,
        0.0,
        z,
    ));

    let mut x = r.origin.x + BAR_PADDING;
    for entry in &desc.entries {
        let keycap_w = (entry.chord.len() as f32 * 7.0 + 12.0).max(24.0);
        let keycap_r = Rect::new(
            x,
            r.origin.y + KEYCAP_INSET,
            keycap_w,
            (r.size.y - KEYCAP_INSET * 2.0).max(16.0),
        );
        // 键帽是控件表面：`ControlFillColorDefault` 填充 + `ControlStrokeColorDefault` 描边。
        primitives.push(outlined(
            keycap_r,
            KEYCAP_RADIUS,
            Token::CONTROL_FILL,
            Token::CONTROL_STROKE,
            1.0,
            0.0,
            z + 1,
        ));
        x += keycap_w + (entry.label.len() as f32 * 7.0) + 20.0;
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_SECONDARY,
        z + 2,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        BAR_RADIUS + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 3,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Status, "Shortcuts", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

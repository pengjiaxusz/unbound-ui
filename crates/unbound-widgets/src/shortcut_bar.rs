//! 底部上下文快捷键提示栏（`ShortcutBar`）与多作用域快捷键管理状态机（`KeybindingManagerState`）。
//!
//! 支持来源标记（[`BindingOrigin`]：默认 / 继承 / 本地覆盖）、作用域隔离、
//! 按关键词或按键组合过滤、单条恢复默认、全局重置与作用域感知冲突检测。

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

/// 快捷键绑定的来源分类。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BindingOrigin {
    /// 内置默认绑定。
    #[default]
    Default,
    /// 从父作用域或上游预设继承而来。
    Inherited,
    /// 当前作用域内的本地用户覆盖。
    LocalOverride,
}

/// 单个快捷键绑定条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub action_id: String,
    pub label: String,
    pub chord: String,
    /// 生效作用域（如 `"global"`、`"editor"`、`"viewport"`）。
    pub scope: String,
    /// 原始默认组合键（用于单条恢复默认与全局重置）。
    pub default_chord: String,
    /// 绑定来源标记（默认 / 继承 / 本地覆盖）。
    pub origin: BindingOrigin,
    /// 未被本地覆盖时的基础来源（用于恢复默认时还原 `Default` 或 `Inherited`）。
    base_origin: BindingOrigin,
}

impl ShortcutEntry {
    #[must_use]
    pub fn new(
        action_id: impl Into<String>,
        label: impl Into<String>,
        chord: impl Into<String>,
    ) -> Self {
        let chord = chord.into();
        Self {
            action_id: action_id.into(),
            label: label.into(),
            default_chord: chord.clone(),
            chord,
            scope: "global".to_owned(),
            origin: BindingOrigin::Default,
            base_origin: BindingOrigin::Default,
        }
    }

    /// 设置生效作用域。
    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// 设置来源标记（默认 / 继承 / 本地覆盖）。
    #[must_use]
    pub const fn with_origin(mut self, origin: BindingOrigin) -> Self {
        self.origin = origin;
        if !matches!(origin, BindingOrigin::LocalOverride) {
            self.base_origin = origin;
        }
        self
    }

    /// 显式设置默认组合键（用于区分继承/覆盖值与出厂默认值）。
    #[must_use]
    pub fn with_default_chord(mut self, default_chord: impl Into<String>) -> Self {
        self.default_chord = default_chord.into();
        self
    }

    /// 是否被用户本地修改过。
    #[must_use]
    pub const fn is_overridden(&self) -> bool {
        matches!(self.origin, BindingOrigin::LocalOverride)
    }
}

/// 快捷键管理器状态（支持作用域继承/覆盖、关键词与组合键过滤、恢复默认及冲突检测）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeybindingManagerState {
    pub bindings: Vec<ShortcutEntry>,
    /// 当前搜索过滤关键词。
    pub search_query: String,
}

impl KeybindingManagerState {
    #[must_use]
    pub fn new(bindings: Vec<ShortcutEntry>) -> Self {
        Self {
            bindings,
            search_query: String::new(),
        }
    }

    /// 在当前绑定中查找并覆盖指定动作的组合键，将来源标记为 [`BindingOrigin::LocalOverride`]。
    ///
    /// 若新组合键与 `default_chord` 相同，则自动还原为 [`BindingOrigin::Default`]。
    pub fn override_binding(&mut self, action_id: &str, new_chord: impl Into<String>) -> bool {
        let Some(entry) = self.bindings.iter_mut().find(|e| e.action_id == action_id) else {
            return false;
        };
        let chord = new_chord.into();
        entry.origin = if chord.eq_ignore_ascii_case(&entry.default_chord) {
            entry.base_origin
        } else {
            BindingOrigin::LocalOverride
        };
        entry.chord = chord;
        true
    }

    /// 将指定动作恢复为默认/继承组合键并还原其基础来源标记。
    pub fn reset_binding(&mut self, action_id: &str) -> bool {
        let Some(entry) = self.bindings.iter_mut().find(|e| e.action_id == action_id) else {
            return false;
        };
        entry.chord = entry.default_chord.clone();
        entry.origin = entry.base_origin;
        true
    }

    /// 全局重置所有本地覆盖条目为默认值/继承值，返回被重置的条目数量。
    pub fn reset_all(&mut self) -> usize {
        let mut count = 0usize;
        for entry in &mut self.bindings {
            if entry.is_overridden() || entry.chord != entry.default_chord {
                entry.chord = entry.default_chord.clone();
                entry.origin = entry.base_origin;
                count += 1;
            }
        }
        count
    }

    /// 使用当前 `search_query` 过滤绑定条目。
    #[must_use]
    pub fn filtered_bindings(&self) -> Vec<&ShortcutEntry> {
        self.filter(&self.search_query)
    }

    /// 按关键词或按键组合（大小写不敏感）过滤绑定条目，匹配 `action_id`、`label`、`chord` 或 `scope`。
    #[must_use]
    pub fn filter(&self, query: &str) -> Vec<&ShortcutEntry> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return self.bindings.iter().collect();
        }
        self.bindings
            .iter()
            .filter(|e| {
                e.action_id.to_ascii_lowercase().contains(&needle)
                    || e.label.to_ascii_lowercase().contains(&needle)
                    || e.chord.to_ascii_lowercase().contains(&needle)
                    || e.scope.to_ascii_lowercase().contains(&needle)
            })
            .collect()
    }

    /// 仅返回指定作用域（含继承自 `"global"` 的绑定）的条目列表。
    #[must_use]
    pub fn entries_for_scope(&self, scope: &str) -> Vec<&ShortcutEntry> {
        self.bindings
            .iter()
            .filter(|e| e.scope.eq_ignore_ascii_case(scope) || e.scope.eq_ignore_ascii_case("global"))
            .collect()
    }

    /// 检测共享同一组合键字符串的所有冲突动作对 `(action_a, action_b, chord)`。
    ///
    /// 仅当两个绑定处于同一作用域，或其中一方为 `"global"` 作用域时才视为冲突。
    #[must_use]
    pub fn detect_conflicts(&self) -> Vec<(String, String, String)> {
        let mut conflicts = Vec::new();
        for i in 0..self.bindings.len() {
            for j in (i + 1)..self.bindings.len() {
                let a = &self.bindings[i];
                let b = &self.bindings[j];
                let scopes_overlap = a.scope.eq_ignore_ascii_case(&b.scope)
                    || a.scope.eq_ignore_ascii_case("global")
                    || b.scope.eq_ignore_ascii_case("global");
                if scopes_overlap && !a.chord.is_empty() && a.chord.eq_ignore_ascii_case(&b.chord) {
                    conflicts.push((a.action_id.clone(), b.action_id.clone(), a.chord.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn keybinding_manager_scope_origin_filter_override_and_reset() {
        let mut mgr = KeybindingManagerState::new(vec![
            ShortcutEntry::new("file.save", "Save Capture", "Ctrl+S"),
            ShortcutEntry::new("viewport.frame", "Frame Selection", "F")
                .with_scope("viewport")
                .with_origin(BindingOrigin::Inherited),
            ShortcutEntry::new("editor.find", "Find In Shader", "F").with_scope("editor"),
        ]);

        // 不同非全局作用域的相同快捷键不构成冲突。
        assert!(mgr.detect_conflicts().is_empty());

        // 覆盖为与全局冲突的快捷键。
        assert!(mgr.override_binding("editor.find", "Ctrl+S"));
        assert_eq!(mgr.bindings[2].origin, BindingOrigin::LocalOverride);
        assert_eq!(mgr.detect_conflicts().len(), 1);

        // 按关键词与按键组合过滤。
        assert_eq!(mgr.filter("ctrl+s").len(), 2);
        assert_eq!(mgr.filter("shader").len(), 1);
        assert_eq!(mgr.entries_for_scope("viewport").len(), 2);

        // 单条恢复默认。
        assert!(mgr.reset_binding("editor.find"));
        assert_eq!(mgr.bindings[2].chord, "F");
        assert_eq!(mgr.bindings[2].origin, BindingOrigin::Default);
        assert!(mgr.detect_conflicts().is_empty());

        // 全局重置：应还原各自的 base_origin（Inherited 保持 Inherited）。
        mgr.override_binding("file.save", "Ctrl+Shift+S");
        mgr.override_binding("viewport.frame", "G");
        assert_eq!(mgr.bindings[1].origin, BindingOrigin::LocalOverride);
        assert_eq!(mgr.reset_all(), 2);
        assert_eq!(mgr.bindings[0].chord, "Ctrl+S");
        assert_eq!(mgr.bindings[0].origin, BindingOrigin::Default);
        assert_eq!(mgr.bindings[1].chord, "F");
        assert_eq!(mgr.bindings[1].origin, BindingOrigin::Inherited);
    }

    #[test]
    fn shortcut_bar_headless_geometry() {
        let rect = Rect::new(16.0, 16.0, 480.0, 32.0);
        let desc = ShortcutBarDesc::new(
            rect,
            vec![
                ShortcutEntry::new("cmd.palette", "Commands", "Ctrl+K"),
                ShortcutEntry::new("view.zoom", "Zoom", "Ctrl+="),
            ],
        );
        let out = emit_shortcut_bar(1, &desc, ControlState::default(), &[]);
        assert_eq!(out.primitives.len(), 4);

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "ShortcutBar",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}

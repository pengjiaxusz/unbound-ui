//! 独立原子化按键帽徽标与组合键序列渲染组件（`Kbd` / `KbdChord`）。
//!
//! 供命令面板、菜单项、Tooltip、设置说明与快捷键管理统一复用，
//! 支持 `Sm` / `Md` 两档 Win11 Fluent 轻量浮起底板 + 底部微阴影描边样式，
//! 并可直接从 [`unbound_input::KeyChord`] 或按键切片生成规范化组合键序列。

use unbound_a11y::A11yNode;
use unbound_input::{KeyChord, KeyName};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{curve_stroke, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, WidgetOutput};

/// 按键帽圆角半径（对齐 WinUI `ControlCornerRadius = 4`）。
const KBD_RADIUS: f32 = 4.0;

/// 按键帽尺寸档位。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum KbdSize {
    /// 紧凑尺寸（20px 高，适用于菜单项与 Tooltip）。
    Sm,
    /// 标准尺寸（24px 高，适用于命令面板与快捷键说明）。
    #[default]
    Md,
}

impl KbdSize {
    /// 键帽高度（逻辑像素）。
    #[must_use]
    pub const fn height(self) -> f32 {
        match self {
            Self::Sm => 20.0,
            Self::Md => 24.0,
        }
    }

    /// 键帽最小宽度（逻辑像素）。
    #[must_use]
    pub const fn min_width(self) -> f32 {
        match self {
            Self::Sm => 20.0,
            Self::Md => 24.0,
        }
    }

    /// 水平内边距（逻辑像素）。
    #[must_use]
    pub const fn padding_x(self) -> f32 {
        match self {
            Self::Sm => 5.0,
            Self::Md => 7.0,
        }
    }

    /// 估算单字符宽度（逻辑像素）。
    #[must_use]
    pub const fn char_width(self) -> f32 {
        match self {
            Self::Sm => 6.5,
            Self::Md => 7.5,
        }
    }

    /// 估算键帽整体宽度。
    #[must_use]
    pub fn measure_key_width(self, label: &str) -> f32 {
        let chars = label.chars().count() as f32;
        (chars * self.char_width() + self.padding_x() * 2.0).max(self.min_width())
    }
}

/// 将任意键名规范化为统一的桌面按键帽文案（如 `"ctrl"` -> `"Ctrl"`、`"escape"` -> `"Esc"`）。
#[must_use]
pub fn normalize_key_label(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "Ctrl".to_owned(),
        "alt" | "option" => "Alt".to_owned(),
        "shift" => "Shift".to_owned(),
        "meta" | "win" | "super" | "cmd" | "command" => "Win".to_owned(),
        "esc" | "escape" => "Esc".to_owned(),
        "enter" | "return" => "Enter".to_owned(),
        "space" | " " => "Space".to_owned(),
        "tab" => "Tab".to_owned(),
        "backspace" => "Backspace".to_owned(),
        "del" | "delete" => "Del".to_owned(),
        "arrowup" | "up" => "Up".to_owned(),
        "arrowdown" | "down" => "Down".to_owned(),
        "arrowleft" | "left" => "Left".to_owned(),
        "arrowright" | "right" => "Right".to_owned(),
        "pageup" => "PgUp".to_owned(),
        "pagedown" => "PgDn".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        other if other.len() == 1 => other.to_ascii_uppercase(),
        other if other.starts_with('f') && other[1..].chars().all(|c| c.is_ascii_digit()) => {
            other.to_ascii_uppercase()
        }
        _ => {
            let mut chars = trimmed.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
    }
}

/// 将 [`KeyChord`] 拆分为规范化的按键帽序列（如 `["Ctrl", "Shift", "P"]`）。
#[must_use]
pub fn key_labels_from_chord(chord: &KeyChord) -> Vec<String> {
    let mut keys = Vec::with_capacity(5);
    if chord.ctrl {
        keys.push("Ctrl".to_owned());
    }
    if chord.alt {
        keys.push("Alt".to_owned());
    }
    if chord.shift {
        keys.push("Shift".to_owned());
    }
    if chord.meta {
        keys.push("Win".to_owned());
    }
    match chord.key {
        KeyName::Char(c) => keys.push(normalize_key_label(&c.to_string())),
        KeyName::Named(name) => keys.push(normalize_key_label(name)),
        KeyName::Function(n) => keys.push(format!("F{n}")),
    }
    keys
}

/// 单个按键帽描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct KbdDesc {
    /// 键帽边界矩形。
    pub rect: Rect,
    /// 规范化后的按键文案。
    pub key: String,
    /// 键帽尺寸档位。
    pub size: KbdSize,
    /// 绘制 z-index。
    pub z: u16,
}

impl KbdDesc {
    /// 创建单个按键帽描述符（自动规范化键名大小写）。
    #[must_use]
    pub fn new(rect: Rect, key: impl Into<String>) -> Self {
        Self {
            rect,
            key: normalize_key_label(&key.into()),
            size: KbdSize::Md,
            z: 20,
        }
    }

    /// 设置尺寸档位。
    #[must_use]
    pub const fn with_size(mut self, size: KbdSize) -> Self {
        self.size = size;
        self
    }

    /// 按键帽高度自动调整推荐矩形尺寸。
    #[must_use]
    pub fn sized_at(origin: Vec2, key: impl Into<String>, size: KbdSize) -> Self {
        let normalized = normalize_key_label(&key.into());
        let w = size.measure_key_width(&normalized);
        let h = size.height();
        Self {
            rect: Rect::new(origin.x, origin.y, w, h),
            key: normalized,
            size,
            z: 20,
        }
    }
}

/// 发射单个按键帽（24 态恒定图元数：底部微阴影唇边 + 浮起主体底板 = 2 个固定图元）。
#[must_use]
pub fn emit_kbd(
    _node: u64,
    desc: &KbdDesc,
    state: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let r = desc.rect;
    let z = desc.z;
    let mut primitives = Vec::with_capacity(2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let (fill, border, lip_color, text_color) = if state.disabled {
        (
            Token::CONTROL_DISABLED_BG,
            Token::CONTROL_DISABLED_BORDER,
            Token::CONTROL_DISABLED_BORDER,
            Token::TEXT_DISABLED,
        )
    } else {
        (
            Token::SURFACE_RAISED,
            Token::CONTROL_STROKE,
            Token::OUTLINE_STRONG,
            Token::TEXT_PRIMARY,
        )
    };

    // 底部 2px 立体键帽阴影唇边（Win11 Fluent 轻量浮起质感）。
    let lip_r = Rect::new(r.origin.x, r.origin.y + 1.0, r.width(), r.height());
    primitives.push(surface(lip_r, KBD_RADIUS, lip_color, z));
    // 键帽主体表面。
    primitives.push(outlined(r, KBD_RADIUS, fill, border, 1.0, 0.0, z + 1));

    if !glyphs.is_empty() {
        emit_glyphs(glyphs, text_color, z + 2, &mut primitives, &mut payloads);
    }

    let a11y = A11yNode::static_text(desc.key.clone(), r).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

/// 组合键按键帽序列描述符（如 `Ctrl` + `Shift` + `P`）。
#[derive(Clone, Debug, PartialEq)]
pub struct KbdChordDesc {
    /// 组合键起始原点（左上角）。
    pub origin: Vec2,
    /// 规范化后的按键帽列表。
    pub keys: Vec<String>,
    /// 键帽尺寸档位。
    pub size: KbdSize,
    /// 分隔符文案（默认 `"+"`）。
    pub separator: String,
    /// 键帽与分隔符间距（默认 `4.0`）。
    pub gap: f32,
    /// 绘制 z-index。
    pub z: u16,
}

impl KbdChordDesc {
    /// 从按键切片构造组合键序列描述符。
    #[must_use]
    pub fn from_keys<S: AsRef<str>>(origin: Vec2, keys: &[S]) -> Self {
        let normalized = keys
            .iter()
            .map(|k| normalize_key_label(k.as_ref()))
            .filter(|k| !k.is_empty())
            .collect();
        Self {
            origin,
            keys: normalized,
            size: KbdSize::Md,
            separator: "+".to_owned(),
            gap: 4.0,
            z: 20,
        }
    }

    /// 直接从 [`KeyChord`] 构造组合键序列描述符。
    #[must_use]
    pub fn from_key_chord(origin: Vec2, chord: &KeyChord) -> Self {
        Self {
            origin,
            keys: key_labels_from_chord(chord),
            size: KbdSize::Md,
            separator: "+".to_owned(),
            gap: 4.0,
            z: 20,
        }
    }

    /// 设置尺寸档位。
    #[must_use]
    pub const fn with_size(mut self, size: KbdSize) -> Self {
        self.size = size;
        self
    }

    /// 设置分隔符字符串。
    #[must_use]
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    /// 计算每个按键帽的布局矩形。
    #[must_use]
    pub fn keycap_rects(&self) -> Vec<Rect> {
        let mut rects = Vec::with_capacity(self.keys.len());
        let h = self.size.height();
        let sep_w = if self.separator.is_empty() {
            0.0
        } else {
            self.separator.chars().count() as f32 * self.size.char_width() + self.gap * 2.0
        };
        let mut x = self.origin.x;
        for (idx, key) in self.keys.iter().enumerate() {
            let w = self.size.measure_key_width(key);
            rects.push(Rect::new(x, self.origin.y, w, h));
            x += w;
            if idx + 1 < self.keys.len() {
                x += if sep_w > 0.0 { sep_w } else { self.gap };
            }
        }
        rects
    }

    /// 计算相邻按键帽之间分隔符的布局矩形列表。
    #[must_use]
    pub fn separator_rects(&self) -> Vec<Rect> {
        let caps = self.keycap_rects();
        if caps.len() < 2 || self.separator.is_empty() {
            return Vec::new();
        }
        let h = self.size.height();
        caps.windows(2)
            .map(|pair| {
                let left = pair[0].max().x;
                let right = pair[1].origin.x;
                Rect::new(left, self.origin.y, (right - left).max(0.0), h)
            })
            .collect()
    }

    /// 计算整组组合键的外包矩形。
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let rects = self.keycap_rects();
        let Some(first) = rects.first() else {
            return Rect::new(self.origin.x, self.origin.y, 0.0, self.size.height());
        };
        let last = rects.last().unwrap_or(first);
        Rect::new(
            first.origin.x,
            first.origin.y,
            (last.max().x - first.origin.x).max(0.0),
            self.size.height(),
        )
    }

    /// 返回用于无障碍播报的规范化组合键文本（如 `"Ctrl+Shift+P"`）。
    #[must_use]
    pub fn formatted_text(&self) -> String {
        self.keys.join(&self.separator)
    }
}

/// 发射组合键按键帽序列（每个键帽固定 2 个图元，24 态图元数恒定）。
#[must_use]
pub fn emit_kbd_chord(
    _node: u64,
    desc: &KbdChordDesc,
    state: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let rects = desc.keycap_rects();
    let sep_rects = desc.separator_rects();
    let z = desc.z;
    let mut primitives = Vec::with_capacity(rects.len() * 2 + sep_rects.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(sep_rects.len() * 2 + glyphs.len());

    let (fill, border, lip_color, text_color) = if state.disabled {
        (
            Token::CONTROL_DISABLED_BG,
            Token::CONTROL_DISABLED_BORDER,
            Token::CONTROL_DISABLED_BORDER,
            Token::TEXT_DISABLED,
        )
    } else {
        (
            Token::SURFACE_RAISED,
            Token::CONTROL_STROKE,
            Token::OUTLINE_STRONG,
            Token::TEXT_PRIMARY,
        )
    };

    for r in &rects {
        let lip_r = Rect::new(r.origin.x, r.origin.y + 1.0, r.width(), r.height());
        primitives.push(surface(lip_r, KBD_RADIUS, lip_color, z));
        primitives.push(outlined(*r, KBD_RADIUS, fill, border, 1.0, 0.0, z + 1));
    }

    // 绘制组合键分隔符（默认 `+` 十字微线段，24 态下数量恒定）。
    let sep_color = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    for sr in &sep_rects {
        let cx = sr.origin.x + sr.width() * 0.5;
        let cy = sr.origin.y + sr.height() * 0.5;
        curve_stroke([cx - 2.5, cy], [cx + 2.5, cy], 1.2, sep_color, z + 1, &mut primitives, &mut payloads);
        curve_stroke([cx, cy - 2.5], [cx, cy + 2.5], 1.2, sep_color, z + 1, &mut primitives, &mut payloads);
    }

    if !glyphs.is_empty() {
        emit_glyphs(glyphs, text_color, z + 2, &mut primitives, &mut payloads);
    }

    let bounds = desc.bounds();
    let a11y = A11yNode::static_text(desc.formatted_text(), bounds).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit: None,
        a11y: Some(a11y),
        focusable: false,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kbd_normalizes_key_labels_and_converts_key_chord() {
        assert_eq!(normalize_key_label("ctrl"), "Ctrl");
        assert_eq!(normalize_key_label("escape"), "Esc");
        assert_eq!(normalize_key_label("p"), "P");
        assert_eq!(normalize_key_label("f12"), "F12");

        let chord = KeyChord::character('p').with_ctrl().with_shift();
        let desc = KbdChordDesc::from_key_chord(Vec2::new(10.0, 10.0), &chord);
        assert_eq!(desc.keys, vec!["Ctrl", "Shift", "P"]);
        assert_eq!(desc.formatted_text(), "Ctrl+Shift+P");

        let rects = desc.keycap_rects();
        assert_eq!(rects.len(), 3);
        assert!(rects[0].max().x < rects[1].origin.x);
        assert!(rects[1].max().x < rects[2].origin.x);
    }

    #[test]
    fn kbd_and_kbd_chord_emit_a11y_and_stable_shape() {
        let single = KbdDesc::sized_at(Vec2::new(16.0, 16.0), "esc", KbdSize::Sm);
        let single_out = emit_kbd(1, &single, ControlState::default(), &[]);
        assert_eq!(single_out.a11y.as_ref().unwrap().label, "Esc");
        assert_eq!(single_out.primitives.len(), 2);

        let chord = KbdChordDesc::from_keys(Vec2::new(16.0, 48.0), &["ctrl", "k"])
            .with_size(KbdSize::Md);
        let chord_out = emit_kbd_chord(2, &chord, ControlState::default(), &[]);
        assert_eq!(chord_out.a11y.as_ref().unwrap().label, "Ctrl+K");
        // 2 个键帽（各 2 图元）+ 1 个 `+` 分隔符（2 线段）= 6 图元。
        assert_eq!(chord_out.primitives.len(), 6);

        // 全 24 态图元数恒定校验。
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let st = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        assert_eq!(emit_kbd(1, &single, st, &[]).primitives.len(), 2);
                        assert_eq!(emit_kbd_chord(2, &chord, st, &[]).primitives.len(), 6);
                    }
                }
            }
        }

        // 无头几何健康检查。
        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let bounds = chord.bounds();
        let mut frame = unbound_headless::FrameBuilder::new(viewport);
        frame.open(unbound_headless::NodeKind::Container, viewport, "Root");
        frame.open(
            unbound_headless::NodeKind::Text {
                ink: bounds,
                ellipsis: false,
            },
            bounds,
            "KbdChord",
        );
        frame.set_text_colors([28, 30, 38], [255, 255, 255]);
        frame.close();
        frame.close();
        assert!(unbound_headless::Harness::assert_clean(&frame.build()).is_clean());
    }
}

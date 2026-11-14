//! PinInput / OtpInput 定长分段验证码输入框，支持粘贴自动拆分。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::GpuPrimitiveInstance;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_PIN_SLOTS: usize = 6;

/// [`PinInputDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinInputState {
    /// 已输入的字符（最多 `length` 个）。
    pub digits: Vec<char>,
    /// 当前激活的单元格索引。
    pub cursor: usize,
}

/// PIN 输入单元格的确定性子节点 ID（`0..6`）。
#[must_use]
pub const fn pin_cell_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x800 + index as u64)
}

impl PinInputState {
    /// 删除最后一个字符或当前槽位内容并将光标回退。
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let idx = (self.cursor - 1).min(self.digits.len().saturating_sub(1));
            if idx < self.digits.len() {
                self.digits.remove(idx);
            }
            self.cursor -= 1;
        }
    }

    /// 插入一个字符并前移光标。
    pub fn push_char(&mut self, ch: char, length: usize) {
        let cap = length.clamp(1, MAX_PIN_SLOTS);
        if self.cursor < cap {
            if self.cursor < self.digits.len() {
                self.digits[self.cursor] = ch;
            } else {
                self.digits.push(ch);
            }
            self.cursor = (self.cursor + 1).min(cap);
        }
    }

    /// 粘贴字符串，将字符拆分填充到各个槽位。
    pub fn paste(&mut self, text: &str, length: usize) {
        for ch in text.chars().filter(|c| !c.is_whitespace()) {
            if self.cursor >= length.clamp(1, MAX_PIN_SLOTS) {
                break;
            }
            self.push_char(ch, length);
        }
    }
}

/// [`PinInput`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PinInputDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// PIN 单元格数量（`1..=6`）。
    pub length: usize,
    /// 当前已填充的单元格数量。
    pub filled: usize,
    /// 是否掩码隐藏数字。
    pub masked: bool,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl PinInputDesc {
    /// 创建 PIN 输入框描述符。
    #[must_use]
    pub fn new(rect: Rect, length: usize, filled: usize) -> Self {
        Self {
            rect,
            length: length.clamp(1, MAX_PIN_SLOTS),
            filled,
            masked: false,
            label: "PIN Input".to_owned(),
            z: 600,
        }
    }
}

/// 输出结构稳定的 PIN 输入框（6 个单元格框 + 6 个填充圆点 + 1 个焦点环 = 13 个图元）。
#[must_use]
pub fn emit_pin_input(
    node: u64,
    desc: &PinInputDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(13 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    let len = desc.length.clamp(1, MAX_PIN_SLOTS);
    let gap = 6.0;
    let cell_w = ((desc.rect.width() - gap * (len - 1) as f32) / len as f32).max(14.0);
    let cell_h = desc.rect.height().max(20.0);

    for idx in 0..MAX_PIN_SLOTS {
        let visible = idx < len;
        let r = Rect::new(
            desc.rect.origin.x + idx.min(len - 1) as f32 * (cell_w + gap),
            desc.rect.origin.y,
            cell_w,
            cell_h,
        );
        let mut cell = outlined(
            r,
            6.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE
            },
            if state.focused && idx == desc.filled.min(len - 1) {
                Token::FOCUS_RING
            } else {
                Token::CARD_BORDER
            },
            1.0,
            0.0,
            desc.z,
        );
        if !visible {
            cell.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(cell.depth_u16(), 0.0);
        }
        primitives.push(cell);

        let dot_r = Rect::new(
            r.origin.x + (cell_w - 8.0) * 0.5,
            r.origin.y + (cell_h - 8.0) * 0.5,
            8.0,
            8.0,
        );
        let mut dot = surface(dot_r, 4.0, Token::PRIMARY, desc.z + 1);
        if !visible || idx >= desc.filled {
            dot.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(dot.depth_u16(), 0.0);
        }
        primitives.push(dot);
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
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
            .with_cursor(CursorKind::Text)
    });
    let a11y =
        A11yNode::new(Role::TextInput, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for idx in 0..len {
            let r = Rect::new(
                desc.rect.origin.x + idx as f32 * (cell_w + gap),
                desc.rect.origin.y,
                cell_w,
                cell_h,
            );
            let key = pin_cell_node(node, idx);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Text),
                A11yNode::new(Role::TextInput, format!("Digit {}", idx + 1), r).clickable(),
                true,
            );
        }
    }

    out
}

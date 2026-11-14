//! Popconfirm 锚定式轻量确认气泡。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, outlined};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// [`Popconfirm`] 锚定气泡的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PopconfirmDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 确认提示标题。
    pub title: String,
    /// 确认按钮文本。
    pub confirm_label: String,
    /// 取消按钮文本。
    pub cancel_label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl PopconfirmDesc {
    /// 创建确认气泡描述符。
    #[must_use]
    pub fn new(rect: Rect, title: impl Into<String>) -> Self {
        Self {
            rect,
            title: title.into(),
            confirm_label: "Confirm".to_owned(),
            cancel_label: "Cancel".to_owned(),
            z: 800,
        }
    }
}

/// 确认气泡取消按钮的确定性子节点 ID。
#[must_use]
pub const fn popconfirm_cancel_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x101
}

/// 确认气泡确认按钮的确定性子节点 ID。
#[must_use]
pub const fn popconfirm_confirm_node(node: u64) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x102
}

/// 输出结构稳定的确认气泡（1 张卡片 + 取消按钮 + 确认按钮 + 1 个焦点环 = 4 个图元）。
#[must_use]
pub fn emit_popconfirm(
    node: u64,
    desc: &PopconfirmDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        Token::SURFACE_RAISED,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        6.0,
        desc.z,
    ));

    let btn_w = 56.0_f32.min((desc.rect.width() - 24.0) * 0.45);
    let btn_h = 22.0_f32.min(desc.rect.height() * 0.4);
    let btn_y = desc.rect.max().y - btn_h - 8.0;
    let confirm_r = Rect::new(desc.rect.max().x - btn_w - 8.0, btn_y, btn_w, btn_h);
    let cancel_r = Rect::new(confirm_r.origin.x - btn_w - 6.0, btn_y, btn_w, btn_h);

    primitives.push(outlined(
        cancel_r,
        4.0,
        Token::SURFACE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        desc.z + 1,
    ));
    primitives.push(outlined(
        confirm_r,
        4.0,
        Token::ERROR,
        Token::ERROR,
        1.0,
        0.0,
        desc.z + 1,
    ));

    primitives.push(focus_ring(
        desc.rect,
        8.0,
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
    let a11y = A11yNode::new(Role::Dialog, desc.title.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        let cancel_key = popconfirm_cancel_node(node);
        out.with_sub_control(
            cancel_key,
            HitRegion::new(cancel_key, CONTROL_LAYER, u32::from(desc.z) + 1, cancel_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, desc.cancel_label.clone(), cancel_r).clickable(),
            true,
        );
        let confirm_key = popconfirm_confirm_node(node);
        out.with_sub_control(
            confirm_key,
            HitRegion::new(confirm_key, CONTROL_LAYER, u32::from(desc.z) + 1, confirm_r)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, desc.confirm_label.clone(), confirm_r).clickable(),
            true,
        );
    }

    out
}

//! 顶部全宽全局状态横幅（Banner，例如热重载或设备丢失提示）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::alert::Severity;
use crate::primitive::{curve_stroke, focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 顶部全宽 [`Banner`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct BannerDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 横幅消息。
    pub message: String,
    /// 严重级别色调。
    pub severity: Severity,
    /// 基础 z-index。
    pub z: u16,
}

impl BannerDesc {
    /// 创建横幅描述符。
    #[must_use]
    pub fn new(rect: Rect, message: impl Into<String>, severity: Severity) -> Self {
        Self {
            rect,
            message: message.into(),
            severity,
            z: 700,
        }
    }
}

/// 发射固定形状的横幅（1 背景 + 1 强调条 + 1 关闭胶囊 + 1 焦点环 = 4 个图元）。
#[must_use]
pub fn emit_banner(
    node: u64,
    desc: &BannerDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let (fill, accent) = match desc.severity {
        Severity::Info => (Token::PRIMARY_SOFT, Token::PRIMARY),
        Severity::Success => (Token::SUCCESS_SOFT, Token::SUCCESS),
        Severity::Warning => (Token::WARNING_SOFT, Token::WARNING),
        Severity::Error => (Token::ERROR_SOFT, Token::ERROR),
    };
    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        4.0,
        fill,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            accent
        },
        1.0,
        0.0,
        desc.z,
    ));

    let bar_r = Rect::new(
        desc.rect.origin.x,
        desc.rect.origin.y,
        4.0,
        desc.rect.height(),
    );
    primitives.push(surface(bar_r, 2.0, accent, desc.z + 1));

    let close_r = Rect::new(
        desc.rect.max().x - 26.0,
        desc.rect.origin.y + (desc.rect.height() - 18.0) * 0.5,
        18.0,
        18.0,
    );
    primitives.push(surface(close_r, 9.0, accent, desc.z + 1));
    let cx = close_r.center().x;
    let cy = close_r.center().y;
    curve_stroke(
        [cx - 3.0, cy - 3.0],
        [cx + 3.0, cy + 3.0],
        1.5,
        Token::ON_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );
    curve_stroke(
        [cx + 3.0, cy - 3.0],
        [cx - 3.0, cy + 3.0],
        1.5,
        Token::ON_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    primitives.push(focus_ring(
        desc.rect,
        4.0,
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
    let a11y = A11yNode::new(Role::Alert, desc.message.clone(), desc.rect).disabled(state.disabled);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    }
}

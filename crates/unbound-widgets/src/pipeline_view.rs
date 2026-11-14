//! 渲染管线与构建流水线阶段耗时检视器（`PipelineView`）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 渲染管线或构建流水线的单个阶段。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineStageItem {
    pub name: String,
    pub duration_ms: f32,
    pub active: bool,
    pub has_error: bool,
}

impl PipelineStageItem {
    #[must_use]
    pub fn new(name: impl Into<String>, duration_ms: f32, active: bool) -> Self {
        Self {
            name: name.into(),
            duration_ms,
            active,
            has_error: false,
        }
    }
}

/// 渲染管线阶段检视器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineViewDesc {
    pub rect: Rect,
    pub stages: Vec<PipelineStageItem>,
    pub z: u16,
}

impl PipelineViewDesc {
    #[must_use]
    pub fn new(rect: Rect, stages: Vec<PipelineStageItem>) -> Self {
        Self {
            rect,
            stages,
            z: 14,
        }
    }
}

/// 发射管线阶段视图（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_pipeline_view(
    node: u64,
    desc: &PipelineViewDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.stages.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let outer_border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    primitives.push(outlined(r, 8.0, Token::SURFACE, outer_border, 1.0, 0.0, z));

    let count = desc.stages.len().max(1) as f32;
    let gap = 8.0;
    let stage_w = ((r.size.x - 24.0 - (count - 1.0) * gap) / count).max(32.0);
    for (idx, stage) in desc.stages.iter().enumerate() {
        let sx = r.origin.x + 12.0 + idx as f32 * (stage_w + gap);
        let card_r = Rect::new(sx, r.origin.y + 12.0, stage_w, (r.size.y - 24.0).max(28.0));
        let (fill, border) = if stage.has_error {
            (Token::ERROR_SOFT, Token::ERROR)
        } else if stage.active {
            (Token::PRIMARY_SOFT, Token::PRIMARY)
        } else {
            (Token::SURFACE_RAISED, Token::OUTLINE)
        };
        primitives.push(outlined(card_r, 6.0, fill, border, 1.0, 0.0, z + 1));

        // 底部 GPU 耗时占比进度条
        let ratio = (stage.duration_ms / 4.0).clamp(0.08, 1.0);
        let bar_r = Rect::new(
            card_r.origin.x + 6.0,
            card_r.max().y - 10.0,
            (card_r.size.x - 12.0) * ratio,
            4.0,
        );
        let bar_tok = if stage.has_error {
            Token::ERROR
        } else {
            Token::PRIMARY
        };
        primitives.push(surface(bar_r, 2.0, bar_tok, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        10.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(A11yNode::new(Role::Group, "Pipeline View", r).disabled(control.disabled)),
        focusable: !control.disabled,
        ..Default::default()
    }
}

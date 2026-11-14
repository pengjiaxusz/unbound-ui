//! 星级评分控件，支持整星/半星精度与只读模式。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{focus_ring, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

const MAX_STARS: usize = 5;

/// [`RatingDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RatingState {
    /// 当前评分（`0.0..=5.0`，步进 `0.5` 或 `1.0`）。
    pub value: f32,
    /// 悬停时的预览分值。
    pub hover_value: Option<f32>,
}

/// [`Rating`] 的描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct RatingDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 当前评分（`0.0..=5.0`）。
    pub value: f32,
    /// 是否允许半星步进。
    pub allow_half: bool,
    /// 只读展示模式。
    pub read_only: bool,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl RatingDesc {
    /// 创建星级评分描述符。
    #[must_use]
    pub fn new(rect: Rect, value: f32) -> Self {
        Self {
            rect,
            value: value.clamp(0.0, MAX_STARS as f32),
            allow_half: true,
            read_only: false,
            label: "Rating".to_owned(),
            z: 600,
        }
    }
}

/// 输出结构稳定的 5 星评分控件（1 个背景 + 5 个星形槽位 + 1 个焦点环 = 7 个图元）。
#[must_use]
pub fn emit_rating(node: u64, desc: &RatingDesc, state: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(7);
    let payloads = Vec::new();

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        6.0,
        if hovered {
            Token::SURFACE_RAISED
        } else {
            Token::SURFACE_SUNKEN
        },
        if state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let gap = 6.0;
    let sz = ((desc.rect.width() - gap * (MAX_STARS + 1) as f32) / MAX_STARS as f32)
        .min(desc.rect.height() - 8.0)
        .max(12.0);
    let y = desc.rect.origin.y + (desc.rect.height() - sz) * 0.5;
    for i in 0..MAX_STARS {
        let r = Rect::new(desc.rect.origin.x + gap + i as f32 * (sz + gap), y, sz, sz);
        let star_tok = if state.disabled {
            if desc.value >= i as f32 + 0.5 {
                Token::DISABLED
            } else {
                Token::CONTROL_DISABLED_BG
            }
        } else if desc.value >= i as f32 + 1.0 {
            Token::WARNING
        } else if desc.allow_half && desc.value >= i as f32 + 0.5 {
            Token::WARNING_SOFT
        } else {
            Token::CARD_BORDER
        };
        primitives.push(surface(r, sz * 0.25, star_tok, desc.z + 1));
    }

    primitives.push(focus_ring(
        desc.rect,
        6.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        state.focused,
    ));

    let interactive = !state.disabled && !desc.read_only;
    let hit = interactive.then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y =
        A11yNode::new(Role::Slider, desc.label.clone(), desc.rect).disabled(state.disabled);
    a11y.value = format!("{:.1}", desc.value);

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: interactive,
        ..Default::default()
    }
}

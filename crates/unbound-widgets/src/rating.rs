//! 星级评分控件，支持整星/半星精度与只读模式。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined};
use crate::shell::{emit_shell_icon_primitives, ShellIcon};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, TextCtx, WidgetOutput};

const MAX_STARS: usize = 5;
/// 实心星形符号字体码点（WinUI 3 `SolidStar` `U+E735`）。
pub const RATING_STAR_FILLED_GLYPH: &str = "\u{E735}";
/// 空心星形符号字体码点（WinUI 3 `OutlineStar` `U+E734`）。
pub const RATING_STAR_OUTLINE_GLYPH: &str = "\u{E734}";

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

/// 输出结构稳定的 5 星评分控件（使用 [`ShellIcon::StarFilled`] / [`ShellIcon::Star`] 矢量星形，
/// 跨全部交互态保持图元数恒定）。
#[must_use]
pub fn emit_rating(node: u64, desc: &RatingDesc, state: ControlState) -> WidgetOutput {
    emit_rating_inner(node, desc, state, None)
}

/// 输出结构稳定的 5 星评分控件，优先使用平台感知符号字体（`U+E735` / `U+E734`），
/// 当无可用符号字体时自动回退 [`ShellIcon::StarFilled`] / [`ShellIcon::Star`] 矢量星形。
#[must_use]
pub fn emit_rating_with_text(
    node: u64,
    desc: &RatingDesc,
    state: ControlState,
    text_ctx: &mut TextCtx<'_>,
) -> WidgetOutput {
    emit_rating_inner(node, desc, state, Some(text_ctx))
}

fn emit_rating_inner(
    node: u64,
    desc: &RatingDesc,
    state: ControlState,
    mut text_ctx: Option<&mut TextCtx<'_>>,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(64);
    let mut payloads = Vec::with_capacity(64);

    let hovered = state.phase == PointerPhase::Hovered;
    // WinUI RatingControl 自身无实心面板，只在悬停时叠 SubtleFillColorSecondary。
    primitives.push(outlined(
        desc.rect,
        4.0, // ControlCornerRadius = 4
        if hovered {
            Token::HOVER
        } else {
            Token::CARD_BG // CardBackgroundFillColorDefault
        },
        if state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER // CardStrokeColorDefault
        },
        1.0,
        0.0,
        desc.z,
    ));

    // RatingControlItemSpacing = 8；ActualRatingFontSize = 32 / 2 = 16。
    let gap = 8.0;
    let sz = ((desc.rect.width() - gap * (MAX_STARS + 1) as f32) / MAX_STARS as f32)
        .min(desc.rect.height() - 8.0)
        .clamp(12.0, 16.0);
    let y = desc.rect.origin.y + (desc.rect.height() - sz) * 0.5;
    for i in 0..MAX_STARS {
        let r = Rect::new(desc.rect.origin.x + gap + i as f32 * (sz + gap), y, sz, sz);
        // 是否实心星仅由分值与半星配置决定，确保跨 24 种交互态图元数恒定。
        let filled = desc.value >= i as f32 + 1.0
            || (desc.allow_half && desc.value >= i as f32 + 0.5);
        let star_tok = if state.disabled {
            if desc.value >= i as f32 + 0.5 {
                Token::TEXT_DISABLED // RatingControlDisabledSelectedForeground = TextFillColorDisabled
            } else {
                Token::DISABLED
            }
        } else if desc.value >= i as f32 + 1.0 {
            Token::PRIMARY // RatingControlSelectedForeground = AccentFillColorDefault
        } else if desc.allow_half && desc.value >= i as f32 + 0.5 {
            Token::PRIMARY_SOFT // 半星沿用强调色，WinUI 以同一画刷裁剪 50%
        } else {
            Token::TEXT_SECONDARY // RatingControlUnselectedForeground = TextFillColorSecondary
        };
        let glyph = if filled {
            RATING_STAR_FILLED_GLYPH
        } else {
            RATING_STAR_OUTLINE_GLYPH
        };
        let shaped = text_ctx
            .as_deref_mut()
            .and_then(|ctx| ctx.shape_symbol_glyph(glyph, r, sz));
        if let Some(quads) = shaped {
            emit_glyphs(&quads, star_tok, desc.z + 1, &mut primitives, &mut payloads);
        } else {
            let icon = if filled {
                ShellIcon::StarFilled
            } else {
                ShellIcon::Star
            };
            emit_shell_icon_primitives(
                icon,
                r,
                star_tok,
                Token::SURFACE,
                desc.z + 1,
                &mut primitives,
                &mut payloads,
            );
        }
    }

    primitives.push(focus_ring(
        desc.rect,
        4.0, // ControlCornerRadius = 4
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

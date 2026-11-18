//! AnnotatedScrollBar：带刻度标签与悬停预览的滚动条。
//!
//! 在标准滚动条轨道与滑块之外，沿轨道绘制固定数量的刻度，并在滑块位置附近
//! 绘制一个悬停预览标签。刻度数量由描述符固定，状态变化只改令牌。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 轨道宽度。
pub const ANNOTATED_TRACK_WIDTH: f32 = 8.0;
/// 滑块最小高度。
pub const ANNOTATED_THUMB_MIN: f32 = 24.0;
/// 预览标签高度。
pub const ANNOTATED_PREVIEW_HEIGHT: f32 = 20.0;

/// 标注滚动条描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotatedScrollBarDesc {
    /// 控件矩形（竖直滚动条占位）。
    pub rect: Rect,
    /// 滑块位置 `0.0..=1.0`。
    pub thumb_norm: f32,
    /// 刻度标签，同时也是刻度数量来源。
    pub labels: Vec<String>,
    /// 悬停预览位置；`None` 表示不显示预览。
    pub preview_norm: Option<f32>,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl AnnotatedScrollBarDesc {
    /// 创建带五个默认刻度的标注滚动条。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            thumb_norm: 0.0,
            labels: vec![
                "Start".to_owned(),
                "25%".to_owned(),
                "50%".to_owned(),
                "75%".to_owned(),
                "End".to_owned(),
            ],
            preview_norm: None,
            disabled: false,
            z: 704,
        }
    }

    /// 设置滑块位置，自动夹紧。
    #[must_use]
    pub fn thumb(mut self, norm: f32) -> Self {
        self.thumb_norm = norm.clamp(0.0, 1.0);
        self
    }

    /// 覆盖刻度标签。
    #[must_use]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let labels: Vec<String> = labels.into_iter().map(Into::into).collect();
        if !labels.is_empty() {
            self.labels = labels;
        }
        self
    }

    /// 设置悬停预览位置。
    #[must_use]
    pub fn preview(mut self, norm: f32) -> Self {
        self.preview_norm = Some(norm.clamp(0.0, 1.0));
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 轨道矩形：竖直居中。
    #[must_use]
    pub fn track_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + (self.rect.width() - ANNOTATED_TRACK_WIDTH) * 0.5,
            self.rect.origin.y,
            ANNOTATED_TRACK_WIDTH,
            self.rect.height(),
        )
    }

    /// 滑块矩形。
    #[must_use]
    pub fn thumb_rect(&self) -> Rect {
        let track = self.track_rect();
        let height = ANNOTATED_THUMB_MIN.min(track.height().max(0.0));
        let travel = (track.height() - height).max(0.0);
        Rect::new(
            track.origin.x,
            track.origin.y + travel * self.thumb_norm.clamp(0.0, 1.0),
            track.width(),
            height,
        )
    }

    /// 第 `index` 个刻度矩形。
    #[must_use]
    pub fn tick_rect(&self, index: usize) -> Rect {
        let track = self.track_rect();
        let count = self.labels.len().max(1);
        let fraction = if count <= 1 {
            0.0
        } else {
            index as f32 / (count - 1) as f32
        };
        Rect::new(
            self.rect.max().x - 6.0,
            track.origin.y + (track.height() - 1.0) * fraction,
            3.0,
            1.0,
        )
    }

    /// 悬停预览标签矩形。
    #[must_use]
    pub fn preview_rect(&self) -> Rect {
        let norm = self.preview_norm.unwrap_or(self.thumb_norm).clamp(0.0, 1.0);
        Rect::new(
            self.rect.origin.x - 64.0,
            self.rect.origin.y + (self.rect.height() - ANNOTATED_PREVIEW_HEIGHT).max(0.0) * norm,
            56.0,
            ANNOTATED_PREVIEW_HEIGHT,
        )
    }
}

/// 输出标注滚动条的图元、命中区域与语义节点。
#[must_use]
pub fn emit_annotated_scroll_bar(
    node: u64,
    desc: &AnnotatedScrollBarDesc,
    control: ControlState,
    label_glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let hovered = !disabled && control.phase == PointerPhase::Hovered;
    let track = desc.track_rect();

    let mut primitives = Vec::with_capacity(desc.labels.len() + 5 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    primitives.push(surface(
        track,
        ANNOTATED_TRACK_WIDTH * 0.5,
        Token::CONTROL_TRACK,
        z,
    ));
    primitives.push(surface(
        desc.thumb_rect(),
        ANNOTATED_TRACK_WIDTH * 0.5,
        if disabled {
            Token::CONTROL_DISABLED_THUMB
        } else if hovered {
            Token::CONTROL_HOVER
        } else {
            Token::CONTROL_THUMB
        },
        z + 1,
    ));
    for index in 0..desc.labels.len() {
        primitives.push(surface(
            desc.tick_rect(index),
            0.0,
            if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            },
            z + 2,
        ));
    }
    primitives.push(outlined(
        desc.preview_rect(),
        4.0,
        if desc.preview_norm.is_some() {
            Token::OVERLAY_BG
        } else {
            Token::NONE
        },
        if desc.preview_norm.is_some() {
            Token::CARD_BORDER
        } else {
            Token::NONE
        },
        1.0,
        4.0,
        z + 3,
    ));
    emit_glyphs(
        label_glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 4,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect,
        4.0,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect).with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(
            A11yNode::new(Role::Slider, "Annotated scroll bar", rect)
                .clickable()
                .disabled(disabled),
        ),
        focusable: !disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> AnnotatedScrollBarDesc {
        AnnotatedScrollBarDesc::new(Rect::new(100.0, 20.0, 24.0, 200.0)).thumb(0.5)
    }

    #[test]
    fn the_thumb_travels_inside_the_track() {
        let d = desc();
        let track = d.track_rect();
        let thumb = d.thumb_rect();
        assert!(track.contains_rect(&thumb));
        assert!(thumb.origin.y > track.origin.y);
    }

    #[test]
    fn ticks_span_the_track_vertically() {
        let d = desc();
        let first = d.tick_rect(0);
        let last = d.tick_rect(d.labels.len() - 1);
        assert!(first.origin.y <= last.origin.y);
        assert!((last.max().y - d.track_rect().max().y).abs() < 1.0);
    }

    #[test]
    fn the_preview_tag_sits_left_of_the_track() {
        let d = desc().preview(0.75);
        assert!(d.preview_rect().max().x <= d.track_rect().origin.x);
        assert!(d.preview_norm.is_some());
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let d = desc().preview(0.4);
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let state = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        counts.push(
                            emit_annotated_scroll_bar(1, &d, state, &mut [])
                                .primitives
                                .len(),
                        );
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}

//! 双面板日期范围选择器（DateRangePicker），支持预设范围与悬停预览。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::date_picker::NaiveDate;
use crate::primitive::{focus_ring, glyphs, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 闭区间 `[start, end]` 日期范围。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DateRange {
    /// 起始日期。
    pub start: NaiveDate,
    /// 结束日期。
    pub end: NaiveDate,
}

impl DateRange {
    /// 归一化 `[a, b]`，确保 `start <= end`。
    #[must_use]
    pub fn ordered(a: NaiveDate, b: NaiveDate) -> Self {
        if (a.year, a.month, a.day) <= (b.year, b.month, b.day) {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

/// [`DateRangePickerDesc`] 的交互状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DateRangePickerState {
    /// 若两端点均已选择，则为已选范围。
    pub range: Option<DateRange>,
    /// 选择第二个端点时的进行中锚点日期。
    pub pending_start: Option<NaiveDate>,
}

impl DateRangePickerState {
    /// 处理点击 `date`：首次点击设置 `pending_start`，第二次点击设置 `range`。
    pub fn pick(&mut self, date: NaiveDate) {
        if let Some(start) = self.pending_start.take() {
            self.range = Some(DateRange::ordered(start, date));
        } else {
            self.pending_start = Some(date);
        }
    }
}

/// 双月 [`DateRangePicker`] 描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DateRangePickerDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 已选日期范围。
    pub range: Option<DateRange>,
    /// 无障碍标签。
    pub label: String,
    /// 基础 z-index。
    pub z: u16,
}

impl DateRangePickerDesc {
    /// 创建日期范围选择器描述符。
    #[must_use]
    pub fn new(rect: Rect, range: Option<DateRange>) -> Self {
        Self {
            rect,
            range,
            label: "Date Range Picker".to_owned(),
            z: 600,
        }
    }
}

/// 日期范围预设胶囊（`0..3`）的确定性子节点 ID。
#[must_use]
pub const fn date_range_preset_node(node: u64, index: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x600 + index as u64)
}

/// 日期范围月份面板（`0..2`）的确定性子节点 ID。
#[must_use]
pub const fn date_range_panel_node(node: u64, panel: usize) -> u64 {
    node.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (0x610 + panel as u64)
}

/// 发射固定形状的双面板日期范围选择器（1 背景 + 2 月份面板 + 3 预设胶囊 + 1 范围高亮 + 1 焦点环 = 8 个图元）。
#[must_use]
pub fn emit_date_range_picker(
    node: u64,
    desc: &DateRangePickerDesc,
    state: ControlState,
    label_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8 + label_glyphs.len());
    let mut payloads = Vec::with_capacity(label_glyphs.len());

    let hovered = state.phase == PointerPhase::Hovered;
    primitives.push(outlined(
        desc.rect,
        8.0,
        Token::SURFACE,
        if hovered || state.focused {
            Token::FOCUS_RING
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        desc.z,
    ));

    let pad = 6.0;
    let preset_h = 20.0_f32.min(desc.rect.height() * 0.25);
    let preset_w = ((desc.rect.width() - pad * 4.0) / 3.0).max(20.0);
    for i in 0..3 {
        let r = Rect::new(
            desc.rect.origin.x + pad + i as f32 * (preset_w + pad),
            desc.rect.origin.y + pad,
            preset_w,
            preset_h,
        );
        primitives.push(outlined(
            r,
            4.0,
            if hovered {
                Token::SURFACE_RAISED
            } else {
                Token::SURFACE_SUNKEN
            },
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        ));
    }

    let panel_y = desc.rect.origin.y + preset_h + pad * 2.0;
    let panel_h = (desc.rect.height() - preset_h - pad * 3.0).max(20.0);
    let panel_w = ((desc.rect.width() - pad * 3.0) * 0.5).max(24.0);
    for m in 0..2 {
        let r = Rect::new(
            desc.rect.origin.x + pad + m as f32 * (panel_w + pad),
            panel_y,
            panel_w,
            panel_h,
        );
        primitives.push(outlined(
            r,
            6.0,
            Token::SURFACE_SUNKEN,
            Token::CARD_BORDER,
            1.0,
            0.0,
            desc.z + 1,
        ));
    }

    let span_r = Rect::new(
        desc.rect.origin.x + pad + 8.0,
        panel_y + panel_h * 0.4,
        (desc.rect.width() - pad * 2.0 - 16.0).max(16.0),
        12.0,
    );
    primitives.push(surface(
        span_r,
        4.0,
        if desc.range.is_some() {
            Token::PRIMARY_SOFT
        } else {
            Token::SURFACE_RAISED
        },
        desc.z + 2,
    ));

    primitives.push(focus_ring(
        desc.rect,
        8.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        state.focused,
    ));

    if !label_glyphs.is_empty() {
        glyphs(
            label_glyphs,
            Token::TEXT_PRIMARY,
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    let hit = (!state.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
            .with_cursor(CursorKind::Pointer)
    });
    let a11y = A11yNode::new(Role::Group, desc.label.clone(), desc.rect).disabled(state.disabled);

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
        ..Default::default()
    };

    if !state.disabled {
        for i in 0..3 {
            let r = Rect::new(
                desc.rect.origin.x + pad + i as f32 * (preset_w + pad),
                desc.rect.origin.y + pad,
                preset_w,
                preset_h,
            );
            let key = date_range_preset_node(node, i);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, format!("Preset {}", i + 1), r).clickable(),
                true,
            );
        }
        for m in 0..2 {
            let r = Rect::new(
                desc.rect.origin.x + pad + m as f32 * (panel_w + pad),
                panel_y,
                panel_w,
                panel_h,
            );
            let key = date_range_panel_node(node, m);
            out.with_sub_control(
                key,
                HitRegion::new(key, CONTROL_LAYER, u32::from(desc.z) + 1, r)
                    .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Group, format!("Month Panel {}", m + 1), r).clickable(),
                false,
            );
        }
    }

    out
}

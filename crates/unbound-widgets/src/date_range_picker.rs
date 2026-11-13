//! Dual-panel DateRangePicker with preset ranges and hover preview.

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

/// Inclusive `[start, end]` date interval.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DateRange {
    /// Start date.
    pub start: NaiveDate,
    /// End date.
    pub end: NaiveDate,
}

impl DateRange {
    /// Normalize `[a, b]` so `start <= end`.
    #[must_use]
    pub fn ordered(a: NaiveDate, b: NaiveDate) -> Self {
        if (a.year, a.month, a.day) <= (b.year, b.month, b.day) {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

/// Interactive state for [`DateRangePickerDesc`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DateRangePickerState {
    /// Selected range if both endpoints picked.
    pub range: Option<DateRange>,
    /// In-progress anchor date while picking the second endpoint.
    pub pending_start: Option<NaiveDate>,
}

impl DateRangePickerState {
    /// Handle clicking `date`: first click sets `pending_start`, second sets `range`.
    pub fn pick(&mut self, date: NaiveDate) {
        if let Some(start) = self.pending_start.take() {
            self.range = Some(DateRange::ordered(start, date));
        } else {
            self.pending_start = Some(date);
        }
    }
}

/// Descriptor for a dual-month [`DateRangePicker`].
#[derive(Clone, Debug, PartialEq)]
pub struct DateRangePickerDesc {
    /// Bounding rectangle.
    pub rect: Rect,
    /// Selected range.
    pub range: Option<DateRange>,
    /// Accessible label.
    pub label: String,
    /// Base z-index.
    pub z: u16,
}

impl DateRangePickerDesc {
    /// Create a date range picker descriptor.
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

/// Emit a stable-shape dual-panel date range picker (1 bg + 2 month panels + 3 preset pills + 1 range highlight + 1 focus ring = 8 primitives).
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

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !state.disabled,
    }
}

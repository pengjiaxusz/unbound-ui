//! CalendarDatePicker：收起态像输入框、右侧带日历图标的日期选择器。
//!
//! 收起时只绘制文本框与右侧的日历 glyph 触发器；点击后语义上展开一个
//! 弹出面板（[`CalendarDatePickerState::open`]）。面板内容由调用方后续补充，
//! 这里先绘制浮层外壳，保证展开状态在场景中有可见反馈。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::date_picker::{
    emit_calendar_view, CalendarSelectionMode, CalendarState, DatePickerDesc, NaiveDate,
};
use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::text_input::{focus_underline, TEXT_INPUT_LAYER};
use crate::{
    AlignX, AlignY, ControlState, PointerPhase, TextCtx, UiWidget, WidgetInputEvent, WidgetOutput,
};

/// 标准高度。对齐 WinUI `TextControlThemeMinHeight = 32`。
pub const CALENDAR_DATE_PICKER_HEIGHT: f32 = 32.0;
/// 圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const CALENDAR_DATE_PICKER_RADIUS: f32 = 4.0;
/// 默认字号。
pub const CALENDAR_DATE_PICKER_FONT_SIZE: f32 = 14.0;
/// 右侧日历图标触发器的宽度。
pub const CALENDAR_DATE_PICKER_ICON_WIDTH: f32 = 32.0;
/// 右侧日历符号图标字形（WinUI 3 `Calendar` 码点 `U+E787`）。
pub const CALENDAR_DATE_PICKER_ICON_GLYPH: &str = "\u{E787}";
/// 右侧日历符号图标字号（对齐 WinUI 3 `CalendarDatePickerCalendarGlyph` 的 12px 尺寸）。
pub const CALENDAR_DATE_PICKER_ICON_FONT_SIZE: f32 = 12.0;
/// 弹出日历面板默认宽度。
pub const CALENDAR_DATE_PICKER_PANEL_WIDTH: f32 = 280.0;
/// 弹出日历面板默认高度（容纳 6×7 完整 CalendarView 零越界）。
pub const CALENDAR_DATE_PICKER_PANEL_HEIGHT: f32 = 280.0;
/// 弹出面板圆角半径。对齐 WinUI `OverlayCornerRadius = 8`。
pub const CALENDAR_DATE_PICKER_PANEL_RADIUS: f32 = 8.0;
/// 弹出面板阴影模糊半径。
pub const CALENDAR_DATE_PICKER_PANEL_SHADOW: f32 = 24.0;
/// 未选日期时浮层日历的默认基准日期。
const DEFAULT_CALENDAR_DATE: NaiveDate = NaiveDate {
    year: 2024,
    month: 3,
    day: 15,
};

/// 日历日期选择器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarDatePickerDesc {
    /// 控件矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 未选择日期时显示的占位文本。
    pub placeholder: String,
    /// 禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
    /// 圆角半径。
    pub radius: f32,
    /// 字号（供调用方塑形时使用）。
    pub font_size: f32,
}

impl CalendarDatePickerDesc {
    /// 使用默认度量创建日历日期选择器。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            disabled: false,
            z: 711,
            radius: CALENDAR_DATE_PICKER_RADIUS,
            font_size: CALENDAR_DATE_PICKER_FONT_SIZE,
        }
    }

    /// 构造带默认 200px 宽度与 32px 高度的日历日期选择器。
    #[must_use]
    pub fn date_picker(placeholder: impl Into<String>) -> Self {
        Self::new(Rect::new(0.0, 0.0, 200.0, CALENDAR_DATE_PICKER_HEIGHT))
            .with_placeholder(placeholder)
    }

    /// 设置绘制深度。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 设置圆角半径。
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 设置字号。
    #[must_use]
    pub fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// 设置占位文本。
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置为禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 右侧日历图标触发器矩形。
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        let width = CALENDAR_DATE_PICKER_ICON_WIDTH.min(self.rect.size.x);
        Rect::new(
            self.rect.max().x - width,
            self.rect.origin.y,
            width,
            self.rect.size.y,
        )
    }

    /// 弹出日历面板矩形（按默认视口高度贴底自动向上翻转）。
    #[must_use]
    pub fn panel_rect(&self) -> Rect {
        self.panel_rect_in_viewport(crate::DEFAULT_VIEWPORT.1)
    }

    /// 在指定视口高度下计算弹出日历面板矩形；若向下展开超出视口底边且上方空间足够，自动向上翻转。
    #[must_use]
    pub fn panel_rect_in_viewport(&self, viewport_height: f32) -> Rect {
        let w = self.rect.size.x.max(CALENDAR_DATE_PICKER_PANEL_WIDTH);
        let h = CALENDAR_DATE_PICKER_PANEL_HEIGHT;
        let below_y = self.rect.max().y + 4.0;
        let y = if below_y + h > viewport_height && self.rect.origin.y >= h + 4.0 {
            self.rect.origin.y - h - 4.0
        } else {
            below_y
        };
        Rect::new(self.rect.origin.x, y, w, h)
    }
}

/// 日历日期选择器状态：是否展开、已选日期与持久化浮层日历状态。
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarDatePickerState {
    /// 弹出面板是否展开。
    pub open: bool,
    /// 已选日期。
    pub date: Option<NaiveDate>,
    /// 弹出日历面板的持久化状态（保留翻页月份与视图钻取层级）。
    pub calendar: CalendarState,
}

impl Default for CalendarDatePickerState {
    fn default() -> Self {
        Self {
            open: false,
            date: None,
            calendar: CalendarState::new(DEFAULT_CALENDAR_DATE, CalendarSelectionMode::Single),
        }
    }
}

impl CalendarDatePickerState {
    /// 创建收起且未选日期的状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建已选日期的收起状态。
    #[must_use]
    pub fn with_date(date: NaiveDate) -> Self {
        let mut calendar = CalendarState::new(date, CalendarSelectionMode::Single);
        calendar.selected = Some(date);
        Self {
            open: false,
            date: Some(date),
            calendar,
        }
    }

    /// 设置已选日期，并同步对齐内部浮层日历的选中项与浏览月份。
    pub fn set_date(&mut self, date: Option<NaiveDate>) {
        self.date = date;
        self.sync_calendar_to_date();
    }

    /// 将内部浮层日历的选中项与浏览月份对齐至 `self.date`。
    pub fn sync_calendar_to_date(&mut self) {
        self.calendar.selected = self.date;
        if let Some(date) = self.date {
            self.calendar.open(date);
        }
    }

    /// 切换展开状态。
    pub fn toggle(&mut self) {
        if self.open {
            self.collapse();
        } else {
            self.expand();
        }
    }

    /// 展开面板，并将浮层日历对齐至当前已选日期（若有）。
    pub fn expand(&mut self) {
        if !self.open {
            self.open = true;
            self.sync_calendar_to_date();
        }
    }

    /// 收起面板。
    pub fn collapse(&mut self) {
        self.open = false;
    }

    /// 已选日期的 ISO 文本；未选择时为空串。
    #[must_use]
    pub fn selected_text(&self) -> String {
        self.date.map(NaiveDate::to_iso).unwrap_or_default()
    }

    /// 处理键盘按键输入，若状态改变则返回 `true`。
    pub fn handle_key(&mut self, chord: &KeyChord) -> bool {
        if self.open {
            return match chord.key {
                KeyName::Named("Escape") => {
                    self.collapse();
                    true
                }
                KeyName::Named("ArrowUp") if chord.alt => {
                    self.collapse();
                    true
                }
                _ => {
                    if self.calendar.handle_key(chord) {
                        if self.calendar.selected != self.date {
                            self.date = self.calendar.selected;
                            self.open = false;
                        }
                        true
                    } else {
                        false
                    }
                }
            };
        }
        match chord.key {
            KeyName::Named("Enter") | KeyName::Named("Space") => {
                self.expand();
                true
            }
            KeyName::Named("ArrowDown") if chord.alt => {
                self.expand();
                true
            }
            KeyName::Named("ArrowLeft") => {
                if let Some(date) = self.date {
                    if let Some(prev) = date.prev_day() {
                        self.set_date(Some(prev));
                        return true;
                    }
                }
                false
            }
            KeyName::Named("ArrowRight") => {
                if let Some(date) = self.date {
                    if let Some(next) = date.next_day() {
                        self.set_date(Some(next));
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

impl UiWidget for CalendarDatePickerDesc {
    type State = CalendarDatePickerState;

    fn initial_state(&self) -> Self::State {
        CalendarDatePickerState::new()
    }

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let sample = if !self.placeholder.is_empty() {
            self.placeholder.as_str()
        } else {
            "2024-00-00"
        };
        let measured = text_ctx.measure_line(sample, self.font_size, 400);
        Vec2::new(
            (measured.x + 48.0).max(self.rect.width()).max(160.0),
            CALENDAR_DATE_PICKER_HEIGHT,
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if self.disabled {
            return false;
        }
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => {
                let mut desc = self.clone();
                desc.rect = rect;
                if state.open && desc.panel_rect().contains_point(*pos) {
                    let cal_desc = DatePickerDesc::new(desc.panel_rect()).disabled(desc.disabled);
                    if state.calendar.handle_click(&cal_desc, *pos) {
                        if state.calendar.selected != state.date {
                            state.date = state.calendar.selected;
                            state.open = false;
                        }
                        return true;
                    }
                    return false;
                }
                if desc.icon_rect().contains_point(*pos) || desc.rect.contains_point(*pos) {
                    state.toggle();
                    true
                } else {
                    false
                }
            }
            WidgetInputEvent::Wheel { delta } if state.open => state.calendar.handle_wheel(delta.y),
            WidgetInputEvent::KeyDown(chord) => state.handle_key(chord),
            _ => false,
        }
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let text = if let Some(date) = state.date {
            date.to_iso()
        } else {
            desc.placeholder.clone()
        };
        let text_rect = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y,
            (rect.width() - 44.0).max(0.0),
            rect.height(),
        );
        let mut glyphs = text_ctx.shape_line(
            &text,
            text_rect,
            desc.font_size,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        // WinUI CalendarDatePicker 日期数字采用等宽表格式步进；Segoe UI Variable Text
        // 默认比例数字 '1' 步进偏窄约 2px，导致 '1' 及后续字符整体左移。
        if state.date.is_some() && glyphs.len() == text.len() {
            let mut shift = 0.0_f32;
            for (ch, quad) in text.bytes().zip(glyphs.iter_mut()) {
                if ch == b'1' {
                    quad.rect.origin.x += shift + 1.0;
                    shift += 2.0;
                } else if shift > 0.0 {
                    quad.rect.origin.x += shift;
                }
            }
        }
        // 右侧日历图标优先通过平台感知符号字体塑形；不可用或被禁用时回退 ShellIcon::Calendar。
        let icon_glyphs = text_ctx
            .shape_symbol_glyph(
                CALENDAR_DATE_PICKER_ICON_GLYPH,
                desc.icon_rect(),
                CALENDAR_DATE_PICKER_ICON_FONT_SIZE,
            )
            .unwrap_or_default();
        emit_calendar_date_picker_with_icon(
            node,
            &desc,
            state,
            control,
            &mut glyphs,
            &icon_glyphs,
            Some(text_ctx),
        )
    }
}

/// 弹出日历视图的确定性子节点键。
#[must_use]
pub const fn calendar_popup_node(node: u64) -> u64 {
    node ^ 0x4341_4C50_4F50
}

/// 日历图标触发器的确定性子节点键。
#[must_use]
pub const fn calendar_icon_node(node: u64) -> u64 {
    node ^ 0x4341_4C45_4E44
}

/// 输出日历日期选择器的图元、命中区域与语义节点（无符号字形时自动回退 [`crate::shell::ShellIcon::Calendar`]）。
#[must_use]
pub fn emit_calendar_date_picker(
    node: u64,
    desc: &CalendarDatePickerDesc,
    state: &CalendarDatePickerState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    emit_calendar_date_picker_with_icon(node, desc, state, control, glyphs, &[], None)
}

/// 输出日历日期选择器的图元、命中区域与语义节点，支持传入已塑形的右侧日历符号字形。
#[must_use]
fn emit_calendar_date_picker_with_icon(
    node: u64,
    desc: &CalendarDatePickerDesc,
    state: &CalendarDatePickerState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
    icon_glyphs: &[GlyphQuad],
    text_ctx: Option<&mut TextCtx<'_>>,
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let radii = [desc.radius; 4];
    let disabled = control.disabled || desc.disabled;
    let icon = desc.icon_rect();

    // 收起态与 TextBox 同色；展开时叠加一档悬停色，提示面板已打开。
    let fill = if disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == PointerPhase::Hovered || state.open {
        Token::CONTROL_HOVER
    } else {
        Token::CONTROL_FILL
    };
    let border = if disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::CONTROL_STROKE
    };
    let ink = if disabled {
        Token::TEXT_DISABLED
    } else if state.date.is_some() {
        Token::TEXT_PRIMARY
    } else {
        Token::TEXT_SECONDARY
    };
    let icon_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };

    let mut primitives = Vec::with_capacity(9 + glyphs.len() + icon_glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len() + icon_glyphs.len() + 2);

    primitives.push(surface(rect, desc.radius, fill, z));
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::NONE,
        border,
        1.0,
        0.0,
        z + 1,
    ));
    primitives.push(focus_underline(
        rect,
        desc.radius,
        z + 1,
        if control.focused && !disabled {
            1.0
        } else {
            0.0
        },
    ));

    // 底边强调线在所有状态保留图元槽位，禁用/聚焦态以 NONE 隐藏，维持恒定形状契约。
    primitives.push(surface(
        Rect::new(
            rect.origin.x + 3.0,
            rect.max().y - 1.0,
            (rect.width() - 6.0).max(0.0),
            1.0,
        ),
        0.0,
        if !disabled && !control.focused {
            Token::CONTROL_STROKE_SECONDARY
        } else {
            Token::NONE
        },
        z + 1,
    ));

    // 右侧日历图标：优先发射符号字体字形；无可用符号字体时回退校准后的 ShellIcon::Calendar。
    if !icon_glyphs.is_empty() {
        emit_glyphs(icon_glyphs, icon_ink, z + 2, &mut primitives, &mut payloads);
    } else {
        crate::shell::emit_shell_icon_primitives(
            crate::shell::ShellIcon::Calendar,
            icon,
            icon_ink,
            Token::SURFACE,
            z + 2,
            &mut primitives,
            &mut payloads,
        );
    }

    emit_glyphs(glyphs, ink, z + 2, &mut primitives, &mut payloads);
    primitives.push(focus_ring(
        rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        z + 2,
        control.focused && !disabled,
    ));

    // 展开态准备浮层投影（当提供 TextCtx 时直接挂载真实 CalendarView，否则保留兼容外壳）。
    let panel_rect = state.open.then(|| desc.panel_rect());
    if let Some(panel) = panel_rect {
        primitives.push(instance(
            panel,
            [0.0; 4],
            Token::NONE,
            Token::NONE,
            0.0,
            CALENDAR_DATE_PICKER_PANEL_SHADOW,
            PrimType::Shadow,
            SubElement::Shadow,
            z + 8,
        ));
        if text_ctx.is_none() {
            primitives.push(surface(
                panel,
                CALENDAR_DATE_PICKER_PANEL_RADIUS,
                Token::OVERLAY_BG,
                z + 8,
            ));
            primitives.push(outlined(
                panel,
                CALENDAR_DATE_PICKER_PANEL_RADIUS,
                Token::NONE,
                Token::CARD_BORDER,
                1.0,
                0.0,
                z + 8,
            ));
        }
    }

    let hit = (!disabled).then(|| {
        HitRegion::new(node, TEXT_INPUT_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded { radii })
            .with_cursor(CursorKind::Pointer)
    });

    let mut a11y = A11yNode::new(
        Role::ComboBox,
        if desc.label.is_empty() {
            desc.placeholder.clone()
        } else {
            desc.label.clone()
        },
        rect,
    )
    .clickable()
    .disabled(disabled)
    .expanded(state.open);
    a11y.value = state.selected_text();

    let mut out = WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    };

    if !disabled {
        let key = calendar_icon_node(node);
        out.with_sub_control(
            key,
            HitRegion::new(key, TEXT_INPUT_LAYER, u32::from(z) + 1, icon)
                .with_cursor(CursorKind::Pointer),
            A11yNode::new(
                Role::Button,
                if state.open {
                    "Close calendar"
                } else {
                    "Open calendar"
                },
                icon,
            )
            .clickable()
            .expanded(state.open),
            true,
        );
    }

    if let (Some(panel), Some(ctx)) = (panel_rect, text_ctx) {
        let cal_desc = DatePickerDesc::new(panel)
            .with_label("CalendarDatePicker flyout")
            .with_z(z.saturating_add(10))
            .disabled(disabled);
        let cal_out = emit_calendar_view(
            calendar_popup_node(node),
            &cal_desc,
            &state.calendar,
            ControlState {
                disabled,
                ..ControlState::default()
            },
            ctx,
        );
        out.extend(cal_out);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CalendarDatePickerDesc {
        CalendarDatePickerDesc::new(Rect::new(0.0, 0.0, 220.0, CALENDAR_DATE_PICKER_HEIGHT))
            .with_placeholder("Select a date")
    }

    fn date() -> NaiveDate {
        NaiveDate::new(2024, 3, 15).expect("valid date")
    }

    #[test]
    fn the_picker_starts_collapsed() {
        let state = CalendarDatePickerState::new();
        assert!(!state.open);
        assert_eq!(state.selected_text(), "");
    }

    #[test]
    fn clicking_the_trigger_expands_the_panel_shell() {
        let mut state = CalendarDatePickerState::with_date(date());
        let collapsed =
            emit_calendar_date_picker(1, &desc(), &state, ControlState::default(), &mut []);
        state.toggle();
        let expanded =
            emit_calendar_date_picker(1, &desc(), &state, ControlState::default(), &mut []);
        assert!(
            expanded.primitives.len() > collapsed.primitives.len(),
            "the open panel must add visible shell primitives"
        );
        assert_eq!(expanded.a11y.expect("reachable").expanded, Some(true));
        assert_eq!(expanded.sub_a11y[0].1.label, "Close calendar");
        state.collapse();
        assert!(!state.open);
    }

    #[test]
    fn the_a11y_value_reports_the_selected_iso_date() {
        let out = emit_calendar_date_picker(
            1,
            &desc(),
            &CalendarDatePickerState::with_date(date()),
            ControlState::default(),
            &mut [],
        );
        assert_eq!(out.a11y.expect("reachable").value, "2024-03-15");
        assert_eq!(out.sub_hits.len(), 1);
        assert_eq!(out.sub_hits[0].node, calendar_icon_node(1));
    }

    #[test]
    fn hover_changes_the_field() {
        let idle = emit_calendar_date_picker(
            1,
            &desc(),
            &CalendarDatePickerState::new(),
            ControlState::default(),
            &mut [],
        );
        let hovered = emit_calendar_date_picker(
            1,
            &desc(),
            &CalendarDatePickerState::new(),
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
            &mut [],
        );
        assert_ne!(idle.primitives, hovered.primitives);
    }

    #[test]
    fn the_primitive_count_is_constant_across_states() {
        let state = CalendarDatePickerState::new();
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        counts.push(
                            emit_calendar_date_picker(1, &desc(), &state, control, &mut [])
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

    #[test]
    fn replacing_wireframe_icon_keeps_all_glyph_payload_indices_in_bounds() {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        };
        let rect = Rect::new(16.0, 16.0, 200.0, CALENDAR_DATE_PICKER_HEIGHT);
        let d = CalendarDatePickerDesc::new(rect).with_placeholder("Select a date");
        let s = CalendarDatePickerState::with_date(date());
        let out = d.emit_laid_out(1, rect, ControlState::default(), &s, &mut text_ctx);
        for prim in &out.primitives {
            let pt = prim.prim_type_and_flags & 0xFF;
            if pt == PrimType::Glyph as u32 || pt == PrimType::GlyphColor as u32 {
                assert!(
                    (prim.extra_payload_idx as usize) < out.payloads.len(),
                    "glyph extra_payload_idx {} out of bounds (payloads len {})",
                    prim.extra_payload_idx,
                    out.payloads.len()
                );
            }
        }
    }

    #[test]
    fn flyout_calendar_navigation_and_drill_up_persist_across_events() {
        use crate::date_picker::{CalendarDisplayMode, YearMonth};

        let d = desc();
        let mut state = CalendarDatePickerState::with_date(date());

        // 点击右侧图标展开浮层，初始视图应与已选日期 2024-03 对齐。
        let opened = d.reduce(
            &mut state,
            d.rect,
            &WidgetInputEvent::PointerDown {
                pos: d.icon_rect().center(),
                click_count: 1,
            },
        );
        assert!(opened);
        assert!(state.open);
        assert_eq!(state.calendar.view, YearMonth { year: 2024, month: 3 });

        // 点击浮层下一月按钮：浮层保持展开且视图持续推进，不弹回原月份。
        let cal_desc = DatePickerDesc::new(d.panel_rect());
        let paged = d.reduce(
            &mut state,
            d.rect,
            &WidgetInputEvent::PointerDown {
                pos: cal_desc.next_rect().center(),
                click_count: 1,
            },
        );
        assert!(paged);
        assert!(state.open);
        assert_eq!(state.calendar.view, YearMonth { year: 2024, month: 4 });

        let paged_again = d.reduce(
            &mut state,
            d.rect,
            &WidgetInputEvent::PointerDown {
                pos: cal_desc.next_rect().center(),
                click_count: 1,
            },
        );
        assert!(paged_again);
        assert!(state.open);
        assert_eq!(state.calendar.view, YearMonth { year: 2024, month: 5 });

        // 点击钻取标题切换到 Year 视图：浮层保持展开且模式持久化。
        let drilled = d.reduce(
            &mut state,
            d.rect,
            &WidgetInputEvent::PointerDown {
                pos: cal_desc.header_title_rect().center(),
                click_count: 1,
            },
        );
        assert!(drilled);
        assert!(state.open);
        assert_eq!(state.calendar.display_mode, CalendarDisplayMode::Year);
    }
}

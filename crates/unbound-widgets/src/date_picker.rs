//! 日期选择器（DatePicker）：以周一为首日的日历，支持单选或范围选择。
//!
//! 日期是没有时钟与时区的纯值（[`NaiveDate`]），这是日历唯一合理的契约：
//! 在应用附加时区之前，“2024-03-01”在上海和柏林表示同一天。
//!
//! 网格固定为 6 行 7 列。渲染固定的 42 格网格可使翻页时弹出面板高度保持稳定，
//! 周围滚动位置不会发生跳动。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, WidgetOutput};

/// 日历显示的单元格总数。
pub const CALENDAR_CELLS: usize = 42;
/// 日历列数。
pub const CALENDAR_COLUMNS: usize = 7;
/// 头部栏高度。
pub const CALENDAR_HEADER: f32 = 34.0;
/// 星期标题行高度。
pub const CALENDAR_WEEKDAYS: f32 = 22.0;
/// 日期单元格圆角半径。
pub const CALENDAR_RADIUS: f32 = 6.0;

/// 不含时间与时区的纯日历日期。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NaiveDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl NaiveDate {
    /// 当三元组有效时构造日期。
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// ISO 格式字符串表示 `YYYY-MM-DD`。
    #[must_use]
    pub fn to_iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// 解析 `YYYY-MM-DD`。
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Self::new(year, month, day)
    }

    /// 以周一为 `0` 的星期索引。
    #[must_use]
    pub fn weekday(self) -> u8 {
        // Sakamoto 算法，偏移为周一优先索引。
        const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let mut year = self.year;
        if self.month < 3 {
            year -= 1;
        }
        let sunday = (year + year / 4 - year / 100
            + year / 400
            + OFFSETS[usize::from(self.month - 1)]
            + i32::from(self.day))
        .rem_euclid(7);
        ((sunday + 6) % 7) as u8
    }

    /// 当月第一天。
    #[must_use]
    pub fn month_start(self) -> Self {
        Self { day: 1, ..self }
    }
}

/// 判断 `year` 在公历中是否为闰年。
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// 计算 `month` 的天数。
#[must_use]
pub const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// 年份与月份。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct YearMonth {
    pub year: i32,
    pub month: u8,
}

impl YearMonth {
    /// 包含 `date` 的月份。
    #[must_use]
    pub const fn of(date: NaiveDate) -> Self {
        Self {
            year: date.year,
            month: date.month,
        }
    }

    /// 上一个月。
    #[must_use]
    pub fn prev(self) -> Self {
        if self.month == 1 {
            Self {
                year: self.year - 1,
                month: 12,
            }
        } else {
            Self {
                year: self.year,
                month: self.month - 1,
            }
        }
    }

    /// 下一个月。
    #[must_use]
    pub fn next(self) -> Self {
        if self.month == 12 {
            Self {
                year: self.year + 1,
                month: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    /// 当月第一天。
    #[must_use]
    pub fn first(self) -> NaiveDate {
        NaiveDate {
            year: self.year,
            month: self.month,
            day: 1,
        }
    }
}

/// 选区包含的天数范围。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionMode {
    /// 仅选中一天。
    #[default]
    Single,
    /// 闭区间日期范围。
    Range,
}

/// 6×7 网格中的单个单元格。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarCell {
    /// 日期值；前置/后置填充格为 `None`。
    pub date: Option<NaiveDate>,
    /// 该日期是否属于当前浏览月份。
    pub in_month: bool,
    /// 该日期是否被选中。
    pub selected: bool,
    /// 该日期是否位于已提交的范围内。
    pub in_range: bool,
    /// 该日期是否为 `today`。
    pub today: bool,
}

/// 日历的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarState {
    view: YearMonth,
    selected: Option<NaiveDate>,
    range_end: Option<NaiveDate>,
    hovered: Option<NaiveDate>,
    today: NaiveDate,
    mode: SelectionMode,
}

impl CalendarState {
    /// 创建打开在 `today` 所在月份的日历。
    #[must_use]
    pub fn new(today: NaiveDate, mode: SelectionMode) -> Self {
        Self {
            view: YearMonth::of(today),
            selected: None,
            range_end: None,
            hovered: None,
            today,
            mode,
        }
    }

    /// 当前浏览的月份。
    #[must_use]
    pub fn view(&self) -> YearMonth {
        self.view
    }

    /// 当前选择模式。
    #[must_use]
    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// 将日历切换至 `date` 所在月份。
    pub fn open(&mut self, date: NaiveDate) {
        self.view = YearMonth::of(date);
    }

    /// 按整月翻页。
    pub fn page(&mut self, months: i32) -> bool {
        if months == 0 {
            return false;
        }
        let mut view = self.view;
        for _ in 0..months.unsigned_abs() {
            view = if months > 0 { view.next() } else { view.prev() };
        }
        self.view = view;
        true
    }

    /// 跳转至包含 `today` 的月份。
    pub fn today(&mut self) {
        self.view = YearMonth::of(self.today);
    }

    /// 返回已提交的有序选区。
    #[must_use]
    pub fn range(&self) -> Option<(NaiveDate, NaiveDate)> {
        match (self.selected, self.range_end) {
            (Some(start), Some(end)) => Some(if start <= end {
                (start, end)
            } else {
                (end, start)
            }),
            (Some(day), None) => Some((day, day)),
            _ => None,
        }
    }

    /// 当前悬停的日期。
    #[must_use]
    pub fn hovered(&self) -> Option<NaiveDate> {
        self.hovered
    }

    /// 设置悬停日期。
    pub fn hover(&mut self, date: Option<NaiveDate>) {
        self.hovered = date;
    }

    /// 选择某一天。
    ///
    /// 单选模式下直接替换选区；范围模式下第一次选择开启范围，第二次选择完成范围；
    /// 再次选择则开启新的范围。
    pub fn select(&mut self, date: NaiveDate) {
        match self.mode {
            SelectionMode::Single => {
                self.selected = Some(date);
                self.range_end = None;
            }
            SelectionMode::Range => {
                if self.selected.is_none() || self.range_end.is_some() {
                    self.selected = Some(date);
                    self.range_end = None;
                } else {
                    self.range_end = Some(date);
                }
            }
        }
        self.view = YearMonth::of(date);
    }

    /// 判断 `date` 是否落在已提交范围内。
    #[must_use]
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.range()
            .is_some_and(|(start, end)| date >= start && date <= end)
    }

    /// 返回当前浏览月份的 42 个单元格，周一在前。
    #[must_use]
    pub fn cells(&self) -> Vec<CalendarCell> {
        let first = self.view.first();
        let lead = usize::from(first.weekday());
        let mut cells = Vec::with_capacity(CALENDAR_CELLS);
        for index in 0..CALENDAR_CELLS {
            let offset = index as i64 - lead as i64;
            let date = day_offset(first, offset);
            let in_month = date
                .is_some_and(|date| date.year == self.view.year && date.month == self.view.month);
            cells.push(CalendarCell {
                date,
                in_month,
                selected: date.is_some() && self.range().is_some_and(|(s, _)| Some(s) == date),
                in_range: date.is_some() && self.contains(date.unwrap()),
                today: date == Some(self.today),
            });
        }
        cells
    }
}

fn day_offset(first: NaiveDate, offset: i64) -> Option<NaiveDate> {
    let mut date = first;
    if offset >= 0 {
        for _ in 0..offset {
            date = next_day(date);
        }
    } else {
        for _ in 0..-offset {
            date = prev_day(date);
        }
    }
    Some(date)
}

fn next_day(date: NaiveDate) -> NaiveDate {
    if date.day < days_in_month(date.year, date.month) {
        NaiveDate {
            day: date.day + 1,
            ..date
        }
    } else if date.month == 12 {
        NaiveDate {
            year: date.year + 1,
            month: 1,
            day: 1,
        }
    } else {
        NaiveDate {
            month: date.month + 1,
            day: 1,
            ..date
        }
    }
}

fn prev_day(date: NaiveDate) -> NaiveDate {
    if date.day > 1 {
        NaiveDate {
            day: date.day - 1,
            ..date
        }
    } else if date.month == 1 {
        NaiveDate {
            year: date.year - 1,
            month: 12,
            day: 31,
        }
    } else {
        let month = date.month - 1;
        NaiveDate {
            year: date.year,
            month,
            day: days_in_month(date.year, month),
        }
    }
}

/// 日历的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct DatePickerDesc {
    /// 日历矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 日历是否禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl DatePickerDesc {
    /// 使用默认度量的日历。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            disabled: false,
            z: 742,
        }
    }

    /// 上一月按钮矩形。
    #[must_use]
    pub fn prev_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + 6.0,
            self.rect.origin.y + 4.0,
            28.0,
            26.0,
        )
    }

    /// 下一月按钮矩形。
    #[must_use]
    pub fn next_rect(&self) -> Rect {
        Rect::new(
            self.rect.max().x - 34.0,
            self.rect.origin.y + 4.0,
            28.0,
            26.0,
        )
    }

    /// 指定网格单元格的矩形。
    #[must_use]
    pub fn cell_rect(&self, index: usize) -> Rect {
        let column = (index % CALENDAR_COLUMNS) as f32;
        let row = (index / CALENDAR_COLUMNS) as f32;
        let width = self.rect.size.x / CALENDAR_COLUMNS as f32;
        let top = CALENDAR_HEADER + CALENDAR_WEEKDAYS;
        let height = ((self.rect.size.y - top) / 6.0).max(0.0);
        Rect::new(
            self.rect.origin.x + column * width + 2.0,
            self.rect.origin.y + top + row * height + 2.0,
            (width - 4.0).max(0.0),
            (height - 4.0).max(0.0),
        )
    }
}

/// 发射日历框架、单元格与语义节点。
#[must_use]
pub fn emit_calendar(
    node: u64,
    desc: &DatePickerDesc,
    state: &CalendarState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(CALENDAR_CELLS + 8 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    primitives.push(surface(rect, 10.0, Token::OVERLAY_BG, desc.z));
    primitives.push(outlined(
        rect,
        10.0,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        desc.z,
    ));
    primitives.push(surface(
        Rect::new(
            rect.origin.x,
            rect.origin.y,
            rect.size.x,
            CALENDAR_HEADER + CALENDAR_WEEKDAYS,
        ),
        0.0,
        Token::HEADER_BG,
        desc.z,
    ));

    // 上一月 / 下一月箭头各自绘制为两条描边。
    for (button, signs) in [
        (desc.prev_rect(), [1.0, -1.0]),
        (desc.next_rect(), [-1.0, 1.0]),
    ] {
        let cx = button.center().x;
        let cy = button.center().y;
        for sign in signs {
            primitives.push(instance(
                Rect::new(cx + sign * 1.5 - 2.0, cy - 0.75, 4.0, 1.5),
                [0.0; 4],
                Token::TEXT_SECONDARY,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Content,
                desc.z,
            ));
        }
    }

    for (index, cell) in state.cells().iter().enumerate() {
        let Some(date) = cell.date else {
            continue;
        };
        let cell_rect = desc.cell_rect(index);
        let fill = if cell.selected {
            Token::PRIMARY
        } else if cell.in_range {
            Token::PRIMARY_SOFT
        } else if state.hovered == Some(date) {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        primitives.push(surface(cell_rect, CALENDAR_RADIUS, fill, desc.z));
        if cell.today {
            primitives.push(outlined(
                cell_rect,
                CALENDAR_RADIUS,
                Token::NONE,
                Token::PRIMARY,
                1.0,
                0.0,
                desc.z,
            ));
        }
    }

    emit_glyphs(
        glyphs,
        if control.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );

    primitives.push(focus_ring(
        rect.inflate(3.0),
        13.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded { radii: [10.0; 4] })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Grid, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state
        .range()
        .map(|(start, end)| {
            if start == end {
                start.to_iso()
            } else {
                format!("{} / {}", start.to_iso(), end.to_iso())
            }
        })
        .unwrap_or_default();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> NaiveDate {
        NaiveDate::new(year, month, day).expect("valid date")
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2100));
        assert!(is_leap_year(2000));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
    }

    #[test]
    fn weekdays_are_monday_first() {
        assert_eq!(date(2024, 1, 1).weekday(), 0);
        assert_eq!(date(2024, 1, 7).weekday(), 6);
        assert_eq!(date(2024, 2, 29).weekday(), 3);
    }

    #[test]
    fn iso_parsing_round_trips() {
        assert_eq!(NaiveDate::parse("2024-02-29"), Some(date(2024, 2, 29)));
        assert_eq!(NaiveDate::parse("2023-02-29"), None);
        assert_eq!(NaiveDate::parse("2024-13-01"), None);
        assert_eq!(date(2024, 2, 29).to_iso(), "2024-02-29");
    }

    #[test]
    fn paging_rolls_over_the_year() {
        let mut state = CalendarState::new(date(2024, 1, 15), SelectionMode::Single);
        assert_eq!(
            state.view(),
            YearMonth {
                year: 2024,
                month: 1
            }
        );
        assert!(state.page(-1));
        assert_eq!(
            state.view(),
            YearMonth {
                year: 2023,
                month: 12
            }
        );
        assert!(state.page(1));
        assert_eq!(
            state.view(),
            YearMonth {
                year: 2024,
                month: 1
            }
        );
        assert!(!state.page(0));
    }

    #[test]
    fn the_grid_has_42_cells_and_pads_the_first_week() {
        let state = CalendarState::new(date(2024, 1, 15), SelectionMode::Single);
        let cells = state.cells();
        assert_eq!(cells.len(), CALENDAR_CELLS);
        assert!(cells[0].date.is_some(), "January 2024 starts on a Monday");
        assert!(cells.iter().any(|cell| cell.today));

        let february = CalendarState::new(date(2024, 2, 1), SelectionMode::Single);
        let cells = february.cells();
        assert!(!cells[0].in_month, "February 2024 starts on a Thursday");
        assert!(cells.iter().any(|cell| cell.date == Some(date(2024, 2, 1))));
    }

    #[test]
    fn single_selection_replaces_the_previous_day() {
        let mut state = CalendarState::new(date(2024, 3, 10), SelectionMode::Single);
        state.select(date(2024, 3, 4));
        state.select(date(2024, 3, 9));
        assert_eq!(state.range(), Some((date(2024, 3, 9), date(2024, 3, 9))));
    }

    #[test]
    fn range_selection_orders_its_endpoints() {
        let mut state = CalendarState::new(date(2024, 3, 10), SelectionMode::Range);
        state.select(date(2024, 3, 20));
        state.select(date(2024, 3, 5));
        assert_eq!(state.range(), Some((date(2024, 3, 5), date(2024, 3, 20))));
        assert!(state.contains(date(2024, 3, 10)));
        assert!(!state.contains(date(2024, 3, 21)));
        assert!(state.cells().iter().any(|cell| cell.in_range));
    }

    #[test]
    fn a_third_range_pick_starts_over() {
        let mut state = CalendarState::new(date(2024, 3, 10), SelectionMode::Range);
        state.select(date(2024, 3, 1));
        state.select(date(2024, 3, 2));
        state.select(date(2024, 3, 8));
        assert_eq!(state.range(), Some((date(2024, 3, 8), date(2024, 3, 8))));
    }

    #[test]
    fn the_grid_places_seven_columns() {
        let desc = DatePickerDesc::new(Rect::new(0.0, 0.0, 280.0, 300.0));
        let zero = desc.cell_rect(0);
        let six = desc.cell_rect(6);
        let seven = desc.cell_rect(7);
        assert!(six.origin.x > zero.origin.x);
        assert!((seven.origin.x - zero.origin.x).abs() < 1e-4);
        assert!(seven.origin.y > zero.origin.y);
    }

    #[test]
    fn the_emitter_reports_a_grid_with_the_selected_range() {
        let mut state = CalendarState::new(date(2024, 3, 10), SelectionMode::Range);
        state.select(date(2024, 3, 4));
        state.select(date(2024, 3, 6));
        let output = emit_calendar(
            21,
            &DatePickerDesc::new(Rect::new(0.0, 0.0, 280.0, 300.0)),
            &state,
            ControlState::default(),
            &mut [],
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Grid);
        assert_eq!(a11y.value, "2024-03-04 / 2024-03-06");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
    }
}

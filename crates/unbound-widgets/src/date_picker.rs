//! 日历视图（CalendarView）：WinUI 3 对齐的单一真源日历控件，支持单选、离散多选与区间范围选择。
//!
//! 日期是没有时钟与时区的纯值（[`NaiveDate`]），这是日历唯一合理的契约：
//! 在应用附加时区之前，“2024-03-01”在上海和柏林表示同一天。
//!
//! 控件支持三级视图钻取（`Month` 6×7 日格 ↔ `Year` 4×3 月格 ↔ `Decade` 4×3 年格）、
//! `MinDate`/`MaxDate` 边界、`BlackoutDates` 禁选与跨区约束、悬停范围预览、
//! 滚轮与键盘导航、ADR-0033 平台符号图标回退及 ADR-0037 局部裁剪自洽。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuClipNode, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, KeyChord, KeyName, Shape};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{
    curve_stroke, focus_ring, glyphs as emit_glyphs, instance, outlined, surface,
};
use crate::shell::{emit_shell_icon_primitives, ShellIcon};
use crate::switch::CONTROL_LAYER;
use crate::{AlignX, AlignY, ControlState, TextCtx, UiWidget, WidgetInputEvent, WidgetOutput};

/// 日历显示的单元格总数（6 行 × 7 列）。
pub const CALENDAR_CELLS: usize = 42;
/// 日历列数。
pub const CALENDAR_COLUMNS: usize = 7;
/// 日历行数。
pub const CALENDAR_ROWS: usize = 6;
/// 年/十年视图单元格总数（4 列 × 3 行）。
pub const CALENDAR_GRID_ITEMS: usize = 12;
/// 头部栏默认参考高度。
pub const CALENDAR_HEADER: f32 = 44.0;
/// 星期标题行默认参考高度。
pub const CALENDAR_WEEKDAYS: f32 = 24.0;
/// 日期单元格与外框圆角半径。对齐 WinUI `ControlCornerRadius = 4`。
pub const CALENDAR_RADIUS: f32 = 4.0;
/// 日期单元格推荐边长。对齐 WinUI `CalendarViewDayItem` 的 `40×40`。
pub const CALENDAR_CELL_SIZE: f32 = 40.0;
/// 日期单元格与网格槽位之间的留白。对齐 WinUI `CalendarViewDayItem` 的 `Margin = 1`。
pub const CALENDAR_CELL_INSET: f32 = 1.0;
/// 弹出层圆角半径。对齐 WinUI `OverlayCornerRadius = 8`。
pub const CALENDAR_POPUP_RADIUS: f32 = 8.0;
/// 弹出层阴影模糊半径。对应 WinUI `FlyoutPresenter.IsDefaultShadowEnabled = True`。
pub const CALENDAR_POPUP_SHADOW: f32 = 24.0;
/// 弹出层内边距。
pub const CALENDAR_POPUP_PADDING: f32 = 4.0;
/// 上翻页符号字形（WinUI 3 `ChevronUp` 码点 `U+E70E`）。
pub const CALENDAR_CHEVRON_UP_GLYPH: &str = "\u{E70E}";
/// 下翻页符号字形（WinUI 3 `ChevronDown` 码点 `U+E70D`）。
pub const CALENDAR_CHEVRON_DOWN_GLYPH: &str = "\u{E70D}";

/// 星期短标签表（周日为 0 .. 周六为 6）。
const WEEKDAY_SHORT_NAMES: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
/// 月份全称表（1..=12）。
const MONTH_FULL_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// 月份简称表（1..=12，用于 Year 视图 4×3 网格）。
const MONTH_SHORT_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// 不含时间与时区的纯日历日期。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// 以周一为 `0` 的星期索引（`0` = 周一 .. `6` = 周日）。
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

    /// 以周日为 `0` 的星期索引（`0` = 周日 .. `6` = 周六，对齐 WinUI 3）。
    #[must_use]
    pub fn weekday_sunday_first(self) -> u8 {
        (self.weekday() + 1) % 7
    }

    /// 当月第一天。
    #[must_use]
    pub fn month_start(self) -> Self {
        Self { day: 1, ..self }
    }

    /// 当月最后一天。
    #[must_use]
    pub fn month_end(self) -> Self {
        Self {
            day: days_in_month(self.year, self.month),
            ..self
        }
    }

    /// 前一天。
    #[must_use]
    pub fn prev_day(self) -> Option<Self> {
        if self.day > 1 {
            Some(Self {
                day: self.day - 1,
                ..self
            })
        } else if self.month > 1 {
            let month = self.month - 1;
            let day = days_in_month(self.year, month);
            Some(Self {
                year: self.year,
                month,
                day,
            })
        } else {
            let year = self.year.checked_sub(1)?;
            Some(Self {
                year,
                month: 12,
                day: 31,
            })
        }
    }

    /// 后一天。
    #[must_use]
    pub fn next_day(self) -> Option<Self> {
        let max_day = days_in_month(self.year, self.month);
        if self.day < max_day {
            Some(Self {
                day: self.day + 1,
                ..self
            })
        } else if self.month < 12 {
            Some(Self {
                year: self.year,
                month: self.month + 1,
                day: 1,
            })
        } else {
            let year = self.year.checked_add(1)?;
            Some(Self {
                year,
                month: 1,
                day: 1,
            })
        }
    }

    /// 偏移指定天数（正数向后，负数向前）。
    #[must_use]
    pub fn offset_days(self, delta: i64) -> Option<Self> {
        day_offset(self, delta)
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// 当月最后一天。
    #[must_use]
    pub fn last(self) -> NaiveDate {
        NaiveDate {
            year: self.year,
            month: self.month,
            day: days_in_month(self.year, self.month),
        }
    }

    /// 英文月份全称与年份标题（如 `"January 2024"`）。
    #[must_use]
    pub fn title(self) -> String {
        let name = MONTH_FULL_NAMES
            .get(usize::from(self.month.saturating_sub(1)))
            .copied()
            .unwrap_or("January");
        format!("{name} {}", self.year)
    }
}

/// 日历选择模式（对齐 WinUI 3 `CalendarViewSelectionMode` 并扩展 `Range`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CalendarSelectionMode {
    /// 禁止选中任何日期。
    None,
    /// 仅选中单个日期（再次点击同一日期可反选）。
    #[default]
    Single,
    /// 离散多选（维护 `selected_dates` 列表）。
    Multiple,
    /// 连续日期范围选择（支持悬停预览与反向选择自动归一化）。
    Range,
}

/// 兼容历史命名的选择模式类型别名。
pub type SelectionMode = CalendarSelectionMode;

/// 日历三级视图模式（对齐 WinUI 3 `CalendarViewDisplayMode`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CalendarDisplayMode {
    /// 月视图：6×7 日格。
    #[default]
    Month,
    /// 年视图：4×3 月格。
    Year,
    /// 十年视图：4×3 年格。
    Decade,
}

/// 日历指针坐标命中目标。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalendarHitTarget {
    /// 顶部左侧月/年/十年标题钻取按钮。
    HeaderTitle,
    /// 上一页翻页按钮（向上箭头）。
    PrevButton,
    /// 下一页翻页按钮（向下箭头）。
    NextButton,
    /// 月视图日期单元格。
    DayCell(NaiveDate),
    /// 年视图月份单元格 `(year, month)`。
    MonthCell(i32, u8),
    /// 十年视图年份单元格。
    YearCell(i32),
}

/// 6×7 网格中的单个单元格。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarCell {
    /// 日期值；前置/后置填充格为实际相邻月份日期。
    pub date: Option<NaiveDate>,
    /// 该日期是否属于当前浏览月份。
    pub in_month: bool,
    /// 该日期是否被选中（单选日期、多选日期或范围端点）。
    pub selected: bool,
    /// 该日期是否位于已提交的范围内。
    pub in_range: bool,
    /// 该日期是否位于 Range 悬停预览区间内。
    pub in_preview_range: bool,
    /// 该日期是否为 `today`。
    pub today: bool,
    /// 该日期是否为禁选黑名单日期（`blackout_dates`）。
    pub blackout: bool,
    /// 该日期是否因越界（`min_date`/`max_date`）或禁选越月而被禁用。
    pub disabled: bool,
    /// 该日期当前是否处于指针悬停状态。
    pub hovered: bool,
    /// 该日期当前是否持有键盘单元格焦点。
    pub focused: bool,
}

/// 日历视图的完整实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarState {
    /// 当前浏览的年月。
    pub view: YearMonth,
    /// 当前视图层级（Month / Year / Decade）。
    pub display_mode: CalendarDisplayMode,
    /// 单选日期或 Range 区间起点。
    pub selected: Option<NaiveDate>,
    /// Range 区间终点（已提交完整区间时为 `Some`）。
    pub range_end: Option<NaiveDate>,
    /// Multiple 模式下的离散选中日期列表（升序去重）。
    pub selected_dates: Vec<NaiveDate>,
    /// 当前指针悬停的日期。
    pub hovered: Option<NaiveDate>,
    /// 当前指针悬停的完整命中目标（包含头部标题与翻页按钮）。
    pub hovered_target: Option<CalendarHitTarget>,
    /// 当前键盘聚焦的日期。
    pub focused_date: Option<NaiveDate>,
    /// 基准“今天”日期。
    pub today: NaiveDate,
    /// 选择模式。
    pub mode: CalendarSelectionMode,
    /// 每周首日索引（`0` = 周日 .. `6` = 周六，默认 `0` 对齐 WinUI 3）。
    pub first_day_of_week: u8,
    /// 允许选择与翻页的最早日期。
    pub min_date: Option<NaiveDate>,
    /// 允许选择与翻页的最晚日期。
    pub max_date: Option<NaiveDate>,
    /// 不可选择的禁选日期列表。
    pub blackout_dates: Vec<NaiveDate>,
    /// 是否以实心强调圆高亮“今天”（默认 `true`）。
    pub is_today_highlighted: bool,
    /// 是否允许点击非当前月的前后补齐日期并自动跳月（默认 `true`）。
    pub is_out_of_scope_enabled: bool,
    /// Range 模式是否允许区间跨越 `blackout_dates`（默认 `false`）。
    pub allow_blackout_in_range: bool,
}

impl Default for CalendarState {
    fn default() -> Self {
        Self::new(
            NaiveDate {
                year: 2024,
                month: 1,
                day: 15,
            },
            CalendarSelectionMode::Single,
        )
    }
}

impl CalendarState {
    /// 创建打开在 `today` 所在月份的日历状态。
    #[must_use]
    pub fn new(today: NaiveDate, mode: CalendarSelectionMode) -> Self {
        Self {
            view: YearMonth::of(today),
            display_mode: CalendarDisplayMode::Month,
            selected: None,
            range_end: None,
            selected_dates: Vec::new(),
            hovered: None,
            hovered_target: None,
            focused_date: None,
            today,
            mode,
            first_day_of_week: 0,
            min_date: None,
            max_date: None,
            blackout_dates: Vec::new(),
            is_today_highlighted: true,
            is_out_of_scope_enabled: true,
            allow_blackout_in_range: false,
        }
    }

    /// 当前浏览的月份。
    #[must_use]
    pub fn view(&self) -> YearMonth {
        self.view
    }

    /// 当前视图层级。
    #[must_use]
    pub fn display_mode(&self) -> CalendarDisplayMode {
        self.display_mode
    }

    /// 当前选择模式。
    #[must_use]
    pub fn mode(&self) -> CalendarSelectionMode {
        self.mode
    }

    /// 切换选择模式并同步清理不兼容的选区字段。
    pub fn set_mode(&mut self, mode: CalendarSelectionMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        match mode {
            CalendarSelectionMode::None => {
                self.selected = None;
                self.range_end = None;
                self.selected_dates.clear();
            }
            CalendarSelectionMode::Single => {
                if self.selected.is_none() {
                    self.selected = self.selected_dates.first().copied();
                }
                self.range_end = None;
                self.selected_dates.clear();
            }
            CalendarSelectionMode::Multiple => {
                if self.selected_dates.is_empty() {
                    if let Some(d) = self.selected {
                        self.selected_dates.push(d);
                    }
                    if let Some(e) = self.range_end {
                        if !self.selected_dates.contains(&e) {
                            self.selected_dates.push(e);
                        }
                    }
                    self.selected_dates.sort();
                }
                self.selected = None;
                self.range_end = None;
            }
            CalendarSelectionMode::Range => {
                if self.selected.is_none() {
                    self.selected = self.selected_dates.first().copied();
                }
                self.selected_dates.clear();
            }
        }
    }

    /// 设置每周首日（`0` = 周日 .. `6` = 周六）。
    pub fn set_first_day_of_week(&mut self, day: u8) {
        self.first_day_of_week = day % 7;
    }

    /// 将日历切换至 `date` 所在月份并回到月视图。
    pub fn open(&mut self, date: NaiveDate) {
        self.view = YearMonth::of(date);
        self.display_mode = CalendarDisplayMode::Month;
    }

    /// 跳转至包含 `today` 的月份。
    pub fn today(&mut self) {
        self.view = YearMonth::of(self.today);
        self.display_mode = CalendarDisplayMode::Month;
    }

    /// 判断某日期是否在 `blackout_dates` 禁选列表中。
    #[must_use]
    pub fn is_blackout(&self, date: NaiveDate) -> bool {
        self.blackout_dates.contains(&date)
    }

    /// 判断某日期是否超出 `min_date` / `max_date` 允许范围。
    #[must_use]
    pub fn is_out_of_bounds(&self, date: NaiveDate) -> bool {
        self.min_date.is_some_and(|min| date < min) || self.max_date.is_some_and(|max| date > max)
    }

    /// 判断某日期在当前约束下是否允许被选中。
    #[must_use]
    pub fn is_date_selectable(&self, date: NaiveDate) -> bool {
        if self.mode == CalendarSelectionMode::None {
            return false;
        }
        if self.is_blackout(date) || self.is_out_of_bounds(date) {
            return false;
        }
        if !self.is_out_of_scope_enabled && YearMonth::of(date) != self.view {
            return false;
        }
        true
    }

    /// 判断某月是否与 `[min_date, max_date]` 存在交集。
    #[must_use]
    pub fn is_month_in_bounds(&self, year: i32, month: u8) -> bool {
        if !(1..=12).contains(&month) {
            return false;
        }
        let ym = YearMonth { year, month };
        let first = ym.first();
        let last = ym.last();
        self.min_date.is_none_or(|min| last >= min) && self.max_date.is_none_or(|max| first <= max)
    }

    /// 判断某年是否与 `[min_date, max_date]` 存在交集。
    #[must_use]
    pub fn is_year_in_bounds(&self, year: i32) -> bool {
        self.min_date.is_none_or(|min| year >= min.year)
            && self.max_date.is_none_or(|max| year <= max.year)
    }

    /// 判断两个端点之间的闭区间是否跨越禁选日期或越界日期。
    #[must_use]
    fn range_crosses_restricted(&self, a: NaiveDate, b: NaiveDate) -> bool {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if self.min_date.is_some_and(|min| lo < min) || self.max_date.is_some_and(|max| hi > max) {
            return true;
        }
        self.blackout_dates
            .iter()
            .any(|&blk| blk >= lo && blk <= hi)
    }

    /// 当前十年视图的起始年份（如 `2024 -> 2020`）。
    #[must_use]
    pub fn decade_start(&self) -> i32 {
        self.view.year.div_euclid(10) * 10
    }

    /// 十年视图 4×3 网格对应的 12 个年份（前 10 年为当前十年，后 2 年为下十年补齐）。
    #[must_use]
    pub fn decade_years(&self) -> [i32; CALENDAR_GRID_ITEMS] {
        let start = self.decade_start();
        std::array::from_fn(|i| start + i as i32)
    }

    /// 当前头部标题文本（`Month` 显示 `"January 2024"`，`Year` 显示 `"2024"`，`Decade` 显示 `"2020 - 2029"`）。
    #[must_use]
    pub fn header_title(&self) -> String {
        match self.display_mode {
            CalendarDisplayMode::Month => self.view.title(),
            CalendarDisplayMode::Year => self.view.year.to_string(),
            CalendarDisplayMode::Decade => {
                let start = self.decade_start();
                format!("{start} - {}", start + 9)
            }
        }
    }

    /// 是否允许向上/向前翻页。
    #[must_use]
    pub fn can_page_prev(&self) -> bool {
        match self.display_mode {
            CalendarDisplayMode::Month => self
                .min_date
                .is_none_or(|min| self.view.prev() >= YearMonth::of(min)),
            CalendarDisplayMode::Year => self.min_date.is_none_or(|min| self.view.year - 1 >= min.year),
            CalendarDisplayMode::Decade => self
                .min_date
                .is_none_or(|min| self.decade_start() - 1 >= min.year),
        }
    }

    /// 是否允许向下/向后翻页。
    #[must_use]
    pub fn can_page_next(&self) -> bool {
        match self.display_mode {
            CalendarDisplayMode::Month => self
                .max_date
                .is_none_or(|max| self.view.next() <= YearMonth::of(max)),
            CalendarDisplayMode::Year => self.max_date.is_none_or(|max| self.view.year + 1 <= max.year),
            CalendarDisplayMode::Decade => self
                .max_date
                .is_none_or(|max| self.decade_start() + 10 <= max.year),
        }
    }

    /// 按当前 `display_mode` 向前翻页（1 月 / 1 年 / 10 年），受 `min_date` 约束。
    pub fn page_prev(&mut self) -> bool {
        if !self.can_page_prev() {
            return false;
        }
        match self.display_mode {
            CalendarDisplayMode::Month => {
                self.view = self.view.prev();
            }
            CalendarDisplayMode::Year => {
                self.view.year -= 1;
            }
            CalendarDisplayMode::Decade => {
                self.view.year -= 10;
            }
        }
        true
    }

    /// 按当前 `display_mode` 向后翻页（1 月 / 1 年 / 10 年），受 `max_date` 约束。
    pub fn page_next(&mut self) -> bool {
        if !self.can_page_next() {
            return false;
        }
        match self.display_mode {
            CalendarDisplayMode::Month => {
                self.view = self.view.next();
            }
            CalendarDisplayMode::Year => {
                self.view.year += 1;
            }
            CalendarDisplayMode::Decade => {
                self.view.year += 10;
            }
        }
        true
    }

    /// 按整月翻页（受 `min_date` / `max_date` 限制）。
    pub fn page(&mut self, months: i32) -> bool {
        if months == 0 {
            return false;
        }
        let mut view = self.view;
        for _ in 0..months.unsigned_abs() {
            let next = if months > 0 { view.next() } else { view.prev() };
            if self.min_date.is_some_and(|min| next < YearMonth::of(min))
                || self.max_date.is_some_and(|max| next > YearMonth::of(max))
            {
                break;
            }
            view = next;
        }
        if view == self.view {
            return false;
        }
        self.view = view;
        true
    }

    /// 向上钻取视图层级：`Month -> Year -> Decade`。
    pub fn drill_up(&mut self) -> bool {
        match self.display_mode {
            CalendarDisplayMode::Month => {
                self.display_mode = CalendarDisplayMode::Year;
                true
            }
            CalendarDisplayMode::Year => {
                self.display_mode = CalendarDisplayMode::Decade;
                true
            }
            CalendarDisplayMode::Decade => false,
        }
    }

    /// 在年视图选择月份并回到月视图。
    pub fn select_month(&mut self, year: i32, month: u8) -> bool {
        if !self.is_month_in_bounds(year, month) {
            return false;
        }
        self.view = YearMonth { year, month };
        self.display_mode = CalendarDisplayMode::Month;
        true
    }

    /// 在十年视图选择年份并回到年视图。
    pub fn select_year(&mut self, year: i32) -> bool {
        if !self.is_year_in_bounds(year) {
            return false;
        }
        self.view.year = year;
        self.display_mode = CalendarDisplayMode::Year;
        true
    }

    /// 返回已提交的有序选区（`Single` / `Range`）。
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

    /// 返回 Range 模式下尚未提交终点时的悬停预览有序区间。
    #[must_use]
    pub fn preview_range(&self) -> Option<(NaiveDate, NaiveDate)> {
        if self.mode != CalendarSelectionMode::Range {
            return None;
        }
        let (Some(start), None, Some(hovered)) = (self.selected, self.range_end, self.hovered)
        else {
            return None;
        };
        if self.is_blackout(hovered) || self.is_out_of_bounds(hovered) {
            return None;
        }
        if !self.allow_blackout_in_range && self.range_crosses_restricted(start, hovered) {
            return None;
        }
        Some(if start <= hovered {
            (start, hovered)
        } else {
            (hovered, start)
        })
    }

    /// 判断 `date` 是否落在 Range 悬停预览区间内。
    #[must_use]
    pub fn is_in_preview_range(&self, date: NaiveDate) -> bool {
        if self.is_blackout(date) || self.is_out_of_bounds(date) {
            return false;
        }
        self.preview_range()
            .is_some_and(|(lo, hi)| date >= lo && date <= hi)
    }

    /// 当前悬停的日期。
    #[must_use]
    pub fn hovered(&self) -> Option<NaiveDate> {
        self.hovered
    }

    /// 设置悬停日期。
    pub fn hover(&mut self, date: Option<NaiveDate>) {
        self.hovered = date;
        self.hovered_target = date.map(CalendarHitTarget::DayCell);
    }

    /// 选择某一天；若状态发生变化则返回 `true`。
    ///
    /// - `None`：禁止选择，返回 `false`；
    /// - `Single`：选择日期，再次点击当前已选日期反选；
    /// - `Multiple`：增删离散日期；
    /// - `Range`：首击定起点，次击归一化区间；已有完整区间再点重置起点；
    ///   `allow_blackout_in_range == false` 时点击跨禁区终点重置为新起点。
    pub fn select(&mut self, date: NaiveDate) -> bool {
        if !self.is_date_selectable(date) {
            return false;
        }
        match self.mode {
            CalendarSelectionMode::None => return false,
            CalendarSelectionMode::Single => {
                if self.selected == Some(date) {
                    self.selected = None;
                } else {
                    self.selected = Some(date);
                }
                self.range_end = None;
            }
            CalendarSelectionMode::Multiple => {
                if let Some(pos) = self.selected_dates.iter().position(|&d| d == date) {
                    self.selected_dates.remove(pos);
                } else {
                    self.selected_dates.push(date);
                    self.selected_dates.sort();
                    self.selected_dates.dedup();
                }
                self.selected = None;
                self.range_end = None;
            }
            CalendarSelectionMode::Range => {
                if self.selected.is_none() || self.range_end.is_some() {
                    self.selected = Some(date);
                    self.range_end = None;
                } else if let Some(start) = self.selected {
                    if !self.allow_blackout_in_range && self.range_crosses_restricted(start, date) {
                        self.selected = Some(date);
                        self.range_end = None;
                    } else {
                        let (lo, hi) = if start <= date {
                            (start, date)
                        } else {
                            (date, start)
                        };
                        self.selected = Some(lo);
                        self.range_end = Some(hi);
                    }
                }
            }
        }
        self.focused_date = Some(date);
        if YearMonth::of(date) != self.view && self.is_out_of_scope_enabled {
            self.view = YearMonth::of(date);
        }
        true
    }

    /// 判断 `date` 是否被直接选中（端点、单选或离散多选）。
    #[must_use]
    pub fn is_selected(&self, date: NaiveDate) -> bool {
        match self.mode {
            CalendarSelectionMode::None => false,
            CalendarSelectionMode::Single => self.selected == Some(date),
            CalendarSelectionMode::Multiple => self.selected_dates.contains(&date),
            CalendarSelectionMode::Range => self
                .range()
                .is_some_and(|(s, e)| date == s || date == e),
        }
    }

    /// 判断 `date` 是否落在已提交范围内（或离散多选集合内）。
    #[must_use]
    pub fn contains(&self, date: NaiveDate) -> bool {
        if self.is_blackout(date) {
            return false;
        }
        match self.mode {
            CalendarSelectionMode::None => false,
            CalendarSelectionMode::Multiple => self.selected_dates.contains(&date),
            CalendarSelectionMode::Single | CalendarSelectionMode::Range => self
                .range()
                .is_some_and(|(start, end)| date >= start && date <= end),
        }
    }

    /// 返回当前浏览月份的 42 个单元格（遵循 `first_day_of_week`）。
    #[must_use]
    pub fn cells(&self) -> Vec<CalendarCell> {
        let first = self.view.first();
        let sunday_idx = first.weekday_sunday_first();
        let lead = usize::from((sunday_idx + 7 - (self.first_day_of_week % 7)) % 7);
        let mut cells = Vec::with_capacity(CALENDAR_CELLS);
        for index in 0..CALENDAR_CELLS {
            let offset = index as i64 - lead as i64;
            let date = day_offset(first, offset);
            let in_month = date
                .is_some_and(|d| d.year == self.view.year && d.month == self.view.month);
            let blackout = date.is_some_and(|d| self.is_blackout(d));
            let out_of_bounds = date.is_some_and(|d| self.is_out_of_bounds(d));
            let disabled = out_of_bounds || (!in_month && !self.is_out_of_scope_enabled);
            let selected = date.is_some_and(|d| self.is_selected(d));
            let in_range = date.is_some_and(|d| self.contains(d));
            let in_preview_range = date.is_some_and(|d| self.is_in_preview_range(d));
            cells.push(CalendarCell {
                date,
                in_month,
                selected,
                in_range,
                in_preview_range,
                today: date == Some(self.today),
                blackout,
                disabled,
                hovered: date.is_some() && self.hovered == date,
                focused: date.is_some() && self.focused_date == date,
            });
        }
        cells
    }

    /// 对屏幕坐标 `pos` 执行精确命中测试。
    #[must_use]
    pub fn hit_test(&self, desc: &DatePickerDesc, pos: Vec2) -> Option<CalendarHitTarget> {
        if desc.disabled || !desc.rect.contains_point(pos) {
            return None;
        }
        if desc.prev_rect().contains_point(pos) {
            return Some(CalendarHitTarget::PrevButton);
        }
        if desc.next_rect().contains_point(pos) {
            return Some(CalendarHitTarget::NextButton);
        }
        if desc.header_title_rect().contains_point(pos) {
            return Some(CalendarHitTarget::HeaderTitle);
        }
        match self.display_mode {
            CalendarDisplayMode::Month => {
                let cells = self.cells();
                for (index, cell) in cells.iter().enumerate() {
                    if let Some(date) = cell.date {
                        if desc.cell_rect(index).contains_point(pos) {
                            return Some(CalendarHitTarget::DayCell(date));
                        }
                    }
                }
            }
            CalendarDisplayMode::Year => {
                for index in 0..CALENDAR_GRID_ITEMS {
                    if desc.grid_item_rect(index).contains_point(pos) {
                        return Some(CalendarHitTarget::MonthCell(
                            self.view.year,
                            (index + 1) as u8,
                        ));
                    }
                }
            }
            CalendarDisplayMode::Decade => {
                let years = self.decade_years();
                for (index, &year) in years.iter().enumerate() {
                    if desc.grid_item_rect(index).contains_point(pos) {
                        return Some(CalendarHitTarget::YearCell(year));
                    }
                }
            }
        }
        None
    }

    /// 处理指针点击事件，状态变化时返回 `true`。
    pub fn handle_click(&mut self, desc: &DatePickerDesc, pos: Vec2) -> bool {
        let Some(target) = self.hit_test(desc, pos) else {
            return false;
        };
        match target {
            CalendarHitTarget::HeaderTitle => self.drill_up(),
            CalendarHitTarget::PrevButton => self.page_prev(),
            CalendarHitTarget::NextButton => self.page_next(),
            CalendarHitTarget::DayCell(date) => self.select(date),
            CalendarHitTarget::MonthCell(year, month) => self.select_month(year, month),
            CalendarHitTarget::YearCell(year) => self.select_year(year),
        }
    }

    /// 处理指针悬停更新，悬停目标发生变化时返回 `true`。
    pub fn handle_hover(&mut self, desc: &DatePickerDesc, pos: Vec2) -> bool {
        let target = self.hit_test(desc, pos);
        let next_day = match target {
            Some(CalendarHitTarget::DayCell(date)) => Some(date),
            _ => None,
        };
        if self.hovered_target == target && self.hovered == next_day {
            return false;
        }
        self.hovered_target = target;
        self.hovered = next_day;
        true
    }

    /// 处理鼠标滚轮事件（向上滚翻上一页，向下滚翻下一页）。
    pub fn handle_wheel(&mut self, delta_y: f32) -> bool {
        if delta_y > 0.0 {
            self.page_prev()
        } else if delta_y < 0.0 {
            self.page_next()
        } else {
            false
        }
    }

    /// 处理键盘导航与选择按键，状态发生变化时返回 `true`。
    pub fn handle_key(&mut self, key: &KeyChord) -> bool {
        match self.display_mode {
            CalendarDisplayMode::Month => self.handle_month_key(key),
            CalendarDisplayMode::Year => match key.key {
                KeyName::Named("PageUp") => self.page_prev(),
                KeyName::Named("PageDown") => self.page_next(),
                KeyName::Named("ArrowLeft") | KeyName::Named("ArrowUp") => {
                    let prev = self.view.prev();
                    if self.is_month_in_bounds(prev.year, prev.month) {
                        self.view = prev;
                        true
                    } else {
                        false
                    }
                }
                KeyName::Named("ArrowRight") | KeyName::Named("ArrowDown") => {
                    let next = self.view.next();
                    if self.is_month_in_bounds(next.year, next.month) {
                        self.view = next;
                        true
                    } else {
                        false
                    }
                }
                KeyName::Named("Enter") | KeyName::Named("Space") | KeyName::Char(' ') => {
                    self.select_month(self.view.year, self.view.month)
                }
                KeyName::Named("Escape") => {
                    self.display_mode = CalendarDisplayMode::Month;
                    true
                }
                _ => false,
            },
            CalendarDisplayMode::Decade => match key.key {
                KeyName::Named("PageUp") => self.page_prev(),
                KeyName::Named("PageDown") => self.page_next(),
                KeyName::Named("ArrowLeft") | KeyName::Named("ArrowUp") => {
                    if self.is_year_in_bounds(self.view.year - 1) {
                        self.view.year -= 1;
                        true
                    } else {
                        false
                    }
                }
                KeyName::Named("ArrowRight") | KeyName::Named("ArrowDown") => {
                    if self.is_year_in_bounds(self.view.year + 1) {
                        self.view.year += 1;
                        true
                    } else {
                        false
                    }
                }
                KeyName::Named("Enter") | KeyName::Named("Space") | KeyName::Char(' ') => {
                    self.select_year(self.view.year)
                }
                KeyName::Named("Escape") => {
                    self.display_mode = CalendarDisplayMode::Month;
                    true
                }
                _ => false,
            },
        }
    }

    fn handle_month_key(&mut self, key: &KeyChord) -> bool {
        let anchor = self
            .focused_date
            .or(self.selected)
            .or_else(|| self.selected_dates.last().copied())
            .unwrap_or_else(|| {
                if YearMonth::of(self.today) == self.view {
                    self.today
                } else {
                    self.view.first()
                }
            });
        match key.key {
            KeyName::Named("ArrowLeft") => self.move_focus_by_days(anchor, -1),
            KeyName::Named("ArrowRight") => self.move_focus_by_days(anchor, 1),
            KeyName::Named("ArrowUp") => self.move_focus_by_days(anchor, -7),
            KeyName::Named("ArrowDown") => self.move_focus_by_days(anchor, 7),
            KeyName::Named("PageUp") => {
                if !self.page_prev() {
                    return false;
                }
                let day = anchor.day.min(days_in_month(self.view.year, self.view.month));
                let next = NaiveDate {
                    year: self.view.year,
                    month: self.view.month,
                    day,
                };
                self.focused_date = Some(self.clamp_date_to_bounds(next));
                true
            }
            KeyName::Named("PageDown") => {
                if !self.page_next() {
                    return false;
                }
                let day = anchor.day.min(days_in_month(self.view.year, self.view.month));
                let next = NaiveDate {
                    year: self.view.year,
                    month: self.view.month,
                    day,
                };
                self.focused_date = Some(self.clamp_date_to_bounds(next));
                true
            }
            KeyName::Named("Home") => {
                let target = self.clamp_date_to_bounds(self.view.first());
                if self.focused_date == Some(target) {
                    return false;
                }
                self.focused_date = Some(target);
                true
            }
            KeyName::Named("End") => {
                let target = self.clamp_date_to_bounds(self.view.last());
                if self.focused_date == Some(target) {
                    return false;
                }
                self.focused_date = Some(target);
                true
            }
            KeyName::Named("Enter") | KeyName::Named("Space") | KeyName::Char(' ') => {
                self.select(anchor)
            }
            _ => false,
        }
    }

    fn clamp_date_to_bounds(&self, mut date: NaiveDate) -> NaiveDate {
        if let Some(min) = self.min_date {
            if date < min {
                date = min;
            }
        }
        if let Some(max) = self.max_date {
            if date > max {
                date = max;
            }
        }
        date
    }

    fn move_focus_by_days(&mut self, anchor: NaiveDate, delta: i64) -> bool {
        let Some(candidate) = day_offset(anchor, delta) else {
            return false;
        };
        if self.is_out_of_bounds(candidate) {
            return false;
        }
        self.focused_date = Some(candidate);
        self.view = YearMonth::of(candidate);
        true
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

/// 日历视图的纯描述符（与 `CalendarViewDesc` 互为规范名别名）。
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

/// WinUI 3 `CalendarView` 描述符规范别名。
pub type CalendarViewDesc = DatePickerDesc;

impl DatePickerDesc {
    /// 使用默认度量的日历描述符。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            disabled: false,
            z: 742,
        }
    }

    /// 设置无障碍标签。
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 设置绘制深度。
    #[must_use]
    pub fn with_z(mut self, z: u16) -> Self {
        self.z = z;
        self
    }

    /// 设置禁用状态。
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 自适应头部栏高度（保证在 260px..420px 高度下严格不越界）。
    #[must_use]
    pub fn header_height(&self) -> f32 {
        (self.rect.size.y * 0.145).clamp(30.0, 48.0).min(self.rect.size.y * 0.35)
    }

    /// 自适应星期标题行高度。
    #[must_use]
    pub fn weekday_height(&self) -> f32 {
        (self.rect.size.y * 0.085).clamp(18.0, 28.0).min(self.rect.size.y * 0.18)
    }

    /// 头部与内容区之间的 1px 分隔线矩形。
    #[must_use]
    pub fn header_divider_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y + self.header_height(),
            self.rect.size.x,
            1.0,
        )
    }

    /// 右上角下一页按钮矩形（向下箭头）。
    #[must_use]
    pub fn next_rect(&self) -> Rect {
        let hh = self.header_height();
        let btn = (hh - 8.0).clamp(22.0, 34.0);
        Rect::new(
            (self.rect.max().x - 8.0 - btn).max(self.rect.origin.x),
            self.rect.origin.y + (hh - btn) * 0.5,
            btn,
            btn,
        )
    }

    /// 右上角上一页按钮矩形（向上箭头，紧邻下一页按钮左侧）。
    #[must_use]
    pub fn prev_rect(&self) -> Rect {
        let next = self.next_rect();
        let btn = next.size.x;
        Rect::new(
            (next.origin.x - 4.0 - btn).max(self.rect.origin.x),
            next.origin.y,
            btn,
            btn,
        )
    }

    /// 左上角可点击月/年/十年标题按钮矩形。
    #[must_use]
    pub fn header_title_rect(&self) -> Rect {
        let prev = self.prev_rect();
        let x = self.rect.origin.x + 8.0;
        let w = (prev.origin.x - x - 6.0).max(24.0);
        Rect::new(x, prev.origin.y, w, prev.size.y)
    }

    /// 星期标题行中指定列的矩形（`col in 0..7`）。
    #[must_use]
    pub fn weekday_cell_rect(&self, col: usize) -> Rect {
        let pad_x = 4.0_f32.min(self.rect.size.x * 0.05);
        let col_w = ((self.rect.size.x - pad_x * 2.0) / CALENDAR_COLUMNS as f32).max(1.0);
        Rect::new(
            self.rect.origin.x + pad_x + col as f32 * col_w,
            self.rect.origin.y + self.header_height() + 1.0,
            col_w,
            self.weekday_height(),
        )
    }

    /// 月视图 6×7 日期网格外包矩形。
    #[must_use]
    pub fn grid_rect(&self) -> Rect {
        let pad_x = 4.0_f32.min(self.rect.size.x * 0.05);
        let pad_bottom = 4.0_f32.min(self.rect.size.y * 0.04);
        let top = self.rect.origin.y + self.header_height() + 1.0 + self.weekday_height() + 2.0;
        let h = (self.rect.max().y - pad_bottom - top).max(6.0);
        Rect::new(
            self.rect.origin.x + pad_x,
            top,
            (self.rect.size.x - pad_x * 2.0).max(7.0),
            h,
        )
    }

    /// 指定月视图网格单元格的矩形（`index in 0..42`），严格落在外框内、零越界。
    #[must_use]
    pub fn cell_rect(&self, index: usize) -> Rect {
        let grid = self.grid_rect();
        let column = (index % CALENDAR_COLUMNS) as f32;
        let row = (index / CALENDAR_COLUMNS) as f32;
        let col_w = grid.size.x / CALENDAR_COLUMNS as f32;
        let row_h = grid.size.y / CALENDAR_ROWS as f32;
        let inset = CALENDAR_CELL_INSET.min(col_w * 0.1).min(row_h * 0.1);
        Rect::new(
            grid.origin.x + column * col_w + inset,
            grid.origin.y + row * row_h + inset,
            (col_w - inset * 2.0).max(0.0),
            (row_h - inset * 2.0).max(0.0),
        )
    }

    /// 年/十年视图 4×3 网格单元格矩形（`index in 0..12`），严格落在外框内。
    #[must_use]
    pub fn grid_item_rect(&self, index: usize) -> Rect {
        let pad_x = 8.0_f32.min(self.rect.size.x * 0.06);
        let pad_y = 8.0_f32.min(self.rect.size.y * 0.06);
        let top = self.rect.origin.y + self.header_height() + 1.0 + pad_y;
        let avail_w = (self.rect.size.x - pad_x * 2.0).max(4.0);
        let avail_h = (self.rect.max().y - pad_y - top).max(3.0);
        let col = (index % 4) as f32;
        let row = (index / 4) as f32;
        let col_w = avail_w / 4.0;
        let row_h = avail_h / 3.0;
        let inset = 4.0_f32.min(col_w * 0.1).min(row_h * 0.1);
        Rect::new(
            self.rect.origin.x + pad_x + col * col_w + inset,
            top + row * row_h + inset,
            (col_w - inset * 2.0).max(0.0),
            (row_h - inset * 2.0).max(0.0),
        )
    }
}

/// 头部标题按钮的确定性子节点键。
#[must_use]
pub const fn calendar_header_node(node: u64) -> u64 {
    node ^ 0x4341_4C5F_4844_5230
}

/// 上一页翻页按钮的确定性子节点键。
#[must_use]
pub const fn calendar_prev_node(node: u64) -> u64 {
    node ^ 0x4341_4C5F_5052_4556
}

/// 下一页翻页按钮的确定性子节点键。
#[must_use]
pub const fn calendar_next_node(node: u64) -> u64 {
    node ^ 0x4341_4C5F_4E45_5854
}

/// 日历网格单元格的确定性子节点键。
#[must_use]
pub const fn calendar_cell_node(node: u64, index: usize) -> u64 {
    node ^ 0x4341_4C5F_4345_4C00 ^ (index as u64)
}

impl UiWidget for DatePickerDesc {
    type State = CalendarState;

    fn initial_state(&self) -> Self::State {
        CalendarState::default()
    }

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(
            self.rect.size.x.max(300.0),
            self.rect.size.y.max(336.0),
        )
    }

    fn reduce(&self, state: &mut Self::State, rect: Rect, event: &WidgetInputEvent) -> bool {
        if self.disabled {
            return false;
        }
        let mut desc = self.clone();
        desc.rect = rect;
        match event {
            WidgetInputEvent::PointerDown { pos, .. } => state.handle_click(&desc, *pos),
            WidgetInputEvent::PointerDrag { pos } => state.handle_hover(&desc, *pos),
            WidgetInputEvent::Wheel { delta } => state.handle_wheel(delta.y),
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
        emit_calendar_view(node, &desc, state, control, text_ctx)
    }
}

/// 发射正式 `CalendarView` 单一真源控件（含完整文本塑形、ADR-0033 符号回退与 ADR-0037 局部裁剪）。
#[must_use]
pub fn emit_calendar_view(
    node: u64,
    desc: &DatePickerDesc,
    state: &CalendarState,
    control: ControlState,
    text: &mut TextCtx<'_>,
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let mut out = WidgetOutput::default();

    // 1. 外围表面卡片与描边（WinUI 3 CalendarView 圆角 4px）
    out.primitives
        .push(surface(rect, CALENDAR_RADIUS, Token::SURFACE, z));
    out.primitives.push(outlined(
        rect,
        CALENDAR_RADIUS,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z + 1,
    ));

    // ADR-0037：为所有内部内容注册自洽局部 AABB 裁剪节点，确保极端尺寸下零视觉溢出。
    let clip_idx = out.push_clip(GpuClipNode::from_aabb(rect.to_min_max_array()));
    let content_start = out.primitives.len();

    // 头部分隔线
    out.primitives.push(surface(
        desc.header_divider_rect(),
        0.0,
        Token::CONTROL_STROKE_SUBTLE,
        z + 1,
    ));

    // 2. 顶部左侧可点击月/年/十年标题按钮
    let title_rect = desc.header_title_rect();
    let title_hovered =
        !disabled && state.hovered_target == Some(CalendarHitTarget::HeaderTitle);
    out.primitives.push(surface(
        title_rect,
        CALENDAR_RADIUS,
        if title_hovered {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        },
        z + 1,
    ));
    let title_text_rect = Rect::new(
        title_rect.origin.x + 8.0,
        title_rect.origin.y,
        (title_rect.size.x - 12.0).max(0.0),
        title_rect.size.y,
    );
    let title_font = (desc.header_height() * 0.32).clamp(12.0, 14.0);
    let title_quads = text.shape_line_ellipsized(
        &state.header_title(),
        title_text_rect,
        title_font,
        600,
        AlignX::Left,
        AlignY::Middle,
    );
    let title_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(
        &title_quads,
        title_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 3. 右上角上下翻页按钮（ADR-0033：优先符号字体，不可用时回退 ShellIcon::ChevronUp / ChevronDown）
    let prev_rect = desc.prev_rect();
    let next_rect = desc.next_rect();
    let prev_enabled = !disabled && state.can_page_prev();
    let next_enabled = !disabled && state.can_page_next();
    let nav_buttons = [
        (
            prev_rect,
            prev_enabled,
            state.hovered_target == Some(CalendarHitTarget::PrevButton),
            CALENDAR_CHEVRON_UP_GLYPH,
            ShellIcon::ChevronUp,
        ),
        (
            next_rect,
            next_enabled,
            state.hovered_target == Some(CalendarHitTarget::NextButton),
            CALENDAR_CHEVRON_DOWN_GLYPH,
            ShellIcon::ChevronDown,
        ),
    ];
    for (btn_rect, enabled, hovered, glyph, fallback_icon) in nav_buttons {
        out.primitives.push(surface(
            btn_rect,
            CALENDAR_RADIUS,
            if enabled && hovered {
                Token::CONTROL_HOVER
            } else {
                Token::NONE
            },
            z + 1,
        ));
        let ink = if enabled {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_DISABLED
        };
        let icon_size = (btn_rect.size.x.min(btn_rect.size.y) * 0.55).clamp(12.0, 16.0);
        let icon_rect = Rect::new(
            btn_rect.center().x - icon_size * 0.5,
            btn_rect.center().y - icon_size * 0.5,
            icon_size,
            icon_size,
        );
        if let Some(quads) = text.shape_symbol_glyph(glyph, icon_rect, 12.0) {
            emit_glyphs(&quads, ink, z + 2, &mut out.primitives, &mut out.payloads);
        } else {
            emit_shell_icon_primitives(
                fallback_icon,
                icon_rect,
                ink,
                Token::SURFACE,
                z + 2,
                &mut out.primitives,
                &mut out.payloads,
            );
        }
    }

    // 4. 视图主体：Month（6×7 日格）/ Year（4×3 月格）/ Decade（4×3 年格）
    match state.display_mode {
        CalendarDisplayMode::Month => {
            emit_month_grid(node, desc, state, control, disabled, z, text, &mut out);
        }
        CalendarDisplayMode::Year => {
            emit_year_grid(node, desc, state, disabled, z, text, &mut out);
        }
        CalendarDisplayMode::Decade => {
            emit_decade_grid(node, desc, state, disabled, z, text, &mut out);
        }
    }

    // 将全部内部内容图元绑定至局部裁剪节点。
    for prim in &mut out.primitives[content_start..] {
        prim.clip_idx = clip_idx;
    }

    // 5. 外框键盘焦点环（置于裁剪之外，避免外扩 2px 被切边）
    out.primitives.push(focus_ring(
        rect.inflate(2.0),
        CALENDAR_RADIUS + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    // 6. 命中区域与无障碍语义树
    if !disabled {
        out.hit = Some(
            HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
                .with_shape(Shape::Rounded {
                    radii: [CALENDAR_RADIUS; 4],
                })
                .with_cursor(CursorKind::Pointer),
        );
        out.with_sub_control(
            calendar_header_node(node),
            HitRegion::new(
                calendar_header_node(node),
                CONTROL_LAYER,
                u32::from(z) + 1,
                title_rect,
            )
            .with_cursor(CursorKind::Pointer),
            A11yNode::new(Role::Button, state.header_title(), title_rect).clickable(),
            true,
        );
        if prev_enabled {
            out.with_sub_control(
                calendar_prev_node(node),
                HitRegion::new(
                    calendar_prev_node(node),
                    CONTROL_LAYER,
                    u32::from(z) + 1,
                    prev_rect,
                )
                .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, "Previous", prev_rect).clickable(),
                true,
            );
        }
        if next_enabled {
            out.with_sub_control(
                calendar_next_node(node),
                HitRegion::new(
                    calendar_next_node(node),
                    CONTROL_LAYER,
                    u32::from(z) + 1,
                    next_rect,
                )
                .with_cursor(CursorKind::Pointer),
                A11yNode::new(Role::Button, "Next", next_rect).clickable(),
                true,
            );
        }
    }

    let mut a11y = A11yNode::new(
        Role::Grid,
        if desc.label.is_empty() {
            state.header_title()
        } else {
            desc.label.clone()
        },
        rect,
    )
    .clickable()
    .disabled(disabled);
    a11y.value = calendar_a11y_value(state);
    out.a11y = Some(a11y);
    out.focusable = !disabled;
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_month_grid(
    node: u64,
    desc: &DatePickerDesc,
    state: &CalendarState,
    control: ControlState,
    disabled: bool,
    z: u16,
    text: &mut TextCtx<'_>,
    out: &mut WidgetOutput,
) {
    // 星期表头（遵循 first_day_of_week）
    let first_day = usize::from(state.first_day_of_week % 7);
    let wd_font = (desc.weekday_height() * 0.52).clamp(10.0, 12.0);
    let wd_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    for col in 0..CALENDAR_COLUMNS {
        let label = WEEKDAY_SHORT_NAMES[(first_day + col) % 7];
        let wd_rect = desc.weekday_cell_rect(col);
        let quads = text.shape_line(label, wd_rect, wd_font, 600, AlignX::Center, AlignY::Middle);
        emit_glyphs(&quads, wd_ink, z + 2, &mut out.primitives, &mut out.payloads);
    }

    // 6×7 日期单元格
    let cells = state.cells();
    let sample_cell = desc.cell_rect(0);
    let day_font = (sample_cell.size.y * 0.42).clamp(11.0, 14.0);
    for (index, cell) in cells.iter().enumerate() {
        let Some(date) = cell.date else {
            continue;
        };
        let cell_rect = desc.cell_rect(index);
        let cell_disabled = disabled || cell.disabled || cell.blackout;
        let pill_d = cell_rect.size.x.min(cell_rect.size.y);
        let pill_rect = Rect::new(
            cell_rect.center().x - pill_d * 0.5,
            cell_rect.center().y - pill_d * 0.5,
            pill_d,
            pill_d,
        );
        let pill_r = pill_d * 0.5;

        // 层 1：Range 区间柔色底（已提交或悬停预览，恒定槽位以遵守稳定形状契约）
        let in_band = !cell.blackout && (cell.in_range || cell.in_preview_range);
        let band_fill = if in_band && !cell.selected {
            Token::PRIMARY_SOFT
        } else {
            Token::NONE
        };
        out.primitives
            .push(surface(cell_rect, CALENDAR_RADIUS, band_fill, z + 2));

        // 层 2：圆底填充（Today 实心强调圆 / Hover 柔灰圆）
        let today_filled = state.is_today_highlighted && cell.today;
        let circle_fill = if today_filled {
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else {
                Token::PRIMARY
            }
        } else if !cell_disabled && cell.hovered {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        out.primitives
            .push(surface(pill_rect, pill_r, circle_fill, z + 3));

        // 层 3：Selected 空心强调环（Today 同时被选中时绘制内环 ON_PRIMARY）
        let ring_rect = if today_filled && cell.selected {
            pill_rect.inset(2.0)
        } else {
            pill_rect
        };
        let ring_r = ring_rect.size.x.min(ring_rect.size.y) * 0.5;
        let ring_stroke = if cell.selected {
            if disabled {
                Token::TEXT_DISABLED
            } else if today_filled {
                Token::ON_PRIMARY
            } else {
                Token::PRIMARY
            }
        } else {
            Token::NONE
        };
        out.primitives.push(outlined(
            ring_rect,
            ring_r,
            Token::NONE,
            ring_stroke,
            1.5,
            0.0,
            z + 3,
        ));

        // 层 4：单元格级键盘焦点环（恒定发射，未聚焦时透明度 0）
        out.primitives.push(focus_ring(
            pill_rect,
            pill_r,
            Token::FOCUS_RING,
            1.5,
            z + 4,
            cell.focused && control.focused && !disabled,
        ));

        // 层 5：日期文本
        let day_ink = if disabled || cell.disabled || cell.blackout {
            Token::TEXT_DISABLED
        } else if today_filled {
            Token::ON_PRIMARY
        } else if cell.selected {
            Token::PRIMARY
        } else if !cell.in_month {
            Token::TEXT_SECONDARY
        } else {
            Token::TEXT_PRIMARY
        };
        let day_str = date.day.to_string();
        let day_weight = if today_filled || cell.selected {
            600
        } else {
            400
        };
        let day_quads = text.shape_line(
            &day_str,
            cell_rect,
            day_font,
            day_weight,
            AlignX::Center,
            AlignY::Middle,
        );
        emit_glyphs(
            &day_quads,
            day_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );

        // 层 6：Blackout 斜线禁用标记（使用 curve_stroke 退化二次贝塞尔线段）
        if cell.blackout {
            let pad = (pill_d * 0.24).max(3.0);
            curve_stroke(
                [pill_rect.origin.x + pad, pill_rect.max().y - pad],
                [pill_rect.max().x - pad, pill_rect.origin.y + pad],
                1.2,
                Token::TEXT_DISABLED,
                z + 5,
                &mut out.primitives,
                &mut out.payloads,
            );
        }

        // 层 7：单元格命中区
        if !disabled {
            let cell_key = calendar_cell_node(node, index);
            out.sub_hits.push(
                HitRegion::new(cell_key, CONTROL_LAYER, u32::from(z) + 1, cell_rect).with_cursor(
                    if cell_disabled {
                        CursorKind::Default
                    } else {
                        CursorKind::Pointer
                    },
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_year_grid(
    node: u64,
    desc: &DatePickerDesc,
    state: &CalendarState,
    disabled: bool,
    z: u16,
    text: &mut TextCtx<'_>,
    out: &mut WidgetOutput,
) {
    let sample = desc.grid_item_rect(0);
    let font = (sample.size.y * 0.28).clamp(12.0, 14.0);
    for (index, &month_name) in MONTH_SHORT_NAMES.iter().enumerate() {
        let month = (index + 1) as u8;
        let item_rect = desc.grid_item_rect(index);
        let in_bounds = state.is_month_in_bounds(state.view.year, month);
        let item_disabled = disabled || !in_bounds;
        let is_current = state.is_today_highlighted
            && state.today.year == state.view.year
            && state.today.month == month;
        let is_selected = state.view.month == month;
        let is_hovered = !item_disabled
            && state.hovered_target == Some(CalendarHitTarget::MonthCell(state.view.year, month));

        let fill = if is_current {
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else {
                Token::PRIMARY
            }
        } else if is_hovered {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        out.primitives
            .push(surface(item_rect, CALENDAR_RADIUS, fill, z + 2));

        let stroke = if is_selected && !is_current {
            if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::PRIMARY
            }
        } else {
            Token::NONE
        };
        out.primitives.push(outlined(
            item_rect,
            CALENDAR_RADIUS,
            Token::NONE,
            stroke,
            1.5,
            0.0,
            z + 3,
        ));

        let ink = if item_disabled {
            Token::TEXT_DISABLED
        } else if is_current {
            Token::ON_PRIMARY
        } else if is_selected {
            Token::PRIMARY
        } else {
            Token::TEXT_PRIMARY
        };
        let quads = text.shape_line(
            month_name,
            item_rect,
            font,
            if is_current || is_selected { 600 } else { 400 },
            AlignX::Center,
            AlignY::Middle,
        );
        emit_glyphs(&quads, ink, z + 4, &mut out.primitives, &mut out.payloads);

        if !disabled {
            let cell_key = calendar_cell_node(node, index);
            out.sub_hits.push(
                HitRegion::new(cell_key, CONTROL_LAYER, u32::from(z) + 1, item_rect).with_cursor(
                    if item_disabled {
                        CursorKind::Default
                    } else {
                        CursorKind::Pointer
                    },
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_decade_grid(
    node: u64,
    desc: &DatePickerDesc,
    state: &CalendarState,
    disabled: bool,
    z: u16,
    text: &mut TextCtx<'_>,
    out: &mut WidgetOutput,
) {
    let years = state.decade_years();
    let decade_end = state.decade_start() + 9;
    let sample = desc.grid_item_rect(0);
    let font = (sample.size.y * 0.28).clamp(12.0, 14.0);
    for (index, &year) in years.iter().enumerate() {
        let item_rect = desc.grid_item_rect(index);
        let in_decade = year <= decade_end;
        let in_bounds = state.is_year_in_bounds(year);
        let item_disabled = disabled || !in_bounds;
        let is_current = state.is_today_highlighted && state.today.year == year;
        let is_selected = state.view.year == year;
        let is_hovered =
            !item_disabled && state.hovered_target == Some(CalendarHitTarget::YearCell(year));

        let fill = if is_current {
            if disabled {
                Token::CONTROL_DISABLED_BG
            } else {
                Token::PRIMARY
            }
        } else if is_hovered {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        out.primitives
            .push(surface(item_rect, CALENDAR_RADIUS, fill, z + 2));

        let stroke = if is_selected && !is_current {
            if disabled {
                Token::TEXT_DISABLED
            } else {
                Token::PRIMARY
            }
        } else {
            Token::NONE
        };
        out.primitives.push(outlined(
            item_rect,
            CALENDAR_RADIUS,
            Token::NONE,
            stroke,
            1.5,
            0.0,
            z + 3,
        ));

        let ink = if item_disabled {
            Token::TEXT_DISABLED
        } else if is_current {
            Token::ON_PRIMARY
        } else if is_selected {
            Token::PRIMARY
        } else if !in_decade {
            Token::TEXT_SECONDARY
        } else {
            Token::TEXT_PRIMARY
        };
        let year_str = year.to_string();
        let quads = text.shape_line(
            &year_str,
            item_rect,
            font,
            if is_current || is_selected { 600 } else { 400 },
            AlignX::Center,
            AlignY::Middle,
        );
        emit_glyphs(&quads, ink, z + 4, &mut out.primitives, &mut out.payloads);

        if !disabled {
            let cell_key = calendar_cell_node(node, index);
            out.sub_hits.push(
                HitRegion::new(cell_key, CONTROL_LAYER, u32::from(z) + 1, item_rect).with_cursor(
                    if item_disabled {
                        CursorKind::Default
                    } else {
                        CursorKind::Pointer
                    },
                ),
            );
        }
    }
}

fn calendar_a11y_value(state: &CalendarState) -> String {
    match state.mode {
        CalendarSelectionMode::None => String::new(),
        CalendarSelectionMode::Multiple => state
            .selected_dates
            .iter()
            .map(|d| d.to_iso())
            .collect::<Vec<_>>()
            .join(", "),
        CalendarSelectionMode::Single | CalendarSelectionMode::Range => state
            .range()
            .map(|(start, end)| {
                if start == end {
                    start.to_iso()
                } else {
                    format!("{} / {}", start.to_iso(), end.to_iso())
                }
            })
            .unwrap_or_default(),
    }
}

/// 兼容旧调用方的日历发射包装器（接受预塑形字形切片；新代码请优先使用 [`emit_calendar_view`]）。
#[must_use]
pub fn emit_calendar(
    node: u64,
    desc: &DatePickerDesc,
    state: &CalendarState,
    control: ControlState,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let z = desc.z;
    let disabled = control.disabled || desc.disabled;
    let mut out = WidgetOutput::default();

    out.primitives.push(instance(
        rect,
        [0.0; 4],
        Token::NONE,
        Token::NONE,
        0.0,
        CALENDAR_POPUP_SHADOW,
        PrimType::Shadow,
        SubElement::Shadow,
        z,
    ));
    out.primitives
        .push(surface(rect, CALENDAR_RADIUS, Token::SURFACE, z));
    out.primitives.push(outlined(
        rect,
        CALENDAR_RADIUS,
        Token::NONE,
        Token::CARD_BORDER,
        1.0,
        0.0,
        z,
    ));

    let clip_idx = out.push_clip(GpuClipNode::from_aabb(rect.to_min_max_array()));
    let content_start = out.primitives.len();

    out.primitives.push(surface(
        desc.header_divider_rect(),
        0.0,
        Token::CONTROL_STROKE_SUBTLE,
        z + 1,
    ));

    // 使用 ShellIcon::ChevronUp / ChevronDown 矢量图标替代历史水平线段。
    let nav_ink = if disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    for (btn_rect, icon) in [
        (desc.prev_rect(), ShellIcon::ChevronUp),
        (desc.next_rect(), ShellIcon::ChevronDown),
    ] {
        let icon_size = (btn_rect.size.x.min(btn_rect.size.y) * 0.55).clamp(12.0, 16.0);
        let icon_rect = Rect::new(
            btn_rect.center().x - icon_size * 0.5,
            btn_rect.center().y - icon_size * 0.5,
            icon_size,
            icon_size,
        );
        emit_shell_icon_primitives(
            icon,
            icon_rect,
            nav_ink,
            Token::SURFACE,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    for (index, cell) in state.cells().iter().enumerate() {
        let Some(_date) = cell.date else {
            continue;
        };
        let cell_rect = desc.cell_rect(index);
        let pill_d = cell_rect.size.x.min(cell_rect.size.y);
        let pill_rect = Rect::new(
            cell_rect.center().x - pill_d * 0.5,
            cell_rect.center().y - pill_d * 0.5,
            pill_d,
            pill_d,
        );
        let pill_r = pill_d * 0.5;
        let today_filled = state.is_today_highlighted && cell.today;
        let fill = if today_filled {
            Token::PRIMARY
        } else if cell.in_range || cell.in_preview_range {
            Token::PRIMARY_SOFT
        } else if cell.hovered && !disabled {
            Token::CONTROL_HOVER
        } else {
            Token::NONE
        };
        out.primitives.push(surface(
            if today_filled { pill_rect } else { cell_rect },
            if today_filled { pill_r } else { CALENDAR_RADIUS },
            fill,
            z + 2,
        ));
        if cell.selected {
            out.primitives.push(outlined(
                pill_rect,
                pill_r,
                Token::NONE,
                if today_filled {
                    Token::ON_PRIMARY
                } else {
                    Token::PRIMARY
                },
                1.5,
                0.0,
                z + 3,
            ));
        }
    }

    emit_glyphs(
        glyphs,
        if disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        },
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    for prim in &mut out.primitives[content_start..] {
        prim.clip_idx = clip_idx;
    }

    out.primitives.push(focus_ring(
        rect.inflate(2.0),
        CALENDAR_RADIUS + 2.0,
        Token::FOCUS_RING,
        2.0,
        z + 5,
        control.focused && !disabled,
    ));

    out.hit = (!disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(z), rect)
            .with_shape(Shape::Rounded {
                radii: [CALENDAR_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Grid, desc.label.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.value = calendar_a11y_value(state);
    out.a11y = Some(a11y);
    out.focusable = !disabled;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> NaiveDate {
        NaiveDate::new(year, month, day).expect("valid date")
    }

    fn key(name: &'static str) -> KeyChord {
        KeyChord::new(KeyName::Named(name))
    }

    fn with_text_ctx<R>(f: impl FnOnce(&mut TextCtx<'_>) -> R) -> R {
        let mut engine = unbound_text::TextEngine::new();
        let mut cache = unbound_text::GlyphCache::new();
        let mut text_ctx = TextCtx {
            engine: &mut engine,
            cache: &mut cache,
            atlas_clock: 1,
            raster_scale: 1.0,
        };
        f(&mut text_ctx)
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
    fn the_grid_has_42_cells_and_respects_first_day_of_week() {
        // 默认周日为首日（对齐 WinUI 3）：2024-01-01 为周一，故 cells[0] 为 2023-12-31。
        let sunday_first = CalendarState::new(date(2024, 1, 15), SelectionMode::Single);
        let cells = sunday_first.cells();
        assert_eq!(cells.len(), CALENDAR_CELLS);
        assert_eq!(cells[0].date, Some(date(2023, 12, 31)));
        assert!(!cells[0].in_month);
        assert_eq!(cells[1].date, Some(date(2024, 1, 1)));
        assert!(cells[1].in_month);
        assert!(cells.iter().any(|cell| cell.today));

        // 显式切换周一为首日：2024-01-01 落在 cells[0]。
        let mut monday_first = CalendarState::new(date(2024, 1, 15), SelectionMode::Single);
        monday_first.set_first_day_of_week(1);
        let mon_cells = monday_first.cells();
        assert_eq!(mon_cells[0].date, Some(date(2024, 1, 1)));
        assert!(mon_cells[0].in_month);
    }

    #[test]
    fn all_cells_stay_strictly_inside_outer_bounds_for_compact_and_standard_heights() {
        for height in [260.0_f32, 274.0, 300.0, 320.0] {
            let rect = Rect::new(12.0, 20.0, 280.0, height);
            let desc = DatePickerDesc::new(rect);
            assert!(desc.header_title_rect().max().y <= rect.max().y);
            assert!(desc.prev_rect().max().x <= rect.max().x);
            assert!(desc.next_rect().max().x <= rect.max().x);
            for idx in 0..CALENDAR_CELLS {
                let cell = desc.cell_rect(idx);
                assert!(
                    cell.origin.x >= rect.origin.x
                        && cell.origin.y >= rect.origin.y
                        && cell.max().x <= rect.max().x + 1e-3
                        && cell.max().y <= rect.max().y + 1e-3,
                    "day cell {idx} ({cell:?}) overflowed outer rect ({rect:?}) at height {height}"
                );
            }
            for idx in 0..CALENDAR_GRID_ITEMS {
                let item = desc.grid_item_rect(idx);
                assert!(
                    item.origin.x >= rect.origin.x
                        && item.origin.y >= rect.origin.y
                        && item.max().x <= rect.max().x + 1e-3
                        && item.max().y <= rect.max().y + 1e-3,
                    "grid item {idx} ({item:?}) overflowed outer rect ({rect:?}) at height {height}"
                );
            }
        }
    }

    #[test]
    fn selection_modes_none_single_multiple_and_range_work() {
        // None 模式禁止选中
        let mut none_state = CalendarState::new(date(2024, 3, 10), CalendarSelectionMode::None);
        assert!(!none_state.select(date(2024, 3, 12)));
        assert_eq!(none_state.range(), None);

        // Single 模式支持选日、替换与再点反选
        let mut single = CalendarState::new(date(2024, 3, 10), CalendarSelectionMode::Single);
        assert!(single.select(date(2024, 3, 4)));
        assert!(single.select(date(2024, 3, 9)));
        assert_eq!(single.range(), Some((date(2024, 3, 9), date(2024, 3, 9))));
        assert!(single.select(date(2024, 3, 9)));
        assert_eq!(single.range(), None);

        // Multiple 模式支持离散增删
        let mut multi = CalendarState::new(date(2024, 3, 10), CalendarSelectionMode::Multiple);
        assert!(multi.select(date(2024, 3, 15)));
        assert!(multi.select(date(2024, 3, 5)));
        assert_eq!(
            multi.selected_dates,
            vec![date(2024, 3, 5), date(2024, 3, 15)]
        );
        assert!(multi.select(date(2024, 3, 5)));
        assert_eq!(multi.selected_dates, vec![date(2024, 3, 15)]);
    }

    #[test]
    fn range_selection_orders_endpoints_previews_and_resets_on_third_click() {
        let mut state = CalendarState::new(date(2024, 3, 10), CalendarSelectionMode::Range);
        state.select(date(2024, 3, 20));
        state.hover(Some(date(2024, 3, 12)));
        assert!(state.is_in_preview_range(date(2024, 3, 15)));
        assert!(!state.is_in_preview_range(date(2024, 3, 10)));

        // 反向选择自动归一化
        state.select(date(2024, 3, 5));
        assert_eq!(state.range(), Some((date(2024, 3, 5), date(2024, 3, 20))));
        assert!(state.contains(date(2024, 3, 10)));
        assert!(!state.contains(date(2024, 3, 21)));
        assert!(!state.is_in_preview_range(date(2024, 3, 15)));

        // 已有完整区间后第三次点击重置为新起点
        state.select(date(2024, 3, 8));
        assert_eq!(state.range(), Some((date(2024, 3, 8), date(2024, 3, 8))));
        assert_eq!(state.range_end, None);
    }

    #[test]
    fn blackout_dates_block_range_by_default_and_allow_when_opted_in() {
        let mut state = CalendarState::new(date(2024, 3, 10), CalendarSelectionMode::Range);
        state.blackout_dates = vec![date(2024, 3, 12)];

        // 黑名单日自身不可选
        assert!(!state.select(date(2024, 3, 12)));

        // 默认 allow_blackout_in_range == false：跨越禁区点击重置为新起点
        assert!(state.select(date(2024, 3, 10)));
        state.hover(Some(date(2024, 3, 15)));
        assert!(!state.is_in_preview_range(date(2024, 3, 13)));
        assert!(state.select(date(2024, 3, 15)));
        assert_eq!(state.selected, Some(date(2024, 3, 15)));
        assert_eq!(state.range_end, None);

        // 开启 allow_blackout_in_range == true：允许跨禁区提交区间
        state.allow_blackout_in_range = true;
        assert!(state.select(date(2024, 3, 10)));
        assert_eq!(state.range(), Some((date(2024, 3, 10), date(2024, 3, 15))));
        assert!(state.contains(date(2024, 3, 11)));
        assert!(!state.contains(date(2024, 3, 12)));
    }

    #[test]
    fn three_level_drill_down_month_year_decade() {
        let mut state = CalendarState::new(date(2024, 6, 15), CalendarSelectionMode::Single);
        assert_eq!(state.display_mode, CalendarDisplayMode::Month);
        assert_eq!(state.header_title(), "June 2024");

        assert!(state.drill_up());
        assert_eq!(state.display_mode, CalendarDisplayMode::Year);
        assert_eq!(state.header_title(), "2024");

        assert!(state.drill_up());
        assert_eq!(state.display_mode, CalendarDisplayMode::Decade);
        assert_eq!(state.header_title(), "2020 - 2029");
        assert!(!state.drill_up());

        assert!(state.select_year(2026));
        assert_eq!(state.display_mode, CalendarDisplayMode::Year);
        assert_eq!(state.view.year, 2026);

        assert!(state.select_month(2026, 11));
        assert_eq!(state.display_mode, CalendarDisplayMode::Month);
        assert_eq!(
            state.view,
            YearMonth {
                year: 2026,
                month: 11
            }
        );
    }

    #[test]
    fn min_and_max_date_constrain_selection_and_paging() {
        let mut state = CalendarState::new(date(2024, 3, 15), CalendarSelectionMode::Single);
        state.min_date = Some(date(2024, 3, 5));
        state.max_date = Some(date(2024, 3, 25));

        assert!(!state.can_page_prev());
        assert!(!state.can_page_next());
        assert!(!state.page_prev());
        assert!(!state.page_next());
        assert!(!state.select(date(2024, 3, 4)));
        assert!(!state.select(date(2024, 3, 26)));
        assert!(state.select(date(2024, 3, 20)));
    }

    #[test]
    fn hit_test_and_handle_click_select_exact_day_and_drill_up() {
        let desc = DatePickerDesc::new(Rect::new(0.0, 0.0, 300.0, 320.0));
        let mut state = CalendarState::new(date(2024, 1, 15), CalendarSelectionMode::Single);
        let cells = state.cells();
        let target_idx = cells
            .iter()
            .position(|c| c.date == Some(date(2024, 1, 18)))
            .expect("jan 18 present");
        let click_pos = desc.cell_rect(target_idx).center();
        assert_eq!(
            state.hit_test(&desc, click_pos),
            Some(CalendarHitTarget::DayCell(date(2024, 1, 18)))
        );
        assert!(state.handle_click(&desc, click_pos));
        assert_eq!(state.selected, Some(date(2024, 1, 18)));

        // 点击头部标题钻取至年视图
        let header_pos = desc.header_title_rect().center();
        assert_eq!(
            state.hit_test(&desc, header_pos),
            Some(CalendarHitTarget::HeaderTitle)
        );
        assert!(state.handle_click(&desc, header_pos));
        assert_eq!(state.display_mode, CalendarDisplayMode::Year);
    }

    #[test]
    fn keyboard_navigation_crosses_months_and_selects() {
        let mut state = CalendarState::new(date(2024, 1, 31), CalendarSelectionMode::Single);
        state.focused_date = Some(date(2024, 1, 31));
        assert!(state.handle_key(&key("ArrowRight")));
        assert_eq!(state.focused_date, Some(date(2024, 2, 1)));
        assert_eq!(
            state.view,
            YearMonth {
                year: 2024,
                month: 2
            }
        );
        assert!(state.handle_key(&key("Enter")));
        assert_eq!(state.selected, Some(date(2024, 2, 1)));
        assert!(state.handle_key(&key("PageDown")));
        assert_eq!(state.view.month, 3);
        assert!(state.handle_key(&key("End")));
        assert_eq!(state.focused_date, Some(date(2024, 3, 31)));
        assert!(state.handle_key(&key("Home")));
        assert_eq!(state.focused_date, Some(date(2024, 3, 1)));
    }

    #[test]
    fn emit_calendar_view_attaches_local_clip_and_a11y() {
        let mut state = CalendarState::new(date(2024, 3, 10), CalendarSelectionMode::Range);
        state.select(date(2024, 3, 4));
        state.select(date(2024, 3, 6));
        let desc = DatePickerDesc::new(Rect::new(0.0, 0.0, 300.0, 274.0));
        let output = with_text_ctx(|text_ctx| {
            emit_calendar_view(21, &desc, &state, ControlState::default(), text_ctx)
        });
        assert!(
            !output.clips.is_empty(),
            "CalendarView must emit local clip node per ADR-0037"
        );
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Grid);
        assert_eq!(a11y.value, "2024-03-04 / 2024-03-06");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
        assert!(
            output.sub_hits.len() >= CALENDAR_CELLS + 3,
            "must emit header, nav and all 42 day cell hit regions"
        );
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
        assert!(!output.clips.is_empty());
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Grid);
        assert_eq!(a11y.value, "2024-03-04 / 2024-03-06");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Pointer))
        );
    }
}

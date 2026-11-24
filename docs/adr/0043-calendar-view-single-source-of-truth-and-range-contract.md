# ADR-0043: `CalendarView` 单一真源、范围选择扩展与 WinUI 3 Parity 契约

Status: accepted

## 背景

此前仓库中存在两套割裂的日历实现：真实控件 `date_picker.rs::emit_calendar` 仅具备基础单月渲染，存在固定 6 行最小高度向下溢出圆角外框、水平短线叠加 ASCII `<`/`>` 畸形图标、整框单 `HitRegion` 与展厅 `% 5` 轮询选日等缺陷；而 WinUI 3 像素差分（`apps/showcase/src/parity.rs`）实际测试的是 `slot_fixture.rs::emit_canonical_calendar_view_fixture` 静态假人（且外挂手工裁剪遮丑），导致真实控件缺陷完全绕过了视觉门禁。

## 决策

1. **单一真源（Single Source of Truth）**：将 `CalendarView` 升级为实现 `UiWidget` 的正式控件（保留 `DatePickerDesc` 兼容别名避免无谓破坏，并在领域层明确区分 `CalendarView`、`CalendarDatePicker` 与三列滚轮式 `DatePicker`），**彻底删除 `slot_fixture.rs` 中的 `CalendarView` 静态假人**，令应用、展厅、`CalendarDatePicker` 展开浮层、`DateRangePicker` 及 `parity.rs` 全部直连同一真实 `CalendarView`。
2. **WinUI 3 完整能力 + `Range` 选择扩展**：
   - `CalendarSelectionMode`：`None`、`Single`、`Multiple`（离散多选 `selected_dates`）及 Unbound 扩展 `Range`（起止区间连续选择 + 悬停实时预览 + 反向选择自动归一化）。
   - `Range` 边界契约：已有完整区间 `[start, end]` 后点击新日期重置为新 `start`；默认禁止区间跨越 `blackout_dates` 或 `min_date`/`max_date` 禁区（点击跨禁区终点重置为新 `start`），并提供 `allow_blackout_in_range: bool` 可选开关兼容放行并过滤场景。
   - `CalendarDisplayMode`：`Month`（6×7 日格）↔ `Year`（4×3 月格）↔ `Decade`（4×3 十年格）三级钻取；支持 `min_date` / `max_date`、`blackout_dates`、`first_day_of_week`（默认周日以对齐 WinUI 3）、`is_today_highlighted`（实心强调圆 vs 选中空心强调环）、`is_out_of_scope_enabled`（前后月淡化 + 点击跨月跳转）、鼠标滚轮翻月与完整键盘导航。
   - 视觉与图标遵循 ADR-0033（头部左侧可点击月年标题 + 右上角 `Segoe Fluent Icons` / `ShellIcon::ChevronUp`/`ChevronDown` 上下翻页键）与 ADR-0042（`WidgetOutput::clips` 自洽裁剪且按容器高度自适应，零越界）。
3. **WinUI 3 Parity 多状态覆盖**：在 `tools/winui-reference/Catalog.cs` 与 `apps/showcase/src/parity.rs` 中接入真实 `CalendarView` 并扩充多状态参考采集与像素差分（`normal` / `disabled` / `focused` / `today_highlighted` / `multiple_selected` / `blackout` / `year_mode` / `decade_mode`），同时将 `CalendarView` 移出 `stable_shape.rs` 的 `INERT_BY_DESIGN`。

## Considered Options

- **继续保留 `slot_fixture` 静态假人，仅修补展厅侧 `date_picker.rs`**：无法通过 Parity 门禁守住真实控件的几何、字形与状态回归。
- **严格限制 `CalendarView` 仅支持 WinUI 3 原生 `None`/`Single`/`Multiple` 而不内置 `Range`**：会导致 `DateRangePicker` 被迫维护第二套日历网格渲染与交互逻辑，违背单一真源原则。

## Consequences

- 任何对 `CalendarView` 视觉、图标回退或布局的改动都会直接受 WinUI 3 像素差分与无头交互测试保护。
- `CalendarDatePicker` 与 `DateRangePicker` 直接复用同一 `CalendarView` 状态机与发射器，大幅减少重复代码。

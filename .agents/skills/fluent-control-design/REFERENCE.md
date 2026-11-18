# WinUI 3 Fluent 设计规律权威数值与映射参考

本文件收录从 `microsoft-ui-xaml` 主题资源与本仓库像素差分闭环中验证通过的设计规律，供校准基准控件与设计 Unbound 自有独有控件时直接查表使用。

---

## 1. 几何与尺寸常数阶梯（Geometry & Sizing）

| 维度 | 规范数值（逻辑像素 DIP） | 适用控件与场景 |
| --- | --- | --- |
| **标准交互控件高度** | `32.0 px` | `Button`、`ComboBox`、`TextBox`、`NumberBox`、`AutoSuggestBox`、`SplitButton`、`DatePicker`、`TimePicker`、`PinInput`、`HotkeyInput` |
| **紧凑控件高度** | `24.0 px` ~ `28.0 px` | `InfoBadge`、`Chip`、`BreadcrumbBar` 项、`PropertyGrid` 内嵌行、`StatusBar`（`28px`） |
| **列表/菜单标准行高** | `36.0 px`（菜单项 `32.0 px`） | `ListView`、`NavigationView` 条目、`MenuFlyout` 项、`CommandPalette` 候选行 |
| **容器与输入内边距** | 左右 `11.0 px` ~ `12.0 px`，上 `5.0 px`，下 `6.0 px` | `Button`、`ComboBox`、`TextBox`（上下不对称 `5/6` 用于抵消 Segoe UI / 思源黑体视觉重心偏下） |
| **图标与文字间距** | `8.0 px` | 按钮前置图标、菜单图标、导航图标与文本标签间距 |
| **控件级圆角（ControlCornerRadius）** | `4.0 px` | 所有内嵌交互控件（按钮、输入框、复选框 `4px`、工具栏按钮、代码行高亮、键帽） |
| **容器/浮层圆角（OverlayCornerRadius）** | `8.0 px` | `ContentDialog`、`Flyout`、`MenuFlyout`、`TeachingTip`、`Card`、`NodeGraph` 节点、`CommandPalette` |
| **全圆胶囊圆角（Pill Radius）** | `height * 0.5` | `ToggleSwitch` 轨道（`20px` 高 → `10px` 圆角）、`Slider` 轨道与拇指、`PersonPicture` |

---

## 2. 表面层级与薄纱透明度规律（Surface & Veil Hierarchy）

Win11 Fluent 不使用多档死板的实色灰，而是通过**在后景（云母/亚克力/基准面）上叠加半透明薄纱**构建视觉深度。由於 GPU 颜色附件在**线性空间**混合而 WinUI 在 **sRGB 空间**混合，所有半透明白/黑薄纱必须经 `unbound_theme::win11::veil()` 或 `light_base_veil()` 反解：

| 视觉角色 | WinUI 3 主题资源名 | Unbound `Token` | 浅色模式表现（基准 `#F3F3F3`） | 深色模式表现（基准 `#202020`） |
| --- | --- | --- | --- | --- |
| **窗口基准面** | `ApplicationPageBackgroundThemeBrush` | `Token::SURFACE` | `#F3F3F3`（不透明清屏底） | `#202020`（不透明清屏底） |
| **卡片/面板容器** | `CardBackgroundFillColorDefaultBrush` | `Token::CARD_BG` | 白色 70% 薄纱（高于基准面） | 白色 ~5% 薄纱（`#2A2A2A` 等效） |
| **次级凹陷区/槽位** | `LayerOnMicaBaseAltFillColorSecondary` | `Token::SURFACE_SUNKEN` | 黑色 3.5% 压暗薄纱 | `#1A1A1A` 深凹表面 |
| **标准控件静止填充** | `ControlFillColorDefaultBrush` | `Token::CONTROL_FILL` | 白色 70%（`#FFFFFFB3`） | 白色 6.05% 薄纱（高于卡片一档） |
| **标准控件悬停填充** | `ControlFillColorSecondaryBrush` | `Token::CONTROL_HOVER` / `Token::HOVER` | 白色 50% / 微压暗 | 白色 8.37% 薄纱（提亮一档） |
| **标准控件按下填充** | `ControlFillColorTertiaryBrush` | `Token::PRESSED` | 白色 30%（明显压暗） | 白色 3.26% 薄纱（回落压暗） |
| **标准控件禁用填充** | `ControlFillColorDisabledBrush` | `Token::CONTROL_DISABLED_BG` | 白色 30% | 白色 4.19% 薄纱 |
| **关闭态轨道/未勾选框** | `ControlAltFillColorSecondaryBrush` | `Token::CONTROL_ALT_FILL_SECONDARY` | 黑色 2.4% 薄纱（`#EDEDED`） | 黑色/暗部微压暗（`#1D1D1D`） |
| **强调色主填充** | `AccentFillColorDefaultBrush` | `Token::PRIMARY` | `#005FB8`（前景色 `ON_PRIMARY` 为白） | `#60CDFF`（前景色 `ON_PRIMARY` 为深黑 `#001420`） |
| **强调色悬停/按下** | `AccentFillColorSecondary / Tertiary` | `Token::PRIMARY_HOVER / PRESSED` | 90% / 80% 强调色混叠 | `#82D8FF` / `#4CC2FF` |

---

## 3. 描边、高光与指示器规律（Strokes & Indicators）

1. **普通控件外描边（`ControlStrokeColorDefault` → `Token::CONTROL_STROKE`）**：
   - 线宽固定为 `1.0 px`。
   - 浅色下为 `5.78%` 黑色薄纱（把白色控件从浅灰底上勾勒出来），深色下为 `6.98%` 白色薄纱。
2. **强对比结构描边（`ControlStrongStrokeColorDefault` → `Token::CONTROL_TRACK_STROKE`）**：
   - 用于 `CheckBox` 未选中框、`RadioButton` 外圈、`ToggleSwitch` 关闭态胶囊轨道。
   - 线宽 `1.0 px`，浅色为 `44.58%` 黑，深色为 `54.42%` 白；禁用时切换为 `Token::CONTROL_STRONG_STROKE_DISABLED`。
3. **文本输入类控件的“底边聚焦强调线”规律**：
   - `TextBox`、`NumberBox`、`AutoSuggestBox`、`ComboBox (Editable)` 以及自有的 `PinInput`、`HotkeyInput`、`AddressBar (Editing)`：
   - **未聚焦（Rest/Hover）**：四周 `1px` `CONTROL_STROKE`，底边叠加 `1px` 较深的 `OUTLINE_STRONG`。
   - **聚焦态（Focused）**：背景切换为实白/深黑 `INPUT_BG`，底部绘制 **`2.0 px` 高的 `Token::PRIMARY` 强调色底边**（随圆角内缩），不绘制外扩虚线框。
4. **导航与列表项的“左侧选中胶囊条（Selection Pill）”规律**：
   - `NavigationView`（`Sidebar` / `NavigationRail`）、`ListView`、`SelectorBar` 以及自有的 `Dock` 侧栏、`PropertyGrid` 激活行、`TimelineTrack` 选中轨：
   - 选中项左侧垂直居中绘制 **`3.0 px` 宽、`16.0 px` 高、圆角 `1.5 px` 的 `Token::PRIMARY` 胶囊条**；按下瞬间高度可微缩至 `10.0 px` 以提供弹性反馈。
5. **键盘焦点环（Focus Visual）**：
   - 非文本输入类控件通过 Tab 键聚焦时，在控件边界外扩 `3.0 px` 处绘制 `2.0 px` 宽、圆角 `radius + 2.0` 的 `Token::FOCUS_RING` 外框。

---

## 4. 交互状态机与动效规律（Behavior & Motion）

1. **四态单源派生**：
   - 控件自身的状态结构体只存业务值（如 `checked: bool`、`value: f32`、`selected: usize`）。
   - 视觉瞬态（`Idle` / `Hovered` / `Pressed`）与 `focused`、`disabled` 一律由 `ControlState` 传入，禁止在组件状态内维护独立的 `is_hovered` 字段。
2. **微动效物理特征**：
   - **旋钮/滑块拇指缩放**：`ToggleSwitch` 旋钮静止直径 `12px`，悬停放大至 `14px`，按下沿运动方向拉长为 `17×14px` 胶囊；`Slider` 拇指外环 `20px`，内芯静止 `12px`、悬停 `14px`、按下收缩至 `10px`。
   - **自有组件迁移法则**：`CurveEditor` 的控制点、`Gizmo` 的拖拽手柄、`NodeGraph` 的端口引脚（Pin）以及 `TimelineTrack` 的关键帧菱形，在 `Idle -> Hovered -> Pressed` 时同样遵循 **“静止基准 → 悬停外扩 `+2px` → 按下内缩 `-1px` 或加深填充”** 的触觉反馈规律。

---

## 5. 自有独有控件合规巡检映射表

在开发或重构以下自有独有控件时，必须对照左侧的 WinUI 3 基准原型复用 Token 与几何常数：

| 自有独有控件 | 对标的 WinUI 3 基准原型 | 必须复用的视觉与行为要素 |
| --- | --- | --- |
| `NodeGraph` / `PipelineView` | `Card` + `Button` + `CheckBox` | 节点背景 `CARD_BG` + `8px` 圆角；选中节点外描边 `PRIMARY`；端口悬停放大反馈同 `Slider` 拇指 |
| `CurveEditor` / `GradientEditor` | `Slider` + `Canvas` | 槽位底色 `SURFACE_SUNKEN`；网格线 `GRID`；控制柄与色标滑块完全复用 `Slider` 拇指的三态内外环缩放规律 |
| `TimelineTrack` / `Timeline` | `ListView` + `Slider` | 轨道行交替/悬停薄纱同 `ListView`；播放头（Playhead）使用 `PRIMARY` 竖线 + 顶部 `4px` 圆角五边形头 |
| `CommandPalette` | `Flyout` + `AutoSuggestBox` + `ListView` | 外框 `8px` 圆角 + `OVERLAY_BG` + 解析阴影；顶部搜索栏同 `AutoSuggestBox`；结果行高亮同 `ListView` |
| `CodeBlock` / `CodeEditor` / `TerminalView` | `ScrollView` + `TextBox` | 容器 `CODE_BG` + `CARD_BORDER` + `8px` 圆角；右上角复制按钮同 `Button (Subtle)`；行号槽使用 `TEXT_SECONDARY` |
| `PinInput` / `HotkeyInput` / `AddressBar` | `TextBox` + `BreadcrumbBar` | 高度严格 `32px`；圆角 `4px`；激活态底部绘制 `2px` `PRIMARY` 强调线 |
| `PropertyGrid` / `SchemaForm` / `QueryBuilder` | `Expander` + `TextBox` + `ComboBox` | 分组折叠头严格对齐 `Expander`；字段行高 `32px~36px`；内嵌输入控件与独立 `TextBox`/`ComboBox` 零差异 |
| `Dock` / `MultiSplit` | `TabView` + `GridSplitter` | 标签栏激活态圆角与关闭按钮同 `TabView`；分割条悬停显示 `PRIMARY` 指示条；拖拽落点预览框使用 `PRIMARY_SOFT` 填充 + `PRIMARY` 描边 |

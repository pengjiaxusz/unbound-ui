# WinUI 3 控件对齐、缺失与自有独有控件全量清单

本文档是 `unbound-ui` 控件体系对标 Windows 11 Fluent（WinUI 3 / Windows App SDK 1.8 + Windows Community Toolkit）的**单一事实源清单**。
所有命名均**以 WinUI 3 官方规范名为主键**，服务于三项工程目标：

1. **命名对齐与像素差分闭环**：明确本框架现有 **134 个目录组件**（121 个源码模块 + `title_bar` 等外壳模块）中，哪些是 WinUI 3 的**基准对齐控件**、它们的 WinUI 3 官方规范名是什么、命名是否已与规范名一致。
2. **缺失控件追踪**：列出 WinUI 3 官方画廊（WinUI 3 Gallery）与 Community Toolkit 中尚未实现的控件，区分**核心必补（P0/P1）**与**社区扩展选补（P2）**。
3. **自有独有控件清单**：界定 WinUI 3 不存在、属于 `unbound-ui` 面向专业工作台/DCC/IDE 场景的**自有独有控件**，后续通过 `.agents/skills/fluent-control-design` 技能将基准控件总结出的外观与行为规律统一迁移到这些独有组件上。

---

## 一、总览统计

| 分类 | 数量 | 说明 |
| --- | --- | --- |
| **1. 已实现 · WinUI 3 基准对齐控件** | **71 个** | 已在 `unbound-widgets` 中实现，目录枚举、展台展示名与展示文档均统一使用 WinUI 3 官方名称，可直接纳入批量像素差分 |
| **2. 已实现 · Community Toolkit 社区标准对齐控件** | **20 个** | 对应 `CommunityToolkit.WinUI.Controls` 中的高频官方扩展组件（如 `Segmented`、`SettingsCard`、`TokenizingTextBox` 等） |
| **3. 已实现 · Unbound 自有独有控件** | **43 个** | WinUI 3 无对应物，属于本框架面向节点图、时间轴、代码/终端、数据工作台等场景的自有独有组件 |
| **4. 待实现 · WinUI 3 缺失必补控件（P1）** | **0 个**（11 项已全部落地） | P0 的 7 个缺失控件与 P1 的 11 项均已落地为独立目录变体；`PipsPager` 变体以 `PaginationStyle::Dots` 纯圆点样式承载 |
| **5. 待实现 · Community Toolkit 选补控件（P2）** | **0 个**（6 项已全部落地） | Windows Community Toolkit 的 6 个选补控件均已实现 |

> 汇总：71 + 20 + 43 = **134 个目录变体**，与 `WidgetCatalogId::COUNT` 及稳定形状探针一致。

---

## 二、已实现的 WinUI 3 基准对齐控件映射表（以 WinUI 3 规范名为主）

> **命名对齐规则**：
> - 表中第一列 **WinUI 3 规范名称** 为主名称（检测系统 `Catalog` 与报告统一使用此名称）。
> - **命名状态** 标注为 `已一致` 或 `已对应`（`Containers` 等非直接官方名变体保留）。
> - **差分接入状态** 标注为 `已接入批量差分`（参考宿主与差分器代码已就绪，实机采集需要 GPU/.NET 环境）或 `待接入批量差分`。当前首批 6 个试点 + 两批 40 个 + 后续补齐 11 个共 57 个已接入。

| WinUI 3 规范名称 | WinUI 3 分类 | 本框架当前 `WidgetCatalogId` | 源码模块 (`crates/unbound-widgets/src/`) | 命名状态 | 差分接入状态 |
| --- | --- | --- | --- | --- | --- |
| **Button** | Basic Input | `Button` | `button.rs` | `已一致`（含 `Standard` / `AccentButtonStyle` / `Subtle` 三种变体） | 已接入批量差分（实机采集待 GPU） |
| **ToggleButton** | Basic Input | `ToggleButton` | `toggle_button.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **SplitButton** | Basic Input | `SplitButton` | `split_button.rs` | `已一致`（另需对齐 `ToggleSplitButton` 变体） | 已接入批量差分（实机采集待 GPU） |
| **CheckBox** | Basic Input | `CheckBox` | `checkbox.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **RadioButton** | Basic Input | `RadioButton` | `radio.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ToggleSwitch** | Basic Input | `ToggleSwitch` | `switch.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ComboBox** | Basic Input | `ComboBox` / `ComboBoxEditable` | `select.rs` / `combobox.rs` | `已一致`：不可编辑下拉为 `ComboBox`，可编辑下拉为 `ComboBoxEditable`（`IsEditable=true`） | 已接入批量差分（`ComboBox`） |
| **Slider** | Basic Input | `Slider` | `slider.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **RatingControl** | Basic Input | `RatingControl` | `rating.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ColorPicker** | Basic Input | `ColorPicker` | `color_picker.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **TextBox** | Text | `TextBox` / `TextBoxMultiline` | `text_input.rs` / `text_area.rs` | `已一致`：多行形态为 `TextBoxMultiline`（`AcceptsReturn=true`） | 已接入批量差分（`TextBox`） |
| **NumberBox** | Text | `NumberBox` | `number_input.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **AutoSuggestBox** | Text | `AutoSuggestBox` | `autocomplete.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **RichTextBlock** | Text | `RichTextBlock` | `rich_text.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ProgressBar** | Status & Info | `ProgressBar` | `progress.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ProgressRing** | Status & Info | `ProgressRing` / `LoadingOverlay` | `spinner.rs` | `已一致`（`LoadingOverlay` 为带遮罩封装） | 已接入批量差分（实机采集待 GPU） |
| **InfoBar** | Status & Info | `InfoBar` / `Banner` / `Callout` | `alert.rs` / `banner.rs` / `callout.rs` | `已一致`（`Banner` 为顶栏通栏变体，`Callout` 为内嵌提示块） | 已接入批量差分（实机采集待 GPU） |
| **InfoBadge** | Status & Info | `InfoBadge` | `badge.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ToolTip** | Status & Info | `ToolTip` | `tooltip.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **TeachingTip** | Status & Info | `Toast` / `Tour` / `Popconfirm` | `toast.rs` / `tour.rs` / `popconfirm.rs` | `已一致`（`Tour` 为多步 `TeachingTip` 序列，`Toast` 兼具应用内通知职责） | 已接入批量差分（实机采集待 GPU） |
| **ContentDialog** | Dialogs & Flyouts | `ContentDialog` / `AlertDialog` | `modal.rs` / `alert_dialog.rs` | `已一致`（标准模态框 vs 确认/破坏性双按钮配置） | 已接入批量差分（实机采集待 GPU） |
| **Flyout** | Dialogs & Flyouts | `Flyout` / `HoverCard` | `popover.rs` / `hover_card.rs` | `已一致`（`HoverCard` 为悬停触发的 `Flyout`） | 已接入批量差分（实机采集待 GPU） |
| **MenuFlyout** | Menus & Toolbars | `MenuFlyout` / `ContextMenu` | `menu.rs` / `context_menu.rs` | `已一致`（`ContextMenu` 为右键触发的 `MenuFlyout`） | 已接入批量差分（实机采集待 GPU） |
| **MenuBar** | Menus & Toolbars | `MenuBar` | `menubar.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **CommandBar** | Menus & Toolbars | `CommandBar` | `toolbar.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **NavigationView** | Navigation | `NavigationView` / `NavigationViewCompact` | `sidebar.rs` / `navigation_rail.rs` | `已一致`（展开态 Left 与紧凑态 LeftCompact 的显示模式） | 已接入批量差分（实机采集待 GPU） |
| **TabView** | Navigation | `TabView` | `tabs.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **Pivot** | Navigation | `Pivot` | `pivot.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **BreadcrumbBar** | Navigation | `BreadcrumbBar` | `breadcrumb.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **SelectorBar** | Navigation | `SelectorBar` | `segmented.rs` | `已一致`：WinUI 3 1.4+ 原生选择条（亦对应 Toolkit `Segmented`） | 已接入批量差分（实机采集待 GPU） |
| **PipsPager** | Navigation | `PipsPager` | `pagination.rs` | `已一致`：WinUI 原生分页指示器；数字条形分页保留为桌面扩展样式 | 已接入批量差分（实机采集待 GPU） |
| **Expander** | Layout & Containers | `Expander` / `ExpanderGroup` | `collapsible.rs` / `accordion.rs` | `已一致`（`ExpanderGroup` 为多个 `Expander` 的编组） | 已接入批量差分（实机采集待 GPU） |
| **SplitView** | Layout & Containers | `SplitView` | `drawer.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **Border / Card** | Layout & Containers | `Containers` | `containers.rs` | `已对应`：对应 WinUI `Border` 与卡片表面样式 | 已接入批量差分（实机采集待 GPU） |
| **StackPanel** | Layout & Containers | `StackPanel` | `containers.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ScrollView** | Scrolling | `ScrollView` | `scroll.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **TreeView** | Collections | `TreeView` | `tree_view.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **ListView** | Collections | `ListView` | `virtual_views.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **FlipView** | Collections | `FlipView` | `carousel.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **CalendarView / CalendarDatePicker** | Date & Time | `CalendarView` / `CalendarDatePicker` | `date_picker.rs` / `calendar_date_picker.rs` | `已一致`：`CalendarView` 渲染展开月历面板，`CalendarDatePicker` 为收起触发器 + 弹层 | 已接入批量差分（实机采集待 GPU） |
| **TimePicker** | Date & Time | `TimePicker` | `time_picker.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **PersonPicture** | Media & Visual | `PersonPicture` / `AvatarGroup` | `avatar.rs` | `已一致`（`AvatarGroup` 为多头像叠放组） | 已接入批量差分（实机采集待 GPU） |
| **Image** | Media & Visual | `Image` | `image.rs` | `已一致` | 已接入批量差分（实机采集待 GPU） |
| **FontIcon / SymbolIcon** | Media & Visual | `FontIcon` | `image.rs` | `已一致`（`SymbolIcon` 由同一图标发射器承载） | 已接入批量差分（实机采集待 GPU） |
| **TitleBar** | Windowing | （位于 `title_bar.rs`，未入 `WidgetCatalogId`） | `title_bar.rs` | `已一致`：WinAppSDK 1.5+ 官方 `TitleBar` 控件，已在 `title_bar.rs` 实现，仍建议纳入目录与差分 | 已接入批量差分（实机采集待 GPU） |
| **RichEditBox** | Text | `RichEditBox` | `rich_edit_box.rs` | `已一致`：顶部 B / I / U 格式工具条 + 细分隔线 + 可编辑文本区 | 已登记，参考截图待采集 |
| **GridView** | Collections | `GridView` | `grid_view.rs` | `已一致`：等宽网格单元，选中单元用强调色描边、悬停显示圆角 | 已登记，参考截图待采集 |
| **ListBox** | Collections | `ListBox` | `list_box.rs` | `已一致`：经典单选列表，选中行铺强调浅底并带左侧指示条 | 已登记，参考截图待采集 |
| **CommandBarFlyout** | Menus & Toolbars | `CommandBarFlyout` | `command_bar_flyout.rs` | `已一致`：迷你图标工具条 + 竖向分隔线 + 右侧更多按钮 | 已登记，参考截图待采集 |
| **AppBarButton** | Menus & Toolbars | `AppBarButton` | `app_bar.rs` | `已一致`：32px 命令栏图标按钮，悬停 / 按下铺不同表层 | 已登记，参考截图待采集 |
| **AppBarToggleButton** | Menus & Toolbars | `AppBarToggleButton` | `app_bar.rs` | `已一致`：命令栏切换按钮，选中时底部绘制强调色指示条 | 已登记，参考截图待采集 |
| **AppBarSeparator** | Menus & Toolbars | `AppBarSeparator` | `app_bar.rs` | `已一致`：命令栏分组之间的 1px 竖向分隔线，纯装饰 | 已登记，参考截图待采集 |
| **SwipeControl** | Touch & Input | `SwipeControl` | `swipe_control.rs` | `已一致`：内容行 + 左右滑动操作槽，露出方向由描述符控制 | 已登记，参考截图待采集 |
| **RefreshContainer** | Scrolling | `RefreshContainer` | `refresh_container.rs` | `已一致`：顶部下拉进度指示器 + 内容区 | 已登记，参考截图待采集 |
| **TeachingTip** | Status & Info | `TeachingTip` | `teaching_tip.rs` | `已一致`：Tail 尖角、标题、正文、主 / 次按钮与关闭按钮；`Toast` / `Tour` 保留各自职责 | 已登记，参考截图待采集 |
| **AnnotatedScrollBar** | Scrolling | `AnnotatedScrollBar` | `annotated_scroll_bar.rs` | `已一致`：滚动轨道 + 固定刻度标签 + 悬停预览标签 | 已登记，参考截图待采集 |
| **ItemsView** | Collections | `ItemsView` | `items_view.rs` | `已一致`：列表 / 网格布局切换与统一选中视觉 | 已登记，参考截图待采集 |

---

## 三、已实现的 Windows Community Toolkit 对齐控件（20 个）

这些控件不在核心 `Microsoft.UI.Xaml.Controls` 命名空间内，但在官方维护的 `CommunityToolkit.WinUI.Controls` 中有标准定义，应对齐其视觉与命名约定：

| Community Toolkit 规范名称 | 本框架当前 `WidgetCatalogId` | 源码模块 | 对齐说明 |
| --- | --- | --- | --- |
| **DataGrid** | `DataGrid` / `Table` / `GenericDataTable` | `data_grid.rs` / `table.rs` / `generic_data_table.rs` | 对应 Toolkit `DataGrid` 与 `WinUI.TableView` |
| **GridSplitter / SizerBase** | `SplitPane` / `MultiSplit` | `split_pane.rs` / `multi_split.rs` | 对应 Toolkit `GridSplitter` / `ContentSizer` |
| **TokenizingTextBox** | `TagInput` | `tag_input.rs` | 对应 Toolkit 标签分词输入框 |
| **SettingsCard / SettingsExpander** | `Form` / `Descriptions` / `SchemaForm` | `form.rs` / `descriptions.rs` / `schema_form.rs` | 对应 Win11 设置页经典的 `SettingsCard` 与键值表单行 |
| **WrapPanel** | `Wrap` | `containers.rs` | 对应 Toolkit `WrapPanel` |
| **StaggeredPanel** | `Masonry` / `VirtualMasonry` | `masonry.rs` / `virtual_masonry.rs` | 对应 Toolkit 瀑布流布局面板 |
| **Shimmer** | `Skeleton` | `skeleton.rs` | 对应 Toolkit `Shimmer` 骨架屏加载动画 |
| **ColorPalette / ColorPickerButton** | `ColorSwatch` | `color_swatch.rs` | 对应 Toolkit 色板选择器 |
| **RangeSelector** | `RangeSelector` | `range_selector.rs` | 双拇指区间滑块，已实现 |
| **RadialGauge** | `RadialGauge` | `radial_gauge.rs` | 环形刻度仪表，已实现 |
| **OrbitView** | `OrbitView` | `orbit_view.rs` | 环绕式布局视图，已实现 |
| **BladeView** | `BladeView` | `blade_view.rs` | 水平多级刀片面板，已实现 |
| **HeaderedContentControl** | `HeaderedContentControl` | `headered_content.rs` | 上方标题 + 下方内容包装器，已实现 |
| **ImageCropper** | `ImageCropper` | `image_cropper.rs` | 裁剪框 + 八向手柄，已实现 |

---

## 四、WinUI 3 缺失待补控件清单（Missing Controls）

以下控件在 WinUI 3 官方画廊（WinUI 3 Gallery）中存在，但 `unbound-widgets` 尚未单独提供或缺少对应形态：

### 4.1 核心必补控件（P0 / P1 —— 共 18 个）

> **进度**：7 个 P0 与 11 个 P1 已全部落地（P1 产出 12 个目录变体：`AppBarButton` / `AppBarToggleButton` / `AppBarSeparator` 三个独立变体，`PipsPager` 变体以 `PaginationStyle::Dots` 样式承载），并接入形状探针与 24 态图元数恒定门禁。下表保留原始缺口分析，描述列已更新为当前实现形态。

| 优先级 | WinUI 3 官方控件名 | 所属分类 | 缺失现状与实现建议 | 状态 |
| --- | --- | --- | --- | --- |
| **P0** | **HyperlinkButton** | Basic Input | 独立描述符 `HyperlinkButtonDesc`（无边框、强调色文字、悬停浅底） | 已实现 |
| **P0** | **RepeatButton** | Basic Input | 按住持续触发点击的按钮（`NumberBox` 步进与滚动条箭头的基础组件） | 已实现 |
| **P0** | **DropDownButton** | Basic Input | 右侧固定带 Chevron 下拉箭头、点击弹出 `MenuFlyout` 的按钮（区别于左主操作右下拉的 `SplitButton`） | 已实现 |
| **P0** | **ToggleSplitButton** | Basic Input | 左侧主按钮带选中切换态（Checked）的 `SplitButton` | 已实现 |
| **P0** | **PasswordBox** | Text | 带掩码字符（`●`）与“按住显示明文（RevealButton）”眼睛图标的密码输入框 | 已实现 |
| **P0** | **DatePicker（三列滚轮选择器）** | Date & Time | 目录变体 `DatePicker`；展开月历对应 `CalendarView` 语义 | 已实现 |
| **P0** | **CalendarDatePicker** | Date & Time | 带日历图标的收起态输入框触发器 + 点击弹出 `CalendarView` 浮层 | 已实现 |
| **P1** | **RichEditBox** | Text | 独立描述符 `RichEditBoxDesc`：顶部 B / I / U 格式工具条 + 可编辑文本区外观 | 已实现 |
| **P1** | **GridView** | Collections | 独立描述符 `GridViewDesc`：网格单元 + 选中描边 + 悬停圆角 | 已实现 |
| **P1** | **ListBox** | Collections | 独立描述符 `ListBoxDesc`：经典单选列表，选中行带强调色指示条 | 已实现 |
| **P1** | **FlipViewIndicator / PipsPager（纯圆点态）** | Navigation | `PaginationStyle::Dots`：纯圆点指示器，选中圆点放大、其余为小圆点 | 已实现 |
| **P1** | **CommandBarFlyout** | Menus & Toolbars | 独立描述符 `CommandBarFlyoutDesc`：迷你图标工具条 + 竖向分隔线 + 右侧更多按钮 | 已实现 |
| **P1** | **AppBarButton / AppBarToggleButton / AppBarSeparator** | Menus & Toolbars | `app_bar.rs` 三个描述符与三个发射器：命令栏按钮、带底部指示条的切换按钮、1px 分隔线 | 已实现 |
| **P1** | **SwipeControl** | Touch & Input | 独立描述符 `SwipeControlDesc`：内容行 + 左右滑动操作槽 | 已实现 |
| **P1** | **RefreshContainer / PullToRefresh** | Scrolling | 独立描述符 `RefreshContainerDesc`：下拉进度指示器 + 内容区 | 已实现 |
| **P1** | **TeachingTip（带 Tail 尖角与 Hero 图完整版）** | Status & Info | 独立描述符 `TeachingTipDesc`：Tail 尖角、标题、正文、主 / 次按钮与关闭按钮 | 已实现 |
| **P1** | **AnnotatedScrollBar** | Scrolling | 独立描述符 `AnnotatedScrollBarDesc`：滚动条 + 固定刻度标签 + 悬停预览标签 | 已实现 |
| **P1** | **ItemsView** | Collections | 独立描述符 `ItemsViewDesc`：列表 / 网格布局切换与统一选中视觉 | 已实现 |

### 4.2 Community Toolkit 选补控件（P2 —— 共 6 个）

| 优先级 | Community Toolkit 控件名 | 所属分类 | 用途说明 | 状态 |
| --- | --- | --- | --- | --- |
| **P2** | **RangeSelector** | Input | `range_selector.rs`：双拇指区间滑块，已实现 | 已实现 |
| **P2** | **RadialGauge** | Status & Info | `radial_gauge.rs`：环形刻度仪表，已实现 | 已实现 |
| **P2** | **OrbitView** | Media & Layout | `orbit_view.rs`：环绕式布局视图，已实现 | 已实现 |
| **P2** | **BladeView** | Layout | `blade_view.rs`：水平多级刀片面板，已实现 | 已实现 |
| **P2** | **HeaderedContentControl / HeaderedItemsControl** | Layout | `headered_content.rs`：上方标题 + 下方内容包装器，已实现 | 已实现 |
| **P2** | **Cropper (ImageCropper)** | Media | `image_cropper.rs`：裁剪框 + 八向手柄，已实现 | 已实现 |

---

## 五、Unbound 自有独有控件清单（Win11 不存在 · 共 43 个）

以下组件在 WinUI 3 官方库中不存在，是我们面向**高性能创作工具、IDE、节点编辑器、调试器与数据密集型工作台**打造的专属竞争力组件。
它们**不进行一对一 WinUI 参考截图比对**，而是在基准对齐控件完成像素校准后，通过 `.agents/skills/fluent-control-design` 技能统一应用 Win11 Fluent 的表面层级、微描边、圆角阶梯、状态透明度与弹簧动效规律：

| 领域分组 | 本框架 `WidgetCatalogId` | 源码模块 | 组件定位与 Fluent 规律应用要点 |
| --- | --- | --- | --- |
| **节点图与无限画布（5 个）** | `NodeGraph` | `node_graph.rs` | 节点连线编辑器；节点卡片复用 `CARD_BG` / `CARD_BORDER` 与 `8px` 圆角，选中态复用 `FOCUS_RING` |
| | `CanvasToolbar` | `canvas_toolbar.rs` | 画布浮动工具条；背景复用 `OVERLAY_BG`（亚克力/云母浮层）与 `AppBarToggleButton` 选中态规律 |
| | `Minimap` | `minimap.rs` | 无限画布鹰眼缩略图；视口框复用 `PRIMARY` 描边与 `PRIMARY_SOFT` 半透明填充 |
| | `Gizmo` | `gizmo.rs` | 2D/3D 变换操纵器手柄；尺寸气泡复用 `ToolTip` 规范 |
| | `Ruler` | `ruler.rs` | 画布顶部/左侧标尺与参考线拖拽带；复用 `SURFACE_SUNKEN` 与 `DIVIDER` 刻度 |
| **动画、曲线与时间轴（3 个）** | `CurveEditor` | `curve_editor.rs` | 贝塞尔关键帧曲线编辑器 |
| | `GradientEditor` | `curve_editor.rs` | OKLCH 渐变色标编辑器 |
| | `TimelineTrack` | `timeline_track.rs` | 多轨非线性动画时间轴与播放头 |
| **IDE、代码与终端（6 个）** | `CodeBlock` | `code_block.rs` | 只读语法高亮代码块 + 复制按钮 |
| | `CodeEditor` | `code_editor.rs` | 带行号槽与当前行高亮的代码编辑器 |
| | `TerminalView` | `terminal_view.rs` | 终端输出与命令行视图 |
| | `CommandPalette` | `command_palette.rs` | `Ctrl+K / Ctrl+P` 顶部模态命令面板（复用 `ContentDialog` / `Flyout` 阴影与 `AutoSuggestBox` 搜索头） |
| | `Dock` | `dock.rs` | IDE 多标签停靠面板与拖拽分屏预览区（标签头对齐 `TabView`） |
| | `StatusBar` | `toolbar.rs` | 窗口底部状态栏与分段指示器 |
| **专业输入与导航扩展（8 个）** | `AddressBar` | `address_bar.rs` | 文件管理器/浏览器式可点击分段 + 点击转文本编辑的地址栏 |
| | `Cascader` | `cascader.rs` | 多级联选择器 |
| | `DateRangePicker` | `date_range_picker.rs` | 双月历面板 + 快捷预设（近 7 天/30 天）的日期区间选择器 |
| | `HotkeyInput` | `hotkey_input.rs` | 快捷键组合录制输入框（键帽视觉对齐 Win11 PowerToys `ShortcutControl`） |
| | `PinInput` | `pin_input.rs` | 多格验证码/PIN 码输入框（单格视觉完全遵循 `TextBox` 底边强调线规律） |
| | `ShortcutBar` | `shortcut_bar.rs` | 底部键盘快捷键提示栏（键帽 + 标签） |
| | `Stepper` | `stepper.rs` | 多步骤向导进度条 |
| | `Transfer` | `transfer.rs` | 双栏穿梭框选择器 |
| **数据可视化与属性检视（7 个）** | `Chart` | `chart.rs` | GPU 折线/柱状图表 |
| | `PropertyGrid` | `property_grid.rs` | DCC/游戏引擎式分组属性检视器（行高与折叠头对齐 `Expander`） |
| | `PipelineView` | `pipeline_view.rs` | 渲染管线阶段耗时可视化视图 |
| | `QueryBuilder` | `query_builder.rs` | AND/OR 结构化查询条件构建器 |
| | `TaskHud` | `task_hud.rs` | 后台任务进度悬浮 HUD |
| | `TreeTable` | `tree_table.rs` | 树形多列数据表 |
| | `Timeline` | `timeline.rs` | 垂直事件流时间线 |
| **通用复合与容器辅助（14 个）** | `Chip` | `badge.rs` | 可关闭/可选中的过滤标签胶囊 |
| | `Divider` | `divider.rs` | 带居中文字的水平/垂直分隔线 |
| | `EmptyState` | `hover_card.rs` | 空状态占位卡片（标题 + 说明 + 主操作按钮） |
| | `FileDrop` | `file_drop.rs` | 拖拽上传虚线/描边感应区 |
| | `NotificationCenter` | `notification_center.rs` | 通知中心聚合面板 |
| | `Watermark` | `watermark.rs` | 防泄密平铺水印层 |
| | `AspectRatioBox` | `masonry.rs` | 固定宽高比约束容器 |
| | `NinePatch` | `image.rs` | 九宫格拉伸纹理图元组件 |
| | `Overlay` | `overlay.rs` | 锚点定位浮层基础原语展示 |
| | `AvatarGroup` | `avatar.rs` | 多头像叠放组 |
| | `Banner` / `Callout` / `Popconfirm` / `Tour` | 各对应模块 | 在保留独立复合易用 API 的同时，底层视觉令牌与 `InfoBar` / `TeachingTip` 严格对齐 |

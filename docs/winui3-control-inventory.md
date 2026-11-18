# WinUI 3 控件对齐、缺失与自有独有控件全量清单

本文档是 `unbound-ui` 控件体系对标 Windows 11 Fluent（WinUI 3 / Windows App SDK 1.8 + Windows Community Toolkit）的**单一事实源清单**。
所有命名均**以 WinUI 3 官方规范名为主键**，服务于三项工程目标：

1. **命名对齐与像素差分闭环**：明确本框架现有 **109 个目录组件**（98 个源码模块 + `title_bar` 等外壳模块）中，哪些是 WinUI 3 的**基准对齐控件**、它们的 WinUI 3 官方规范名是什么、需要改名还是归并。
2. **缺失控件追踪**：列出 WinUI 3 官方画廊（WinUI 3 Gallery）与 Community Toolkit 中尚未实现的控件，区分**核心必补（P0/P1）**与**社区扩展选补（P2）**。
3. **自有独有控件清单**：界定 WinUI 3 不存在、属于 `unbound-ui` 面向专业工作台/DCC/IDE 场景的**自有独有控件**，后续通过 `.agents/skills/fluent-control-design` 技能将基准控件总结出的外观与行为规律统一迁移到这些独有组件上。

---

## 一、总览统计

| 分类 | 数量 | 说明 |
| --- | --- | --- |
| **1. 已实现 · WinUI 3 基准对齐控件** | **52 个**（对应 WinUI 3 的 41 个标准控件/变体） | 已在 `unbound-widgets` 中实现（部分存在二合一拆分或 Web 风格命名），需统一对齐至 WinUI 3 官方名称并纳入批量像素差分 |
| **2. 已实现 · Community Toolkit 社区标准对齐控件** | **14 个** | 对应 `CommunityToolkit.WinUI.Controls` 中的高频官方扩展组件（如 `Segmented`、`SettingsCard`、`TokenizingTextBox` 等） |
| **3. 已实现 · Unbound 自有独有控件** | **43 个** | WinUI 3 无对应物，属于本框架面向节点图、时间轴、代码/终端、数据工作台等场景的自有独有组件 |
| **4. 待实现 · WinUI 3 缺失必补控件（P0 / P1）** | **18 个** | WinUI 3 官方核心控件或标准子变体中尚未单独提供或尚未完整对齐的组件 |
| **5. 待实现 · Community Toolkit 选补控件（P2）** | **6 个** | Windows Community Toolkit 中值得按需补充的桌面增强控件 |

---

## 二、已实现的 WinUI 3 基准对齐控件映射表（以 WinUI 3 规范名为主）

> **命名对齐规则**：
> - 表中第一列 **WinUI 3 规范名称** 为主名称（检测系统 `Catalog` 与报告统一使用此名称）。
> - **命名状态** 标注为 `已一致`、`需改名` 或 `需归并/拆分`。
> - **差分接入状态** 标注为 `已接入试点`（当前 6 个试点控件）或 `待接入批量差分`。

| WinUI 3 规范名称 | WinUI 3 分类 | 本框架当前 `WidgetCatalogId` | 源码模块 (`crates/unbound-widgets/src/`) | 命名状态与迁移动作 | 差分接入状态 |
| --- | --- | --- | --- | --- | --- |
| **Button** | Basic Input | `Button` | `button.rs` | `已一致`（含 `Standard` / `AccentButtonStyle` / `Subtle` 三种变体） | 已接入试点 |
| **ToggleButton** | Basic Input | `ToggleButton` | `toggle_button.rs` | `已一致` | 待接入批量差分 |
| **SplitButton** | Basic Input | `SplitButton` | `split_button.rs` | `已一致`（另需对齐 `ToggleSplitButton` 变体） | 待接入批量差分 |
| **CheckBox** | Basic Input | `Checkbox` | `checkbox.rs` | `需改名`：`Checkbox` → `CheckBox`（驼峰大写 B 对齐 WinUI） | 已接入试点 |
| **RadioButton** | Basic Input | `Radio` | `radio.rs` | `需改名`：`Radio` → `RadioButton`（容器对应 `RadioButtons`） | 待接入批量差分 |
| **ToggleSwitch** | Basic Input | `Switch` | `switch.rs` | `需改名`：`Switch` → `ToggleSwitch` | 已接入试点 |
| **ComboBox** | Basic Input | `Select` / `Combobox` | `select.rs` / `combobox.rs` | `需归并/改名`：WinUI 中不可编辑下拉为 `ComboBox`（当前 `Select`），可编辑下拉为 `ComboBox (IsEditable=true)`（当前 `Combobox`） | 已接入试点（`Select`） |
| **Slider** | Basic Input | `Slider` | `slider.rs` | `已一致` | 已接入试点 |
| **RatingControl** | Basic Input | `Rating` | `rating.rs` | `需改名`：`Rating` → `RatingControl` | 待接入批量差分 |
| **ColorPicker** | Basic Input | `ColorPicker` | `color_picker.rs` | `已一致` | 待接入批量差分 |
| **TextBox** | Text | `TextInput` / `TextArea` | `text_input.rs` / `text_area.rs` | `需归并/改名`：`TextInput` → `TextBox`；`TextArea` 对应 `TextBox (AcceptsReturn=true)` | 已接入试点（`TextInput`） |
| **NumberBox** | Text | `NumberInput` | `number_input.rs` | `需改名`：`NumberInput` → `NumberBox` | 待接入批量差分 |
| **AutoSuggestBox** | Text | `AutoComplete` | `autocomplete.rs` | `需改名`：`AutoComplete` → `AutoSuggestBox` | 待接入批量差分 |
| **RichTextBlock** | Text | `RichText` | `rich_text.rs` | `需改名`：`RichText` → `RichTextBlock` | 待接入批量差分 |
| **ProgressBar** | Status & Info | `Progress` | `progress.rs` | `需改名`：`Progress` → `ProgressBar` | 待接入批量差分 |
| **ProgressRing** | Status & Info | `Spinner` / `LoadingOverlay` | `spinner.rs` | `需改名`：`Spinner` → `ProgressRing`（`LoadingOverlay` 为带遮罩封装） | 待接入批量差分 |
| **InfoBar** | Status & Info | `Alert` / `Banner` / `Callout` | `alert.rs` / `banner.rs` / `callout.rs` | `需归并/改名`：`Alert` → `InfoBar`；`Banner` 为顶栏通栏变体，`Callout` 为内嵌提示块 | 待接入批量差分 |
| **InfoBadge** | Status & Info | `Badge` | `badge.rs` | `需改名`：`Badge` → `InfoBadge` | 待接入批量差分 |
| **ToolTip** | Status & Info | `Tooltip` | `tooltip.rs` | `需改名`：`Tooltip` → `ToolTip`（驼峰大写 T） | 待接入批量差分 |
| **TeachingTip** | Status & Info | `Toast` / `Tour` / `Popconfirm` | `toast.rs` / `tour.rs` / `popconfirm.rs` | `需归并/改名`：非模态气泡引导与轻提示在 WinUI 中为 `TeachingTip`（`Tour` 为多步 `TeachingTip` 序列，`Toast` 兼具应用内通知职责） | 待接入批量差分 |
| **ContentDialog** | Dialogs & Flyouts | `Modal` / `AlertDialog` | `modal.rs` / `alert_dialog.rs` | `需归并/改名`：`Modal` 与 `AlertDialog` 均对应 WinUI `ContentDialog`（标准模态框 vs 确认/破坏性双按钮配置） | 待接入批量差分 |
| **Flyout** | Dialogs & Flyouts | `Popover` / `HoverCard` | `popover.rs` / `hover_card.rs` | `需归并/改名`：`Popover` → `Flyout`（`HoverCard` 为悬停触发的 `Flyout`） | 待接入批量差分 |
| **MenuFlyout** | Menus & Toolbars | `Menu` / `ContextMenu` | `menu.rs` / `context_menu.rs` | `需归并/改名`：`Menu` 与 `ContextMenu` 对应 WinUI `MenuFlyout` | 待接入批量差分 |
| **MenuBar** | Menus & Toolbars | `Menubar` | `menubar.rs` | `需改名`：`Menubar` → `MenuBar`（驼峰大写 B） | 待接入批量差分 |
| **CommandBar** | Menus & Toolbars | `Toolbar` | `toolbar.rs` | `需改名`：`Toolbar` → `CommandBar` | 待接入批量差分 |
| **NavigationView** | Navigation | `Sidebar` / `NavigationRail` | `sidebar.rs` / `navigation_rail.rs` | `需归并/改名`：`Sidebar`（展开态 Left）与 `NavigationRail`（紧凑态 LeftCompact）均为 `NavigationView` 的显示模式 | 待接入批量差分 |
| **TabView** | Navigation | `Tabs` | `tabs.rs` | `需改名`：`Tabs` → `TabView` | 待接入批量差分 |
| **Pivot** | Navigation | `Pivot` | `pivot.rs` | `已一致` | 待接入批量差分 |
| **BreadcrumbBar** | Navigation | `Breadcrumb` | `breadcrumb.rs` | `需改名`：`Breadcrumb` → `BreadcrumbBar` | 待接入批量差分 |
| **SelectorBar** | Navigation | `Segmented` | `segmented.rs` | `需对齐`：WinUI 3 1.4+ 新增 `SelectorBar`（亦对应 Community Toolkit 的 `Segmented`） | 待接入批量差分 |
| **PipsPager** | Navigation | `Pagination` | `pagination.rs` | `需对齐`：WinUI 原生分页指示器为 `PipsPager`，数字条形分页保留为桌面扩展样式 | 待接入批量差分 |
| **Expander** | Layout & Containers | `Collapsible` / `Accordion` | `collapsible.rs` / `accordion.rs` | `需归并/改名`：`Collapsible` → `Expander`；`Accordion` 为多个 `Expander` 的手风琴编组 | 待接入批量差分 |
| **SplitView** | Layout & Containers | `Drawer` | `drawer.rs` | `需改名`：`Drawer` → `SplitView` | 待接入批量差分 |
| **Border / Card** | Layout & Containers | `Containers` | `containers.rs` | `已对应`：对应 WinUI `Border` 与卡片表面样式 | 待接入批量差分 |
| **StackPanel** | Layout & Containers | `Stack` | `containers.rs` | `需改名`：`Stack` → `StackPanel` | 待接入批量差分 |
| **ScrollView** | Scrolling | `Scroll` | `scroll.rs` | `需改名`：`Scroll` → `ScrollView` | 待接入批量差分 |
| **TreeView** | Collections | `TreeView` | `tree_view.rs` | `已一致` | 待接入批量差分 |
| **ListView** | Collections | `VirtualViews` | `virtual_views.rs` | `需改名`：`VirtualViews` → `ListView` | 待接入批量差分 |
| **FlipView** | Collections | `Carousel` | `carousel.rs` | `需改名`：`Carousel` → `FlipView` | 待接入批量差分 |
| **CalendarView / CalendarDatePicker** | Date & Time | `DatePicker` | `date_picker.rs` | `需细分`：我方 `DatePicker` 渲染的是展开的月历面板，对应 WinUI `CalendarView`；收起触发器 + 弹层对应 `CalendarDatePicker` | 待接入批量差分 |
| **TimePicker** | Date & Time | `TimePicker` | `time_picker.rs` | `已一致` | 待接入批量差分 |
| **PersonPicture** | Media & Visual | `Avatar` / `AvatarGroup` | `avatar.rs` | `需改名`：`Avatar` → `PersonPicture` | 待接入批量差分 |
| **Image** | Media & Visual | `Image` | `image.rs` | `已一致` | 待接入批量差分 |
| **FontIcon / SymbolIcon** | Media & Visual | `Icon` | `image.rs` | `需改名`：`Icon` → `FontIcon` / `SymbolIcon` | 待接入批量差分 |
| **TitleBar** | Windowing | （位于 `title_bar.rs`，未入 `WidgetCatalogId`） | `title_bar.rs` | `需入列`：WinAppSDK 1.5+ 官方 `TitleBar` 控件，已在 `title_bar.rs` 实现，建议纳入目录与差分 | 待接入批量差分 |

---

## 三、已实现的 Windows Community Toolkit 对齐控件（14 个）

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

---

## 四、WinUI 3 缺失待补控件清单（Missing Controls）

以下控件在 WinUI 3 官方画廊（WinUI 3 Gallery）中存在，但 `unbound-widgets` 尚未单独提供或缺少对应形态：

### 4.1 核心必补控件（P0 / P1 —— 共 18 个）

| 优先级 | WinUI 3 官方控件名 | 所属分类 | 缺失现状与实现建议 |
| --- | --- | --- | --- |
| **P0** | **HyperlinkButton** | Basic Input | 尚无独立变体；可在 `button.rs` 补充 `ButtonVariant::Hyperlink` 或独立描述符（无边框、强调色文字、悬停浅底） |
| **P0** | **RepeatButton** | Basic Input | 按住持续触发点击的按钮（`NumberBox` 步进与滚动条箭头的基础组件） |
| **P0** | **DropDownButton** | Basic Input | 右侧固定带 Chevron 下拉箭头、点击弹出 `MenuFlyout` 的按钮（区别于左主操作右下拉的 `SplitButton`） |
| **P0** | **ToggleSplitButton** | Basic Input | 左侧主按钮带选中切换态（Checked）的 `SplitButton` |
| **P0** | **PasswordBox** | Text | 带掩码字符（`●`）与“按住显示明文（RevealButton）”眼睛图标的密码输入框，可在 `text_input.rs` 扩展 |
| **P0** | **DatePicker（三列滚轮选择器）** | Date & Time | 注意区分：WinUI 的 `DatePicker` 是「月 / 日 / 年」三列下拉选择器（与我方 `TimePicker` 结构同构），而展开日历网格在 WinUI 中叫 `CalendarView` / `CalendarDatePicker` |
| **P0** | **CalendarDatePicker** | Date & Time | 带日历图标的收起态输入框触发器 + 点击弹出 `CalendarView` 浮层 |
| **P1** | **RichEditBox** | Text | 可编辑的富文本输入框（当前 `RichText` 仅只读渲染，`CodeEditor` 为代码专用） |
| **P1** | **GridView** | Collections | 网格集合视图（支持选中框、悬停圆角边框与多选勾选角标） |
| **P1** | **ListBox** | Collections | 经典单选/多选内嵌列表框 |
| **P1** | **FlipViewIndicator / PipsPager（纯圆点态）** | Navigation | 与 `FlipView` 联动的紧凑圆点指示器（当前 `Pagination` 为数字按钮条，缺少纯圆点缩放形态） |
| **P1** | **CommandBarFlyout** | Menus & Toolbars | 文本选中或右键时弹出的迷你图标工具栏 + 二级菜单浮层 |
| **P1** | **AppBarButton / AppBarToggleButton / AppBarSeparator** | Menus & Toolbars | `CommandBar` 内部专用的带图标+底部紧凑标签按钮与竖向分隔线 |
| **P1** | **SwipeControl** | Touch & Input | 列表项左右滑动露出操作按钮的容器 |
| **P1** | **RefreshContainer / PullToRefresh** | Scrolling | 下拉刷新容器与指示器 |
| **P1** | **TeachingTip（带 Tail 尖角与 Hero 图完整版）** | Status & Info | 当前 `Popover` 有箭头、`Toast` 有文本，需合流为支持 Tail 指向目标、标题、正文、操作按钮与关闭按钮的完整 `TeachingTip` |
| **P1** | **AnnotatedScrollBar** | Scrolling | WinAppSDK 1.5+ 新增的带刻度标签与预览提示的滚动条变体 |
| **P1** | **ItemsView** | Collections | WinAppSDK 1.4+ 新一代统一集合视图（支持列表/网格布局切换与统一选中视觉） |

### 4.2 Community Toolkit 选补控件（P2 —— 共 6 个）

| 优先级 | Community Toolkit 控件名 | 所属分类 | 用途说明 |
| --- | --- | --- | --- |
| **P2** | **RangeSelector** | Input | 双滑块区间选择器（最小/最大双拇指滑块，可在 `slider.rs` 扩展） |
| **P2** | **RadialGauge** | Status & Info | 环形仪表盘刻度控件 |
| **P2** | **orb / OrbitView** | Media & Layout | 环绕式布局视图 |
| **P2** | **BladeView** | Layout | 水平横向展开的多级刀片面板（Azure 门户风格） |
| **P2** | **HeaderedContentControl / HeaderedItemsControl** | Layout | 统一的「上方标题 + 下方内容」包装器（可统一 `ToggleSwitch`、`Slider`、`TextBox` 的 `Header` 排版逻辑） |
| **P2** | **Cropper (ImageCropper)** | Media | 图像裁剪框控件 |

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

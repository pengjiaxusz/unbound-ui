# 05：接入并校准第二批已实现基准控件（导航、浮层、集合与日期类 23 个）

**要构建什么：** 将已实现的导航、对话框、浮层、集合与日期时间类基准控件（`InfoBar`、`TeachingTip`、`ContentDialog`、`Flyout`、`MenuFlyout`、`MenuBar`、`CommandBar`、`NavigationView`、`TabView`、`Pivot`、`BreadcrumbBar`、`SelectorBar`、`PipsPager`、`Expander`、`SplitView`、`Border/Card`、`StackPanel`、`ScrollView`、`TreeView`、`ListView`、`FlipView`、`CalendarView`、`TimePicker`、`PersonPicture`、`Image`、`FontIcon`、`TitleBar`）纳入批量像素差分或结构快照闭环，并校准其表面层级、8px 容器圆角、3×16px 左侧选中胶囊条与行高。

**阻塞项：** 04：接入并校准第一批已实现基准控件（基础输入与文本类 12 个）

**状态：** ready-for-agent

- [ ] 所有静态可截的容器、导航、日期与状态控件接入状态拼版与 ROI 差分闭环并通过四重门禁
- [ ] `NavigationView`、`ListView`、`TreeView`、`SelectorBar` 的行高、悬停薄纱与左侧 3×16px 选中胶囊条（Selection Pill）与 WinUI 3 完全对齐
- [ ] `ContentDialog`、`Flyout`、`MenuFlyout`、`TeachingTip`、`Card` 的 8px 外圆角、描边与解析阴影对齐 WinUI 3 规范
- [ ] 将容器、浮层与导航类的实测规律追加回填至 `fluent-control-design` 技能参考手册

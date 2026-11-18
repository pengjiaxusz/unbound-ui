# 06：实现并校准 WinUI 3 缺失必补 P0 控件（7 个核心输入与日期选择器）

**要构建什么：** 实现三合一清单中标注为 P0 的 7 个 WinUI 3 核心缺失控件：`HyperlinkButton`、`RepeatButton`、`DropDownButton`、`ToggleSplitButton`、`PasswordBox`（含掩码与按住显密眼睛按钮）、三列滚轮式 `DatePicker`（月/日/年下拉选择）与 `CalendarDatePicker`（收起态带日历图标触发器 + 弹出日历面板），并同步接入展台、24 态形态恒定测试与批量 ROI 像素差分。

**阻塞项：** 03：校准首批 6 个试点基准控件通过有效对比区（ROI）四重门禁

**状态：** ready-for-agent

- [ ] 7 个 P0 控件全部在组件库中实现，具备完整的键盘/指针交互与无障碍角色语义，并注册进组件目录与展台画廊
- [ ] 7 个 P0 控件全部通过 24 态图元数恒定（Stable Shape）与无头五项几何体检
- [ ] 7 个 P0 控件全部接入参考截图宿主与 ROI 像素差分闭环，并通过四重门禁

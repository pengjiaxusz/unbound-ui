# 07：实现并校准 WinUI 3 缺失 P1 控件与 Community Toolkit P2 扩展控件

**要构建什么：** 实现清单中的 P1 缺失控件（`RichEditBox`、`GridView`、`ListBox`、纯圆点态 `PipsPager`、`CommandBarFlyout`、`AppBarButton / AppBarToggleButton / AppBarSeparator`、`SwipeControl`、`RefreshContainer`、带 Tail 尖角与 Hero 区的完整 `TeachingTip`、`AnnotatedScrollBar`、`ItemsView`）及按需选补的 Community Toolkit 控件（如 `RangeSelector`、`HeaderedContentControl`），接入展台与差分验证。

**阻塞项：** 05：接入并校准第二批已实现基准控件（导航、浮层、集合与日期类 23 个）, 06：实现并校准 WinUI 3 缺失必补 P0 控件（7 个核心输入与日期选择器）

**状态：** ready-for-agent

- [ ] 新增控件全部接入组件目录穷举探针、24 态形态恒定测试、无头几何体检与展台交互页
- [ ] 静态与复合态截图接入批量 ROI 像素差分并通过门禁
- [ ] 更新 `docs/winui3-control-inventory.md` 中的实现状态为“已实现并对齐”

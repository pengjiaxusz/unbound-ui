# 04：切片 3 —— 集合与树形列表类控件对齐

**要构建什么：** 
打通集合与树形列表类基准控件（`TreeView`、`ListView`、`GridView`、`ListBox`、`FlipView`、`RefreshContainer`、`AnnotatedScrollBar`）的离屏差分对齐闭环。彻底废除此前树形与列表控件使用的历史展厅 mock 数据（如 `crates/unbound-widgets`），对齐官方 2 级树展开结构与标准列表项数据源，消除几何偏移与色差。

**阻塞项：** 01：容器标准插桩通用辅助库与配方契约基础

**状态：** ready-for-agent

- [x] `TreeView` 在差分器中全面对齐 2 级展开结构 `Root`（首项选中态）+ `Child`，消除展台路径 mock 数据并对齐折叠箭头与左侧 Accent 胶囊条
- [x] `ListView` 与 `ListBox` 统一列表条目数据源、行高与选中指示器
- [x] `GridView` 对齐 2D 网格单元布局与选中高亮边框
- [x] `FlipView` 对齐内容卡片与翻页导航前后按钮
- [x] `RefreshContainer` 与 `AnnotatedScrollBar` 消除边界与指示器槽位偏差
- [x] 该组 7 个控件在 ROI 四重门禁下全量回归通过

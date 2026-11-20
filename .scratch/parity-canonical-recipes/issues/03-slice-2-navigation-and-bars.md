# 03：切片 2 —— 导航与命令条类控件对齐

**要构建什么：** 
打通导航与命令条类基准控件（`BreadcrumbBar`、`SelectorBar`、`PipsPager`、`CommandBar`、`MenuBar`、`TitleBar`、`NavigationView`、`TabView`）的离屏差分对齐闭环。为缺失独立分支的导航组件接入官方镜像配方与 3 级路径插桩，消除此前滑入展台 demo 的文本偏差，实现整组控件在 ROI 四重门禁下的全量通过。

**阻塞项：** 01：容器标准插桩通用辅助库与配方契约基础

**状态：** closed

- [x] `BreadcrumbBar` 在差分器中实现独立分支，插桩文本 1:1 对齐 C# 宿主的 `Root > Folder > File.txt`，校准分隔符与字重
- [x] `SelectorBar` 对齐 `Day`、`Week`、`Month` 三段分段项与选中胶囊滑块指示器
- [x] `PipsPager` 对齐 5 页指示点与当前选中页标记
- [x] `CommandBar` 与 `MenuBar` 补齐独立分支，对齐图标槽位、文本标签与高度
- [x] `TitleBar` 消除局部边距与窗口标题文字基线偏差
- [x] `NavigationView` 与 `TabView` 补齐独立分支，对齐菜单折叠态与标签页排版
- [x] 该组 8 个控件在 ROI 四重门禁下全量回归通过

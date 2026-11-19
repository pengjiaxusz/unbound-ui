# 02：切片 1 —— 布局容器与文本编辑控件对齐

**要构建什么：** 
打通布局容器与文本编辑类控件（`StackPanel`、`ScrollView`、`Border`、`Card`、`RichEditBox`、`SwipeControl`）的基准差分端到端对齐闭环。将 `RichEditBox` 剥离内置工具栏还原为官方纯净富文本编辑区域；将 `SwipeControl` normal 态对齐为静态未滑开卡片并修正底色；为 `StackPanel` 与 `ScrollView` 接入统一的 3 行文本标准插桩；并在有效对比区四重门禁下跑通全部 24 个用例。

**阻塞项：** 01：容器标准插桩通用辅助库与配方契约基础

**状态：** ready-for-agent

- [ ] `RichEditBox` 描述符重构：剥离顶部 `[B][I][U]` 格式工具栏绘制与额外包围盒计算，尺寸、边框与内边距 100% 对齐 WinUI 3 官方标准
- [ ] `SwipeControl` 状态机与渲染对齐：normal 态对齐为静止未滑动列表行，修正暗黑模式下的底板背景画刷，checked 态方展现滑出删除动作
- [ ] `StackPanel` 在差分器中实现独立分支，接入标准 `Row 1`、`Row 2`、`Row 3` 插桩并对齐项间距
- [ ] `ScrollView` 在差分器中实现独立分支，接入标准 3 行条目插桩并校准滚动条槽位与视口
- [ ] `Border` 与 `Card` 对齐圆角、微描边与固有尺寸
- [ ] 该组 6 个控件在 ROI 四重门禁（包围盒误差 $\le 1\text{px}$、固有尺寸 $\le 1\text{px}$、mean $\le 1.5$、p95 $\le 4.0$、超限像素 $\le 2\%$）下 100% 通过差分

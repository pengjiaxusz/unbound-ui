# spec(parity): WinUI 3 基准控件差分配方统一、标准插桩与原生形态对齐

## 问题陈述（Problem Statement）

在当前框架的 WinUI 3 像素差分自动化测试中，未通过的 33 个基准对齐控件中绝大多数并非由于细微抗锯齿或调色板色差失败，而是因为双端测试的**内容本身存在结构性巨大差异**。

核心痛点源于双端缺乏唯一的用例规格事实源：
1. **展台演示降级逃逸**：16 个基准控件在差分器中未实现独立分支，直接隐式回退到交互展厅演示目录（`catalog.rs`），绘制了带有随机 mock 数据的展台界面（如 `Root > src > main.rs`、Layer 3 堆叠卡片、3000px 虚拟滚动条等），而 WinUI 3 参考宿主则是标准的 `Root > Folder > File.txt` 或 `Row 1/2/3`；
2. **两端硬编码假数据脱节**：17 个已有分支的控件在两端分别由不同开发者手写假数据，甚至存在组件形态理解偏差（如 `RichEditBox` 内部强行绘制了顶部 `[B][I][U]` 格式工具栏，`SwipeControl` 预先画出了滑开删除态），导致包围盒与有效对比区（ROI）直接触发门禁拦截（fail），掩盖了底层的真实渲染精度校验。

## 解决方案（Solution）

依据 ADR 0016 与领域模型，确立**双端差分配方单一事实源（Single Source of Truth）**与**基准形态对齐（Canonical Form Parity）**工程标准：

1. **彻底斩断展台兜底通道**：所有基准控件在差分器中均实现 1:1 独立镜像分支，杜绝展台 mock 数据污染；在四个切片全部就绪后，将通用兜底分支彻底收口报错；
2. **基准形态纯净化**：基准控件 100% 对齐 WinUI 3 官方原生标准模板，剥离内置非标外壳。`RichEditBox` 还原为纯净富文本编辑区域（工具栏转为展台组合示例）；`SwipeControl` normal 态对齐为静止未滑动态；
3. **标准化插桩（Standardized Slot Fixture）**：为容器与集合类控件制定确定性的标准测试插桩规范（固定路径 3 级、树形 2 级展开、列表 3 条目），并严格确保禁用态（disabled）递归传递给内部子节点；
4. **四个垂直切片渐进落地**：按领域结构将 33 个控件分为四批垂直切片依次推进，并在既有 CIEDE2000 有效对比区四重门禁下逐一切片验证闭环。

## 用户故事（User Stories）

1. 作为框架维护者，我希望每个基准对齐控件在离屏差分器中都拥有确定的独立分支，以便彻底杜绝展台演示 mock 数据的隐式污染。
2. 作为 UI 开发者，我希望基准对齐控件严格还原 WinUI 3 官方纯净形态，以便在差分中专注于官方标准几何、描边与圆角的精准度，不受外挂工具栏干扰。
3. 作为测试自动化引擎，我希望容器与复合控件使用确定性的标准插桩子项，以便在测试边框、内边距与滚动条时，排版内容在两端绝对可复现。
4. 作为质量保证工程师，我希望基准控件的禁用态能够递归禁用其内部所有插桩子项，以便精准捕获两端在禁用半透明度与文本变灰上的一致性。
5. 作为框架开发者，我希望带工具栏的富文本编辑器以专属复合示例（如展台示例或自有组件）的形式存在，以便在保留高级交互功能的同时不污染官方基准控件。
6. 作为差分回归系统，我希望继续沿用 CIEDE2000 有效对比区四重门禁，以便在内容完全一致后严格把关真实视觉偏差，并通过显式白名单隔离纯抗锯齿残留。
7. 作为协作 Agent，我希望实施过程划分为清晰的四个垂直切片，以便分步推进、增量验证并降低回归风险。

## 实现决策（Implementation Decisions）

- **垂直切片划分与推进顺序**：
  - **切片 1（布局容器与文本编辑，共 6 个）**：`StackPanel`、`ScrollView`、`Border`、`Card`、`RichEditBox`、`SwipeControl`；
  - **切片 2（导航与命令条，共 8 个）**：`BreadcrumbBar`、`SelectorBar`、`PipsPager`、`CommandBar`、`MenuBar`、`TitleBar`、`NavigationView`、`TabView`；
  - **切片 3（集合与树形列表，共 7 个）**：`TreeView`、`ListView`、`GridView`、`ListBox`、`FlipView`、`RefreshContainer`、`AnnotatedScrollBar`；
  - **切片 4（浮层、日期与辅助，共 12 个）**：`Flyout`、`MenuFlyout`、`InfoBar`、`Expander`、`SplitView`、`CalendarView`、`TimePicker`、`PersonPicture`、`FontIcon`、`AppBarButton`、`AppBarToggleButton`、`Pivot`。
- **基准控件形态解耦与标准化**：
  - `RichEditBox` 描述符剥离顶部格式按钮绘制与包围盒计算，尺寸、内边距与边框对齐官方原生 TextBox/RichEditBox 规范；
  - `SwipeControl` 状态机 normal 态对齐为静态未滑开列表项，checked 态为滑开动作条目；
  - 容器标准插桩：路径固定 3 级（`Root > Folder > File.txt`）、树形固定 2 级展开（`Root` + `Child`）、堆叠/滚动固定 3 条目（`Row 1`、`Row 2`、`Row 3`）；
  - 禁用态（disabled）递归遍历插桩子项，统一下发变灰与半透明度衰减；
  - 差分器兜底通道收口：在第 4 切片收官时，将通用兜底替换为显式未实现错误（`Err(format!("unimplemented parity control: {other}"))`）。

## 测试决策（Testing Decisions）

- **测试质量标准**：只测试外部可见的视觉几何、内容排版对齐与像素级 CIEDE2000 色差，不耦合内部私有状态与具体图元下发顺序。
- **最高层级测试接缝**：复用既有的离屏差分器命令与报告输出接缝（`cargo run -p showcase -- --parity-run` 结合 `target/parity/report.json`）。
- **底层快速守护**：配套更新/补充纯代码几何断言测试（如 `crates/unbound-widgets/tests/pilot_geometry.rs`），作为毫秒级快速回归守护。
- **四重门禁硬性指标**：
  1. 内容包围盒逐边误差 $\le 1\text{px}$；
  2. 自由维度固有尺寸误差 $\le 1\text{px}$；
  3. ROI 区域 $\Delta E2000$ 均值 $\le 1.5$ 且 p95 $\le 4.0$；
  4. 显著色差像素占比（$\Delta E2000 > 5.0$）$\le 2\%$；
  5. 极少数经复核确认的纯光栅化字形边缘残差严格走 ADR 0010 的显式 `known-aa` 白名单，不人为下调全局门槛。

## 范围之外（Out of Scope）

- 云母（Mica）与亚克力（Acrylic）等多后景材质渲染（受 ADR 0008 约束，待外部后景管线彻底完工后单独解锁）；
- 43 个 Unbound 自有独有组件的一对一像素对比（依据 ADR 0008 与 ADR 0016，自有独有组件豁免参考图差分，改走 Fluent 规律与设计令牌巡检）；
- 跨语言 DSL 编译器（在 ADR 0016 决策中已被否决，维持代码级 1:1 镜像分支）。

## 补充说明（Further Notes）

- 严格遵循 `AGENTS.md` 编码规范：代码注释强制中文，代码字符串守卫为英文，回复前执行 `just check` / `just test` / `just gate-check` 并提交推送。
- 参考宿主构建环境：`$env:DOTNET_ROOT = "C:\Users\pengj\.dotnet-sdk"`。

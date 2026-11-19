# ADR 0016：基准控件差分配方单一事实源、原生形态对齐与展台兜底截断

- 状态：已采纳
- 日期：2026-11-20
- 关联：ADR 0008, ADR 0010, ADR 0015

## 背景

在基准控件自动化像素差分中，未通过的 33 个控件大面积由“两端内容本身不一致”导致，而非单纯的微观抗锯齿色差：

1. **展台演示降级逃逸**：`apps/showcase/src/parity.rs` 对 `BreadcrumbBar`、`ScrollView`、`StackPanel`、`CommandBar`、`TabView` 等 16 个基准控件缺少独立分支，直接滑入 `other => emit_widget_with_shaper` 兜底，渲染的是 `crates/unbound-widgets/src/catalog.rs` 为交互展厅随手写的 mock 数据（如 `Root > src > main.rs`、Layer 3 堆叠卡片、3000px 虚拟滚动条等），而 WinUI 3 参考宿主则是 `Root > Folder > File.txt` 或 `Row 1/2/3`。
2. **两端硬编码双向脱节**：在已具备独立分支的控件中，两端各自硬编码了互不相通的测试内容与形态。典型如 `RichEditBox`，WinUI 3 原生控件是纯净富文本编辑区域（无工具栏），而 Unbound 端强行在控件内部绘制了顶部 `[B][I][U]` 格式工具栏；`SwipeControl` 在 WinUI 3 处于静止未滑动态，而 Unbound 绘制了已展开露出红色删除按钮的状态。

## 决策

### 决策一：双端差分配方单一事实源（Single Source of Truth）

- **彻底斩断展台兜底通道**：所有基准对齐控件必须在 `parity.rs` 中具备确定的 1:1 独立分支，严格禁止回退到展台演示目录（`catalog.rs`）；兜底分支在全部 33 个控件补齐后改为显式报错。
- **1:1 镜像配方契约**：两端测试用例的尺寸约束、文案、子项数量、初始选中项与属性必须严格对齐。

### 决策二：基准形态对齐（Canonical Form Parity）

- **以 WinUI 3 官方标准模板为唯一真理**：基准对齐控件的第一职责是精准还原 WinUI 3 官方原生标准控件的几何与边框，剥离所有非标外挂外壳。
- **`RichEditBox` 纯净化**：`RichEditBox` 还原为标准的纯净富文本编辑框（几何、边框与内边距 100% 对齐 WinUI 3 TextBox/RichEditBox 规范）；原有的格式工具栏逻辑不作为内置外壳，而是作为展台（Showcase）的高级组合示例保留。
- **`SwipeControl` 状态语义对齐**：normal 态对齐静止未滑动卡片，checked 态或动作态才展现滑出动作。

### 决策三：容器与复合控件标准化插桩（Standardized Slot Fixture）

- **统一标准化插桩模板**：
  1. 路径/分段类（`BreadcrumbBar`, `SelectorBar`）：固定 3 级路径 `Root > Folder > File.txt`；
  2. 树形类（`TreeView`）：固定 2 级展开结构，根节点 `Root`（首项选中态）+ 展开子节点 `Child`；
  3. 滚动/布局类（`ScrollView`, `StackPanel`）：固定 3 个纯文本子项 `Row 1`、`Row 2`、`Row 3`；
  4. 禁用态继承（`disabled`）：双端在禁用态下必须递归将所有子项设为 `disabled`，确保子文本与图标同步应用半透明变灰。

### 决策四：保持 ROI 四重门禁与四批垂直切片推进

- **守住门禁**：不放宽现有的有效对比区（ROI）四重门禁（包围盒 $\le 1\text{px}$、固有尺寸 $\le 1\text{px}$、ROI mean $\le 1.5$、p95 $\le 4.0$、超限像素 $\le 2\%$）；字形微观残差走 ADR 0010 的显式 `known-aa` 白名单。
- **四个垂直切片渐进推进**：
  - 切片 1：布局容器与文本编辑（`StackPanel`、`ScrollView`、`Border`、`Card`、`RichEditBox`、`SwipeControl`，共 6 个）；
  - 切片 2：导航与命令条（`BreadcrumbBar`、`SelectorBar`、`PipsPager`、`CommandBar`、`MenuBar`、`TitleBar`、`NavigationView`、`TabView`，共 8 个）；
  - 切片 3：集合与树形列表（`TreeView`、`ListView`、`GridView`、`ListBox`、`FlipView`、`RefreshContainer`、`AnnotatedScrollBar`，共 7 个）；
  - 切片 4：浮层、日期与辅助（`Flyout`、`MenuFlyout`、`InfoBar`、`Expander`、`SplitView`、`CalendarView`、`TimePicker`、`PersonPicture`、`FontIcon`、`AppBarButton`、`AppBarToggleButton`、`Pivot`，共 12 个）。

## 备选方案

- **定义跨语言的 DSL / AST 代码生成统一树**：评估后认为维护两套代码生成器（C# XAML + Rust 描述符）成本过高且过于脆弱，采用规范化的 1:1 独立镜像分支兼顾了代码类型安全与直接性。
- **差分时引入文字全区域掩模（Text Masking）**：直接剔除文字区域虽然能隐藏差异，但会导致行高、内边距与基准线错误漏检，违背了基准差分的守卫目的。

## 后果

- 彻底根除两端因 mock 数据随机漂移导致的差分假阳性失败。
- 所有基准控件在 `unbound-widgets` 中保持纯净，不再揉杂业务外壳。
- `parity.rs` 结构更加严密，杜绝隐式回退带来的不可预测性。

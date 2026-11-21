# ADR-0024：壳层融合 TabView、双模 AddressBar 与虚拟化导航 TreeView 契约

- **状态**：已采纳
- **影响 crate**：`unbound-widgets`、`unbound-world`、`unbound-app`、`unbound-theme`

## 背景

桌面文件管理器与多标签工作台需要三类高度协同的 Fluent（WinUI 3）导航控件：可融入无边框标题栏的卡片式 `TabView`、支持面包屑与原位路径编辑的双模 `AddressBar`，以及支持懒加载与多挂载点隔离的虚拟化 `TreeView`。

当前框架内三者存在明显能力落差：`tabs.rs` 仅提供矩形底部指示条形态，不支持标题栏融合卡片与自适应宽度；`address_bar.rs` 仅为 198 行静态展示骨架，缺少独立命中区、图标与真实编辑态；`tree_view.rs` 缺少行图标槽、懒加载契约与出现键隔离。同时，根据术语表中的**基准形态对齐**原则，不得在 WinUI 3 基准对齐控件内塞入非标外壳逻辑而污染像素差分。

## 决策

1. **守住基准形态对齐边界**：
   - `TitleBar`、`TabView`、`BreadcrumbBar` 与 `TreeView` 继续作为 WinUI 3 基准对齐控件，默认形态严格对齐官方模板并参与批量像素差分；
   - `AddressBar` 作为自有独有复合控件在 `address_bar.rs` 内完整重写，通过组合 `IconButton`、`BreadcrumbBar`、`编辑核心`（`TextEditState`）、`MenuFlyout` 与 `AutoSuggestBox` 遵循 Fluent 设计规律，不污染基准差分。
2. **标题栏与标签栏以官方协作原语融合，不新建打包窗口控件**：
   - `TabView`（`tabs.rs`）新增 `TabViewStyle::ShellChrome` 卡片融合形态（顶部圆角、底边与下方工具栏表面无缝衔接、非选中标签间细竖线、图标 + 截断标题 + 独立关闭按钮、`+` 新增按钮、自适应宽度 `100..=240px` 与溢出滚动）；
   - `TitleBar`（`title_bar.rs`）扩展左侧内容槽与自适应拖拽区计算（始终保留不小于 `TitleBarMinDragRegionWidth = 48px` 的可拖拽空白区与右侧 `46×3` 窗口控制按钮区），实现类似 WinUI 3 `ExtendsContentIntoTitleBar` + `TabView` 的官方协作模式。
3. **分阶段落地交互深度（Phase 1 / Phase 2）**：
   - **Phase 1（本阶段必达）**：
     - `TabView`：Win11 卡片视觉、标题栏融合、选中/悬停/关闭 `×`、中键关闭、`+` 新增事件与溢出滚动；
     - `AddressBar`：外部独立 `← → ↑ ↻` 微妙图标按钮、一体化地址框、面包屑分段点击、`›` 分隔符触发同级子目录 `MenuFlyout`、点击空白或快捷键原位切入 `编辑核心` 文本编辑，以及右侧可折叠内联搜索框；
     - `TreeView`：行图标槽（`TreeRowIcon`，同时支持框架矢量图标与外部纹理句柄变体）、紧凑行高档位（28px 导航窗格密度，默认保持 WinUI 36px）、懒加载三态（`has_children / is_loaded / is_loading`）与 `ExpandRequested` 事件、以及 `OccurrenceKey` 与逻辑实体 ID 分离。
   - **Phase 2（后续迭代）**：
     - `TabView` 鼠标拖拽重排与跨窗撕出交互衔接（复用 ADR-0023 多窗口原语）；
     - `AddressBar` 文本编辑态路径历史与自动补全建议弹层；
     - `TreeView` 多级冻结祖先（`sticky_items`）与树节点拖拽落点反馈。

## 后果

- 任何使用 `unbound-ui` 的桌面文件管理器或多标签工作台均可直接通过标准 `ViewBuilder` API 组装出与 Windows 11 资源管理器视觉及交互一致的三层壳层（融合标签标题栏 → 双模地址/搜索栏 → 紧凑 `CommandBar` + 虚拟化导航树），无需在应用层自建绕行控件。
- `TreeRowIcon` 预留 `ExternalTexture(ExternalTextureHandle)` 变体，使应用在初期使用框架矢量图标时，未来可零破坏切换至平台原生系统图标纹理。
- `TabView` 与 `TreeView` 的默认构造与 24 态图元恒定门禁保持向后兼容，不影响现有批量像素差分基线。

# ADR 0024：NavigationView 1:1 规范复刻、废弃旧侧边栏与多形态 Parity 矩阵

- 状态：已采纳
- 日期：2026-11-22
- 关联：ADR 0006, ADR 0008, ADR 0015, ADR 0016

## 背景

`unbound-ui` 此前在侧边栏导航领域存在三处分裂，与「基准对齐控件严格对齐 WinUI 3 官方原生形态」的原则相悖：

1. **命名与数据结构分裂**：`crates/unbound-widgets` 同时存在 `sidebar.rs`（以类型别名 `NavigationViewDesc` 挂靠）与 `navigation_rail.rs`（映射为 `WidgetCatalogId::NavigationViewCompact`，硬编码上限 6 项），且 `shell.rs` 中另有一套 `SidebarItemDesc` / `SidebarSectionHeaderDesc`，`apps/showcase/src/shell.rs` 则手写独立滚动侧栏，导致同一概念存在多份真相。
2. **功能与形态残缺**：现有实现仅支持静态单层垂直排版与布尔折叠开关，缺少 WinUI 3 `NavigationView` 的完整形态（`Auto` / `Left` / `LeftCompact` / `LeftMinimal` / `Top`）、多级层级菜单与紧凑态子菜单 Flyout、`AutoSuggestBox` / `BackButton` / `PaneToggleButton` 与 `TitleBar` 联动、窗格独立滚动、键盘树导航以及选中指示条滑动连接动画。
3. **差分校验失真**：`apps/showcase/src/parity.rs` 的 `"NavigationView"` 分支此前调用 `slot_fixture::emit_canonical_navigation_view_fixture` 发射手写假图元，而非驱动真实控件；`tools/winui-reference/Catalog.cs` 也仅采集一个 `300×200` 的 `Left` 静态快照，无法守护完整复刻的视觉一致性。

## 决策

1. **彻底清除旧模块与别名，统一为规范 `NavigationView`**：
   - 删除 `crates/unbound-widgets/src/sidebar.rs` 与 `navigation_rail.rs`，不保留 `SidebarDesc` / `NavigationRailDesc` 等兼容别名，新增单一规范模块 `navigation_view.rs`。
   - 将 `WidgetCatalogId::NavigationViewCompact` 合并回 `WidgetCatalogId::NavigationView`（目录总数从 134 调整为 133），全量迁移仓库内全部调用点；在 `apps/showcase` 切换至新 `NavigationView` 后，一并移除 `shell.rs` 中的 `SidebarItemDesc` 与 `SidebarSectionHeaderDesc`。
2. **WinUI 3 精确词汇 + Rust 编码规范**：
   - 控件顶层遵循 `unbound-widgets/build.rs` 契约，命名为 `NavigationViewDesc`、`NavigationViewState` 与 `emit_navigation_view`。
   - 所有子类型、枚举、枚举值、属性与事件名 1:1 对齐 WinUI 3 官方 API 并严格遵守 Rust 命名规范（类型/变体 `UpperCamelCase`，字段/方法 `snake_case`），覆盖 `NavigationViewItem`、`NavigationViewItemHeader`、`NavigationViewItemSeparator`、`NavigationViewPaneDisplayMode::{Auto, Left, LeftCompact, LeftMinimal, Top}`、`NavigationViewDisplayMode::{Minimal, Compact, Expanded}`、`NavigationViewBackButtonVisible::{Collapsed, Visible, Auto}` 及 `NavigationViewEvent`（`ItemInvoked`、`SelectionChanged`、`BackRequested`、`PaneOpening`、`PaneClosing`、`DisplayModeChanged`、`Expanding`、`Collapsed`）。
3. **完整应用导航壳与纯状态机职责边界**：
   - `NavigationView` 负责整块视口的窗格、覆盖浮层、页头与右侧圆角内容区布局，支持与 `TitleBarDesc` 联动返回键和窗格折叠按钮。
   - 控件本身不持有页面路由历史栈（BackStack），仅管理导航 UI 状态与事件输出；Overlay 浮层打开时，返回键、`Esc` 与点击外部优先关闭浮层（触发 `PaneClosing`）。
4. **原生材质分层、动效与键盘/无障碍契约**：
   - **材质（聚焦原生路径，不造自绘亚克力）**：`Left` 常驻展开态与 `Top` 导航条保持透明以透出窗口级原生 Mica，右侧内容区绘制带圆角与描边的 `LayerFillColorDefault` 表面；`LeftCompact` / `LeftMinimal` 展开的 Overlay 浮层与子菜单 Flyout 通过引擎现有 `BackdropRequest`（Dual-Kawase 实时模糊）叠加 WinUI 3 半透明叠色、描边与投影，直接对齐原生 In-App Acrylic。
   - **动效**：通过 `unbound-anim` 驱动选中指示条跨项滑动连接动画、窗格/浮层开合过渡与层级展开动画，以显式 `tick(dt)` 推进并提供 `snap_animations()` 供无头测试与 Parity 瞬间收敛。
   - **键盘与无障碍**：严格对齐 WinUI 3 方向键/`Space`/`Enter`/`Esc` 树形与浮层导航规则及 AccessKit 语义角色（`Navigation`、`Tab`/`TreeItem`、`Button`），焦点环统一按 `rect.inflate(3.0)` 绘制。
5. **删除假插桩，建立 7 场景多形态 Parity 矩阵**：
   - 删除 `slot_fixture::emit_canonical_navigation_view_fixture`，`parity.rs` 100% 驱动真实 `NavigationView` 发射器。
   - 在 `tools/winui-reference/Catalog.cs` 与 `apps/showcase/src/parity.rs` 双端同步注册 7 个核心形态场景：`NavigationView`（`Left` 展开）、`NavigationViewLeftClosed`、`NavigationViewLeftCompact`（折叠 + Overlay）、`NavigationViewLeftMinimal`（极简 + Overlay）、`NavigationViewTop`（含溢出菜单）、`NavigationViewHierarchical`（多级展开、折叠态祖先指示条回退、Compact/Top 子菜单 Flyout）、`NavigationViewChrome`（`AutoSuggestBox`、`BackButton`、`PaneTitle`/`PaneHeader`、`Header`），叠加 Light/Dark 与交互状态走 ROI 四重门禁。
6. **两阶段垂直切片落地**：
   - **切片 A**：完整数据模型 + 5 种 `PaneDisplayMode` 布局与原生材质视觉 + 旧模块清除与全仓迁移 + 7 场景双端静态 Parity 闭环。
   - **切片 B**：层级展开/Flyout + 窗格独立滚动 + 搜索框/TitleBar 联动 + 键盘树导航 + AccessKit + 选中指示条滑动与窗格开合动效 + `apps/showcase` 全量 dogfood 替换。

## 备选方案

- **保留 `SidebarDesc` / `NavigationRailDesc` 为薄兼容别名**：虽可减少一次性迁移范围，但会在代码库中长期残留非 WinUI 3 命名与重复概念，违背基准对齐控件单一事实源目标，故否决。
- **仅做左侧 4 种模式、推迟 `Top` 模式与原生 In-App Acrylic**：无法达成「与 Windows 11 完整一致」及全量 Parity 对比目标；且引擎已内建 `BackdropRequest` Dual-Kawase 局部模糊管线，无需大改渲染管线即可实现浮层实时毛玻璃，故选择一步到位纳入设计。
- **修改 `build.rs` 去掉 `Desc` 后缀**：会破坏 `unbound-widgets` 全库统一的结构命名与编译期门禁检查，收益低于一致性损失，故保留 `*Desc` / `*State` / `emit_*` 顶层契约。

## 后果

- `WidgetCatalogId::COUNT` 从 134 变更为 133，相关目录探针与快照测试需同步更新。
- 仓库内 18 个引用旧 `sidebar` / `navigation_rail` 的文件需在切片 A 内一次性迁移，短期改动面集中但彻底消除历史债务。
- `NavigationView` 成为首个在双端 Parity 中覆盖多形态场景矩阵的复合控件，其双端镜像模式可作为后续复杂容器控件全量差分校验的模板。

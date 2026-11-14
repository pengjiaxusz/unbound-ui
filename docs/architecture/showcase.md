# 架构：showcase
状态：已落地
路径：`apps/showcase/`、`apps/gallery/`
入口：`apps/showcase/src/main.rs`、`apps/showcase/src/pages/components.rs`、`apps/showcase/src/pages/systems.rs`
守卫：`cargo test -p showcase` / `just showcase -- --shot /tmp/out.png`

## 范围
- [负责] 自举式 UI 组件与引擎架构展台（8 大导航分类、24 页示例卡、侧边栏全部 91 个 `WidgetCatalogId` 组件直达导航、`PageId::Systems` 全部 21 个核心引擎子系统架构全景页、Knob 实时调参、代码/A11y/文档面板、搜索与 HUD、`Ctrl+N` 多窗口）、无头出图及 `gallery` KPI 压测入口
- [不负责] 通用框架运行时能力 -> 委托给 `crates/unbound-app/` 与 `crates/unbound-widgets/`

## 拓扑与流程
- 自举架构：壳层几何由 `UiWorld` + `LayoutEngine` 求解，事件与帧差分经 `AppRuntime<GalleryRuntime>` 归约，悬停/聚焦走局部补丁上传
- 侧边栏平滑折叠/展开：侧边栏宽度由临界阻尼弹簧 `Spring::critically_damped(420.0, NAV_EXPANDED).with_rest_thresholds(0.25, 2.0)` 驱动并在布局求解前对齐至物理像素网格；`draw_nav`、`nav_rows` 与 `nav_height` 统一使用 `NAV_COLLAPSE_THRESHOLD = 140.0`，折叠态图标按钮采用固定宽度（`NAV_COLLAPSED - 28.0 = 40.0`）与固定水平中心点（`x = 34.0`），并开启 `layout.nav` 裁剪作用域，彻底消除折叠与展开过程中的弹簧过冲反弹和图标水平位移抖动
- 页面、组件与子系统三级编译期完备性门禁：
  1. **页面全覆盖门禁**：`pages/mod.rs` 与 `shell.rs` 的编译期 `const` 断言强制校验 `PageId::ALL`（24 页）中的每一个页面均在侧边栏分类树（`PageId::Home` + `CategoryId::ALL`）中精确出现一次
  2. **91 组件全覆盖与侧边栏直达门禁**：`pages/components.rs` 通过 `define_showcase_components!` 从单一组件列表同时生成 `COMPONENT_EXAMPLES`、`SIDEBAR_COMPONENT_IDS`、`component_nav_label`、`component_card_index`、`component_scroll_offset` 与 `showcase_entry_for_widget` 穷举映射；侧边栏在 `CategoryId::Components` 下为全部 91 个组件生成 `Target::NavComponent(u8)` 导航项，点击即可跳转并自动滚动至对应组件卡片
  3. **21 引擎子系统全景页门禁**：`pages/systems.rs` 定义 `SubsystemId`（覆盖工作区全部 21 个核心 crate 子系统），通过 `#[deny(unreachable_patterns)]` 与编译期 `const` 断言确保无任何子系统遗漏，并提供交互式子系统矩阵卡片、端到端帧管线时序舞台与编译期契约验证舞台
- 文档联动：`PageInfo::docs` 与 `SubsystemMeta::doc_path` 指向 `docs/architecture/<module>.md`，供展台侧边栏跳转查阅

## 取舍（禁止重构）
- [代价] `pages/components.rs`、`pages/systems.rs`、`pages/mod.rs` 与 `shell.rs` 采用多重编译期 `const` 断言 + 穷举 `match` -> [原因] 强制任何新增页面、新增 `WidgetCatalogId` 组件或新增引擎子系统都必须同步接入展台卡片与侧边栏导航，杜绝遗漏或手绘冒充 -> [位置] `apps/showcase/src/pages/{mod.rs, components.rs, systems.rs}`、`apps/showcase/src/shell.rs`
- [代价] 侧边栏折叠动画采用临界阻尼弹簧 + 固定折叠图标中心坐标而非随 `nav_width` 线性缩放按钮宽度 -> [原因] 欠阻尼弹簧撞击 `.clamp(68.0, 248.0)` 会产生反弹抖动，而随宽度缩放折叠按钮会导致图标中心每帧水平漂移 -> [位置] `apps/showcase/src/shell.rs`
- [代价] 展台支持 `--shot <path>` 无头离屏出图 -> [原因] 让 AI Agent 与 CI 在无人工盯屏时验证真实渲染像素 -> [位置] `apps/showcase/src/main.rs`

## 不变量与红线
- [禁止] 在组件画廊页面用手绘基础矩形替代 `catalog::emit_widget` 真实发射器 -> [原因] 绕过组件真实发射覆盖率检验 -> [守卫] `apps/showcase/tests/shell.rs`
- [必须] 运行时滚动全页后 `emitted_widgets` 并集 100% 覆盖 `WidgetCatalogId::ALL`，且 `sidebar_all_targets()` 100% 覆盖全部 `PageId::ALL` 与 `WidgetCatalogId::ALL` -> [守卫] `apps/showcase/tests/shell.rs`
- [必须] 侧边栏折叠与展开过程中 `layout.nav_width` 每帧严格单调变化，零反向抖动 -> [守卫] `apps/showcase/tests/shell.rs::sidebar_collapse_and_expand_is_monotonic_without_jitter`
- [必须] 10 个 `unbound-app` 示例逐个开窗抓屏验收 -> [原因] 示例走通用 `WindowedRunner`，与展台自管呈现链不同；「进程不崩」无法证明像素正确 -> [守卫] `just test-examples` + 开窗抓屏

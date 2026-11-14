# 架构：showcase
状态：已落地
路径：`apps/showcase/`、`apps/gallery/`
入口：`apps/showcase/src/main.rs`、`apps/showcase/src/pages/components.rs`
守卫：`cargo test -p showcase` / `just showcase -- --shot /tmp/out.png`

## 范围
- [负责] 自举式 UI 组件展台（21 页示例卡、Knob 实时调参、代码/A11y/文档面板、搜索与 HUD、`Ctrl+N` 多窗口）、无头出图及 `gallery` KPI 压测入口
- [不负责] 通用框架运行时能力 -> 委托给 `crates/unbound-app/` 与 `crates/unbound-widgets/`

## 拓扑与流程
- 自举架构：壳层几何由 `UiWorld` + `LayoutEngine` 求解，事件与帧差分经 `AppRuntime<GalleryRuntime>` 归约，悬停/聚焦走局部补丁上传
- 组件覆盖门禁：`pages/components.rs` 通过 `define_showcase_components!` 从单一 ID 列表生成示例卡与 `showcase_entry_for_widget` 穷举映射，每张卡经 `Canvas::widget_output` 登记 `emitted_widgets`
- 文档联动：`PageInfo::docs` 指向 `docs/architecture/<module>.md`，供展台侧边栏跳转查阅

## 取舍（禁止重构）
- [代价] `pages/components.rs` 编译期 `const` 断言遍历 `WidgetCatalogId::ALL` -> [原因] 强制任何新增组件必须接入真实展台卡片，杜绝手绘矩形冒充或遗漏 -> [位置] `apps/showcase/src/pages/components.rs`
- [代价] 展台支持 `--shot <path>` 无头离屏出图 -> [原因] 让 AI Agent 与 CI 在无人工盯屏时验证真实渲染像素 -> [位置] `apps/showcase/src/main.rs`

## 不变量与红线
- [禁止] 在组件画廊页面用手绘基础矩形替代 `catalog::emit_widget` 真实发射器 -> [原因] 绕过组件真实发射覆盖率检验 -> [守卫] `apps/showcase/tests/shell.rs`
- [必须] 运行时滚动全页后 `emitted_widgets` 并集 100% 覆盖 `WidgetCatalogId::ALL` -> [守卫] `apps/showcase/tests/shell.rs`
- [必须] 10 个 `unbound-app` 示例逐个开窗抓屏验收 -> [原因] 示例走通用 `WindowedRunner`，与展台自管呈现链不同；「进程不崩」无法证明像素正确 -> [守卫] `just test-examples` + 开窗抓屏

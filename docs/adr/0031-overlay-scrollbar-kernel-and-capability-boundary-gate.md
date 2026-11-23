# ADR 0031：统一按需叠加滚动条内核与控件能力边界双门禁

- 状态：已采纳
- 日期：2026-11-24
- 关联：ADR 0014（能力闭环 DoD）、ADR 0016（ScrollView 差分基线）、ADR 0024（TabView/TreeView 契约）、ADR 0026（NavigationView 规范复刻与 Parity 矩阵）

## 背景

全仓核查暴露出两个互为因果的系统性盲区：

1. **复合组件普遍缺滚动条**：除 `scroll.rs` 自身外，全仓 7 个垂直可滚动复合组件（`NavigationView` Left 窗格、`TreeView`、`Table`、`GenericDataTable`、`VirtualGrid`、`TextArea`、`Menu`）虽已响应滚轮滚动，却全部未渲染按需滚动条；原因在于 `scroll.rs` 缺少无需接管 `clip`/`transform` 的轻量独立 `ScrollBarDesc` / `emit_scrollbar`，同时现有 `emit_scroll_view` 亦存在静止 `bar_opacity = 0.0` 全透明、漏注册 `track_rect` 翻页命中区与展开态缺轨道胶囊等缺口。
2. **WinUI 3 Parity 静态小样本遗漏溢出维度**：现有 Parity 矩阵（如 `NavigationView` 7 场景）仅塞入 2–4 条未溢出数据，导致双端参考图均不触发滚动条，像素差分一路绿灯却漏掉了整个溢出与滚动条交互维度。

## 决策

- **抽出轻量独立 `ScrollBarDesc` / `emit_scrollbar` 内核**：在 `crates/unbound-widgets/src/scroll.rs` 提供不强绑 `ScrollView` 裁剪变换的独立 Win11 叠加滚动条组件与 `ScrollBarVisibility::{Auto, AlwaysExpanded, Hidden}`；默认 `Auto` 策略下，未溢出不发射任何图元与命中区，溢出后静止态显示克制的 2px 细指示条，悬停滚动条热区或拖拽时展开为 6px 粗滑块与半透明轨道胶囊，并补齐 `track_rect` 命中区以支持点击轨道翻页；`emit_scroll_view` 内部复用该内核且保持 ADR-0016 未溢出基线不受破坏。
- **7 个垂直可滚动组件统一接入，显式排除非滚动条控件**：`NavigationView`（仅 Left 窗格菜单区）、`TreeView`、`Table`（仅 `body_rect`）、`GenericDataTable`、`VirtualGrid`、`TextArea`、`Menu`（仅溢出视口高时）一次性接入该内核；`TabView`（`tabs.rs`）按 WinUI 3 规范走左右翻页箭头按钮而非底边滚动条，`Form` 保持无内置滚动容器（交由外层 `ScrollView`），二者显式排除。
- **建立「编译期穷举 + 运行期契约 + 流程技能」三层防漏门禁**：
  1. 在 `crates/unbound-widgets/src/canonical.rs` 对全部 `WidgetCatalogId` 穷举实现 `capabilities() -> WidgetCapabilities`（显式声明 `OverflowStrategy` 等能力维度），新增控件漏声明即触发编译错误 `E0004`；
  2. 在 `crates/unbound-widgets/tests/integration/` 增加参数化契约测试，对全部声明 `OverlayScrollbar*` 的组件强制校验「不溢出 = 0 滚动条图元、溢出 = 产出可见滚动条图元与轨道/滑块命中区」，并在 `parity.rs` 补齐 `NavigationView` 溢出态场景；
  3. 将能力边界维度核对表固化至 `.agents/skills/fluent-control-design/SKILL.md` 与 `.agents/skills/to-spec/SKILL.md`。

## 备选方案

- **强迫所有复合组件改用 `ScrollView` 包裹**：`VirtualGrid`、`Table`（含固定表头）、`TreeView`、`NavigationView` 均自带虚拟化或自定义裁剪区域，强行嵌套 `ScrollView` 会破坏虚拟化与布局契约；抽离独立 `ScrollBarDesc` / `emit_scrollbar` 复用成本最低。
- **仅靠人工清单或单测防漏，不在 `canonical.rs` 加穷举表**：未来新增控件仍可能遗忘注册测试；把能力声明挂在无 `#[non_exhaustive]` 的 `WidgetCatalogId` 上可获得编译器级强制约束。

## 影响

- `unbound-widgets` 导出 `ScrollBarDesc`、`ScrollBarVisibility`、`emit_scrollbar` 与 `WidgetCapabilities` / `OverflowStrategy`；
- 7 个可滚动复合组件统一具备一致的 Win11 按需叠加滚动条体验；
- 未来任何新控件在 Spec 与编译阶段均须显式裁决能力边界维度，杜绝仅靠静态小样本漏掉溢出与交互维度。

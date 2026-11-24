# ADR-0044: 控件命中路由、零私有图集与展台真实交互契约

Status: accepted

## 背景

在自举展台（`apps/showcase`）与多区域复合控件联调过程中，暴露出四类具有共同架构根因的跨 crate 回归：

1. **私有图集导致的字形 UV 失配**：`catalog.rs` 中多处控件发射路径在函数内部私自构造并当场丢弃局部 `GlyphCache::new()` 与 `TextEngine::new()`。产出的 `GlyphQuad` UV 指向从未上传到 GPU 共享图集的局部缓存，导致文字渲染为空白或错乱图块。
2. **热区 `NodeId` 跨区复用与 Z 序扁平化**：宿主画布（含 Showcase `Canvas`）在不同 UI 区域复用相同 `NodeId`，或在合并 `WidgetOutput` 时将控件内部的 `hit` 与 `sub_hits` 统一覆写为同一平面 Z 值，造成跨区点击串扰或底板热区反过来遮挡子热区。
3. **布局坐标与命中判定割裂及未裁剪滚动热区**：部分 `UiWidget::reduce` 忽略布局阶段传入的 `rect` 参数（退回描述符上可能为 `Rect::ZERO` 的初始矩形）；带内部滚动视口的复合控件（如 `NavigationView`）虽已按 ADR-0042 裁剪视觉图元，但其 `sub_hits` 与指针命中仍未受 `menu_viewport_rect` 裁剪，滚出视口的主菜单项会抢占固定页脚点击；同时 `AppRuntime` 未统一向 `mounted_widgets` 转发 `PointerMove`，导致悬停与子热区追踪缺失。
4. **展台假交互与宽度拍脑袋估算**：展台演示舞台对部分组件以盲目取模轮换（如 `% 5`）伪造交互，未传入真实子热区与指针坐标；徽章、卡片与复合水平组件按固定字符宽估算占位而非消费实测宽度，导致窄宽下溢出或互相重叠。

## 决策

冻结以下四条跨 crate 长期不变式，任何新增控件、目录发射器或宿主画布均须遵守：

1. **零私有图集不变式（Zero Private Atlas Invariant）**：`catalog.rs` 及任何控件发射器严禁私自构造并丢弃局部 `GlyphCache::new()` / `TextEngine::new()`。所有文字字形必须通过宿主传入的 `shape` 回调或已真实上传 GPU 的共享图集生成，确保每条字形 UV 与 GPU 图集页严格一一对应。
2. **热区作用域唯一与相对 Z 序守恒（Hit Scope Isolation & Relative Z-Order Preservation）**：宿主画布（含 Showcase `Canvas`）严禁在不同 UI 区域复用相同 `NodeId`，必须按区域分配互不重叠的 `NodeId` 命名空间；合并 `WidgetOutput` 时必须严格保留控件内部 `hit` 与 `sub_hits` 的相对 Z 序（仅允许整体平移基底 Z 偏移），禁止扁平覆写为单一 Z 值。
3. **布局与命中同源 + 视口热区裁剪（Layout-Hit Parity & Viewport Hit Clipping）**：`UiWidget::reduce` 必须以布局阶段传入的 `rect` 作为唯一几何基准执行命中判定；所有带内部滚动视口的复合控件（如 `NavigationView`）必须将 `sub_hits` 边界与指针命中判定同步裁剪至 `menu_viewport_rect`（视觉裁剪与交互裁剪严格同源）；`AppRuntime` 统一向 `mounted_widgets` 转发 `PointerMove` 事件。
4. **展台真实交互与徽章测绘同源（Authentic Showcase Interaction & Measured-Width Single Source of Truth）**：展台演示舞台必须向受测组件传递真实子热区与指针坐标（严禁 `% 5` 等假交互轮换）；徽章、卡片与复合水平组件（如 `AddressBar`、`CommandBarFlyout`、`MenuBar`、`ShortcutBar`）必须以实测宽度为单一事实源，并在窄宽下保证子项不溢出容器且互不重叠。

## Considered Options

- **在 `catalog.rs` 中缓存局部 `TextEngine` / `GlyphCache` 再回拷位图到主图集**：引入额外 CPU 拷贝与双图集生命周期同步复杂度，仍无法防止不同发射器各自持有一份图集状态。
- **仅靠宿主外层按绘制顺序或手工 Z 偏移规避热区冲突**：每次新增展台页面或复合控件都极易漏配，无法从协议层保证 `hit` / `sub_hits` 相对深度守恒。
- **仅做视觉局部裁剪（ADR-0042）而不裁剪滚动子热区**：画面虽不溢出，但用户点击页脚仍会被不可见的滚出视口主菜单行拦截，形成「所见非所点」交互缺陷。

## Consequences

- `unbound-widgets` 通过无头回归门禁测试（`crates/unbound-widgets/tests/hit_routing_and_atlas_contract.rs`）对零私有图集、`PivotDesc::reduce` 布局同源、`Pivot` 指示条与焦点环对齐、`NavigationView` 视口热区裁剪、`AddressBar` 刷新弧线与窄宽保护以及水平复合组件防重叠实施自动化守护。
- 所有宿主（`apps/showcase`、`AppRuntime` 与下游应用）共享同一套 `NodeId` 作用域隔离、相对 Z 序合入与 `PointerMove` 转发契约，消除跨区误命中与展台假交互。

- 存量展台演示中尚未直连独立子热区的只读/次要组件允许分批渐进迁移，但所有新增或修改的交互组件及全部导航/选择器控件必须严格遵守真实子热区与坐标归约。

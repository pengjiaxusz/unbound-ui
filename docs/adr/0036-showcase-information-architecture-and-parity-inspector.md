# ADR 0036：展台信息架构重构与主从式像素对照检视台

**Status**: accepted

## Context

原有自举展台（`apps/showcase`）存在三处结构性摩擦：一是 27 个手写专题页与末尾把全部 133 个 `WidgetCatalogId` 堆入单页的 `All Components` 双轨割裂，且导航标题中的计数手工维护易过期；二是首页与索引页被强行套入带 `OPTIONS` 旋钮和底部代码折叠条的 `ExampleCard` 模板，首屏视觉噪音大且缺少直观导览；三是像素对照报告页（`ParityReport`）纯纵向平铺全部用例，缺少按状态/维度筛选与交互式像素对比视图（如卷帘滑块与混叠），排查失败项效率低。

## Decision

1. **单一事实源元数据下沉至 `WidgetCatalogId`**：在 `unbound-widgets` 中为 `WidgetCatalogId` 增加穷举 `const fn` 元数据，涵盖 10 个功能分类（`ControlCategory`：Basic Input、Text & Form、Date & Time、Collections & Data、Layout & Containers、Navigation、Menus & Toolbars、Dialogs & Overlays、Status/Media/Scrolling、Canvas/Graph & DevTools）与三层来源归属（`ControlOrigin`：`WinUi3`、`Toolkit`、`Unbound`）。展台导航树、首页分类卡片与编译期计数全部由此派生。
2. **三套展台页面模板与一控件一详情页**：拆分 Landing（首页）、Category Hub（全部控件与 10 个分类索引页）与 Detail / Lab（单控件交互工坊与引擎实验室）三套模板；废除把 133 个控件堆在一页的超长 `All Components` 滚动页，复用 `NavigationView` 多级折叠树（默认折叠分类，展开直达单控件详情页），并将既有深度交互专题融入对应控件详情页。
3. **无头逐页覆盖率门禁**：覆盖率红线由“单超长页全发射”升级为“编译期 `WidgetCatalogId` 穷举元数据 + 无头回归测试逐页遍历首页、10 个分类索引页与全部 133 个控件详情页并汇总真实发射器见证”。
4. **主从式像素对照检视台（Parity Inspector）**：`ParityReport` 改为左列表 + 右舞台的 Master-Detail 布局。列表区提供带计数的状态筛选胶囊（有失败/无效用例时默认聚焦失败项）、关键词/主题/后景多维过滤以及默认按状态严重度与 `mean ΔE` 降序排序，支持键盘 `↑`/`↓` 切换；检视舞台支持一键切换**并排（Side-by-Side）**、**卷帘（Curtain，默认鼠标跟随、点击锁定/解锁分界线）**与**叠加（Overlay，透明度混叠与热力图叠层）**三种对比模式，并支持 `1x / 2x / 4x` 像素缩放。

## Consequences

- 新增 `WidgetCatalogId` 变体时必须在编译期同步声明 `ControlCategory`、`ControlOrigin` 与展示元信息，展台导航与分类页自动收录，不再出现计数过期或遗漏。
- 展台壳层路由由纯 `PageId` 扩展为同时承载分类索引与单控件详情目标（含最近访问与收藏状态），无头集成测试与 `Target` 标识空间需同步演进。
- 卷帘与叠加对比模式直接复用 `AssetHub` 既有纹理与裁剪原语，不引入额外离屏合成拷贝。
